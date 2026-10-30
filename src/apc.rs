//! AKAI APC mini mk2 の配置と、MIDIの入出力(Communications Protocol v1.0 より)。
//!
//! - パッド 64個: Note 0x00..=0x3F (左下が0x00、右上が0x3F)。LEDはNote On: ベロシティ=色、チャンネル=明るさ
//! - トラックボタン 8個: Note 0x64..=0x6B / シーンボタン 8個: Note 0x70..=0x77 / SHIFT: Note 0x7A
//!   (単色LED: ベロシティ 0=消灯 1=点灯 2=点滅)
//! - フェーダー 9本: CC 0x30..=0x38

use crate::engine::*;
use crate::fixture::COLORS;
use midir::{Ignore, MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};
use std::collections::HashMap;

// パレットの色(ベロシティ)
const C_WHITE: u8 = 3;
const C_RED: u8 = 5;
const C_ORANGE: u8 = 9;
const C_YELLOW: u8 = 13;
const C_GREEN: u8 = 21;
const C_LIGHT_BLUE: u8 = 37;
const C_BLUE: u8 = 45;
const C_VIOLET: u8 = 49;
const C_MAGENTA: u8 = 53;
const C_PINK: u8 = 57;
/// カラーホイールの色(COLORS と同じ並び)に近いパレットの色
const WHEEL_PADS: [u8; 8] = [C_WHITE, C_RED, C_GREEN, C_BLUE, C_YELLOW, C_LIGHT_BLUE, C_ORANGE, C_MAGENTA];
/// 明るさ(MIDIチャンネル): 選ばれている=100% / それ以外=25%
const LV_ON: u8 = 6;
const LV_IDLE: u8 = 1;

const MOTIONS: [Motion; 8] = [Motion::Still, Motion::Circle, Motion::Eight, Motion::PanSweep, Motion::TiltSweep, Motion::Cross, Motion::Random, Motion::Wave];
const SPREADS: [Spread; 3] = [Spread::Sync, Spread::Fan, Spread::Mirror];
const CMODES: [ColorMode; 5] = [ColorMode::Fixed, ColorMode::Beat, ColorMode::Bar, ColorMode::Split, ColorMode::Rainbow];
const GMODES: [GoboMode; 5] = [GoboMode::Fixed, GoboMode::Beat, GoboMode::Bar, GoboMode::Shake, GoboMode::Rotate];
const DIMS: [DimFx; 8] = [DimFx::Steady, DimFx::Pulse, DimFx::Chase, DimFx::Alternate, DimFx::Random, DimFx::Wave, DimFx::FastChase, DimFx::Build];

/// パッドの割り当て。row は上から 0..8、col は左から 0..8
pub fn pad(row: usize, col: usize) -> Option<Action> {
    let c = col as u8;
    Some(match (row, col) {
        (0, _) => Action::Motion(MOTIONS[col]),
        (1, 0..=4) => Action::Rate(c),
        (1, _) => Action::Spread(SPREADS[col - 5]),
        (2, _) => Action::Color(c),
        (3, 0..=4) => Action::ColorMode(CMODES[col]),
        (3, _) => Action::Position(c - 5),
        (4, _) => Action::Gobo(c),
        (5, 0..=4) => Action::GoboMode(GMODES[col]),
        (5, 5) => return None,
        (5, 6) => Action::Freeze,
        (5, 7) => Action::Auto,
        (6, _) => Action::Dim(DIMS[col]),
        (7, _) => [Action::Flash, Action::Strobe, Action::StrobeFast, Action::Blackout, Action::Tap, Action::Downbeat, Action::Half, Action::Double][col],
        _ => return None,
    })
}

/// トラックボタン(下の段、左から 0..8)
pub fn track(i: usize) -> Option<Action> {
    (i < N).then_some(Action::Fixture(i as u8))
}

pub fn note_action(note: u8) -> Option<Action> {
    match note {
        0x00..=0x3F => pad(7 - note as usize / 8, note as usize % 8),
        0x64..=0x6B => track((note - 0x64) as usize),
        0x70..=0x77 => Some(Action::Scene(note - 0x70)),
        0x7A => Some(Action::Shift),
        _ => None,
    }
}

pub fn pad_note(row: usize, col: usize) -> u8 {
    ((7 - row) * 8 + col) as u8
}

