//! サーバー/クライアント構成(`--remote-listen` / `dmxctl connect`)。
//!
//! - **サーバー**: いつもどおり Core・出力ループ・OSC/Link/NDI/音声/操作パネルを動かし、
//!   LANのクライアントへプロジェクトと実行中の状態を配る。DMXの送出はサーバーだけが行う。
//! - **クライアント**: 画面(egui)とMIDIコントローラーだけを手元で動かす。手元の Core はサーバーの写しで、
//!   出力ループも外部との連携も動かさない。
//!
//! 手元の操作は2通りでサーバーへ届ける:
//! 1. 実行中の状態を変える操作(TAP・シーン・色・MIDIの入力など): `Core::live` が `LiveOp` をそのまま送る。
//!    手元の写しは変えず、サーバーから戻ってくる状態で更新される
//! 2. 画面がプロジェクトを直接書き換える編集(スライダー・名前の欄など): 1フレームの前後を比べた差分
//!    (`project_patch::Change`)を送る。手元の写しには、確認される前から編集が入っている
//!
//! 通信は TCP の上で1行1つのJSON。信頼できるネットワークだけで使う前提で、認証はしない。
//! サーバーは約30Hzで状態を、約10Hzでプロジェクトの差分を配る。クライアントは送ったものに通し番号(`seq`)を付け、
//! サーバーは処理済みの番号(`ack`)を状態に付けて返す。
//!
//! プロジェクトの食い違いを防ぐため、クライアントは「サーバーが持っているはずのプロジェクト」(`confirmed`)と
//! 「まだ処理済みと言われていない自分の編集」(`pending`)を分けて持つ。サーバーは接続ごとに、そのクライアントが
//! 持っているはずのプロジェクトを覚えて差分を作る。どちらも「処理済みの自分の編集 → 届いた差分」の順に当てるので、
//! 行き違いがあっても同じものになる。手元の写しは `confirmed` に `pending` を重ねたもの。

use crate::engine::{lock, Core, Shared, EMERGENCY};
use crate::fixture::{Library, Profile};
use crate::live::LiveOp;
use crate::project::{Project, PAGES};
use crate::project_patch::{self, Change};
use anyhow::{bail, ensure, Context as _};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const DEFAULT_PORT: u16 = 9300;
const PROTOCOL: u64 = 2;
/// 1行の上限(機材定義・プロジェクト全体を1行で送るので大きめ)
const MAX_LINE: u64 = 32 * 1024 * 1024;
/// 状態を配る間隔(約30Hz)と、プロジェクトの変化を調べる間隔(その3回に1回)
const BROADCAST: Duration = Duration::from_millis(33);
const PROJECT_EVERY: u32 = 3;
/// 同時につなげるクライアントの数
const MAX_CLIENTS: usize = 16;

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn age(t: Option<Instant>) -> Option<u64> {
    t.map(|t| t.elapsed().as_millis() as u64)
}

fn from_age(v: &Value) -> Option<Instant> {
    v.as_u64().and_then(|ms| Instant::now().checked_sub(Duration::from_millis(ms)))
}

fn hex(data: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        s.push(D[(b >> 4) as usize] as char);
        s.push(D[(b & 15) as usize] as char);
    }
    s
}

fn unhex(s: &str, out: &mut [u8; 512]) {
    let v = |c: u8| match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    };
    for (i, pair) in s.as_bytes().chunks(2).take(512).enumerate() {
        if pair.len() == 2 {
            out[i] = (v(pair[0]) << 4) | v(pair[1]);
        }
    }
}

fn sorted_pairs<K: Ord + Copy, V: Copy>(m: &HashMap<K, V>) -> Vec<(K, V)> {
    let mut v: Vec<(K, V)> = m.iter().map(|(k, v)| (*k, *v)).collect();
    v.sort_by_key(|p| p.0);
    v
}

fn read_line(reader: &mut BufReader<TcpStream>, buf: &mut Vec<u8>) -> anyhow::Result<bool> {
    buf.clear();
    let n = reader.by_ref().take(MAX_LINE + 1).read_until(b'\n', buf)?;
    if n == 0 {
        return Ok(false);
    }
    ensure!(buf.len() as u64 <= MAX_LINE, "message is too large");
    Ok(true)
}

// ------------------------------------------------------------------ 実行中の状態の写し(サーバー → クライアント)

/// 画面の表示に使う実行中の状態
fn rt_state(c: &Core) -> Value {
    let rt = &c.rt;
    let mut v = json!({
        "blackout": rt.blackout,
        "master": rt.master,
        "page": rt.page,
        "key_page": rt.key_page,
        "color_global": rt.color_global,
        "selected": sorted_ids(&rt.selected),
        "muted": sorted_ids(&rt.muted),
        "level": sorted_pairs(&rt.level),
        "learn": rt.learn,
        "osc_learn": rt.osc_learn,
        "last_position": rt.last_position,
        "last_color": rt.last_color,
        "toggle_state": sorted_pairs(&rt.toggle_state),
        "pad_state": sorted_pairs(&rt.pad_state),
        // ARMは保存しない項目(プロジェクトのJSONに入らない)なので、ここで配る
        "armed": c.project.fixtures.iter().filter(|f| f.armed).map(|f| f.id).collect::<Vec<_>>(),
    });
    // json! は項目が多いと展開しきれないので、2つに分けて合わせる
    let more = json!({
        "beat": rt.beat,
        "beat_age": age(rt.last_beat_t),
        "link_peers": rt.link_peers,
        "link": rt.link.is_some(),
        "osc_status": rt.osc_status,
        "audio_status": rt.audio_status,
        "audio_conf": rt.audio_conf,
        "audio_seen": age(rt.audio_seen),
        "ndi": {"sources": rt.ndi.sources, "status": rt.ndi.status, "audio": rt.ndi.audio, "video": rt.ndi.video},
        "phrase": rt.phrase,
        "auto_key": rt.auto_key,
        "energy": rt.energy,
        "idle_mix": rt.idle_mix,
        "notes_down": (0..128u8).filter(|&n| rt.notes_down[n as usize]).collect::<Vec<_>>(),
        "cc_value": rt.cc_value.to_vec(),
        "used_len": rt.used_len,
        "output_status": rt.output_status,
        "last_midi": rt.last_midi,
        "last_osc": rt.last_osc,
        "chase_idx": rt.chase_idx,
        "last_scene": rt.last_scene,
        "color_swapped": rt.color_swapped,
        "dim": rt.dim,
        "dim_target": rt.dim_target,
        "flash": rt.flash,
        "hue": rt.hue,
        "px_speed": rt.px_speed,
        "osc_rgb": rt.osc_rgb.map(|o| o.map(|o| o.current)),
        "osc_level": rt.osc_level.map(|o| o.current[0]),
        "live_main": rt.live_main,
        "live_sub": rt.live_sub,
        "keys_down": rt.keys_down,
        "fades": rt.fades.len(),
        "physical_output": rt.physical_output_enabled,
        "auto_age": age(rt.auto_last),
        "blt_age": age(Some(rt.blt_seen)),
        "manual_age": age(rt.manual_last),
        "clock_age": age(rt.clock.last),
        "emergency": EMERGENCY.load(Ordering::SeqCst),
        "dirty": c.dirty,
        "frame": rt.frame_id,
    });
    if let (Some(a), Value::Object(b)) = (v.as_object_mut(), more) {
        a.extend(b);
    }
    v
}

