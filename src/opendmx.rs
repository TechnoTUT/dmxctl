//! FTDI(FT232)系の安価なUSB-DMX(Open DMX方式)。
//!
//! このタイプのインターフェースにはDMXを組み立てるマイコンが無く、PC側が
//! BREAK → MAB → スタートコード+スロット を250kbaud/8N2で連続して送り続ける必要がある。
//! 送出は専用スレッドで行い、接続が切れたら自動で再接続する。

use crate::output::Output;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// フレーム周期。512スロットでも約23ms+BREAKなので25msで足りる
const FRAME_PERIOD: Duration = Duration::from_millis(25);
const BREAK_US: u64 = 150;
const MAB_US: u64 = 20;

#[derive(Clone, Debug)]
pub enum LinkKind {
    Serial(String),
    D2xx(u32),
}

trait Link {
    /// `data`はスタートコード込み
    fn send(&mut self, data: &[u8]) -> Result<(), String>;
}

fn spin(us: u64) {
    let t = Instant::now();
    let d = Duration::from_micros(us);
    while t.elapsed() < d {
        std::hint::spin_loop();
    }
}

pub struct OpenDmx {
    frame: Arc<Mutex<Vec<u8>>>,
    status: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl OpenDmx {
    pub fn new(kind: LinkKind) -> Self {
        let frame = Arc::new(Mutex::new(vec![0u8; 25]));
        let status = Arc::new(Mutex::new("接続待ち".to_string()));
        let stop = Arc::new(AtomicBool::new(false));
        let (f, s, st) = (frame.clone(), status.clone(), stop.clone());
        let handle = std::thread::Builder::new()
            .name("open-dmx".into())
            .spawn(move || run(kind, f, s, st))
            .ok();
        OpenDmx { frame, status, stop, handle }
    }
}

impl Output for OpenDmx {
    fn send(&mut self, frame: &[u8; 512], len: usize) {
        let mut f = self.frame.lock().unwrap_or_else(|e| e.into_inner());
        f.clear();
        f.push(0); // スタートコード
        f.extend_from_slice(&frame[..len.min(512)]);
    }
    fn status(&self) -> String {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Drop for OpenDmx {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn run(kind: LinkKind, frame: Arc<Mutex<Vec<u8>>>, status: Arc<Mutex<String>>, stop: Arc<AtomicBool>) {
    let set = |s: String| *status.lock().unwrap_or_else(|e| e.into_inner()) = s;
    let mut link: Option<Box<dyn Link>> = None;

    while !stop.load(Ordering::SeqCst) {
        if link.is_none() {
            let opened: Result<Box<dyn Link>, String> = match &kind {
                LinkKind::Serial(p) => SerialLink::open(p).map(|l| Box::new(l) as Box<dyn Link>),
                LinkKind::D2xx(i) => D2xxLink::open(*i).map(|l| Box::new(l) as Box<dyn Link>),
            };
            match opened {
                Ok(l) => {
                    set("接続中".into());
                    link = Some(l);
                }
                Err(e) => {
                    set(format!("未接続: {e}"));
                    // 1秒ごとに再試行(停止要求にはすぐ反応する)
                    for _ in 0..20 {
                        if stop.load(Ordering::SeqCst) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    continue;
                }
            }
        }

        let t0 = Instant::now();
        let data = frame.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(l) = link.as_mut()
            && let Err(e) = l.send(&data) {
            set(format!("切断: {e}"));
            link = None;
            continue;
        }
        let elapsed = t0.elapsed();
        if elapsed < FRAME_PERIOD {
            std::thread::sleep(FRAME_PERIOD - elapsed);
        }
    }
}

// ---------------------------------------------------------------- COMポート方式

struct SerialLink(Box<dyn serialport::SerialPort>);

impl SerialLink {
    fn open(name: &str) -> Result<Self, String> {
        let mut p = serialport::new(name, 250_000)
            .data_bits(serialport::DataBits::Eight)
            .parity(serialport::Parity::None)
            .stop_bits(serialport::StopBits::Two)
            .flow_control(serialport::FlowControl::None)
            .timeout(Duration::from_millis(200))
            .open()
            .map_err(|e| e.to_string())?;
        // Enttec Open DMXの参照実装に合わせ、RTSをLow(送信側)にしておく
        let _ = p.write_request_to_send(false);
        let _ = p.clear_break();
        Ok(SerialLink(p))
    }
}

impl Link for SerialLink {
    fn send(&mut self, data: &[u8]) -> Result<(), String> {
        self.0.set_break().map_err(|e| e.to_string())?;
        spin(BREAK_US);
        self.0.clear_break().map_err(|e| e.to_string())?;
        spin(MAB_US);
        self.0.write_all(data).map_err(|e| e.to_string())
    }
}

/// COMポート名と説明(FTDIなどUSB機器は製品名つき)
pub fn list_serial_ports() -> Vec<(String, String)> {
    serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            let desc = match &p.port_type {
                serialport::SerialPortType::UsbPort(u) => {
                    format!("{} ({})", p.port_name, u.product.clone().unwrap_or_else(|| format!("USB {:04X}:{:04X}", u.vid, u.pid)))
                }
                _ => p.port_name.clone(),
            };
            (p.port_name, desc)
        })
        .collect()
}

// ---------------------------------------------------------------- D2XX方式

type FtHandle = *mut c_void;
type FtStatus = u32;

struct D2xxApi {
    _lib: libloading::Library,
    create_list: unsafe extern "system" fn(*mut u32) -> FtStatus,
    info_detail: unsafe extern "system" fn(u32, *mut u32, *mut u32, *mut u32, *mut u32, *mut c_void, *mut c_void, *mut FtHandle) -> FtStatus,
    open: unsafe extern "system" fn(i32, *mut FtHandle) -> FtStatus,
    close: unsafe extern "system" fn(FtHandle) -> FtStatus,
    reset: unsafe extern "system" fn(FtHandle) -> FtStatus,
    set_baud: unsafe extern "system" fn(FtHandle, u32) -> FtStatus,
    set_data: unsafe extern "system" fn(FtHandle, u8, u8, u8) -> FtStatus,
    set_flow: unsafe extern "system" fn(FtHandle, u16, u8, u8) -> FtStatus,
    clr_rts: unsafe extern "system" fn(FtHandle) -> FtStatus,
    break_on: unsafe extern "system" fn(FtHandle) -> FtStatus,
    break_off: unsafe extern "system" fn(FtHandle) -> FtStatus,
    purge: unsafe extern "system" fn(FtHandle, u32) -> FtStatus,
    write: unsafe extern "system" fn(FtHandle, *const u8, u32, *mut u32) -> FtStatus,
}

impl D2xxApi {
    /// D2XXドライバのライブラリ(Windows: ftd2xx.dll / Mac: libftd2xx.dylib / Linux: libftd2xx.so)を実行時に読み込む。
    /// 無くてもアプリ自体は動く
    fn load() -> Result<Arc<D2xxApi>, String> {
        let mut last = String::new();
        let names: &[&str] = if cfg!(windows) {
            &["ftd2xx64.dll", "ftd2xx.dll"]
        } else if cfg!(target_os = "macos") {
            &["libftd2xx.dylib", "/usr/local/lib/libftd2xx.dylib", "/opt/homebrew/lib/libftd2xx.dylib"]
        } else {
            &["libftd2xx.so"]
        };
        // Macのアプリ(.app)に同梱したもの(Contents/Frameworks)を最初に探す
        let bundled = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.parent()?.join("Frameworks").join("libftd2xx.dylib"))).filter(|p| cfg!(target_os = "macos") && p.exists());
        // 確認用: DMXCTL_D2XX_LIB=ライブラリのパス で直接指定
        let custom = std::env::var_os("DMXCTL_D2XX_LIB").filter(|v| !v.is_empty());
        let candidates: Vec<std::ffi::OsString> = custom.into_iter().chain(bundled.map(|p| p.into_os_string())).chain(names.iter().map(|n| n.into())).collect();
        for name in candidates {
            // SAFETY: FTDI公式のDLL。シンボルはD2XXの公開APIと同じシグネチャで宣言している
            match unsafe { libloading::Library::new(&name) } {
                Ok(lib) => return Self::bind(lib).map(Arc::new),
                Err(e) => last = e.to_string(),
            }
        }
        Err(format!("D2XXドライバ({})が見つかりません(FTDIドライバ未導入?): {last}", names[0]))
    }

