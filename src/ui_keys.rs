//! キーボードタブ: MIDIコントローラーが無いときに、パソコンのキーボードで操作する。
//! 実際のキーの並びと同じ絵に、割り当てと「効いているか」を出す(キーを押しても、絵をクリックしても動く)

use crate::engine::{lock, Core};
use crate::keys::{self, COLS, ROWS, ROW_NAMES};
use crate::motion::{MotionOp, POSITIONS, RATES, SHAPES, SPREADS};
use crate::pixel::FACE_SETS;
use crate::project::*;
use crate::theme::{self, alpha};
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, RichText, Sense, Stroke, StrokeKind};

/// 段ごとの左へのずれ(キーの幅に対する割合。実際のキーボードと同じく、下の段ほど右へずれる)
const STAGGER: [f32; 4] = [0.0, 0.5, 0.75, 1.25];

impl crate::app::App {
    pub fn tab_keys(&mut self, ui: &mut egui::Ui) {
        let shared = self.core.clone();
        let mut guard = lock(&shared);
        let core = &mut *guard;

        theme::heading(ui, "キーボード");
        let has_keys = core.project.bindings.iter().any(|b| b.trigger.is_some_and(|t| t.kind == TriggerKind::Key));
        if !has_keys {
            ui.label(RichText::new("キーボードの割り当てがまだありません。MIDIコントローラーが無いときは、キーボードで操作できます。").color(theme::TEXT_DIM));
            if ui.button("キーボードの配置を作る").clicked() {
                let lib = core.lib.clone();
                crate::layout::apply_keys(&mut core.project, &lib);
                core.dirty = true;
            }
            return;
        }
        // Shiftを押している間は、絵も色の段に変わる(ボタンで色の段を出したままにもできる)
        let shift = ui.input(|i| i.modifiers.shift);
        ui.horizontal(|ui| {
            ui.label(RichText::new("段ごとに種類がそろえてあります。キーを押すか、絵をクリックしてください。光っているキーが今効いているもの。").small().color(theme::TEXT_DIM));
            ui.add_space(12.0);
            if ui.selectable_label(!self.key_color_view && !shift, "通常の段").clicked() {
                self.key_color_view = false;
            }
            if ui.selectable_label(self.key_color_view || shift, "色の段(Shift)").clicked() {
                self.key_color_view = true;
            }
        });
        ui.add_space(6.0);
        let color = self.key_color_view || shift;

        egui::ScrollArea::vertical().show(ui, |ui| {
            self.keyboard(ui, core, color);
            ui.add_space(10.0);
            // 下: 左=決まったキーと動き / 右=テンポと色のプレビュー
            let preview_w = 380.0f32.min(ui.available_width() * 0.45);
            ui.horizontal_top(|ui| {
                let left_w = (ui.available_width() - preview_w - 12.0).max(200.0);
                ui.vertical(|ui| {
                    ui.set_width(left_w);
                    ui.horizontal_wrapped(|ui| {
                        for (k, d) in keys::SPECIAL {
                            ui.label(RichText::new(k).font(theme::bold(12.0)).color(theme::ACCENT));
                            ui.label(RichText::new(d).small().color(theme::TEXT_DIM));
                            ui.add_space(10.0);
                        }
                    });
                    ui.add_space(10.0);
                    if !core.motion_heads().is_empty() {
                        motion_panel(ui, core);
                    }
                });
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    // カードの余白・枠のぶん(とスクロールバー)を引く
                    ui.set_width(preview_w - 32.0);
                    crate::ui_map::tempo_color_preview(ui, core);
                });
            });
        });
    }

    /// キーボードの絵
    /// `color`=Shiftの段(色)を出す
    fn keyboard(&mut self, ui: &mut egui::Ui, core: &mut Core, color: bool) {
        let offset = if color { keys::SHIFT } else { 0 };
        let label_w = 150.0;
        let gap = 6.0;
        let avail = ui.available_width() - label_w;
        let size = ((avail - gap * COLS as f32) / (COLS as f32 + STAGGER[3])).clamp(52.0, 104.0);
        let h = size * 0.78;
        let total = vec2(label_w + (size + gap) * (COLS as f32 + STAGGER[3]), (h + gap) * ROWS.len() as f32);
        let (area, _) = ui.allocate_exact_size(total, Sense::hover());
        let page = core.rt.key_page;
        let mut press: Option<u8> = None;

        for (r, row) in ROWS.iter().enumerate() {
            let y = area.top() + r as f32 * (h + gap);
            // 段の名前(数字の段は、今の模様のセット)
            let mut title = if color { format!("Shift+\n{}", keys::SHIFT_ROW_NAMES[r]) } else { ROW_NAMES[r].to_string() };
            if r == 0 && !color && let Some(set) = FACE_SETS.get(page as usize) {
                title = format!("{title}\n「{}」", set.name);
            }
            ui.painter().text(pos2(area.left(), y + h / 2.0), Align2::LEFT_CENTER, title, FontId::proportional(12.0), theme::TEXT_DIM);
            for (c, &(_, cap)) in row.iter().enumerate() {
                let code = keys::code(r, c) + offset;
                let x = area.left() + label_w + (STAGGER[r] + c as f32) * (size + gap);
                let rect = Rect::from_min_size(pos2(x, y), vec2(size, h));
                let b = core.project.bindings.iter().find(|b| b.trigger.is_some_and(|t| t.kind == TriggerKind::Key && t.number == code) && b.page.is_none_or(|p| p == page));
                let down = core.rt.keys_down.contains(&code);
                let resp = ui.interact(rect, ui.id().with(("key", code)), Sense::click_and_drag());
                if resp.is_pointer_button_down_on() && b.is_some() {
                    press = Some(code);
                }
                let p = ui.painter();
                let mut active = false;
                let (fill, text) = match b {
                    // 色を選ぶキーは、その色で塗る(選んでいる色は白い枠)
                    Some(Binding { action: Action::Color { color: Some(nc), .. }, .. }) => {
                        let [cr, cg, cb] = nc.rgb().map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8);
                        let col = Color32::from_rgb(cr, cg, cb);
                        active = crate::led::is_active(core, &b.unwrap().action);
                        let fill = if active { col } else { theme::mix(theme::WIDGET, col, 0.55) };
                        (fill, theme::text_on(fill))
                    }
                    Some(b) => {
                        let col = crate::ui_map::action_color(&b.action, &core.project);
                        if crate::led::is_active(core, &b.action) {
                            (col, theme::text_on(col))
                        } else {
                            (theme::mix(theme::WIDGET, col, 0.18), theme::TEXT)
                        }
                    }
                    None => (theme::alpha(theme::WIDGET, 0.5), theme::TEXT_FAINT),
                };
                p.rect_filled(rect, 6.0, fill);
                let stroke = if down || active { Stroke::new(if active { 3.0 } else { 2.0 }, Color32::WHITE) } else if resp.hovered() && b.is_some() { Stroke::new(1.0, theme::TEXT_DIM) } else { Stroke::new(1.0, theme::LINE) };
                p.rect_stroke(rect, 6.0, stroke, StrokeKind::Inside);
                p.text(rect.left_top() + vec2(6.0, 4.0), Align2::LEFT_TOP, cap, theme::bold(13.0), alpha(text, 0.7));
                if let Some(b) = b {
                    let galley = p.layout(b.name.clone(), FontId::proportional((size * 0.15).clamp(10.0, 13.0)), text, size - 8.0);
                    let pos = pos2(rect.center().x - galley.size().x / 2.0, rect.bottom() - galley.size().y - 6.0);
                    p.galley(pos, galley, text);
                }
            }
        }

        // 絵のクリック: 押している間だけ「押している」(フラッシュなど押している間だけのものも使える)
        if press != self.key_mouse {
            if let Some(old) = self.key_mouse.take() {
                core.handle_key(old, false);
            }
            if let Some(new) = press {
                core.handle_key(new, true);
                self.key_mouse = Some(new);
            }
        }
    }
}

