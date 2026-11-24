//! 機材プロファイル(チャンネル定義)。アプリ内のエディタで作る/編集する。実体はJSONファイル。

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// チャンネルの分類。コントロール画面のグループ分けや、シーンの部分保存に使う。自由に追加もできる
pub const CATEGORIES: [&str; 6] = ["動き", "色", "光量", "形", "制御", "その他"];
pub const OTHER: &str = "その他";

/// 16 ビット DMX を粗値と細値で表したときの上限
pub const DMX_FINE_MAX: f32 = 255.0 + 255.0 / 256.0;

fn set_dmx_value(values: &mut [u8], coarse: usize, fine: Option<usize>, value: f32) {
    let max = if fine.is_some() { DMX_FINE_MAX } else { 255.0 };
    let value = value.clamp(0.0, max);
    let whole = value.floor();
    if let Some(slot) = values.get_mut(coarse) {
        *slot = whole as u8;
    }
    if let Some(slot) = fine.and_then(|index| values.get_mut(index)) {
        *slot = ((value - whole) * 256.0).floor() as u8;
    }
}

/// カテゴリの表示順: 標準のものを先に、それ以外は名前順
pub fn category_order(name: &str) -> (usize, String) {
    (CATEGORIES.iter().position(|c| *c == name).unwrap_or(CATEGORIES.len()), name.to_string())
}

#[derive(schemars::JsonSchema, Debug, Clone, Deserialize, Serialize)]
pub struct Range {
    pub from: u8,
    pub to: u8,
    pub label: String,
    /// 範囲内の値が速度などの連続パラメータになっている
    #[serde(default)]
    pub sweep: bool,
    /// 誤操作すると危険/面倒な項目(リセットなど)
    #[serde(default)]
    pub warn: bool,
    /// この範囲が出す色(RGB)。色の全体制御・色相フェーダー・パッドのLEDが使う。
    /// Noneなら、表示名の色の名前(赤・青など)から判定する
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
}

impl Range {
    pub fn mid(&self) -> u8 {
        ((self.from as u16 + self.to as u16) / 2) as u8
    }
    pub fn contains(&self, v: u8) -> bool {
        (self.from..=self.to).contains(&v)
    }
}

/// レンジ一覧の問題点(空なら問題なし)。0〜255の抜け・重複・逆転を調べる
pub fn range_issues(ranges: &[Range]) -> Vec<String> {
    let mut issues = Vec::new();
    // レンジが無いチャンネルは0〜255のスライダーになるので、問題なし
    if ranges.is_empty() {
        return issues;
    }
    for r in ranges {
        if r.from > r.to {
            issues.push(format!("「{}」の開始が終了より大きい", r.label));
        }
    }
    let mut count = [0u8; 256];
    for r in ranges.iter().filter(|r| r.from <= r.to) {
        for v in r.from..=r.to {
            count[v as usize] = count[v as usize].saturating_add(1);
        }
    }
    let spans = |pred: &dyn Fn(u8) -> bool| -> Vec<String> {
        let mut out = Vec::new();
        let mut v = 0usize;
        while v < 256 {
            if pred(count[v]) {
                let s = v;
                while v < 256 && pred(count[v]) {
                    v += 1;
                }
                out.push(if s == v - 1 { format!("{s}") } else { format!("{s}〜{}", v - 1) });
            } else {
                v += 1;
            }
        }
        out
    };
    let gaps = spans(&|c| c == 0);
    if !gaps.is_empty() {
        issues.push(format!("未定義の値: {}", gaps.iter().take(4).cloned().collect::<Vec<_>>().join(", ")));
    }
    let overlaps = spans(&|c| c > 1);
    if !overlaps.is_empty() {
        issues.push(format!("重複している値: {}", overlaps.iter().take(4).cloned().collect::<Vec<_>>().join(", ")));
    }
    issues
}

#[derive(schemars::JsonSchema, Debug, Clone, Deserialize, Serialize)]
pub struct ChannelDef {
    pub name: String,
    /// 機能名(pan / tilt / dimmer / color ...)。MIDI割り当てやエフェクトはこの名前で対象を指定する
    pub attr: String,
    /// 「動き」「色」など。空なら「その他」
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub default: u8,
    /// このチャンネルが「Nチャンネル(1始まり)」の下位バイトであることを示す
    #[serde(default)]
    pub fine_of: Option<usize>,
    /// ブラックアウト/ARM解除時に強制出力する値
    #[serde(default)]
    pub safe: Option<u8>,
    #[serde(default)]
    pub ranges: Vec<Range>,
    /// 絶対に出力しない値の範囲([from, to])。レーザーのオートショーなど。
    /// 出力の直前に、安全値(それも禁止なら一番近い使える値)へ置き換える
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub forbid: Vec<[u8; 2]>,
}

