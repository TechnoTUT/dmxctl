//! スマホなどのブラウザから操作する、小さなWebの操作パネル(http://このPCのIP:8080/)。
//! 画面の無いPC(dmxctl headless)で動かしているときに、様子を見たり、ブラックアウト・オート・プリセット・色・シーンを操作する。
//! 外部のクレートは使わず、std の TCP で最小限の HTTP/1.1 を話す。通常は1回の要求ごとに接続を閉じ、`/api/events` だけは接続を保持して様子を送り続ける

use dmxctl_core::color::{ColorRole, ColorTarget, NamedColor};
use dmxctl_core::engine::{lock, Core, Shared, EMERGENCY};
use dmxctl_core::project::PHRASES;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// 操作パネルのページ(web/ の React を単一のHTMLにビルドしたもの)
const PAGE: &str = include_str!(concat!(env!("OUT_DIR"), "/web/index.html"));

pub struct WebServer {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    pub port: u16,
}

impl WebServer {
    /// `ctx`は操作したときに画面を更新させるためのもの
    pub fn start(core: Shared, ctx: Option<dmxctl_core::wake::Wake>, port: u16) -> Result<WebServer, String> {
        let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| format!("ポート{port}を開けません: {e}"))?;
        // 0 を指定したときは、割り当てられたポートを返す
        let port = listener.local_addr().map_or(port, |a| a.port());
        Self::start_listener(core, ctx, listener, port)
    }

    fn start_listener(core: Shared, ctx: Option<dmxctl_core::wake::Wake>, listener: TcpListener, port: u16) -> Result<Self, String> {
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let feed = Arc::new(Feed::default());
        let handle = std::thread::Builder::new()
            .name("web".into())
            .spawn(move || {
                while !s2.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let (core, ctx, stop, feed) = (core.clone(), ctx.clone(), s2.clone(), feed.clone());
                            // 1つの接続が遅くても、ほかを待たせない
                            let _ = std::thread::Builder::new().name("web-conn".into()).spawn(move || serve(stream, &core, ctx.as_ref(), &stop, &feed));
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(50)),
                        Err(_) => std::thread::sleep(Duration::from_millis(200)),
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(WebServer { stop, handle: Some(handle), port })
    }
}

impl Drop for WebServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// このPCの、ほかの機器から見たIPアドレス(案内用。分からなければ None)。
/// UDPのソケットを外向きに「つなぐ」だけで、実際には何も送らない
pub fn local_ip() -> Option<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("192.0.2.1:80").ok()?;
    s.local_addr().ok().map(|a| a.ip()).filter(|ip| !ip.is_unspecified())
}

/// 案内用のURL
pub fn url(port: u16) -> String {
    format!("http://{}:{port}/", local_ip().map(|ip| ip.to_string()).unwrap_or_else(|| "このPCのIPアドレス".into()))
}

struct Request {
    method: String,
    path: String,
    pin: Option<String>,
    body: Vec<u8>,
}

/// 要求の本文の上限
const MAX_BODY: usize = 64 * 1024;
/// 要求のヘッダーの上限
const MAX_HEAD: usize = 16 * 1024;

/// 要求を読めなかった理由。接続が切れた(Closed)とき以外は、理由を答えてから閉じる
enum ReadError {
    Closed,
    /// HTTP の状態と、理由
    Reply(&'static str, &'static str),
}

