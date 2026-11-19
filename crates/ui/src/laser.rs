//! レーザータブ: レーザーの位置・大きさと、動いてよい範囲をマウスだけで合わせる。
//! MIDIのフェーダーやパッドには位置を置かない(誤操作で客席に向けないように)。
//! 動いてよい範囲は保存され、エフェクト・シーン・MIDIのどれで動かしても、出力の直前に必ずこの中に収める

use dmxctl_core::engine::{lock, Core};
use dmxctl_core::fixture::ChannelDef;
use dmxctl_core::project::*;
use crate::theme::{self, alpha};
use crate::map::arm_button;
use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, RichText, Sense, Stroke, StrokeKind};

/// 位置の軸(X=pan / Y=tilt)
struct Axis {
    attr: &'static str,
    /// 位置として使える一番上の値(それより上はレーザーの自動の動きなので使わない)
    max: u8,
    /// 動いてよい範囲
    lo: u8,
    hi: u8,
    /// 今の値(ベース値)と、実際に出ている値
    base: u8,
    out: u8,
}

impl Axis {
    fn new(core: &Core, fx: &FixtureInstance, attr: &'static str) -> Option<Axis> {
        let mode = core.lib.mode(&fx.profile, fx.mode)?;
        let ch = mode.find_attr(attr)?;
        let max = position_max(&mode.channels[ch]);
        let [lo, hi] = fx.limits.get(attr).copied().unwrap_or([0, max]);
        let addr = fx.address as usize + ch;
        let out = core.rt.frames.get(fx.universe).and_then(|f| addr.checked_sub(1).and_then(|a| f.get(a))).copied().unwrap_or(0);
        Some(Axis { attr, max, lo: lo.min(hi).min(max), hi: hi.max(lo).min(max), base: fx.values.get(ch).copied().unwrap_or(0), out })
    }
    /// 0..1 にした位置
    fn t(&self, v: u8) -> f32 {
        v as f32 / self.max.max(1) as f32
    }
    fn value_at(&self, t: f32) -> u8 {
        ((t.clamp(0.0, 1.0) * self.max as f32).round() as u8).clamp(self.lo, self.hi)
    }
}

/// 位置として使える一番上の値: 位置の範囲(連続で動く範囲)の上端。禁止の値は使わない
fn position_max(ch: &ChannelDef) -> u8 {
    let top = ch.ranges.iter().find(|r| r.sweep && r.from == 0).map(|r| r.to).unwrap_or(255);
    (0..=top).rev().find(|&v| !ch.is_forbidden(v)).unwrap_or(0)
}

impl crate::app::App {
    pub fn tab_laser(&mut self, ui: &mut egui::Ui) {
        let shared = self.host.core.clone();
        let mut guard = lock(&shared);
        let core = &mut *guard;
        let mut changed = false;

        theme::heading(ui, "レーザー");
        let ids: Vec<u32> = core
            .project
            .fixtures
            .iter()
            .filter(|f| core.lib.get(&f.profile).is_some_and(|p| p.needs_arm) && core.lib.mode(&f.profile, f.mode).is_some_and(|m| m.find_attr("pan").is_some()))
            .map(|f| f.id)
            .collect();
        if ids.is_empty() {
            ui.label(RichText::new("レーザーがありません").color(theme::TEXT_DIM));
            return;
        }
        if self.laser_edit.is_none_or(|id| !ids.contains(&id)) {
            self.laser_edit = None;
        }

        laser_cue_bar(ui, core, &mut self.laser_gui_hold);
        ui.add_space(6.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for id in ids {
                    let Some(i) = core.project.fixture_index(id) else { continue };
                    theme::card().show(ui, |ui| {
                        ui.vertical(|ui| {
                            changed |= laser_card(ui, core, i, &mut self.laser_edit);
                        });
                    });
                }
            });
        });
        if changed {
            core.dirty = true;
        }
    }
}

