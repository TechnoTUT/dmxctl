//! 右のパネル(何も選んでいないとき): テンポ、LEDバーの今の模様、メイン/サブ色、機材ごとの色。

use super::*;
use super::pad::{energy_color, rgb32};

/// 右のパネル(何も選んでいないとき): LEDバーの今の模様、メイン/サブ色、機材ごとの色
pub(super) fn stage_panel(ui: &mut egui::Ui, core: &mut Core) {
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
            // 本数が多いときは1本を細く
            let (row_h, row_gap) = if bars.len() > 8 { (6.0, 2.0) } else { (9.0, 4.0) };
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

    color_card(ui, core);
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

/// テンポと、メイン/サブ色だけのプレビュー(キーボードタブの右下)
pub(crate) fn tempo_color_preview(ui: &mut egui::Ui, core: &mut Core) {
    tempo_card(ui, core);
    ui.add_space(6.0);
    color_card(ui, core);
}

/// メイン/サブ色: 色見本と、色相フェーダーと同じ並びの色相の帯(目印=フェーダーの位置)
fn color_card(ui: &mut egui::Ui, core: &Core) {
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
                TempoSource::Audio => {
                    let ok = core.rt.audio_seen.is_some_and(|t| now.duration_since(t) < std::time::Duration::from_secs(5));
                    ui.label(RichText::new("音声").small().strong().color(if ok { theme::OK } else { theme::WARN })).on_hover_text(core.rt.audio_status.clone());
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