    fn bind(lib: libloading::Library) -> Result<D2xxApi, String> {
        macro_rules! sym {
            ($n:literal) => {
                // SAFETY: 型は上のフィールド宣言と同じ。ライブラリは_libで保持するので関数ポインタは有効
                *unsafe { lib.get($n) }.map_err(|e| format!("{}: {e}", String::from_utf8_lossy($n)))?
            };
        }
        Ok(D2xxApi {
            create_list: sym!(b"FT_CreateDeviceInfoList"),
            info_detail: sym!(b"FT_GetDeviceInfoDetail"),
            open: sym!(b"FT_Open"),
            close: sym!(b"FT_Close"),
            reset: sym!(b"FT_ResetDevice"),
            set_baud: sym!(b"FT_SetBaudRate"),
            set_data: sym!(b"FT_SetDataCharacteristics"),
            set_flow: sym!(b"FT_SetFlowControl"),
            clr_rts: sym!(b"FT_ClrRts"),
            break_on: sym!(b"FT_SetBreakOn"),
            break_off: sym!(b"FT_SetBreakOff"),
            purge: sym!(b"FT_Purge"),
            write: sym!(b"FT_Write"),
            _lib: lib,
        })
    }
}

/// 接続中のFTDIデバイス名の一覧
pub fn list_d2xx() -> Result<Vec<String>, String> {
    let api = D2xxApi::load()?;
    let mut n = 0u32;
    // SAFETY: 出力先は有効なu32
    if unsafe { (api.create_list)(&mut n) } != 0 {
        return Err("デバイス列挙に失敗しました".into());
    }
    let mut out = Vec::new();
    for i in 0..n {
        let (mut flags, mut ty, mut id, mut loc) = (0u32, 0u32, 0u32, 0u32);
        let mut serial = [0u8; 64];
        let mut desc = [0u8; 64];
        let mut h: FtHandle = std::ptr::null_mut();
        // SAFETY: バッファはD2XXの要求(16/64バイト以上)を満たしている
        let st = unsafe {
            (api.info_detail)(i, &mut flags, &mut ty, &mut id, &mut loc, serial.as_mut_ptr().cast(), desc.as_mut_ptr().cast(), &mut h)
        };
        if st == 0 {
            out.push(format!("{}: {} [{}]", i, cstr(&desc), cstr(&serial)));
        }
    }
    Ok(out)
}

fn cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).into_owned()
}

