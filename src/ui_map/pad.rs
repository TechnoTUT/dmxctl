//! 配置図の1コントロール分の描画: 種類ごとの色、パッドに出す文字、フェーダーの値、プレビュー。

use super::*;

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

// 種類ごとの色(実機のパッドのLEDとおおよそ揃える)
const COL_SET: Color32 = rgb(0x3B, 0x82, 0xF6);
const COL_SCENE: Color32 = rgb(0x10, 0xB9, 0x81);
const COL_FX: Color32 = rgb(0xF5, 0x9E, 0x0B);
const COL_CALM: Color32 = rgb(0x38, 0xBD, 0xF8);
const COL_GROOVE: Color32 = rgb(0x8B, 0x5C, 0xF6);
const COL_PEAK: Color32 = rgb(0xF4, 0x3F, 0x5E);
const COL_MONO: Color32 = rgb(0xCB, 0xD5, 0xE1);
const COL_DARK: Color32 = rgb(0xEF, 0x44, 0x44);
const COL_LEVEL: Color32 = rgb(0xFB, 0xBF, 0x24);
const COL_TEMPO: Color32 = rgb(0x22, 0xD3, 0xEE);
const COL_CHASE: Color32 = rgb(0x84, 0xCC, 0x16);
const COL_PAIR: Color32 = rgb(0xEC, 0x48, 0x99);
const COL_COLOR: Color32 = rgb(0x14, 0xB8, 0xA6);
const COL_SWAP: Color32 = rgb(0xD9, 0x46, 0xEF);
const COL_POSITION: Color32 = rgb(0x0E, 0xA5, 0xE9);
const COL_PAGE: Color32 = rgb(0x64, 0x74, 0x8B);
const COL_FLASH: Color32 = rgb(0xFD, 0xE6, 0x8A);
const COL_PIXEL: Color32 = rgb(0xA7, 0x8B, 0xFA);

/// パッドの地の色
const PAD: Color32 = rgb(0x1B, 0x1F, 0x27);

/// 凡例(種類と色)
const LEGEND: [(&str, Color32); 16] = [
    ("LEDバーの模様: チル", COL_CALM),
    ("LEDバーの模様: グルーヴ", COL_GROOVE),
    ("LEDバーの模様: ピーク", COL_PEAK),
    ("LEDバーの模様: モノクロ", COL_MONO),
    ("エフェクト(ムービングなど)", COL_FX),
    ("色・メイン/サブ色(実際の色で表示)", COL_COLOR),
    ("メイン⇄サブ入れ替え", COL_SWAP),
    ("チャンネル値", COL_SET),
    ("シーン", COL_SCENE),
    ("チェイス", COL_CHASE),
    ("テンポ", COL_TEMPO),
    ("明るさ・全体光量", COL_LEVEL),
    ("LEDバーの速さ・斜め", COL_PIXEL),
    ("フラッシュ", COL_FLASH),
    ("ブラックアウト・暗転・ミュート", COL_DARK),
    ("ページ・位置・ペア", COL_PAGE),
];

pub(super) fn energy_color(e: Option<Energy>) -> Color32 {
    match e {
        Some(Energy::Calm) => COL_CALM,
        Some(Energy::Groove) => COL_GROOVE,
        Some(Energy::Peak) => COL_PEAK,
        Some(Energy::Mono) => COL_MONO,
        None => COL_FX,
    }
}

