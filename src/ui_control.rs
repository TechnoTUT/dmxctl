//! コントロールタブ: 機材ごとのチャンネル操作。カテゴリ(動き/色/光量...)ごとにまとめて表示する。

use crate::app::{App, RED, WARN};
use crate::color::{ColorRole, ColorTarget, NamedColor};
use crate::engine::{lock, PairView};
use crate::fixture::{Mode, Profile};
use crate::project::FixtureInstance;
use eframe::egui::{self, Color32, RichText, Slider};
use std::collections::{HashMap, HashSet};

/// カテゴリの目印の色(標準のものは固定、自作は名前から決める)
pub fn category_color(name: &str) -> Color32 {
    match name {
        "動き" => Color32::from_rgb(90, 150, 235),
        "色" => Color32::from_rgb(225, 105, 200),
        "光量" => Color32::from_rgb(230, 180, 60),
        "形" => Color32::from_rgb(90, 190, 110),
        "制御" => Color32::from_rgb(170, 170, 175),
        "その他" => Color32::from_rgb(150, 150, 150),
        other => {
            let h = other.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
            Color32::from_rgb(110 + (h % 100) as u8, 110 + ((h >> 8) % 100) as u8, 110 + ((h >> 16) % 100) as u8)
        }
    }
}

impl App {
    pub fn tab_control(&mut self, ui: &mut egui::Ui) {
        let shared = self.host.core.clone();
        let mut guard = lock(&shared);
        let core = &mut *guard;
        let lib = core.lib.clone();
        let mut touched: Vec<(u32, usize)> = Vec::new();

        if core.project.fixtures.is_empty() {
            ui.label("機材がありません。「パッチ」タブで追加してください。");
        }

        // 配置中の機材にあるカテゴリだけを、表示の切り替えボタンにする
        let present = core.present_categories();
        if present.len() > 1 {
            ui.horizontal_wrapped(|ui| {
                ui.label("表示するカテゴリ:");
                for c in &present {
                    let mut on = !self.cat_hidden.contains(c);
                    let text = RichText::new(c.as_str()).color(if on { Color32::BLACK } else { category_color(c) });
                    let btn = egui::Button::new(text).fill(if on { category_color(c) } else { Color32::TRANSPARENT }).stroke(egui::Stroke::new(1.0, category_color(c)));
                    if ui.add(btn).clicked() {
                        on = !on;
                        if on {
                            self.cat_hidden.remove(c);
                        } else {
                            self.cat_hidden.insert(c.clone());
                        }
                    }
                }
                if !self.cat_hidden.is_empty() && ui.small_button("すべて表示").clicked() {
                    self.cat_hidden.clear();
                }
            });
            ui.separator();
        }

        // 色の全体制御。操作先: 全体/個別スイッチ(上部のボタン) / メイン色 / サブ色
        if !core.project.fixtures.is_empty() {
            let target = self.color_edit;
            ui.horizontal_wrapped(|ui| {
                let n = core.color_targets(ColorTarget::Scope).len();
                let scope = if core.rt.color_global { "全体".to_string() } else { format!("個別 {n}台") };
                ui.label(RichText::new("色").strong());
                ui.selectable_value(&mut self.color_edit, ColorTarget::Scope, scope).on_hover_text("選んだ色を、機材に直接かけます(上部の「色: 全体/個別」に従う)");
                ui.selectable_value(&mut self.color_edit, ColorTarget::Main, "→メイン").on_hover_text("選んだ色を「メイン色」にします(メインの機材に出ます)");
                ui.selectable_value(&mut self.color_edit, ColorTarget::Sub, "→サブ").on_hover_text("選んだ色を「サブ色」にします(サブの機材に出ます)");
                ui.label(":");
                let current = match target {
                    ColorTarget::Main => Some(core.project.palette.main),
                    ColorTarget::Sub => Some(core.project.palette.sub),
                    _ => None,
                };
                for c in NamedColor::ALL {
                    let [r, g, b] = c.rgb();
                    let lum = 0.3 * r + 0.59 * g + 0.11 * b;
                    let text = RichText::new(c.label()).color(if lum > 0.55 { Color32::BLACK } else { Color32::WHITE });
                    let mut btn = egui::Button::new(text).fill(rgb32(c.rgb()));
                    let on = match current {
                        Some(cur) => crate::color::same_rgb(cur, c.rgb()),
                        None => core.rt.last_color == Some(c),
                    };
                    if on {
                        btn = btn.stroke(egui::Stroke::new(2.0, Color32::from_rgb(255, 210, 60)));
                    }
                    if ui.add(btn).clicked() {
                        core.live(crate::live::LiveOp::NamedColor { color: c, target });
                    }
                }
                // HSVのH(0〜360度)。MIDIフェーダーで動かしたときも、このスライダーが追従する
                let mut deg = current.map(crate::color::hue_of).unwrap_or(core.rt.hue);
                ui.label("H");
                if ui.add(Slider::new(&mut deg, 0.0..=360.0).fixed_decimals(0).suffix("°")).on_hover_text("色相(HSVのH)。0=赤 / 120=緑 / 240=青 / 360=赤に戻る").changed() {
                    core.live(crate::live::LiveOp::Hue { degrees: deg, target });
                }
                let (swatch, _) = ui.allocate_exact_size(egui::vec2(24.0, 18.0), egui::Sense::hover());
                ui.painter().rect_filled(swatch, 3.0, rgb32(current.unwrap_or_else(|| crate::color::hsv(deg, 1.0, 1.0))));
            });
            self.palette_row(ui, core);
            ui.separator();
        }

        // 位置プリセット(パン/チルト)
        let has_pt = core.project.fixtures.iter().any(|f| lib.mode(&f.profile, f.mode).is_some_and(|m| m.find_attr("pan").is_some() || m.find_attr("tilt").is_some()));
        if has_pt {
            let mut recall: Option<String> = None;
            let mut overwrite: Option<usize> = None;
            let mut delete: Option<usize> = None;
            let mut save = false;
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("位置:").strong()).on_hover_text("パン/チルトの向きを保存して呼び出します。右クリックで上書き・フェード・削除");
                for (i, p) in core.project.positions.iter_mut().enumerate() {
                    let mut btn = egui::Button::new(&p.name);
                    if core.rt.last_position.as_deref() == Some(p.name.as_str()) {
                        btn = btn.stroke(egui::Stroke::new(2.0, Color32::from_rgb(255, 210, 60)));
                    }
                    let resp = ui.add(btn);
                    if resp.clicked() {
                        recall = Some(p.name.clone());
                    }
                    resp.context_menu(|ui| {
                        ui.horizontal(|ui| {
                            ui.label("フェード");
                            if ui.add(egui::DragValue::new(&mut p.fade).range(0.0..=10.0).speed(0.05).suffix(" 秒")).changed() {
                                core.dirty = true;
                            }
                        });
                        if ui.button("今の位置で上書き").clicked() {
                            overwrite = Some(i);
                            ui.close();
                        }
                        if ui.button("削除").clicked() {
                            delete = Some(i);
                            ui.close();
                        }
                    });
                }
                ui.separator();
                ui.add(egui::TextEdit::singleline(&mut self.pos_name).hint_text("名前(客席 など)").desired_width(110.0));
                ui.add(egui::DragValue::new(&mut self.pos_fade).range(0.0..=10.0).speed(0.05).suffix(" 秒")).on_hover_text("呼び出したときに、この時間をかけて動く");
                if ui.button("＋ 今の位置を保存").clicked() {
                    save = true;
                }
            });
            if let Some(n) = recall {
                core.live(crate::live::LiveOp::RecallPosition { name: n });
            }
            if let Some(i) = overwrite {
                let (name, fade) = (core.project.positions[i].name.clone(), core.project.positions[i].fade);
                core.project.positions[i] = core.capture_position(&name, fade);
                core.dirty = true;
            }
            if let Some(i) = delete {
                core.project.positions.remove(i);
                core.dirty = true;
            }
            if save {
                let name = if self.pos_name.trim().is_empty() { format!("位置{}", core.project.positions.len() + 1) } else { self.pos_name.trim().to_string() };
                let p = core.capture_position(&name, self.pos_fade);
                // 同じ名前があれば上書き
                match core.project.positions.iter().position(|x| x.name == name) {
                    Some(i) => core.project.positions[i] = p,
                    None => core.project.positions.push(p),
                }
                core.rt.last_position = Some(name);
                self.pos_name.clear();
                core.dirty = true;
            }
            ui.separator();
        }

        ui.spacing_mut().slider_width = 260.0;
        // ペアで連動している機材は、連動しているチャンネルを(連動後の値で)操作不可にして表示する
        let views: HashMap<u32, PairView> = core.project.fixtures.iter().filter_map(|f| core.paired_view(f.id).map(|v| (f.id, v))).collect();
        let hidden = &self.cat_hidden;
        let frames = &core.rt.frames;
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            for fx in core.project.fixtures.iter_mut() {
                let Some(profile) = lib.get(&fx.profile) else {
                    ui.colored_label(RED, format!("{}: 機種定義が見つかりません ({})", fx.name, fx.profile));
                    continue;
                };
                let Some(mode) = profile.modes.get(fx.mode) else {
                    ui.colored_label(RED, format!("{}: モードが不正です", fx.name));
                    continue;
                };
                if fx.values.len() != mode.channels.len() {
                    fx.values = mode.default_values();
                }
                // LEDバーの見本用に、今の出力値(エフェクト込み)を取り出す
                let out: Option<Vec<u8>> = (fx.address > 0).then(|| frames.get(fx.universe)).flatten().map(|f| {
                    let a = (fx.address as usize - 1).min(512);
                    f[a..(a + mode.channels.len()).min(512)].to_vec()
                });
                fixture_panel(ui, fx, profile, mode, hidden, views.get(&fx.id), out.as_deref(), &mut touched);
            }
        });

        if !touched.is_empty() {
            // 手動で触ったチャンネルは実行中のフェードを止める
            core.rt.fades.retain(|f| !touched.contains(&(f.fixture, f.ch)));
            core.dirty = true;
        }
    }
}

