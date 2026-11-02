//! 音声からの自動BPM検出。
//!
//! 音声入力(ミキサーのライン出力・オーディオインターフェース・マイク)を受け取り、
//! 低音寄りのオンセット(音の立ち上がり)の強さを5msごとに作り、自己相関でテンポ、
//! 拍の山の位置で位相(拍のタイミング)を求める。結果は `Core::audio_report` に渡す。

use crate::engine::{lock, Shared};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// オンセット列の間隔(秒)
const HOP_SECS: f64 = 0.005;
/// 解析に使う長さ(秒)
const WINDOW_SECS: f64 = 8.0;
/// 検出するBPMの範囲(この範囲に倍/半分で折り返す)
pub const DETECT_MIN: f64 = 78.0;
pub const DETECT_MAX: f64 = 170.0;
/// 解析結果を出す間隔(秒)
const REPORT_SECS: f64 = 0.5;
/// 解析を始めるのに必要な長さ(秒)
const WARMUP_SECS: f64 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Estimate {
    pub bpm: f32,
    /// 最後の拍から今までの経過(拍単位 0.0〜1.0)
    pub phase: f32,
    /// 確からしさ 0.0〜1.0
    pub confidence: f32,
}

/// 音声(モノラル)を流し込むと、テンポと拍の位置を推定する
pub struct Analyzer {
    hop: usize,
    sr: f64,
    // 低域通過(キック・ベース寄り)と、全域の差分をあわせてオンセットにする
    lp: f32,
    lp_alpha: f32,
    prev_hp: f32,
    acc_low: f32,
    acc_full: f32,
    acc_n: usize,
    prev_low: f32,
    prev_full: f32,
    /// オンセットの強さ(古い順)
    env: std::collections::VecDeque<f32>,
    max_len: usize,
    since_report: usize,
    pub level: f32,
}

impl Analyzer {
    pub fn new(sample_rate: f64) -> Analyzer {
        let hop = (sample_rate * HOP_SECS).round().max(1.0) as usize;
        let cutoff = 150.0;
        let alpha = (1.0 - (-2.0 * std::f64::consts::PI * cutoff / sample_rate).exp()) as f32;
        Analyzer {
            hop,
            sr: sample_rate,
            lp: 0.0,
            lp_alpha: alpha,
            prev_hp: 0.0,
            acc_low: 0.0,
            acc_full: 0.0,
            acc_n: 0,
            prev_low: 0.0,
            prev_full: 0.0,
            env: Default::default(),
            max_len: (WINDOW_SECS / HOP_SECS) as usize,
            since_report: 0,
            level: 0.0,
        }
    }

    fn hop_secs(&self) -> f64 {
        self.hop as f64 / self.sr
    }

    /// モノラルのサンプルを流し込む。解析結果が出る間隔ごとに Some を返す
    pub fn push(&mut self, samples: &[f32]) -> Option<Estimate> {
        let mut out = None;
        for &x in samples {
            self.lp += self.lp_alpha * (x - self.lp);
            // 高域は一次の高域通過で粗く拾う
            let hp = x - self.lp;
            let d = hp - self.prev_hp;
            self.prev_hp = hp;
            self.acc_low += self.lp * self.lp;
            self.acc_full += d * d;
            self.acc_n += 1;
            self.level = self.level.max(x.abs());
            if self.acc_n >= self.hop {
                let n = self.acc_n as f32;
                let low = (self.acc_low / n).sqrt();
                let full = (self.acc_full / n).sqrt();
                self.acc_low = 0.0;
                self.acc_full = 0.0;
                self.acc_n = 0;
                // 対数にして、音量の大小に左右されにくくする
                let l = (1.0 + 50.0 * low).ln();
                let f = (1.0 + 20.0 * full).ln();
                let onset = (l - self.prev_low).max(0.0) * 1.0 + (f - self.prev_full).max(0.0) * 0.5;
                self.prev_low = l;
                self.prev_full = f;
                self.env.push_back(onset);
                if self.env.len() > self.max_len {
                    self.env.pop_front();
                }
                self.since_report += 1;
                if self.since_report as f64 * self.hop_secs() >= REPORT_SECS {
                    self.since_report = 0;
                    out = self.estimate();
                }
            }
        }
        out
    }