/// 画面とLEDで使う色(パレット番号)
pub fn palette(a: Action) -> u8 {
    match a {
        Action::Motion(_) | Action::Freeze => C_LIGHT_BLUE,
        Action::Rate(_) => C_BLUE,
        Action::Spread(_) | Action::Half | Action::Double => C_VIOLET,
        Action::Color(i) => WHEEL_PADS[i as usize % 8],
        Action::ColorMode(_) => C_PINK,
        Action::Position(_) | Action::Auto | Action::Tap => C_GREEN,
        Action::Gobo(0) | Action::GoboMode(_) => C_WHITE,
        Action::Gobo(_) | Action::Downbeat => C_YELLOW,
        Action::Dim(_) => C_ORANGE,
        Action::Flash | Action::Strobe | Action::StrobeFast => C_WHITE,
        Action::Blackout | Action::Fixture(_) | Action::Shift => C_RED,
        Action::Scene(_) => C_GREEN,
    }
}

/// 画面表示用の色(RGB)
pub fn palette_rgb(v: u8) -> [u8; 3] {
    match v {
        C_WHITE => [235, 235, 235],
        C_RED => [255, 50, 50],
        C_ORANGE => [255, 140, 30],
        C_YELLOW => [250, 220, 40],
        C_GREEN => [50, 220, 80],
        C_LIGHT_BLUE => [70, 200, 255],
        C_BLUE => [60, 90, 255],
        C_VIOLET => [150, 90, 255],
        C_MAGENTA => [255, 50, 210],
        C_PINK => [255, 110, 170],
        _ => [128, 128, 128],
    }
}

pub fn label(a: Action) -> String {
    match a {
        Action::Motion(m) => match m {
            Motion::Still => "止める",
            Motion::Circle => "円",
            Motion::Eight => "8の字",
            Motion::PanSweep => "左右",
            Motion::TiltSweep => "上下",
            Motion::Cross => "クロス",
            Motion::Random => "ランダム",
            Motion::Wave => "ウェーブ",
        }
        .into(),
        Action::Rate(r) => RATES[r as usize % RATES.len()].1.into(),
        Action::Spread(s) => match s {
            Spread::Sync => "揃える",
            Spread::Fan => "ずらす",
            Spread::Mirror => "左右対称",
        }
        .into(),
        Action::Color(i) => COLORS[i as usize % 8].0.into(),
        Action::ColorMode(m) => match m {
            ColorMode::Fixed => "色 固定",
            ColorMode::Beat => "色 拍ごと",
            ColorMode::Bar => "色 小節",
            ColorMode::Split => "色 バラバラ",
            ColorMode::Rainbow => "レインボー",
        }
        .into(),
        Action::Position(p) => POSITION_NAMES[p as usize % 3].into(),
        Action::Gobo(0) => "オープン".into(),
        Action::Gobo(g) => format!("ゴボ{g}"),
        Action::GoboMode(m) => match m {
            GoboMode::Fixed => "ゴボ 固定",
            GoboMode::Beat => "ゴボ 拍ごと",
            GoboMode::Bar => "ゴボ 小節",
            GoboMode::Shake => "シェイク",
            GoboMode::Rotate => "ゴボ回転",
        }
        .into(),
        Action::Freeze => "動き停止".into(),
        Action::Auto => "オート".into(),
        Action::Dim(d) => match d {
            DimFx::Steady => "点灯",
            DimFx::Pulse => "パルス",
            DimFx::Chase => "チェイス",
            DimFx::Alternate => "交互",
            DimFx::Random => "ランダム光",
            DimFx::Wave => "明暗の波",
            DimFx::FastChase => "チェイス×2",
            DimFx::Build => "ビルド",
        }
        .into(),
        Action::Flash => "フラッシュ".into(),
        Action::Strobe => "ストロボ".into(),
        Action::StrobeFast => "ストロボ速".into(),
        Action::Blackout => "暗転".into(),
        Action::Tap => "TAP".into(),
        Action::Downbeat => "1拍目".into(),
        Action::Half => "BPM ÷2".into(),
        Action::Double => "BPM ×2".into(),
        Action::Fixture(i) => format!("{}台目", i + 1),
        Action::Scene(i) => format!("シーン{}", i + 1),
        Action::Shift => "SHIFT".into(),
    }
}