impl ChannelDef {
    pub fn is_forbidden(&self, v: u8) -> bool {
        self.forbid.iter().any(|&[a, b]| (a.min(b)..=a.max(b)).contains(&v))
    }
    /// 範囲の値がすべて禁止か(選択肢に出さない)
    pub fn forbids_range(&self, r: &Range) -> bool {
        (r.from..=r.to).all(|v| self.is_forbidden(v))
    }
    /// 出してよい値にする: 禁止の値なら安全値へ、安全値も禁止なら一番近い使える値へ
    pub fn allowed(&self, v: u8) -> u8 {
        if !self.is_forbidden(v) {
            return v;
        }
        if let Some(s) = self.safe.filter(|&s| !self.is_forbidden(s)) {
            return s;
        }
        (1..=255u8)
            .flat_map(|d| [v.checked_sub(d), v.checked_add(d)])
            .flatten()
            .find(|&c| !self.is_forbidden(c))
            .unwrap_or(v)
    }
    pub fn range_of(&self, v: u8) -> Option<&Range> {
        self.ranges.iter().find(|r| r.contains(v))
    }
    pub fn category_name(&self) -> &str {
        if self.category.trim().is_empty() { OTHER } else { self.category.trim() }
    }
    pub fn blank(name: &str, attr: &str, category: &str) -> ChannelDef {
        ChannelDef { name: name.into(), attr: attr.into(), category: category.into(), default: 0, fine_of: None, safe: None, ranges: Vec::new(), forbid: Vec::new() }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Deserialize, Serialize)]
pub struct Mode {
    pub name: String,
    pub channels: Vec<ChannelDef>,
}

/// LEDバーなどのピクセルの色チャンネルは、機能名を「red.3」のように「色.番号(1始まり)」にする。
/// 「red.3」→ ("red", Some(3)) / 「dimmer」→ ("dimmer", None)
pub fn split_pixel(attr: &str) -> (&str, Option<u16>) {
    match attr.rsplit_once('.') {
        Some((base, n)) => match n.parse::<u16>() {
            Ok(n) if n > 0 && !base.is_empty() => (base, Some(n)),
            _ => (attr, None),
        },
        None => (attr, None),
    }
}

/// 1ピクセル分の色チャンネル(モード内のインデックス)
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Pixel {
    pub red: Option<usize>,
    pub green: Option<usize>,
    pub blue: Option<usize>,
    pub white: Option<usize>,
}

impl Mode {
    /// 機能名のチャンネル。ピクセルのある機材で「red」のように番号なしで探すと、1番目のピクセルのものを返す
    pub fn find_attr(&self, attr: &str) -> Option<usize> {
        self.channels
            .iter()
            .position(|c| c.attr == attr)
            .or_else(|| self.channels.iter().position(|c| c.fine_of.is_none() && matches!(split_pixel(&c.attr), (b, Some(_)) if b == attr)))
    }

    /// 値を書き込むチャンネル。ピクセルのある機材で「red」を指定すると、全ピクセルの赤
    pub fn attr_channels(&self, attr: &str) -> Vec<usize> {
        if let Some(i) = self.channels.iter().position(|c| c.attr == attr) {
            return vec![i];
        }
        self.channels.iter().enumerate().filter(|(_, c)| c.fine_of.is_none() && matches!(split_pixel(&c.attr), (b, Some(_)) if b == attr)).map(|(i, _)| i).collect()
    }

    /// ピクセル(番号順)。LEDバーでなければ空
    pub fn pixels(&self) -> Vec<Pixel> {
        let mut map: std::collections::BTreeMap<u16, Pixel> = std::collections::BTreeMap::new();
        for (i, c) in self.channels.iter().enumerate() {
            if c.fine_of.is_some() {
                continue;
            }
            if let (base, Some(n)) = split_pixel(&c.attr) {
                let p = map.entry(n).or_default();
                match base {
                    "red" => p.red = Some(i),
                    "green" => p.green = Some(i),
                    "blue" => p.blue = Some(i),
                    "white" => p.white = Some(i),
                    _ => {}
                }
            }
        }
        map.into_values().filter(|p| p.red.is_some() || p.green.is_some() || p.blue.is_some() || p.white.is_some()).collect()
    }

    /// ピクセルに色(0.0〜1.0のRGB)を書き込む。白のチャンネルがあれば、共通成分を白に回す
    pub fn set_pixel(&self, values: &mut [u8], p: &Pixel, rgb: [f32; 3]) {
        let mut v = [rgb[0], rgb[1], rgb[2]].map(|x| x.clamp(0.0, 1.0) * 255.0);
        if let Some(w) = p.white {
            let m = v[0].min(v[1]).min(v[2]);
            v = [v[0] - m, v[1] - m, v[2] - m];
            self.set_f(values, w, m);
        }
        for (ch, x) in [(p.red, v[0]), (p.green, v[1]), (p.blue, v[2])] {
            if let Some(ch) = ch {
                self.set_f(values, ch, x);
            }
        }
    }