    fn estimate(&self) -> Option<Estimate> {
        let hs = self.hop_secs();
        if (self.env.len() as f64) * hs < WARMUP_SECS {
            return None;
        }
        let e: Vec<f32> = self.env.iter().copied().collect();
        let mean = e.iter().sum::<f32>() / e.len() as f32;
        let x: Vec<f32> = e.iter().map(|v| v - mean).collect();
        let energy: f32 = x.iter().map(|v| v * v).sum();
        if energy < 1e-4 {
            return None;
        }
        let n = x.len();
        let ac = |lag: f64| -> f32 {
            // 小数のラグは線形補間
            let l0 = lag.floor() as usize;
            let fr = (lag - l0 as f64) as f32;
            let at = |l: usize| -> f32 {
                if l >= n {
                    return 0.0;
                }
                let mut s = 0.0;
                for i in 0..n - l {
                    s += x[i] * x[i + l];
                }
                s / (n - l) as f32
            };
            at(l0) * (1.0 - fr) + at(l0 + 1) * fr
        };
        let r0 = ac(0.0).max(1e-9);
        let lag_min = 60.0 / DETECT_MAX / hs;
        let lag_max = 60.0 / DETECT_MIN / hs;
        // 候補ごとに、拍・2拍・4拍ぶん離れた位置の相関を合わせて評価する(倍/半分の取り違えを減らす)
        let mut best: Option<(f64, f32)> = None;
        let mut lag = lag_min;
        let step = 0.25;
        let mut scores = Vec::new();
        while lag <= lag_max {
            let bpm = 60.0 / (lag * hs);
            // 120〜130付近を少し優先(DJの曲に多い)
            let prior = (-0.5 * ((bpm / 125.0).ln() / 0.35).powi(2)).exp() as f32;
            let s = (ac(lag) + 0.5 * ac(lag * 2.0) + 0.25 * ac(lag * 4.0)) / r0 * (0.6 + 0.4 * prior);
            scores.push((lag, s));
            if best.is_none_or(|(_, b)| s > b) {
                best = Some((lag, s));
            }
            lag += step;
        }
        let (blag, bscore) = best?;
        if bscore <= 0.0 {
            return None;
        }
        // 山の鋭さ(平均との比)を確からしさにする
        let avg = scores.iter().map(|(_, s)| *s).sum::<f32>() / scores.len() as f32;
        let confidence = ((bscore - avg) / bscore.max(1e-6)).clamp(0.0, 1.0) * (bscore * 2.0).min(1.0);
        let period = blag; // hop単位
        // 位相: 周期ごとの山の合計が最大になる位置
        let steps = 48;
        let mut best_ph = (0.0f64, f32::MIN);
        for k in 0..steps {
            let off = period * k as f64 / steps as f64;
            let mut sum = 0.0;
            let mut t = off;
            while t < n as f64 - 1.0 {
                let i = t.floor() as usize;
                let fr = (t - i as f64) as f32;
                // 少し幅を持たせて拾う
                let v = e[i] * (1.0 - fr) + e[i + 1] * fr;
                sum += v;
                t += period;
            }
            if sum > best_ph.1 {
                best_ph = (off, sum);
            }
        }
        // 最後の山の位置(末尾から見て、最も新しい拍)
        let last_idx = n as f64 - 1.0;
        let k = ((last_idx - best_ph.0) / period).floor();
        let last_beat = best_ph.0 + k * period;
        let since = (last_idx - last_beat) / period;
        Some(Estimate { bpm: (60.0 / (period * hs)) as f32, phase: since.clamp(0.0, 1.0) as f32, confidence })
    }
}

/// 音声入力の機器名の一覧
pub fn input_devices() -> Vec<String> {
    let host = cpal::default_host();
    host.input_devices().map(|it| it.filter_map(|d| d.description().ok().map(|x| x.name().to_string())).collect()).unwrap_or_default()
}

pub struct AudioService {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
    pub device: String,
}