pub(crate) fn action_color(a: &Action, p: &Project) -> Color32 {
    match a {
        Action::SetValue { .. } => COL_SET,
        Action::Scene { .. } => COL_SCENE,
        Action::Effect { name, .. } => energy_color(crate::pixel::energy_of(p, name)),
        Action::Blackout { .. } | Action::Fade { .. } | Action::Mute { .. } => COL_DARK,
        Action::Master { .. } | Action::Level { .. } => COL_LEVEL,
        Action::Tempo { .. } => COL_TEMPO,
        Action::Chase { .. } => COL_CHASE,
        Action::Pair { .. } => COL_PAIR,
        Action::Color { .. } | Action::ColorScope { .. } | Action::Select { .. } => COL_COLOR,
        Action::Palette { .. } => COL_SWAP,
        Action::Position { .. } => COL_POSITION,
        Action::Page { .. } => COL_PAGE,
        Action::Flash { .. } => COL_FLASH,
        Action::Pixel { .. } => COL_PIXEL,
        Action::Role { .. } | Action::LiveColor { .. } => COL_COLOR,
        Action::Arm { .. } => COL_DARK,
        Action::SwapBeats { .. } => COL_SWAP,
        Action::BarCount { .. } => COL_PIXEL,
        // 動きは種類ごとに色分け(APCのLEDと同じ: 形=オレンジ / 1周の長さ=黄 / 揃え方=ピンク / 向き=水色)
        Action::Motion { op } => match op {
            crate::motion::MotionOp::Beats(_) => COL_LEVEL,
            crate::motion::MotionOp::Spread(_) => COL_PAIR,
            crate::motion::MotionOp::Position(_) => COL_POSITION,
            _ => COL_FX,
        },
        Action::KeyPage { .. } => COL_PAGE,
    }
}

/// 対象の名前(前置きなし)
fn target_plain(sel: &Selector, meta: &SelMeta) -> String {
    match sel {
        Selector::All => "全機材".into(),
        Selector::Fixture(_) => meta.label(sel),
        Selector::Group(g) => g.clone(),
        Selector::Profile(p) => meta.profiles.iter().find(|(id, _)| id == p).map(|(_, n)| n.clone()).unwrap_or_else(|| p.clone()),
    }
}

