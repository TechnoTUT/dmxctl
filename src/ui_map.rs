//! マップタブ: APC mini mk2 の配置図。割り当ての確認・編集と、押した/動かした状態の表示。

use crate::apcmini::{pad_note, Ctl};
use crate::app::{App, SelMeta};
use crate::engine::lock;
use crate::project::*;
use crate::ui_binding::{binding_editor, default_action, names, summary, EditCtx, Learn};
use eframe::egui::{self, vec2, Align, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, StrokeKind};
use std::collections::HashSet;

const PAD_W: f32 = 78.0;
const PAD_H: f32 = 50.0;
const GAP: f32 = 4.0;
const SIDE_W: f32 = 58.0;
const TRACK_H: f32 = 34.0;
const FADER_H: f32 = 118.0;

const COL_SET: Color32 = Color32::from_rgb(62, 95, 165);
const COL_SCENE: Color32 = Color32::from_rgb(46, 125, 80);
const COL_FX: Color32 = Color32::from_rgb(125, 80, 165);
const COL_BLACKOUT: Color32 = Color32::from_rgb(170, 50, 50);
const COL_MASTER: Color32 = Color32::from_rgb(190, 135, 35);
const COL_TEMPO: Color32 = Color32::from_rgb(40, 140, 150);
const COL_CHASE: Color32 = Color32::from_rgb(120, 130, 45);
const COL_PAIR: Color32 = Color32::from_rgb(165, 85, 125);
const COL_COLOR: Color32 = Color32::from_rgb(45, 140, 105);
const COL_SELECT_FX: Color32 = Color32::from_rgb(95, 105, 60);
const COL_POSITION: Color32 = Color32::from_rgb(55, 125, 190);
const COL_PAGE: Color32 = Color32::from_rgb(105, 105, 120);
const COL_FLASH: Color32 = Color32::from_rgb(150, 150, 95);
const COL_EMPTY: Color32 = Color32::from_rgb(48, 48, 54);
const COL_SELECT: Color32 = Color32::from_rgb(255, 210, 60);

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

