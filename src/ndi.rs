//! NDI受信: 映像の平均色と、音の低音・中音・高音を、照明の色(メイン色/サブ色)に使う。
//!
//! NDIのライブラリ(libndi)はアプリに同梱せず、インストール済みのもの(NDI Tools・NDIランタイム)を実行時に読み込む。
//! 受信はこのスレッドで行い、結果(色)はCoreの`rt.ndi`に書く。照明への反映はエンジンが行う

use crate::engine::{lock, Shared};
use serde::{Deserialize, Serialize};
use std::ffi::{c_char, c_void, CStr, CString};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// メイン色/サブ色を、何から作るか
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum LiveSource {
    /// 使わない(パッド・フェーダーで決めた色)
    #[default]
    Off,
    /// 音: 低音=赤 / 中音=緑 / 高音=青
    Audio,
    /// 映像の平均の色
    Video,
}

impl LiveSource {
    pub fn label(self) -> &'static str {
        match self {
            LiveSource::Off => "使わない",
            LiveSource::Audio => "音(低=赤 中=緑 高=青)",
            LiveSource::Video => "映像の平均色",
        }
    }
}

/// NDIの設定(保存する)
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NdiCfg {
    /// 受信する
    pub enabled: bool,
    /// 受信するNDIソースの名前(「PC名 (ソース名)」)
    pub source: String,
    pub main: LiveSource,
    pub sub: LiveSource,
}

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

// ---------------------------------------------------------------- libndi(NDI SDK v5/v6 の C API)

#[repr(C)]
#[derive(Clone, Copy)]
struct Source {
    name: *const c_char,
    url: *const c_char,
}

#[repr(C)]
struct FindCreate {
    show_local_sources: bool,
    groups: *const c_char,
    extra_ips: *const c_char,
}

#[repr(C)]
struct RecvCreateV3 {
    source: Source,
    color_format: i32,
    bandwidth: i32,
    allow_video_fields: bool,
    recv_name: *const c_char,
}

#[repr(C)]
struct VideoFrameV2 {
    xres: i32,
    yres: i32,
    fourcc: u32,
    frame_rate_n: i32,
    frame_rate_d: i32,
    aspect: f32,
    frame_format: i32,
    timecode: i64,
    data: *mut u8,
    line_stride: i32,
    metadata: *const c_char,
    timestamp: i64,
}

#[repr(C)]
struct AudioFrameV2 {
    sample_rate: i32,
    channels: i32,
    samples: i32,
    timecode: i64,
    data: *mut f32,
    channel_stride: i32,
    metadata: *const c_char,
    timestamp: i64,
}

#[repr(C)]
struct MetadataFrame {
    length: i32,
    timecode: i64,
    data: *mut c_char,
}

const FRAME_VIDEO: i32 = 1;
const FRAME_AUDIO: i32 = 2;
const FRAME_METADATA: i32 = 3;
/// 受け取る映像の形式: BGRX/BGRA
const COLOR_BGRX_BGRA: i32 = 0;
/// 帯域: 一番軽い映像(平均色にはこれで十分) / 音だけ
const BANDWIDTH_LOWEST: i32 = 0;
const BANDWIDTH_AUDIO_ONLY: i32 = 10;

const fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | (b as u32) << 8 | (c as u32) << 16 | (d as u32) << 24
}

struct Api {
    _lib: libloading::Library,
    initialize: unsafe extern "C" fn() -> bool,
    find_create: unsafe extern "C" fn(*const FindCreate) -> *mut c_void,
    find_destroy: unsafe extern "C" fn(*mut c_void),
    find_sources: unsafe extern "C" fn(*mut c_void, *mut u32) -> *const Source,
    recv_create: unsafe extern "C" fn(*const RecvCreateV3) -> *mut c_void,
    recv_destroy: unsafe extern "C" fn(*mut c_void),
    recv_capture: unsafe extern "C" fn(*mut c_void, *mut VideoFrameV2, *mut AudioFrameV2, *mut MetadataFrame, u32) -> i32,
    free_video: unsafe extern "C" fn(*mut c_void, *const VideoFrameV2),
    free_audio: unsafe extern "C" fn(*mut c_void, *const AudioFrameV2),
    free_metadata: unsafe extern "C" fn(*mut c_void, *const MetadataFrame),
}