    /// ピクセルの今の色(0.0〜1.0のRGB。白は各色に足す)
    pub fn pixel_rgb(&self, values: &[u8], p: &Pixel) -> [f32; 3] {
        let g = |ch: Option<usize>| ch.map(|c| values.get(c).copied().unwrap_or(0) as f32 / 255.0).unwrap_or(0.0);
        let w = g(p.white);
        [(g(p.red) + w).min(1.0), (g(p.green) + w).min(1.0), (g(p.blue) + w).min(1.0)]
    }
    /// 上位バイト`coarse`(0始まり)に対応する下位バイトのインデックス
    pub fn fine_of(&self, coarse: usize) -> Option<usize> {
        self.channels.iter().position(|c| c.fine_of == Some(coarse + 1))
    }
    pub fn is_fine(&self, ch: usize) -> bool {
        self.channels[ch].fine_of.is_some()
    }
    pub fn default_values(&self) -> Vec<u8> {
        self.channels.iter().map(|c| c.default).collect()
    }
    /// このモードに含まれるカテゴリ(表示順)
    pub fn categories(&self) -> Vec<String> {
        let mut v: Vec<String> = self.channels.iter().filter(|c| c.fine_of.is_none()).map(|c| c.category_name().to_string()).collect();
        v.sort_by_key(|c| category_order(c));
        v.dedup();
        v
    }
    /// 上位/下位をまとめた値(0..256の小数)
    pub fn get_f(&self, values: &[u8], ch: usize) -> f32 {
        let c = values.get(ch).copied().unwrap_or(0) as f32;
        match self.fine_of(ch) {
            Some(f) => c + values.get(f).copied().unwrap_or(0) as f32 / 256.0,
            None => c,
        }
    }
    pub fn set_f(&self, values: &mut [u8], ch: usize, v: f32) {
        set_dmx_value(values, ch, self.fine_of(ch), v);
    }

    /// 値を反転する(左右反転など)。レンジのないチャンネルは 0〜最大 の中で反転。
    /// レンジがあるチャンネルは、値が入っている「連続」レンジの中でだけ反転し(例: レーザーのパン位置 0〜127)、
    /// 選択式のレンジ(マクロなど)の値は変えない
    pub fn invert_f(&self, ch: usize, v: f32) -> f32 {
        let c = &self.channels[ch];
        if c.ranges.is_empty() {
            let max = if self.fine_of(ch).is_some() { 255.0 + 255.0 / 256.0 } else { 255.0 };
            return (max - v).clamp(0.0, max);
        }
        match c.range_of(v.floor().clamp(0.0, 255.0) as u8) {
            Some(r) if r.sweep => ((r.from as f32 + r.to as f32) - v).clamp(r.from as f32, r.to as f32),
            _ => v,
        }
    }

    // ---- エディタ用: チャンネルの追加・削除・並べ替え(16bitの参照番号も追従させる) ----

    /// すべての`fine_of`(1始まり)を、旧インデックス→新インデックスの対応で書き換える。対応がなければ参照を外す
    fn remap_fine(&mut self, f: impl Fn(usize) -> Option<usize>) {
        for c in &mut self.channels {
            if let Some(n) = c.fine_of {
                c.fine_of = f(n - 1).map(|x| x + 1);
            }
        }
    }

    pub fn insert_channel(&mut self, at: usize, ch: ChannelDef) {
        let at = at.min(self.channels.len());
        self.remap_fine(|j| Some(if j >= at { j + 1 } else { j }));
        self.channels.insert(at, ch);
    }

    pub fn remove_channel(&mut self, i: usize) {
        if i >= self.channels.len() {
            return;
        }
        self.channels.remove(i);
        self.remap_fine(|j| match j.cmp(&i) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(j - 1),
            std::cmp::Ordering::Less => Some(j),
        });
    }

    /// `i`番目のチャンネルを`delta`(+1=下へ / -1=上へ)だけ動かす
    pub fn move_channel(&mut self, i: usize, delta: isize) {
        let j = i as isize + delta;
        if j < 0 || j as usize >= self.channels.len() || i >= self.channels.len() {
            return;
        }
        let j = j as usize;
        self.channels.swap(i, j);
        self.remap_fine(|k| Some(if k == i { j } else if k == j { i } else { k }));
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Deserialize, Serialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub manufacturer: String,
    /// trueの機材(レーザー等)は、ARMしない限り`safe`値を出力し続ける
    #[serde(default)]
    pub needs_arm: bool,
    pub modes: Vec<Mode>,
}

#[derive(Default)]
pub struct Report {
    /// 保存できない問題(画面に出す日本語)
    pub errors: Vec<String>,
    /// `errors` と同じ問題の英語の文(CLI・MCP の JSON に使う)
    pub errors_en: Vec<String>,
    /// 保存はできるが、確認したほうがよい点
    pub warnings: Vec<String>,
}