fn rgb32(c: [f32; 3]) -> Color32 {
    let f = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(f(c[0]), f(c[1]), f(c[2]))
}

impl App {
    /// メイン色/サブ色: 色の指定、入れ替え、拍での自動入れ替え、機材ごと・機種ごとの割り当て
    fn palette_row(&mut self, ui: &mut egui::Ui, core: &mut crate::engine::Core) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("メイン/サブ:").strong()).on_hover_text("機材をメイン/サブに分けて、2つの色を出し分けます。入れ替えで一瞬で切り替わります");
            let swapped = core.palette_swapped();
            for (slot, name) in [(ColorRole::Main, "メイン"), (ColorRole::Sub, "サブ")] {
                ui.label(name);
                let mut rgb = if slot == ColorRole::Main { core.project.palette.main } else { core.project.palette.sub };
                if ui.color_edit_button_rgb(&mut rgb).on_hover_text("クリックで色を自由に選べます").changed() {
                    core.live(crate::live::LiveOp::SetPaletteColor { slot, rgb });
                }
            }
            let text = if swapped { "⇄ 入れ替え中" } else { "⇄ 入れ替え" };
            if ui.selectable_label(core.rt.color_swapped, text).on_hover_text("メインの機材にサブ色、サブの機材にメイン色を出します(もう一度で戻る)").clicked() {
                core.live(crate::live::LiveOp::SwapPalette);
            }
            let mut auto = core.project.palette.auto;
            if ui.checkbox(&mut auto, "拍で自動入れ替え").changed() {
                core.live(crate::live::LiveOp::SetPaletteAuto { on: auto });
            }
            let mut beats = core.project.palette.auto_beats;
            if ui.add(egui::DragValue::new(&mut beats).range(0.25..=64.0).speed(0.05).suffix(" 拍ごと")).changed() {
                core.project.palette.auto_beats = beats;
                core.dirty = true;
            }
        });
        // 機材ごと(同じ機種が複数あるときは、機種ごとにもまとめて選べる)
        let fixtures: Vec<(u32, String, String, ColorRole)> = core.project.fixtures.iter().map(|f| (f.id, f.name.clone(), f.profile.clone(), f.color_role)).collect();
        let mut set: Vec<(u32, ColorRole)> = Vec::new();
        egui::CollapsingHeader::new("機材ごとのメイン/サブ").id_salt("crole_list").default_open(false).show(ui, |ui| ui.horizontal_wrapped(|ui| {
            for (id, name, _, role) in &fixtures {
                let (sw, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
                match core.role_rgb(*role) {
                    Some(c) => ui.painter().rect_filled(sw, 2.0, rgb32(c)),
                    None => ui.painter().rect_stroke(sw, 2.0, egui::Stroke::new(1.0, Color32::GRAY), egui::StrokeKind::Inside),
                };
                ui.label(name);
                egui::ComboBox::from_id_salt(("crole", *id)).selected_text(role.label()).width(64.0).show_ui(ui, |ui| {
                    for r in ColorRole::ALL {
                        if ui.selectable_label(*role == r, r.label()).clicked() && *role != r {
                            set.push((*id, r));
                        }
                    }
                });
            }
            let mut profiles: Vec<&String> = Vec::new();
            for (_, _, p, _) in &fixtures {
                if fixtures.iter().filter(|f| &f.2 == p).count() > 1 && !profiles.contains(&p) {
                    profiles.push(p);
                }
            }
            if !profiles.is_empty() {
                ui.separator();
                ui.label(RichText::new("機種ごと:").weak());
                for p in profiles {
                    let name = core.lib.get(p).map(|x| x.name.clone()).unwrap_or_else(|| p.clone());
                    ui.menu_button(name, |ui| {
                        for r in ColorRole::ALL {
                            if ui.button(format!("すべて{}", r.label())).clicked() {
                                set.extend(fixtures.iter().filter(|f| &f.2 == p).map(|f| (f.0, r)));
                                ui.close();
                            }
                        }
                    });
                }
            }
        }));
        for (id, r) in set {
            core.live(crate::live::LiveOp::SetColorRole { fixture: id, role: r });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fixture_panel(ui: &mut egui::Ui, fx: &mut FixtureInstance, profile: &Profile, mode: &Mode, hidden: &HashSet<String>, view: Option<&PairView>, out: Option<&[u8]>, touched: &mut Vec<(u32, usize)>) {
    let title = format!("{}   {} / {}   U{} @{}", fx.name, profile.name, mode.name, fx.universe + 1, fx.address);
    egui::CollapsingHeader::new(RichText::new(title).strong()).id_salt(fx.id).default_open(false).show(ui, |ui| {
        if profile.needs_arm {
            ui.horizontal(|ui| {
                let (text, fill) = if fx.armed {
                    ("ARMED  出力許可中 (クリックで解除)", RED)
                } else {
                    ("SAFE  出力停止中 (クリックでARM)", Color32::from_rgb(45, 110, 60))
                };
                if ui.add(egui::Button::new(RichText::new(text).strong().color(Color32::WHITE)).fill(fill)).clicked() {
                    fx.armed = !fx.armed;
                }
                ui.label(RichText::new("Class 3B: 人・動物・反射物に向けない / 床から3m以上").color(WARN).small());
            });
        }
        if let Some(v) = view {
            let inv = if v.inverted.is_empty() { String::new() } else { format!("  反転: {}", v.inverted.join(", ")) };
            ui.label(RichText::new(format!("ペア: {} に連動中{inv}", v.leader)).color(Color32::from_rgb(200, 110, 150)).strong());
        }
        let cats = mode.categories();
        let multi = cats.len() > 1;
        let pan_tilt = mode.find_attr("pan").zip(mode.find_attr("tilt"));
        let pixels = mode.pixels();
        let pixel_cat = pixels.first().and_then(|p| p.red.or(p.green).or(p.blue).or(p.white)).map(|ch| mode.channels[ch].category_name().to_string());
        for cat in cats {
            if hidden.contains(&cat) {
                continue;
            }
            if multi {
                ui.add_space(2.0);
                ui.label(RichText::new(format!("■ {cat}")).strong().color(category_color(&cat)));
            }
            // パン/チルトがある機材は、そのカテゴリの先頭にXYパッドを出す
            if let Some((pc, tc)) = pan_tilt.filter(|(pc, _)| mode.channels[*pc].category_name() == cat) {
                let pair = view.filter(|v| v.mask.get(pc).copied().unwrap_or(false) || v.mask.get(tc).copied().unwrap_or(false));
                let shown = pair.map(|v| v.values.clone()).unwrap_or_else(|| fx.values.clone());
                if let Some((p, t)) = xy_pad(ui, mode, &shown, pc, tc, pair.is_some()) {
                    mode.set_f(&mut fx.values, pc, p);
                    mode.set_f(&mut fx.values, tc, t);
                    touched.push((fx.id, pc));
                    touched.push((fx.id, tc));
                }
            }
            // LEDバー: ピクセルの色は、1本の帯としてまとめて表示・操作する
            if pixel_cat.as_deref() == Some(cat.as_str()) {
                let locked = view.is_some_and(|v| v.mask.iter().any(|m| *m));
                pixel_panel(ui, fx, mode, &pixels, out, locked, touched);
            }
            egui::Grid::new(("ch", fx.id, cat.clone())).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                for ci in 0..mode.channels.len() {
                    if mode.is_fine(ci) || mode.channels[ci].category_name() != cat {
                        continue;
                    }
                    if !pixels.is_empty() && crate::fixture::split_pixel(&mode.channels[ci].attr).1.is_some() {
                        continue;
                    }
                    match view.filter(|v| v.mask.get(ci).copied().unwrap_or(false)) {
                        Some(v) => {
                            let mut shown = v.values.clone();
                            channel_row(ui, fx.id, &mut shown, mode, ci, true, &mut Vec::new());
                        }
                        None => channel_row(ui, fx.id, &mut fx.values, mode, ci, false, touched),
                    }
                }
            });
        }
    });
    ui.add_space(6.0);
}

