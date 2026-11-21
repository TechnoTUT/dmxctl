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

/// ポート 0 で待ち受け、割り当てられたポートで接続する。固定のポートだと、ほかのプロセスと重なって失敗することがある
fn start(core: Shared) -> WebServer {
    WebServer::start(core, None, 0).expect("待ち受けできない")
}

#[test]
fn events_push_state_changes() {
    let core = shared("");
    let server = start(core.clone());
    let mut s = open(server.port, None);
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
    let server = start(core);
    let port = server.port;
    let mut denied = open(port, Some("wrong"));
    let mut got = String::new();
    read_until(&mut denied, &mut got, "401");
    assert!(got.starts_with("HTTP/1.1 401"), "{got}");
    let mut allowed = open(port, Some("secret"));
    let mut got = String::new();
    assert!(read_until(&mut allowed, &mut got, "event: state"), "{got}");
}

#[test]
fn events_refuse_more_than_the_limit() {
    let server = start(shared(""));
    // 上限まではつながり、それを超えると 503 で断られる
    let mut open_streams: Vec<TcpStream> = Vec::new();
    for _ in 0..16 {
        let mut s = open(server.port, None);
        let mut got = String::new();
        assert!(read_until(&mut s, &mut got, "event: state"), "{got}");
        open_streams.push(s);
    }
    let mut over = open(server.port, None);
    let mut got = String::new();
    read_until(&mut over, &mut got, "503");
    assert!(got.starts_with("HTTP/1.1 503"), "{got}");
}

#[test]
fn events_are_quiet_while_nothing_changes() {
    let server = start(shared(""));
    let mut s = open(server.port, None);
    let mut got = String::new();
    assert!(read_until(&mut s, &mut got, "event: state"));
    // 変化が無ければ、続けて様子は届かない
    got.clear();
    let until = Instant::now() + Duration::from_millis(1500);
    let mut buf = [0u8; 4096];
    while Instant::now() < until {
        if let Ok(n) = s.read(&mut buf) {
            got.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    }
    assert!(!got.contains("event: state"), "変化が無いのに送られた: {got}");
}