impl Profile {
    pub fn validate(&self) -> Report {
        let mut r = Report::default();
        fn error(r: &mut Report, ja: String, en: String) {
            r.errors.push(ja);
            r.errors_en.push(en);
        }
        if self.name.trim().is_empty() {
            error(&mut r, "機材名が空です".into(), "profile name is empty".into());
        }
        if self.modes.is_empty() {
            error(&mut r, "モードが1つもありません".into(), "profile has no modes".into());
        }
        let mut mode_names = HashSet::new();
        for m in &self.modes {
            let mn = format!("モード「{}」", m.name);
            let me = format!("mode \"{}\"", m.name);
            if m.name.trim().is_empty() {
                error(&mut r, "モード名が空です".into(), "mode name is empty".into());
            } else if !mode_names.insert(m.name.trim().to_string()) {
                error(&mut r, format!("{mn}の名前が重複しています"), format!("{me}: duplicate mode name"));
            }
            if m.channels.is_empty() {
                error(&mut r, format!("{mn}にチャンネルがありません"), format!("{me}: no channels"));
            }
            if m.channels.len() > 512 {
                error(&mut r, format!("{mn}のチャンネル数が512を超えています"), format!("{me}: more than 512 channels"));
            }
            let mut attrs = HashSet::new();
            let mut fine_targets = HashSet::new();
            for (i, c) in m.channels.iter().enumerate() {
                let cn = format!("{mn} ch{} 「{}」", i + 1, c.name);
                let ce = format!("{me} ch{} \"{}\"", i + 1, c.name);
                if c.attr.trim().is_empty() {
                    error(&mut r, format!("{cn}: 機能名が空です"), format!("{ce}: attribute is empty"));
                } else if !attrs.insert(c.attr.trim().to_string()) {
                    error(&mut r, format!("{cn}: 機能名「{}」がモード内で重複しています", c.attr), format!("{ce}: duplicate attribute {} in the mode", c.attr));
                }
                if let Some(n) = c.fine_of {
                    if n == 0 || n > m.channels.len() || n - 1 == i {
                        error(&mut r, format!("{cn}: 16bitの上位チャンネルの指定が不正です"), format!("{ce}: invalid fine_of"));
                    } else if m.channels[n - 1].fine_of.is_some() {
                        error(&mut r, format!("{cn}: 下位バイトのチャンネルを上位に指定できません"), format!("{ce}: fine_of points to another fine channel"));
                    } else if !fine_targets.insert(n) {
                        error(&mut r, format!("{cn}: ch{n}の下位バイトが複数指定されています"), format!("{ce}: ch{n} has more than one fine channel"));
                    }
                }
                // レンジのないチャンネルは0〜255のスライダーになるので、レンジがあるものだけ調べる
                if c.fine_of.is_none() && !c.ranges.is_empty() {
                    for issue in range_issues(&c.ranges) {
                        let msg = format!("{cn}: {issue}");
                        if issue.contains("大きい") { error(&mut r, msg, format!("{ce}: a range starts after it ends")) } else { r.warnings.push(msg) }
                    }
                }
            }
        }
        r
    }
}

/// 出力の合成で毎フレーム使う、モードごとの下調べ(機能名の解析を毎回しないため)
pub struct ModeInfo {
    pub pixels: Vec<Pixel>,
    /// 全体光量・ブラックアウトがかかるチャンネル: ディマーがあればそれだけ。無ければ色の強さ(ピクセル含む)
    pub master: Vec<usize>,
    by_attr: HashMap<String, Vec<usize>>,
    /// 上位バイト → 下位バイトのチャンネル(16bit)
    fine: Vec<Option<usize>>,
}

impl ModeInfo {
    pub fn new(m: &Mode) -> ModeInfo {
        let mut by_attr: HashMap<String, Vec<usize>> = HashMap::new();
        for c in m.channels.iter().filter(|c| c.fine_of.is_none()) {
            by_attr.entry(c.attr.clone()).or_insert_with(|| m.attr_channels(&c.attr));
            let base = split_pixel(&c.attr).0;
            by_attr.entry(base.to_string()).or_insert_with(|| m.attr_channels(base));
        }
        let pick = |names: &[&str]| -> Vec<usize> { m.channels.iter().enumerate().filter(|(_, c)| c.fine_of.is_none() && names.contains(&split_pixel(&c.attr).0)).map(|(i, _)| i).collect() };
        let dim = pick(&["dimmer"]);
        let master = if dim.is_empty() { pick(&["red", "green", "blue", "white", "amber", "uv"]) } else { dim };
        let fine = (0..m.channels.len()).map(|i| m.fine_of(i)).collect();
        ModeInfo { pixels: m.pixels(), master, by_attr, fine }
    }