fn action_color(a: &Action) -> Color32 {
    match a {
        Action::SetValue { .. } => COL_SET,
        Action::Scene { .. } => COL_SCENE,
        Action::Effect { .. } => COL_FX,
        Action::Blackout { .. } => COL_BLACKOUT,
        Action::Master { .. } => COL_MASTER,
        Action::Tempo { .. } => COL_TEMPO,
        Action::Chase { .. } => COL_CHASE,
        Action::Pair { .. } => COL_PAIR,
        Action::Color { .. } => COL_COLOR,
        Action::ColorScope { .. } | Action::Select { .. } => COL_SELECT_FX,
        Action::Position { .. } => COL_POSITION,
        Action::Page { .. } => COL_PAGE,
        Action::Palette { .. } => COL_COLOR,
        Action::Flash { .. } => COL_FLASH,
        Action::Fade { .. } => COL_BLACKOUT,
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

/// 図に描く1コントロール分の情報
struct Info {
    text: String,
    tip: String,
    count: usize,
    color: Option<Color32>,
    /// エフェクトON中・トグルON中・ブラックアウト中など「効いている」状態
    active: bool,
    /// 色相(HSVのH)のフェーダーか
    hue: bool,
    /// 実際の色(色を指定するパッドなど)
    rgb: Option<[f32; 3]>,
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
        let mut color_template = false;
        let mut palette_template = false;
        let mut pixel_template = false;
        let mut page_template = false;
        let mut move_common: Option<u8> = None;
        let mut new_page: Option<u8> = None;
        let mut remove: Option<usize> = None;
        let page = core.rt.page;
        let use_pages = pages_in_use(&core.project);
        // 今の出力の色(全体光量・ブラックアウト・エフェクト込み)と、メイン/サブ色
        let looks = crate::look::looks(core);
        let look_rgb = crate::led::look_colors(core);
        let palette_now = (core.role_rgb(crate::color::ColorRole::Main).unwrap_or([1.0; 3]), core.role_rgb(crate::color::ColorRole::Sub).unwrap_or([0.0; 3]), core.palette_swapped());

        // 左にマップ、右に編集パネル。右から順に置いて、パネルの幅を固定する
        let full = ui.available_size();
        let map_w = 8.0 * (PAD_W + GAP) + 10.0 + SIDE_W + 16.0;
        let editor_w = (full.x - map_w - 16.0).max(380.0);
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        ui.allocate_ui_with_layout(vec2(editor_w, full.y), Layout::top_down(Align::Min), |ui| {
            ui.set_width(editor_w);
            egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| match self.map_sel {
                None => {
                    ui.add_space(6.0);
                    ui.label("図のパッド・ボタン・フェーダーをクリックすると、ここで割り当てを編集できます。");
                    ui.label(RichText::new("空のパッドに出ている数字は、そのパッドのNote番号です。").weak());
                    let stray = core
                        .project
                        .bindings
                        .iter()
                        // OSCだけで動かす割り当ては、図に載らなくて正常
                        .filter(|b| !(b.trigger.is_none() && b.osc.is_some()))
                        .filter(|b| !b.trigger.is_some_and(|t| Ctl::from_trigger(t.kind, t.number).is_some_and(|c| c.matches(&t))))
                        .count();
                    if stray > 0 {
                        ui.add_space(8.0);
                        ui.colored_label(crate::app::WARN, format!("この図に載らない割り当てが {stray} 件あります(未設定、または別のMIDI機器/チャンネル)。MIDI/OSCタブで確認できます。"));
                    }
                }
                Some(ctl) => {
                    ui.add_space(6.0);
                    ui.label(RichText::new(ctl.label()).strong().size(15.0));
                    let (kind, num) = ctl.trigger();
                    let live = match kind {
                        TriggerKind::Cc => match core.rt.cc_value[num as usize] {
                            Some(v) => format!("現在の値: {v} / 127"),
                            None => "現在の値: (まだ動かしていません)".to_string(),
                        },
                        TriggerKind::Note => if core.rt.notes_down[num as usize] { "押されています" } else { "離されています" }.to_string(),
                    };
                    ui.label(RichText::new(live).weak());
                    ui.separator();

                    // 今のページで効く割り当て(共通 + このページ)だけを編集対象にする
                    let mine = |b: &Binding| b.trigger.is_some_and(|t| ctl.matches(&t));
                    let idxs: Vec<usize> = core.project.bindings.iter().enumerate().filter(|(_, b)| mine(b) && b.on_page(page)).map(|(i, _)| i).collect();
                    let elsewhere = core.project.bindings.iter().filter(|b| mine(b) && !b.on_page(page)).count();
                    if use_pages {
                        ui.label(RichText::new(format!("ページ{} を表示中", page + 1)).weak());
                    }
                    if idxs.is_empty() {
                        ui.label("まだ割り当てがありません。");
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
                        ui.label(RichText::new(format!("ほかのページにも {elsewhere} 件の割り当てがあります(上のページ番号で切り替え)")).weak());
                    }
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
        });

        ui.separator();
        ui.allocate_ui_with_layout(vec2(map_w, full.y), Layout::top_down(Align::Min), |ui| {
            let fx_on: HashSet<&str> = core.project.effects.iter().filter(|e| e.enabled).map(|e| e.name.as_str()).chain(core.project.pixel_fx.iter().filter(|e| e.enabled).map(|e| e.name.as_str())).collect();
            let pairs = &core.project.pairs;
            let chase_on: HashSet<&str> = core.project.chases.iter().filter(|c| c.enabled).map(|c| c.name.as_str()).collect();
            let bindings = &core.project.bindings;
            let rt = &core.rt;

            let positions_on = rt.last_position.as_deref();
            let palette = &core.project.palette;
            let core_ref: &crate::engine::Core = core;
            let info = |ctl: Ctl| -> Info {
                let hits: Vec<(usize, &Binding)> =
                    bindings.iter().enumerate().filter(|(_, b)| b.on_page(page) && b.trigger.is_some_and(|t| ctl.matches(&t))).collect();
                let Some((_, first)) = hits.first() else {
                    return Info { text: String::new(), tip: format!("{}\n割り当てなし", ctl.label()), count: 0, color: None, active: false, hue: false, rgb: None };
                };
                let active = hits.iter().any(|(i, b)| match &b.action {
                    Action::Effect { name, .. } => fx_on.contains(name.as_str()),
                    Action::Chase { name, .. } => chase_on.contains(name.as_str()),
                    Action::Color { color: Some(c), target } => match target.slot() {
                        Some(crate::color::ColorRole::Sub) => crate::color::same_rgb(palette.sub, c.rgb()),
                        Some(_) => crate::color::same_rgb(palette.main, c.rgb()),
                        None => rt.last_color == Some(*c),
                    },
                    Action::Palette { op: crate::color::PaletteOp::Auto } => palette.auto,
                    Action::Palette { .. } => rt.color_swapped,
                    Action::Flash { .. } => rt.flash.is_some(),
                    Action::Fade { .. } => rt.dim_target < 0.5,
                    Action::ColorScope { .. } => !rt.color_global,
                    Action::Select { fixture, .. } => rt.selected.contains(fixture),
                    Action::Pair { name, op, .. } => pairs.iter().any(|p| p.name == *name && if *op == PairOp::Link { p.enabled } else { p.mirror }),
                    Action::Blackout { .. } => rt.blackout,
                    Action::SetValue { mode: PadMode::Toggle, .. } => rt.toggle_state.get(i).copied().unwrap_or(false),
                    Action::Position { name } => positions_on == Some(name.as_str()),
                    Action::Page { op: PageOp::Go(p) } => *p == page,
                    _ => false,
                });
                let tip = hits.iter().map(|(_, b)| format!("{}: {}", b.name, summary(&b.action, &meta).replace('\n', " "))).collect::<Vec<_>>().join("\n");
                Info {
                    text: summary(&first.action, &meta),
                    tip: format!("{}\n{tip}", ctl.label()),
                    count: hits.len(),
                    color: Some(action_color(&first.action)),
                    active,
                    hue: matches!(first.action, Action::Color { color: None, .. }),
                    rgb: if matches!(ctl, Ctl::Pad(_)) { crate::led::action_rgb(core_ref, &first.action, &look_rgb) } else { None },
                }
            };

            // ページ(クリックで実際のページも切り替わる)
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("ページ").strong());
                for p in 0..PAGES {
                    let used = bindings.iter().any(|b| b.page == Some(p));
                    let text = if used { RichText::new(format!(" {} ", p + 1)).strong() } else { RichText::new(format!(" {} ", p + 1)) };
                    if ui.selectable_label(p == page, text).on_hover_text("このページに切り替えます(コントローラーのパッドもこのページになります)").clicked() {
                        new_page = Some(p);
                    }
                }
                if ui.button("ページ切替を配置").on_hover_text("右の列のシーンボタン1〜8を「ページ1〜8へ」にします(使用中のボタンは飛ばします)").clicked() {
                    page_template = true;
                }
                let common_pads = bindings.iter().filter(|b| b.page.is_none() && b.trigger.is_some_and(|t| t.kind == TriggerKind::Note && t.number < 64 && t.channel.is_none_or(|c| c == 0))).count();
                if use_pages && common_pads > 0 {
                    if ui
                        .button(format!("共通のパッド {common_pads} 件をページ{}専用にする", page + 1))
                        .on_hover_text("「全ページ共通」になっているパッドの割り当てを、表示中のページだけで効くようにします(ボタンとフェーダーは共通のまま)")
                        .clicked()
                    {
                        move_common = Some(page);
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("■ チャンネル値").color(COL_SET));
                ui.label(RichText::new("■ シーン").color(COL_SCENE));
                ui.label(RichText::new("■ エフェクト").color(COL_FX));
                ui.label(RichText::new("■ 全体光量").color(COL_MASTER));
                ui.label(RichText::new("■ テンポ").color(COL_TEMPO));
                ui.label(RichText::new("■ チェイス").color(COL_CHASE));
                ui.label(RichText::new("■ ペア").color(COL_PAIR));
                ui.label(RichText::new("■ 色").color(COL_COLOR));
                ui.label(RichText::new("■ 位置").color(COL_POSITION));
                ui.label(RichText::new("■ ページ").color(COL_PAGE));
                ui.label(RichText::new("■ フラッシュ").color(COL_FLASH));
                ui.label(RichText::new("■ ブラックアウト").color(COL_BLACKOUT));
                ui.label(RichText::new("明るい = 押している/効いている   黄色の枠 = 編集中").weak());
                if ui.button("色の操作を配置").on_hover_text("下段のパッド=色 / 右下のシーンボタン=全体⇄個別 / トラックボタン=機材の選択 を、空いているところに割り当てます").clicked() {
                    color_template = true;
                }
                if ui.button("メイン/サブ色を配置").on_hover_text("下から2段目のパッド=メイン色 / 3段目=サブ色 / シーンボタン7=メイン⇄サブ入れ替え / 6=自動入れ替え / フェーダー6・7=メイン・サブの色相 を、空いているところに割り当てます").clicked() {
                    palette_template = true;
                }
                if ui.button("LEDバーの模様を配置").on_hover_text("メイン色/サブ色を混ぜる模様(縞々・ウェーブなど8種)を、パッドの最上段にトグルで並べます。ページを使っているときは表示中のページに置きます").clicked() {
                    pixel_template = true;
                }
                if ui.add_enabled(self.midi.can_request_faders(), egui::Button::new("フェーダー位置を読み取る")).clicked() {
                    if let Err(e) = self.midi.request_faders() {
                        self.notice = format!("フェーダー位置の読み取りに失敗: {e}");
                    }
                }
            });
            look_row(ui, &looks, palette_now);
            ui.add_space(4.0);

            let cell_w = PAD_W + GAP;
            let cell_h = PAD_H + GAP;
            let side_x = 8.0 * cell_w + 10.0;
            let track_y = 8.0 * cell_h + 8.0;
            let fader_y = track_y + TRACK_H + 8.0;
            let total = vec2(side_x + SIDE_W, fader_y + FADER_H);

            egui::ScrollArea::both().auto_shrink([false; 2]).show(ui, |ui| {
                let (area, _) = ui.allocate_exact_size(total, Sense::hover());
                let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(area.min + vec2(x, y), vec2(w, h));
                let mut clicked: Option<Ctl> = None;

                // パッド(最上段が0x38..0x3F)
                for row in 0..8u8 {
                    for col in 0..8u8 {
                        let ctl = Ctl::Pad(pad_note(row, col));
                        let r = at(col as f32 * cell_w, row as f32 * cell_h, PAD_W, PAD_H);
                        if button_cell(ui, r, ctl, &info(ctl), rt.notes_down[ctl.trigger().1 as usize], self.map_sel == Some(ctl)) {
                            clicked = Some(ctl);
                        }
                    }
                }
                // シーンボタン(上から1..8)
                for i in 0..8u8 {
                    let ctl = Ctl::Scene(i);
                    let r = at(side_x, i as f32 * cell_h, SIDE_W, PAD_H);
                    if button_cell(ui, r, ctl, &info(ctl), rt.notes_down[ctl.trigger().1 as usize], self.map_sel == Some(ctl)) {
                        clicked = Some(ctl);
                    }
                }
                // トラックボタン(左から1..8)とSHIFT
                for i in 0..8u8 {
                    let ctl = Ctl::Track(i);
                    let r = at(i as f32 * cell_w, track_y, PAD_W, TRACK_H);
                    if button_cell(ui, r, ctl, &info(ctl), rt.notes_down[ctl.trigger().1 as usize], self.map_sel == Some(ctl)) {
                        clicked = Some(ctl);
                    }
                }
                let shift = Ctl::Shift;
                let r = at(side_x, track_y, SIDE_W, TRACK_H);
                if button_cell(ui, r, shift, &info(shift), rt.notes_down[shift.trigger().1 as usize], self.map_sel == Some(shift)) {
                    clicked = Some(shift);
                }
                // フェーダー(左から1..8、9がマスター)
                for i in 0..9u8 {
                    let ctl = Ctl::Fader(i);
                    let (x, w) = if i < 8 { (i as f32 * cell_w, PAD_W) } else { (side_x, SIDE_W) };
                    let r = at(x, fader_y, w, FADER_H);
                    let v = rt.cc_value[ctl.trigger().1 as usize];
                    let inf = info(ctl);
                    if fader_cell(ui, r, ctl, &inf, v, self.map_sel == Some(ctl)) {
                        clicked = Some(ctl);
                    }
                }
                if let Some(c) = clicked {
                    self.map_sel = Some(c);
                }
            });
        });
        });

        if let Some(p) = new_page {
            core.rt.page = p;
        }
        if page_template {
            let (added, skipped) = add_page_template(&mut core.project);
            self.notice = if skipped > 0 { format!("ページ切替を {added} 個配置しました({skipped} 個は使用中のため飛ばしました)") } else { format!("ページ切替を {added} 個配置しました") };
            changed = true;
        }
        if let Some(p) = move_common {
            for b in core.project.bindings.iter_mut() {
                if b.page.is_none() && b.trigger.is_some_and(|t| t.kind == TriggerKind::Note && t.number < 64 && t.channel.is_none_or(|c| c == 0)) {
                    b.page = Some(p);
                }
            }
            changed = true;
        }
        if pixel_template {
            let (added, skipped) = crate::pixel::add_pad_template(&mut core.project, use_pages.then_some(page));
            self.notice = if skipped > 0 { format!("LEDバーの模様を {added} 個配置しました({skipped} 個は使用中のため飛ばしました)") } else { format!("LEDバーの模様を {added} 個配置しました(エフェクトタブで速さや幅を変えられます)") };
            changed = true;
        }
        if palette_template {
            let (added, skipped) = crate::color::add_palette_template(&mut core.project);
            self.notice = if skipped > 0 { format!("メイン/サブ色の操作を {added} 個配置しました({skipped} 個は使用中のため飛ばしました)") } else { format!("メイン/サブ色の操作を {added} 個配置しました") };
            changed = true;
        }
        if color_template {
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
    }
}