fn sorted_ids(set: &std::collections::HashSet<u32>) -> Vec<u32> {
    let mut v: Vec<u32> = set.iter().copied().collect();
    v.sort();
    v
}

fn glide<const N: usize>(current: [f32; N]) -> crate::engine::OscGlide<N> {
    crate::engine::OscGlide { target: current, current, secs: 0.0, due: None, last: Instant::now() }
}

fn val<T: DeserializeOwned>(v: &Value, k: &str, old: T) -> T {
    v.get(k).and_then(|x| serde_json::from_value(x.clone()).ok()).unwrap_or(old)
}

/// 届いた状態を手元の写しに入れる。ラーンの状態だけは別(画面で書き換えるので、フレームの境目で入れる)
fn apply_rt_state(c: &mut Core, d: &Value) {
    let rt = &mut c.rt;
    rt.blackout = val(d, "blackout", rt.blackout);
    rt.master = val::<f32>(d, "master", rt.master).clamp(0.0, 1.0);
    rt.page = val::<u8>(d, "page", rt.page).min(PAGES - 1);
    rt.key_page = val(d, "key_page", rt.key_page);
    rt.color_global = val(d, "color_global", rt.color_global);
    rt.selected = val::<Vec<u32>>(d, "selected", Vec::new()).into_iter().collect();
    rt.muted = val::<Vec<u32>>(d, "muted", Vec::new()).into_iter().collect();
    rt.level = val::<Vec<(u32, f32)>>(d, "level", Vec::new()).into_iter().collect();
    rt.last_position = val(d, "last_position", None);
    rt.last_color = val(d, "last_color", None);
    rt.toggle_state = val::<Vec<(usize, bool)>>(d, "toggle_state", Vec::new()).into_iter().collect();
    rt.pad_state = val::<Vec<(usize, bool)>>(d, "pad_state", Vec::new()).into_iter().collect();
    rt.beat = val(d, "beat", rt.beat);
    rt.last_beat_t = from_age(&d["beat_age"]);
    rt.link_peers = val(d, "link_peers", 0);
    rt.remote_link = val(d, "link", false);
    rt.osc_status = val(d, "osc_status", String::new());
    rt.audio_status = val(d, "audio_status", String::new());
    rt.audio_conf = val(d, "audio_conf", 0.0);
    rt.audio_seen = from_age(&d["audio_seen"]);
    rt.ndi.sources = val(&d["ndi"], "sources", Vec::new());
    rt.ndi.status = val(&d["ndi"], "status", String::new());
    rt.ndi.audio = val(&d["ndi"], "audio", None);
    rt.ndi.video = val(&d["ndi"], "video", None);
    rt.phrase = val(d, "phrase", String::new());
    rt.auto_key = val(d, "auto_key", String::new());
    rt.energy = val(d, "energy", 0.0);
    rt.idle_mix = val(d, "idle_mix", 0.0);
    rt.notes_down = [false; 128];
    for n in val::<Vec<u8>>(d, "notes_down", Vec::new()) {
        rt.notes_down[(n & 0x7F) as usize] = true;
    }
    for (i, v) in val::<Vec<Option<u8>>>(d, "cc_value", Vec::new()).into_iter().take(128).enumerate() {
        rt.cc_value[i] = v;
    }
    for (i, v) in val::<Vec<usize>>(d, "used_len", Vec::new()).into_iter().enumerate() {
        if let Some(x) = rt.used_len.get_mut(i) {
            *x = v.min(512);
        }
    }
    rt.output_status = val(d, "output_status", Vec::new());
    rt.last_midi = val(d, "last_midi", String::new());
    rt.last_osc = val(d, "last_osc", String::new());
    rt.chase_idx = val(d, "chase_idx", HashMap::new());
    rt.last_scene = val(d, "last_scene", None);
    rt.color_swapped = val(d, "color_swapped", false);
    rt.dim = val(d, "dim", 1.0);
    rt.dim_target = val(d, "dim_target", 1.0);
    rt.flash = val(d, "flash", None);
    rt.hue = val(d, "hue", rt.hue);
    rt.px_speed = val(d, "px_speed", rt.px_speed);
    rt.osc_rgb = val::<[Option<[f32; 3]>; 2]>(d, "osc_rgb", [None, None]).map(|o| o.map(glide));
    rt.osc_level = val::<Option<f32>>(d, "osc_level", None).map(|v| glide([v]));
    rt.live_main = val(d, "live_main", None);
    rt.live_sub = val(d, "live_sub", None);
    rt.keys_down = val(d, "keys_down", Vec::new());
    rt.remote_fades = val(d, "fades", 0);
    // 実機に出しているか(サーバーの起動のしかた)。画面の「シミュレーション中」の表示に使う
    rt.physical_output_enabled = val(d, "physical_output", false);
    rt.auto_last = from_age(&d["auto_age"]);
    rt.blt_seen = from_age(&d["blt_age"]).unwrap_or_else(Instant::now);
    rt.manual_last = from_age(&d["manual_age"]);
    rt.clock.last = from_age(&d["clock_age"]);
    rt.frame_id = val(d, "frame", rt.frame_id);
    // このプロセスの EMERGENCY(終了のシグナルなど)は上書きしない
    rt.remote_emergency = val(d, "emergency", false);
    c.dirty = val(d, "dirty", false);
    apply_armed(c, d);
}

fn apply_armed(c: &mut Core, d: &Value) {
    let ids: Vec<u32> = val(d, "armed", Vec::new());
    for f in c.project.fixtures.iter_mut() {
        f.armed = ids.contains(&f.id);
    }
}

fn frames_json(frames: &[[u8; 512]], universes: impl Iterator<Item = usize>) -> Value {
    Value::Object(universes.map(|u| (u.to_string(), Value::String(hex(&frames[u])))).collect())
}

fn library_json(lib: &Library) -> (Value, Value) {
    (json!(lib.profiles.iter().map(|p| p.as_ref()).collect::<Vec<_>>()), json!(lib.builtin_ids()))
}

/// サーバーのPCの機器の一覧(出力のシリアルポート・D2XX・音声入力・MIDIクロックの入力を選ぶのに使う。
/// これらはサーバーで開くので、クライアントのPCの一覧ではなく、こちらを出す)
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Devices {
    pub serial: Vec<(String, String)>,
    pub d2xx: Result<Vec<String>, String>,
    pub audio: Vec<String>,
    pub midi: Vec<String>,
}

