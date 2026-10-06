//! MIDIタブ(メイン画面): APC mini mk2 の配置図。割り当ての確認・編集と、押した/動かした状態の表示。
//! 右側は、何も選んでいないときは「ステージ」(LEDバーの今の模様・メイン/サブ色・機材の色)、
//! 図のコントロールを選ぶと、その割り当ての編集になる

use crate::apcmini::{pad_note, Ctl};
use crate::app::{App, SelMeta};
use crate::engine::{lock, Core};
use crate::pixel::{Energy, PixelParam};
use crate::project::*;
use crate::theme::{self, alpha, mix};
use crate::ui_binding::{binding_editor, default_action, names, summary, EditCtx, Learn};
use eframe::egui::{self, pos2, vec2, Align, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, StrokeKind, UiBuilder};

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

fn energy_color(e: Option<Energy>) -> Color32 {
    match e {
        Some(Energy::Calm) => COL_CALM,
        Some(Energy::Groove) => COL_GROOVE,
        Some(Energy::Peak) => COL_PEAK,
        Some(Energy::Mono) => COL_MONO,
        None => COL_FX,
    }
}

fn action_color(a: &Action, p: &Project) -> Color32 {
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
fn pad_text(a: &Action, meta: &SelMeta, core: &Core) -> (String, String) {
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
        },
        Action::Palette { op } => match op {
            crate::color::PaletteOp::Swap => (s("色"), s("メイン⇄サブ")),
            crate::color::PaletteOp::HoldSwap => (s("押中"), s("メイン⇄サブ")),
            crate::color::PaletteOp::Auto => (s("拍で"), s("自動入れ替え")),
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
    }
}

/// フェーダーの値の出し方
#[derive(Clone, Copy, PartialEq)]
enum Readout {
    Raw,
    Percent,
    Hue,
    Speed,
    Skew,
}