/// libndi を探す場所(先に見つかったものを使う)
fn lib_paths() -> Vec<std::path::PathBuf> {
    let mut v: Vec<std::path::PathBuf> = Vec::new();
    let file = if cfg!(windows) { "Processing.NDI.Lib.x64.dll" } else if cfg!(target_os = "macos") { "libndi.dylib" } else { "libndi.so.6" };
    // NDIランタイム・SDKが知らせる場所
    for k in ["DMXCTL_NDI_LIB", "NDI_RUNTIME_DIR_V6", "NDI_RUNTIME_DIR_V5"] {
        if let Some(p) = std::env::var_os(k).filter(|p| !p.is_empty()) {
            let p = std::path::PathBuf::from(p);
            v.push(if p.extension().is_some() { p } else { p.join(file) });
        }
    }
    if cfg!(target_os = "macos") {
        for p in [
            "/usr/local/lib/libndi.dylib",
            "/Library/NDI SDK for Apple/lib/macOS/libndi.dylib",
            "/Applications/NDI Virtual Input.app/Contents/Frameworks/libndi.dylib",
            "/Applications/NDI Scan Converter.app/Contents/Frameworks/libndi.dylib",
            "/Applications/NDI Video Monitor.app/Contents/Frameworks/libndi_advanced.dylib",
            "/Applications/NDI Test Patterns.app/Contents/Frameworks/libndi_advanced.dylib",
        ] {
            v.push(p.into());
        }
    } else {
        v.push(file.into());
    }
    v
}

impl Api {
    fn load() -> Result<Api, String> {
        let mut last = "NDIのライブラリが見つかりません(NDI Tools をインストールしてください)".to_string();
        for path in lib_paths() {
            if !path.exists() && path.is_absolute() {
                continue;
            }
            // SAFETY: NDI公式のライブラリ。関数は NDI SDK の公開APIと同じ形で宣言している
            match unsafe { libloading::Library::new(&path) } {
                Ok(lib) => return Self::bind(lib),
                Err(e) => last = format!("{}: {e}", path.display()),
            }
        }
        Err(last)
    }