impl Devices {
    fn list() -> Self {
        Devices { serial: crate::opendmx::list_serial_ports(), d2xx: crate::opendmx::list_d2xx(), audio: crate::audio::input_devices(), midi: crate::midi::list_ports() }
    }
}

// ------------------------------------------------------------------ サーバー

/// サーバーの Host への知らせ(接続のスレッドからは保存・サービスの起動・ファイルの読み込みができないので、Host のループで拾う)
#[derive(Default)]
pub struct ServerEvents {
    pub save: AtomicBool,
    pub services: AtomicBool,
    /// クライアントから頼まれた、プロジェクトの読み込み
    pub open: Mutex<Vec<OpenRequest>>,
}

/// クライアントから頼まれた、プロジェクトの読み込み(ローカルの画面の「開く」と同じく、中身を読み込むだけで保存先は変えない)
pub struct OpenRequest {
    pub source: OpenSource,
    /// 結果を、頼んだクライアントに返す口
    reply: mpsc::SyncSender<String>,
}

pub enum OpenSource {
    /// サーバーのPCのファイル(`files` で配った一覧のもの)
    Path(std::path::PathBuf),
    /// クライアントのPCで選んだファイルの中身
    Text { name: String, text: String },
}

impl OpenRequest {
    /// 結果を、頼んだクライアントの画面の下に出す
    pub fn reply(&self, msg: &str) {
        let _ = self.reply.try_send(json!({"t":"notice","msg":msg}).to_string());
    }
}

/// クライアントに見せる、サーバーのPCのプロジェクトのファイル(プロジェクトのフォルダの .json と、バックアップ)
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Files {
    /// 今の保存先
    pub current: String,
    pub projects: Vec<String>,
    pub backups: Vec<String>,
}

/// 新しいものから。多すぎると一覧が長くなるので、バックアップは最新50個まで
fn list_json(dir: &std::path::Path, limit: usize) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<(std::time::SystemTime, String)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p.display().to_string())))
        .collect();
    files.sort_by(|a, b| b.cmp(a));
    files.into_iter().take(limit).map(|(_, p)| p).collect()
}

fn project_files(project_path: Option<&std::path::Path>) -> Files {
    let Some(path) = project_path else { return Files::default() };
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    let mut projects = list_json(dir, 200);
    projects.sort();
    Files { current: path.display().to_string(), projects, backups: list_json(&dir.join("backups"), 50) }
}

/// クライアントが開いてよいのは、配った一覧にあるファイルだけ
fn allowed_file(project_path: Option<&std::path::Path>, path: &str) -> bool {
    let files = project_files(project_path);
    files.projects.iter().chain(&files.backups).any(|p| p == path)
}

struct ConnState {
    /// このクライアントが持っているはずのプロジェクト(Null なら、次に全部を送る)
    sent: Value,
    /// このクライアントの編集を処理した回数(配る直前に編集が割り込んだら、その回はプロジェクトを配らない)
    edits: u64,
    /// 処理済みの通し番号
    ack: u64,
    need_frames: bool,
    profiles_sent: bool,
}

struct Conn {
    tx: mpsc::SyncSender<String>,
    state: Mutex<ConnState>,
    alive: AtomicBool,
}

pub struct RemoteServer {
    pub address: SocketAddr,
    pub events: Arc<ServerEvents>,
    stop: Arc<AtomicBool>,
    conns: Arc<Mutex<Vec<Arc<Conn>>>>,
    threads: Vec<JoinHandle<()>>,
}

impl RemoteServer {
    /// LANのクライアントを待ち受ける
    pub fn start(core: Shared, ctx: eframe::egui::Context, address: SocketAddr) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(address).with_context(|| format!("クライアントの待ち受けを始められません({address})"))?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let conns: Arc<Mutex<Vec<Arc<Conn>>>> = Arc::default();
        let events: Arc<ServerEvents> = Arc::default();
        let accept = {
            let (core, ctx, stop, conns, events) = (core.clone(), ctx.clone(), stop.clone(), conns.clone(), events.clone());
            std::thread::Builder::new().name("remote-accept".into()).spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if guard(&conns).iter().filter(|c| c.alive.load(Ordering::Relaxed)).count() >= MAX_CLIENTS {
                                continue;
                            }
                            let (core, ctx, conns, events) = (core.clone(), ctx.clone(), conns.clone(), events.clone());
                            let _ = std::thread::Builder::new().name("remote-conn".into()).spawn(move || {
                                let _ = serve_conn(stream, &core, &ctx, &events, &conns);
                            });
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
                        Err(_) => std::thread::sleep(Duration::from_millis(200)),
                    }
                }
            })?
        };
        let broadcast = {
            let (stop, conns) = (stop.clone(), conns.clone());
            std::thread::Builder::new().name("remote-broadcast".into()).spawn(move || broadcast_loop(&core, &conns, &stop))?
        };
        Ok(RemoteServer { address, events, stop, conns, threads: vec![accept, broadcast] })
    }

    /// つながっているクライアントの数
    pub fn clients(&self) -> usize {
        guard(&self.conns).iter().filter(|c| c.alive.load(Ordering::Relaxed)).count()
    }
}

impl Drop for RemoteServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for c in guard(&self.conns).iter() {
            c.alive.store(false, Ordering::Relaxed);
        }
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}