/// 要求を読む(ヘッダーは16KB、本文は64KBまで)
fn read_request(stream: &mut impl Read) -> Result<Request, ReadError> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    let head_end = loop {
        let n = stream.read(&mut chunk).map_err(|_| ReadError::Closed)?;
        if n == 0 {
            return Err(ReadError::Closed);
        }
        buf.extend_from_slice(&chunk[..n]);
        let delimiter = buf.windows(4).position(|w| w == b"\r\n\r\n");
        let head_len = delimiter.map_or(buf.len(), |index| index + 4);
        if head_len > MAX_HEAD {
            return Err(ReadError::Reply("431 Request Header Fields Too Large", "header too large"));
        }
        if let Some(index) = delimiter {
            break index;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().ok_or(ReadError::Closed)?.split_whitespace();
    let method = first.next().ok_or(ReadError::Reply("400 Bad Request", "bad request line"))?.to_string();
    let path = first.next().and_then(|p| p.split('?').next()).ok_or(ReadError::Reply("400 Bad Request", "bad request line"))?.to_string();
    let mut len = None;
    let mut pin = None;
    for l in lines {
        let Some((k, v)) = l.split_once(':') else { continue };
        match k.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                let field = v.trim();
                let invalid = || ReadError::Reply("400 Bad Request", "invalid Content-Length");
                if field.is_empty() || !field.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(invalid());
                }
                let value = field.parse::<usize>().map_err(|_| invalid())?;
                if len.is_some_and(|previous| previous != value) {
                    return Err(invalid());
                }
                len = Some(value);
            }
            "x-pin" => pin = Some(v.trim().to_string()),
            _ => {}
        }
    }
    let len = len.unwrap_or(0);
    if len > MAX_BODY {
        return Err(ReadError::Reply("413 Payload Too Large", "body too large (max 64KB)"));
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk).map_err(|_| bad_body())?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    if body.len() < len {
        return Err(bad_body());
    }
    body.truncate(len);
    Ok(Request { method, path, pin, body })
}

fn bad_body() -> ReadError {
    ReadError::Reply("400 Bad Request", "incomplete body")
}