    /// Mode::get_fと同じ(下位バイトを探し直さない)
    pub fn get_f(&self, values: &[u8], ch: usize) -> f32 {
        let c = values.get(ch).copied().unwrap_or(0) as f32;
        match self.fine.get(ch).copied().flatten() {
            Some(f) => c + values.get(f).copied().unwrap_or(0) as f32 / 256.0,
            None => c,
        }
    }

    /// Mode::set_fと同じ(下位バイトを探し直さない)
    pub fn set_f(&self, values: &mut [u8], ch: usize, v: f32) {
        set_dmx_value(values, ch, self.fine.get(ch).copied().flatten(), v);
    }

    /// Mode::set_pixelと同じ
    pub fn set_pixel(&self, values: &mut [u8], p: &Pixel, rgb: [f32; 3]) {
        let mut v = rgb.map(|x| x.clamp(0.0, 1.0) * 255.0);
        if let Some(w) = p.white {
            let m = v[0].min(v[1]).min(v[2]);
            v = [v[0] - m, v[1] - m, v[2] - m];
            self.set_f(values, w, m);
        }
        for (ch, x) in [(p.red, v[0]), (p.green, v[1]), (p.blue, v[2])] {
            if let Some(ch) = ch {
                self.set_f(values, ch, x);
            }
        }
    }