fn serve_conn(stream: TcpStream, core: &Shared, ctx: &eframe::egui::Context, events: &ServerEvents, conns: &Mutex<Vec<Arc<Conn>>>) -> anyhow::Result<()> {
    // macOS/BSD では accept した接続がリスナーのノンブロッキングを引き継ぐ
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut writer = stream.try_clone()?;
    writer.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    ensure!(read_line(&mut reader, &mut buf)?, "closed before hello");
    let hello: Value = serde_json::from_slice(&buf)?;
    if hello["t"] != "hello" || hello["protocol"].as_u64() != Some(PROTOCOL) {
        let msg = "サーバーとクライアントの版が合いません(同じ版の dmxctl を使ってください)";
        let _ = writeln!(writer, "{}", json!({"t":"error","fatal":true,"msg":msg}));
        bail!("{msg}");
    }
    reader.get_ref().set_read_timeout(None)?;

    let (welcome, project) = {
        let c = lock(core);
        let project = serde_json::to_value(&c.project)?;
        let (profiles, builtin) = library_json(&c.lib);
        (json!({"t":"welcome","protocol":PROTOCOL,"project":project,"profiles":profiles,"builtin":builtin,"rt":rt_state(&c)}), project)
    };
    let (tx, rx) = mpsc::sync_channel::<String>(64);
    let conn = Arc::new(Conn {
        tx,
        state: Mutex::new(ConnState { sent: project, edits: 0, ack: hello["seq"].as_u64().unwrap_or(0), need_frames: true, profiles_sent: true }),
        alive: AtomicBool::new(true),
    });
    writeln!(writer, "{welcome}")?;
    // 送るのは別のスレッド(遅いクライアントが、ほかのクライアントや操作の処理を待たせない)
    let writer_thread = {
        let conn = conn.clone();
        std::thread::Builder::new().name("remote-send".into()).spawn(move || {
            while conn.alive.load(Ordering::Relaxed) {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(line) => {
                        if writeln!(writer, "{line}").is_err() {
                            break;
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(_) => break,
                }
            }
            conn.alive.store(false, Ordering::Relaxed);
            let _ = writer.shutdown(std::net::Shutdown::Both);
        })?
    };
    guard(conns).push(conn.clone());

    let result = (|| -> anyhow::Result<()> {
        while conn.alive.load(Ordering::Relaxed) {
            if !read_line(&mut reader, &mut buf)? {
                return Ok(());
            }
            let v: Value = serde_json::from_slice(&buf)?;
            let seq = v["seq"].as_u64().unwrap_or(0);
            // サーバーのPCのプロジェクトのファイルの一覧
            if v["t"] == "files" {
                let path = lock(core).rt.project_path.clone();
                let files = project_files(path.as_deref());
                let _ = conn.tx.try_send(json!({"t":"files","files":files}).to_string());
                guard(&conn.state).ack = seq;
                continue;
            }
            // プロジェクトの読み込みは Host が行う(サービスの設定し直し・バックアップなどがあるので)
            if v["t"] == "open" || v["t"] == "open_content" {
                let source = if v["t"] == "open" {
                    let path = v["path"].as_str().unwrap_or("").to_string();
                    let project_path = lock(core).rt.project_path.clone();
                    if allowed_file(project_path.as_deref(), &path) { Some(OpenSource::Path(path.into())) } else { None }
                } else {
                    Some(OpenSource::Text { name: v["name"].as_str().unwrap_or("").to_string(), text: v["text"].as_str().unwrap_or("").to_string() })
                };
                match source {
                    Some(source) => guard(&events.open).push(OpenRequest { source, reply: conn.tx.clone() }),
                    None => {
                        let _ = conn.tx.try_send(json!({"t":"error","msg":"そのファイルは開けません(プロジェクトのフォルダとバックアップのものだけ開けます)"}).to_string());
                    }
                }
                guard(&conn.state).ack = seq;
                ctx.request_repaint();
                continue;
            }
            // 機器の一覧は時間がかかることがあるので、Core を持たずに作る
            if v["t"] == "devices" {
                let devices = Devices::list();
                let _ = conn.tx.try_send(json!({"t":"devices","devices":devices}).to_string());
                guard(&conn.state).ack = seq;
                continue;
            }
            let outcome: anyhow::Result<bool> = {
                let mut c = lock(core);
                let r = match v["t"].as_str().unwrap_or("") {
                    "live" => serde_json::from_value::<LiveOp>(v["op"].clone()).map_err(anyhow::Error::from).map(|op| {
                        let services = op.needs_services();
                        c.live(op);
                        services
                    }),
                    "patch" => {
                        let changes: Vec<Change> = serde_json::from_value(v["changes"].clone()).unwrap_or_default();
                        let r = c.apply_project_patch(&changes);
                        let mut s = guard(&conn.state);
                        // クライアントは、送った編集が通ったと思って自分の写しに当てる。こちらも同じように当てておき、
                        // 通らなかったときは次に全部を送り直す
                        if r.is_err() || project_patch::apply(&mut s.sent, &changes).is_err() {
                            s.sent = Value::Null;
                        }
                        s.edits += 1;
                        r.map(|a| a.services)
                    }
                    // 機材定義はサーバーのフォルダに保存する(変わった定義は、次の状態で全クライアントに配られる)
                    "profile_save" => serde_json::from_value::<Profile>(v["profile"].clone()).map_err(anyhow::Error::from)
                        .and_then(|p| c.save_profile(&p, false, false)).map(|_| false),
                    "profile_delete" => c.delete_profile(v["id"].as_str().unwrap_or(""), false, false).map(|_| false),
                    "resync" => {
                        guard(&conn.state).sent = Value::Null;
                        Ok(false)
                    }
                    "save" => {
                        events.save.store(true, Ordering::SeqCst);
                        Ok(false)
                    }
                    other => Err(anyhow::anyhow!("unknown message: {other}")),
                };
                // 処理済みの番号は、Core を持ったまま進める(配るときに読む番号と、状態がずれないように)
                guard(&conn.state).ack = seq;
                r
            };
            match outcome {
                Ok(true) => events.services.store(true, Ordering::SeqCst),
                Ok(false) => {}
                Err(e) => {
                    let _ = conn.tx.try_send(json!({"t":"error","msg":e.to_string()}).to_string());
                }
            }
            ctx.request_repaint();
        }
        Ok(())
    })();
    conn.alive.store(false, Ordering::Relaxed);
    let _ = writer_thread.join();
    result
}

fn broadcast_loop(core: &Shared, conns: &Mutex<Vec<Arc<Conn>>>, stop: &AtomicBool) {
    let mut prev_frames: Vec<[u8; 512]> = Vec::new();
    let mut prev_profiles: Vec<usize> = Vec::new();
    let mut library = (Value::Null, Value::Null);
    let mut tick: u32 = 0;
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(BROADCAST);
        tick = tick.wrapping_add(1);
        let list: Vec<Arc<Conn>> = {
            let mut l = guard(conns);
            l.retain(|c| c.alive.load(Ordering::Relaxed));
            l.clone()
        };
        if list.is_empty() {
            prev_frames.clear();
            continue;
        }
        // 先に処理済みの番号を読む(読んだ後に処理した操作は、この回の状態に入っていないかもしれないので)
        let marks: Vec<(u64, u64, bool)> = list
            .iter()
            .map(|c| {
                let s = guard(&c.state);
                (s.ack, s.edits, s.sent.is_null())
            })
            .collect();
        let with_project = tick % PROJECT_EVERY == 0 || marks.iter().any(|m| m.2);
        let (rt, frames, project, profiles) = {
            let c = lock(core);
            (rt_state(&c), c.rt.frames.clone(), with_project.then(|| c.project.clone()), c.lib.clone())
        };
        // プロジェクトはロックの外でJSONにする(40Hzの出力を止めない)
        let project = project.and_then(|p| serde_json::to_value(p).ok());
        let delta = frames_json(&frames, (0..frames.len()).filter(|&u| prev_frames.get(u) != Some(&frames[u])));
        let mut full: Option<Value> = None;
        prev_frames = frames.clone();
        let ids: Vec<usize> = profiles.profiles.iter().map(|p| Arc::as_ptr(p) as usize).collect();
        let profiles_changed = ids != prev_profiles;
        if profiles_changed || library.0.is_null() {
            library = library_json(&profiles);
            prev_profiles = ids;
        }
        for (conn, (ack, edits, _)) in list.iter().zip(marks) {
            let mut s = guard(&conn.state);
            let mut msg = json!({"t":"state","ack":ack,"rt":rt});
            msg["frames"] = if s.need_frames { full.get_or_insert_with(|| frames_json(&frames, 0..frames.len())).clone() } else { delta.clone() };
            s.need_frames = false;
            if let Some(p) = &project
                && s.edits == edits
            {
                let changes = project_patch::diff(&s.sent, p);
                if !changes.is_empty() {
                    s.sent = p.clone();
                    msg["project"] = json!(changes);
                }
            }
            if profiles_changed || !s.profiles_sent {
                msg["profiles"] = library.0.clone();
                msg["builtin"] = library.1.clone();
                s.profiles_sent = true;
            }
            match conn.tx.try_send(msg.to_string()) {
                Ok(()) => {}
                // 送りきれていない: 次からすべてを送り直す
                Err(mpsc::TrySendError::Full(_)) => {
                    s.need_frames = true;
                    s.sent = Value::Null;
                    s.profiles_sent = false;
                }
                Err(mpsc::TrySendError::Disconnected(_)) => conn.alive.store(false, Ordering::Relaxed),
            }
        }
    }
}

