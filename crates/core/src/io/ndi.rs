//! NDI受信: 映像の平均色と、音の低音・中音・高音を、照明の色(メイン色/サブ色)に使う。
//!
//! NDIのライブラリ(libndi)はアプリに同梱せず、インストール済みのもの(NDI Tools・NDIランタイム)を実行時に読み込む。
//! 読み込みとNDIの呼び出しは moip-engine(https://github.com/TechnoTUT/moip-engine)の ndi クレートを使う。
//! 受信はこのスレッドで行い、結果(色)はCoreの`rt.ndi`に書く。照明への反映はエンジンが行う

use crate::engine::{lock, Shared};

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub use crate::project::{LiveSource, NdiCfg};

/// 受信の結果(保存しない)
#[derive(Debug, Clone, Default)]
pub struct NdiState {
    pub sources: Vec<String>,
    pub status: String,
    /// 音から作った色(0〜1)。受信していなければNone
    pub audio: Option<[f32; 3]>,
    /// 映像の平均色(0〜1)
    pub video: Option<[f32; 3]>,
}

// ---------------------------------------------------------------- NDIのライブラリ(読み込みと呼び出しは moip-engine の ndi クレート)

use moip_ndi::{Bandwidth, Capture, ColorFormat, ErrorKind, FOURCC_BGRA, FOURCC_BGRX, FOURCC_RGBA, FOURCC_RGBX, FOURCC_UYVA, FOURCC_UYVY, RecvOptions};

/// ndiクレートが探す場所(NDIランタイムの環境変数・標準の場所)より先に探す場所:
/// 直接の指定(`DMXCTL_NDI_LIB`)と、macOSでNDI Toolsのアプリに入っているもの
fn extra_lib_paths() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("DMXCTL_NDI_LIB").filter(|p| !p.is_empty()) {
        let p = PathBuf::from(p);
        v.push(if p.extension().is_some() { p } else { p.join(moip_ndi::LIBRARY_NAME) });
    }
    if cfg!(target_os = "macos") {
        for p in [
            "/Applications/NDI Virtual Input.app/Contents/Frameworks/libndi.dylib",
            "/Applications/NDI Scan Converter.app/Contents/Frameworks/libndi.dylib",
            "/Applications/NDI Video Monitor.app/Contents/Frameworks/libndi_advanced.dylib",
            "/Applications/NDI Test Patterns.app/Contents/Frameworks/libndi_advanced.dylib",
        ] {
            v.push(p.into());
        }
    }
    v
}

// ---------------------------------------------------------------- 色の計算

/// 2次のIIRフィルタ(RBJの式)
#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

