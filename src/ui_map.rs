//! MIDIタブ(メイン画面): APC mini mk2 の配置図。割り当ての確認・編集と、押した/動かした状態の表示。
//! 右側は、何も選んでいないときは「ステージ」(LEDバーの今の模様・メイン/サブ色・機材の色)、
//! 図のコントロールを選ぶと、その割り当ての編集になる
//!
//! - `board`: 図全体(コントロールごとの情報を集めて、実機と同じ配置に並べる)
//! - `pad`: 図の1コントロール分の描画(色・文字・値・プレビュー)
//! - `editor`: 右のパネル(選んだコントロールの割り当ての編集)
//! - `stage`: 右のパネル(何も選んでいないとき)
//! - `settings`: 右のパネル(2ページ目の設定)

mod board;
mod editor;
mod pad;
mod settings;
mod stage;

use board::{control_info, controller_map};
use editor::{control_editor, EditorOut};
use pad::{button_cell, effect_path, fader_cell, legend, pad_text, readout, Info, Preview, Readout};
use settings::settings_panel;
pub(crate) use pad::action_color;
pub(crate) use stage::tempo_color_preview;
pub(crate) use settings::arm_button;
use stage::stage_panel;

use crate::apcmini::{pad_note, Ctl};
use crate::app::{App, SelMeta};
use crate::engine::{lock, Core, Runtime};
use crate::midi::Midi;
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

/// 全ページ共通の、パッド(チャンネル1のNote 0x00〜0x3F)の割り当てか
fn is_common_pad(b: &Binding) -> bool {
    b.page.is_none() && b.trigger.is_some_and(|t| t.kind == TriggerKind::Note && t.number < 64 && t.channel.is_none_or(|c| c == 0))
}

/// 割り当てのひな形を配置したときのお知らせ。`hint`=飛ばしたものが無いときに添える一言
fn placed_notice(what: &str, (added, skipped): (usize, usize), hint: &str) -> String {
    if skipped > 0 {
        format!("{what}を {added} 個配置しました({skipped} 個は使用中のため飛ばしました)")
    } else {
        format!("{what}を {added} 個配置しました{hint}")
    }
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

/// ページの切り替え(使っているページと、表示中のページ)。戻り値=押されたページ
fn page_bar(ui: &mut egui::Ui, project: &Project, page: u8) -> Option<u8> {
    let mut new_page = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new("ページ").small().color(theme::TEXT_DIM));
        for p in 0..PAGES {
            let used = project.bindings.iter().any(|b| b.page == Some(p));
            if !used && p != page {
                continue;
            }
            let text = RichText::new(format!("{}", p + 1)).color(if used || p == page { theme::TEXT } else { theme::TEXT_FAINT });
            if ui.selectable_label(p == page, text).on_hover_text("このページに切り替えます(コントローラーのパッドもこのページになります)").clicked() {
                new_page = Some(p);
            }
        }
    });
    new_page
}

/// 「配置 ▼」メニュー。`movable`=表示中のページ専用にできる共通のパッドの数(ページを使っていなければ0)
fn layout_menu(ui: &mut egui::Ui, menu: &mut MenuOut, midi: &Midi, movable: usize, page: u8) {
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
        if ui.add_enabled(midi.can_request_faders(), egui::Button::new("フェーダー位置を読み取る")).clicked() {
            menu.read_faders = true;
            ui.close();
        }
        if movable > 0
            && ui.button(format!("共通のパッド {movable} 件をページ{}専用にする", page + 1)).on_hover_text("「全ページ共通」になっているパッドの割り当てを、表示中のページだけで効くようにします(ボタンとフェーダーは共通のまま)").clicked() {
            menu.move_common = true;
            ui.close();
        }
    });
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
        let mut editor = EditorOut::default();
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
        let mut lui = ui.new_child(UiBuilder::new().max_rect(left).layout(Layout::top_down(Align::Min)));
        let (new_page, clicked) = {
            let ui = &mut lui;
            let new_page = if use_pages {
                let p = page_bar(ui, &core.project, page);
                ui.add_space(6.0);
                p
            } else {
                None
            };
            let core_ref: &Core = core;
            let look_rgb = crate::led::look_colors(core_ref);
            let info = |ctl: Ctl| control_info(core_ref, &meta, &look_rgb, page, ctl);
            (new_page, controller_map(ui, &core_ref.rt, self.map_sel, &info))
        };

        // ---- 右: ステージ / 割り当ての編集 ----
        let movable = if use_pages { core.project.bindings.iter().filter(|b| is_common_pad(b)).count() } else { 0 };
        let mut rui = ui.new_child(UiBuilder::new().max_rect(right).layout(Layout::top_down(Align::Min)));
        {
            let ui = &mut rui;
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    layout_menu(ui, &mut menu, &self.midi, movable, page);
                    legend(ui);
                });
            });
            egui::ScrollArea::vertical().id_salt("map_panel").auto_shrink([false; 2]).show(ui, |ui| match self.map_sel {
                // 2ページ目(設定)では設定パネル、それ以外はステージ
                None if page == 1 => changed |= settings_panel(ui, core),
                None => stage_panel(ui, core),
                Some(ctl) => editor = control_editor(ui, core, ctl, page, use_pages, &cx),
            });
        }

        if let Some(c) = clicked {
            // もう一度クリックすると閉じる
            self.map_sel = if self.map_sel == Some(c) { None } else { Some(c) };
        }
        if editor.close {
            self.map_sel = None;
        }
        changed |= editor.changed;
        if let Some(p) = new_page {
            core.rt.page = p;
        }
        changed |= self.apply_menu(core, &menu, page, use_pages);
        if let Some(i) = editor.remove {
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

    /// 配置メニューの「空いているところに足す」などを実行する(Coreを借りている間にできるもの)。戻り値=割り当てを変えた
    fn apply_menu(&mut self, core: &mut Core, menu: &MenuOut, page: u8, use_pages: bool) -> bool {
        let mut changed = false;
        if menu.page {
            self.notice = placed_notice("ページ切替", add_page_template(&mut core.project), "");
            changed = true;
        }
        if menu.move_common {
            for b in core.project.bindings.iter_mut().filter(|b| is_common_pad(b)) {
                b.page = Some(page);
            }
            changed = true;
        }
        if menu.pixel {
            self.notice = placed_notice("LEDバーの模様", crate::pixel::add_pad_template(&mut core.project, use_pages.then_some(page)), "(エフェクトタブで速さや幅を変えられます)");
            changed = true;
        }
        if menu.palette {
            self.notice = placed_notice("メイン/サブ色の操作", crate::color::add_palette_template(&mut core.project), "");
            changed = true;
        }
        if menu.color {
            self.notice = placed_notice("色の操作", crate::color::add_template(&mut core.project), "");
            changed = true;
        }
        changed
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