// ------------------------------------------------------------------ クライアント

struct LinkInner {
    tx: Mutex<Option<mpsc::Sender<String>>>,
    seq: AtomicU64,
}

/// クライアントからサーバーへ送る口(`Runtime::remote` に入れる)
#[derive(Clone)]
pub struct Link {
    inner: Arc<LinkInner>,
}

impl Link {
    fn new() -> Self {
        Link { inner: Arc::new(LinkInner { tx: Mutex::new(None), seq: AtomicU64::new(0) }) }
    }

    fn send(&self, mut msg: Value) -> u64 {
        let seq = self.inner.seq.fetch_add(1, Ordering::SeqCst) + 1;
        msg["seq"] = json!(seq);
        if let Some(tx) = guard(&self.inner.tx).as_ref() {
            let _ = tx.send(msg.to_string());
        }
        seq
    }

    /// 実行中の状態を変える操作を送る
    pub fn live(&self, op: &LiveOp) {
        self.send(json!({"t":"live","op":op}));
    }

    fn sent(&self) -> u64 {
        self.inner.seq.load(Ordering::SeqCst)
    }
}

struct ClientSync {
    /// サーバーが持っているはずのプロジェクト
    confirmed: Value,
    /// まだ処理済みと言われていない自分の編集(通し番号つき)
    pending: Vec<(u64, Vec<Change>)>,
    /// 手元の写しのプロジェクトを、confirmed + pending で作り直す(画面のフレームの境目で)
    rebuild: bool,
    /// サーバーから最後に届いたラーンの状態と、自分が最後にラーンを送った番号
    learn: (Option<usize>, Option<usize>),
    learn_seq: u64,
    /// 最後に届いた状態の処理済みの番号と、ARMの状態
    ack: u64,
    armed: Value,
    /// サーバーのPCの機器の一覧と、受け取った回数
    devices: Option<Devices>,
    devices_count: u64,
    /// サーバーのPCのプロジェクトのファイルの一覧
    files: Option<Files>,
    connected: bool,
    status: String,
    errors: Vec<String>,
}

/// 初めの接続で受け取ったもの(手元の写しのCoreを作る)
pub struct Initial {
    pub project: Project,
    pub lib: Library,
}

/// サーバーへの接続(`dmxctl connect`)
pub struct Client {
    pub target: String,
    link: Link,
    sync: Arc<Mutex<ClientSync>>,
    stop: Arc<AtomicBool>,
    pending: Option<(TcpStream, BufReader<TcpStream>, Value)>,
    supervisor: Option<JoinHandle<()>>,
    ctx: eframe::egui::Context,
    /// 画面の1フレームの前のプロジェクトとラーンの状態
    before: Option<(Value, (Option<usize>, Option<usize>))>,
}

fn resolve(target: &str) -> anyhow::Result<SocketAddr> {
    let has_port = target.parse::<SocketAddr>().is_ok() || target.rsplit_once(':').is_some_and(|(h, p)| !h.contains(':') && p.parse::<u16>().is_ok());
    let with_port = if has_port { target.to_string() } else { format!("{target}:{DEFAULT_PORT}") };
    with_port.to_socket_addrs()?.next().with_context(|| format!("接続先が見つかりません: {target}"))
}

fn handshake(target: &str, seq: u64) -> anyhow::Result<(TcpStream, BufReader<TcpStream>, Value)> {
    let address = resolve(target)?;
    let stream = TcpStream::connect_timeout(&address, Duration::from_secs(3)).with_context(|| format!("サーバー({address})につながりません"))?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut writer = stream.try_clone()?;
    writer.set_write_timeout(Some(Duration::from_secs(5)))?;
    writeln!(writer, "{}", json!({"t":"hello","protocol":PROTOCOL,"seq":seq}))?;
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    ensure!(read_line(&mut reader, &mut buf)?, "サーバーが接続を閉じました");
    let v: Value = serde_json::from_slice(&buf)?;
    if v["t"] == "error" {
        bail!("{}", v["msg"].as_str().unwrap_or("サーバーに断られました"));
    }
    ensure!(v["t"] == "welcome", "サーバーの応答が不正です");
    // 状態は約30Hzで届く。長く届かなければ切れたとみなす
    reader.get_ref().set_read_timeout(Some(Duration::from_secs(5)))?;
    Ok((writer, reader, v))
}

fn library_from(v: &Value) -> anyhow::Result<Library> {
    let profiles: Vec<Profile> = serde_json::from_value(v["profiles"].clone())?;
    let builtin: Vec<String> = serde_json::from_value(v.get("builtin").cloned().unwrap_or(json!([]))).unwrap_or_default();
    Ok(Library::from_profiles(profiles, builtin))
}

/// サーバーのプロジェクトに、その機械だけの項目(MIDI機器)を戻したもの
fn project_from(server: &Value, local: Option<&Project>) -> anyhow::Result<Project> {
    let mut project: Project = serde_json::from_value(server.clone())?;
    project.midi_input = local.map(|p| p.midi_input.clone()).unwrap_or_default();
    Ok(project)
}

impl Client {
    /// サーバーにつなぎ、プロジェクトと機材定義を受け取る。続きの同期は `attach` で始める
    pub fn connect(target: &str, ctx: eframe::egui::Context) -> anyhow::Result<(Self, Initial)> {
        let (writer, reader, v) = handshake(target, 0)?;
        let confirmed = v["project"].clone();
        let mut project = project_from(&confirmed, None)?;
        let lib = library_from(&v)?;
        let armed = v["rt"]["armed"].clone();
        let ids: Vec<u32> = serde_json::from_value(armed.clone()).unwrap_or_default();
        for f in project.fixtures.iter_mut() {
            f.armed = ids.contains(&f.id);
        }
        let learn = (val(&v["rt"], "learn", None), val(&v["rt"], "osc_learn", None));
        let sync = ClientSync {
            confirmed,
            pending: Vec::new(),
            rebuild: false,
            learn,
            learn_seq: 0,
            ack: 0,
            armed,
            devices: None,
            devices_count: 0,
            files: None,
            connected: true,
            status: format!("接続中: {target}"),
            errors: Vec::new(),
        };
        let client = Client {
            target: target.to_string(),
            link: Link::new(),
            sync: Arc::new(Mutex::new(sync)),
            stop: Arc::new(AtomicBool::new(false)),
            pending: Some((writer, reader, v)),
            supervisor: None,
            ctx,
            before: None,
        };
        Ok((client, Initial { project, lib }))
    }