impl Drop for AudioService {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl AudioService {
    /// `device`が空なら既定の入力。streamはこのスレッドの中で作る(スレッドをまたがせないため)
    pub fn start(core: Shared, device: &str) -> AudioService {
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let dev_name = device.to_string();
        let handle = std::thread::Builder::new()
            .name("audio-in".into())
            .spawn(move || run(core, &dev_name, s2))
            .ok();
        AudioService { stop, handle, device: device.to_string() }
    }
}

fn run(core: Shared, device: &str, stop: Arc<AtomicBool>) {
    let set_status = |s: String| lock(&core).rt.audio_status = s;
    let host = cpal::default_host();
    let dev = if device.is_empty() {
        host.default_input_device()
    } else {
        host.input_devices().ok().and_then(|mut it| it.find(|d| d.description().ok().is_some_and(|x| x.name() == device)))
    };
    let Some(dev) = dev else {
        set_status("音声入力が見つかりません".into());
        return;
    };
    let cfg = match dev.default_input_config() {
        Ok(c) => c,
        Err(e) => {
            set_status(format!("音声入力を開けません: {e}"));
            return;
        }
    };
    let sr = cfg.sample_rate() as f64;
    let ch = cfg.channels() as usize;
    let analyzer = Arc::new(std::sync::Mutex::new(Analyzer::new(sr)));
    let (tx, rx) = std::sync::mpsc::channel::<Estimate>();
    let level = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let (a2, l2) = (analyzer.clone(), level.clone());
    let feed = move |mono: &[f32]| {
        let mut a = a2.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(est) = a.push(mono) {
            let _ = tx.send(est);
        }
        l2.store(a.level.to_bits(), Ordering::Relaxed);
    };
    let err_fn = |_e| {};
    let stream_cfg: cpal::StreamConfig = cfg.clone().into();
    macro_rules! build {
        ($t:ty, $conv:expr) => {{
            let feed = feed.clone();
            let mut buf: Vec<f32> = Vec::new();
            dev.build_input_stream(
                stream_cfg.clone(),
                move |data: &[$t], _| {
                    buf.clear();
                    for frame in data.chunks(ch.max(1)) {
                        let s: f32 = frame.iter().map(|&v| $conv(v)).sum::<f32>() / frame.len() as f32;
                        buf.push(s);
                    }
                    feed(&buf);
                },
                err_fn,
                None,
            )
        }};
    }
    let stream = match cfg.sample_format() {
        cpal::SampleFormat::F32 => build!(f32, |v: f32| v),
        cpal::SampleFormat::I16 => build!(i16, |v: i16| v as f32 / 32768.0),
        cpal::SampleFormat::U16 => build!(u16, |v: u16| (v as f32 - 32768.0) / 32768.0),
        cpal::SampleFormat::I32 => build!(i32, |v: i32| v as f32 / 2147483648.0),
        f => {
            set_status(format!("未対応の音声形式です: {f:?}"));
            return;
        }
    };
    let stream = match stream {
        Ok(s) => s,
        Err(e) => {
            set_status(format!("音声入力を開けません: {e}"));
            return;
        }
    };
    if let Err(e) = stream.play() {
        set_status(format!("音声入力を開始できません: {e}"));
        return;
    }
    set_status("音を聞いています…".into());
    let mut last_est = Instant::now();
    while !stop.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(est) => {
                last_est = Instant::now();
                lock(&core).audio_report(est, Instant::now());
            }
            Err(_) => {
                let lv = f32::from_bits(level.load(Ordering::Relaxed));
                if last_est.elapsed() > Duration::from_secs(3) {
                    let msg = if lv < 0.002 { "音が入ってきていません" } else { "音を聞いています…" };
                    lock(&core).rt.audio_status = msg.into();
                }
            }
        }
    }
    drop(stream);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// bpmで刻むキック(低い短いバースト)を作る
    fn kicks(sr: f64, bpm: f64, secs: f64, offset: f64) -> Vec<f32> {
        let n = (sr * secs) as usize;
        let period = sr * 60.0 / bpm;
        let mut out = vec![0.0f32; n];
        let mut t = offset * sr;
        while (t as usize) < n {
            for i in 0..(sr * 0.08) as usize {
                let idx = t as usize + i;
                if idx >= n {
                    break;
                }
                let env = (-(i as f64) / (sr * 0.02)).exp();
                out[idx] += (env * (2.0 * std::f64::consts::PI * 60.0 * i as f64 / sr).sin()) as f32 * 0.8;
            }
            t += period;
        }
        out
    }

    fn detect(bpm: f64) -> Estimate {
        let sr = 44100.0;
        let sig = kicks(sr, bpm, 14.0, 0.13);
        let mut a = Analyzer::new(sr);
        let mut last = None;
        for ch in sig.chunks(512) {
            if let Some(e) = a.push(ch) {
                last = Some(e);
            }
        }
        last.expect("推定が出る")
    }

    #[test]
    fn detects_common_tempos() {
        for bpm in [90.0, 100.0, 120.0, 128.0, 140.0, 150.0] {
            let e = detect(bpm);
            assert!((e.bpm as f64 - bpm).abs() < 1.5, "{bpm}BPMのはずが{}", e.bpm);
            assert!(e.confidence > 0.2, "確からしさが低い: {}", e.confidence);
        }
    }

    #[test]
    fn silence_gives_no_estimate() {
        let mut a = Analyzer::new(44100.0);
        assert!(a.push(&vec![0.0; 44100 * 10]).is_none());
    }

    #[test]
    fn phase_points_to_last_kick() {
        // 128BPM、最初のキックが0.13秒。解析の末尾(14秒)から最後のキックまでの拍数
        let e = detect(128.0);
        let period: f64 = 60.0 / 128.0;
        let last_kick = 0.13 + ((14.0 - 0.13) / period).floor() * period;
        let want = ((14.0 - last_kick) / period) as f32;
        let d = (e.phase - want).abs().min(1.0 - (e.phase - want).abs());
        assert!(d < 0.12, "位相 {} 期待 {}", e.phase, want);
    }
}
