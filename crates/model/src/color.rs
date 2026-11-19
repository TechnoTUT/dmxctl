//! 色の全体制御: 「赤」「青」などの色を、機材ごとの仕組み(RGB / カラーホイール / 色レンジ)に合わせて設定する。

use crate::fixture::{Mode, Range};
use serde::{Deserialize, Serialize};

/// パッドに割り当てられる色
#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum NamedColor {
    Red,
    Orange,
    Yellow,
    Green,
    Cyan,
    Blue,
    Violet,
    Magenta,
    Pink,
    White,
}

impl NamedColor {
    pub const ALL: [NamedColor; 10] = [
        NamedColor::Red,
        NamedColor::Orange,
        NamedColor::Yellow,
        NamedColor::Green,
        NamedColor::Cyan,
        NamedColor::Blue,
        NamedColor::Violet,
        NamedColor::Magenta,
        NamedColor::Pink,
        NamedColor::White,
    ];

    pub fn label(self) -> &'static str {
        match self {
            NamedColor::Red => "赤",
            NamedColor::Orange => "オレンジ",
            NamedColor::Yellow => "黄",
            NamedColor::Green => "緑",
            NamedColor::Cyan => "水色",
            NamedColor::Blue => "青",
            NamedColor::Violet => "紫",
            NamedColor::Magenta => "マゼンタ",
            NamedColor::Pink => "ピンク",
            NamedColor::White => "白",
        }
    }

    /// 0.0〜1.0のRGB
    pub fn rgb(self) -> [f32; 3] {
        match self {
            NamedColor::Red => [1.0, 0.0, 0.0],
            NamedColor::Orange => [1.0, 0.5, 0.0],
            NamedColor::Yellow => [1.0, 1.0, 0.0],
            NamedColor::Green => [0.0, 1.0, 0.0],
            NamedColor::Cyan => [0.0, 0.75, 1.0],
            NamedColor::Blue => [0.0, 0.0, 1.0],
            NamedColor::Violet => [0.5, 0.0, 1.0],
            NamedColor::Magenta => [1.0, 0.0, 1.0],
            NamedColor::Pink => [1.0, 0.3, 0.55],
            NamedColor::White => [1.0, 1.0, 1.0],
        }
    }

    /// APC mini mk2 のパッドの色(パレット番号)
    pub fn led(self) -> u8 {
        use crate::apcmini::*;
        match self {
            NamedColor::Red => C_RED,
            NamedColor::Orange => C_ORANGE,
            NamedColor::Yellow => C_YELLOW,
            NamedColor::Green => C_GREEN,
            NamedColor::Cyan => C_LIGHT_BLUE,
            NamedColor::Blue => C_BLUE,
            NamedColor::Violet => C_VIOLET,
            NamedColor::Magenta => C_MAGENTA,
            NamedColor::Pink => C_PINK,
            NamedColor::White => C_WHITE,
        }
    }
}

