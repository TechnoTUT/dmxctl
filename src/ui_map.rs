//! MIDIタブ(メイン画面): APC mini mk2 の配置図。割り当ての確認・編集と、押した/動かした状態の表示。
//! 右側は、何も選んでいないときは「ステージ」(LEDバーの今の模様・メイン/サブ色・機材の色)、
//! 図のコントロールを選ぶと、その割り当ての編集になる
//!
//! - `pad`: 図の1コントロール分の描画(色・文字・値・プレビュー)
//! - `stage`: 右のパネル(何も選んでいないとき)
//! - `settings`: 右のパネル(2ページ目の設定)

mod pad;
mod settings;
mod stage;

use pad::{action_color, button_cell, effect_path, fader_cell, legend, pad_text, readout, Info, Preview, Readout};
use settings::settings_panel;
use stage::stage_panel;

use crate::apcmini::{pad_note, Ctl};
use crate::app::{App, SelMeta};
use crate::engine::{lock, Core};
use crate::pixel::{Energy, PixelParam};
use crate::project::*;
use crate::theme::{self, alpha, mix};
use crate::ui_binding::{binding_editor, default_action, names, summary, EditCtx, Learn};
use eframe::egui::{self, pos2, vec2, Align, Align2, Color32, FontId, Layout, Rect, RichText, Sense, Stroke, StrokeKind, UiBuilder};

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
                        if !used && p != page {
                            continue;
                        }
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
                            Some(f) => Preview::Face(core_ref.face_preview(f, 16, 30)),
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
                            RichText::new("上の4段=LEDバーの模様(チル/グルーヴ/ピーク/モノクロ)、5段目=ムービングなど、6・7段目=メイン色・サブ色、最下段=フラッシュなど。右の列=TAP・入れ替え・BLACKOUT、下の列=機種ごとの消灯、フェーダー=明るさ・速さ・斜め・色相")
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
                        if use_pages && common_pads > 0
                            && ui.button(format!("共通のパッド {common_pads} 件をページ{}専用にする", page + 1)).on_hover_text("「全ページ共通」になっているパッドの割り当てを、表示中のページだけで効くようにします(ボタンとフェーダーは共通のまま)").clicked() {
                            menu.move_common = true;
                            ui.close();
                        }
                    });
                    legend(ui);
                });
            });
            egui::ScrollArea::vertical().id_salt("map_panel").auto_shrink([false; 2]).show(ui, |ui| match self.map_sel {
                // 2ページ目(設定)では設定パネル、それ以外はステージ
                None if page == 1 => changed |= settings_panel(ui, core),
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

        if menu.read_faders
            && let Err(e) = self.midi.request_faders() {
            self.notice = format!("フェーダー位置の読み取りに失敗: {e}");
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
