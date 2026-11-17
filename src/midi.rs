//! MIDI入力(パッド/フェーダー)。APC mini mk2 の場合は、出力ポートも開いてフェーダーの現在位置を読み取る。

use crate::apcmini;
use crate::engine::{lock, ClockMsg, MidiMsg, Shared};
use crate::led::{LedSync, Out};
use crate::project::TriggerKind;
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput};
use std::time::Instant;

pub fn list_ports() -> Vec<String> {
    let Ok(input) = MidiInput::new("dmxctl-list") else { return Vec::new() };
    input.ports().iter().filter_map(|p| input.port_name(p).ok()).collect()
}

pub fn parse(bytes: &[u8]) -> Option<MidiMsg> {
    let (&status, rest) = bytes.split_first()?;
    let channel = status & 0x0F;
    match (status & 0xF0, rest) {
        (0x90, &[note, vel, ..]) => Some(MidiMsg { kind: TriggerKind::Note, channel, number: note, value: vel }),
        (0x80, &[note, _, ..]) => Some(MidiMsg { kind: TriggerKind::Note, channel, number: note, value: 0 }),
        (0xB0, &[cc, val, ..]) => Some(MidiMsg { kind: TriggerKind::Cc, channel, number: cc, value: val }),
        _ => None,
    }
}

/// MIDIクロック系のメッセージ(1バイト)
pub fn parse_clock(bytes: &[u8]) -> Option<ClockMsg> {
    match bytes.first()? {
        0xF8 => Some(ClockMsg::Tick),
        0xFA => Some(ClockMsg::Start),
        0xFB => Some(ClockMsg::Continue),
        0xFC => Some(ClockMsg::Stop),
        _ => None,
    }
}

#[derive(Default)]
pub struct Midi {
    conn: Option<MidiInputConnection<()>>,
    /// APC mini mk2 へ送る用(フェーダー位置の要求)
    out: Option<Out>,
    /// パッドLEDを実機へ送るスレッド(APC mini mk2 につないでいる間だけ動く)
    led: Option<LedSync>,
    pub connected: Option<String>,
    /// MIDIクロック専用の入力(メインの入力とは別のポートから受け取るとき)
    clock: Option<MidiInputConnection<()>>,
    pub clock_connected: Option<String>,
}

impl Midi {
    /// `ctx`は入力があった瞬間に画面を更新させるためのもの(普段は低頻度の更新で済ませる)
    pub fn connect(&mut self, core: &Shared, ctx: eframe::egui::Context, name: &str) -> Result<(), String> {
        self.disconnect();
        let mut input = MidiInput::new("dmxctl").map_err(|e| e.to_string())?;
        // フェーダー位置の応答(SysEx)とMIDIクロックを受け取るため、無視するのはアクティブセンシングだけ
        input.ignore(Ignore::ActiveSense);
        let port = input
            .ports()
            .into_iter()
            .find(|p| input.port_name(p).is_ok_and(|n| n == name))
            .ok_or_else(|| format!("MIDIポートが見つかりません: {name}"))?;
        let core2 = core.clone();
        let ctx2 = ctx.clone();
        let conn = input
            .connect(
                &port,
                "dmxctl-in",
                move |_ts, bytes, _| {
                    // クロックは1拍に24回来るので、画面は更新させない
                    if let Some(k) = parse_clock(bytes) {
                        lock(&core2).clock_msg(k, Instant::now());
                    } else if let Some(m) = parse(bytes) {
                        lock(&core2).live(crate::live::LiveOp::Midi { kind: m.kind, channel: m.channel, number: m.number, value: m.value });
                        ctx2.request_repaint();
                    } else if let Some(f) = apcmini::parse_fader_report(bytes) {
                        lock(&core2).live(crate::live::LiveOp::FaderPositions { positions: f });
                        ctx2.request_repaint();
                    }
                },
                (),
            )
            .map_err(|e| e.to_string())?;
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
        let output = MidiOutput::new("dmxctl-out").map_err(|e| e.to_string())?;
        let port = output
            .ports()
            .into_iter()
            .find(|p| output.port_name(p).is_ok_and(|n| n == name))
            .ok_or_else(|| "出力ポートが見つかりません".to_string())?;
        let out: Out = std::sync::Arc::new(std::sync::Mutex::new(output.connect(&port, "dmxctl-out").map_err(|e| e.to_string())?));
        self.out = Some(out.clone());
        self.request_faders()?;
        // 初期化(Introduction Message)のあとで、パッドのLED表示を始める
        self.led = Some(LedSync::spawn(core.clone(), out));
        Ok(())
    }

    /// APC mini mk2 に9本のフェーダーの現在位置を送らせる(応答は入力側のコールバックで受け取る)
    pub fn request_faders(&mut self) -> Result<(), String> {
        match self.out.as_mut() {
            Some(o) => o.lock().unwrap_or_else(|e| e.into_inner()).send(&apcmini::intro_message()).map_err(|e| e.to_string()),
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
        let mut input = MidiInput::new("dmxctl-clock").map_err(|e| e.to_string())?;
        input.ignore(Ignore::SysexAndActiveSense);
        let port = input
            .ports()
            .into_iter()
            .find(|p| input.port_name(p).is_ok_and(|n| n == name))
            .ok_or_else(|| format!("MIDIポートが見つかりません: {name}"))?;
        let core2 = core.clone();
        let conn = input
            .connect(
                &port,
                "dmxctl-clock-in",
                move |_ts, bytes, _| {
                    if let Some(k) = parse_clock(bytes) {
                        lock(&core2).clock_msg(k, Instant::now());
                    }
                },
                (),
            )
            .map_err(|e| format!("{name} を開けません(他のアプリが使用中かもしれません): {e}"))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_note_and_cc() {
        let m = parse(&[0x99, 36, 100]).unwrap();
        assert_eq!((m.kind, m.channel, m.number, m.value), (TriggerKind::Note, 9, 36, 100));
        // ベロシティ0のノートオンはノートオフ扱い(値0)
        assert_eq!(parse(&[0x90, 60, 0]).unwrap().value, 0);
        assert_eq!(parse(&[0x80, 60, 64]).unwrap().value, 0);
        let c = parse(&[0xB2, 7, 127]).unwrap();
        assert_eq!((c.kind, c.channel, c.number, c.value), (TriggerKind::Cc, 2, 7, 127));
        assert!(parse(&[0xF8]).is_none());
        assert!(parse(&[]).is_none());
    }

    #[test]
    fn parses_clock_messages() {
        assert_eq!(parse_clock(&[0xF8]), Some(ClockMsg::Tick));
        assert_eq!(parse_clock(&[0xFA]), Some(ClockMsg::Start));
        assert_eq!(parse_clock(&[0xFB]), Some(ClockMsg::Continue));
        assert_eq!(parse_clock(&[0xFC]), Some(ClockMsg::Stop));
        assert_eq!(parse_clock(&[0x90, 1, 2]), None);
        assert_eq!(parse_clock(&[]), None);
    }
}