/// 色相(0〜360度)・彩度・明度(0〜1)からRGB(0〜1)
pub fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h = h.rem_euclid(360.0) / 60.0;
    let (i, f) = (h.floor() as i32, h - h.floor());
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    match i {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

/// レンジ名が表す色(1色だけを表すものに限る。「マゼンタ/オレンジ」「レインボー」などは None)
pub fn label_rgb(label: &str) -> Option<[f32; 3]> {
    if label.contains('/') || label.contains("レインボー") || label.contains("チェンジ") || label.contains("フラッシュ") {
        return None;
    }
    let l = label.to_lowercase();
    // 「ライトブルー」は「青」より先に判定する
    const TABLE: [(&str, NamedColor); 22] = [
        ("マゼンタ", NamedColor::Magenta),
        ("ライトブルー", NamedColor::Cyan),
        ("シアン", NamedColor::Cyan),
        ("オレンジ", NamedColor::Orange),
        ("ピンク", NamedColor::Pink),
        ("紫", NamedColor::Violet),
        ("赤", NamedColor::Red),
        ("緑", NamedColor::Green),
        ("青", NamedColor::Blue),
        ("黄", NamedColor::Yellow),
        ("白", NamedColor::White),
        ("magenta", NamedColor::Magenta),
        ("cyan", NamedColor::Cyan),
        ("orange", NamedColor::Orange),
        ("pink", NamedColor::Pink),
        ("purple", NamedColor::Violet),
        ("violet", NamedColor::Violet),
        ("red", NamedColor::Red),
        ("green", NamedColor::Green),
        ("blue", NamedColor::Blue),
        ("yellow", NamedColor::Yellow),
        ("white", NamedColor::White),
    ];
    TABLE.iter().find(|(k, _)| l.contains(k)).map(|(_, c)| c.rgb())
}

/// この範囲が出す色。色を直接指定していればそれ、なければ表示名の色の名前から判定する
pub fn range_rgb(r: &Range) -> Option<[f32; 3]> {
    r.color.map(|c| [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0]).or_else(|| label_rgb(&r.label))
}

/// パレットの色のうち、`rgb`に一番近いもの(パッドのLEDの色を決めるため)
pub fn nearest_named(rgb: [f32; 3]) -> NamedColor {
    NamedColor::ALL.into_iter().min_by(|a, b| dist2(a.rgb(), rgb).partial_cmp(&dist2(b.rgb(), rgb)).unwrap_or(std::cmp::Ordering::Equal)).unwrap_or(NamedColor::White)
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum()
}

/// 機材に設定する値: (機能名attr, 値0〜255)
pub type ColorPlan = Vec<(String, f32)>;

/// このモードの機材に、`rgb`の色を出させるための設定。
/// RGBのチャンネルがあればそれを、なければ「color」チャンネルの色レンジのうち一番近いものを選ぶ。どちらも無ければ空
pub fn plan_for_mode(mode: &Mode, rgb: [f32; 3]) -> ColorPlan {
    // LEDバーは red.1, red.2 ... のようにピクセルごとにある(「red」への書き込みで全ピクセルにかかる)
    let has = |a: &str| mode.channels.iter().any(|c| crate::fixture::split_pixel(&c.attr).0 == a && c.fine_of.is_none());
    if has("red") && has("green") && has("blue") {
        let mut v = [rgb[0] * 255.0, rgb[1] * 255.0, rgb[2] * 255.0];
        let mut plan = ColorPlan::new();
        if has("white") {
            // RGBW: 共通する成分を白に回す
            let w = v[0].min(v[1]).min(v[2]);
            v = [v[0] - w, v[1] - w, v[2] - w];
            plan.push(("white".into(), w));
        }
        plan.push(("red".into(), v[0]));
        plan.push(("green".into(), v[1]));
        plan.push(("blue".into(), v[2]));
        return plan;
    }
    if let Some(ch) = mode.find_attr("color") {
        let def = &mode.channels[ch];
        let best = def
            .ranges
            .iter()
            .filter_map(|r| range_rgb(r).map(|c| (dist2(c, rgb), r)))
            .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        if let Some((_, r)) = best {
            return vec![("color".into(), r.mid() as f32)];
        }
    }
    Vec::new()
}

/// 操作の対象
#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ColorTarget {
    /// 「全体/個別」スイッチに従う
    Scope,
    /// 常に全機材
    All,
    /// 常に選択中の機材
    Selected,
    /// メイン色を決める(メイン色の機材に出る)
    Main,
    /// サブ色を決める(サブ色の機材に出る)
    Sub,
}

impl ColorTarget {
    pub const ALL: [ColorTarget; 5] = [ColorTarget::Scope, ColorTarget::All, ColorTarget::Selected, ColorTarget::Main, ColorTarget::Sub];
    pub fn label(self) -> &'static str {
        match self {
            ColorTarget::Scope => "全体/個別スイッチに従う",
            ColorTarget::All => "常に全機材",
            ColorTarget::Selected => "常に選択中の機材",
            ColorTarget::Main => "メイン色にする",
            ColorTarget::Sub => "サブ色にする",
        }
    }
    /// メイン/サブの系統を決める操作なら、その系統
    pub fn slot(self) -> Option<ColorRole> {
        match self {
            ColorTarget::Main => Some(ColorRole::Main),
            ColorTarget::Sub => Some(ColorRole::Sub),
            _ => None,
        }
    }
}

/// 機材がどちらの色の系統に従うか
#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ColorRole {
    #[default]
    Main,
    Sub,
    /// メイン/サブの色を出さない(手動や他の操作だけで色を決める)
    Off,
}

