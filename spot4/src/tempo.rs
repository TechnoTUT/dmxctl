//! テンポ(BPM)と拍の位置。時刻はアプリ起動からの秒数(f64)で扱う。

pub const MIN_BPM: f64 = 60.0;
pub const MAX_BPM: f64 = 200.0;
/// これより間が空いたタップは、新しいタップの始まりとみなす
const TAP_RESET_SECS: f64 = 2.0;
const TAP_KEEP: usize = 8;

#[derive(Clone, Debug)]
pub struct Clock {
    bpm: f64,
    /// 拍0の時刻
    origin: f64,
    taps: Vec<f64>,
}

impl Clock {
    pub fn new(bpm: f64) -> Self {
        Clock { bpm: bpm.clamp(MIN_BPM, MAX_BPM), origin: 0.0, taps: Vec::new() }
    }

    pub fn bpm(&self) -> f64 {
        self.bpm
    }

    /// 時刻 t の拍位置(1拍=1.0、4拍=1小節)
    pub fn beat(&self, t: f64) -> f64 {
        (t - self.origin) * self.bpm / 60.0
    }

    /// BPMを変える。今の拍位置は変えない(途中で飛ばない)
    pub fn set_bpm(&mut self, bpm: f64, t: f64) {
        let b = self.beat(t);
        self.bpm = bpm.clamp(MIN_BPM, MAX_BPM);
        self.origin = t - b * 60.0 / self.bpm;
    }

    /// タップ。2回目からBPMを決め、タップした瞬間を拍の頭に合わせる
    pub fn tap(&mut self, t: f64) {
        if self.taps.last().is_some_and(|&l| t - l > TAP_RESET_SECS || t < l) {
            self.taps.clear();
        }
        self.taps.push(t);
        if self.taps.len() > TAP_KEEP {
            self.taps.remove(0);
        }
        if self.taps.len() >= 2 {
            let span = self.taps[self.taps.len() - 1] - self.taps[0];
            let bpm = 60.0 * (self.taps.len() - 1) as f64 / span;
            self.bpm = bpm.clamp(MIN_BPM, MAX_BPM);
        }
        let n = self.beat(t).round();
        self.origin = t - n * 60.0 / self.bpm;
    }

    /// 今この瞬間を小節の頭(1拍目)にする
    pub fn downbeat(&mut self, t: f64) {
        let b = self.beat(t);
        let n = (b / 4.0).floor() * 4.0 + 4.0;
        self.origin = t - n * 60.0 / self.bpm;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_sets_bpm_and_phase() {
        let mut c = Clock::new(120.0);
        for i in 0..4 {
            c.tap(10.0 + i as f64 * 0.5); // 120BPM
        }
        assert!((c.bpm() - 120.0).abs() < 1e-9);
        let mut c = Clock::new(120.0);
        for i in 0..5 {
            c.tap(3.0 + i as f64 * 60.0 / 128.0);
        }
        assert!((c.bpm() - 128.0).abs() < 1e-6);
        // 最後のタップが拍の頭
        let t = 3.0 + 4.0 * 60.0 / 128.0;
        assert!(c.beat(t).fract().abs() < 1e-6);
    }

    #[test]
    fn long_gap_restarts_tapping() {
        let mut c = Clock::new(120.0);
        c.tap(0.0);
        c.tap(0.5);
        c.tap(10.0); // 間が空いた: BPMは変わらない
        assert!((c.bpm() - 120.0).abs() < 1e-9);
        c.tap(10.4);
        assert!((c.bpm() - 150.0).abs() < 1e-6);
    }

    #[test]
    fn set_bpm_keeps_position() {
        let mut c = Clock::new(120.0);
        let b = c.beat(7.3);
        c.set_bpm(140.0, 7.3);
        assert!((c.beat(7.3) - b).abs() < 1e-9);
        c.set_bpm(1000.0, 7.3);
        assert_eq!(c.bpm(), MAX_BPM);
    }

    #[test]
    fn downbeat_starts_a_bar() {
        let mut c = Clock::new(128.0);
        let b0 = c.beat(5.55);
        c.downbeat(5.55);
        let b = c.beat(5.55);
        assert!((b / 4.0 - (b / 4.0).round()).abs() < 1e-9);
        assert!(b > b0);
    }
}
