//! キーボードタブ: MIDIコントローラーが無いときに、パソコンのキーボードで操作する。
//! 実際のキーの並びと同じ絵に、割り当てと「効いているか」を出す(キーを押しても、絵をクリックしても動く)

use dmxctl_core::engine::{lock, Core};
use dmxctl_core::keys::{self, code, COLS, ROWS, ROW_NAMES};
use dmxctl_core::motion::{MotionOp, POSITIONS, RATES, SHAPES, SPREADS};
use dmxctl_core::pixel::FACE_SETS;
use dmxctl_core::project::*;
use crate::theme::{self, alpha};
use eframe::egui::{self, Key, pos2, vec2, Align2, Color32, FontId, Rect, RichText, Sense, Stroke, StrokeKind};

/// 段ごとの左へのずれ(キーの幅に対する割合。実際のキーボードと同じく、下の段ほど右へずれる)
/// 段ごとのキー。文字は `core::keys::ROWS` と同じ並び
pub const KEYS: [[Key; COLS]; 4] = [
    [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9, Key::Num0],
    [Key::Q, Key::W, Key::E, Key::R, Key::T, Key::Y, Key::U, Key::I, Key::O, Key::P],
    [Key::A, Key::S, Key::D, Key::F, Key::G, Key::H, Key::J, Key::K, Key::L, Key::Semicolon],
    [Key::Z, Key::X, Key::C, Key::V, Key::B, Key::N, Key::M, Key::Comma, Key::Period, Key::Slash],
];

/// キーから番号。割り当てのないキーは None
pub fn from_key(k: Key) -> Option<u8> {
    KEYS.iter().enumerate().find_map(|(r, row)| row.iter().position(|x| *x == k).map(|c| code(r, c)))
}

const STAGGER: [f32; 4] = [0.0, 0.5, 0.75, 1.25];