fn readout(a: &Action) -> Readout {
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

/// ページを使っているか(ページ別の割り当てか、ページ切替の割り当てがある)
fn pages_in_use(p: &Project) -> bool {
    p.bindings.iter().any(|b| b.page.is_some() || matches!(b.action, Action::Page { .. }))
}

/// シーンボタン1〜8(右の列)を「ページ1〜8へ」にする。使用中のボタンは飛ばす。戻り値=(追加, 飛ばした)
fn add_page_template(p: &mut Project) -> (usize, usize) {
    let (mut added, mut skipped) = (0, 0);
    for i in 0..PAGES.min(8) {
        let ctl = Ctl::Scene(i);
        if p.bindings.iter().any(|b| b.trigger.is_some_and(|t| ctl.matches(&t))) {
            skipped += 1;
            continue;
        }
        let (kind, number) = ctl.trigger();
        p.bindings.push(Binding {
            name: format!("ページ{}", i + 1),
            trigger: Some(MidiTrigger { kind, channel: None, number }),
            action: Action::Page { op: PageOp::Go(i) },
            page: None,
            osc: None,
        });
        added += 1;
    }
    (added, skipped)
}

/// パッドの中に描くプレビュー
enum Preview {
    None,
    /// LEDバーの模様: バー(行) × 点(列) の色
    Face(Vec<Vec<[f32; 3]>>),
    /// ムービングなどのエフェクト: 1周期の軌跡(0〜1の座標)と、機材ごとの今の位置
    Path { pts: Vec<[f32; 2]>, now: Vec<[f32; 2]> },
}

/// エフェクト(LEDバー以外)の1周期の軌跡。Pan/Tiltがあれば2次元(円運動など)、無ければ時間と値の波形
fn effect_path(core: &Core, name: &str) -> Option<Preview> {
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
struct Info {
    tag: String,
    title: String,
    tip: String,
    count: usize,
    color: Option<Color32>,
    /// エフェクトON中・トグルON中・ブラックアウト中など「効いている」状態
    active: bool,
    /// 実際の色(色を指定するパッドなど)
    rgb: Option<[f32; 3]>,
    readout: Readout,
    preview: Preview,
}

/// 配置メニューで選ばれたこと(Coreを借りている間は実行できないものは、借り終えてから実行する)
#[derive(Default)]
struct MenuOut {
    recommended: bool,
    page: bool,
    color: bool,
    palette: bool,
    pixel: bool,
    read_faders: bool,
    move_common: bool,
}

impl App {
    pub fn tab_map(&mut self, ui: &mut egui::Ui) {
        let shared = self.core.clone();
        let mut guard = lock(&shared);
        let core = &mut *guard;
        let lib = core.lib.clone();
        let meta = SelMeta::new(&core.project, &lib);
        let nm = names(&core.project);
        let cx = EditCtx::new(&meta, &lib, &nm);
        let mut changed = false;
        let mut menu = MenuOut::default();
        let mut new_page: Option<u8> = None;
        let mut remove: Option<usize> = None;
        let mut clicked: Option<Ctl> = None;
        let mut close_editor = false;
        let page = core.rt.page;
        let use_pages = pages_in_use(&core.project);

        // 左に図、右にパネル
        let avail = ui.available_rect_before_wrap();
        let panel_w = (avail.width() * 0.32).clamp(330.0, 470.0);
        let left = Rect::from_min_max(avail.min, pos2(avail.max.x - panel_w - 16.0, avail.max.y));
        let right = Rect::from_min_max(pos2(avail.max.x - panel_w, avail.min.y), avail.max);
        ui.allocate_rect(avail, Sense::hover());
        ui.painter().vline(left.max.x + 8.0, avail.y_range(), Stroke::new(1.0, theme::LINE));

        // ---- 左: APC mini mk2 の図 ----
        let common_pads = core.project.bindings.iter().filter(|b| b.page.is_none() && b.trigger.is_some_and(|t| t.kind == TriggerKind::Note && t.number < 64 && t.channel.is_none_or(|c| c == 0))).count();
        let mut lui = ui.new_child(UiBuilder::new().max_rect(left).layout(Layout::top_down(Align::Min)));
        {
            let ui = &mut lui;
            if use_pages {
            ui.horizontal(|ui| {
                {
                    ui.label(RichText::new("ページ").small().color(theme::TEXT_DIM));
                    for p in 0..PAGES {
                        let used = core.project.bindings.iter().any(|b| b.page == Some(p));
                        let text = RichText::new(format!("{}", p + 1)).color(if used || p == page { theme::TEXT } else { theme::TEXT_FAINT });
                        if ui.selectable_label(p == page, text).on_hover_text("このページに切り替えます(コントローラーのパッドもこのページになります)").clicked() {
                            new_page = Some(p);
                        }
                    }
                }
            });
            ui.add_space(6.0);
            }

            let rt = &core.rt;
            let bindings = &core.project.bindings;
            let core_ref: &Core = core;
            let look_rgb = crate::led::look_colors(core_ref);
            let info = |ctl: Ctl| -> Info {
                let hits: Vec<&Binding> = bindings.iter().filter(|b| b.on_page(page) && b.trigger.is_some_and(|t| ctl.matches(&t))).collect();
                let Some(first) = hits.first() else {
                    return Info { tag: String::new(), title: String::new(), tip: format!("{}\n割り当てなし", ctl.label()), count: 0, color: None, active: false, rgb: None, readout: Readout::Raw, preview: Preview::None };
                };
                let (tag, title) = pad_text(&first.action, &meta, core_ref);
                // フェーダーとチャンネル値は、名前を付けてあればその名前のほうが分かりやすい(「LEDバー 明るさ」「ストロボ」など)
                let named = !first.name.trim().is_empty() && first.name != ctl.short_name();
                let title = match ctl {
                    // フェーダーは「対象 / 何を」の2行に分ける
                    Ctl::Fader(_) if named => match first.name.trim().rsplit_once(' ') {
                        Some((a, b)) => format!("{a}\n{b}"),
                        None => first.name.trim().to_string(),
                    },
                    _ if named && matches!(first.action, Action::SetValue { .. }) => first.name.clone(),
                    _ => title,
                };
                let tip = hits.iter().map(|b| format!("{}: {}", b.name, summary(&b.action, &meta).replace('\n', " "))).collect::<Vec<_>>().join("\n");
                Info {
                    tag,
                    title: if hits.len() > 1 { format!("{title} +{}", hits.len() - 1) } else { title },
                    tip: format!("{}\n{tip}", ctl.label()),
                    count: hits.len(),
                    color: Some(action_color(&first.action, &core_ref.project)),
                    active: hits.iter().any(|b| crate::led::is_active(core_ref, &b.action)),
                    rgb: if matches!(ctl, Ctl::Pad(_)) { crate::led::action_rgb(core_ref, &first.action, &look_rgb) } else { None },
                    readout: readout(&first.action),
                    preview: match &first.action {
                        Action::Effect { name, .. } if matches!(ctl, Ctl::Pad(_)) => match core_ref.project.pixel_fx.iter().find(|f| f.name == *name) {
                            Some(f) => Preview::Face(core_ref.face_preview(f, 8, 30)),
                            None => effect_path(core_ref, name).unwrap_or(Preview::None),
                        },
                        _ => Preview::None,
                    },
                }
            };

            // 図の大きさを、残りの場所に合わせて決める(パッドは横長、フェーダーはパッド2段ぶん)
            let area = ui.available_rect_before_wrap();
            let gap = 6.0;
            let side_k = 0.72;
            let (track_k, fader_k) = (0.42, 2.9);
            let pad_h = ((area.height() - 9.0 * gap - 8.0) / (8.0 + track_k + fader_k)).clamp(30.0, 86.0);
            let pad_w = ((area.width() - 8.0 * gap - 8.0) / (8.0 + side_k)).clamp(pad_h, pad_h * 1.8);
            let side_w = (pad_w * side_k).max(54.0);
            let track_h = (pad_h * track_k).max(22.0);
            let fader_h = pad_h * fader_k;
            let side_x = 8.0 * (pad_w + gap) + 2.0;
            let track_y = 8.0 * (pad_h + gap) + 2.0;
            let fader_y = track_y + track_h + gap + 2.0;
            let total = vec2(side_x + side_w, fader_y + fader_h);

            egui::ScrollArea::both().auto_shrink([false; 2]).show(ui, |ui| {
                let (area, _) = ui.allocate_exact_size(total, Sense::hover());
                let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(area.min + vec2(x, y), vec2(w, h));
                let down = |ctl: Ctl| rt.notes_down[ctl.trigger().1 as usize];
                let sel = self.map_sel;

                // パッド(最上段が0x38..0x3F)
                for row in 0..8u8 {
                    for col in 0..8u8 {
                        let ctl = Ctl::Pad(pad_note(row, col));
                        let r = at(col as f32 * (pad_w + gap), row as f32 * (pad_h + gap), pad_w, pad_h);
                        if button_cell(ui, r, ctl, &info(ctl), down(ctl), sel == Some(ctl)) {
                            clicked = Some(ctl);
                        }
                    }
                }
                // シーンボタン(上から1..8)
                for i in 0..8u8 {
                    let ctl = Ctl::Scene(i);
                    let r = at(side_x, i as f32 * (pad_h + gap), side_w, pad_h);
                    if button_cell(ui, r, ctl, &info(ctl), down(ctl), sel == Some(ctl)) {
                        clicked = Some(ctl);
                    }
                }
                // トラックボタン(左から1..8)とSHIFT
                for i in 0..8u8 {
                    let ctl = Ctl::Track(i);
                    let r = at(i as f32 * (pad_w + gap), track_y, pad_w, track_h);
                    if button_cell(ui, r, ctl, &info(ctl), down(ctl), sel == Some(ctl)) {
                        clicked = Some(ctl);
                    }
                }
                let r = at(side_x, track_y, side_w, track_h);
                if button_cell(ui, r, Ctl::Shift, &info(Ctl::Shift), down(Ctl::Shift), sel == Some(Ctl::Shift)) {
                    clicked = Some(Ctl::Shift);
                }
                // フェーダー(左から1..8、9がマスター)
                for i in 0..9u8 {
                    let ctl = Ctl::Fader(i);
                    let (x, w) = if i < 8 { (i as f32 * (pad_w + gap), pad_w) } else { (side_x, side_w) };
                    let r = at(x, fader_y, w, fader_h);
                    if fader_cell(ui, r, ctl, &info(ctl), rt.cc_value[ctl.trigger().1 as usize], sel == Some(ctl)) {
                        clicked = Some(ctl);
                    }
                }
            });
        }

        // ---- 右: ステージ / 割り当ての編集 ----
        let mut rui = ui.new_child(UiBuilder::new().max_rect(right).layout(Layout::top_down(Align::Min)));
        {
            let ui = &mut rui;
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.menu_button("配置 ▼", |ui| {
                        ui.set_min_width(300.0);
                        ui.label(RichText::new("おすすめ配置").font(theme::bold(14.0)));
                        ui.label(
                            RichText::new("上の4段=LEDバーの模様(チル/グルーヴ/ピーク/モノクロ)、5段目=ムービングなど、6・7段目=メイン色・サブ色、最下段=フラッシュなど。右の列=TAP・入れ替え・BLACKOUT、下の列=LEDバー1〜8のミュート、フェーダー=明るさ・速さ・斜め・色相")
                                .small()
                                .color(theme::TEXT_DIM),
                        );
                        if ui.button(RichText::new("おすすめ配置にする(今の割り当てを置き換え)").color(theme::WARN)).on_hover_text("置き換える前の状態はバックアップに残します(ファイル → バックアップから戻す)").clicked() {
                            menu.recommended = true;
                            ui.close();
                        }
                        ui.separator();
                        ui.label(RichText::new("空いているところに足す").small().color(theme::TEXT_DIM));
                        if ui.button("ページ切替(右の列)").clicked() {
                            menu.page = true;
                            ui.close();
                        }
                        if ui.button("色の操作(最下段の色・全体/個別・機材の選択)").clicked() {
                            menu.color = true;
                            ui.close();
                        }
                        if ui.button("メイン/サブ色").clicked() {
                            menu.palette = true;
                            ui.close();
                        }
                        if ui.button("LEDバーの模様(1本用・最上段)").clicked() {
                            menu.pixel = true;
                            ui.close();
                        }
                        ui.separator();
                        if ui.add_enabled(self.midi.can_request_faders(), egui::Button::new("フェーダー位置を読み取る")).clicked() {
                            menu.read_faders = true;
                            ui.close();
                        }
                        if use_pages && common_pads > 0 {
                            if ui.button(format!("共通のパッド {common_pads} 件をページ{}専用にする", page + 1)).on_hover_text("「全ページ共通」になっているパッドの割り当てを、表示中のページだけで効くようにします(ボタンとフェーダーは共通のまま)").clicked() {
                                menu.move_common = true;
                                ui.close();
                            }
                        }
                    });
                    legend(ui);
                });
            });
            egui::ScrollArea::vertical().id_salt("map_panel").auto_shrink([false; 2]).show(ui, |ui| match self.map_sel {
                None => stage_panel(ui, core),
                Some(ctl) => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(ctl.label()).font(theme::bold(14.0)).color(theme::TEXT));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("×").on_hover_text("閉じる(ステージの表示に戻る)").clicked() {
                                close_editor = true;
                            }
                        });
                    });
                    let (kind, num) = ctl.trigger();
                    let live = match kind {
                        TriggerKind::Cc => match core.rt.cc_value[num as usize] {
                            Some(v) => format!("現在の値: {v} / 127"),
                            None => "現在の値: (まだ動かしていません)".to_string(),
                        },
                        TriggerKind::Note => if core.rt.notes_down[num as usize] { "押されています" } else { "離されています" }.to_string(),
                    };
                    ui.label(RichText::new(live).small().color(theme::TEXT_DIM));
                    ui.add_space(4.0);

                    // 今のページで効く割り当て(共通 + このページ)だけを編集対象にする
                    let mine = |b: &Binding| b.trigger.is_some_and(|t| ctl.matches(&t));
                    let idxs: Vec<usize> = core.project.bindings.iter().enumerate().filter(|(_, b)| mine(b) && b.on_page(page)).map(|(i, _)| i).collect();
                    let elsewhere = core.project.bindings.iter().filter(|b| mine(b) && !b.on_page(page)).count();
                    if use_pages {
                        ui.label(RichText::new(format!("ページ{} を表示中", page + 1)).small().color(theme::TEXT_DIM));
                    }
                    if idxs.is_empty() {
                        ui.label(RichText::new("まだ割り当てがありません。").color(theme::TEXT_DIM));
                    }
                    for i in idxs {
                        let learn = Learn { midi: &mut core.rt.learn, osc: &mut core.rt.osc_learn };
                        let out = binding_editor(ui, i, &mut core.project.bindings[i], learn, &cx);
                        changed |= out.changed;
                        if out.remove {
                            remove = Some(i);
                        }
                    }
                    if elsewhere > 0 {
                        ui.label(RichText::new(format!("ほかのページにも {elsewhere} 件の割り当てがあります(ページ番号で切り替え)")).small().color(theme::TEXT_DIM));
                    }
                    ui.add_space(4.0);
                    if ui.button("＋ このコントロールに割り当てを追加").clicked() {
                        // ページを使っているときは、パッドの新しい割り当ては表示中のページ専用にする
                        let pg = (use_pages && matches!(ctl, Ctl::Pad(_))).then_some(page);
                        core.project.bindings.push(Binding {
                            name: ctl.short_name(),
                            trigger: Some(MidiTrigger { kind, channel: None, number: num }),
                            action: default_action(matches!(ctl, Ctl::Fader(_))),
                            page: pg,
                            osc: None,
                        });
                        changed = true;
                    }
                }
            });
        }

        if let Some(c) = clicked {
            // もう一度クリックすると閉じる
            self.map_sel = if self.map_sel == Some(c) { None } else { Some(c) };
        }
        if close_editor {
            self.map_sel = None;
        }
        if let Some(p) = new_page {
            core.rt.page = p;
        }
        if menu.page {
            let (added, skipped) = add_page_template(&mut core.project);
            self.notice = if skipped > 0 { format!("ページ切替を {added} 個配置しました({skipped} 個は使用中のため飛ばしました)") } else { format!("ページ切替を {added} 個配置しました") };
            changed = true;
        }
        if menu.move_common {
            for b in core.project.bindings.iter_mut() {
                if b.page.is_none() && b.trigger.is_some_and(|t| t.kind == TriggerKind::Note && t.number < 64 && t.channel.is_none_or(|c| c == 0)) {
                    b.page = Some(page);
                }
            }
            changed = true;
        }
        if menu.pixel {
            let (added, skipped) = crate::pixel::add_pad_template(&mut core.project, use_pages.then_some(page));
            self.notice = if skipped > 0 { format!("LEDバーの模様を {added} 個配置しました({skipped} 個は使用中のため飛ばしました)") } else { format!("LEDバーの模様を {added} 個配置しました(エフェクトタブで速さや幅を変えられます)") };
            changed = true;
        }
        if menu.palette {
            let (added, skipped) = crate::color::add_palette_template(&mut core.project);
            self.notice = if skipped > 0 { format!("メイン/サブ色の操作を {added} 個配置しました({skipped} 個は使用中のため飛ばしました)") } else { format!("メイン/サブ色の操作を {added} 個配置しました") };
            changed = true;
        }
        if menu.color {
            let (added, skipped) = crate::color::add_template(&mut core.project);
            self.notice = if skipped > 0 { format!("色の操作を {added} 個配置しました({skipped} 個は使用中のため飛ばしました)") } else { format!("色の操作を {added} 個配置しました") };
            changed = true;
        }
        if let Some(i) = remove {
            core.project.bindings.remove(i);
            core.rt.pad_state.clear();
            core.rt.toggle_state.clear();
            core.rt.learn = None;
            changed = true;
        }
        if changed {
            core.dirty = true;
        }
        drop(guard);

        if menu.read_faders {
            if let Err(e) = self.midi.request_faders() {
                self.notice = format!("フェーダー位置の読み取りに失敗: {e}");
            }
        }
        if menu.recommended {
            self.apply_recommended_layout();
        }
    }

    /// おすすめ配置にする。置き換える前の状態は保存してバックアップに残す
    pub fn apply_recommended_layout(&mut self) {
        self.save();
        let backup = backup_to(&self.project_path, &backup_dir(), BACKUP_KEEP);
        let mut c = lock(&self.core);
        let lib = c.lib.clone();
        let r = crate::layout::apply(&mut c.project, &lib);
        c.rt.pad_state.clear();
        c.rt.toggle_state.clear();
        c.rt.learn = None;
        c.dirty = true;
        drop(c);
        self.map_sel = None;
        self.notice = match backup {
            Some(_) => format!("{}。元の配置は「ファイル → バックアップから戻す」で戻せます", r.text()),
            None => r.text(),
        };
    }
}