impl Biquad {
    fn new(kind: u8, freq: f32, q: f32, rate: f32) -> Biquad {
        let w = std::f32::consts::TAU * freq / rate.max(1.0);
        let (s, c) = w.sin_cos();
        let alpha = s / (2.0 * q);
        let (b, a0, a) = match kind {
            // ローパス
            0 => ([(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0], 1.0 + alpha, [-2.0 * c, 1.0 - alpha]),
            // バンドパス(ピーク0dB)
            1 => ([alpha, 0.0, -alpha], 1.0 + alpha, [-2.0 * c, 1.0 - alpha]),
            // ハイパス
            _ => ([(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0], 1.0 + alpha, [-2.0 * c, 1.0 - alpha]),
        };
        Biquad { b: b.map(|x| x / a0), a: a.map(|x| x / a0), z: [0.0; 2] }
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// 音 → 色。低音(〜200Hz)=赤 / 中音(〜1.5kHz)=緑 / 高音(4kHz〜)=青。帯域ごとに自動で音量を合わせる
pub struct AudioColor {
    rate: i32,
    bands: [Biquad; 3],
    /// 帯域ごとの今の強さ(0〜1)と、自動ゲイン用の最近の最大値
    env: [f32; 3],
    peak: [f32; 3],
}

impl Default for AudioColor {
    fn default() -> Self {
        AudioColor { rate: 0, bands: [Biquad::default(); 3], env: [0.0; 3], peak: [1e-3; 3] }
    }
}

impl AudioColor {
    /// 1フレーム分(チャンネルごとに並んだサンプル)を入れて、今の色を返す
    pub fn push(&mut self, rate: i32, channels: &[&[f32]]) -> [f32; 3] {
        if rate != self.rate {
            let r = rate as f32;
            self.bands = [Biquad::new(0, 200.0, 0.707, r), Biquad::new(1, 1500.0, 0.8, r), Biquad::new(2, 4000.0, 0.707, r)];
            self.rate = rate;
        }
        let n = channels.iter().map(|c| c.len()).min().unwrap_or(0);
        if n == 0 {
            return self.env;
        }
        let mut sum = [0.0f32; 3];
        for i in 0..n {
            let x = channels.iter().map(|c| c[i]).sum::<f32>() / channels.len() as f32;
            for (b, s) in self.bands.iter_mut().zip(sum.iter_mut()) {
                let y = b.run(x);
                *s += y * y;
            }
        }
        let rms = sum.map(|s| (s / n as f32).sqrt());
        // 自動ゲイン: 帯域ごとに最近の最大値をゆっくり下げながら追う。
        // ほかの帯域よりずっと小さい帯域(漏れ込みなど)を最大にしないよう、全体の最大の3割を下限にする。無音に近いときは上げすぎない
        let global = (0..3).map(|k| (self.peak[k] * 0.995).max(rms[k])).fold(0.0f32, f32::max);
        for k in 0..3 {
            let rms = rms[k];
            self.peak[k] = (self.peak[k] * 0.995).max(rms).max(global * 0.3).max(0.002);
            let v = (rms / self.peak[k]).clamp(0.0, 1.0);
            // 立ち上がりは速く、消えるのはゆっくり
            let k2 = if v > self.env[k] { 0.6 } else { 0.15 };
            self.env[k] += (v - self.env[k]) * k2;
        }
        self.env
    }
}

/// 映像の平均色(0〜1)。全部の画素は見ずに、格子状に間引いて数える。BGRX/BGRA・RGBX/RGBA・UYVY に対応
fn average_color(w: usize, h: usize, stride: usize, fourcc: u32, data: &[u8]) -> Option<[f32; 3]> {
    let bgra = fourcc == FOURCC_BGRA || fourcc == FOURCC_BGRX;
    let rgba = fourcc == FOURCC_RGBA || fourcc == FOURCC_RGBX;
    // UYVA は先頭が UYVY と同じ並び(後ろにアルファの面が付く)
    let uyvy = fourcc == FOURCC_UYVY || fourcc == FOURCC_UYVA;
    let row_bytes = if bgra || rgba { w * 4 } else if uyvy { w.div_ceil(2) * 4 } else { return None };
    if w == 0 || h == 0 || stride < row_bytes || data.len() < stride * (h - 1) + row_bytes {
        return None;
    }
    let (sx, sy) = ((w / 64).max(1), (h / 36).max(1));
    let mut sum = [0.0f64; 3];
    let mut n = 0.0f64;
    for y in (0..h).step_by(sy) {
        let row = &data[y * stride..];
        for x in (0..w).step_by(sx) {
            let rgb = if bgra {
                let p = &row[x * 4..x * 4 + 3];
                [p[2], p[1], p[0]].map(|v| v as f64)
            } else if rgba {
                let p = &row[x * 4..x * 4 + 3];
                [p[0], p[1], p[2]].map(|v| v as f64)
            } else {
                // 2画素で U Y0 V Y1
                let p = &row[(x / 2) * 4..(x / 2) * 4 + 4];
                let (u, yy, v) = (p[0] as f64 - 128.0, p[1 + (x % 2) * 2] as f64 - 16.0, p[2] as f64 - 128.0);
                let c = 1.164 * yy;
                [c + 1.596 * v, c - 0.392 * u - 0.813 * v, c + 2.017 * u].map(|v| v.clamp(0.0, 255.0))
            };
            for k in 0..3 {
                sum[k] += rgb[k];
            }
            n += 1.0;
        }
    }
    if n == 0.0 {
        return None;
    }
    Some(sum.map(|s| (s / n / 255.0) as f32))
}

/// 平均色はくすみやすいので、彩度を上げる
fn vivid(c: [f32; 3]) -> [f32; 3] {
    let l = 0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2];
    c.map(|x| (l + (x - l) * 1.8).clamp(0.0, 1.0))
}

// ---------------------------------------------------------------- 受信スレッド

pub struct NdiService {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl NdiService {
    pub fn start(core: Shared, ctx: Option<crate::wake::Wake>) -> NdiService {
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = std::thread::Builder::new().name("ndi".into()).spawn(move || run(core, ctx, s2)).ok();
        NdiService { stop, handle }
    }
}

impl Drop for NdiService {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        // 受信をやめたら、色の元も消す
    }
}

fn set_state(core: &Shared, f: impl FnOnce(&mut NdiState)) {
    f(&mut lock(core).rt.ndi);
}

fn run(core: Shared, ctx: Option<crate::wake::Wake>, stop: Arc<AtomicBool>) {
    if let Err(e) = moip_ndi::init_with_paths(&extra_lib_paths()) {
        eprintln!("NDI: {e}");
        let msg = load_error_status(e.kind(), e.detail(), &e.to_string());
        set_state(&core, |s| s.status = msg);
        return;
    }
    // 同じPCのソース(TouchDesignerなど)は、マルチキャストの探索が使えないときでも見つかるよう、自分宛ても直接たずねる
    let finder = match moip_ndi::Finder::new(true, Some("127.0.0.1")) {
        Ok(f) => f,
        Err(_) => {
            set_state(&core, |s| s.status = "NDIのソース検索を開始できませんでした".into());
            return;
        }
    };
    let mut recv: Option<moip_ndi::Receiver> = None;
    let mut connected = String::new();
    let mut want_video = false;
    let mut audio = AudioColor::default();
    let mut last_find = Instant::now() - Duration::from_secs(10);
    let mut last_frame = Instant::now();
    // 届かないまま、この時間たったら受信し直す(送り手が止めているだけのこともあるので、やり直すたびに延ばす)
    let mut retry_after = RETRY_FIRST;
    let mut found: Vec<moip_ndi::Source> = Vec::new();
    // 届いたのに読めなかったもの(知らない映像・音の形式)。色にできないので、届いていない扱いにして理由を出す
    let mut unreadable: Option<&'static str> = None;
    set_state(&core, |s| s.status = "ソースを探しています".into());

    while !stop.load(Ordering::SeqCst) {
        // 1秒ごとにソースの一覧を更新
        if last_find.elapsed() > Duration::from_secs(1) {
            last_find = Instant::now();
            found = finder.sources();
            let names: Vec<String> = found.iter().map(|s| s.name.clone()).collect();
            set_state(&core, |s| s.sources = names);
        }
        let (source, need_video, need_any) = {
            let c = lock(&core);
            let cfg = &c.project.ndi;
            (cfg.source.clone(), cfg.main == LiveSource::Video || cfg.sub == LiveSource::Video, cfg.main != LiveSource::Off || cfg.sub != LiveSource::Off)
        };
        // つなぐソース・必要なもの(映像の有無)が変わったら、受信し直す。
        // 相手が消えた接続の後始末は何秒もかかることがあるので、別のスレッドで捨てる
        if recv.is_some() && (source != connected || need_video != want_video) {
            if let Some(r) = recv.take() {
                r.destroy_in_background();
            }
            connected.clear();
            retry_after = RETRY_FIRST;
        }
        if recv.is_none() && !source.is_empty() {
            if let Some(s) = found.iter().find(|s| s.name == source) {
                let opts = RecvOptions {
                    color_format: ColorFormat::BgrxBgra,
                    // 一番軽い映像(平均色にはこれで十分) / 音だけ
                    bandwidth: if need_video { Bandwidth::Lowest } else { Bandwidth::AudioOnly },
                    allow_video_fields: false,
                    name: Some("DMX Controller".into()),
                };
                match moip_ndi::Receiver::new(Some(s), &opts) {
                    Ok(r) => {
                        recv = Some(r);
                        connected = source.clone();
                        want_video = need_video;
                        last_frame = Instant::now();
                        set_state(&core, |st| st.status = format!("{source} に接続中"));
                    }
                    Err(_) => set_state(&core, |st| st.status = format!("{source} に接続できませんでした")),
                }
            } else {
                set_state(&core, |st| st.status = format!("{source} が見つかりません"));
            }
        } else if source.is_empty() {
            set_state(&core, |st| st.status = "ソースを選んでください".into());
        }
        let Some(r) = &recv else {
            std::thread::sleep(Duration::from_millis(200));
            continue;
        };

        match r.capture(100, want_video, true) {
            Capture::Video(f) => {
                let c = average_color(f.width(), f.height(), f.stride(), f.fourcc(), f.data()).map(vivid);
                let (w, h) = (f.width(), f.height());
                drop(f);
                // 色にできたときだけ「届いている」(読めない形式だけが届き続けても、古い色で光り続けないように)
                if c.is_some() {
                    last_frame = Instant::now();
                    retry_after = RETRY_FIRST;
                    unreadable = None;
                } else {
                    unreadable = Some("映像の形式を読めません");
                }
                if let Some(c) = c {
                    set_state(&core, |st| {
                        // 急に変わりすぎないよう少しなめらかに
                        let prev = st.video.unwrap_or(c);
                        st.video = Some([0, 1, 2].map(|k| prev[k] + (c[k] - prev[k]) * 0.35));
                        st.status = format!("{connected} を受信中 ({w}×{h})");
                    });
                }
            }
            Capture::Audio(a) => {
                let ch: Vec<&[f32]> = (0..a.channels()).map(|k| a.channel(k)).filter(|c| !c.is_empty()).collect();
                let rgb = (!ch.is_empty()).then(|| audio.push(a.sample_rate(), &ch));
                drop(a);
                if rgb.is_some() {
                    last_frame = Instant::now();
                    retry_after = RETRY_FIRST;
                    unreadable = None;
                } else {
                    unreadable = Some("音の形式を読めません");
                }
                if let Some(rgb) = rgb {
                    set_state(&core, |st| st.audio = Some(rgb));
                }
            }
            _ => {}
        }
        let silent = last_frame.elapsed();
        // 送り手とのつながりが生きているか(一時停止しているだけの送り手なら、つなぎ直さない)
        let alive = r.is_connected();
        // 2秒届かなければ、色の元を消す(古い色で光り続けないように)
        if silent > Duration::from_secs(2) {
            let why = unreadable.unwrap_or(if alive { "受信が止まっています(送り手は接続中)" } else { "受信が止まっています" });
            set_state(&core, |st| {
                st.audio = None;
                st.video = None;
                st.status = format!("{connected}: {why}");
            });
        }
        // もっと長く届かず、つながりも切れていれば、受信をつくり直す(次の周でつなぎ直す)。
        // つながっているのに届かないだけ(送り手が止めている)なら、つくり直しても変わらないので待つ
        if silent > retry_after {
            if alive {
                retry_after = (retry_after * 2).min(Duration::from_secs(30));
            } else {
                if let Some(r) = recv.take() {
                    r.destroy_in_background();
                }
                connected.clear();
                unreadable = None;
                retry_after = (retry_after * 2).min(Duration::from_secs(30));
            }
        }
        if need_any
            && let Some(c) = &ctx {
            c.request_repaint_after(Duration::from_millis(250));
        }
    }
    if let Some(r) = recv.take() {
        r.destroy_in_background();
    }
    drop(finder);
    set_state(&core, |st| {
        st.audio = None;
        st.video = None;
        st.status = "停止中".into();
    });
}

/// 読み込めなかったときに画面に出す文。ndiクレートが返すエラーの種類で分ける(エラー文の言い回しには頼らない)。
/// ライブラリはあったのに使えなかった(関数が足りない・初期化できない)ときはその理由を、見つからなかったときはインストールの案内を出す
fn load_error_status(kind: ErrorKind, detail: Option<&str>, msg: &str) -> String {
    let first_line = || msg.lines().next().unwrap_or_default();
    match kind {
        ErrorKind::RuntimeNotFound => "NDIのライブラリが見つかりません(NDI Tools をインストールしてください)".into(),
        ErrorKind::RuntimeMissingSymbol | ErrorKind::RuntimeInitFailed => {
            format!("NDIのライブラリを使えません: {}", detail.unwrap_or_else(first_line))
        }
        _ => format!("NDIのライブラリを読み込めませんでした: {}", first_line()),
    }
}

/// 届かないとき、最初に受信をつくり直すまでの時間
const RETRY_FIRST: Duration = Duration::from_secs(5);

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, rate: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (std::f32::consts::TAU * freq * i as f32 / rate).sin() * 0.5).collect()
    }

