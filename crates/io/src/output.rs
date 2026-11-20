//! 出力先(Art-Net / sACN / USB-DMX)。

use crate::opendmx::{LinkKind, OpenDmx};
use dmxctl_model::project::{OutputCfg, OutputKind};
use std::net::{Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::OnceLock;

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
        OutputKind::Sacn { ip, sacn_universe, priority } => match Sacn::new(ip, *sacn_universe, *priority) {
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

/// sACN (E1.31) の UDP ポート
pub const SACN_PORT: u16 = 5568;
/// 送り手の名前(受け手の画面に出る)
const SACN_SOURCE_NAME: &str = "dmxctl";

/// このアプリの送り手としての識別子(CID)。1つのプロセスでは、どのユニバースも同じものを使う
fn sacn_cid() -> [u8; 16] {
    static CID: OnceLock<[u8; 16]> = OnceLock::new();
    *CID.get_or_init(|| {
        use sha2::{Digest, Sha256};
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let mut h = Sha256::new();
        h.update(b"dmxctl sACN CID");
        h.update(std::process::id().to_le_bytes());
        h.update(now.as_nanos().to_le_bytes());
        let mut cid = [0u8; 16];
        cid.copy_from_slice(&h.finalize()[..16]);
        // UUID の版4(乱数)・変種の印
        cid[6] = (cid[6] & 0x0F) | 0x40;
        cid[8] = (cid[8] & 0x3F) | 0x80;
        cid
    })
}

/// sACN のマルチキャストの宛先(239.255.上位.下位)
pub fn sacn_multicast(universe: u16) -> Ipv4Addr {
    Ipv4Addr::new(239, 255, (universe >> 8) as u8, (universe & 0xFF) as u8)
}

pub struct Sacn {
    sock: UdpSocket,
    dest: SocketAddr,
    universe: u16,
    priority: u8,
    seq: u8,
    last: [u8; 512],
    error: Option<String>,
    dest_label: String,
}

impl Sacn {
    pub fn new(ip: &str, universe: u16, priority: u8) -> Result<Self, String> {
        if !(1..=63999).contains(&universe) {
            return Err(format!("sACN のユニバースは 1〜63999 です: {universe}"));
        }
        let ip = ip.trim();
        let (addr, dest_label) = if ip.is_empty() {
            let a = sacn_multicast(universe);
            (a, format!("マルチキャスト {a}"))
        } else {
            (ip.parse::<Ipv4Addr>().map_err(|_| format!("IPアドレスが不正です: {ip}"))?, ip.to_string())
        };
        let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
        sock.set_nonblocking(true).map_err(|e| e.to_string())?;
        if addr.is_broadcast() {
            sock.set_broadcast(true).map_err(|e| e.to_string())?;
        }
        Ok(Sacn { sock, dest: SocketAddr::from((addr, SACN_PORT)), universe, priority: priority.min(200), seq: 0, last: [0; 512], error: None, dest_label })
    }

    fn send_packet(&mut self, terminated: bool) {
        let pkt = sacn_packet(&sacn_cid(), SACN_SOURCE_NAME, self.priority, self.seq, terminated, self.universe, &self.last);
        self.seq = self.seq.wrapping_add(1);
        self.error = self.sock.send_to(&pkt, self.dest).err().map(|e| e.to_string());
    }
}

impl Output for Sacn {
    fn send(&mut self, frame: &[u8; 512], _len: usize) {
        self.last = *frame;
        self.send_packet(false);
    }
    fn status(&self) -> String {
        match &self.error {
            None => format!("送信中 → {} (sACN Univ {})", self.dest_label, self.universe),
            Some(e) => format!("送信エラー: {e}"),
        }
    }
}

impl Drop for Sacn {
    /// 送るのをやめたことを、受け手に知らせる(規格どおり3回)。受け手はすぐに別の送り手へ切り替えられる
    fn drop(&mut self) {
        for _ in 0..3 {
            self.send_packet(true);
        }
    }
}

/// E1.31 のデータパケット(ルート層・フレーミング層・DMP層)。`data` は 1〜512 バイト
pub fn sacn_packet(cid: &[u8; 16], source: &str, priority: u8, seq: u8, terminated: bool, universe: u16, data: &[u8]) -> Vec<u8> {
    let slots = data.len().min(512);
    let total = 126 + slots;
    let flen = |len: usize| (0x7000 | (len as u16 & 0x0FFF)).to_be_bytes();
    let mut p = Vec::with_capacity(total);
    // ルート層
    p.extend_from_slice(&[0x00, 0x10, 0x00, 0x00]); // プリアンブル・ポストアンブルの長さ
    p.extend_from_slice(b"ASC-E1.17\0\0\0");
    p.extend_from_slice(&flen(total - 16));
    p.extend_from_slice(&4u32.to_be_bytes()); // VECTOR_ROOT_E131_DATA
    p.extend_from_slice(cid);
    // フレーミング層
    p.extend_from_slice(&flen(total - 38));
    p.extend_from_slice(&2u32.to_be_bytes()); // VECTOR_E131_DATA_PACKET
    let mut name = [0u8; 64];
    let n = source.len().min(63);
    name[..n].copy_from_slice(&source.as_bytes()[..n]);
    p.extend_from_slice(&name);
    p.push(priority.min(200));
    p.extend_from_slice(&[0, 0]); // 同期のアドレス(使わない)
    p.push(seq);
    p.push(if terminated { 0x40 } else { 0 }); // オプション: Stream_Terminated
    p.extend_from_slice(&universe.to_be_bytes());
    // DMP層
    p.extend_from_slice(&flen(total - 115));
    p.push(0x02); // VECTOR_DMP_SET_PROPERTY
    p.push(0xA1); // アドレスとデータの型
    p.extend_from_slice(&[0, 0]); // 最初のアドレス
    p.extend_from_slice(&[0, 1]); // アドレスの増分
    p.extend_from_slice(&((slots + 1) as u16).to_be_bytes());
    p.push(0); // スタートコード
    p.extend_from_slice(&data[..slots]);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sacn_layout() {
        let cid = [7u8; 16];
        let mut data = [0u8; 512];
        data[0] = 255;
        data[511] = 9;
        let p = sacn_packet(&cid, "dmxctl", 100, 42, false, 0x0102, &data);
        assert_eq!(p.len(), 638);
        assert_eq!(&p[0..4], &[0x00, 0x10, 0x00, 0x00]);
        assert_eq!(&p[4..16], b"ASC-E1.17\0\0\0");
        assert_eq!(&p[16..18], &[0x72, 0x6E]); // 0x7000 | 622
        assert_eq!(&p[18..22], &[0, 0, 0, 4]);
        assert_eq!(&p[22..38], &cid);
        assert_eq!(&p[38..40], &[0x72, 0x58]); // 0x7000 | 600
        assert_eq!(&p[40..44], &[0, 0, 0, 2]);
        assert_eq!(&p[44..50], b"dmxctl");
        assert_eq!(p[50], 0);
        assert_eq!(p[108], 100); // 優先度
        assert_eq!(&p[109..111], &[0, 0]);
        assert_eq!(p[111], 42); // シーケンス
        assert_eq!(p[112], 0); // オプション
        assert_eq!(&p[113..115], &[0x01, 0x02]); // ユニバース
        assert_eq!(&p[115..117], &[0x72, 0x0B]); // 0x7000 | 523
        assert_eq!(&p[117..119], &[0x02, 0xA1]);
        assert_eq!(&p[119..123], &[0, 0, 0, 1]);
        assert_eq!(&p[123..125], &[0x02, 0x01]); // 513
        assert_eq!(p[125], 0); // スタートコード
        assert_eq!(p[126], 255);
        assert_eq!(p[637], 9);
        // 送るのをやめる合図
        assert_eq!(sacn_packet(&cid, "dmxctl", 100, 0, true, 1, &data)[112], 0x40);
    }

    #[test]
    fn sacn_destination_and_checks() {
        assert_eq!(sacn_multicast(1), Ipv4Addr::new(239, 255, 0, 1));
        assert_eq!(sacn_multicast(0x0203), Ipv4Addr::new(239, 255, 2, 3));
        assert!(Sacn::new("", 0, 100).is_err());
        assert!(Sacn::new("", 64000, 100).is_err());
        assert!(Sacn::new("nope", 1, 100).is_err());
        let s = Sacn::new("", 5, 100).unwrap();
        assert_eq!(s.dest, SocketAddr::from((Ipv4Addr::new(239, 255, 0, 5), SACN_PORT)));
        assert!(s.status().contains("239.255.0.5"));
        // 優先度は 200 までに収める
        assert_eq!(Sacn::new("127.0.0.1", 1, 255).unwrap().priority, 200);
    }

    #[test]
    fn sacn_loopback_sends_frames_and_terminates_on_drop() {
        let Ok(rx) = UdpSocket::bind("127.0.0.1:0") else { return };
        rx.set_read_timeout(Some(std::time::Duration::from_secs(1))).unwrap();
        let port = rx.local_addr().unwrap().port();
        let mut s = Sacn::new("127.0.0.1", 7, 150).unwrap();
        s.dest.set_port(port);
        let mut f = [0u8; 512];
        f[3] = 77;
        s.send(&f, 4);
        s.send(&f, 4);
        let mut buf = [0u8; 700];
        let (n, _) = rx.recv_from(&mut buf).unwrap();
        assert_eq!(n, 638);
        assert_eq!(&buf[113..115], &[0, 7]);
        assert_eq!(buf[108], 150);
        assert_eq!(buf[126 + 3], 77);
        let seq0 = buf[111];
        rx.recv_from(&mut buf).unwrap();
        assert_eq!(buf[111], seq0.wrapping_add(1), "シーケンスが進む");
        drop(s);
        for _ in 0..3 {
            rx.recv_from(&mut buf).unwrap();
            assert_eq!(buf[112], 0x40, "やめる合図");
            assert_eq!(buf[126 + 3], 77, "最後の値のまま");
        }
    }

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