/// 今の状態から、全LEDのMIDIメッセージ(ノート番号 → メッセージ)
pub fn leds(s: &State) -> Vec<(u8, [u8; 3])> {
    let mut v = Vec::with_capacity(80);
    for row in 0..8 {
        for col in 0..8 {
            let note = pad_note(row, col);
            v.push((
                note,
                match pad(row, col) {
                    Some(a) => [0x90 | if s.is_active(a) { LV_ON } else { LV_IDLE }, note, palette(a)],
                    None => [0x90, note, 0],
                },
            ));
        }
    }
    for i in 0..8u8 {
        let on = match track(i as usize) {
            Some(a) => s.is_active(a) as u8,
            // 右端は拍の表示
            None if i == 7 => s.is_active(Action::Tap) as u8,
            None => 0,
        };
        v.push((0x64 + i, [0x90, 0x64 + i, on]));
        let scene = if s.active_scene == Some(i) { 2 } else { s.setup.scenes[i as usize].is_some() as u8 };
        v.push((0x70 + i, [0x90, 0x70 + i, scene]));
    }
    v
}

/// Introduction Message。これを送ると、9本のフェーダーの現在位置が返ってくる
const INTRO: [u8; 12] = [0xF0, 0x47, 0x7F, 0x4F, 0x60, 0x00, 0x04, 0x00, 0x01, 0x00, 0x00, 0xF7];

/// フェーダー位置の応答(SysEx 0x61)。長さフィールドは信用せず、0xF7までの9個を読む
fn parse_fader_report(bytes: &[u8]) -> Option<[u8; 9]> {
    let body = bytes.strip_prefix(&[0xF0, 0x47, 0x7F, 0x4F, 0x61])?.strip_suffix(&[0xF7])?;
    body.get(2..11)?.try_into().ok()
}

#[derive(Debug, PartialEq)]
pub enum Input {
    Button(Action, bool),
    Fader(usize, f32),
    Faders([f32; 9]),
}

pub fn parse(bytes: &[u8]) -> Option<Input> {
    if let Some(f) = parse_fader_report(bytes) {
        return Some(Input::Faders(f.map(|v| v as f32 / 127.0)));
    }
    match *bytes {
        [s, note, vel, ..] if s & 0xF0 == 0x90 => Some(Input::Button(note_action(note)?, vel > 0)),
        [s, note, ..] if s & 0xF0 == 0x80 => Some(Input::Button(note_action(note)?, false)),
        [s, cc @ 0x30..=0x38, v, ..] if s & 0xF0 == 0xB0 => Some(Input::Fader((cc - 0x30) as usize, v as f32 / 127.0)),
        _ => None,
    }
}

/// APC mini mk2 の入力ポートを探す。Windowsでは「MIDIIN2 (APC mini mk2)」も見えるので、通常のポートを優先する
pub fn find_port(ports: &[String]) -> Option<String> {
    let is_apc = |p: &&String| p.to_lowercase().contains("apc mini");
    ports.iter().filter(is_apc).find(|p| !p.to_uppercase().contains("MIDIIN")).or_else(|| ports.iter().find(is_apc)).cloned()
}

pub fn input_ports() -> Vec<String> {
    let Ok(m) = MidiInput::new("spot4-list") else { return Vec::new() };
    m.ports().iter().filter_map(|p| m.port_name(p).ok()).collect()
}

/// LEDの送り先。変わったLEDだけ送る
#[derive(Default)]
pub struct LedOut {
    conn: Option<MidiOutputConnection>,
    sent: HashMap<u8, [u8; 3]>,
}

impl LedOut {
    pub fn sync(&mut self, s: &State) {
        let Some(c) = self.conn.as_mut() else { return };
        for (note, msg) in leds(s) {
            if self.sent.get(&note) != Some(&msg) && c.send(&msg).is_ok() {
                self.sent.insert(note, msg);
            }
        }
    }

    fn all_off(&mut self) {
        if let Some(c) = self.conn.as_mut() {
            for &note in self.sent.keys() {
                let _ = c.send(&[0x90, note, 0]);
            }
        }
    }
}

pub struct Connection {
    pub port: String,
    _input: MidiInputConnection<()>,
}

