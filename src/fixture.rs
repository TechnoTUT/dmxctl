//! STAGE EVOLUTION MINI SPOT 30 のDMXチャート(取扱説明書より)。
//!
//! 9chモード:  1 Pan / 2 Tilt / 3 カラー / 4 ゴボ / 5 ストロボ / 6 調光 / 7 Pan・Tiltスピード / 8 動作モード / 9 ムーブ・リセット
//! 11chモード: 1 Pan / 2 Pan Fine / 3 Tilt / 4 Tilt Fine / 5〜11 は9chモードの3〜9と同じ
//!
//! 8ch(動作モード)は 0〜59 で「DMX制御」、9ch(ムーブ/リセット)は 250〜255 で「リセット」になるので、
//! このアプリでは両方とも常に0を送る。

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Mode {
    Ch9,
    Ch11,
}

impl Mode {
    pub fn channels(self) -> usize {
        match self {
            Mode::Ch9 => 9,
            Mode::Ch11 => 11,
        }
    }
}

/// カラーホイールの単色(名前, DMX値=範囲の中央, 画面表示の色)。並びはAPCのパッドの並び
pub const COLORS: [(&str, u8, [u8; 3]); 8] = [
    ("白", 5, [255, 255, 255]),
    ("赤", 15, [255, 30, 30]),
    ("緑", 25, [40, 230, 60]),
    ("青", 35, [40, 80, 255]),
    ("黄", 45, [255, 230, 40]),
    ("水色", 55, [80, 200, 255]),
    ("オレンジ", 65, [255, 140, 20]),
    ("マゼンタ", 75, [255, 40, 200]),
];

/// ゴボ: 0=オープン、1〜7=ゴボ1〜7(DMX値は範囲の中央)
pub const GOBOS: [u8; 8] = [3, 11, 20, 28, 35, 43, 51, 59];

/// レインボー(カラーホイール連続回転)。speed 0..1 = 遅い→速い
pub fn rainbow(speed: f32) -> u8 {
    140 + (speed.clamp(0.0, 1.0) * 115.0).round() as u8
}

/// ゴボ1〜7のシェイク。speed 0..1 = 遅い→速い
pub fn gobo_shake(gobo: u8, speed: f32) -> u8 {
    let g = gobo.clamp(1, 7);
    72 + (g - 1) * 8 + (speed.clamp(0.0, 1.0) * 7.0).round() as u8
}

/// ゴボホイール回転。speed 0..1 = 遅い→速い
pub fn gobo_rotate(speed: f32) -> u8 {
    128 + (speed.clamp(0.0, 1.0) * 127.0).round() as u8
}

/// ストロボ。speed 0..1 = 遅い→速い(0はストロボなし)
pub fn strobe(speed: f32) -> u8 {
    10 + (speed.clamp(0.0, 1.0) * 239.0).round() as u8
}

/// 1台ぶんの出力
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Head {
    /// 0..1 (0.5が中央。Panは540°、Tiltは270°の範囲)
    pub pan: f32,
    pub tilt: f32,
    /// カラーchのDMX値
    pub color: u8,
    /// ゴボchのDMX値
    pub gobo: u8,
    /// ストロボchのDMX値(0=なし)
    pub strobe: u8,
    /// 0..1
    pub dimmer: f32,
    /// Pan/TiltスピードchのDMX値(0=最速)
    pub pt_speed: u8,
}

fn to8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn to16(v: f32) -> (u8, u8) {
    let x = (v.clamp(0.0, 1.0) * 65535.0).round() as u16;
    ((x >> 8) as u8, (x & 0xFF) as u8)
}

/// 1台ぶんのチャンネル値(`out` の長さは mode.channels())
pub fn encode(h: &Head, mode: Mode, out: &mut [u8]) {
    let rest = match mode {
        Mode::Ch9 => {
            out[0] = to8(h.pan);
            out[1] = to8(h.tilt);
            &mut out[2..9]
        }
        Mode::Ch11 => {
            (out[0], out[1]) = to16(h.pan);
            (out[2], out[3]) = to16(h.tilt);
            &mut out[4..11]
        }
    };
    rest[0] = h.color;
    rest[1] = h.gobo;
    rest[2] = h.strobe;
    rest[3] = to8(h.dimmer);
    rest[4] = h.pt_speed;
    rest[5] = 0; // 動作モード: DMX制御
    rest[6] = 0; // ムーブ/リセット: なし
}

/// 4台ぶんを1ユニバースに書き込む。`addresses` は1始まり。範囲外の台は書かない
pub fn write_universe(heads: &[Head; 4], mode: Mode, addresses: &[u16; 4], universe: &mut [u8; 512]) {
    let n = mode.channels();
    for (h, &a) in heads.iter().zip(addresses) {
        let start = a as usize;
        if start == 0 || start - 1 + n > 512 {
            continue;
        }
        encode(h, mode, &mut universe[start - 1..start - 1 + n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_stay_in_manual_ranges() {
        assert_eq!(rainbow(0.0), 140);
        assert_eq!(rainbow(1.0), 255);
        assert_eq!(gobo_shake(1, 0.0), 72);
        assert_eq!(gobo_shake(7, 1.0), 127);
        assert_eq!(gobo_rotate(0.0), 128);
        assert_eq!(strobe(0.0), 10);
        assert_eq!(strobe(1.0), 249);
        for (_, v, _) in COLORS {
            assert!(v < 80);
        }
    }

    #[test]
    fn encode_9ch_never_triggers_reset_or_auto() {
        let h = Head { pan: 1.0, tilt: 0.0, color: 15, gobo: 11, strobe: 0, dimmer: 1.0, pt_speed: 0 };
        let mut out = [0xEE; 9];
        encode(&h, Mode::Ch9, &mut out);
        assert_eq!(out, [255, 0, 15, 11, 0, 255, 0, 0, 0]);
    }

    #[test]
    fn encode_11ch_has_fine_channels() {
        let h = Head { pan: 0.5, tilt: 0.25, dimmer: 0.5, ..Default::default() };
        let mut out = [0xEE; 11];
        encode(&h, Mode::Ch11, &mut out);
        assert_eq!(&out[..4], &[0x80, 0x00, 0x40, 0x00]);
        assert_eq!(out[7], 128);
        assert_eq!(&out[9..], &[0, 0]);
    }

    #[test]
    fn universe_uses_addresses() {
        let heads = [Head { dimmer: 1.0, ..Default::default() }; 4];
        let mut u = [0u8; 512];
        write_universe(&heads, Mode::Ch9, &[1, 10, 19, 28], &mut u);
        for a in [1, 10, 19, 28] {
            assert_eq!(u[a - 1 + 5], 255, "address {a}");
        }
        // はみ出す台は書かない
        let mut u = [0u8; 512];
        write_universe(&heads, Mode::Ch9, &[505, 0, 504, 504], &mut u);
        assert_eq!(u[503 + 5], 255);
        assert!(u[..503].iter().all(|&v| v == 0));
    }
}
