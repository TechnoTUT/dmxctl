//! ピクセルエフェクト: LEDバー(RGBが並んだ機材)のピクセルに、流れる色やレインボーを出す。
//! 速さはBPMに同期(1周期の拍数)。色はメイン色/サブ色(入れ替えにも追従)か、指定の色。

use crate::color::{self, ColorRole};
use crate::engine::Core;
use crate::project::Selector;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum PixelKind {
    /// 色相がバーに沿って流れる
    Rainbow,
    /// 色Aの点(幅=ピクセル数)が、色Bの上を流れる
    Chase,
    /// 色Aで端から埋めていき、次に色Bで埋め戻す
    Wipe,
    /// 色Aと色Bを交互に並べ、拍で入れ替える
    Alternate,
    /// 色Bの上に、色Aのピクセルがランダムに光る
    Sparkle,
    /// 色A→色Bのグラデーションが流れる
    Gradient,
    /// 色Aと色Bの縞が、1周で縞2本分(A+B)流れる
    Stripes,
    /// 色Aと色Bが、なめらかな波になって流れる
    Wave,
    /// 隣り合う組が、色A⇄色Bを逆向きにゆっくり行き来する(交互にふわっと入れ替わる)
    AltFade,
}

impl PixelKind {
    pub const ALL: [PixelKind; 9] = [
        PixelKind::Stripes,
        PixelKind::Wave,
        PixelKind::AltFade,
        PixelKind::Alternate,
        PixelKind::Chase,
        PixelKind::Gradient,
        PixelKind::Wipe,
        PixelKind::Sparkle,
        PixelKind::Rainbow,
    ];
    pub fn label(self) -> &'static str {
        match self {
            PixelKind::Rainbow => "レインボー",
            PixelKind::Chase => "流れる点",
            PixelKind::Wipe => "ワイプ",
            PixelKind::Alternate => "交互(入れ替え)",
            PixelKind::Sparkle => "きらめき",
            PixelKind::Gradient => "グラデーション",
            PixelKind::Stripes => "流れる縞",
            PixelKind::Wave => "ウェーブ",
            PixelKind::AltFade => "交互ウェーブ",
        }
    }
    /// 「幅」の意味(画面の説明用)
    pub fn width_label(self) -> Option<&'static str> {
        match self {
            PixelKind::Chase => Some("点の幅"),
            PixelKind::Alternate | PixelKind::AltFade => Some("1色の幅"),
            PixelKind::Stripes => Some("縞の幅"),
            PixelKind::Sparkle => Some("光る数"),
            PixelKind::Rainbow | PixelKind::Gradient | PixelKind::Wave => Some("繰り返し"),
            PixelKind::Wipe => None,
        }
    }
    pub fn uses_colors(self) -> bool {
        self != PixelKind::Rainbow
    }
}

/// 複数のバーを平行に並べた「面」で、模様をどちら向きに流すか
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum PixelAxis {
    /// バーの長さ方向(今までの動き)
    #[default]
    Along,
    /// バーをまたぐ方向(パッチの順に1本目→2本目…。1本のバーは全体で同じ色)
    Across,
}