fn respond(stream: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) {
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn serve(mut stream: TcpStream, core: &Shared, ctx: Option<&dmxctl_core::wake::Wake>, stop: &AtomicBool, feed: &Arc<Feed>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let req = match read_request(&mut stream) {
        Ok(req) => req,
        Err(ReadError::Closed) => return,
        Err(ReadError::Reply(status, reason)) => {
            respond(&mut stream, status, "application/json", json!({ "error": reason }).to_string().as_bytes());
            return;
        }
    };
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => respond(&mut stream, "200 OK", "text/html; charset=utf-8", PAGE.as_bytes()),
        // 状態は GET、操作は POST だけ(GET で操作が実行されないように)
        ("POST", "/api/state") | ("GET", "/api/cmd") | ("PUT" | "DELETE" | "PATCH", "/api/state" | "/api/cmd") => {
            let allow = if req.path == "/api/cmd" { "POST" } else { "GET" };
            let body = json!({ "error": format!("method not allowed (use {allow})") }).to_string();
            let head = format!("HTTP/1.1 405 Method Not Allowed\r\nAllow: {allow}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body.as_bytes());
        }
        // 様子が変わるたびに送り続ける(Server-Sent Events)。この接続のスレッドは切れるまで戻らない
        ("GET", "/api/events") => {
            let c = lock(core);
            let pin = c.project.web_pin.trim().to_string();
            if !pin.is_empty() && req.pin.as_deref() != Some(pin.as_str()) {
                drop(c);
                respond(&mut stream, "401 Unauthorized", "application/json", br#"{"error":"pin"}"#);
                return;
            }
            drop(c);
            // 上限を超えたクライアントには、1秒ごとの読み直しを使ってもらう
            let Some(_guard) = Feed::acquire(feed) else {
                respond(&mut stream, "503 Service Unavailable", "application/json", br#"{"error":"too many streams"}"#);
                return;
            };
            stream_events(&mut stream, core, feed, &pin, stop);
        }
        ("GET", "/api/state") | ("POST", "/api/cmd") => {
            let mut c = lock(core);
            // 合言葉を決めていれば、APIはそれが合うときだけ
            let pin = c.project.web_pin.trim().to_string();
            if !pin.is_empty() && req.pin.as_deref() != Some(pin.as_str()) {
                drop(c);
                respond(&mut stream, "401 Unauthorized", "application/json", br#"{"error":"pin"}"#);
                return;
            }
            if req.path == "/api/cmd" {
                let cmd: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
                if let Err(e) = command(&mut c, &cmd) {
                    drop(c);
                    respond(&mut stream, "400 Bad Request", "application/json", json!({ "error": e }).to_string().as_bytes());
                    return;
                }
                if let Some(ctx) = ctx {
                    ctx.request_repaint();
                }
            }
            let body = state(&c).to_string();
            drop(c);
            respond(&mut stream, "200 OK", "application/json", body.as_bytes());
        }
        _ => respond(&mut stream, "404 Not Found", "text/plain; charset=utf-8", "見つかりません".as_bytes()),
    }
}

/// 同時に保持する `/api/events` の接続数の上限
const MAX_STREAMS: usize = 16;
/// 様子が変わったかを確かめる間隔
const STREAM_TICK: Duration = Duration::from_millis(200);
/// 変化が無いときに、切れた接続を見つけるために送る合図の間隔
const STREAM_HEARTBEAT: Duration = Duration::from_secs(10);

/// 全接続で共有する、ある時点の様子と合言葉
struct Snap {
    at: Instant,
    body: Arc<str>,
    pin: Arc<str>,
}

/// `/api/events` の接続数と、全接続で共有する最新の様子。
/// 様子は接続ごとではなく全体で `STREAM_TICK` ごとに1回だけ作るので、接続が増えても Core のロックを取る回数は増えない
#[derive(Default)]
struct Feed {
    streams: AtomicUsize,
    snap: Mutex<Option<Snap>>,
}

/// 保持している `/api/events` の接続数を数える。破棄すると1つ減る
struct StreamGuard(Arc<Feed>);

impl Feed {
    fn acquire(this: &Arc<Feed>) -> Option<StreamGuard> {
        if this.streams.fetch_add(1, Ordering::SeqCst) >= MAX_STREAMS {
            this.streams.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(StreamGuard(this.clone()))
    }

    /// 直近の様子と合言葉。古ければ作り直す。ロックの中では値を取るだけで、文字列にするのはロックの外で行う
    fn snapshot(&self, core: &Shared) -> (Arc<str>, Arc<str>) {
        let mut snap = self.snap.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = snap.as_ref().filter(|s| s.at.elapsed() < STREAM_TICK / 2) {
            return (s.body.clone(), s.pin.clone());
        }
        let (value, pin) = {
            let c = lock(core);
            (state(&c), c.project.web_pin.trim().to_string())
        };
        let (body, pin): (Arc<str>, Arc<str>) = (value.to_string().into(), pin.into());
        *snap = Some(Snap { at: Instant::now(), body: body.clone(), pin: pin.clone() });
        (body, pin)
    }
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        self.0.streams.fetch_sub(1, Ordering::SeqCst);
    }
}

/// 様子が変わるたびに `event: state` を送る。変化が無いあいだは、合図のコメントだけを送る。
/// 書き込みに失敗する・合言葉が変わる・サーバーを止めると終了する
fn stream_events(stream: &mut TcpStream, core: &Shared, feed: &Feed, pin: &str, stop: &AtomicBool) {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: keep-alive\r\nX-Accel-Buffering: no\r\n\r\n";
    if stream.write_all(head.as_bytes()).is_err() {
        return;
    }
    let mut last: Arc<str> = Arc::from("");
    let mut quiet = Duration::ZERO;
    while !stop.load(Ordering::SeqCst) {
        let (body, now_pin) = feed.snapshot(core);
        // 合言葉が変わったら切る。クライアントは接続し直して、合言葉を聞かれる
        if *now_pin != *pin {
            return;
        }
        let msg = if body != last {
            last = body;
            quiet = Duration::ZERO;
            Some(format!("event: state\ndata: {last}\n\n"))
        } else if quiet >= STREAM_HEARTBEAT {
            quiet = Duration::ZERO;
            Some(": keep-alive\n\n".to_string())
        } else {
            None
        };
        if let Some(m) = msg
            && stream.write_all(m.as_bytes()).and_then(|_| stream.flush()).is_err()
        {
            return;
        }
        std::thread::sleep(STREAM_TICK);
        quiet += STREAM_TICK;
    }
}

/// 今の様子(パネルの表示用)
pub fn state(c: &Core) -> Value {
    let a = &c.project.auto;
    let to255 = |rgb: Option<[f32; 3]>| rgb.map(|v| v.map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8));
    json!({
        "bpm": c.project.bpm,
        "link_peers": c.rt.link_peers,
        "blackout": c.rt.blackout,
        "master": c.rt.master,
        "fading": c.rt.dim < 0.999,
        "emergency": EMERGENCY.load(Ordering::SeqCst),
        "auto": {
            "enabled": a.enabled,
            "online": c.auto_online(),
            // 秒の端数と盛り上がりの細かい違いでは変わらないようにする。変わるたびにストリームで送るため
            "manual": c.manual_remaining().map(f32::ceil),
            "phrase": c.rt.phrase,
            "look": c.rt.auto_key,
            "energy": (c.rt.energy * 100.0).round() / 100.0,
            "preset": a.preset,
            "presets": a.presets.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
        },
        "blt": c.blt_state().message(),
        "main": to255(c.role_rgb(ColorRole::Main)),
        "sub": to255(c.role_rgb(ColorRole::Sub)),
        "colors": NamedColor::ALL.iter().map(|n| json!({ "name": format!("{n:?}"), "label": n.label(), "rgb": to255(Some(n.rgb())) })).collect::<Vec<_>>(),
        "scenes": c.project.scenes.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
        "last_scene": c.rt.last_scene,
        "phrases": PHRASES.iter().map(|(k, ja)| json!({ "key": k, "label": ja })).collect::<Vec<_>>(),
        "outputs": c.rt.output_status,
    })
}

/// 操作パネルの FLASH は、on: true が届いてからこの時間で消える。画面は押している間 `FLASH_RESEND_MS` ごとに送り直す
pub const WEB_FLASH_HOLD: Duration = Duration::from_millis(1500);

/// 操作(パネルのボタン)。{"cmd": "...", ...}
pub fn command(c: &mut Core, v: &Value) -> Result<(), String> {
    let cmd = v["cmd"].as_str().ok_or("cmd がありません")?;
    // on は真偽値だけ(文字列などを切り替えとして扱わない)。無ければ切り替え
    let on = match v.get("on") {
        None => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err("on は true か false です".into()),
    };
    // value が無いときと、数値でないときを分けて知らせる
    let num = || -> Result<f64, String> {
        match v.get("value") {
            None => Err("value がありません".into()),
            Some(n) => n.as_f64().ok_or_else(|| "value は数値で指定してください".into()),
        }
    };
    let text = |k: &str| v[k].as_str().map(str::to_string).ok_or(format!("{k} がありません"));
    match cmd {
        "blackout" => c.rt.blackout = on.unwrap_or(!c.rt.blackout),
        // 範囲外は黙って丸めずに拒否する(ctl・MCP と同じ)
        "master" => {
            let v = num()?;
            if !(0.0..=1.0).contains(&v) {
                return Err("value は 0〜1 です".into());
            }
            c.rt.master = v as f32;
        }
        "fade" => {
            let dark = on.unwrap_or(c.rt.dim > 0.5);
            // secs があればその秒数(OSC の /dmx/fade と同じ)。無ければ設定の秒数
            let secs = match v.get("secs") {
                Some(s) => s.as_f64().filter(|s| (0.0..=60.0).contains(s)).ok_or("secs は 0〜60 の数値です")? as f32,
                None => c.project.fade_secs,
            };
            c.fade_to_black(dark, secs);
        }
        // 押している間だけ。画面は押している間 on: true を送り続け、離したら on: false を送る。
        // 電波切れやタブを閉じて離した合図が届かなくても、送り続けが途絶えたら消える
        "flash" => {
            let on = on.unwrap_or(false);
            c.set_flash(on.then_some(true));
            c.rt.flash_deadline = on.then(|| c.now() + WEB_FLASH_HOLD);
        }
        "tap" => c.tap(),
        "resync" => c.resync(),
        "bpm" => {
            let v = num()?;
            use dmxctl_core::project::{BPM_MAX, BPM_MIN};
            if !(f64::from(BPM_MIN)..=f64::from(BPM_MAX)).contains(&v) {
                return Err(format!("value は {BPM_MIN}〜{BPM_MAX} です"));
            }
            c.set_bpm(v as f32);
        }
        "auto" => {
            c.project.auto.enabled = on.unwrap_or(!c.project.auto.enabled);
            c.dirty = true;
        }
        "auto_resume" => c.rt.manual_last = None,
        "preset" => {
            let i = v["index"].as_u64().ok_or("index がありません")? as usize;
            if i >= c.project.auto.presets.len() {
                return Err("そのプリセットはありません".into());
            }
            c.project.auto.preset = i;
            c.dirty = true;
            // 今のフレーズの見た目を、新しいプリセットで出し直す
            let key = c.rt.auto_key.clone();
            if !key.is_empty() {
                c.auto_phrase(&key);
            }
        }
        // フレーズの見た目を今すぐ試す(オートの画面の「試す」と同じ)
        "look" => {
            let key = text("phrase")?;
            let look = c.preset_look(&key).ok_or("そのフレーズはありません")?;
            c.apply_look(&key, &look);
        }
        "color" => {
            let name = text("name")?;
            let n = NamedColor::from_name(&name).ok_or("その色はありません")?;
            let target = if v["slot"].as_str() == Some("sub") { ColorTarget::Sub } else { ColorTarget::Main };
            // 人が選んだ色なので、しばらくオートより優先する
            c.note_manual();
            c.apply_named_color(n, target);
        }
        "swap" => c.swap_palette(),
        "scene" => {
            let name = text("name")?;
            if !c.project.scenes.iter().any(|s| s.name == name) {
                return Err("そのシーンはありません".into());
            }
            c.note_manual();
            c.recall_scene(&name);
        }
        _ => return Err(format!("知らない操作です: {cmd}")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmxctl_core::fixture::Library;
    use dmxctl_core::project::Project;

    fn core() -> Core {
        let (lib, _) = Library::load(std::path::Path::new("none"));
        Core::new(Project::starter(&lib), Arc::new(lib))
    }

    #[test]
    fn on_must_be_a_boolean_and_values_must_be_in_range() {
        let mut c = core();
        // 真偽値でない on は、切り替えとして扱わずに拒む
        assert_eq!(command(&mut c, &json!({"cmd": "blackout", "on": "no"})).unwrap_err(), "on は true か false です");
        assert!(!c.rt.blackout);
        assert!(command(&mut c, &json!({"cmd": "auto", "on": "off"})).is_err());
        command(&mut c, &json!({"cmd": "blackout", "on": true})).unwrap();
        assert!(c.rt.blackout);
        command(&mut c, &json!({"cmd": "blackout"})).unwrap();
        assert!(!c.rt.blackout, "on が無ければ切り替え");
        // 範囲外の全体光量・BPM は、丸めずに拒む
        for v in [json!({"cmd": "master", "value": 2}), json!({"cmd": "master", "value": -0.1}), json!({"cmd": "bpm", "value": 99999}), json!({"cmd": "bpm", "value": -5})] {
            assert!(command(&mut c, &v).is_err(), "{v}");
        }
        assert_eq!((c.rt.master, c.project.bpm), (1.0, 120.0));
        command(&mut c, &json!({"cmd": "master", "value": 0.5})).unwrap();
        command(&mut c, &json!({"cmd": "bpm", "value": 128})).unwrap();
        assert_eq!((c.rt.master, c.project.bpm), (0.5, 128.0));
    }

    #[test]
    fn rejects_null_switches_and_values_just_outside_the_range() {
        let mut c = core();
        for request in [
            json!({"cmd": "blackout", "on": null}),
            json!({"cmd": "master", "value": 1.00000001}),
            json!({"cmd": "master", "value": -1e-50}),
            json!({"cmd": "bpm", "value": 300.000001}),
        ] {
            assert!(command(&mut c, &request).is_err(), "{request}");
            assert!(!c.rt.blackout);
            assert_eq!((c.rt.master, c.project.bpm), (1.0, 120.0));
        }
    }

    #[test]
    fn header_limit_applies_when_the_delimiter_arrives_with_the_oversize_header() {
        for (extra, is_ok) in [(0, true), (1, false)] {
            let mut request = b"GET /api/state HTTP/1.1\r\nX-Example: ".to_vec();
            request.resize(MAX_HEAD + extra - 4, b'a');
            request.extend_from_slice(b"\r\n\r\n");
            let result = read_request(&mut std::io::Cursor::new(request));
            if is_ok {
                assert!(result.is_ok());
            } else {
                assert!(matches!(result, Err(ReadError::Reply("431 Request Header Fields Too Large", _))));
            }
        }
    }

    #[test]
    fn rejects_ambiguous_or_invalid_content_lengths() {
        for request in [
            b"GET /api/state HTTP/1.1\r\nContent-Length: +0\r\n\r\n".as_slice(),
            b"GET /api/state HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 0\r\n\r\n".as_slice(),
        ] {
            assert!(matches!(read_request(&mut std::io::Cursor::new(request)), Err(ReadError::Reply("400 Bad Request", _))));
        }
    }

    /// 操作パネルの HTTP の窓口に、生の要求を送って、答えの1行目を返す
    fn http(port: u16, request: &[u8]) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let _ = s.write_all(request);
        // 送り終えたことを伝える(本文が途中で切れた要求でも、本体が続きを待ち続けないように)
        let _ = s.shutdown(std::net::Shutdown::Write);
        let mut out = String::new();
        let _ = s.read_to_string(&mut out);
        out.lines().next().unwrap_or("").to_string()
    }

    #[test]
    fn methods_and_malformed_requests_get_an_answer() {
        let shared: Shared = Arc::new(std::sync::Mutex::new(core()));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let _server = WebServer::start_listener(shared.clone(), None, listener, port).unwrap();
        let cmd = br#"{"cmd":"blackout","on":true}"#;
        let with_body = |method: &str, path: &str, body: &[u8]| {
            let mut r = format!("{method} {path} HTTP/1.1\r\nHost: x\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
            r.extend_from_slice(body);
            r
        };
        // 操作は POST だけ。GET では実行しない
        assert!(http(port, &with_body("GET", "/api/cmd", cmd)).contains("405"));
        assert!(!lock(&shared).rt.blackout, "GET で操作が実行された");
        assert!(http(port, &with_body("POST", "/api/state", b"")).contains("405"));
        assert!(http(port, &with_body("POST", "/api/cmd", cmd)).contains("200"));
        assert!(lock(&shared).rt.blackout);
        assert!(http(port, b"GET /api/state HTTP/1.1\r\nHost: x\r\n\r\n").contains("200"));
        // 読めない要求にも、理由を答える
        assert!(http(port, &with_body("POST", "/api/cmd", &vec![b'a'; 70 * 1024])).contains("413"));
        assert!(http(port, b"POST /api/cmd HTTP/1.1\r\nHost: x\r\nContent-Length: abc\r\n\r\n").contains("400"));
        assert!(http(port, b"POST /api/cmd HTTP/1.1\r\nHost: x\r\nContent-Length: 50\r\n\r\n{\"cmd\"").contains("400"), "本文が途中で切れた");
    }

    /// #109: 操作パネルの FLASH は、送り続けが途絶えたら消える
    #[test]
    fn panel_flash_expires_without_resend() {
        let (lib, _) = dmxctl_core::fixture::Library::load(std::path::Path::new("none"));
        let mut c = Core::new(dmxctl_core::project::Project::starter(&lib), std::sync::Arc::new(lib));
        command(&mut c, &json!({"cmd":"flash","on":true})).unwrap();
        assert_eq!(c.rt.flash, Some(true));
        assert!(c.rt.flash_deadline.is_some_and(|d| d <= c.now() + WEB_FLASH_HOLD));
        command(&mut c, &json!({"cmd":"flash","on":false})).unwrap();
        assert!(c.rt.flash.is_none() && c.rt.flash_deadline.is_none());
        assert_eq!(command(&mut c, &json!({"cmd":"bpm","value":"abc"})).unwrap_err(), "value は数値で指定してください");
    }
}