impl ColorRole {
    pub const ALL: [ColorRole; 3] = [ColorRole::Main, ColorRole::Sub, ColorRole::Off];
    pub fn label(self) -> &'static str {
        match self {
            ColorRole::Main => "メイン",
            ColorRole::Sub => "サブ",
            ColorRole::Off => "対象外",
        }
    }
}

/// メイン色とサブ色。機材ごとにどちらを出すかは`FixtureInstance::color_role`
#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Palette {
    /// 0.0〜1.0のRGB
    pub main: [f32; 3],
    pub sub: [f32; 3],
    /// 拍に合わせて、メインとサブを自動で入れ替える
    pub auto: bool,
    /// 自動入れ替えの間隔(拍)
    pub auto_beats: f32,
    /// メイン色⇄サブ色を、間隔の拍数をかけて少しずつ行き来する(グラデーション)。自動入れ替えとはどちらか一方
    pub fade: bool,
}

impl Default for Palette {
    fn default() -> Self {
        Palette { main: NamedColor::White.rgb(), sub: NamedColor::Blue.rgb(), auto: false, auto_beats: 4.0, fade: false }
    }
}

/// メイン/サブの系統の操作
#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PaletteOp {
    /// 押すたびに、メインとサブを入れ替える
    Swap,
    /// 押している間だけ入れ替える
    HoldSwap,
    /// 拍に合わせた自動入れ替えのON/OFF
    Auto,
    /// メイン⇄サブを少しずつ行き来する(グラデーション)のON/OFF
    Fade,
}

impl PaletteOp {
    pub const ALL: [PaletteOp; 4] = [PaletteOp::Swap, PaletteOp::HoldSwap, PaletteOp::Auto, PaletteOp::Fade];
    pub fn label(self) -> &'static str {
        match self {
            PaletteOp::Swap => "押すたびに メイン⇄サブ入れ替え",
            PaletteOp::HoldSwap => "押している間だけ 入れ替え",
            PaletteOp::Auto => "拍で自動入れ替え ON/OFF",
            PaletteOp::Fade => "グラデーションで行き来 ON/OFF",
        }
    }
}

/// RGBの色相(0〜360度)。無彩色は0
pub fn hue_of(c: [f32; 3]) -> f32 {
    let (max, min) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let d = max - min;
    if d <= 1e-6 {
        return 0.0;
    }
    let h = if max == c[0] {
        ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if max == c[1] {
        (c[2] - c[0]) / d + 2.0
    } else {
        (c[0] - c[1]) / d + 4.0
    };
    h * 60.0
}

pub fn same_rgb(a: [f32; 3], b: [f32; 3]) -> bool {
    dist2(a, b) < 1e-4
}

/// 「全体/個別」スイッチの操作
#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ScopeOp {
    /// 押すたびに 全体 ⇄ 個別
    Toggle,
    /// 押している間だけ「個別」
    HoldIndividual,
    Global,
    Individual,
}

impl ScopeOp {
    pub const ALL: [ScopeOp; 4] = [ScopeOp::Toggle, ScopeOp::HoldIndividual, ScopeOp::Global, ScopeOp::Individual];
    pub fn label(self) -> &'static str {
        match self {
            ScopeOp::Toggle => "押すたびに 全体⇄個別",
            ScopeOp::HoldIndividual => "押している間だけ 個別",
            ScopeOp::Global => "全体にする",
            ScopeOp::Individual => "個別にする",
        }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SelectOp {
    /// この機材だけを選ぶ
    Only,
    /// 選択に追加/解除(複数選べる)
    Toggle,
}

impl SelectOp {
    pub fn label(self) -> &'static str {
        match self {
            SelectOp::Only => "この機材だけを選ぶ",
            SelectOp::Toggle => "選択に追加/解除",
        }
    }
}