struct D2xxLink {
    api: Arc<D2xxApi>,
    handle: FtHandle,
}

// SAFETY: ハンドルはこのスレッドでしか使わない(生成後にスレッドへ移動するだけ)
unsafe impl Send for D2xxLink {}

impl D2xxLink {
    fn open(index: u32) -> Result<Self, String> {
        let api = D2xxApi::load()?;
        let mut h: FtHandle = std::ptr::null_mut();
        // SAFETY: 以降のD2XX呼び出しは、FT_Openが返した有効なハンドルに対して行う
        unsafe {
            check((api.open)(index as i32, &mut h), "FT_Open(デバイスが他のアプリで使用中か未接続)")?;
            let link = D2xxLink { api: api.clone(), handle: h }; // 以降のエラーでもDropでcloseされる
            check((api.reset)(h), "FT_ResetDevice")?;
            check((api.set_baud)(h, 250_000), "FT_SetBaudRate")?;
            check((api.set_data)(h, 8, 2, 0), "FT_SetDataCharacteristics")?; // 8bit, 2stop, パリティなし
            check((api.set_flow)(h, 0, 0, 0), "FT_SetFlowControl")?;
            check((api.clr_rts)(h), "FT_ClrRts")?;
            check((api.purge)(h, 3), "FT_Purge")?;
            check((api.break_off)(h), "FT_SetBreakOff")?;
            Ok(link)
        }
    }
}

fn check(st: FtStatus, what: &str) -> Result<(), String> {
    if st == 0 { Ok(()) } else { Err(format!("{what} 失敗(コード{st})")) }
}

impl Link for D2xxLink {
    fn send(&mut self, data: &[u8]) -> Result<(), String> {
        let h = self.handle;
        // SAFETY: hは有効なハンドル、dataは呼び出し中有効
        unsafe {
            check((self.api.break_on)(h), "FT_SetBreakOn")?;
            spin(BREAK_US);
            check((self.api.break_off)(h), "FT_SetBreakOff")?;
            spin(MAB_US);
            let mut written = 0u32;
            check((self.api.write)(h, data.as_ptr(), data.len() as u32, &mut written), "FT_Write")?;
            if written as usize != data.len() {
                return Err("FT_Write: 書き込みが途中で終わりました".into());
            }
        }
        Ok(())
    }
}