impl crate::app::App {
    pub fn tab_keys(&mut self, ui: &mut egui::Ui) {
        let shared = self.host.core.clone();
        let mut guard = lock(&shared);
        let core = &mut *guard;

        theme::heading(ui, "キーボード");
        let has_keys = core.project.bindings.iter().any(|b| b.trigger.is_some_and(|t| t.kind == TriggerKind::Key));
        if !has_keys {
            ui.label(RichText::new("キーボードの割り当てがまだありません。MIDIコントローラーが無いときは、キーボードで操作できます。").color(theme::TEXT_DIM));
            if ui.button("キーボードの配置を作る").clicked() {
                let lib = core.lib.clone();
                dmxctl_core::layout::apply_keys(&mut core.project, &lib);
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
                    crate::map::tempo_color_preview(ui, core);
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
            for (c, &cap) in row.iter().enumerate() {
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
                    Some(Binding { id: 0, action: Action::Color { color: Some(nc), .. }, .. }) => {
                        let [cr, cg, cb] = nc.rgb().map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8);
                        let col = Color32::from_rgb(cr, cg, cb);
                        active = dmxctl_core::led::is_active(core, &b.unwrap().action);
                        let fill = if active { col } else { theme::mix(theme::WIDGET, col, 0.55) };
                        (fill, theme::text_on(fill))
                    }
                    Some(b) => {
                        let col = crate::map::action_color(&b.action, &core.project);
                        if dmxctl_core::led::is_active(core, &b.action) {
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
                core.live(dmxctl_core::live::LiveOp::Key { number: old, pressed: false });
            }
            if let Some(new) = press {
                core.live(dmxctl_core::live::LiveOp::Key { number: new, pressed: true });
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
                core.live(dmxctl_core::live::LiveOp::Motion { op: MotionOp::Size, value: size / 100.0 });
            }
        });
        // 画面にも、形・1周・揃え方・向きのボタン(キーボードと同じもの)
        let row = |ui: &mut egui::Ui, core: &mut Core, ops: Vec<MotionOp>| {
            ui.horizontal_wrapped(|ui| {
                for op in ops {
                    let a = Action::Motion { op };
                    let on = dmxctl_core::led::is_active(core, &a);
                    if ui.selectable_label(on, op.label()).clicked() {
                        core.live(dmxctl_core::live::LiveOp::Motion { op, value: 0.0 });
                    }
                }
            });
        };
        row(ui, core, SHAPES.iter().map(|x| MotionOp::Shape(x.0)).collect());
        row(ui, core, RATES.iter().map(|x| MotionOp::Beats(x.0)).chain(SPREADS.iter().map(|x| MotionOp::Spread(x.0))).collect());
        row(ui, core, (0..POSITIONS.len() as u8).map(MotionOp::Position).collect());

        ui.add_space(6.0);
        egui::CollapsingHeader::new("4点の動き(決めた4つの向きを順に移る)").id_salt("motion_points").default_open(m.shape == dmxctl_core::motion::Shape::Points).show(ui, |ui| {
            points_editor(ui, core);
        });
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

/// 「4点」の向きの編集: 図の点をドラッグ、数値、今の向きの登録、移る時間
fn points_editor(ui: &mut egui::Ui, core: &mut Core) {
    use dmxctl_core::motion::{Shape, DEFAULT_POINTS};
    ui.label(
        RichText::new("動きを「4点」にすると、1→2→3→4 の向きを順に移ります(1周 = 4つ。1周を「1小節」にすれば1拍ごと)。揃え方「ずらす」で台ごとに1つ先の向き、「左右対称」で右半分が逆になります")
            .small()
            .color(theme::TEXT_DIM),
    );
    let mut m = core.project.motion.clone();
    let before = m.clone();
    // 図: 横 = 左右(±90°)、縦 = 奥(上)〜客席(下)。真ん中の十字が真上
    const X: f32 = 90.0;
    const Y0: f32 = -45.0;
    const Y1: f32 = 90.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(240.0, 180.0), Sense::click_and_drag());
    let to_screen = |p: [f32; 2]| pos2(rect.left() + (p[0] + X) / (2.0 * X) * rect.width(), rect.top() + (p[1] - Y0) / (Y1 - Y0) * rect.height());
    let from_screen = |s: egui::Pos2| [((s.x - rect.left()) / rect.width() * 2.0 * X - X).clamp(-X, X).round(), ((s.y - rect.top()) / rect.height() * (Y1 - Y0) + Y0).clamp(Y0, Y1).round()];
    let p = ui.painter();
    p.rect_filled(rect, 6.0, theme::WIDGET);
    let o = to_screen([0.0, 0.0]);
    p.line_segment([pos2(rect.left(), o.y), pos2(rect.right(), o.y)], Stroke::new(1.0, theme::LINE));
    p.line_segment([pos2(o.x, rect.top()), pos2(o.x, rect.bottom())], Stroke::new(1.0, theme::LINE));
    p.text(o + vec2(4.0, -2.0), Align2::LEFT_BOTTOM, "真上", FontId::proportional(10.0), theme::TEXT_FAINT);
    p.text(pos2(rect.center().x, rect.bottom() - 3.0), Align2::CENTER_BOTTOM, "客席", FontId::proportional(10.0), theme::TEXT_FAINT);
    p.text(pos2(rect.center().x, rect.top() + 3.0), Align2::CENTER_TOP, "奥", FontId::proportional(10.0), theme::TEXT_FAINT);
    let pts: Vec<egui::Pos2> = m.points.iter().map(|&q| to_screen(q)).collect();
    for k in 0..4 {
        p.line_segment([pts[k], pts[(k + 1) % 4]], Stroke::new(1.5, alpha(theme::ACCENT, 0.5)));
    }
    for (k, &q) in pts.iter().enumerate() {
        p.circle_filled(q, 9.0, theme::ACCENT);
        p.text(q, Align2::CENTER_CENTER, format!("{}", k + 1), theme::bold(11.0), theme::text_on(theme::ACCENT));
    }
    // ドラッグ: 押したところに一番近い点を動かす
    let drag_id = ui.id().with("points_drag");
    if resp.drag_started()
        && let Some(pos) = resp.interact_pointer_pos() {
        let near = pts.iter().enumerate().min_by(|a, b| a.1.distance(pos).total_cmp(&b.1.distance(pos))).map(|(k, _)| k);
        ui.data_mut(|d| d.insert_temp(drag_id, near));
    }
    if resp.dragged()
        && let (Some(Some(k)), Some(pos)) = (ui.data(|d| d.get_temp::<Option<usize>>(drag_id)), resp.interact_pointer_pos()) {
        m.points[k] = from_screen(pos);
    }
    // 数値と、今の向きの登録
    egui::Grid::new("points_grid").num_columns(4).spacing([8.0, 4.0]).show(ui, |ui| {
        for k in 0..4 {
            ui.label(RichText::new(format!("{}", k + 1)).font(theme::bold(12.0)));
            ui.add(egui::DragValue::new(&mut m.points[k][0]).range(-X..=X).suffix("° 右"));
            ui.add(egui::DragValue::new(&mut m.points[k][1]).range(Y0..=Y1).suffix("° 前"));
            if ui.small_button("今の向き").on_hover_text("左端のムービングが今向いている向き(位置プリセット・フェーダーで合わせたもの)を、この点にします。動きを「止める」にしてから合わせてください").clicked()
                && let Some(l) = core.head_lean(0) {
                m.points[k] = l.map(|v| v.round());
            }
            ui.end_row();
        }
    });
    ui.horizontal(|ui| {
        ui.label("移る時間");
        ui.add(egui::Slider::new(&mut m.glide, 0.0..=1.0).custom_formatter(|v, _| match v {
            v if v < 0.01 => "一瞬".into(),
            v if v > 0.99 => "ずっと動く".into(),
            v => format!("{:.0}%", v * 100.0),
        }))
        .on_hover_text("1つの向きにいる時間のうち、次の向きへ動くのにかける割合。0 = 拍でパッと移る / 100% = 止まらずに動き続ける。次の拍ちょうどに着きます");
        if ui.small_button("最初の4点に戻す").clicked() {
            m.points = DEFAULT_POINTS;
        }
        if m.shape != Shape::Points && ui.button("「4点」で動かす").clicked() {
            core.live(dmxctl_core::live::LiveOp::Motion { op: MotionOp::Shape(Shape::Points), value: 0.0 });
            m.shape = Shape::Points;
        }
    });
    if m != before {
        core.project.motion.points = m.points;
        core.project.motion.glide = m.glide;
        core.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_codes_round_trip() {
        for (r, row) in KEYS.iter().enumerate() {
            for (c, k) in row.iter().enumerate() {
                assert_eq!(from_key(*k), Some(code(r, c)));
            }
        }
        assert_eq!(from_key(Key::Space), None);
    }
}
