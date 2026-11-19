//! サーバー: クライアントを待ち受け、プロジェクトと実行中の状態を配る。
use super::*;

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
pub(super) fn list_json(dir: &std::path::Path, limit: usize) -> Vec<String> {
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

pub(super) fn project_files(project_path: Option<&std::path::Path>) -> Files {
    let Some(path) = project_path else { return Files::default() };
    let dir = path.parent().unwrap_or(std::path::Path::new("."));
    let mut projects = list_json(dir, 200);
    projects.sort();
    Files { current: path.display().to_string(), projects, backups: list_json(&dir.join("backups"), 50) }
}

/// クライアントが開いてよいのは、配った一覧にあるファイルだけ
pub(super) fn allowed_file(project_path: Option<&std::path::Path>, path: &str) -> bool {
    let files = project_files(project_path);
    files.projects.iter().chain(&files.backups).any(|p| p == path)
}

pub(super) struct ConnState {
    /// このクライアントが持っているはずのプロジェクト(Null なら、次に全部を送る)
    sent: Value,
    /// このクライアントの編集を処理した回数(配る直前に編集が割り込んだら、その回はプロジェクトを配らない)
    edits: u64,
    /// 処理済みの通し番号
    ack: u64,
    need_frames: bool,
    profiles_sent: bool,
}

pub(super) struct Conn {
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
    /// `allow_safety_edit`: クライアントから、機材定義の安全の設定を変えることを許す(`--remote-allow-safety-edit`)
    pub fn start(core: Shared, ctx: dmxctl_core::wake::Wake, address: SocketAddr, allow_safety_edit: bool) -> anyhow::Result<Self> {
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
                                refuse(stream, &format!("つながっているクライアントが多すぎます({MAX_CLIENTS}台まで)"));
                                continue;
                            }
                            let (core, ctx, conns, events) = (core.clone(), ctx.clone(), conns.clone(), events.clone());
                            let _ = std::thread::Builder::new().name("remote-conn".into()).spawn(move || {
                                let _ = serve_conn(stream, &core, &ctx, &events, &conns, allow_safety_edit);
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

/// 理由を伝えてから接続を閉じる(クライアントは接続中の表示に理由を出す)。
/// 先に hello を読んでおく(読まずに閉じると、理由が届く前に接続がリセットされることがある)
pub(super) fn refuse(stream: TcpStream, msg: &str) {
    let line = json!({"t":"error","fatal":true,"msg":msg}).to_string();
    let _ = std::thread::Builder::new().name("remote-refuse".into()).spawn(move || {
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
        let Ok(mut writer) = stream.try_clone() else { return };
        let _ = read_line(&mut BufReader::new(stream), &mut Vec::new());
        let _ = writeln!(writer, "{line}");
        let _ = writer.shutdown(std::net::Shutdown::Write);
    });
}

pub(super) fn serve_conn(stream: TcpStream, core: &Shared, ctx: &dmxctl_core::wake::Wake, events: &ServerEvents, conns: &Mutex<Vec<Arc<Conn>>>, allow_safety_edit: bool) -> anyhow::Result<()> {
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
    // このクライアントが押したまま、まだ離していないパッド・キー(切れたら離す)
    let mut held = Held::default();

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
            // 機器の一覧は作るのに時間がかかる(数百ms)ので、別のスレッドで作る。
            // このループで作ると、そのあいだ同じクライアントの操作(MIDI・パッド)が待たされる
            if v["t"] == "devices" {
                let tx = conn.tx.clone();
                let _ = std::thread::Builder::new().name("remote-devices".into()).spawn(move || {
                    let devices = Devices::list();
                    let _ = tx.try_send(json!({"t":"devices","devices":devices}).to_string());
                });
                guard(&conn.state).ack = seq;
                continue;
            }
            let outcome: anyhow::Result<bool> = {
                let mut c = lock(core);
                let r = match v["t"].as_str().unwrap_or("") {
                    "live" => serde_json::from_value::<LiveOp>(v["op"].clone()).map_err(anyhow::Error::from).map(|op| {
                        let services = op.needs_services();
                        held.track(&op);
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
                        // 当てる場所が無いのも、ほかのクライアントが一覧に追加・削除をした行き違い
                        r.map(|a| a.services).map_err(|e| if e.is::<project_patch::Conflict>() { e } else { anyhow::anyhow!("{}({e})", project_patch::Conflict) })
                    }
                    // 機材定義はサーバーのフォルダに保存する(変わった定義は、次の状態で全クライアントに配られる)。
                    // 自動操作の窓口と同じく、レーザーなどの安全の設定(安全な値・ARMが要るか)は変えさせない。
                    // サーバーを --remote-allow-safety-edit で起動したときだけ許す(画面の無いサーバーで変えるため)
                    "profile_save" => serde_json::from_value::<Profile>(v["profile"].clone()).map_err(anyhow::Error::from)
                        .and_then(|p| c.save_profile(&p, !allow_safety_edit, false).context(SAFETY_READ_ONLY)).map(|_| false),
                    "profile_delete" => c.delete_profile(v["id"].as_str().unwrap_or(""), !allow_safety_edit, false).context(SAFETY_READ_ONLY).map(|_| false),
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
    // 押している間だけ効くもの(フラッシュ・暗転など)を押したまま切れたら、離したことにする
    // (照明が戻らなくならないように)
    let releases = held.releases();
    if !releases.is_empty() {
        let mut c = lock(core);
        for op in releases {
            c.live(op);
        }
        drop(c);
        ctx.request_repaint();
    }
    let _ = writer_thread.join();
    result
}

/// クライアントが押したまま離していないMIDIのノートとキーボードのキー
#[derive(Default)]
pub(super) struct Held {
    notes: std::collections::BTreeSet<(u8, u8)>,
    keys: std::collections::BTreeSet<u8>,
}

impl Held {
    fn track(&mut self, op: &LiveOp) {
        use dmxctl_core::project::TriggerKind;
        match *op {
            LiveOp::Midi { kind: TriggerKind::Note, channel, number, value } => {
                let note = (channel & 0x0F, number & 0x7F);
                if value > 0 { self.notes.insert(note); } else { self.notes.remove(&note); }
            }
            LiveOp::Midi { kind: TriggerKind::Key, number, value, .. } => self.key(number, value > 0),
            LiveOp::Key { number, pressed } => self.key(number, pressed),
            _ => {}
        }
    }

    fn key(&mut self, number: u8, pressed: bool) {
        if pressed { self.keys.insert(number); } else { self.keys.remove(&number); }
    }

    /// 全部を離す操作
    fn releases(&self) -> Vec<LiveOp> {
        let notes = self.notes.iter().map(|&(channel, number)| LiveOp::Midi { kind: dmxctl_core::project::TriggerKind::Note, channel, number, value: 0 });
        notes.chain(self.keys.iter().map(|&number| LiveOp::Key { number, pressed: false })).collect()
    }
}

pub(super) fn broadcast_loop(core: &Shared, conns: &Mutex<Vec<Arc<Conn>>>, stop: &AtomicBool) {
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

