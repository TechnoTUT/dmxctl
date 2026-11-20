//! 操作パネルのサーバーのテスト。`/api/events` が様子の変化を送り続けることを確かめる

use crate::web::WebServer;
use dmxctl_core::engine::*;
use dmxctl_core::fixture::Library;
use dmxctl_core::project::*;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn shared(pin: &str) -> Shared {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    let mut core = Core::new(Project::starter(&lib), lib);
    core.project.web_pin = pin.into();
    Arc::new(std::sync::Mutex::new(core))
}

fn open(port: u16, pin: Option<&str>) -> TcpStream {
    let mut s = TcpStream::connect(("127.0.0.1", port)).expect("接続できない");
    s.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let pin = pin.map(|p| format!("X-Pin: {p}\r\n")).unwrap_or_default();
    write!(s, "GET /api/events HTTP/1.1\r\nHost: x\r\n{pin}\r\n").unwrap();
    s
}

/// `needle` が届くまで読む。届いたところまでの文字列を返す
fn read_until(s: &mut TcpStream, got: &mut String, needle: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline {
        if got.contains(needle) {
            return true;
        }
        match s.read(&mut buf) {
            Ok(0) => return got.contains(needle),
            Ok(n) => got.push_str(&String::from_utf8_lossy(&buf[..n])),
            Err(_) => {}
        }
    }
    got.contains(needle)
}

fn port(offset: u16) -> u16 {
    39500 + offset + (std::process::id() % 200) as u16
}

#[test]
fn events_push_state_changes() {
    let core = shared("");
    let port = port(0);
    let _server = WebServer::start(core.clone(), None, port).expect("待ち受けできない");
    let mut s = open(port, None);
    let mut got = String::new();
    assert!(read_until(&mut s, &mut got, "event: state"), "最初の様子が届かない: {got}");
    assert!(got.starts_with("HTTP/1.1 200 OK"));
    assert!(got.contains("text/event-stream"));
    // 様子を変えると、次の `event: state` で届く
    lock(&core).rt.master = 0.25;
    got.clear();
    assert!(read_until(&mut s, &mut got, "\"master\":0.25"), "変化が届かない: {got}");
}

#[test]
fn events_need_the_pin() {
    let core = shared("secret");
    let port = port(1);
    let _server = WebServer::start(core, None, port).expect("待ち受けできない");
    let mut denied = open(port, Some("wrong"));
    let mut got = String::new();
    read_until(&mut denied, &mut got, "401");
    assert!(got.starts_with("HTTP/1.1 401"), "{got}");
    let mut allowed = open(port, Some("secret"));
    let mut got = String::new();
    assert!(read_until(&mut allowed, &mut got, "event: state"), "{got}");
}