    #[test]
    fn low_mid_high_tones_become_red_green_blue() {
        for (freq, k) in [(80.0, 0usize), (1500.0, 1), (8000.0, 2)] {
            let mut a = AudioColor::default();
            let mut c = [0.0; 3];
            // 同じ強さの音を続けると、その帯域の色だけが強くなる
            for _ in 0..30 {
                c = a.push(48000, &[&tone(freq, 48000.0, 1024)]);
            }
            let max = (0..3).max_by(|x, y| c[*x].total_cmp(&c[*y])).unwrap();
            assert_eq!(max, k, "{freq}Hz → {c:?}");
            assert!(c[k] > 0.8, "{freq}Hz → {c:?}");
        }
    }

    #[test]
    fn silence_fades_out() {
        let mut a = AudioColor::default();
        for _ in 0..20 {
            a.push(48000, &[&tone(80.0, 48000.0, 1024)]);
        }
        let mut c = [1.0; 3];
        for _ in 0..40 {
            c = a.push(48000, &[&vec![0.0; 1024]]);
        }
        assert!(c.iter().all(|x| *x < 0.05), "{c:?}");
    }

    #[test]
    fn average_color_of_bgra_and_uyvy() {
        // 左半分が赤、右半分が青のBGRA
        let (w, h) = (64usize, 4usize);
        let mut px = vec![0u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let p = &mut px[(y * w + x) * 4..(y * w + x) * 4 + 4];
                p.copy_from_slice(if x < w / 2 { &[0, 0, 255, 255] } else { &[255, 0, 0, 255] });
            }
        }
        let c = average_color(w, h, w * 4, FOURCC_BGRA, &px).unwrap();
        assert!((c[0] - 0.5).abs() < 0.02 && c[1] < 0.01 && (c[2] - 0.5).abs() < 0.02, "{c:?}");
        // UYVYの白
        let mut uy = vec![0u8; w * h * 2];
        for q in uy.chunks_mut(4) {
            q.copy_from_slice(&[128, 235, 128, 235]);
        }
        let c2 = average_color(w, h, w * 2, FOURCC_UYVY, &uy).unwrap();
        assert!(c2.iter().all(|x| *x > 0.97), "{c2:?}");
        // 足りないデータ・知らない形式は読まない
        assert!(average_color(w, h, w * 4, FOURCC_BGRA, &px[..px.len() - 1]).is_none());
        assert!(average_color(w, h, w * 2, FOURCC_BGRA, &px).is_none());
        assert!(average_color(w, h, w * 4, 0, &px).is_none());
    }

    #[test]
    fn load_error_says_not_found_unless_the_library_was_broken() {
        let msg = "NDI runtime (libndi.so.6) could not be loaded.\n  /usr/local/lib/libndi.so.6: not found";
        assert!(load_error_status(ErrorKind::RuntimeNotFound, None, msg).contains("見つかりません"));
        let line = "/opt/x/libndi.so.6: missing NDI symbol: NDIlib_recv_capture_v3";
        assert_eq!(load_error_status(ErrorKind::RuntimeMissingSymbol, Some(line), msg), format!("NDIのライブラリを使えません: {line}"));
        let line = "/opt/x/libndi.so.6: NDIlib_initialize failed (unsupported CPU?)";
        assert_eq!(load_error_status(ErrorKind::RuntimeInitFailed, Some(line), msg), format!("NDIのライブラリを使えません: {line}"));
        // 種類が分からないときは、エラー文の1行目を出す
        assert_eq!(load_error_status(ErrorKind::Other, None, "boom\n  more"), "NDIのライブラリを読み込めませんでした: boom");
    }
}

