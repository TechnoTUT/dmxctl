//! AKAI APC mini mk2 のコントロール配置(Communications Protocol v1.0 より)。
//!
//! 入力側(コントローラー → PC)のメッセージだけを扱う:
//! - パッド 64個: Note 0x00..=0x3F (左下が0x00、右上が0x3F)
//! - トラックボタン 8個: Note 0x64..=0x6B / シーンボタン 8個: Note 0x70..=0x77 / SHIFT: Note 0x7A
//! - フェーダー 9本(8本 + マスター): CC 0x30..=0x38 (値は絶対位置)
//!
//! すべてチャンネル0(Session View時)

use crate::project::TriggerKind;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Ctl {
    /// Note番号 0..=63
    Pad(u8),
    /// 0..=7 (左から)
    Track(u8),
    /// 0..=7 (上から)
    Scene(u8),
    Shift,
    /// 0..=8 (左から。8はマスター)
    Fader(u8),
}

impl Ctl {
    pub fn trigger(self) -> (TriggerKind, u8) {
        match self {
            Ctl::Pad(n) => (TriggerKind::Note, n),
            Ctl::Track(i) => (TriggerKind::Note, 0x64 + i),
            Ctl::Scene(i) => (TriggerKind::Note, 0x70 + i),
            Ctl::Shift => (TriggerKind::Note, 0x7A),
            Ctl::Fader(i) => (TriggerKind::Cc, 0x30 + i),
        }
    }

    pub fn from_trigger(kind: TriggerKind, number: u8) -> Option<Ctl> {
        match (kind, number) {
            (TriggerKind::Note, 0x00..=0x3F) => Some(Ctl::Pad(number)),
            (TriggerKind::Note, 0x64..=0x6B) => Some(Ctl::Track(number - 0x64)),
            (TriggerKind::Note, 0x70..=0x77) => Some(Ctl::Scene(number - 0x70)),
            (TriggerKind::Note, 0x7A) => Some(Ctl::Shift),
            (TriggerKind::Cc, 0x30..=0x38) => Some(Ctl::Fader(number - 0x30)),
            _ => None,
        }
    }

    /// 割り当てが「このコントロールのもの」か。チャンネル指定なし(全ch)またはch1のとき該当
    pub fn matches(self, t: &crate::project::MidiTrigger) -> bool {
        (t.kind, t.number) == self.trigger() && t.channel.is_none_or(|c| c == 0)
    }

    pub fn label(self) -> String {
        let (_, n) = self.trigger();
        match self {
            Ctl::Pad(n) => format!("パッド 左から{}列・下から{}行 (Note {n} / 0x{n:02X})", n % 8 + 1, n / 8 + 1),
            Ctl::Track(i) => format!("トラックボタン {} (Note {n} / 0x{n:02X})", i + 1),
            Ctl::Scene(i) => format!("シーンボタン {} (Note {n} / 0x{n:02X})", i + 1),
            Ctl::Shift => format!("SHIFT (Note {n} / 0x{n:02X})"),
            Ctl::Fader(8) => format!("マスターフェーダー (CC {n} / 0x{n:02X})"),
            Ctl::Fader(i) => format!("フェーダー {} (CC {n} / 0x{n:02X})", i + 1),
        }
    }

    /// 割り当ての名前の初期値
    pub fn short_name(self) -> String {
        match self {
            Ctl::Pad(n) => format!("パッド{}", n),
            Ctl::Track(i) => format!("トラック{}", i + 1),
            Ctl::Scene(i) => format!("シーン{}", i + 1),
            Ctl::Shift => "SHIFT".into(),
            Ctl::Fader(8) => "マスターフェーダー".into(),
            Ctl::Fader(i) => format!("フェーダー{}", i + 1),
        }
    }
}

/// Introduction Message(SysEx 0x60)。これを送ると、デバイスが9本のフェーダーの現在位置を返す(SysEx 0x61)。
/// アプリのバージョンは 1.0.0 として通知する
pub fn intro_message() -> Vec<u8> {
    vec![0xF0, 0x47, 0x7F, 0x4F, 0x60, 0x00, 0x04, 0x00, 0x01, 0x00, 0x00, 0xF7]
}