/// パッドに出す文字: (小さい見出し, 大きい文字)
pub(super) fn pad_text(a: &Action, meta: &SelMeta, core: &Core) -> (String, String) {
    let mode = |m: &PadMode| match m {
        PadMode::Continuous => "連続",
        PadMode::Momentary => "押中",
        PadMode::Toggle => "",
        PadMode::Trigger => "1回",
    };
    let s = |x: &str| x.to_string();
    match a {
        Action::Effect { name, mode: m } => {
            // LEDバーの模様は雰囲気、ほかのエフェクトは対象の機材
            let head = match crate::pixel::energy_of(&core.project, name) {
                Some(e) => s(e.label()),
                None => core.project.effects.iter().find(|e| e.name == *name).map(|e| target_plain(&e.sel, meta)).unwrap_or_else(|| s("エフェクト")),
            };
            let head = if *m == PadMode::Toggle { head } else { format!("{head}・{}", mode(m)) };
            (head, name.clone())
        }
        Action::Color { color, target } => {
            let head = match target {
                crate::color::ColorTarget::Main => "メイン",
                crate::color::ColorTarget::Sub => "サブ",
                crate::color::ColorTarget::Selected => "選択中の色",
                _ => "色",
            };
            (s(head), s(color.map(|c| c.label()).unwrap_or("色相")))
        }
        Action::SetValue { sel, attr, mode: m, on, .. } => {
            // 選択式のチャンネルは、ON値の名前(「回転」「オートプログラム1」など)
            let title = meta.ranges(&core.lib, sel, attr).iter().find(|r| r.contains(on.clamp(0.0, 255.0) as u8)).map(|r| r.label.clone()).unwrap_or_else(|| attr.clone());
            let head = target_plain(sel, meta);
            let head = if mode(m).is_empty() { head } else { format!("{head}・{}", mode(m)) };
            (head, title)
        }
        Action::Flash { white, mode: m } => (if *m == PadMode::Toggle { s("フラッシュ") } else { s("フラッシュ・押中") }, s(if *white { "白で全開" } else { "今の色で全開" })),
        Action::Blackout { mode: m } => (s(if *m == PadMode::Toggle { "全部消す" } else { "押中" }), s(if *m == PadMode::Toggle { "BLACKOUT" } else { "消灯" })),
        Action::Fade { secs, .. } => (format!("{secs}秒"), s("暗転")),
        Action::Tempo { op } => match op {
            TempoOp::Tap => (s("テンポ"), s("TAP")),
            TempoOp::Resync => (s("テンポ"), s("頭出し")),
            TempoOp::Set { .. } => (s("テンポ"), s("BPM")),
            TempoOp::Nudge { delta } => (s("テンポ"), format!("BPM {delta:+}")),
            TempoOp::Link => (s("テンポ"), s("Link")),
            TempoOp::Audio => (s("テンポ"), s("音声")),
        },
        Action::Palette { op } => match op {
            crate::color::PaletteOp::Swap => (s("色"), s("メイン⇄サブ")),
            crate::color::PaletteOp::HoldSwap => (s("押中"), s("メイン⇄サブ")),
            crate::color::PaletteOp::Auto => (s("拍で"), s("自動入れ替え")),
            crate::color::PaletteOp::Fade => (s("ゆっくり"), s("グラデーション")),
        },
        Action::Mute { sel, .. } => (s("ミュート"), target_plain(sel, meta)),
        Action::Level { sel } => (s("明るさ"), target_plain(sel, meta)),
        Action::Pixel { param } => (s("LEDバー"), s(if *param == PixelParam::Speed { "速さ" } else { "斜め" })),
        Action::Master { .. } => (s("全体光量"), s("MASTER")),
        Action::Scene { name } => (s("シーン"), name.clone()),
        Action::Chase { name, .. } => (s("チェイス"), name.clone()),
        Action::Pair { name, op, .. } => (s(if *op == PairOp::Link { "ペア連動" } else { "ペア反転" }), name.clone()),
        Action::Position { name } => (s("位置"), name.clone()),
        Action::Select { fixture, .. } => (s("選択"), meta.fixtures.iter().find(|f| f.id == *fixture).map(|f| f.name.clone()).unwrap_or_else(|| s("(削除された機材)"))),
        Action::ColorScope { .. } => (s("色の操作先"), s("全体/個別")),
        Action::Page { op } => (s("ページ"), op.label()),
        Action::Role { sel, op } => (target_plain(sel, meta), s(match op {
            RoleOp::Toggle => {
                // 今の系統を出す
                match core.resolve(sel).first().map(|&i| core.project.fixtures[i].color_role) {
                    Some(crate::color::ColorRole::Sub) => "サブ",
                    Some(crate::color::ColorRole::Off) => "対象外",
                    _ => "メイン",
                }
            }
            RoleOp::Main => "全部メイン",
            RoleOp::Sub => "全部サブ",
            RoleOp::Alternate => "1台おき",
        })),
        Action::Arm { sel, .. } => (target_plain(sel, meta), s("ARM")),
        Action::LiveColor { role, source } => (format!("NDI → {}", role.label()), s(match source {
            crate::ndi::LiveSource::Video => "映像の色",
            crate::ndi::LiveSource::Audio => "音の色",
            crate::ndi::LiveSource::Off => "使わない",
        })),
        Action::SwapBeats { beats } => (s("入れ替え"), format!("{beats}拍ごと")),
        Action::BarCount { count } => (s("LEDバー"), format!("{count}本")),
        Action::Motion { op } => (s(op.group()), op.label()),
        Action::KeyPage { op } => (s("模様セット"), s(if *op == PageOp::Prev { "◀ 前" } else { "次 ▶" })),
    }
}

/// フェーダーの値の出し方
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Readout {
    Raw,
    Percent,
    Hue,
    Speed,
    Skew,
}

pub(super) fn readout(a: &Action) -> Readout {
    match a {
        Action::Master { .. } | Action::Level { .. } => Readout::Percent,
        Action::Color { color: None, .. } => Readout::Hue,
        Action::Pixel { param: PixelParam::Speed } => Readout::Speed,
        Action::Pixel { param: PixelParam::Skew } => Readout::Skew,
        _ => Readout::Raw,
    }
}