    /// 手元の写しのCoreを、サーバーの状態に合わせ続ける(切れたら2秒ごとにつなぎ直す)。
    /// このあと、Core::live の操作はサーバーへ送られる
    pub fn attach(&mut self, core: Shared) {
        let (writer, reader, welcome) = self.pending.take().expect("attach once");
        {
            let mut c = lock(&core);
            c.rt.remote = Some(self.link.clone());
            apply_rt_state(&mut c, &welcome["rt"]);
            let s = guard(&self.sync);
            c.rt.learn = s.learn.0;
            c.rt.osc_learn = s.learn.1;
        }
        // 送る口は先に開いておく(スレッドが動き出す前に送ったものも落とさない)
        let (tx, rx) = mpsc::channel::<String>();
        *guard(&self.link.inner.tx) = Some(tx);
        self.request_devices();
        let (link, sync, stop, target, ctx) = (self.link.clone(), self.sync.clone(), self.stop.clone(), self.target.clone(), self.ctx.clone());
        self.supervisor = Some(
            std::thread::Builder::new()
                .name("remote-client".into())
                .spawn(move || {
                    let mut conn = Some((writer, reader, rx));
                    while !stop.load(Ordering::Relaxed) {
                        let (writer, reader, rx) = match conn.take() {
                            Some(c) => c,
                            None => match handshake(&target, link.sent()) {
                                Ok((w, r, v)) => {
                                    let (tx, rx) = mpsc::channel::<String>();
                                    apply_welcome(&core, &sync, &v);
                                    *guard(&link.inner.tx) = Some(tx);
                                    link.send(json!({"t":"devices"}));
                                    (w, r, rx)
                                }
                                Err(e) => {
                                    guard(&sync).status = format!("切断(再接続中): {e}");
                                    ctx.request_repaint();
                                    std::thread::sleep(Duration::from_secs(2));
                                    continue;
                                }
                            },
                        };
                        {
                            let mut s = guard(&sync);
                            s.connected = true;
                            s.status = format!("接続中: {target}");
                        }
                        let error = run_connection(writer, reader, rx, &core, &sync, &link, &stop, &ctx);
                        *guard(&link.inner.tx) = None;
                        {
                            let mut s = guard(&sync);
                            s.connected = false;
                            s.status = format!("切断(再接続中){}", error.map(|e| format!(": {e}")).unwrap_or_default());
                        }
                        ctx.request_repaint();
                        if !stop.load(Ordering::Relaxed) {
                            std::thread::sleep(Duration::from_secs(1));
                        }
                    }
                })
                .expect("remote client thread"),
        );
    }

    /// 画面の1フレームの前: サーバーから届いたプロジェクト・ラーンの状態を写しに入れ、前の状態を覚える
    pub fn before_frame(&mut self, core: &Shared) {
        let mut c = lock(core);
        let mut s = guard(&self.sync);
        if s.rebuild {
            s.rebuild = false;
            let mut value = s.confirmed.clone();
            let ClientSync { pending, errors, .. } = &mut *s;
            pending.retain(|(_, changes)| {
                let ok = project_patch::apply(&mut value, changes).is_ok();
                if !ok {
                    errors.push("サーバーで変わった項目と重なった編集を取り消しました".into());
                }
                ok
            });
            match project_from(&value, Some(&c.project)) {
                Ok(project) => {
                    c.project = project;
                    apply_armed(&mut c, &s.armed);
                }
                Err(e) => s.errors.push(format!("サーバーのプロジェクトを読めません: {e}")),
            }
        }
        // ラーンは、自分が送ったものが処理された後だけ、サーバーの状態に合わせる
        if s.ack >= s.learn_seq {
            c.rt.learn = s.learn.0;
            c.rt.osc_learn = s.learn.1;
        }
        self.before = serde_json::to_value(&c.project).ok().map(|p| (p, (c.rt.learn, c.rt.osc_learn)));
    }

    /// 画面の1フレームの後: 手元で書き換えたプロジェクト・ラーンの状態をサーバーへ送る
    pub fn after_frame(&mut self, core: &Shared) {
        let Some((before, learn)) = self.before.take() else { return };
        let c = lock(core);
        let mut s = guard(&self.sync);
        if !s.connected {
            return;
        }
        if let Ok(now) = serde_json::to_value(&c.project) {
            let changes = project_patch::diff_project(&before, &now);
            if !changes.is_empty() {
                let seq = self.link.send(json!({"t":"patch","changes":changes}));
                s.pending.push((seq, changes));
            }
        }
        let now_learn = (c.rt.learn, c.rt.osc_learn);
        if now_learn != learn {
            self.link.live(&LiveOp::SetLearn { midi: now_learn.0, osc: now_learn.1 });
            s.learn_seq = self.link.sent();
        }
    }

    /// サーバーで保存する
    pub fn save(&self) {
        self.link.send(json!({"t":"save"}));
    }

    /// サーバーのPCの機器の一覧を頼む(届いたら `devices` で読める)
    pub fn request_devices(&self) {
        self.link.send(json!({"t":"devices"}));
    }

    /// サーバーのPCの機器の一覧と、受け取った回数(変わったかどうかを見る)
    pub fn devices(&self) -> Option<(u64, Devices)> {
        let s = guard(&self.sync);
        s.devices.clone().map(|d| (s.devices_count, d))
    }

    /// サーバーのPCのプロジェクトのファイルの一覧を頼む(届いたら `files` で読める)
    pub fn request_files(&self) {
        self.link.send(json!({"t":"files"}));
    }

    pub fn files(&self) -> Option<Files> {
        guard(&self.sync).files.clone()
    }

    /// サーバーのPCのファイルを読み込んでもらう
    pub fn open_path(&self, path: &str) {
        self.link.send(json!({"t":"open","path":path}));
    }

    /// このPCのファイルの中身を送って、サーバーで読み込んでもらう
    pub fn open_text(&self, name: &str, text: &str) {
        self.link.send(json!({"t":"open_content","name":name,"text":text}));
    }

    /// 機材定義をサーバーのフォルダに保存する / 消す
    pub fn save_profile(&self, profile: &Profile) {
        self.link.send(json!({"t":"profile_save","profile":profile}));
    }

    pub fn delete_profile(&self, id: &str) {
        self.link.send(json!({"t":"profile_delete","id":id}));
    }

