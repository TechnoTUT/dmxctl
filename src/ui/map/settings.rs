//! 右のパネル(2ページ目): NDI・テンポの元・レーザー・色の系統。

use super::*;
use super::pad::rgb32;

/// 右のパネル(2ページ目): NDI・テンポの元・レーザー・色の系統。戻り値=設定を変えた
pub(super) fn settings_panel(ui: &mut egui::Ui, core: &mut Core) -> bool {
    use crate::core::color::ColorRole;
    use crate::core::io::ndi::LiveSource;
    let mut changed = false;
    // 長い名前などで、パネルの幅からはみ出さないようにする
    let width = ui.available_width();
    ui.set_max_width(width);
    let short = |s: &str, n: usize| if s.chars().count() > n { format!("{}…", s.chars().take(n).collect::<String>()) } else { s.to_string() };
    let swatch = |ui: &mut egui::Ui, c: Option<[f32; 3]>| {
        let (r, _) = ui.allocate_exact_size(vec2(22.0, 16.0), Sense::hover());
        match c {
            Some(c) => ui.painter().rect_filled(r, 4.0, rgb32(c, 1.0)),
            None => ui.painter().rect_stroke(r, 4.0, Stroke::new(1.0, theme::TEXT_FAINT), StrokeKind::Inside),
        };
    };

    // LEDバーの本数(9本以上パッチしてあるとき)
    let (using, patched) = core.bar_count();
    if patched > 8 {
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("LEDバー").font(theme::bold(13.0)));
                for n in [8, patched] {
                    if ui.selectable_label(using == n, format!("{n}本")).clicked() && using != n {
                        core.live(crate::core::live::LiveOp::SetBarCount { count: n });
                        changed = true;
                    }
                }
            });
        });
        ui.add_space(6.0);
    }

    // NDI
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("NDI").font(theme::bold(13.0)));
            changed |= ui.checkbox(&mut core.project.ndi.enabled, "受信").changed();
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                swatch(ui, core.rt.ndi.audio);
                ui.label(RichText::new("音").small().color(theme::TEXT_DIM));
                swatch(ui, core.rt.ndi.video);
                ui.label(RichText::new("映像").small().color(theme::TEXT_DIM));
            });
        });
        if core.project.ndi.enabled {
            let sources = core.rt.ndi.sources.clone();
            let cur = if core.project.ndi.source.is_empty() { "(ソースを選ぶ)".to_string() } else { short(&core.project.ndi.source, 34) };
            egui::ComboBox::from_id_salt("ndi_src").selected_text(cur).width(ui.available_width() - 8.0).show_ui(ui, |ui| {
                if sources.is_empty() {
                    ui.label(RichText::new("見つかりません").small().color(theme::TEXT_DIM));
                }
                for s in sources {
                    changed |= ui.selectable_value(&mut core.project.ndi.source, s.clone(), s).changed();
                }
            });
            ui.add(egui::Label::new(RichText::new(&core.rt.ndi.status).small().color(theme::TEXT_DIM)).truncate()).on_hover_text(&core.rt.ndi.status);
            for (label, slot) in [("メイン", 0), ("サブ", 1)] {
                ui.horizontal(|ui| {
                    ui.add_sized([44.0, 18.0], egui::Label::new(RichText::new(label).small().color(theme::TEXT_DIM)));
                    let v = if slot == 0 { &mut core.project.ndi.main } else { &mut core.project.ndi.sub };
                    for (s, t) in [(LiveSource::Off, "使わない"), (LiveSource::Video, "映像"), (LiveSource::Audio, "音")] {
                        changed |= ui.selectable_value(v, s, t).changed();
                    }
                });
            }
        }
    });
    ui.add_space(6.0);

    // テンポの元
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(RichText::new("テンポ").font(theme::bold(13.0)));
            for s in TempoSource::ALL {
                let label = match s {
                    TempoSource::Internal => "内部",
                    TempoSource::Link => "Ableton Link",
                    TempoSource::MidiClock => "MIDIクロック",
                    TempoSource::Audio => "音声",
                };
                if ui.selectable_label(core.project.tempo_source == s, label).clicked() && core.project.tempo_source != s {
                    core.live(crate::core::live::LiveOp::SetTempoSource { source: s });
                    changed = true;
                }
            }
        });
        if core.project.tempo_source == TempoSource::Link {
            let peers = core.rt.link_peers;
            let (text, color) = if peers > 0 { (format!("Link 接続中のアプリ {peers}"), theme::OK) } else { ("Link: 相手が見つかりません(相手のLinkをONに)".to_string(), theme::WARN) };
            ui.label(RichText::new(text).small().color(color));
        }
        if core.project.tempo_source == TempoSource::Audio {
            let ok = core.rt.audio_seen.is_some_and(|t| std::time::Instant::now().duration_since(t) < std::time::Duration::from_secs(5));
            ui.label(RichText::new(core.rt.audio_status.clone()).small().color(if ok { theme::OK } else { theme::WARN }));
        }
    });
    ui.add_space(6.0);

    // レーザー(ARMだけ。位置はフェーダーやパッドで誤って動かさないよう、「レーザー」タブでマウスで合わせる)
    let lasers: Vec<(u32, String, bool)> = core
        .project
        .fixtures
        .iter()
        .filter(|f| core.lib.get(&f.profile).is_some_and(|p| p.needs_arm))
        .map(|f| (f.id, f.name.clone(), f.armed))
        .collect();
    if !lasers.is_empty() {
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("レーザー").font(theme::bold(13.0)));
            for (id, name, armed) in lasers {
                ui.horizontal(|ui| {
                    if arm_button(ui, armed).clicked() {
                        core.live(crate::core::live::LiveOp::SetArm { fixture: id, on: !armed });
                    }
                    ui.add(egui::Label::new(RichText::new(&name).small()).truncate());
                });
            }
        });
        ui.add_space(6.0);
    }

    // 色の系統(機材ごと)
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new("色の系統").font(theme::bold(13.0)));
        let list: Vec<(u32, String, ColorRole)> = core.project.fixtures.iter().map(|f| (f.id, f.name.clone(), f.color_role)).collect();
        let main = core.role_rgb(ColorRole::Main);
        let sub = core.role_rgb(ColorRole::Sub);
        egui::Grid::new("roles").num_columns(2).spacing([8.0, 4.0]).show(ui, |ui| {
            for (id, name, role) in list {
                ui.horizontal(|ui| {
                    swatch(ui, match role {
                        ColorRole::Main => main,
                        ColorRole::Sub => sub,
                        ColorRole::Off => None,
                    });
                    ui.add_sized([110.0, 18.0], egui::Label::new(RichText::new(&name).small()).truncate());
                });
                ui.horizontal(|ui| {
                    for r in ColorRole::ALL {
                        if ui.selectable_label(role == r, RichText::new(r.label()).small()).clicked() && role != r {
                            core.live(crate::core::live::LiveOp::SetColorRole { fixture: id, role: r });
                            changed = true;
                        }
                    }
                });
                ui.end_row();
            }
        });
    });
    changed
}

/// レーザーのARMボタン(押すと出力を許可/禁止)
pub(crate) fn arm_button(ui: &mut egui::Ui, armed: bool) -> egui::Response {
    let (text, fill) = if armed { ("ARM中", theme::DANGER) } else { ("ARM", theme::WIDGET) };
    ui.add(egui::Button::new(RichText::new(text).small().color(Color32::WHITE)).fill(fill).min_size(vec2(52.0, 0.0))).on_hover_text("出力を許可します(OFFの間は安全値=レーザーOFF)")
}
