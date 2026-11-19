//! MIDI のポートの一覧・入力・出力と、受け取ったバイト列の解釈。
//! 受け取った入力を何に使うかは `dmxctl-core` の `io::midi` が受け持つ。
use dmxctl_model::project::TriggerKind;
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use std::sync::{Arc, Mutex};

/// ノート・CC の1メッセージ
#[derive(Debug, Clone, Copy)]
pub struct MidiMsg {
    pub kind: TriggerKind,
    /// 0..15
    pub channel: u8,
    pub number: u8,
    /// ベロシティ/CC値。ノートオフは0
    pub value: u8,
}

/// MIDIクロック系のメッセージ
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClockMsg {
    Tick,
    Start,
    Continue,
    Stop,
}

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

/// ポートを開けなかった理由
#[derive(Debug)]
pub enum OpenError {
    /// その名前のポートが無い
    NotFound(String),
    /// ポートはあるが、開けなかった。ほかのアプリが使っているときなど
    Failed(String),
}

impl OpenError {
    pub fn message(&self, name: &str) -> String {
        match self {
            OpenError::NotFound(_) => format!("MIDIポートが見つかりません: {name}"),
            OpenError::Failed(e) => e.clone(),
        }
    }
}

/// 入力で、受け取らないメッセージの種類
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// アクティブセンシングだけ。フェーダー位置の応答(SysEx)とクロックは受け取る
    ActiveSense,
    /// SysEx とアクティブセンシング。クロックだけを受け取る
    SysexAndActiveSense,
}

/// 開いている入力。Dropで閉じる
pub struct MidiIn {
    _conn: MidiInputConnection<()>,
}

/// 名前で指定した入力を開く。`on_bytes` は受け取るたびに、MIDI のスレッドから呼ばれる
pub fn open_input(
    name: &str,
    client: &str,
    port_label: &str,
    skip: Skip,
    mut on_bytes: impl FnMut(&[u8]) + Send + 'static,
) -> Result<MidiIn, OpenError> {
    let mut input = MidiInput::new(client).map_err(|e| OpenError::Failed(e.to_string()))?;
    input.ignore(match skip {
        Skip::ActiveSense => Ignore::ActiveSense,
        Skip::SysexAndActiveSense => Ignore::SysexAndActiveSense,
    });
    let port = input
        .ports()
        .into_iter()
        .find(|p| input.port_name(p).is_ok_and(|n| n == name))
        .ok_or_else(|| OpenError::NotFound(name.to_string()))?;
    let conn = input
        .connect(&port, port_label, move |_ts, bytes, _| on_bytes(bytes), ())
        .map_err(|e| OpenError::Failed(e.to_string()))?;
    Ok(MidiIn { _conn: conn })
}

/// 開いている出力。複製して、複数のスレッドから送れる
#[derive(Clone)]
pub struct MidiOut(Arc<Mutex<MidiOutputConnection>>);

impl MidiOut {
    pub fn send(&self, bytes: &[u8]) -> Result<(), String> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).send(bytes).map_err(|e| e.to_string())
    }
}

/// 名前で指定した出力を開く
pub fn open_output(name: &str) -> Result<MidiOut, String> {
    let output = MidiOutput::new("dmxctl-out").map_err(|e| e.to_string())?;
    let port = output
        .ports()
        .into_iter()
        .find(|p| output.port_name(p).is_ok_and(|n| n == name))
        .ok_or_else(|| "出力ポートが見つかりません".to_string())?;
    let conn = output.connect(&port, "dmxctl-out").map_err(|e| e.to_string())?;
    Ok(MidiOut(Arc::new(Mutex::new(conn))))
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
