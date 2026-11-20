//! OSC のメッセージの組み立てと解釈、UDP の受信。
//! 受けたメッセージをどう動作に変えるかは `dmxctl-core` の `io::osc` が受け持つ。

use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
}

impl Arg {
    pub fn num(&self) -> Option<f32> {
        match self {
            Arg::Int(i) => Some(*i as f32),
            Arg::Float(f) => Some(*f as f32),
            Arg::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Arg::Str(_) => None,
        }
    }
    pub fn text(&self) -> Option<&str> {
        match self {
            Arg::Str(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Message {
    pub addr: String,
    pub args: Vec<Arg>,
}

// ------------------------------------------------------------ 解析

fn read_str(b: &[u8], pos: &mut usize) -> Option<String> {
    let start = *pos;
    let end = start + b.get(start..)?.iter().position(|&c| c == 0)?;
    let s = String::from_utf8_lossy(&b[start..end]).into_owned();
    // 終端の0を含めて4バイト単位
    *pos = (end + 4) & !3;
    Some(s)
}

fn read_bytes<const N: usize>(b: &[u8], pos: &mut usize) -> Option<[u8; N]> {
    let v: [u8; N] = b.get(*pos..*pos + N)?.try_into().ok()?;
    *pos += N;
    Some(v)
}

/// OSCパケット(メッセージ、またはバンドル)を読む。読めないものは空
pub fn parse(b: &[u8]) -> Vec<Message> {
    let mut out = Vec::new();
    parse_into(b, &mut out, 0);
    out
}

fn parse_into(b: &[u8], out: &mut Vec<Message>, depth: u32) {
    if depth > 8 {
        return;
    }
    if b.starts_with(b"#bundle\0") {
        let mut pos = 16; // "#bundle\0" + タイムタグ(8バイト)
        while let Some(len) = read_bytes::<4>(b, &mut pos).map(i32::from_be_bytes) {
            let Ok(len) = usize::try_from(len) else { return };
            let Some(el) = b.get(pos..pos + len) else { return };
            parse_into(el, out, depth + 1);
            pos += len;
        }
        return;
    }
    if let Some(m) = parse_message(b) {
        out.push(m);
    }
}

fn parse_message(b: &[u8]) -> Option<Message> {
    let mut pos = 0;
    let addr = read_str(b, &mut pos)?;
    if !addr.starts_with('/') {
        return None;
    }
    let mut args = Vec::new();
    // 型タグが無い古い送信元もある
    if pos >= b.len() {
        return Some(Message { addr, args });
    }
    let tags = read_str(b, &mut pos)?;
    for t in tags.strip_prefix(',')?.chars() {
        let a = match t {
            'i' => Arg::Int(i32::from_be_bytes(read_bytes(b, &mut pos)?) as i64),
            'h' => Arg::Int(i64::from_be_bytes(read_bytes(b, &mut pos)?)),
            'f' => Arg::Float(f32::from_be_bytes(read_bytes(b, &mut pos)?) as f64),
            'd' => Arg::Float(f64::from_be_bytes(read_bytes(b, &mut pos)?)),
            's' | 'S' => Arg::Str(read_str(b, &mut pos)?),
            'T' => Arg::Bool(true),
            'F' => Arg::Bool(false),
            'N' | 'I' => continue,
            'b' => {
                let n = i32::from_be_bytes(read_bytes(b, &mut pos)?).max(0) as usize;
                pos = (pos + n + 3) & !3;
                continue;
            }
            // 知らない型は、その先が読めないので打ち切る
            _ => break,
        };
        args.push(a);
    }
    Some(Message { addr, args })
}

/// テスト・送信用: 1つのメッセージを組み立てる
pub fn encode(addr: &str, args: &[Arg]) -> Vec<u8> {
    fn put_str(v: &mut Vec<u8>, s: &str) {
        v.extend_from_slice(s.as_bytes());
        v.push(0);
        while !v.len().is_multiple_of(4) {
            v.push(0);
        }
    }
    let mut v = Vec::new();
    put_str(&mut v, addr);
    let mut tags = String::from(",");
    for a in args {
        tags.push(match a {
            Arg::Int(_) => 'i',
            Arg::Float(_) => 'f',
            Arg::Str(_) => 's',
            Arg::Bool(true) => 'T',
            Arg::Bool(false) => 'F',
        });
    }
    put_str(&mut v, &tags);
    for a in args {
        match a {
            Arg::Int(i) => v.extend_from_slice(&(*i as i32).to_be_bytes()),
            Arg::Float(f) => v.extend_from_slice(&(*f as f32).to_be_bytes()),
            Arg::Str(s) => put_str(&mut v, s),
            Arg::Bool(_) => {}
        }
    }
    v
}

// ------------------------------------------------------------ 受信スレッド

/// 指定のUDPポートでOSCを待ち受ける。Dropで止まる
pub struct OscListener {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    pub port: u16,
}

impl OscListener {
    /// メッセージを受けるたびに `on_receive(受けたメッセージ, ここまでに受けた件数)` を呼ぶ。
    /// 呼ぶのは受信のスレッドで、重い処理や長い待ちは入れない
    pub fn start(port: u16, mut on_receive: impl FnMut(&[Message], u64) + Send + 'static) -> Result<OscListener, String> {
        Self::start_with_port(port, move |messages, count, _| on_receive(messages, count))
    }

    /// ポートに 0 を指定すると OS が選び、受信時に実際のポートを通知する
    pub fn start_with_port(port: u16, mut on_receive: impl FnMut(&[Message], u64, u16) + Send + 'static) -> Result<OscListener, String> {
        let sock = UdpSocket::bind(("0.0.0.0", port)).map_err(|e| format!("ポート{port}を開けません: {e}"))?;
        let port = sock.local_addr().map_err(|e| e.to_string())?.port();
        sock.set_read_timeout(Some(Duration::from_millis(200))).map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = std::thread::Builder::new()
            .name("osc-in".into())
            .spawn(move || {
                let mut buf = [0u8; 4096];
                let mut count: u64 = 0;
                while !s2.load(Ordering::SeqCst) {
                    let Ok((n, _)) = sock.recv_from(&mut buf) else { continue };
                    let msgs = parse(&buf[..n]);
                    if msgs.is_empty() {
                        continue;
                    }
                    count += msgs.len() as u64;
                    on_receive(&msgs, count, port);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(OscListener { stop, handle: Some(handle), port })
    }
}

impl Drop for OscListener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}