fn draw_text(ui: &egui::Ui, rect: Rect, text: &str, color: Color32, size: f32) {
    let p = ui.painter().with_clip_rect(rect);
    let galley = p.layout(text.to_string(), FontId::proportional(size), color, rect.width() - 6.0);
    p.galley(rect.min + vec2(3.0, 2.0), galley, color);
}

fn frame(ui: &egui::Ui, rect: Rect, fill: Color32, selected: bool, pressed: bool) {
    let p = ui.painter();
    p.rect_filled(rect, 4.0, fill);
    if selected {
        p.rect_stroke(rect, 4.0, Stroke::new(2.5, COL_SELECT), StrokeKind::Inside);
    } else if pressed {
        p.rect_stroke(rect, 4.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
    }
}

/// パッド/ボタン1個を描いて、クリックされたらtrueを返す
fn button_cell(ui: &mut egui::Ui, rect: Rect, ctl: Ctl, info: &Info, down: bool, selected: bool) -> bool {
    let resp = ui.interact(rect, ui.id().with(("apc", ctl.trigger().1, ctl.trigger().0 == TriggerKind::Cc)), Sense::click());
    // 色に関係するパッドは実際の色(待機中は暗め、効いている間は明るく)
    let mut fill = match info.rgb {
        Some(c) => rgb32(c, if info.active { 1.0 } else { 0.45 }),
        None => {
            let base = info.color.unwrap_or(COL_EMPTY);
            if info.active { mix(base, Color32::WHITE, 0.35) } else { base }
        }
    };
    if down {
        fill = mix(fill, Color32::WHITE, 0.55);
    }
    if resp.hovered() {
        fill = mix(fill, Color32::WHITE, 0.08);
    }
    frame(ui, rect, fill, selected, down);
    if info.count == 0 {
        let hex = format!("0x{:02X}", ctl.trigger().1);
        let c = if down { Color32::BLACK } else { Color32::from_gray(120) };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, hex, FontId::monospace(11.0), c);
    } else {
        let text = if info.count > 1 { format!("{} (+{})", info.text, info.count - 1) } else { info.text.clone() };
        // 幅の狭いボタン(シーン/SHIFT)は文字を小さくして「BLACKOUT」などが折り返されないようにする
        let size = if rect.width() < 70.0 { 10.0 } else { 11.0 };
        // 文字は背景の明るさに合わせて黒/白
        let lum = 0.3 * fill.r() as f32 + 0.59 * fill.g() as f32 + 0.11 * fill.b() as f32;
        draw_text(ui, rect, &text, if lum > 140.0 { Color32::BLACK } else { Color32::WHITE }, size);
        if info.rgb.is_some() && info.active && !selected {
            // 実際の色のパッドは、効いている目印に白い枠
            ui.painter().rect_stroke(rect, 4.0, Stroke::new(2.0, Color32::WHITE), StrokeKind::Inside);
        }
    }
    resp.on_hover_text(&info.tip).clicked()
}