    fn bind(lib: libloading::Library) -> Result<Api, String> {
        macro_rules! sym {
            ($n:literal) => {
                // SAFETY: 型は上の宣言と同じ。ライブラリは_libで保持するので関数ポインタは有効
                *unsafe { lib.get($n) }.map_err(|e| format!("NDIの関数が見つかりません: {e}"))?
            };
        }
        Ok(Api {
            initialize: sym!(b"NDIlib_initialize\0"),
            find_create: sym!(b"NDIlib_find_create_v2\0"),
            find_destroy: sym!(b"NDIlib_find_destroy\0"),
            find_sources: sym!(b"NDIlib_find_get_current_sources\0"),
            recv_create: sym!(b"NDIlib_recv_create_v3\0"),
            recv_destroy: sym!(b"NDIlib_recv_destroy\0"),
            recv_capture: sym!(b"NDIlib_recv_capture_v2\0"),
            free_video: sym!(b"NDIlib_recv_free_video_v2\0"),
            free_audio: sym!(b"NDIlib_recv_free_audio_v2\0"),
            free_metadata: sym!(b"NDIlib_recv_free_metadata\0"),
            _lib: lib,
        })
    }
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
fn average_color(f: &VideoFrameV2) -> Option<[f32; 3]> {
    let (w, h, stride) = (f.xres.max(0) as usize, f.yres.max(0) as usize, f.line_stride.max(0) as usize);
    if w == 0 || h == 0 || stride == 0 || f.data.is_null() {
        return None;
    }
    // SAFETY: NDIが渡した、stride×高さ のフレーム
    let data = unsafe { std::slice::from_raw_parts(f.data, stride * h) };
    let (sx, sy) = ((w / 64).max(1), (h / 36).max(1));
    let mut sum = [0.0f64; 3];
    let mut n = 0.0f64;
    let bgra = [fourcc(b'B', b'G', b'R', b'A'), fourcc(b'B', b'G', b'R', b'X')];
    let rgba = [fourcc(b'R', b'G', b'B', b'A'), fourcc(b'R', b'G', b'B', b'X')];
    let uyvy = fourcc(b'U', b'Y', b'V', b'Y');
    for y in (0..h).step_by(sy) {
        let row = &data[y * stride..];
        for x in (0..w).step_by(sx) {
            let rgb = if bgra.contains(&f.fourcc) {
                let p = &row[x * 4..x * 4 + 3];
                [p[2], p[1], p[0]].map(|v| v as f64)
            } else if rgba.contains(&f.fourcc) {
                let p = &row[x * 4..x * 4 + 3];
                [p[0], p[1], p[2]].map(|v| v as f64)
            } else if f.fourcc == uyvy {
                // 2画素で U Y0 V Y1
                let p = &row[(x / 2) * 4..(x / 2) * 4 + 4];
                let (u, yy, v) = (p[0] as f64 - 128.0, p[1 + (x % 2) * 2] as f64 - 16.0, p[2] as f64 - 128.0);
                let c = 1.164 * yy;
                [c + 1.596 * v, c - 0.392 * u - 0.813 * v, c + 2.017 * u].map(|v| v.clamp(0.0, 255.0))
            } else {
                return None;
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
    pub fn start(core: Shared, ctx: Option<eframe::egui::Context>) -> NdiService {
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

fn run(core: Shared, ctx: Option<eframe::egui::Context>, stop: Arc<AtomicBool>) {
    let api = match Api::load() {
        Ok(a) => a,
        Err(e) => {
            set_state(&core, |s| s.status = e);
            return;
        }
    };
    // SAFETY: 以降のNDI呼び出しは、作ったインスタンスをこのスレッドだけで使い、最後に破棄する
    unsafe {
        if !(api.initialize)() {
            set_state(&core, |s| s.status = "NDIを初期化できませんでした(このCPUには対応していません)".into());
            return;
        }
        // 同じPCのソース(TouchDesignerなど)は、マルチキャストの探索が使えないときでも見つかるよう、自分宛ても直接たずねる
        let extra = CString::new("127.0.0.1").unwrap_or_default();
        let find = (api.find_create)(&FindCreate { show_local_sources: true, groups: std::ptr::null(), extra_ips: extra.as_ptr() });
        if find.is_null() {
            set_state(&core, |s| s.status = "NDIのソース検索を開始できませんでした".into());
            return;
        }
        let mut recv: *mut c_void = std::ptr::null_mut();
        let mut connected = String::new();
        let mut want_video = false;
        let mut audio = AudioColor::default();
        let mut last_find = Instant::now() - Duration::from_secs(10);
        let mut last_frame = Instant::now();
        let mut found: Vec<(String, Source)> = Vec::new();
        set_state(&core, |s| s.status = "ソースを探しています".into());

        while !stop.load(Ordering::SeqCst) {
            // 1秒ごとにソースの一覧を更新
            if last_find.elapsed() > Duration::from_secs(1) {
                last_find = Instant::now();
                let mut n = 0u32;
                let p = (api.find_sources)(find, &mut n);
                found = (0..n as usize)
                    .filter_map(|i| {
                        let s = *p.add(i);
                        (!s.name.is_null()).then(|| (CStr::from_ptr(s.name).to_string_lossy().into_owned(), s))
                    })
                    .collect();
                let names: Vec<String> = found.iter().map(|(n, _)| n.clone()).collect();
                set_state(&core, |s| s.sources = names);
            }
            let (source, need_video, need_any) = {
                let c = lock(&core);
                let cfg = &c.project.ndi;
                (cfg.source.clone(), cfg.main == LiveSource::Video || cfg.sub == LiveSource::Video, cfg.main != LiveSource::Off || cfg.sub != LiveSource::Off)
            };
            // つなぐソース・必要なもの(映像の有無)が変わったら、受信し直す
            if !recv.is_null() && (source != connected || need_video != want_video) {
                (api.recv_destroy)(recv);
                recv = std::ptr::null_mut();
                connected.clear();
            }
            if recv.is_null() && !source.is_empty() {
                if let Some((_, s)) = found.iter().find(|(n, _)| *n == source) {
                    let name = CString::new("DMX Controller").unwrap_or_default();
                    let cfg = RecvCreateV3 {
                        source: *s,
                        color_format: COLOR_BGRX_BGRA,
                        bandwidth: if need_video { BANDWIDTH_LOWEST } else { BANDWIDTH_AUDIO_ONLY },
                        allow_video_fields: false,
                        recv_name: name.as_ptr(),
                    };
                    recv = (api.recv_create)(&cfg);
                    if recv.is_null() {
                        set_state(&core, |st| st.status = format!("{source} に接続できませんでした"));
                    } else {
                        connected = source.clone();
                        want_video = need_video;
                        last_frame = Instant::now();
                        set_state(&core, |st| st.status = format!("{source} に接続中"));
                    }
                } else {
                    set_state(&core, |st| st.status = if source.is_empty() { "ソースを選んでください".into() } else { format!("{source} が見つかりません") });
                }
            }
            if recv.is_null() {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            }

            let mut video: VideoFrameV2 = std::mem::zeroed();
            let mut aud: AudioFrameV2 = std::mem::zeroed();
            let mut meta: MetadataFrame = std::mem::zeroed();
            match (api.recv_capture)(recv, &mut video, &mut aud, &mut meta, 100) {
                FRAME_VIDEO => {
                    let c = average_color(&video).map(vivid);
                    (api.free_video)(recv, &video);
                    last_frame = Instant::now();
                    if let Some(c) = c {
                        set_state(&core, |st| {
                            // 急に変わりすぎないよう少しなめらかに
                            let prev = st.video.unwrap_or(c);
                            st.video = Some([0, 1, 2].map(|k| prev[k] + (c[k] - prev[k]) * 0.35));
                            st.status = format!("{connected} を受信中 ({}×{})", video.xres, video.yres);
                        });
                    }
                }
                FRAME_AUDIO => {
                    let chans = aud.channels.max(0) as usize;
                    let samples = aud.samples.max(0) as usize;
                    let stride = (aud.channel_stride.max(0) as usize) / std::mem::size_of::<f32>();
                    let rgb = if !aud.data.is_null() && chans > 0 && samples > 0 && stride >= samples {
                        let all = std::slice::from_raw_parts(aud.data, stride * chans);
                        let ch: Vec<&[f32]> = (0..chans).map(|k| &all[k * stride..k * stride + samples]).collect();
                        Some(audio.push(aud.sample_rate, &ch))
                    } else {
                        None
                    };
                    (api.free_audio)(recv, &aud);
                    last_frame = Instant::now();
                    if let Some(rgb) = rgb {
                        set_state(&core, |st| st.audio = Some(rgb));
                    }
                }
                FRAME_METADATA => (api.free_metadata)(recv, &meta),
                _ => {}
            }
            // 2秒届かなければ、色の元を消す(古い色で光り続けないように)
            if last_frame.elapsed() > Duration::from_secs(2) {
                set_state(&core, |st| {
                    st.audio = None;
                    st.video = None;
                    st.status = format!("{connected}: 受信が止まっています");
                });
            }
            if need_any {
                if let Some(c) = &ctx {
                    c.request_repaint_after(Duration::from_millis(250));
                }
            }
        }
        if !recv.is_null() {
            (api.recv_destroy)(recv);
        }
        (api.find_destroy)(find);
    }
    set_state(&core, |st| {
        st.audio = None;
        st.video = None;
        st.status = "停止中".into();
    });
}

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
        let f = VideoFrameV2 {
            xres: w as i32,
            yres: h as i32,
            fourcc: fourcc(b'B', b'G', b'R', b'A'),
            frame_rate_n: 30,
            frame_rate_d: 1,
            aspect: 0.0,
            frame_format: 1,
            timecode: 0,
            data: px.as_mut_ptr(),
            line_stride: (w * 4) as i32,
            metadata: std::ptr::null(),
            timestamp: 0,
        };
        let c = average_color(&f).unwrap();
        assert!((c[0] - 0.5).abs() < 0.02 && c[1] < 0.01 && (c[2] - 0.5).abs() < 0.02, "{c:?}");
        // UYVYの白
        let mut uy = vec![0u8; w * h * 2];
        for q in uy.chunks_mut(4) {
            q.copy_from_slice(&[128, 235, 128, 235]);
        }
        let f2 = VideoFrameV2 { fourcc: fourcc(b'U', b'Y', b'V', b'Y'), data: uy.as_mut_ptr(), line_stride: (w * 2) as i32, ..f };
        let c2 = average_color(&f2).unwrap();
        assert!(c2.iter().all(|x| *x > 0.97), "{c2:?}");
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
            if c.project.ndi.source.is_empty() {
                if let Some(s) = c.rt.ndi.sources.iter().find(|s| s.contains(&want)).cloned() {
                    println!("ソース: {:?} → {s} につなぐ", c.rt.ndi.sources);
                    c.project.ndi.source = s;
                }
            }
            let st = &c.rt.ndi;
            println!("{:>4.1}s {} / 映像 {:?} / 音 {:?}", t0.elapsed().as_secs_f32(), st.status, st.video.map(|v| v.map(|x| (x * 255.0) as u8)), st.audio.map(|v| v.map(|x| (x * 100.0) as u8)));
        }
        drop(svc);
    }
}