    /// Mode::attr_channelsと同じ
    pub fn channels(&self, attr: &str) -> &[usize] {
        self.by_attr.get(attr).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

#[derive(Clone)]
pub struct Library {
    pub profiles: Vec<Arc<Profile>>,
    /// プロファイルid → (作ったときのプロファイル, モードごとの下調べ)
    infos: HashMap<String, (Arc<Profile>, Vec<Arc<ModeInfo>>)>,
    pub user_dir: PathBuf,
    /// ユーザーフォルダの中にあるプロファイル(id → ファイル)
    user_files: HashMap<String, PathBuf>,
    builtin_ids: HashSet<String>,
}

impl Default for Library {
    fn default() -> Self {
        Library { profiles: Vec::new(), infos: HashMap::new(), user_dir: PathBuf::new(), user_files: HashMap::new(), builtin_ids: HashSet::new() }
    }
}

const BUILTIN: &[&str] = &[
    include_str!("../../../fixtures/stage-evolution_minispot30.json"),
    include_str!("../../../fixtures/stage-evolution_laserscan300.json"),
    include_str!("../../../fixtures/stage-evolution_twinkle-ball-dmx.json"),
    include_str!("../../../fixtures/glint_glp-185l.json"),
];

/// ピクセルの色の並び(DMXでの順番)
pub const PIXEL_ORDERS: [&str; 6] = ["RGB", "GRB", "BGR", "BRG", "RBG", "RGBW"];

/// `first`番(1始まり)から`count`個のピクセルのチャンネルを作る。`order`は「RGB」「GRB」「RGBW」など
pub fn pixel_channels(first: u16, count: u16, order: &str) -> Vec<ChannelDef> {
    let mut out = Vec::new();
    for n in first..first + count {
        for ch in order.chars() {
            let (name, attr) = match ch {
                'R' => ("赤", "red"),
                'G' => ("緑", "green"),
                'B' => ("青", "blue"),
                'W' => ("白", "white"),
                _ => continue,
            };
            let mut c = ChannelDef::blank(&format!("{n} {name}"), &format!("{attr}.{n}"), "色");
            // ブラックアウトで消えるように
            c.safe = Some(0);
            out.push(c);
        }
    }
    out
}

/// 内蔵の汎用LEDバー(RGBがピクセルの数だけ並んでいるもの)
pub fn generic_led_bar() -> Profile {
    let mut modes = Vec::new();
    for n in [1u16, 4, 6, 8, 10, 12, 16, 24, 32] {
        let name = if n == 1 { "3ch (全体で1色)".to_string() } else { format!("{}ch ({n}ピクセル)", n * 3) };
        modes.push(Mode { name, channels: pixel_channels(1, n, "RGB") });
    }
    Profile { id: "generic/led-bar-rgb".into(), name: "LEDバー (RGBピクセル)".into(), manufacturer: "汎用".into(), needs_arm: false, modes }
}

/// idからファイル名に使える文字列を作る
pub fn slug(id: &str) -> String {
    let s: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect();
    // 日本語の名前だけだと「____」になってしまうので、英数字が1つも無ければ既定名にする
    if s.chars().any(|c| c.is_ascii_alphanumeric()) { s } else { "fixture".into() }
}

/// 内蔵の定義を上書きした機材定義にも、内蔵の禁止範囲を引き継ぐ(編集で消えて、オートショーなどに入らないように)
fn inherit_forbid(p: &mut Profile, builtin: &Profile) {
    for (i, m) in p.modes.iter_mut().enumerate() {
        let Some(bm) = builtin.modes.iter().find(|b| b.name == m.name).or_else(|| builtin.modes.get(i)) else { continue };
        for ch in m.channels.iter_mut() {
            for bc in bm.channels.iter().filter(|b| b.attr == ch.attr) {
                for span in &bc.forbid {
                    if !ch.forbid.contains(span) {
                        ch.forbid.push(*span);
                    }
                }
            }
        }
    }
}

impl Library {
    pub fn preview_user(&self,profile:&Profile)->Self {
        let mut candidate=self.clone();let mut profile=profile.clone();
        for source in BUILTIN {
            if let Ok(builtin)=serde_json::from_str::<Profile>(source)
                && builtin.id==profile.id {inherit_forbid(&mut profile,&builtin);}
        }
        candidate.add(profile);candidate
    }

    pub fn register_user_file(&mut self,id:&str,path:PathBuf) {self.user_files.insert(id.to_string(),path);}

    pub fn preview_delete(&self,id:&str)->Self {
        let mut candidate=self.clone();
        candidate.profiles.retain(|p|p.id!=id);candidate.infos.remove(id);candidate.user_files.remove(id);
        for source in BUILTIN {if let Ok(profile)=serde_json::from_str::<Profile>(source)&& profile.id==id {candidate.add(profile);}}
        let bar=generic_led_bar();if bar.id==id {candidate.add(bar);}
        candidate
    }

    /// 内蔵プロファイル + ユーザーフォルダ(同じidならユーザーのものが優先)
    pub fn load(user_dir: &Path) -> (Self, Vec<String>) {
        let mut lib = Library { user_dir: user_dir.to_path_buf(), ..Library::default() };
        let mut errors = Vec::new();
        let mut builtin: HashMap<String, Profile> = HashMap::new();
        for src in BUILTIN {
            match serde_json::from_str::<Profile>(src) {
                Ok(p) => {
                    lib.builtin_ids.insert(p.id.clone());
                    builtin.insert(p.id.clone(), p.clone());
                    lib.add(p)
                }
                Err(e) => errors.push(format!("内蔵プロファイル: {e}")),
            }
        }
        let bar = generic_led_bar();
        lib.builtin_ids.insert(bar.id.clone());
        lib.add(bar);
        if let Ok(rd) = std::fs::read_dir(user_dir) {
            let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json")).collect();
            paths.sort();
            for path in paths {
                let res = std::fs::read_to_string(&path)
                    .map_err(|e| e.to_string())
                    .and_then(|s| serde_json::from_str::<Profile>(s.trim_start_matches('\u{feff}')).map_err(|e| e.to_string()));
                match res {
                    Ok(mut p) => {
                        if let Some(b) = builtin.get(&p.id) {
                            inherit_forbid(&mut p, b);
                        }
                        lib.user_files.insert(p.id.clone(), path);
                        lib.add(p)
                    }
                    Err(e) => errors.push(format!("{}: {e}", path.display())),
                }
            }
        }
        (lib, errors)
    }

    /// サーバーから受け取った機材定義で作る(クライアントの写し用。ファイルには保存しない)
    pub fn from_profiles(profiles: Vec<Profile>, builtin: Vec<String>) -> Self {
        let mut lib = Library::default();
        for p in profiles {
            lib.add(p);
        }
        lib.builtin_ids = builtin.into_iter().collect();
        lib
    }

    /// 内蔵の定義のidの一覧(クライアントへ送る)
    pub fn builtin_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.builtin_ids.iter().cloned().collect();
        ids.sort();
        ids
    }

    pub fn add(&mut self, p: Profile) {
        self.profiles.retain(|x| x.id != p.id);
        let p = Arc::new(p);
        self.infos.insert(p.id.clone(), (p.clone(), p.modes.iter().map(|m| Arc::new(ModeInfo::new(m))).collect()));
        self.profiles.push(p);
    }

    /// モードの下調べ。読み込み時に作ったもの(無ければその場で作る)
    pub fn info(&self, profile: &str, mode: usize) -> Option<Arc<ModeInfo>> {
        let p = self.get(profile)?;
        match self.infos.get(profile) {
            // 同じプロファイルから作ったものだけを使う(差し替えられていたら作り直す)
            Some((src, v)) if Arc::ptr_eq(src, p) => v.get(mode).cloned(),
            _ => p.modes.get(mode).map(|m| Arc::new(ModeInfo::new(m))),
        }
    }

    pub fn get(&self, id: &str) -> Option<&Arc<Profile>> {
        self.profiles.iter().find(|p| p.id == id)
    }

    pub fn mode(&self, profile: &str, mode: usize) -> Option<&Mode> {
        self.get(profile)?.modes.get(mode)
    }

    /// アプリ内蔵の定義があるid
    pub fn is_builtin(&self, id: &str) -> bool {
        self.builtin_ids.contains(id)
    }

    /// ユーザーフォルダに保存された定義があるid(内蔵を編集したものを含む)
    pub fn has_user_file(&self, id: &str) -> bool {
        self.user_files.contains_key(id)
    }

    /// ユーザーフォルダに保存する。途中で壊れないよう一時ファイル経由で置き換える
    pub fn save_user(&self, p: &Profile) -> Result<PathBuf, String> {
        std::fs::create_dir_all(&self.user_dir).map_err(|e| e.to_string())?;
        let path = self.user_files.get(&p.id).cloned().unwrap_or_else(|| self.user_dir.join(format!("{}.json", slug(&p.id))));
        let tmp = path.with_extension("json.tmp");
        let json = serde_json::to_string_pretty(p).map_err(|e| e.to_string())?;
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        Ok(path)
    }

    /// ユーザーフォルダの定義を削除する(内蔵の定義があれば、それに戻る)
    pub fn delete_user(&self, id: &str) -> Result<(), String> {
        match self.user_files.get(id) {
            Some(path) => std::fs::remove_file(path).map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }

    /// 使われていないidを作る
    pub fn unique_id(&self, base: &str) -> String {
        let base = format!("user/{}", slug(base).to_lowercase());
        if self.get(&base).is_none() {
            return base;
        }
        (2..).map(|n| format!("{base}-{n}")).find(|id| self.get(id).is_none()).unwrap()
    }

    /// 全機種で使われているカテゴリ(標準 + 自作)
    pub fn all_categories(&self) -> Vec<String> {
        let mut v: Vec<String> = CATEGORIES.iter().map(|c| c.to_string()).collect();
        for p in &self.profiles {
            for m in &p.modes {
                for c in &m.channels {
                    let n = c.category_name().to_string();
                    if !v.contains(&n) {
                        v.push(n);
                    }
                }
            }
        }
        v.sort_by_key(|c| category_order(c));
        v
    }

    /// 全機種で使われている機能名(attr)。エディタの候補用
    pub fn all_attrs(&self) -> Vec<String> {
        let mut v: Vec<String> = ["pan", "tilt", "dimmer", "color", "gobo", "strobe", "pt_speed", "mode"].iter().map(|s| s.to_string()).collect();
        for p in &self.profiles {
            for m in &p.modes {
                for c in &m.channels {
                    if c.fine_of.is_none() && !v.contains(&c.attr) {
                        v.push(c.attr.clone());
                    }
                }
            }
        }
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_profiles_parse() {
        let (lib, errs) = Library::load(Path::new("does-not-exist"));
        assert!(errs.is_empty(), "{errs:?}");
        assert_eq!(lib.profiles.len(), 5);
    }

    #[test]
    fn channel_counts_match_manuals() {
        let (lib, _) = Library::load(Path::new("does-not-exist"));
        let spot = lib.get("stage-evolution/minispot30").unwrap();
        assert_eq!(spot.modes[0].channels.len(), 9);
        assert_eq!(spot.modes[1].channels.len(), 11);
        let laser = lib.get("stage-evolution/laserscan300").unwrap();
        assert_eq!(laser.modes[0].channels.len(), 17);
    }

    #[test]
    fn ranges_cover_full_byte_without_gaps() {
        let (lib, _) = Library::load(Path::new("does-not-exist"));
        for p in &lib.profiles {
            for m in &p.modes {
                for c in m.channels.iter().filter(|c| !c.ranges.is_empty()) {
                    for v in 0..=255u8 {
                        let n = c.ranges.iter().filter(|r| r.contains(v)).count();
                        assert_eq!(n, 1, "{} / {} / {}: value {v} matched {n} ranges", p.name, m.name, c.name);
                    }
                }
            }
        }
    }

    #[test]
    fn sixteen_bit_roundtrip() {
        let (lib, _) = Library::load(Path::new("does-not-exist"));
        let m = lib.mode("stage-evolution/minispot30", 1).unwrap();
        let mut v = m.default_values();
        m.set_f(&mut v, 0, 100.5);
        assert_eq!((v[0], v[1]), (100, 128));
        assert!((m.get_f(&v, 0) - 100.5).abs() < 0.01);
        assert_eq!(m.fine_of(0), Some(1));
        assert_eq!(m.fine_of(2), Some(3));
    }

    #[test]
    fn builtin_profiles_are_valid_and_categorized() {
        let (lib, _) = Library::load(Path::new("does-not-exist"));
        for p in &lib.profiles {
            let r = p.validate();
            assert!(r.errors.is_empty(), "{}: {:?}", p.name, r.errors);
            assert!(r.warnings.is_empty(), "{}: {:?}", p.name, r.warnings);
            for m in &p.modes {
                for c in m.channels.iter().filter(|c| c.fine_of.is_none()) {
                    assert!(!c.category.is_empty(), "{} / {} / {}: カテゴリ未設定", p.name, m.name, c.name);
                }
            }
        }
        let spot = lib.mode("stage-evolution/minispot30", 0).unwrap();
        assert_eq!(spot.categories(), vec!["動き", "色", "光量", "形", "制御"]);
    }

    fn sample_mode() -> Mode {
        // ch1 Pan / ch2 Pan Fine(ch1の下位) / ch3 Tilt / ch4 Tilt Fine(ch3の下位) / ch5 Dimmer
        let mut m = Mode { name: "m".into(), channels: vec![] };
        for (n, a) in ["Pan", "Pan Fine", "Tilt", "Tilt Fine", "Dimmer"].iter().zip(["pan", "pan_fine", "tilt", "tilt_fine", "dimmer"]) {
            m.channels.push(ChannelDef::blank(n, a, "動き"));
        }
        m.channels[1].fine_of = Some(1);
        m.channels[3].fine_of = Some(3);
        m
    }

    #[test]
    fn moving_channels_keeps_16bit_links() {
        let mut m = sample_mode();
        // Dimmerを先頭へ(4回上へ)
        for i in (1..5).rev() {
            m.move_channel(i, -1);
        }
        assert_eq!(m.channels[0].attr, "dimmer");
        assert_eq!(m.channels[2].attr, "pan_fine");
        assert_eq!(m.channels[2].fine_of, Some(2), "Pan Fineは今ch2のPanの下位");
        assert_eq!(m.channels[4].fine_of, Some(4));
        assert_eq!(m.fine_of(1), Some(2));
    }

    #[test]
    fn inserting_and_removing_channels_fixes_references() {
        let mut m = sample_mode();
        m.insert_channel(0, ChannelDef::blank("New", "new", "色"));
        assert_eq!(m.channels[2].fine_of, Some(2), "Pan Fine(ch3)の上位はPan(ch2)");
        assert_eq!(m.channels[4].fine_of, Some(4));
        // Pan(上位)を消すと、下位側の参照は外れる
        m.remove_channel(1);
        assert_eq!(m.channels[1].attr, "pan_fine");
        assert_eq!(m.channels[1].fine_of, None);
        assert_eq!(m.channels[3].fine_of, Some(3), "Tilt Fine の上位 Tilt はch3に繰り上がる");
    }

    #[test]
    fn validation_catches_common_mistakes() {
        let (lib, _) = Library::load(Path::new("none"));
        let mut p = (**lib.get("stage-evolution/minispot30").unwrap()).clone();
        assert!(p.validate().errors.is_empty());
        p.modes[0].channels[1].attr = "pan".into(); // 重複
        p.modes[0].channels[2].attr.clear(); // 空
        p.modes[1].channels[1].fine_of = Some(2); // 自分自身(ch2)を上位に
        let e = p.validate().errors.join("\n");
        assert!(e.contains("重複"), "{e}");
        assert!(e.contains("機能名が空"), "{e}");
        assert!(e.contains("上位チャンネルの指定が不正"), "{e}");
    }

    #[test]
    fn range_issue_detection() {
        let r = |a: u8, b: u8| Range { from: a, to: b, label: "x".into(), sweep: false, warn: false, color: None };
        assert!(range_issues(&[r(0, 127), r(128, 255)]).is_empty());
        assert!(range_issues(&[r(0, 99), r(110, 255)]).iter().any(|s| s.contains("100〜109")));
        assert!(range_issues(&[r(0, 130), r(128, 255)]).iter().any(|s| s.contains("128〜130")));
    }

    #[test]
    fn user_profiles_save_override_and_delete() {
        let dir = std::env::temp_dir().join(format!("dmxctl-fx-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let (lib, _) = Library::load(&dir);
        let mut p = (**lib.get("stage-evolution/minispot30").unwrap()).clone();
        p.name = "編集済みスポット".into();
        let path = lib.save_user(&p).unwrap();
        assert!(path.exists());

        let (lib2, errs) = Library::load(&dir);
        assert!(errs.is_empty());
        assert_eq!(lib2.get("stage-evolution/minispot30").unwrap().name, "編集済みスポット", "同じidならユーザー定義が優先");
        assert!(lib2.is_builtin("stage-evolution/minispot30") && lib2.has_user_file("stage-evolution/minispot30"));
        assert_eq!(lib2.profiles.len(), lib.profiles.len(), "上書きなので、機材の数は増えない");

        lib2.delete_user("stage-evolution/minispot30").unwrap();
        let (lib3, _) = Library::load(&dir);
        assert_eq!(lib3.get("stage-evolution/minispot30").unwrap().name, "MINI SPOT 30", "削除で内蔵の定義に戻る");

        // 新規: 使われていないidが作られる
        assert_eq!(lib3.unique_id("My Light"), "user/my_light");
        assert_eq!(lib3.unique_id("新しい機材"), "user/fixture");
        std::fs::remove_dir_all(&dir).ok();
    }
}