fn rgb32(c: [f32; 3], level: f32) -> Color32 {
    let f = |x: f32| (x.clamp(0.0, 1.0) * level.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(f(c[0]), f(c[1]), f(c[2]))
}

/// 種類と色の凡例(マウスを乗せると出る)
fn legend(ui: &mut egui::Ui) {
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
fn button_cell(ui: &mut egui::Ui, rect: Rect, ctl: Ctl, info: &Info, down: bool, selected: bool) -> bool {
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
    if !info.title.contains(' ') && info.title.is_ascii() && one_line > inner_w {
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

fn fader_cell(ui: &mut egui::Ui, rect: Rect, ctl: Ctl, info: &Info, value: Option<u8>, selected: bool) -> bool {
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

/// 右のパネル(何も選んでいないとき): LEDバーの今の模様、メイン/サブ色、機材ごとの色
fn stage_panel(ui: &mut egui::Ui, core: &mut Core) {
    use crate::look::Look;
    tempo_card(ui, core);
    ui.add_space(6.0);
    let core: &Core = core;
    let looks = crate::look::looks(core);
    let bars: Vec<&crate::look::FixtureLook> = looks.iter().filter(|l| matches!(l.look, Look::Strip(_))).collect();

    if !bars.is_empty() {
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            // 8本 × ピクセル の面のプレビュー(実際の出力の色)
            let w = ui.available_width();
            let (row_h, row_gap) = (9.0, 4.0);
            let (rect, _) = ui.allocate_exact_size(vec2(w, bars.len() as f32 * (row_h + row_gap) + 10.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(rect, 8.0, Color32::from_rgb(0x07, 0x08, 0x0B));
            for (b, l) in bars.iter().enumerate() {
                let Look::Strip(px) = &l.look else { continue };
                let n = px.len().max(1);
                let y = rect.min.y + 5.0 + b as f32 * (row_h + row_gap);
                let x0 = rect.min.x + 8.0;
                let cw = (w - 16.0) / n as f32;
                let mut mesh = egui::Mesh::default();
                for (k, c) in px.iter().enumerate() {
                    mesh.add_colored_rect(Rect::from_min_size(pos2(x0 + k as f32 * cw, y), vec2(cw.max(1.0) + 0.3, row_h)), rgb32(*c, 1.0));
                }
                p.add(egui::Shape::mesh(mesh));
                if l.note == Some("MUTE") {
                    p.text(pos2(rect.max.x - 10.0, y + row_h / 2.0), Align2::RIGHT_CENTER, "MUTE", FontId::proportional(9.0), theme::DANGER);
                }
            }
            // 動いている模様
            let on: Vec<&crate::pixel::PixelFx> = core.project.pixel_fx.iter().filter(|f| f.enabled).collect();
            ui.horizontal_wrapped(|ui| {
                for f in on {
                    let c = energy_color(Some(f.energy()));
                    egui::Frame::new().fill(alpha(c, 0.16)).stroke(Stroke::new(1.0, alpha(c, 0.5))).corner_radius(10).inner_margin(egui::Margin::symmetric(8, 2)).show(ui, |ui| {
                        ui.label(RichText::new(format!("▶ {}", f.name)).small().color(mix(c, Color32::WHITE, 0.3)));
                    });
                }
            });
        });
        ui.add_space(6.0);
    }

    // メイン/サブ色: 色見本と、色相フェーダーと同じ並びの色相の帯(目印=フェーダーの位置)
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        for (label, role, target) in [("メイン", crate::color::ColorRole::Main, crate::color::ColorTarget::Main), ("サブ", crate::color::ColorRole::Sub, crate::color::ColorTarget::Sub)] {
            let c = core.role_rgb(role).unwrap_or([0.0; 3]);
            // この色の色相フェーダー(割り当てがあれば、その実機の位置)
            let fader = core.project.bindings.iter().find_map(|b| match (&b.action, b.trigger) {
                (Action::Color { color: None, target: t }, Some(tr)) if *t == target && tr.kind == TriggerKind::Cc => Some(tr.number),
                _ => None,
            });
            let pos = fader.and_then(|n| core.rt.cc_value[n as usize]).map(|v| v as f32 / 127.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(26.0, 18.0), Sense::hover());
                ui.painter().rect_filled(r, 4.0, rgb32(c, 1.0));
                ui.painter().rect_stroke(r, 4.0, Stroke::new(1.0, alpha(Color32::WHITE, 0.15)), StrokeKind::Inside);
                let w = ui.available_width() - 4.0;
                let (bar, resp) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
                hue_bar(ui.painter(), bar);
                // 目印: フェーダーの位置(まだ動かしていなければ、今の色の色相)
                let (t, known) = match pos {
                    Some(t) => (t, true),
                    None => (hue_of(c) / 360.0, false),
                };
                let x = bar.min.x + bar.width() * t.clamp(0.0, 1.0);
                let mark = Rect::from_center_size(pos2(x, bar.center().y), vec2(6.0, bar.height() + 6.0));
                ui.painter().rect_filled(mark, 1.5, if known { Color32::WHITE } else { alpha(Color32::WHITE, 0.55) });
                ui.painter().rect_stroke(mark, 1.5, Stroke::new(1.0, Color32::BLACK), StrokeKind::Outside);
                let tip = match (fader, pos) {
                    (Some(n), Some(t)) => format!("{label}の色相フェーダー(CC {n}): {:.0}°", t * 360.0),
                    (Some(n), None) => format!("{label}の色相フェーダー(CC {n})はまだ動かしていません。目印は今の色の色相"),
                    (None, _) => format!("{label}の色相フェーダーは割り当てがありません。目印は今の色の色相"),
                };
                resp.on_hover_text(tip);
            });
        }
    });
    ui.add_space(6.0);

    // 機材ごとの今の色
    let others: Vec<&crate::look::FixtureLook> = looks.iter().filter(|l| !matches!(l.look, Look::Strip(_))).collect();
    if !others.is_empty() {
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let col_w = (ui.available_width() - 8.0) / 2.0;
            egui::Grid::new("stage_fx").num_columns(2).min_col_width(col_w).max_col_width(col_w).spacing([8.0, 5.0]).show(ui, |ui| {
                for (k, l) in others.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.set_max_width(col_w);
                        let (r, resp) = ui.allocate_exact_size(vec2(22.0, 16.0), Sense::hover());
                        let p = ui.painter();
                        let tip = match &l.look {
                            Look::One { rgb, level, label } => {
                                let fill = match rgb {
                                    Some(c) => rgb32(*c, *level),
                                    None => Color32::from_gray((30.0 + 70.0 * level) as u8),
                                };
                                p.rect_filled(r, 4.0, fill);
                                if *level <= 0.001 {
                                    p.line_segment([r.left_bottom(), r.right_top()], Stroke::new(1.2, theme::TEXT_FAINT));
                                }
                                format!("{}: {label} / 明るさ {:.0}%", l.name, level * 100.0)
                            }
                            Look::Strip(_) => String::new(),
                        };
                        p.rect_stroke(r, 4.0, Stroke::new(1.0, alpha(Color32::WHITE, 0.12)), StrokeKind::Inside);
                        resp.on_hover_text(tip);
                        if let Some(n) = l.note {
                            // 名前より先に置いて、狭くても状態(SAFEなど)が見えるようにする
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                ui.label(RichText::new(n).small().color(theme::WARN));
                                ui.add(egui::Label::new(RichText::new(&l.name).small().color(theme::TEXT_DIM)).truncate());
                            });
                        } else {
                            ui.add(egui::Label::new(RichText::new(&l.name).small().color(theme::TEXT_DIM)).truncate());
                        }
                    });
                    if k % 2 == 1 {
                        ui.end_row();
                    }
                }
            });
        });
        ui.add_space(6.0);
    }

    let stray = core
        .project
        .bindings
        .iter()
        // OSCだけで動かす割り当ては、図に載らなくて正常
        .filter(|b| !(b.trigger.is_none() && b.osc.is_some()))
        .filter(|b| !b.trigger.is_some_and(|t| Ctl::from_trigger(t.kind, t.number).is_some_and(|c| c.matches(&t))))
        .count();
    if stray > 0 {
        ui.label(RichText::new(format!("この図に載らない割り当てが {stray} 件あります(未設定、または別のMIDI機器/チャンネル)。「MIDI設定」タブで確認できます。")).small().color(theme::WARN));
    }
}