    pub fn connected(&self) -> bool {
        guard(&self.sync).connected
    }

    pub fn status(&self) -> String {
        guard(&self.sync).status.clone()
    }

    /// サーバーが返したエラーなど(画面に出す)
    pub fn take_errors(&self) -> Vec<String> {
        std::mem::take(&mut guard(&self.sync).errors)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        *guard(&self.link.inner.tx) = None;
        if let Some(t) = self.supervisor.take() {
            let _ = t.join();
        }
    }
}

/// つなぎ直したとき: サーバーの今のプロジェクト・状態にそろえる(切れている間の手元の編集は捨てる)
fn apply_welcome(core: &Shared, sync: &Mutex<ClientSync>, v: &Value) {
    let lib = library_from(v);
    let mut c = lock(core);
    let mut s = guard(sync);
    match lib {
        Ok(lib) => c.lib = Arc::new(lib),
        Err(e) => s.errors.push(format!("機材定義を読めません: {e}")),
    }
    s.confirmed = v["project"].clone();
    s.pending.clear();
    s.rebuild = true;
    s.armed = v["rt"]["armed"].clone();
    s.learn = (val(&v["rt"], "learn", None), val(&v["rt"], "osc_learn", None));
    s.learn_seq = 0;
    apply_rt_state(&mut c, &v["rt"]);
}

fn apply_state(core: &Shared, sync: &Mutex<ClientSync>, link: &Link, v: &Value) {
    let mut c = lock(core);
    let mut s = guard(sync);
    if v.get("profiles").is_some() {
        match library_from(v) {
            Ok(lib) => c.lib = Arc::new(lib),
            Err(e) => s.errors.push(format!("機材定義を読めません: {e}")),
        }
    }
    let ack = v["ack"].as_u64().unwrap_or(0);
    s.ack = ack;
    // サーバーと同じ順に当てる: 処理済みの自分の編集 → 届いた差分
    let mut changed = false;
    while s.pending.first().is_some_and(|(seq, _)| *seq <= ack) {
        let (_, changes) = s.pending.remove(0);
        let ClientSync { confirmed, .. } = &mut *s;
        // 通らなかった編集はサーバーも当てていない(サーバーは全部を送り直してくる)
        let _ = project_patch::apply(confirmed, &changes);
        changed = true;
    }
    if let Some(changes) = v.get("project") {
        let changes: Vec<Change> = serde_json::from_value(changes.clone()).unwrap_or_default();
        let ClientSync { confirmed, .. } = &mut *s;
        if project_patch::apply(confirmed, &changes).is_err() {
            // 食い違った: 全部を送り直してもらう
            link.send(json!({"t":"resync"}));
        }
        changed = true;
    }
    if changed {
        s.rebuild = true;
    }
    if let Some(frames) = v["frames"].as_object() {
        for (u, h) in frames {
            if let (Ok(u), Some(h)) = (u.parse::<usize>(), h.as_str())
                && let Some(f) = c.rt.frames.get_mut(u)
            {
                unhex(h, f);
            }
        }
    }
    let learn = (c.rt.learn, c.rt.osc_learn);
    apply_rt_state(&mut c, &v["rt"]);
    // ラーンは画面で書き換えるので、ここでは戻さない(フレームの境目で入れる)
    (c.rt.learn, c.rt.osc_learn) = learn;
    s.learn = (val(&v["rt"], "learn", None), val(&v["rt"], "osc_learn", None));
    s.armed = v["rt"]["armed"].clone();
}

