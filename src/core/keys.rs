//! パソコンのキーボードの割り当て(MIDIコントローラーが無いとき用)。
//! キーは位置で決める(日本語入力中や、JIS/USの違いがあっても同じ場所のキー)。
//! 番号 = 段 × 16 + 左からの位置。段: 0=数字 / 1=QWERTY / 2=ASDF / 3=ZXCV。
//! Shiftを押しながらのキーは別の割り当て(色の段): 番号に SHIFT を足す

use eframe::egui::Key;

/// 1段に並べるキーの数
pub const COLS: usize = 10;

/// 段ごとのキーと、キーに書いてある文字
pub const ROWS: [[(Key, &str); COLS]; 4] = [
    [(Key::Num1, "1"), (Key::Num2, "2"), (Key::Num3, "3"), (Key::Num4, "4"), (Key::Num5, "5"), (Key::Num6, "6"), (Key::Num7, "7"), (Key::Num8, "8"), (Key::Num9, "9"), (Key::Num0, "0")],
    [(Key::Q, "Q"), (Key::W, "W"), (Key::E, "E"), (Key::R, "R"), (Key::T, "T"), (Key::Y, "Y"), (Key::U, "U"), (Key::I, "I"), (Key::O, "O"), (Key::P, "P")],
    [(Key::A, "A"), (Key::S, "S"), (Key::D, "D"), (Key::F, "F"), (Key::G, "G"), (Key::H, "H"), (Key::J, "J"), (Key::K, "K"), (Key::L, "L"), (Key::Semicolon, ";")],
    [(Key::Z, "Z"), (Key::X, "X"), (Key::C, "C"), (Key::V, "V"), (Key::B, "B"), (Key::N, "N"), (Key::M, "M"), (Key::Comma, ","), (Key::Period, "."), (Key::Slash, "/")],
];

/// 段の名前(画面の左に出す)
pub const ROW_NAMES: [&str; 4] = ["LEDバーの模様", "ムービングの動き", "1周の長さ・揃え方", "アクセント・ムービングの向き"];

/// Shiftを押しながらのキー(色の段)は、番号にこれを足す
pub const SHIFT: u8 = 64;

/// Shiftを押しているときの段の名前
pub const SHIFT_ROW_NAMES: [&str; 4] = ["メイン色", "サブ色", "色の入れ替え", "色の系統"];

/// 段と位置から、キーの番号
pub const fn code(row: usize, col: usize) -> u8 {
    (row * 16 + col) as u8
}

/// キーから番号(割り当てのないキーはNone)
pub fn from_key(k: Key) -> Option<u8> {
    ROWS.iter().enumerate().find_map(|(r, row)| row.iter().position(|(x, _)| *x == k).map(|c| code(r, c)))
}

/// 番号から、キーに書いてある文字
pub fn name(code: u8) -> String {
    let base = code % SHIFT;
    let (r, c) = ((base / 16) as usize, (base % 16) as usize);
    let cap = ROWS.get(r).and_then(|row| row.get(c)).map(|x| x.1).unwrap_or("?");
    if code >= SHIFT { format!("Shift+{cap}") } else { cap.to_string() }
}

/// 割り当てとは別に、決まった動作をする特別なキー(画面の説明用)
pub const SPECIAL: [(&str, &str); 6] = [
    ("Shift+キー", "色(押している間、絵も色の段に変わる)"),
    ("Space", "TAP(タップテンポ)"),
    ("Enter", "頭出し(小節の頭へ)"),
    ("Esc", "ブラックアウト ON/OFF"),
    ("← / →", "APCのページ"),
    (if cfg!(target_os = "macos") { "⌘S" } else { "Ctrl+S" }, "保存"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for (r, row) in ROWS.iter().enumerate() {
            for (c, (k, label)) in row.iter().enumerate() {
                assert_eq!(from_key(*k), Some(code(r, c)));
                assert_eq!(name(code(r, c)), *label);
                assert_eq!(name(code(r, c) + SHIFT), format!("Shift+{label}"));
            }
        }
        assert_eq!(from_key(Key::Space), None);
    }
}