/// テンポ: BPM・拍の位置・TAP・頭出し
fn tempo_card(ui: &mut egui::Ui, core: &mut Core) {
    let now = std::time::Instant::now();
    let (src, peers, clock_ok) = (core.project.tempo_source, core.rt.link_peers, core.clock_alive(now));
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        // 1行目: 見出し・同期の状態・TAP・頭出し
        ui.horizontal(|ui| {
            ui.label(RichText::new("BPM").small().color(theme::TEXT_FAINT));
            match src {
                TempoSource::Link => {
                    ui.label(RichText::new(format!("Link {peers}")).small().strong().color(if peers > 0 { theme::OK } else { theme::TEXT_DIM })).on_hover_text(format!("Ableton Link: 接続中のアプリ {peers} 個"));
                }
                TempoSource::MidiClock => {
                    ui.label(RichText::new("CLK").small().strong().color(if clock_ok { theme::OK } else { theme::WARN })).on_hover_text(if clock_ok { "MIDIクロックを受信中" } else { "MIDIクロックが来ていません(内部のテンポで走っています)" });
                }
                TempoSource::Internal => {}
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("頭出し").on_hover_text("拍の位置を小節の頭に戻します(R)").clicked() {
                    core.resync();
                }
                if ui.add(egui::Button::new(RichText::new("TAP").font(theme::bold(13.0))).min_size(vec2(56.0, 0.0))).on_hover_text("リズムに合わせて叩くとBPMが決まり、拍の位置も合います(Space)").clicked() {
                    core.tap();
                }
            });
        });
        // 2行目: BPMの数字と、拍の位置(四角は数字の欄と同じ大きさ。1拍目はアンバー)
        ui.horizontal(|ui| {
            // MIDIクロック中は、BPMはクロックで決まる
            let by_clock = src == TempoSource::MidiClock && clock_ok;
            let mut bpm = core.project.bpm;
            let resp = ui
                .scope(|ui| {
                    ui.style_mut().override_font_id = Some(theme::bold(24.0));
                    let drag = egui::DragValue::new(&mut bpm).range(BPM_MIN..=BPM_MAX).speed(0.2).fixed_decimals(1).update_while_editing(false);
                    ui.add_enabled(!by_clock, drag)
                })
                .inner;
            let box_rect = resp.rect;
            if resp.on_hover_text("ドラッグかダブルクリックで変更").changed() {
                core.set_bpm(bpm);
            }
            ui.add_space(4.0);
            let side = box_rect.height();
            let gap = (side * 0.2).max(4.0);
            let in_bar = (core.rt.beat.floor() as i64).rem_euclid(BEATS_PER_BAR);
            let (rect, _) = ui.allocate_exact_size(vec2(BEATS_PER_BAR as f32 * (side + gap), side), Sense::hover());
            for b in 0..BEATS_PER_BAR {
                let r = Rect::from_min_size(pos2(rect.min.x + b as f32 * (side + gap), rect.min.y), vec2(side, side));
                let color = match (b == in_bar, b) {
                    (true, 0) => theme::WARN,
                    (true, _) => theme::ACCENT,
                    _ => theme::WIDGET_ACTIVE,
                };
                ui.painter().rect_filled(r, 4.0, color);
            }
        });
    });
}