/// Introduction Messageへの応答(SysEx 0x61)から、フェーダー1..9の位置を取り出す。
/// 仕様書は長さフィールドを 0x04 と書いているが、実際は9個の値が続くので、長さは信用せず0xF7までを読む
pub fn parse_fader_report(bytes: &[u8]) -> Option<[u8; 9]> {
    let body = bytes.strip_prefix(&[0xF0, 0x47, 0x7F, 0x4F, 0x61])?;
    let body = body.strip_suffix(&[0xF7])?;
    let data = body.get(2..)?; // 長さ(MSB, LSB)を飛ばす
    let mut out = [0u8; 9];
    out.copy_from_slice(data.get(..9)?);
    Some(out)
}

/// 「APC mini」系のポートか
pub fn is_apc_port(name: &str) -> bool {
    name.to_lowercase().contains("apc mini")
}

/// 画面上の行(0=最上段)と列(0=左端)から、そのパッドのNote番号。最上段が0x38..=0x3F、最下段が0x00..=0x07
pub fn pad_note(row_from_top: u8, col: u8) -> u8 {
    (7 - row_from_top) * 8 + col
}

/// MIDI入力ポート名の中からAPC mini mk2 を探す。
/// Windowsでは2つ目のポートが「MIDIIN2 (APC mini mk2)」の名前で見える(Note Mode用)ので、通常のポートを優先する
pub fn find_port(ports: &[String]) -> Option<String> {
    let is_apc = |p: &&String| p.to_lowercase().contains("apc mini");
    ports.iter().filter(is_apc).find(|p| !p.to_uppercase().contains("MIDIIN")).or_else(|| ports.iter().find(is_apc)).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::MidiTrigger;

    #[test]
    fn pad_layout_matches_protocol_diagram() {
        // プロトコルPDFの図: 上段左から 0x38..0x3F、最下段 0x00..0x07
        assert_eq!(pad_note(0, 0), 0x38);
        assert_eq!(pad_note(0, 7), 0x3F);
        assert_eq!(pad_note(3, 3), 0x20 + 3); // 上から4行目 = 0x20..0x27
        assert_eq!(pad_note(7, 0), 0x00);
        assert_eq!(pad_note(7, 7), 0x07);
    }

    #[test]
    fn every_control_roundtrips_and_is_unique() {
        let mut all: Vec<Ctl> = (0..64).map(Ctl::Pad).collect();
        all.extend((0..8).map(Ctl::Track));
        all.extend((0..8).map(Ctl::Scene));
        all.push(Ctl::Shift);
        all.extend((0..9).map(Ctl::Fader));
        assert_eq!(all.len(), 64 + 8 + 8 + 1 + 9);

        let mut seen = std::collections::HashSet::new();
        for c in all {
            let (k, n) = c.trigger();
            assert!(seen.insert((k == TriggerKind::Cc, n)), "重複: {c:?}");
            assert_eq!(Ctl::from_trigger(k, n), Some(c));
        }
        // 割り当てのない番号
        assert_eq!(Ctl::from_trigger(TriggerKind::Note, 0x40), None);
        assert_eq!(Ctl::from_trigger(TriggerKind::Note, 0x6C), None);
        assert_eq!(Ctl::from_trigger(TriggerKind::Cc, 0x39), None);
    }

    #[test]
    fn documented_note_numbers() {
        assert_eq!(Ctl::Track(0).trigger(), (TriggerKind::Note, 0x64));
        assert_eq!(Ctl::Track(7).trigger(), (TriggerKind::Note, 0x6B));
        assert_eq!(Ctl::Scene(0).trigger(), (TriggerKind::Note, 0x70));
        assert_eq!(Ctl::Scene(7).trigger(), (TriggerKind::Note, 0x77));
        assert_eq!(Ctl::Shift.trigger(), (TriggerKind::Note, 0x7A));
        assert_eq!(Ctl::Fader(0).trigger(), (TriggerKind::Cc, 0x30));
        assert_eq!(Ctl::Fader(8).trigger(), (TriggerKind::Cc, 0x38));
    }

    #[test]
    fn channel_filter_for_matching() {
        let any = MidiTrigger { kind: TriggerKind::Note, channel: None, number: 5 };
        let ch1 = MidiTrigger { kind: TriggerKind::Note, channel: Some(0), number: 5 };
        let ch10 = MidiTrigger { kind: TriggerKind::Note, channel: Some(9), number: 5 };
        assert!(Ctl::Pad(5).matches(&any));
        assert!(Ctl::Pad(5).matches(&ch1));
        assert!(!Ctl::Pad(5).matches(&ch10));
        assert!(!Ctl::Pad(6).matches(&any));
    }

    #[test]
    fn fader_report_parses_both_length_variants() {
        let mut msg = vec![0xF0, 0x47, 0x7F, 0x4F, 0x61, 0x00, 0x09];
        msg.extend([0, 10, 20, 30, 40, 50, 60, 70, 127]);
        msg.push(0xF7);
        assert_eq!(parse_fader_report(&msg), Some([0, 10, 20, 30, 40, 50, 60, 70, 127]));
        // 仕様書どおりの長さ(0x04)が書かれていても読める
        msg[6] = 0x04;
        assert_eq!(parse_fader_report(&msg).map(|f| f[8]), Some(127));
        // 別のメッセージや短すぎるものは無視
        assert_eq!(parse_fader_report(&[0xF0, 0x47, 0x7F, 0x4F, 0x60, 0x00, 0x04, 0xF7]), None);
        assert_eq!(parse_fader_report(&[0xF0, 0x47, 0x7F, 0x4F, 0x61, 0x00, 0x04, 1, 2, 3, 0xF7]), None);
    }

    #[test]
    fn intro_message_matches_protocol() {
        let m = intro_message();
        assert_eq!(&m[..5], &[0xF0, 0x47, 0x7F, 0x4F, 0x60]);
        assert_eq!((m[5], m[6]), (0x00, 0x04)); // データ長 4
        assert_eq!(m.len(), 5 + 2 + 4 + 1);
        assert_eq!(*m.last().unwrap(), 0xF7);
    }

    #[test]
    fn finds_apc_port_preferring_the_main_one() {
        let ports = vec!["Microsoft GS Wavetable".to_string(), "MIDIIN2 (APC mini mk2)".to_string(), "APC mini mk2".to_string()];
        assert_eq!(find_port(&ports).as_deref(), Some("APC mini mk2"));
        let only2 = vec!["MIDIIN2 (APC mini mk2)".to_string()];
        assert_eq!(find_port(&only2).as_deref(), Some("MIDIIN2 (APC mini mk2)"));
        assert_eq!(find_port(&["loopMIDI".to_string()]), None);
    }
}