/// 実機での確認用(普段は実行しない)。NDIのソースを探して受信し、映像の平均色と音の色を表示する。
/// 実行: DMXCTL_NDI_SOURCE=ソース名の一部 cargo test --release hw_ndi -- --ignored --nocapture
#[cfg(test)]
mod hw_tests {
    use super::*;
    use crate::engine::Core;
    use std::sync::Mutex;

    #[test]
    #[ignore]
    fn hw_ndi() {
        let (lib, _) = crate::fixture::Library::load(std::path::Path::new("none"));
        let core: Shared = Arc::new(Mutex::new(Core::new(crate::project::Project::default(), Arc::new(lib))));
        {
            let mut c = lock(&core);
            c.project.ndi = NdiCfg { enabled: true, source: String::new(), main: LiveSource::Video, sub: LiveSource::Audio };
        }
        let svc = NdiService::start(core.clone(), None);
        let want = std::env::var("DMXCTL_NDI_SOURCE").unwrap_or_default();
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(15) {
            std::thread::sleep(Duration::from_millis(500));
            let mut c = lock(&core);
            if c.project.ndi.source.is_empty()
                && let Some(s) = c.rt.ndi.sources.iter().find(|s| s.contains(&want)).cloned() {
                println!("ソース: {:?} → {s} につなぐ", c.rt.ndi.sources);
                c.project.ndi.source = s;
            }
            let st = &c.rt.ndi;
            println!("{:>4.1}s {} / 映像 {:?} / 音 {:?}", t0.elapsed().as_secs_f32(), st.status, st.video.map(|v| v.map(|x| (x * 255.0) as u8)), st.audio.map(|v| v.map(|x| (x * 100.0) as u8)));
        }
        drop(svc);
    }
}
