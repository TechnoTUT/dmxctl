//! FTDI(FT232)系の安価なUSB-DMX(Open DMX方式)への出力。
//!
//! このタイプにはDMXを組み立てるマイコンが無く、PC側が
//! BREAK → MAB → スタートコード+512スロット を250kbaud/8N2で送り続ける必要がある。
//! 送出は専用スレッドで行い、抜けたら1秒ごとに再接続する。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const FRAME_PERIOD: Duration = Duration::from_millis(25);
const BREAK_US: u64 = 150;
const MAB_US: u64 = 20;

pub type Frame = Arc<Mutex<[u8; 512]>>;

pub struct OpenDmx {
    status: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl OpenDmx {
    pub fn new(port: String, frame: Frame) -> Self {
        let status = Arc::new(Mutex::new("接続待ち".to_string()));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (status.clone(), stop.clone());
        let handle = std::thread::Builder::new().name("open-dmx".into()).spawn(move || run(&port, frame, s, st)).ok();
        OpenDmx { status, stop, handle }
    }

    pub fn status(&self) -> String {
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

fn spin(us: u64) {
    let t = Instant::now();
    let d = Duration::from_micros(us);
    while t.elapsed() < d {
        std::hint::spin_loop();
    }
}

fn open(name: &str) -> Result<Box<dyn serialport::SerialPort>, String> {
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
    Ok(p)
}

fn send(p: &mut Box<dyn serialport::SerialPort>, data: &[u8]) -> Result<(), String> {
    p.set_break().map_err(|e| e.to_string())?;
    spin(BREAK_US);
    p.clear_break().map_err(|e| e.to_string())?;
    spin(MAB_US);
    p.write_all(data).map_err(|e| e.to_string())
}

fn run(port: &str, frame: Frame, status: Arc<Mutex<String>>, stop: Arc<AtomicBool>) {
    let set = |s: String| *status.lock().unwrap_or_else(|e| e.into_inner()) = s;
    let mut link = None;
    let mut data = [0u8; 513]; // 先頭はスタートコード(0)
    while !stop.load(Ordering::SeqCst) {
        let Some(p) = link.as_mut() else {
            match open(port) {
                Ok(p) => {
                    set("出力中".into());
                    link = Some(p);
                }
                Err(e) => {
                    set(format!("未接続: {e}"));
                    for _ in 0..20 {
                        if stop.load(Ordering::SeqCst) {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
            }
            continue;
        };
        let t0 = Instant::now();
        data[1..].copy_from_slice(&*frame.lock().unwrap_or_else(|e| e.into_inner()));
        if let Err(e) = send(p, &data) {
            set(format!("切断: {e}"));
            link = None;
            continue;
        }
        if let Some(rest) = FRAME_PERIOD.checked_sub(t0.elapsed()) {
            std::thread::sleep(rest);
        }
    }
    // 終了時は全チャンネル0を一度送って消灯する
    if let Some(p) = link.as_mut() {
        let _ = send(p, &[0u8; 513]);
    }
}

/// シリアルポート(名前, 説明)。USB-DMX(FTDI)らしいものを先頭に
pub fn list_ports() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = serialport::available_ports()
        .unwrap_or_default()
        .into_iter()
        .filter(|p| !p.port_name.starts_with("/dev/tty.")) // Macは cu.* を使う
        .map(|p| {
            let desc = match p.port_type {
                serialport::SerialPortType::UsbPort(u) => u.product.unwrap_or_else(|| "USB".into()),
                _ => String::new(),
            };
            (p.port_name, desc)
        })
        .collect();
    v.sort_by_key(|(n, d)| !(n.contains("usbserial") || d.contains("FT232") || d.contains("DMX")));
    v
}