/// LEDバーのピクセル: 今の出力(エフェクト込み)の見本、全体の色、ピクセルごとの色
fn pixel_panel(ui: &mut egui::Ui, fx: &mut FixtureInstance, mode: &Mode, pixels: &[crate::fixture::Pixel], out: Option<&[u8]>, locked: bool, touched: &mut Vec<(u32, usize)>) {
    let n = pixels.len();
    let touch = |p: &crate::fixture::Pixel, touched: &mut Vec<(u32, usize)>| {
        for ch in [p.red, p.green, p.blue, p.white].into_iter().flatten() {
            touched.push((fx.id, ch));
        }
    };
    ui.horizontal(|ui| {
        ui.label(format!("ピクセル {n}"));
        // 出力の見本(エフェクト・全体光量・ブラックアウト込み)
        let cell = ((ui.available_width() - 160.0) / n.max(1) as f32).clamp(6.0, 22.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(cell * n as f32, 18.0), egui::Sense::hover());
        let src = out.filter(|o| o.len() == mode.channels.len()).unwrap_or(&fx.values);
        for (k, p) in pixels.iter().enumerate() {
            let r = egui::Rect::from_min_size(rect.min + egui::vec2(k as f32 * cell, 0.0), egui::vec2(cell - 1.0, 18.0));
            ui.painter().rect_filled(r, 2.0, rgb32(mode.pixel_rgb(src, p)));
        }
        ui.painter().rect_stroke(rect.expand(1.0), 2.0, egui::Stroke::new(1.0, Color32::from_gray(90)), egui::StrokeKind::Outside);
        ui.add_enabled_ui(!locked, |ui| {
            ui.label("全体");
            let mut all = pixels.first().map(|p| mode.pixel_rgb(&fx.values, p)).unwrap_or([0.0; 3]);
            if ui.color_edit_button_rgb(&mut all).on_hover_text("全ピクセルを同じ色にします").changed() {
                for p in pixels {
                    mode.set_pixel(&mut fx.values, p, all);
                    touch(p, touched);
                }
            }
        });
    })
    .response
    .on_hover_text("帯は今の出力(ピクセルエフェクト込み)。画面の更新は操作時と拍ごとなので、速い動きは間引いて見えます");
    ui.add_enabled_ui(!locked, |ui| {
        egui::CollapsingHeader::new("ピクセルごとの色").id_salt(("px", fx.id)).default_open(false).show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                for (k, p) in pixels.iter().enumerate() {
                    let mut c = mode.pixel_rgb(&fx.values, p);
                    ui.label(RichText::new(format!("{}", k + 1)).weak().small());
                    if ui.color_edit_button_rgb(&mut c).changed() {
                        mode.set_pixel(&mut fx.values, p, c);
                        touch(p, touched);
                    }
                }
            });
        });
    });
}