fn fader_cell(ui: &mut egui::Ui, rect: Rect, ctl: Ctl, info: &Info, value: Option<u8>, selected: bool) -> bool {
    let resp = ui.interact(rect, ui.id().with(("apc", ctl.trigger().1, true)), Sense::click());
    let base = info.color.unwrap_or(COL_EMPTY);
    frame(ui, rect, mix(COL_EMPTY, Color32::BLACK, 0.2), selected, false);
    // 現在位置までを色付きバーで表示し、実機のつまみのように白い線を重ねる
    if let Some(v) = value {
        let top = rect.max.y - 2.0 - (rect.height() - 4.0) * v as f32 / 127.0;
        let bar = Rect::from_min_max(egui::pos2(rect.min.x + 2.0, top), egui::pos2(rect.max.x - 2.0, rect.max.y - 2.0));
        // 色相フェーダーは、その位置の色でバーを塗る
        let bar_color = if info.hue {
            let [r, g, b] = crate::color::hsv(v as f32 / 127.0 * 360.0, 1.0, 1.0);
            Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
        } else if info.color.is_some() {
            mix(base, Color32::WHITE, 0.15)
        } else {
            Color32::from_gray(90)
        };
        ui.painter().rect_filled(bar, 3.0, bar_color);
        let cap_y = top.clamp(rect.min.y + 5.0, rect.max.y - 5.0);
        let cap = Rect::from_center_size(egui::pos2(rect.center().x, cap_y), vec2(rect.width() - 12.0, 8.0));
        ui.painter().rect_filled(cap, 3.0, Color32::from_gray(235));
    }
    if selected {
        ui.painter().rect_stroke(rect, 4.0, Stroke::new(2.5, COL_SELECT), StrokeKind::Inside);
    }
    let title = match ctl {
        Ctl::Fader(8) => "MST".to_string(),
        Ctl::Fader(i) => format!("F{}", i + 1),
        _ => String::new(),
    };
    let label = if info.count == 0 { title } else { format!("{title}\n{}", info.text) };
    draw_text(ui, rect, &label, Color32::WHITE, 11.0);
    // 色相フェーダーは、値の代わりに度数(H)を出す
    let val = value.map(|v| if info.hue { format!("{}°", (v as f32 / 127.0 * 360.0).round() as i32) } else { v.to_string() }).unwrap_or_else(|| "--".into());
    // つまみ(白線)と重ならないよう、数値は右上に出す
    ui.painter().text(egui::pos2(rect.max.x - 5.0, rect.min.y + 9.0), Align2::RIGHT_CENTER, val, FontId::monospace(12.0), Color32::WHITE);
    resp.on_hover_text(&info.tip).clicked()
}