impl Drop for D2xxLink {
    fn drop(&mut self) {
        // SAFETY: 有効なハンドルを一度だけ閉じる
        unsafe {
            (self.api.close)(self.handle);
        }
    }
}

/// 実機での確認用(普段は実行しない)。USB-DMXをつないで、全チャンネル0のフレームを3秒間送る。
/// 実行: DMXCTL_TEST_PORT=/dev/cu.usbserial-XXXX cargo test --release hw_ -- --ignored --nocapture --test-threads=1
#[cfg(test)]
mod hw_tests {
    use super::*;

    fn send_for(link: &mut dyn Link, secs: f64) -> (u32, Duration) {
        let data = vec![0u8; 513];
        let start = Instant::now();
        let mut n = 0;
        while start.elapsed().as_secs_f64() < secs {
            let t0 = Instant::now();
            link.send(&data).unwrap();
            n += 1;
            if let Some(rest) = FRAME_PERIOD.checked_sub(t0.elapsed()) {
                std::thread::sleep(rest);
            }
        }
        (n, start.elapsed())
    }

    #[test]
    #[ignore]
    fn hw_serial() {
        let port = std::env::var("DMXCTL_TEST_PORT").expect("DMXCTL_TEST_PORT");
        let mut link = SerialLink::open(&port).unwrap();
        let (n, t) = send_for(&mut link, 3.0);
        // 送り終わるまで待って、実際に送れた速さを見る
        let t1 = Instant::now();
        link.0.flush().unwrap();
        println!("COMポート: {n}フレーム / {:.2}秒 = {:.1}fps(送り切りまで +{:.1}ms)", t.as_secs_f64(), n as f64 / t.as_secs_f64(), t1.elapsed().as_secs_f64() * 1000.0);
    }

    /// ムービング(MINI SPOT 30 ×4、アドレス1/10/19/28)の動作確認。3つの段階で12秒ずつ:
    /// A=9chモードのつもりでパンを左右いっぱいに / B=11chモードのつもりで同じ / C=本体の内蔵プログラム
    #[test]
    #[ignore]
    fn hw_spot_sweep() {
        let port = std::env::var("DMXCTL_TEST_PORT").expect("DMXCTL_TEST_PORT");
        let mut link = SerialLink::open(&port).unwrap();
        let addrs = [1usize, 10, 19, 28];
        let start = Instant::now();
        let mut phase = 99;
        while start.elapsed().as_secs_f64() < 36.0 {
            let t = start.elapsed().as_secs_f64();
            let ph = (t / 12.0) as usize;
            if ph != phase {
                phase = ph;
                println!("{:>4.1}秒: 段階{}", t, ["A(9ch)", "B(11ch)", "C(内蔵プログラム)"][ph.min(2)]);
            }
            let local = t % 12.0;
            // 0→255→0 を6秒で1往復
            let tri = 1.0 - ((local / 6.0).fract() * 2.0 - 1.0).abs();
            let pan = (tri * 255.0) as u8;
            let mut d = vec![0u8; 89];
            for &a in &addrs {
                let ch = |n: usize| a + n - 1; // n=1始まりのチャンネル → スタートコードを含む配列の位置
                match ph {
                    0 => {
                        d[ch(1)] = pan;
                        d[ch(2)] = 128;
                        d[ch(6)] = 255; // dimmer
                    }
                    1 => {
                        d[ch(1)] = pan;
                        d[ch(3)] = 128;
                        d[ch(8)] = 255; // dimmer(11ch)
                    }
                    _ => {
                        d[ch(6)] = 255;
                        d[ch(8)] = 100; // 9ch: auto=内蔵プログラム / 11ch: dimmer
                        d[ch(10)] = 100; // 11ch: auto
                    }
                }
            }
            let t0 = Instant::now();
            link.send(&d).unwrap();
            if let Some(rest) = FRAME_PERIOD.checked_sub(t0.elapsed()) {
                std::thread::sleep(rest);
            }
        }
        // 最後は全部0(消灯)
        link.send(&[0u8; 89]).unwrap();
        link.0.flush().unwrap();
        println!("終了");
    }

    #[test]
    #[ignore]
    fn hw_d2xx() {
        println!("D2XX: {:?}", list_d2xx());
        let mut link = D2xxLink::open(0).unwrap();
        let (n, t) = send_for(&mut link, 3.0);
        println!("D2XX: {n}フレーム / {:.2}秒 = {:.1}fps", t.as_secs_f64(), n as f64 / t.as_secs_f64());
    }
}