/// パン(左右)/チルト(上下)を1つの面で動かす。動かしたら新しい(パン, チルト)を返す(値は0〜255.99、16bitの下位も含む)
fn xy_pad(ui: &mut egui::Ui, mode: &Mode, vals: &[u8], pc: usize, tc: usize, locked: bool) -> Option<(f32, f32)> {
    const MAXV: f32 = 255.0 + 255.0 / 256.0;
    let sense = if locked { egui::Sense::hover() } else { egui::Sense::click_and_drag() };
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(180.0, 180.0), sense);
    let resp = resp.on_hover_text(if locked {
        "ペアで連動中のため、ここでは動かせません"
    } else {
        "ドラッグでパン(左右)とチルト(上下)。Shiftを押しながらドラッグすると細かく動きます"
    });
    let (w, h) = (rect.width(), rect.height());
    let pan = mode.get_f(vals, pc);
    let tilt = mode.get_f(vals, tc);

    let mut out = None;
    if !locked && (resp.dragged() || resp.clicked()) {
        let fine = ui.input(|i| i.modifiers.shift);
        let new = if fine {
            let d = resp.drag_delta();
            Some((pan + d.x / w * MAXV * 0.1, tilt + d.y / h * MAXV * 0.1))
        } else {
            resp.interact_pointer_pos().map(|p| ((p.x - rect.left()) / w * MAXV, (p.y - rect.top()) / h * MAXV))
        };
        out = new.map(|(p, t)| (p.clamp(0.0, MAXV), t.clamp(0.0, MAXV)));
    }
    let (pan, tilt) = out.unwrap_or((pan, tilt));

    let p = ui.painter();
    p.rect_filled(rect, 4.0, Color32::from_gray(28));
    for k in 1..4 {
        let f = k as f32 / 4.0;
        let c = if k == 2 { Color32::from_gray(70) } else { Color32::from_gray(48) };
        p.line_segment([egui::pos2(rect.left() + w * f, rect.top()), egui::pos2(rect.left() + w * f, rect.bottom())], egui::Stroke::new(1.0, c));
        p.line_segment([egui::pos2(rect.left(), rect.top() + h * f), egui::pos2(rect.right(), rect.top() + h * f)], egui::Stroke::new(1.0, c));
    }
    let dot = egui::pos2(rect.left() + pan / MAXV * w, rect.top() + tilt / MAXV * h);
    let accent = if locked { Color32::from_gray(140) } else { category_color("動き") };
    p.line_segment([egui::pos2(rect.left(), dot.y), egui::pos2(rect.right(), dot.y)], egui::Stroke::new(1.0, accent.gamma_multiply(0.5)));
    p.line_segment([egui::pos2(dot.x, rect.top()), egui::pos2(dot.x, rect.bottom())], egui::Stroke::new(1.0, accent.gamma_multiply(0.5)));
    p.circle_filled(dot, 7.0, accent);
    p.circle_stroke(dot, 7.0, egui::Stroke::new(1.5, Color32::WHITE));
    p.text(rect.left_bottom() + egui::vec2(5.0, -4.0), egui::Align2::LEFT_BOTTOM, format!("Pan {pan:.0}  Tilt {tilt:.0}"), egui::FontId::monospace(11.0), Color32::from_gray(200));
    p.text(rect.left_top() + egui::vec2(5.0, 3.0), egui::Align2::LEFT_TOP, "XY", egui::FontId::proportional(11.0), Color32::from_gray(120));
    out
}