// ---------------------------------------------------------------- パッド・ボタンのLED

// パレットの色(ベロシティ)。仕様書の「Velocity to RGB Color Chart」より
pub const C_WHITE: u8 = 3;
pub const C_RED: u8 = 5;
pub const C_ORANGE: u8 = 9;
pub const C_YELLOW: u8 = 13;
pub const C_GREEN: u8 = 21;
pub const C_LIGHT_BLUE: u8 = 37;
pub const C_BLUE: u8 = 45;
pub const C_VIOLET: u8 = 49;
pub const C_MAGENTA: u8 = 53;
pub const C_PINK: u8 = 57;

/// 明るさ(MIDIチャンネル)
pub const LV_100: u8 = 6;
/// 待機中の明るさの選択肢: 10% / 25% / 50%
pub const IDLE_LEVELS: [u8; 3] = [0, 1, 2];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Led {
    Off,
    /// パッド: 色(パレット番号)と明るさ(MIDIチャンネル)
    Rgb { color: u8, level: u8 },
    /// ボタン: 1=点灯 / 2=点滅
    Single(u8),
    /// パッド: 実際の色(RGB各0〜255。明るさは値に含める)。full=効いている(最大の明るさ)
    True { rgb: [u8; 3], full: bool },
}

impl Led {
    /// 送るMIDIメッセージ。パレットの色はNote On、実際の色はSysEx(RGB LED Color Lighting, 0x24)
    pub fn message(self, note: u8) -> Vec<u8> {
        match self {
            Led::Off => vec![0x90, note, 0],
            Led::Rgb { color, level } => vec![0x90 | (level & 0x0F), note, color],
            Led::Single(v) => vec![0x90, note, v],
            Led::True { rgb, .. } => {
                // 開始パッド・終了パッド + RGBを各8bit(上位1bit/下位7bit)= 8バイト
                let mut m = vec![0xF0, 0x47, 0x7F, 0x4F, 0x24, 0x00, 0x08, note, note];
                for c in rgb {
                    m.push(c >> 7);
                    m.push(c & 0x7F);
                }
                m.push(0xF7);
                m
            }
        }
    }

    /// 効いている(最大の明るさで光っている)か
    pub fn is_full(self) -> bool {
        match self {
            Led::Rgb { level, .. } => level == LV_100,
            Led::True { full, .. } => full,
            Led::Single(v) => v == 2,
            Led::Off => false,
        }
    }
}