/// 入力と出力(LED)を開く。`on_input` はMIDIスレッドから呼ばれる
pub fn connect(port: &str, leds: &std::sync::Mutex<LedOut>, on_input: impl Fn(Input) + Send + 'static) -> Result<Connection, String> {
    let mut input = MidiInput::new("spot4").map_err(|e| e.to_string())?;
    input.ignore(Ignore::TimeAndActiveSense);
    let p = input.ports().into_iter().find(|p| input.port_name(p).is_ok_and(|n| n == port)).ok_or("入力ポートが見つかりません")?;
    let conn_in = input
        .connect(
            &p,
            "spot4-in",
            move |_, bytes, _| {
                if let Some(i) = parse(bytes) {
                    on_input(i);
                }
            },
            (),
        )
        .map_err(|e| e.to_string())?;

    // 出力ポートは名前が少し違うことがある(Windowsの「MIDIOUT2」など)ので、APCを含む名前で探す
    let output = MidiOutput::new("spot4").map_err(|e| e.to_string())?;
    let ports = output.ports();
    let names: Vec<String> = ports.iter().map(|p| output.port_name(p).unwrap_or_default()).collect();
    let idx = names.iter().position(|n| n == port).or_else(|| find_port(&names).and_then(|f| names.iter().position(|n| *n == f)));
    if let Some(i) = idx {
        let mut out = output.connect(&ports[i], "spot4-out").map_err(|e| e.to_string())?;
        let _ = out.send(&INTRO);
        let mut l = leds.lock().unwrap_or_else(|e| e.into_inner());
        l.all_off();
        *l = LedOut { conn: Some(out), sent: HashMap::new() };
    }
    Ok(Connection { port: port.to_string(), _input: conn_in })
}

pub fn disconnect(leds: &std::sync::Mutex<LedOut>) {
    let mut l = leds.lock().unwrap_or_else(|e| e.into_inner());
    l.all_off();
    *l = LedOut::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pad_has_unique_action() {
        let mut seen = std::collections::HashSet::new();
        for r in 0..8 {
            for c in 0..8 {
                if let Some(a) = pad(r, c) {
                    assert!(seen.insert(a), "{a:?} twice");
                    assert_eq!(note_action(pad_note(r, c)), Some(a));
                }
            }
        }
        assert_eq!(seen.len(), 63);
    }

    #[test]
    fn parses_notes_faders_and_report() {
        assert_eq!(parse(&[0x90, 0x00, 127]), Some(Input::Button(Action::Flash, true)));
        assert_eq!(parse(&[0x90, 0x00, 0]), Some(Input::Button(Action::Flash, false)));
        assert_eq!(parse(&[0x80, 0x38, 0]), Some(Input::Button(Action::Motion(Motion::Still), false)));
        assert_eq!(parse(&[0x90, 0x64, 127]), Some(Input::Button(Action::Fixture(0), true)));
        assert_eq!(parse(&[0x90, 0x6B, 127]), None);
        assert_eq!(parse(&[0x90, 0x7A, 127]), Some(Input::Button(Action::Shift, true)));
        assert_eq!(parse(&[0xB0, 0x38, 127]), Some(Input::Fader(8, 1.0)));
        assert_eq!(parse(&[0xB0, 0x07, 127]), None);
        let report = [0xF0, 0x47, 0x7F, 0x4F, 0x61, 0x00, 0x04, 0, 127, 0, 0, 0, 0, 0, 0, 127, 0xF7];
        match parse(&report) {
            Some(Input::Faders(f)) => assert_eq!((f[1], f[8]), (1.0, 1.0)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn leds_cover_all_controls() {
        let s = State::new(Setup::default(), 120.0);
        let l = leds(&s);
        assert_eq!(l.len(), 64 + 16);
        // 選ばれているパッドは明るく、それ以外は暗く
        let circle = pad_note(0, 1);
        let still = pad_note(0, 0);
        let get = |n: u8| l.iter().find(|(k, _)| *k == n).unwrap().1;
        assert_eq!(get(circle)[0] & 0x0F, LV_ON);
        assert_eq!(get(still)[0] & 0x0F, LV_IDLE);
    }

    #[test]
    fn finds_apc_port() {
        let ports = vec!["IAC".to_string(), "MIDIIN2 (APC mini mk2)".to_string(), "APC mini mk2".to_string()];
        assert_eq!(find_port(&ports).as_deref(), Some("APC mini mk2"));
    }
}