/// 今の動きと、動きの大きさ・向き合わせ
fn motion_panel(ui: &mut egui::Ui, core: &mut Core) {
    theme::card().show(ui, |ui| {
        let m = core.project.motion.clone();
        let name = |op: MotionOp| op.label();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("ムービングの動き").font(theme::bold(14.0)));
            ui.add_space(8.0);
            let pos = m.position.and_then(|p| POSITIONS.get(p as usize).copied()).unwrap_or("今の位置");
            let text = format!(
                "{} ・ 1周 {} ・ {} ・ 向き {} ・ 大きさ {:.0}%",
                name(MotionOp::Shape(m.shape)),
                name(MotionOp::Beats(m.beats)),
                name(MotionOp::Spread(m.spread)),
                pos,
                m.size * 100.0
            );
            ui.label(RichText::new(text).color(theme::ACCENT));
        });
        ui.add_space(4.0);
        let mut size = m.size * 100.0;
        ui.horizontal(|ui| {
            ui.label(RichText::new("動きの大きさ").small().color(theme::TEXT_DIM));
            if ui.add(egui::Slider::new(&mut size, 0.0..=100.0).suffix("%").fixed_decimals(0)).changed() {
                core.motion_op(MotionOp::Size, size / 100.0);
            }
        });
        // 画面にも、形・1周・揃え方・向きのボタン(キーボードと同じもの)
        let row = |ui: &mut egui::Ui, core: &mut Core, ops: Vec<MotionOp>| {
            ui.horizontal_wrapped(|ui| {
                for op in ops {
                    let a = Action::Motion { op };
                    let on = crate::led::is_active(core, &a);
                    if ui.selectable_label(on, op.label()).clicked() {
                        core.motion_op(op, 0.0);
                    }
                }
            });
        };
        row(ui, core, SHAPES.iter().map(|x| MotionOp::Shape(x.0)).collect());
        row(ui, core, RATES.iter().map(|x| MotionOp::Beats(x.0)).chain(SPREADS.iter().map(|x| MotionOp::Spread(x.0))).collect());
        row(ui, core, (0..POSITIONS.len() as u8).map(MotionOp::Position).collect());

        ui.add_space(6.0);
        egui::CollapsingHeader::new("向き合わせ(最初に1回)").id_salt("motion_calib").show(ui, |ui| {
            ui.label(
                RichText::new("1.「向き」の「真上」を選び、ビームがまっすぐ上を向くまで「真上のTilt」を動かす\n2.「客席」を選び、客席側へ倒れるまで「正面のPan」を動かす\n   (左右と前後が入れ替わって見えるときも「正面のPan」で直ります)")
                    .small()
                    .color(theme::TEXT_DIM),
            );
            let mut changed = false;
            let mm = &mut core.project.motion;
            let mut pan = mm.pan_front * 255.0;
            let mut tilt = mm.tilt_up * 255.0;
            ui.horizontal(|ui| {
                ui.label("正面のPan");
                changed |= ui.add(egui::Slider::new(&mut pan, 0.0..=255.0).fixed_decimals(0)).changed();
            });
            ui.horizontal(|ui| {
                ui.label("真上のTilt");
                changed |= ui.add(egui::Slider::new(&mut tilt, 0.0..=255.0).fixed_decimals(0)).changed();
            });
            if changed {
                mm.pan_front = pan / 255.0;
                mm.tilt_up = tilt / 255.0;
                core.dirty = true;
            }
        });
    });
}
