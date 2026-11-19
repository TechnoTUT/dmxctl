//! パソコンのキーボードの割り当て(MIDIコントローラーが無いとき用)。
//! キーは位置で決める(日本語入力中や、JIS/USの違いがあっても同じ場所のキー)。
//! 番号 = 段 × 16 + 左からの位置。段: 0=数字 / 1=QWERTY / 2=ASDF / 3=ZXCV。
//! Shiftを押しながらのキーは別の割り当て(色の段): 番号に SHIFT を足す

/// 1段に並べるキーの数
pub const COLS: usize = 10;

/// 段ごとの、キーに書いてある文字。キーそのものとの対応は、画面の側 `ui::keys` にある
pub const ROWS: [[&str; COLS]; 4] = [
    ["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"],
    ["Q", "W", "E", "R", "T", "Y", "U", "I", "O", "P"],
    ["A", "S", "D", "F", "G", "H", "J", "K", "L", ";"],
    ["Z", "X", "C", "V", "B", "N", "M", ",", ".", "/"],
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

/// 番号から、キーに書いてある文字
pub fn name(code: u8) -> String {
    let base = code % SHIFT;
    let (r, c) = ((base / 16) as usize, (base % 16) as usize);
    let cap = ROWS.get(r).and_then(|row| row.get(c)).copied().unwrap_or("?");
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
    fn names_follow_the_labels() {
        for (r, row) in ROWS.iter().enumerate() {
            for (c, label) in row.iter().enumerate() {
                assert_eq!(name(code(r, c)), *label);
                assert_eq!(name(code(r, c) + SHIFT), format!("Shift+{label}"));
            }
        }
        assert_eq!(name(code(0, COLS)), "?");
    }
}