fn readout_text(r: Readout, v: u8) -> String {
    let f = v as f32 / 127.0;
    match r {
        Readout::Raw => v.to_string(),
        Readout::Percent => format!("{:.0}%", f * 100.0),
        Readout::Hue => format!("{:.0}°", f * 360.0),
        Readout::Speed => crate::pixel::speed_label(crate::pixel::speed_step(f)),
        Readout::Skew => format!("+{:.1}周", f * crate::pixel::SKEW_FADER_MAX),
    }
}

/// パッドの中に描くプレビュー
pub(super) enum Preview {
    None,
    /// LEDバーの模様: バー(行) × 点(列) の色
    Face(Vec<Vec<[f32; 3]>>),
    /// ムービングなどのエフェクト: 1周期の軌跡(0〜1の座標)と、機材ごとの今の位置
    Path { pts: Vec<[f32; 2]>, now: Vec<[f32; 2]> },
}

/// エフェクト(LEDバー以外)の1周期の軌跡。Pan/Tiltがあれば2次元(円運動など)、無ければ時間と値の波形
pub(super) fn effect_path(core: &Core, name: &str) -> Option<Preview> {
    let fx: Vec<&Effect> = core.project.effects.iter().filter(|e| e.name == name).collect();
    let first = *fx.first()?;
    // 拍(自由な速さのエフェクトは秒)を単位にして、機能ごとに違う周期(8の字など)も1つの軌跡にする
    let period = |e: &Effect| if e.beats > 0.0 { e.beats as f64 } else { 1.0 / e.rate_hz.max(0.001) as f64 };
    let clock = if first.beats > 0.0 { core.rt.beat } else { core.rt.t0.elapsed().as_secs_f64() };
    let val = |e: &Effect, t: f64| -> f32 {
        let p = t / period(e) + e.phase as f64 / 360.0;
        let w = e.wave.eval(p.rem_euclid(1.0) as f32, p.floor() as i64 as u64, 1);
        (e.min + (e.max - e.min) * w) / 255.0
    };
    let span = fx.iter().map(|e| period(e)).fold(0.0, f64::max);
    let pan = fx.iter().find(|e| e.attr == "pan");
    let tilt = fx.iter().find(|e| e.attr == "tilt");
    const N: usize = 64;
    // 機材ごとのずれ(spread)は、それぞれの今の位置の点で見せる
    let count = core.resolve(&first.sel).len().clamp(1, 8);
    let shift = |e: &Effect, n: usize| e.spread as f64 / 360.0 * period(e) * n as f64;
    let now = |n: usize, at: &dyn Fn(f64, usize) -> [f32; 2]| at(clock.rem_euclid(span), n);
    let preview = match (pan, tilt) {
        (Some(p), Some(t)) => {
            let at = |s: f64, n: usize| [val(p, s + shift(p, n)), 1.0 - val(t, s + shift(t, n))];
            Preview::Path { pts: (0..=N).map(|i| at(i as f64 / N as f64 * span, 0)).collect(), now: (0..count).map(|n| now(n, &at)).collect() }
        }
        _ => {
            let curve = |s: f64| [(s / span) as f32, 1.0 - val(first, s)];
            let at = |s: f64, n: usize| curve((s + shift(first, n)).rem_euclid(span));
            Preview::Path { pts: (0..=N).map(|i| curve(i as f64 / N as f64 * span)).collect(), now: (0..count).map(|n| now(n, &at)).collect() }
        }
    };
    Some(preview)
}

/// 図に描く1コントロール分の情報
pub(super) struct Info {
    pub(super) tag: String,
    pub(super) title: String,
    pub(super) tip: String,
    pub(super) count: usize,
    pub(super) color: Option<Color32>,
    /// エフェクトON中・トグルON中・ブラックアウト中など「効いている」状態
    pub(super) active: bool,
    /// 実際の色(色を指定するパッドなど)
    pub(super) rgb: Option<[f32; 3]>,
    pub(super) readout: Readout,
    pub(super) preview: Preview,
}