fn channel_row(ui: &mut egui::Ui, fx_id: u32, values: &mut [u8], mode: &Mode, ci: usize, locked: bool, touched: &mut Vec<(u32, usize)>) {
    let ch = &mode.channels[ci];
    ui.label(&ch.name);
    ui.horizontal(|ui| {
        ui.add_enabled_ui(!locked, |ui| {
        let mut changed = false;
        if !ch.ranges.is_empty() {
            let mut v = values[ci];
            let text = ch.range_of(v).map(|r| r.label.clone()).unwrap_or_default();
            egui::ComboBox::from_id_salt(format!("cb{fx_id}_{ci}")).selected_text(text).width(250.0).show_ui(ui, |ui| {
                // 色を持つ範囲が1つでもあれば、選択肢の左に色見本を出す
                let any_color = ch.ranges.iter().any(|r| crate::color::range_rgb(r).is_some());
                // 禁止の範囲(レーザーのオートショーなど)は出さない
                for r in ch.ranges.iter().filter(|r| !ch.forbids_range(r)) {
                    let mut label = RichText::new(&r.label);
                    if r.warn {
                        label = label.color(WARN);
                    }
                    let clicked = ui
                        .horizontal(|ui| {
                            if any_color {
                                match crate::color::range_rgb(r) {
                                    Some(c) => ui.label(RichText::new("■").color(Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8))),
                                    None => ui.label(RichText::new("　")),
                                };
                            }
                            ui.selectable_label(r.contains(v), label).clicked()
                        })
                        .inner;
                    if clicked {
                        v = r.mid();
                        changed = true;
                    }
                }
            });
            // 速度など範囲内の値に意味があるレンジは、その範囲でスライダーを出す
            if let Some(r) = ch.range_of(v)
                && r.sweep && ui.add(Slider::new(&mut v, r.from..=r.to)).changed() {
                changed = true;
            }
            values[ci] = v;
        } else if let Some(fi) = mode.fine_of(ci) {
            let mut v16 = ((values[ci] as u32) << 8) | values[fi] as u32;
            if ui.add(Slider::new(&mut v16, 0..=65535)).changed() {
                values[ci] = (v16 >> 8) as u8;
                values[fi] = (v16 & 0xFF) as u8;
                changed = true;
            }
        } else {
            let mut v = values[ci];
            if ui.add(Slider::new(&mut v, 0..=255)).changed() {
                values[ci] = v;
                changed = true;
            }
        }
        if changed {
            touched.push((fx_id, ci));
        }
        });
    });
    ui.end_row();
}