#[allow(clippy::too_many_arguments)]
fn run_connection(
    writer: TcpStream,
    mut reader: BufReader<TcpStream>,
    rx: mpsc::Receiver<String>,
    core: &Shared,
    sync: &Mutex<ClientSync>,
    link: &Link,
    stop: &AtomicBool,
    ctx: &eframe::egui::Context,
) -> Option<String> {
    let alive = Arc::new(AtomicBool::new(true));
    let sender = {
        let alive = alive.clone();
        let mut writer = writer;
        std::thread::Builder::new()
            .name("remote-client-send".into())
            .spawn(move || {
                while alive.load(Ordering::Relaxed) {
                    match rx.recv_timeout(Duration::from_millis(500)) {
                        Ok(line) => {
                            if writeln!(writer, "{line}").is_err() {
                                break;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(_) => break,
                    }
                }
                alive.store(false, Ordering::Relaxed);
                let _ = writer.shutdown(std::net::Shutdown::Both);
            })
            .expect("remote client sender")
    };
    let mut buf = Vec::new();
    let error = loop {
        if stop.load(Ordering::Relaxed) || !alive.load(Ordering::Relaxed) {
            break None;
        }
        match read_line(&mut reader, &mut buf) {
            Ok(true) => {}
            Ok(false) => break Some("サーバーが接続を閉じました".to_string()),
            Err(e) => break Some(e.to_string()),
        }
        let Ok(v) = serde_json::from_slice::<Value>(&buf) else { continue };
        match v["t"].as_str() {
            Some("state") => {
                apply_state(core, sync, link, &v);
                ctx.request_repaint();
            }
            Some("files") => {
                if let Ok(f) = serde_json::from_value::<Files>(v["files"].clone()) {
                    guard(sync).files = Some(f);
                }
                ctx.request_repaint();
            }
            // サーバーからの知らせ(読み込みの結果など)。エラーと同じく画面の下に出す
            Some("notice") => {
                guard(sync).errors.push(v["msg"].as_str().unwrap_or("").to_string());
                ctx.request_repaint();
            }
            Some("devices") => {
                if let Ok(d) = serde_json::from_value::<Devices>(v["devices"].clone()) {
                    let mut s = guard(sync);
                    s.devices = Some(d);
                    s.devices_count += 1;
                }
                ctx.request_repaint();
            }
            Some("error") => {
                guard(sync).errors.push(v["msg"].as_str().unwrap_or("").to_string());
                ctx.request_repaint();
                if v["fatal"] == true {
                    break v["msg"].as_str().map(str::to_string);
                }
            }
            _ => {}
        }
    };
    alive.store(false, Ordering::Relaxed);
    let _ = reader.get_ref().shutdown(std::net::Shutdown::Both);
    *guard(&link.inner.tx) = None;
    let _ = sender.join();
    error
}

/// `--remote-listen` の値: `0.0.0.0:9300` / `:9300` / `9300`
pub fn parse_listen(value: &str) -> anyhow::Result<SocketAddr> {
    if let Ok(port) = value.trim_start_matches(':').parse::<u16>() {
        return Ok(SocketAddr::from(([0, 0, 0, 0], port)));
    }
    value.parse().with_context(|| format!("待ち受けのアドレスが不正です: {value}(例: 0.0.0.0:{DEFAULT_PORT})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Core;

    fn wait(mut f: impl FnMut() -> bool) -> bool {
        let t = Instant::now();
        while t.elapsed() < Duration::from_secs(5) {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn server() -> (Shared, RemoteServer) {
        let (lib, _) = Library::load(std::path::Path::new("/nonexistent"));
        let project = Project::starter(&lib);
        let core: Shared = Arc::new(Mutex::new(Core::new(project, Arc::new(lib))));
        let server = RemoteServer::start(core.clone(), eframe::egui::Context::default(), "127.0.0.1:0".parse().unwrap()).unwrap();
        (core, server)
    }

    fn client(server: &RemoteServer) -> (Shared, Client) {
        let (mut client, initial) = Client::connect(&server.address.to_string(), eframe::egui::Context::default()).unwrap();
        let core: Shared = Arc::new(Mutex::new(Core::new(initial.project, Arc::new(initial.lib))));
        client.attach(core.clone());
        (core, client)
    }

    /// 画面の1フレーム(edit で写しを書き換える)
    fn frame(client: &mut Client, core: &Shared, edit: impl FnOnce(&mut Core)) {
        client.before_frame(core);
        edit(&mut lock(core));
        client.after_frame(core);
    }

    #[test]
    fn live_ops_run_on_the_server_and_come_back() {
        let (server_core, server) = server();
        let (core, _client) = client(&server);
        lock(&core).live(LiveOp::SetBlackout { on: true });
        // 手元の写しは変えずに送る
        assert!(!lock(&core).rt.blackout);
        assert!(wait(|| lock(&server_core).rt.blackout));
        assert!(wait(|| lock(&core).rt.blackout));
        let id = lock(&server_core).project.fixtures[0].id;
        lock(&core).live(LiveOp::SetArm { fixture: id, on: true });
        assert!(wait(|| lock(&server_core).project.fixtures[0].armed));
        assert!(wait(|| lock(&core).project.fixtures[0].armed));
    }

    #[test]
    fn edits_flow_both_ways() {
        let (server_core, server) = server();
        let (core, mut client) = client(&server);
        frame(&mut client, &core, |c| c.project.fixtures[0].values[0] = 123);
        assert!(wait(|| lock(&server_core).project.fixtures[0].values[0] == 123));
        // サーバーで変わったもの(自動操作など)は、次のフレームの境目で写しに入る
        lock(&server_core).project.fixtures[1].name = "from server".into();
        assert!(wait(|| {
            frame(&mut client, &core, |_| {});
            lock(&core).project.fixtures[1].name == "from server"
        }));
        assert_eq!(lock(&core).project.fixtures[0].values[0], 123);
    }

    #[test]
    fn a_pending_edit_is_not_reverted_by_an_older_state() {
        let (server_core, server) = server();
        let (core, mut client) = client(&server);
        // サーバーで、クライアントの知らない変更をしておく
        lock(&server_core).project.fixtures[1].name = "server edit".into();
        // 同じころ、クライアントは別の値を書き換える
        frame(&mut client, &core, |c| c.project.fixtures[0].values[0] = 77);
        for _ in 0..30 {
            frame(&mut client, &core, |_| {});
            // 手元の編集は、どの時点でも戻らない
            assert_eq!(lock(&core).project.fixtures[0].values[0], 77);
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(lock(&core).project.fixtures[1].name, "server edit");
        assert_eq!(lock(&server_core).project.fixtures[0].values[0], 77);
    }

    #[test]
    fn learn_and_midi_reach_the_server() {
        let (server_core, server) = server();
        let (core, mut client) = client(&server);
        frame(&mut client, &core, |c| c.rt.learn = Some(0));
        assert!(wait(|| lock(&server_core).rt.learn == Some(0)));
        // ラーンは、届いた状態に上書きされずに残る
        frame(&mut client, &core, |_| {});
        assert_eq!(lock(&core).rt.learn, Some(0));
        lock(&core).live(LiveOp::Midi { kind: crate::project::TriggerKind::Note, channel: 0, number: 60, value: 100 });
        assert!(wait(|| lock(&server_core).rt.notes_down[60]));
    }

    #[test]
    fn a_client_reconnects_to_a_restarted_server() {
        let (_old_core, server) = server();
        let address = server.address;
        let (core, mut client) = client(&server);
        drop(server);
        assert!(wait(|| !client.connected()));
        // 同じアドレスで、別のプロジェクトのサーバーを起動し直す
        let (lib, _) = Library::load(std::path::Path::new("/nonexistent"));
        let mut project = Project::starter(&lib);
        project.fixtures[0].name = "after restart".into();
        let new_core: Shared = Arc::new(Mutex::new(Core::new(project, Arc::new(lib))));
        let _server = RemoteServer::start(new_core.clone(), eframe::egui::Context::default(), address).unwrap();
        assert!(wait(|| {
            frame(&mut client, &core, |_| {});
            client.connected() && lock(&core).project.fixtures[0].name == "after restart"
        }));
        // つなぎ直したあとの操作も届く
        lock(&core).live(LiveOp::SetBlackout { on: true });
        assert!(wait(|| lock(&new_core).rt.blackout));
    }

    #[test]
    fn device_lists_and_profiles_go_through_the_server() {
        let dir = std::env::temp_dir().join(format!("dmxctl-remote-profiles-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (lib, _) = Library::load(&dir);
        let project = Project::starter(&lib);
        let server_core: Shared = Arc::new(Mutex::new(Core::new(project, Arc::new(lib))));
        let server = RemoteServer::start(server_core.clone(), eframe::egui::Context::default(), "127.0.0.1:0".parse().unwrap()).unwrap();
        let (core, client) = client(&server);
        // つないだときに、サーバーのPCの機器の一覧が届く
        assert!(wait(|| client.devices().is_some()));
        let (count, _) = client.devices().unwrap();
        client.request_devices();
        assert!(wait(|| client.devices().is_some_and(|(n, _)| n > count)));
        // クライアントで保存した機材定義は、サーバーのフォルダに入り、クライアントにも配られる
        let mut profile = (*lock(&core).lib.profiles[0]).clone();
        profile.id = "test/from-client".into();
        profile.name = "From client".into();
        client.save_profile(&profile);
        assert!(wait(|| lock(&server_core).lib.get("test/from-client").is_some()));
        assert!(wait(|| lock(&core).lib.get("test/from-client").is_some()));
        assert!(std::fs::read_dir(&dir).unwrap().count() > 0, "saved into the server's folder");
        client.delete_profile("test/from-client");
        assert!(wait(|| lock(&server_core).lib.get("test/from-client").is_none()));
        assert!(wait(|| lock(&core).lib.get("test/from-client").is_none()));
        drop(client);
        drop(server);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn listen_addresses_accept_a_bare_port() {
        assert_eq!(parse_listen("9400").unwrap(), "0.0.0.0:9400".parse().unwrap());
        assert_eq!(parse_listen(":9400").unwrap(), "0.0.0.0:9400".parse().unwrap());
        assert_eq!(parse_listen("127.0.0.1:9401").unwrap(), "127.0.0.1:9401".parse().unwrap());
        assert!(parse_listen("nope").is_err());
    }
}
