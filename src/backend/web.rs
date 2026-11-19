//! スマホなどのブラウザから操作する、小さなWebの操作パネル(http://このPCのIP:8080/)。
//! 画面の無いPC(dmxctl headless)で動かしているときに、様子を見たり、ブラックアウト・オート・プリセット・色・シーンを操作する。
//! 外部のクレートは使わず、std の TCP で最小限の HTTP/1.1 を話す(1回の要求ごとに接続を閉じる)

use crate::core::color::{ColorRole, ColorTarget, NamedColor};
use crate::core::engine::{lock, Core, Shared, EMERGENCY};
use crate::core::project::PHRASES;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

/// 操作パネルのページ(1枚のHTML。CSS・JavaScriptも中に入っている)
const PAGE: &str = include_str!("web/panel.html");

pub struct WebServer {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    pub port: u16,
}

impl WebServer {
    /// `ctx`は操作したときに画面を更新させるためのもの
    pub fn start(core: Shared, ctx: Option<eframe::egui::Context>, port: u16) -> Result<WebServer, String> {
        let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| format!("ポート{port}を開けません: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = std::thread::Builder::new()
            .name("web".into())
            .spawn(move || {
                while !s2.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let (core, ctx) = (core.clone(), ctx.clone());
                            // 1つの接続が遅くても、ほかを待たせない
                            let _ = std::thread::Builder::new().name("web-conn".into()).spawn(move || serve(stream, &core, ctx.as_ref()));
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

/// 要求を読む(ヘッダーは16KB、本文は64KBまで)
fn read_request(stream: &mut TcpStream) -> Option<Request> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    let head_end = loop {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 16 * 1024 {
            return None;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split_whitespace();
    let method = first.next()?.to_string();
    let path = first.next()?.split('?').next()?.to_string();
    let mut len = 0usize;
    let mut pin = None;
    for l in lines {
        let Some((k, v)) = l.split_once(':') else { continue };
        match k.trim().to_ascii_lowercase().as_str() {
            "content-length" => len = v.trim().parse().ok()?,
            "x-pin" => pin = Some(v.trim().to_string()),
            _ => {}
        }
    }
    if len > 64 * 1024 {
        return None;
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);
    Some(Request { method, path, pin, body })
}

fn respond(stream: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) {
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

fn serve(mut stream: TcpStream, core: &Shared, ctx: Option<&eframe::egui::Context>) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let Some(req) = read_request(&mut stream) else { return };
    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") | ("GET", "/index.html") => respond(&mut stream, "200 OK", "text/html; charset=utf-8", PAGE.as_bytes()),
        (m, "/api/state") | (m, "/api/cmd") if m == "GET" || m == "POST" => {
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
            "manual": c.manual_remaining(),
            "phrase": c.rt.phrase,
            "look": c.rt.auto_key,
            "energy": c.rt.energy,
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

/// 操作(パネルのボタン)。{"cmd": "...", ...}
pub fn command(c: &mut Core, v: &Value) -> Result<(), String> {
    let cmd = v["cmd"].as_str().ok_or("cmd がありません")?;
    let on = v["on"].as_bool();
    let num = v["value"].as_f64().map(|x| x as f32);
    let text = |k: &str| v[k].as_str().map(str::to_string).ok_or(format!("{k} がありません"));
    match cmd {
        "blackout" => c.rt.blackout = on.unwrap_or(!c.rt.blackout),
        "master" => c.rt.master = num.ok_or("value がありません")?.clamp(0.0, 1.0),
        "fade" => {
            let dark = on.unwrap_or(c.rt.dim > 0.5);
            let secs = c.project.fade_secs;
            c.fade_to_black(dark, secs);
        }
        // 押している間だけ(ボタンを離したら on: false が来る)
        "flash" => c.set_flash(on.unwrap_or(false).then_some(true)),
        "tap" => c.tap(),
        "resync" => c.resync(),
        "bpm" => c.set_bpm(num.ok_or("value がありません")?),
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
            let n = NamedColor::ALL.into_iter().find(|n| format!("{n:?}") == name).ok_or("その色はありません")?;
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
