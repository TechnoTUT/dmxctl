//! 出力先(Art-Net / USB-DMX)。

use crate::opendmx::{LinkKind, OpenDmx};
use crate::project::{OutputCfg, OutputKind};
use std::net::{SocketAddr, UdpSocket};

pub trait Output: Send {
    /// `len`は実際に使っているスロット数
    fn send(&mut self, frame: &[u8; 512], len: usize);
    fn status(&self) -> String;
}

pub fn create(cfg: &OutputCfg) -> Box<dyn Output> {
    if !cfg.enabled {
        return Box::new(Disabled);
    }
    match &cfg.kind {
        OutputKind::ArtNet { ip, art_universe } => match ArtNet::new(ip, *art_universe) {
            Ok(a) => Box::new(a),
            Err(e) => Box::new(Failed(e)),
        },
        OutputKind::OpenDmxSerial { port } => Box::new(OpenDmx::new(LinkKind::Serial(port.clone()))),
        OutputKind::OpenDmxD2xx { index } => Box::new(OpenDmx::new(LinkKind::D2xx(*index))),
    }
}

struct Disabled;
impl Output for Disabled {
    fn send(&mut self, _: &[u8; 512], _: usize) {}
    fn status(&self) -> String {
        "無効".into()
    }
}

struct Failed(String);
impl Output for Failed {
    fn send(&mut self, _: &[u8; 512], _: usize) {}
    fn status(&self) -> String {
        format!("エラー: {}", self.0)
    }
}

pub struct ArtNet {
    sock: UdpSocket,
    dest: SocketAddr,
    art_universe: u16,
    seq: u8,
    error: Option<String>,
    dest_label: String,
}

impl ArtNet {
    pub fn new(ip: &str, art_universe: u16) -> Result<Self, String> {
        let dest: SocketAddr = format!("{}:6454", ip.trim()).parse().map_err(|_| format!("IPアドレスが不正です: {ip}"))?;
        let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        sock.set_broadcast(true).map_err(|e| e.to_string())?;
        sock.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(ArtNet { sock, dest, art_universe, seq: 0, error: None, dest_label: ip.trim().to_string() })
    }
}

impl Output for ArtNet {
    fn send(&mut self, frame: &[u8; 512], _len: usize) {
        // 0はシーケンス無効の意味なので1..=255で回す
        self.seq = if self.seq == 255 { 1 } else { self.seq + 1 };
        let pkt = artdmx_packet(self.seq, self.art_universe, frame);
        self.error = self.sock.send_to(&pkt, self.dest).err().map(|e| e.to_string());
    }
    fn status(&self) -> String {
        match &self.error {
            None => format!("送信中 → {} (Art-Net Univ {})", self.dest_label, self.art_universe),
            Some(e) => format!("送信エラー: {e}"),
        }
    }
}

/// ArtDMXパケット。`data`は2..=512バイト(偶数)
pub fn artdmx_packet(seq: u8, universe: u16, data: &[u8]) -> Vec<u8> {
    let len = data.len();
    let mut p = Vec::with_capacity(18 + len);
    p.extend_from_slice(b"Art-Net\0");
    p.extend_from_slice(&[0x00, 0x50]); // OpDmx (0x5000, リトルエンディアン)
    p.extend_from_slice(&[0x00, 14]); // プロトコルバージョン
    p.push(seq);
    p.push(0); // Physical
    p.push((universe & 0xFF) as u8); // SubUni
    p.push(((universe >> 8) & 0x7F) as u8); // Net
    p.push((len >> 8) as u8);
    p.push((len & 0xFF) as u8);
    p.extend_from_slice(data);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artdmx_layout() {
        let data = [1u8, 2, 3, 4];
        let p = artdmx_packet(7, 0x0102, &data);
        assert_eq!(&p[..8], b"Art-Net\0");
        assert_eq!(&p[8..10], &[0x00, 0x50]);
        assert_eq!(&p[10..12], &[0x00, 14]);
        assert_eq!(p[12], 7);
        assert_eq!(p[14], 0x02); // SubUni
        assert_eq!(p[15], 0x01); // Net
        assert_eq!(&p[16..18], &[0, 4]);
        assert_eq!(&p[18..], &data);
    }

    #[test]
    fn artnet_loopback_receives_frame() {
        let rx = UdpSocket::bind("127.0.0.1:6454");
        // 他のプロセスがポートを使っていればスキップ
        let Ok(rx) = rx else { return };
        rx.set_read_timeout(Some(std::time::Duration::from_secs(1))).unwrap();
        let mut a = ArtNet::new("127.0.0.1", 3).unwrap();
        let mut f = [0u8; 512];
        f[0] = 255;
        f[9] = 42;
        a.send(&f, 24);
        let mut buf = [0u8; 600];
        let (n, _) = rx.recv_from(&mut buf).unwrap();
        assert_eq!(n, 18 + 512);
        assert_eq!(buf[14], 3);
        assert_eq!(buf[18], 255);
        assert_eq!(buf[18 + 9], 42);
    }
}