pub(super) fn rgb32(c: [f32; 3], level: f32) -> Color32 {
    let f = |x: f32| (x.clamp(0.0, 1.0) * level.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(f(c[0]), f(c[1]), f(c[2]))
}

/// 種類と色の凡例(マウスを乗せると出る)
pub(super) fn legend(ui: &mut egui::Ui) {
    let resp = ui.label(RichText::new("凡例").small().color(theme::TEXT_DIM));
    resp.on_hover_ui(|ui| {
        ui.label(RichText::new("パッドの色").font(theme::bold(13.0)));
        for (name, c) in LEGEND {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                ui.painter().rect_filled(r, 3.0, c);
                ui.label(RichText::new(name).small());
            });
        }
        ui.add_space(4.0);
        ui.label(RichText::new("明るく光っている = 効いている(ON)\n白い枠 = 押している / ミントの枠 = 編集中").small().color(theme::TEXT_DIM));
    });
}

/// パッド/ボタン1個を描いて、クリックされたらtrueを返す
pub(super) fn button_cell(ui: &mut egui::Ui, rect: Rect, ctl: Ctl, info: &Info, down: bool, selected: bool) -> bool {
    let resp = ui.interact(rect, ui.id().with(("apc", ctl.trigger().1, ctl.trigger().0 == TriggerKind::Cc)), Sense::click());
    let p = ui.painter();
    let radius = 7.0;
    let small = rect.width() < 70.0 || rect.height() < 34.0;

    if info.count == 0 {
        p.rect_filled(rect, radius, if resp.hovered() { mix(PAD, Color32::WHITE, 0.05) } else { PAD });
        p.rect_stroke(rect, radius, Stroke::new(1.0, mix(PAD, Color32::WHITE, 0.06)), StrokeKind::Inside);
        if down {
            p.rect_stroke(rect, radius, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
        }
        if selected {
            p.rect_stroke(rect.expand(1.5), radius + 1.5, Stroke::new(2.5, theme::ACCENT), StrokeKind::Outside);
        }
        let hex = format!("{:02X}", ctl.trigger().1);
        p.text(rect.center(), Align2::CENTER_CENTER, hex, FontId::monospace(10.0), theme::TEXT_FAINT);
        return resp.on_hover_text(&info.tip).clicked();
    }

    // 色に関係するパッドは実際の色(待機中は暗め)、ほかは種類の色
    let accent = info.rgb.map(|c| rgb32(c, 1.0)).or(info.color).unwrap_or(theme::LINE);
    let mut fill = if info.rgb.is_some() {
        if info.active { accent } else { mix(PAD, accent, 0.42) }
    } else if info.active {
        mix(PAD, accent, 0.82)
    } else {
        mix(PAD, accent, 0.16)
    };
    if resp.hovered() {
        fill = mix(fill, Color32::WHITE, 0.07);
    }
    if down {
        fill = mix(fill, Color32::WHITE, 0.45);
    }
    if info.active {
        // 効いている間は、まわりにぼんやり光を出す
        p.rect_filled(rect.expand(3.0), radius + 3.0, alpha(accent, 0.30));
    }
    p.rect_filled(rect, radius, fill);
    // プレビュー(LEDバーの模様 / エフェクトの軌跡)
    let has_preview = !small && !matches!(info.preview, Preview::None);
    if has_preview {
        draw_preview(p, rect, &info.preview, info.active, accent);
        if resp.hovered() {
            // マウスを乗せている間は、なめらかに動かす
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(66));
        }
    }
    // 種類の色の細い線(実機のLEDの代わり)。実際の色のパッドは全体が色なので付けない
    if has_preview {
        p.rect_stroke(rect, radius, Stroke::new(1.5, alpha(accent, if info.active { 1.0 } else { 0.55 })), StrokeKind::Inside);
    } else if info.rgb.is_none() && !small {
        let strip = Rect::from_min_size(rect.min + vec2(7.0, 5.0), vec2(rect.width() - 14.0, 3.0));
        p.rect_filled(strip, 1.5, if info.active { alpha(Color32::WHITE, 0.85) } else { accent });
    }
    if down {
        p.rect_stroke(rect, radius, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
    } else if info.rgb.is_some() && info.active {
        p.rect_stroke(rect, radius, Stroke::new(2.0, alpha(Color32::WHITE, 0.9)), StrokeKind::Inside);
    }
    if selected {
        p.rect_stroke(rect.expand(1.5), radius + 1.5, Stroke::new(2.5, theme::ACCENT), StrokeKind::Outside);
    }

    let lit = info.active || info.rgb.is_some() || down;
    let fg = if lit { theme::text_on(fill) } else { theme::TEXT };
    let tag_color = if lit { alpha(fg, 0.72) } else { mix(accent, Color32::WHITE, 0.35) };
    let clip = p.with_clip_rect(rect.shrink(2.0));
    let inner_w = rect.width() - if small { 10.0 } else { 13.0 };
    let mut title_size = if small { 11.0 } else { (rect.height() * 0.21).clamp(11.0, 14.0) };
    // 1語の長い名前(BLACKOUTなど)は、途中で折り返さないよう文字を小さくして収める
    let one_line = clip.layout_no_wrap(info.title.clone(), theme::bold(title_size), fg).size().x;
    // 背の低いボタン(トラックボタンなど)は、名前が長くても1行に収める
    if ((!info.title.contains(' ') && info.title.is_ascii()) || small) && one_line > inner_w {
        title_size = (title_size * inner_w / one_line).max(6.5);
    }
    let title = clip.layout(info.title.clone(), theme::bold(title_size), fg, inner_w);
    let tag_h = 24.0;
    if has_preview {
        // プレビューが名前の代わり(名前はマウスを乗せると出る)
    } else if small || title.size().y > rect.height() - tag_h - 4.0 {
        // 狭いボタンや、名前が2行になって見出しと重なるときは、名前だけを真ん中に
        clip.galley(pos2(rect.min.x + 7.0, rect.center().y - title.size().y / 2.0 + if small { 0.0 } else { 3.0 }), title, fg);
    } else {
        let tag = clip.layout_no_wrap(info.tag.clone(), FontId::proportional(10.0), tag_color);
        clip.galley(pos2(rect.min.x + 7.0, rect.min.y + 11.0), tag, tag_color);
        clip.galley(pos2(rect.min.x + 7.0, rect.max.y - title.size().y - 6.0), title, fg);
    }
    resp.on_hover_text(&info.tip).clicked()
}

pub(super) fn fader_cell(ui: &mut egui::Ui, rect: Rect, ctl: Ctl, info: &Info, value: Option<u8>, selected: bool) -> bool {
    let resp = ui.interact(rect, ui.id().with(("apc", ctl.trigger().1, true)), Sense::click());
    let p = ui.painter();
    let radius = 8.0;
    let accent = info.color.unwrap_or(theme::TEXT_FAINT);
    p.rect_filled(rect, radius, if resp.hovered() { mix(PAD, Color32::WHITE, 0.04) } else { mix(PAD, Color32::BLACK, 0.15) });
    p.rect_stroke(rect, radius, Stroke::new(1.0, mix(PAD, Color32::WHITE, 0.06)), StrokeKind::Inside);

    // 見出し(上)と値(下)のあいだに、つまみの溝
    let title = match ctl {
        Ctl::Fader(8) => "MASTER".to_string(),
        Ctl::Fader(i) => format!("F{}", i + 1),
        _ => String::new(),
    };
    let head_h = if info.count > 0 { 36.0 } else { 20.0 };
    let groove = Rect::from_min_max(pos2(rect.center().x - 2.0, rect.min.y + head_h), pos2(rect.center().x + 2.0, rect.max.y - 22.0));
    p.rect_filled(groove, 2.0, mix(PAD, Color32::BLACK, 0.45));
    if let Some(v) = value {
        let t = v as f32 / 127.0;
        let y = groove.max.y - groove.height() * t;
        let fill_color = if info.readout == Readout::Hue {
            let [r, g, b] = crate::color::hsv(t * 360.0, 1.0, 1.0);
            Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
        } else {
            accent
        };
        p.rect_filled(Rect::from_min_max(pos2(groove.min.x, y), groove.max), 2.0, fill_color);
        let cap = Rect::from_center_size(pos2(rect.center().x, y), vec2((rect.width() - 16.0).min(46.0), 9.0));
        p.rect_filled(cap, 3.0, Color32::from_gray(232));
        p.hline(cap.x_range().shrink(5.0), cap.center().y, Stroke::new(1.0, fill_color));
    }
    if selected {
        p.rect_stroke(rect.expand(1.5), radius + 1.5, Stroke::new(2.5, theme::ACCENT), StrokeKind::Outside);
    }
    let clip = p.with_clip_rect(rect.shrink(2.0));
    clip.text(pos2(rect.min.x + 7.0, rect.min.y + 6.0), Align2::LEFT_TOP, &title, FontId::monospace(10.0), if info.count > 0 { accent } else { theme::TEXT_FAINT });
    if info.count > 0 {
        let g = clip.layout(info.title.clone(), theme::bold(11.0), theme::TEXT, rect.width() - 10.0);
        clip.galley(pos2(rect.min.x + 6.0, rect.min.y + 19.0), g, theme::TEXT);
    }
    let val = value.map(|v| readout_text(info.readout, v)).unwrap_or_else(|| "--".into());
    clip.text(pos2(rect.center().x, rect.max.y - 11.0), Align2::CENTER_CENTER, val, FontId::monospace(11.0), theme::TEXT_DIM);
    resp.on_hover_text(&info.tip).clicked()
}

/// パッドの中のプレビューを描く
fn draw_preview(p: &egui::Painter, rect: Rect, preview: &Preview, active: bool, accent: Color32) {
    let area = rect.shrink(4.0);
    match preview {
        Preview::None => {}
        Preview::Face(rows) => {
            let level = if active { 1.0 } else { 0.55 };
            let n = rows.len().max(1);
            let gap = 1.0;
            let rh = ((area.height() - gap * (n as f32 - 1.0)) / n as f32).max(1.0);
            // 点の数が多いので、四角を1つずつ描かずに1つのメッシュにまとめる(描画が軽い)
            let mut mesh = egui::Mesh::default();
            for (r, row) in rows.iter().enumerate() {
                let cols = row.len().max(1);
                let cw = area.width() / cols as f32;
                let y = area.min.y + r as f32 * (rh + gap);
                for (c, col) in row.iter().enumerate() {
                    mesh.add_colored_rect(Rect::from_min_size(pos2(area.min.x + c as f32 * cw, y), vec2(cw + 0.4, rh)), rgb32(*col, level));
                }
            }
            p.add(egui::Shape::mesh(mesh));
        }
        Preview::Path { pts, now } => {
            // 枠いっぱいに描く(動く範囲を拡大)
            let area = area.shrink2(vec2(6.0, 5.0));
            let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
            for q in pts.iter().chain(now.iter()) {
                for d in 0..2 {
                    lo[d] = lo[d].min(q[d]);
                    hi[d] = hi[d].max(q[d]);
                }
            }
            let norm = |v: f32, d: usize| if hi[d] - lo[d] < 1e-4 { 0.5 } else { (v - lo[d]) / (hi[d] - lo[d]) };
            let to = |q: &[f32; 2]| pos2(area.min.x + norm(q[0], 0) * area.width(), area.min.y + norm(q[1], 1) * area.height());
            let line: Vec<egui::Pos2> = pts.iter().map(to).collect();
            p.add(egui::Shape::line(line, Stroke::new(1.5, alpha(accent, if active { 0.95 } else { 0.6 }))));
            for q in now {
                let dot = Rect::from_center_size(to(q), vec2(6.0, 6.0));
                p.rect_filled(dot, 1.0, if active { Color32::WHITE } else { alpha(Color32::WHITE, 0.7) });
            }
        }
    }
}