/// 合図: ARMしたレーザーは、ここ・パッド・オートのフレーズ・OSC のどれかで合図があるときだけ出る
fn laser_cue_bar(ui: &mut egui::Ui, core: &mut Core, gui_hold: &mut bool) {
    ui.horizontal_wrapped(|ui| {
        let latch = core.rt.laser_latch;
        let (text, fill) = if latch { ("常時出力 ON", theme::DANGER) } else { ("常時出力", theme::WIDGET) };
        if ui.add(egui::Button::new(RichText::new(text).strong()).fill(fill)).on_hover_text("ARM済みのレーザーを、再度押すまで出力します(起動のたびにOFF)").clicked() {
            core.live(dmxctl_core::live::LiveOp::SetLaserLatch { on: !latch });
        }
        // 押下中のみ出力(位置を合わせるときなど)
        let hold = ui.add(egui::Button::new(RichText::new("押下中のみ出力").strong()).fill(if core.rt.laser_hold { theme::DANGER } else { theme::WIDGET }).sense(Sense::click_and_drag()));
        // 押した・離したときだけ変える(パッドやOSCで押している間を、ここで切らない)
        let down = hold.is_pointer_button_down_on();
        if down != *gui_hold {
            *gui_hold = down;
            core.live(dmxctl_core::live::LiveOp::SetLaserHold { on: down });
        }
        let auto = core.rt.auto_laser && core.project.auto.enabled && core.rt.auto_was_online;
        let state = if core.laser_cue() {
            let why = if core.rt.laser_hold { "押下中" } else if latch { "常時出力" } else if auto { "オートのフレーズ" } else { "" };
            RichText::new(format!("合図あり({why}): ARM済みのレーザーを出力中")).color(theme::DANGER).strong()
        } else {
            RichText::new("合図なし: ARM済みでも出力しません(パッド・オートのフレーズ・OSC /dmx/laser で出力)").color(theme::TEXT_DIM)
        };
        ui.label(state);
    });
}