fn rgb32(c: [f32; 3], level: f32) -> Color32 {
    let f = |x: f32| (x.clamp(0.0, 1.0) * level.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(f(c[0]), f(c[1]), f(c[2]))
}

/// 各機材が今出している色(実際の出力値から)。暗いほど明るさ(ディマー・全体光量)が低い
fn look_row(ui: &mut egui::Ui, looks: &[crate::look::FixtureLook], palette: ([f32; 3], [f32; 3], bool)) {
    use crate::look::Look;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("今の色").strong()).on_hover_text("実際に出力している値から表示します(全体光量・ブラックアウト・エフェクト込み)。暗く見えるのは明るさが低いとき");
        for l in looks {
            let resp = ui
                .horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    ui.label(RichText::new(&l.name).small());
                    match &l.look {
                        Look::Strip(px) => {
                            let cell = (90.0 / px.len().max(1) as f32).clamp(3.0, 10.0);
                            let (rect, _) = ui.allocate_exact_size(vec2(cell * px.len() as f32, 18.0), Sense::hover());
                            for (k, c) in px.iter().enumerate() {
                                let r = Rect::from_min_size(rect.min + vec2(k as f32 * cell, 0.0), vec2(cell, 18.0));
                                ui.painter().rect_filled(r, 0.0, rgb32(*c, 1.0));
                            }
                            ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, Color32::from_gray(90)), StrokeKind::Outside);
                        }
                        Look::One { rgb, level, .. } => {
                            let (rect, _) = ui.allocate_exact_size(vec2(26.0, 18.0), Sense::hover());
                            let p = ui.painter();
                            match rgb {
                                Some(c) => {
                                    p.rect_filled(rect, 3.0, rgb32(*c, *level));
                                }
                                None => {
                                    p.rect_filled(rect, 3.0, Color32::from_gray((40.0 + 80.0 * level) as u8));
                                    p.text(rect.center(), Align2::CENTER_CENTER, "?", FontId::proportional(12.0), Color32::WHITE);
                                }
                            }
                            p.rect_stroke(rect, 3.0, Stroke::new(1.0, Color32::from_gray(90)), StrokeKind::Outside);
                            if *level <= 0.001 {
                                // 消えている
                                p.line_segment([rect.left_bottom(), rect.right_top()], Stroke::new(1.5, Color32::from_gray(150)));
                            }
                        }
                    }
                    if let Some(n) = l.note {
                        ui.label(RichText::new(n).small().color(crate::app::WARN));
                    }
                })
                .response;
            let tip = match &l.look {
                Look::One { label, level, .. } => format!("{}: {label} / 明るさ {:.0}%", l.name, level * 100.0),
                Look::Strip(px) => format!("{}: {}ピクセル", l.name, px.len()),
            };
            resp.on_hover_text(tip);
            ui.add_space(6.0);
        }
        ui.separator();
        let (main, sub, swapped) = palette;
        ui.label(RichText::new("メイン").small());
        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        ui.painter().rect_filled(r, 3.0, rgb32(main, 1.0));
        ui.label(RichText::new("サブ").small());
        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
        ui.painter().rect_filled(r, 3.0, rgb32(sub, 1.0));
        if swapped {
            ui.label(RichText::new("⇄入れ替え中").small().color(crate::app::WARN));
        }
    });
}