impl PixelAxis {
    pub const ALL: [PixelAxis; 2] = [PixelAxis::Along, PixelAxis::Across];
    pub fn label(self) -> &'static str {
        match self {
            PixelAxis::Along => "バーに沿って",
            PixelAxis::Across => "バーをまたいで",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum PixelColor {
    Main,
    Sub,
    Off,
    White,
    Custom([f32; 3]),
}

impl PixelColor {
    pub const CHOICES: [PixelColor; 5] = [PixelColor::Main, PixelColor::Sub, PixelColor::Off, PixelColor::White, PixelColor::Custom([1.0, 0.0, 0.0])];
    pub fn label(self) -> &'static str {
        match self {
            PixelColor::Main => "メイン色",
            PixelColor::Sub => "サブ色",
            PixelColor::Off => "消灯",
            PixelColor::White => "白",
            PixelColor::Custom(_) => "指定の色",
        }
    }
    pub fn same_kind(self, o: PixelColor) -> bool {
        std::mem::discriminant(&self) == std::mem::discriminant(&o)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PixelFx {
    pub name: String,
    pub enabled: bool,
    pub sel: Selector,
    pub kind: PixelKind,
    pub a: PixelColor,
    pub b: PixelColor,
    /// 1周期の拍数(BPM同期)。0なら止まったまま
    pub beats: f32,
    /// 点の幅・交互の幅・光る数・繰り返し回数(種類による)
    pub width: f32,
    /// 流れる向きを逆にする
    pub reverse: bool,
    /// 複数台を1本のバーとしてつなげる(OFFなら各台で同じ動き)
    pub span: bool,
    /// 中央から左右対称(模様が中央から外へ広がる)
    pub mirror: bool,
    /// 流す向き(バーに沿って / バーをまたいで)
    pub axis: PixelAxis,
    /// もう一方の向きに付ける周期のずれ(端から端までで何周ずらすか)。0以外で斜めに流れる
    pub skew: f32,
}

impl Default for PixelFx {
    fn default() -> Self {
        PixelFx {
            name: "ピクセル".into(),
            enabled: false,
            sel: Selector::All,
            kind: PixelKind::Chase,
            a: PixelColor::Main,
            b: PixelColor::Sub,
            beats: 4.0,
            width: 2.0,
            reverse: false,
            span: true,
            mirror: false,
            axis: PixelAxis::Along,
            skew: 0.0,
        }
    }
}

/// メイン色とサブ色を混ぜる模様のプリセット(名前, 種類, 1周の拍数, 幅, 中央から左右対称)。
/// 先頭の8つは、マップの「LEDバーの模様を配置」でパッドの最上段に並ぶ
pub const PRESETS: [(&str, PixelKind, f32, f32, bool); 11] = [
    ("縞々", PixelKind::Alternate, 0.0, 2.0, false),
    ("縞々スクロール", PixelKind::Stripes, 4.0, 2.0, false),
    ("太縞スクロール", PixelKind::Stripes, 8.0, 4.0, false),
    ("交互チェンジ", PixelKind::Alternate, 1.0, 1.0, false),
    ("ウェーブ", PixelKind::Wave, 4.0, 1.0, false),
    ("交互ウェーブ", PixelKind::AltFade, 2.0, 1.0, false),
    ("センターウェーブ", PixelKind::Wave, 4.0, 1.0, true),
    ("センター縞", PixelKind::Stripes, 4.0, 2.0, true),
    ("細かいウェーブ", PixelKind::Wave, 2.0, 2.0, false),
    ("グラデーション", PixelKind::Gradient, 8.0, 1.0, false),
    ("メイン点が走る", PixelKind::Chase, 4.0, 2.0, false),
];

/// プリセットからピクセルエフェクトを作る(色A=メイン色、色B=サブ色。OFFの状態で)
pub fn from_preset(i: usize) -> Option<PixelFx> {
    let (name, kind, beats, width, mirror) = *PRESETS.get(i)?;
    Some(PixelFx { name: name.into(), kind, beats, width, mirror, a: PixelColor::Main, b: PixelColor::Sub, ..PixelFx::default() })
}

fn hash(a: u64, b: u64) -> f32 {
    let mut x = a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 31;
    x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 29;
    (x & 0xFFFF) as f32 / 65535.0
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// `i`番目(0始まり)/全`n`ピクセルの色。`phase`=経過した周期(拍/1周期の拍数)
pub fn pixel_color(kind: PixelKind, i: usize, n: usize, phase: f64, width: f32, a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    let n = n.max(1);
    let nf = n as f64;
    let pos = i as f64 / nf;
    let frac = phase.rem_euclid(1.0);
    let w = width.max(0.0) as f64;
    match kind {
        PixelKind::Rainbow => color::hsv((((pos * w.max(1.0)) + frac).rem_euclid(1.0) * 360.0) as f32, 1.0, 1.0),
        PixelKind::Chase => {
            // 1ピクセルずつ進む(点の先頭をピクセルの位置にそろえる)
            let head = (frac * nf).floor();
            let d = (i as f64 - head).rem_euclid(nf);
            if d < w.max(1.0) { a } else { b }
        }
        PixelKind::Wipe => {
            // 前半: Aで埋める / 後半: Bで埋め戻す
            let t = frac * 2.0;
            let edge = (t.fract() * nf).floor();
            let first = t < 1.0;
            let filled = (i as f64) < edge;
            match (first, filled) {
                (true, true) | (false, false) => a,
                _ => b,
            }
        }
        PixelKind::Alternate => {
            let group = (i as f64 / w.max(1.0)).floor() as i64;
            if (group + phase.floor() as i64).rem_euclid(2) == 0 { a } else { b }
        }
        PixelKind::Sparkle => {
            // 1周期に8回、光るピクセルを選び直す
            let step = (phase * 8.0).floor() as i64 as u64;
            let p = (w / nf).clamp(0.0, 1.0) as f32;
            if hash(step, i as u64) < p { a } else { b }
        }
        PixelKind::Gradient => {
            let t = ((pos * w.max(1.0)) + frac).rem_euclid(1.0);
            // 行って戻る(端でつながるように)
            let tri = if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 };
            mix(a, b, tri as f32)
        }
        PixelKind::Stripes => {
            // 1周で縞2本分(A+B)進むので、つなぎ目なく流れ続ける。1ピクセルずつ進む
            let sw = w.max(1.0);
            let offset = (frac * sw * 2.0).floor();
            if ((i as f64 - offset) / sw).floor().rem_euclid(2.0) < 1.0 { a } else { b }
        }
        PixelKind::Wave => {
            let t = 0.5 - 0.5 * (std::f64::consts::TAU * (pos * w.max(1.0) - frac)).cos();
            mix(a, b, t as f32)
        }
        PixelKind::AltFade => {
            let t = (0.5 - 0.5 * (std::f64::consts::TAU * frac).cos()) as f32;
            let even = (i as f64 / w.max(1.0)).floor().rem_euclid(2.0) < 1.0;
            if even { mix(a, b, t) } else { mix(b, a, t) }
        }
    }
}

/// プリセット(先頭から`count`個)を、まだ無ければプロジェクトに足す。戻り値=追加した数
pub fn add_presets(p: &mut crate::project::Project, count: usize) -> usize {
    let mut added = 0;
    for i in 0..count.min(PRESETS.len()) {
        let Some(fx) = from_preset(i) else { continue };
        if !p.pixel_fx.iter().any(|x| x.name == fx.name) {
            p.pixel_fx.push(fx);
            added += 1;
        }
    }
    added
}

/// LEDバーの模様(プリセットの先頭8つ)を、パッドに「トグル」で並べる。最上段から順に、使用中のパッドは飛ばして
/// 次の空き(2段目まで)に置く。すでにパッドに置いてある模様は置かない。戻り値=(配置した数, 空きが無くて置けなかった数)
pub fn add_pad_template(p: &mut crate::project::Project, page: Option<u8>) -> (usize, usize) {
    use crate::apcmini::{pad_note, Ctl};
    use crate::project::*;
    add_presets(p, 8);
    let (mut added, mut skipped) = (0, 0);
    let visible = |b: &Binding| page.is_none_or(|pg| b.on_page(pg));
    let candidates: Vec<Ctl> = (0..2u8).flat_map(|row| (0..8u8).map(move |col| Ctl::Pad(pad_note(row, col)))).collect();
    for (name, ..) in PRESETS.iter().take(8) {
        let placed = p.bindings.iter().any(|b| visible(b) && b.trigger.is_some() && matches!(&b.action, Action::Effect { name: n, .. } if n == name));
        if placed {
            continue;
        }
        // 共通で置くなら、どのページでも空いているパッドだけ。ページ専用なら、そのページで空いていればよい
        let free = candidates.iter().copied().find(|ctl| !p.bindings.iter().any(|b| b.trigger.is_some_and(|t| ctl.matches(&t)) && visible(b)));
        let Some(ctl) = free else {
            skipped += 1;
            continue;
        };
        let (kind, number) = ctl.trigger();
        p.bindings.push(Binding {
            name: format!("模様:{name}"),
            trigger: Some(MidiTrigger { kind, channel: None, number }),
            action: Action::Effect { name: (*name).into(), mode: PadMode::Toggle },
            page,
            osc: None,
        });
        added += 1;
    }
    (added, skipped)
}

impl Core {
    /// ピクセルエフェクトをON/OFFする。ONにしたときは、同じLEDバーに掛かっている他のピクセルエフェクトをOFFにする
    /// (模様は後のものが上書きするので、1本のバーには1つずつ)
    pub fn set_pixel_fx(&mut self, name: &str, on: bool) {
        let mut mine: Vec<usize> = Vec::new();
        for fx in self.project.pixel_fx.iter().filter(|f| f.name == name) {
            mine.extend(self.resolve(&fx.sel));
        }
        let others: Vec<usize> = (0..self.project.pixel_fx.len())
            .filter(|&k| {
                let fx = &self.project.pixel_fx[k];
                on && fx.enabled && fx.name != name && self.resolve(&fx.sel).iter().any(|i| mine.contains(i))
            })
            .collect();
        for fx in self.project.pixel_fx.iter_mut().filter(|f| f.name == name) {
            fx.enabled = on;
        }
        for k in others {
            self.project.pixel_fx[k].enabled = false;
        }
        self.dirty = true;
    }

    fn pixel_rgb_of(&self, c: PixelColor) -> [f32; 3] {
        match c {
            PixelColor::Main => self.role_rgb(ColorRole::Main).unwrap_or([1.0; 3]),
            PixelColor::Sub => self.role_rgb(ColorRole::Sub).unwrap_or([0.0; 3]),
            PixelColor::Off => [0.0; 3],
            PixelColor::White => [1.0; 3],
            PixelColor::Custom(c) => c,
        }
    }

    /// 動作中のピクセルエフェクトを、出力値(`vals`: 機材ごとの値)に書き込む
    pub fn render_pixel_fx(&self, vals: &mut [Vec<u8>]) {
        for fx in self.project.pixel_fx.iter().filter(|f| f.enabled) {
            let a = self.pixel_rgb_of(fx.a);
            let b = self.pixel_rgb_of(fx.b);
            let phase = if fx.beats > 0.0 { self.rt.beat / fx.beats as f64 } else { 0.0 };
            // 対象のうち、ピクセルを持つ機材(パッチの順)
            let targets: Vec<(usize, &crate::fixture::Mode, std::sync::Arc<crate::fixture::ModeInfo>)> = self
                .resolve(&fx.sel)
                .into_iter()
                .filter_map(|i| {
                    let f = &self.project.fixtures[i];
                    let m = self.lib.mode(&f.profile, f.mode)?;
                    let info = self.lib.info(&f.profile, f.mode)?;
                    (!info.pixels.is_empty() && vals[i].len() == m.channels.len()).then_some((i, m, info))
                })
                .collect();
            let total: usize = targets.iter().map(|t| t.2.pixels.len()).sum();
            let bars = targets.len();
            let mut offset = 0;
            for (bar, (i, _, info)) in targets.iter().enumerate() {
                let px = &info.pixels;
                for (k, p) in px.iter().enumerate() {
                    // g=模様の中での位置 / n=模様の長さ / across=もう一方の向きでの位置(0〜1、斜めのずれ用)
                    let (g, n, across) = match fx.axis {
                        PixelAxis::Along if fx.span => (offset + k, total, bar as f64 / bars as f64),
                        PixelAxis::Along => (k, px.len(), bar as f64 / bars as f64),
                        PixelAxis::Across => (bar, bars, k as f64 / px.len() as f64),
                    };
                    let phase = phase + fx.skew as f64 * across;
                    let g = if fx.reverse { n - 1 - g } else { g };
                    // 中央から左右対称: 中央=0、端=半分の長さ-1 として、半分の長さで模様を作る
                    let (g, n2) = if fx.mirror {
                        let half = n.div_ceil(2);
                        (half - 1 - g.min(n - 1 - g), half)
                    } else {
                        (g, n)
                    };
                    let rgb = pixel_color(fx.kind, g, n2, phase, fx.width, a, b);
                    info.set_pixel(&mut vals[*i], p, rgb);
                }
                offset += px.len();
            }
        }
    }
}