/// レーザー1台分。戻り値=保存する設定を変えた
fn laser_card(ui: &mut egui::Ui, core: &mut Core, i: usize, editing: &mut Option<u32>) -> bool {
    let mut changed = false;
    let fx = core.project.fixtures[i].clone();
    let (Some(x), y) = (Axis::new(core, &fx, "pan"), Axis::new(core, &fx, "tilt")) else { return false };
    let edit = *editing == Some(fx.id);

    ui.horizontal(|ui| {
        if arm_button(ui, fx.armed).clicked() {
            core.live(dmxctl_core::live::LiveOp::SetArm { fixture: fx.id, on: !fx.armed });
        }
        ui.label(RichText::new(&fx.name).font(theme::bold(14.0)));
    });
    ui.add_space(4.0);

    // 位置(ドラッグ)。編集中は、ドラッグで動いてよい範囲を描く
    let side = ui.available_width().clamp(220.0, 320.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(side, side), Sense::click_and_drag());
    let to_pos = |tx: f32, ty: f32| pos2(rect.left() + tx * rect.width(), rect.bottom() - ty * rect.height());
    let ty = |v: u8| y.as_ref().map(|a| a.t(v)).unwrap_or(0.5);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, theme::WIDGET);
    for k in 1..4 {
        let f = k as f32 / 4.0;
        let c = theme::LINE;
        painter.line_segment([pos2(rect.left() + f * rect.width(), rect.top()), pos2(rect.left() + f * rect.width(), rect.bottom())], Stroke::new(1.0, c));
        painter.line_segment([pos2(rect.left(), rect.top() + f * rect.height()), pos2(rect.right(), rect.top() + f * rect.height())], Stroke::new(1.0, c));
    }
    // 動いてよい範囲
    let (ylo, yhi) = y.as_ref().map(|a| (a.lo, a.hi)).unwrap_or((0, 0));
    let allowed = Rect::from_two_pos(to_pos(x.t(x.lo), ty(ylo)), to_pos(x.t(x.hi), ty(yhi)));
    let accent = if edit { theme::WARN } else { theme::ACCENT };
    painter.rect_filled(allowed, 3.0, alpha(accent, 0.12));
    painter.rect_stroke(allowed, 3.0, Stroke::new(1.5, accent), StrokeKind::Inside);
    // 実際に出ている位置(輪)と、合わせた位置(点)
    let lit = fx.armed && core.laser_cue() && !core.rt.blackout;
    let out = to_pos(x.t(x.out), y.as_ref().map(|a| a.t(a.out)).unwrap_or(0.5));
    painter.circle_stroke(out, 9.0, Stroke::new(2.0, if lit { theme::DANGER } else { theme::TEXT_FAINT }));
    let base = to_pos(x.t(x.base), y.as_ref().map(|a| a.t(a.base)).unwrap_or(0.5));
    painter.circle_filled(base, 4.5, if lit { Color32::WHITE } else { theme::TEXT_DIM });
    let font = FontId::monospace(10.0);
    painter.text(rect.left_top() + vec2(6.0, 4.0), Align2::LEFT_TOP, format!("X {}  Y {}", x.base, y.as_ref().map(|a| a.base).unwrap_or(0)), font, theme::TEXT_DIM);

    if let Some(p) = resp.interact_pointer_pos() {
        let tx = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let tyv = ((rect.bottom() - p.y) / rect.height()).clamp(0.0, 1.0);
        if edit {
            // 押したところから今のところまでを、動いてよい範囲にする
            if let Some(start) = ui.input(|inp| inp.pointer.press_origin()) {
                let sx = ((start.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
                let sy = ((rect.bottom() - start.y) / rect.height()).clamp(0.0, 1.0);
                let f = &mut core.project.fixtures[i];
                set_limit(f, &x, sx, tx);
                if let Some(y) = &y {
                    set_limit(f, y, sy, tyv);
                }
                changed = true;
            }
        } else {
            let sel = Selector::Fixture(fx.id);
            core.live(dmxctl_core::live::LiveOp::SetAttr { sel: sel.clone(), attr: x.attr.to_string(), value: x.value_at(tx) as f32 });
            if let Some(y) = &y {
                core.live(dmxctl_core::live::LiveOp::SetAttr { sel: sel.clone(), attr: y.attr.to_string(), value: y.value_at(tyv) as f32 });
            }
            changed = true;
        }
    }

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.add_enabled(!edit, egui::Button::new("中央")).clicked() {
            let sel = Selector::Fixture(fx.id);
            core.live(dmxctl_core::live::LiveOp::SetAttr { sel: sel.clone(), attr: x.attr.to_string(), value: ((x.lo as u16 + x.hi as u16) / 2) as f32 });
            if let Some(y) = &y {
                core.live(dmxctl_core::live::LiveOp::SetAttr { sel: sel.clone(), attr: y.attr.to_string(), value: ((y.lo as u16 + y.hi as u16) / 2) as f32 });
            }
            changed = true;
        }
        // 範囲の編集は、ボタンで切り替えたときだけ(ふだんのドラッグで範囲が変わらないように)
        let label = if edit { "範囲を決定" } else { "範囲を変更" };
        let fill = if edit { theme::WARN } else { theme::WIDGET };
        if ui.add(egui::Button::new(RichText::new(label).color(theme::text_on(fill))).fill(fill)).clicked() {
            *editing = if edit { None } else { Some(fx.id) };
        }
        if edit && ui.button("範囲を全部に").clicked() {
            core.project.fixtures[i].limits.remove(x.attr);
            if let Some(y) = &y {
                core.project.fixtures[i].limits.remove(y.attr);
            }
            changed = true;
        }
    });

    // 大きさ(0=100% / 127=5%)
    if let Some(ch) = core.lib.mode(&fx.profile, fx.mode).and_then(|m| m.find_attr("size")) {
        let v = fx.values.get(ch).copied().unwrap_or(0).min(127);
        let mut pct = 100.0 - v as f32 * 95.0 / 127.0;
        ui.horizontal(|ui| {
            ui.label(RichText::new("大きさ").small().color(theme::TEXT_DIM));
            ui.spacing_mut().slider_width = (side - 90.0).max(80.0);
            if ui.add(egui::Slider::new(&mut pct, 5.0..=100.0).suffix("%").fixed_decimals(0)).changed() {
                core.live(dmxctl_core::live::LiveOp::SetAttr { sel: Selector::Fixture(fx.id), attr: "size".into(), value: ((100.0 - pct) * 127.0 / 95.0).round() });
                changed = true;
            }
        });
    }
    changed
}

/// 押したところ`a`から今のところ`b`(どちらも0..1)を、その軸の動いてよい範囲にする
fn set_limit(f: &mut FixtureInstance, axis: &Axis, a: f32, b: f32) {
    let v = |t: f32| (t * axis.max as f32).round() as u8;
    let (lo, hi) = (v(a.min(b)), v(a.max(b)));
    if lo == 0 && hi == axis.max {
        f.limits.remove(axis.attr);
    } else {
        f.limits.insert(axis.attr.to_string(), [lo, hi]);
    }
}