/// 色の操作を、空いているコントローラーのボタンにまとめて割り当てる(APC mini mk2 の配置)。
/// 下段のパッド8個=色 / シーンボタン8=全体⇄個別 / トラックボタン1〜8=機材の選択。
/// フェーダー8=色相(H)。すでに使われているボタンは飛ばす。戻り値=(追加した数, 飛ばした数)
pub fn add_template(project: &mut crate::project::Project) -> (usize, usize) {
    use crate::apcmini::Ctl;
    use crate::project::*;
    let mut added = 0;
    let mut skipped = 0;
    let mut put = |project: &mut Project, ctl: Ctl, name: String, action: Action| {
        if project.bindings.iter().any(|b| b.trigger.is_some_and(|t| ctl.matches(&t))) {
            skipped += 1;
            return;
        }
        let (kind, number) = ctl.trigger();
        project.bindings.push(Binding { id: 0, name, trigger: Some(MidiTrigger { kind, channel: None, number }), action, page: None, osc: None });
        added += 1;
    };
    let palette = [NamedColor::Red, NamedColor::Orange, NamedColor::Yellow, NamedColor::Green, NamedColor::Cyan, NamedColor::Blue, NamedColor::Violet, NamedColor::White];
    for (i, c) in palette.iter().enumerate() {
        put(project, Ctl::Pad(i as u8), format!("色:{}", c.label()), Action::Color { color: Some(*c), target: ColorTarget::Scope });
    }
    put(project, Ctl::Scene(7), "色:全体/個別".into(), Action::ColorScope { op: ScopeOp::Toggle });
    let fixtures: Vec<(u32, String)> = project.fixtures.iter().take(8).map(|f| (f.id, f.name.clone())).collect();
    for (i, (id, name)) in fixtures.into_iter().enumerate() {
        put(project, Ctl::Track(i as u8), format!("選択:{name}"), Action::Select { fixture: id, op: SelectOp::Only });
    }
    // フェーダー8を色相(HSVのH)にする
    put(project, Ctl::Fader(7), "色相(H)".into(), Action::Color { color: None, target: ColorTarget::Scope });
    (added, skipped)
}

/// メイン/サブの色の操作を、空いているところにまとめて割り当てる。
/// 下から2段目のパッド=メイン色 / 3段目=サブ色 / 入れ替え・自動入れ替え=シーンボタン7・6(使用中ならトラックボタン8・7)
/// フェーダー6=メインの色相 / フェーダー7=サブの色相。戻り値=(追加した数, 飛ばした数)
pub fn add_palette_template(project: &mut crate::project::Project) -> (usize, usize) {
    use crate::apcmini::Ctl;
    use crate::project::*;
    let used = |p: &Project, ctl: Ctl| p.bindings.iter().any(|b| b.trigger.is_some_and(|t| ctl.matches(&t)));
    let (mut added, mut skipped) = (0, 0);
    let mut put = |project: &mut Project, ctls: &[Ctl], name: String, action: Action| {
        let Some(ctl) = ctls.iter().copied().find(|c| !used(project, *c)) else {
            skipped += 1;
            return;
        };
        let (kind, number) = ctl.trigger();
        project.bindings.push(Binding { id: 0, name, trigger: Some(MidiTrigger { kind, channel: None, number }), action, page: None, osc: None });
        added += 1;
    };
    let colors = [NamedColor::Red, NamedColor::Orange, NamedColor::Yellow, NamedColor::Green, NamedColor::Cyan, NamedColor::Blue, NamedColor::Violet, NamedColor::White];
    for (row, target) in [(1u8, ColorTarget::Main), (2u8, ColorTarget::Sub)] {
        let who = if target == ColorTarget::Main { "メイン" } else { "サブ" };
        for (i, c) in colors.iter().enumerate() {
            put(project, &[Ctl::Pad(row * 8 + i as u8)], format!("{who}:{}", c.label()), Action::Color { color: Some(*c), target });
        }
    }
    put(project, &[Ctl::Scene(6), Ctl::Track(7)], "メイン⇄サブ".into(), Action::Palette { op: PaletteOp::Swap });
    put(project, &[Ctl::Scene(5), Ctl::Track(6)], "自動入れ替え".into(), Action::Palette { op: PaletteOp::Auto });
    put(project, &[Ctl::Fader(5)], "メインの色相".into(), Action::Color { color: None, target: ColorTarget::Main });
    put(project, &[Ctl::Fader(6)], "サブの色相".into(), Action::Color { color: None, target: ColorTarget::Sub });
    (added, skipped)
}
