//! MIDI入力(パッド/フェーダー)。APC mini mk2 の場合は、出力ポートも開いてフェーダーの現在位置を読み取る。
//! ポートを開く部分は `dmxctl-io` の `midi`。ここは、受け取った入力で `Core` を動かす。

use crate::io::apcmini;
use crate::engine::{lock, Shared};
use crate::led::LedSync;
use dmxctl_io::midi::{self as port, MidiIn, MidiOut, OpenError, Skip};
use std::time::Instant;

pub use dmxctl_io::midi::{list_ports, parse, parse_clock, ClockMsg, MidiMsg};

#[derive(Default)]
pub struct Midi {
    conn: Option<MidiIn>,
    /// APC mini mk2 へ送る用(フェーダー位置の要求)
    out: Option<MidiOut>,
    /// パッドLEDを実機へ送るスレッド(APC mini mk2 につないでいる間だけ動く)
    led: Option<LedSync>,
    pub connected: Option<String>,
    /// MIDIクロック専用の入力(メインの入力とは別のポートから受け取るとき)
    clock: Option<MidiIn>,
    pub clock_connected: Option<String>,
}

impl Midi {
    /// `ctx`は入力があった瞬間に画面を更新させるためのもの(普段は低頻度の更新で済ませる)
    pub fn connect(&mut self, core: &Shared, ctx: crate::wake::Wake, name: &str) -> Result<(), String> {
        self.disconnect();
        let core2 = core.clone();
        // フェーダー位置の応答(SysEx)とMIDIクロックを受け取るため、無視するのはアクティブセンシングだけ
        let conn = port::open_input(name, "dmxctl", "dmxctl-in", Skip::ActiveSense, move |bytes| {
            // クロックは1拍に24回来るので、画面は更新させない
            if let Some(k) = parse_clock(bytes) {
                lock(&core2).clock_msg(k, Instant::now());
            } else if let Some(m) = parse(bytes) {
                lock(&core2).live(crate::live::LiveOp::Midi { kind: m.kind, channel: m.channel, number: m.number, value: m.value });
                ctx.request_repaint();
            } else if let Some(f) = apcmini::parse_fader_report(bytes) {
                lock(&core2).live(crate::live::LiveOp::FaderPositions { positions: f });
                ctx.request_repaint();
            }
        })
        .map_err(|e| e.message(name))?;
        self.conn = Some(conn);
        self.connected = Some(name.to_string());
        lock(core).rt.midi_status = format!("接続中: {name}");

        if apcmini::is_apc_port(name)
            && let Err(e) = self.open_output_and_request_faders(core, name) {
            lock(core).rt.midi_status = format!("接続中: {name} (フェーダー位置は動かすと反映: {e})");
        }
        Ok(())
    }

    fn open_output_and_request_faders(&mut self, core: &Shared, name: &str) -> Result<(), String> {
        let out = port::open_output(name)?;
        self.out = Some(out.clone());
        self.request_faders()?;
        // 初期化(Introduction Message)のあとで、パッドのLED表示を始める
        self.led = Some(LedSync::spawn(core.clone(), out));
        Ok(())
    }

    /// APC mini mk2 に9本のフェーダーの現在位置を送らせる(応答は入力側のコールバックで受け取る)
    pub fn request_faders(&mut self) -> Result<(), String> {
        match self.out.as_ref() {
            Some(o) => o.send(&apcmini::intro_message()),
            None => Err("出力ポートが開かれていません".into()),
        }
    }

    pub fn can_request_faders(&self) -> bool {
        self.out.is_some()
    }

    pub fn led_active(&self) -> bool {
        self.led.is_some()
    }

    /// MIDIクロックだけを受け取る入力を開く(映像ソフト→loopMIDI など)
    pub fn connect_clock(&mut self, core: &Shared, name: &str) -> Result<(), String> {
        self.disconnect_clock();
        let core2 = core.clone();
        let conn = port::open_input(name, "dmxctl-clock", "dmxctl-clock-in", Skip::SysexAndActiveSense, move |bytes| {
            if let Some(k) = parse_clock(bytes) {
                lock(&core2).clock_msg(k, Instant::now());
            }
        })
        .map_err(|e| match e {
            OpenError::NotFound(_) => e.message(name),
            OpenError::Failed(detail) => format!("{name} を開けません(他のアプリが使用中かもしれません): {detail}"),
        })?;
        self.clock = Some(conn);
        self.clock_connected = Some(name.to_string());
        Ok(())
    }

    pub fn disconnect_clock(&mut self) {
        self.clock = None;
        self.clock_connected = None;
    }

    pub fn disconnect(&mut self) {
        self.led = None; // 止まるときにパッドを消灯する
        self.conn = None; // Dropで切断される
        self.out = None;
        self.connected = None;
    }
}