/// 色相の帯(左=0° 赤 → 右=360° 赤。色相フェーダーの下端→上端と同じ)
fn hue_bar(p: &egui::Painter, rect: Rect) {
    let n = 72;
    let w = rect.width() / n as f32;
    for k in 0..n {
        let [r, g, b] = crate::color::hsv((k as f32 + 0.5) / n as f32 * 360.0, 1.0, 1.0);
        let seg = Rect::from_min_size(pos2(rect.min.x + k as f32 * w, rect.min.y), vec2(w + 0.6, rect.height()));
        p.rect_filled(seg, 0.0, Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8));
    }
    p.rect_stroke(rect, 2.0, Stroke::new(1.0, alpha(Color32::WHITE, 0.12)), StrokeKind::Inside);
}

/// RGB(0〜1)の色相(0〜360度)
fn hue_of(c: [f32; 3]) -> f32 {
    let (max, min) = (c[0].max(c[1]).max(c[2]), c[0].min(c[1]).min(c[2]));
    let d = max - min;
    if d <= 1e-6 {
        return 0.0;
    }
    let h = if max == c[0] {
        60.0 * ((c[1] - c[2]) / d).rem_euclid(6.0)
    } else if max == c[1] {
        60.0 * ((c[2] - c[0]) / d + 2.0)
    } else {
        60.0 * ((c[0] - c[1]) / d + 4.0)
    };
    h.rem_euclid(360.0)
}

#[cfg(test)]
mod tests {
    use super::hue_of;

    #[test]
    fn hue_matches_the_hue_fader() {
        for h in [0.0, 30.0, 120.0, 200.0, 300.0] {
            assert!((hue_of(crate::color::hsv(h, 1.0, 1.0)) - h).abs() < 0.5, "{h}");
        }
        assert_eq!(hue_of([1.0, 1.0, 1.0]), 0.0);
    }
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
