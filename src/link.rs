//! Ableton Link(映像ソフトやDAWとテンポ・拍を共有する)。本体は公式のC++実装(native/link_shim.cpp 経由)。

use std::ffi::c_void;

unsafe extern "C" {
    fn dmx_link_new(bpm: f64) -> *mut c_void;
    fn dmx_link_free(p: *mut c_void);
    fn dmx_link_enable(p: *mut c_void, on: bool);
    fn dmx_link_peers(p: *mut c_void) -> u64;
    fn dmx_link_state(p: *mut c_void, quantum: f64, tempo: *mut f64, beat: *mut f64) -> bool;
    fn dmx_link_set_tempo(p: *mut c_void, bpm: f64);
    fn dmx_link_force_beat(p: *mut c_void, beat: f64, quantum: f64);
}

/// 有効にしている間、同じネットワーク(同じPCを含む)のLink対応アプリとテンポ・拍の位置が揃う
pub struct Link {
    p: *mut c_void,
}

// Link本体はスレッドセーフ(アプリ用の関数だけを使っている)
unsafe impl Send for Link {}
unsafe impl Sync for Link {}

impl Link {
    pub fn start(bpm: f64) -> Option<Link> {
        let p = unsafe { dmx_link_new(bpm) };
        if p.is_null() {
            return None;
        }
        unsafe { dmx_link_enable(p, true) };
        Some(Link { p })
    }

    pub fn peers(&self) -> u64 {
        unsafe { dmx_link_peers(self.p) }
    }

    /// 今のテンポ(BPM)と拍の位置
    pub fn state(&self, quantum: f64) -> Option<(f64, f64)> {
        let (mut t, mut b) = (0.0, 0.0);
        unsafe { dmx_link_state(self.p, quantum, &mut t, &mut b) }.then_some((t, b))
    }

    pub fn set_tempo(&self, bpm: f64) {
        unsafe { dmx_link_set_tempo(self.p, bpm) }
    }

    pub fn force_beat(&self, beat: f64, quantum: f64) {
        unsafe { dmx_link_force_beat(self.p, beat, quantum) }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        unsafe {
            dmx_link_enable(self.p, false);
            dmx_link_free(self.p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 別プロセスのLink参加者として動く(起動中のアプリとの連携確認用)。
    /// 実行: set LINK_PROBE_BPM=133 && cargo test --release link_probe -- --ignored --nocapture
    #[test]
    #[ignore]
    fn link_probe() {
        let l = Link::start(120.0).unwrap();
        let t = std::time::Instant::now();
        while l.peers() == 0 && t.elapsed().as_secs() < 10 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let (tempo, beat) = l.state(4.0).unwrap();
        println!("[probe] peers={} tempo={tempo:.2} beat={beat:.2}", l.peers());
        if let Some(bpm) = std::env::var("LINK_PROBE_BPM").ok().and_then(|s| s.parse::<f64>().ok()) {
            l.set_tempo(bpm);
            println!("[probe] set tempo {bpm}");
        }
        std::thread::sleep(std::time::Duration::from_secs(3));
        let (tempo, _) = l.state(4.0).unwrap();
        println!("[probe] after: peers={} tempo={tempo:.2}", l.peers());
    }

    #[test]
    fn link_runs_and_follows_tempo_changes() {
        let l = Link::start(120.0).expect("Linkを起動できない");
        let (t, b0) = l.state(4.0).unwrap();
        assert!((t - 120.0).abs() < 1e-6);
        std::thread::sleep(std::time::Duration::from_millis(250));
        let (_, b1) = l.state(4.0).unwrap();
        // 120BPM で 0.25秒 = 0.5拍 進む
        assert!((b1 - b0 - 0.5).abs() < 0.1, "{b0} -> {b1}");
        l.set_tempo(90.0);
        assert!((l.state(4.0).unwrap().0 - 90.0).abs() < 1e-6);
        l.force_beat(0.0, 4.0);
        let (_, b) = l.state(4.0).unwrap();
        assert!(b.abs() < 0.05, "頭出し後の拍 {b}");
    }
}
