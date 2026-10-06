//! シーンタブ / エフェクトタブ。

use crate::app::{selector_combo, App, SelMeta};
use crate::engine::lock;
use crate::project::*;
use eframe::egui::{self, RichText};

/// エフェクトの速さの選択肢(0=自由、それ以外は1周期の拍数)
const BEAT_PRESETS: [f32; 8] = [0.0, 16.0, 8.0, 4.0, 2.0, 1.0, 0.5, 0.25];

fn beats_label(b: f32) -> String {
    if b <= 0.0 { "自由(Hz)".into() } else { format!("BPM同期: {b}拍で1周期") }
}

enum SceneOp {
    Recall(String),
    Update(usize),
    Delete(usize),
}

impl App {
    pub fn tab_scenes(&mut self, ui: &mut egui::Ui) {
        let shared = self.core.clone();
        let mut guard = lock(&shared);
        let core = &mut *guard;
        let mut op: Option<SceneOp> = None;
        let mut changed = false;

        ui.label(RichText::new("シーン = 選んだ機材の現在のチャンネル値を保存したもの。MIDIパッドに割り当てて呼び出せます").weak());
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.scene_name).hint_text("シーン名").desired_width(160.0));
            ui.label("フェード");
            ui.add(egui::DragValue::new(&mut self.scene_fade).range(0.0..=120.0).speed(0.05).suffix(" 秒"));
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("保存する機材:");
            for f in &core.project.fixtures {
                let mut on = !self.scene_skip.contains(&f.id);
                if ui.checkbox(&mut on, &f.name).changed() {
                    if on {
                        self.scene_skip.remove(&f.id);
                    } else {
                        self.scene_skip.insert(f.id);
                    }
                }
            }
        });
        let cats = core.present_categories();
        if cats.len() > 1 {
            ui.horizontal_wrapped(|ui| {
                ui.label("保存する項目:");
                for c in &cats {
                    let mut on = !self.scene_skip_cats.contains(c);
                    if ui.checkbox(&mut on, c.as_str()).changed() {
                        if on {
                            self.scene_skip_cats.remove(c);
                        } else {
                            self.scene_skip_cats.insert(c.clone());
                        }
                    }
                }
                ui.label(RichText::new("(「動きだけ」「色だけ」のシーンを作れます)").weak());
            });
        }
        if ui.button("＋ 現在の状態をシーンとして保存").clicked() && !self.scene_name.trim().is_empty() {
            let ids: Vec<u32> = core.project.fixtures.iter().map(|f| f.id).filter(|id| !self.scene_skip.contains(id)).collect();
            let name = self.scene_name.trim().to_string();
            let scene = core.capture_scene(&name, self.scene_fade, &ids, &self.scene_skip_cats);
            match core.project.scenes.iter_mut().find(|s| s.name == name) {
                Some(s) => *s = scene,
                None => core.project.scenes.push(scene),
            }
            self.scene_name.clear();
            changed = true;
        }
        ui.separator();

        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            if core.project.scenes.is_empty() {
                ui.label("シーンはまだありません。");
            }
            egui::Grid::new("scenes").striped(true).spacing([12.0, 6.0]).show(ui, |ui| {
                for (i, s) in core.project.scenes.iter_mut().enumerate() {
                    changed |= ui.add(egui::TextEdit::singleline(&mut s.name).desired_width(160.0)).changed();
                    changed |= ui.add(egui::DragValue::new(&mut s.fade).range(0.0..=120.0).speed(0.05).suffix(" 秒")).changed();
                    ui.label(RichText::new(format!("{}項目", s.values.len())).weak());
                    if ui.button("呼び出し").clicked() {
                        op = Some(SceneOp::Recall(s.name.clone()));
                    }
                    if ui.button("現在値で上書き").clicked() {
                        op = Some(SceneOp::Update(i));
                    }
                    if ui.button("削除").clicked() {
                        op = Some(SceneOp::Delete(i));
                    }
                    ui.end_row();
                }
            });
        });

        match op {
            Some(SceneOp::Recall(name)) => core.recall_scene(&name),
            Some(SceneOp::Update(i)) => {
                core.refresh_scene(i);
                changed = true;
            }
            Some(SceneOp::Delete(i)) => {
                core.project.scenes.remove(i);
                changed = true;
            }
            None => {}
        }
        if changed {
            core.dirty = true;
        }
    }

    pub fn tab_effects(&mut self, ui: &mut egui::Ui) {
        let shared = self.core.clone();
        let mut guard = lock(&shared);
        let core = &mut *guard;
        let lib = core.lib.clone();
        let meta = SelMeta::new(&core.project, &lib);
        let bpm = core.project.bpm;
        let mut changed = false;
        let mut remove: Option<usize> = None;

        ui.label(
            RichText::new("エフェクト = 対象チャンネルを 最小〜最大 の間で周期的に動かします(ベース値は上書きされます)。同じ名前のエフェクトは1つのパッドでまとめてON/OFFできます")
                .weak(),
        );
        ui.horizontal(|ui| {
            if ui.button("＋ エフェクト").clicked() {
                core.project.effects.push(Effect::new(&format!("エフェクト{}", core.project.effects.len() + 1), "pan"));
                changed = true;
            }
            if ui.button("＋ 円運動 (Pan/Tilt)").clicked() {
                // ARMが必要な機材(レーザー)を除いた、Pan/Tiltを持つ最初の機種を対象にする
                let sel = core
                    .project
                    .fixtures
                    .iter()
                    .find_map(|f| {
                        let p = lib.get(&f.profile)?;
                        let m = p.modes.get(f.mode)?;
                        (!p.needs_arm && m.find_attr("pan").is_some() && m.find_attr("tilt").is_some()).then(|| Selector::Profile(f.profile.clone()))
                    })
                    .unwrap_or(Selector::All);
                let mut a = Effect::new("円運動", "pan");
                a.sel = sel.clone();
                let mut b = Effect::new("円運動", "tilt");
                b.sel = sel;
                b.phase = 90.0;
                core.project.effects.push(a);
                core.project.effects.push(b);
                changed = true;
            }
        });
        ui.separator();

        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            for (i, e) in core.project.effects.iter_mut().enumerate() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut e.enabled, "有効").changed();
                        changed |= ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(160.0)).changed();
                        if ui.button("削除").clicked() {
                            remove = Some(i);
                        }
                    });
                    egui::Grid::new(("fx", i)).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                        ui.label("対象");
                        changed |= selector_combo(ui, &format!("fxsel{i}"), &mut e.sel, &meta);
                        ui.end_row();

                        ui.label("機能");
                        let mut attrs = meta.attrs(&lib, &e.sel);
                        if !attrs.contains(&e.attr) {
                            attrs.push(e.attr.clone());
                        }
                        egui::ComboBox::from_id_salt(format!("fxattr{i}")).selected_text(&e.attr).show_ui(ui, |ui| {
                            for a in attrs {
                                if ui.selectable_label(e.attr == a, &a).clicked() {
                                    e.attr = a;
                                    changed = true;
                                }
                            }
                        });
                        ui.end_row();

                        ui.label("波形");
                        egui::ComboBox::from_id_salt(format!("fxwave{i}")).selected_text(e.wave.label()).show_ui(ui, |ui| {
                            for w in Wave::ALL {
                                changed |= ui.selectable_value(&mut e.wave, w, w.label()).changed();
                            }
                        });
                        ui.end_row();

                        ui.label("速さ");
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt(format!("fxbeats{i}")).selected_text(beats_label(e.beats)).width(150.0).show_ui(ui, |ui| {
                                for b in BEAT_PRESETS {
                                    if ui.selectable_label((e.beats - b).abs() < 1e-6, beats_label(b)).clicked() {
                                        e.beats = b;
                                        changed = true;
                                    }
                                }
                            });
                            if e.beats > 0.0 {
                                changed |= ui.add(egui::DragValue::new(&mut e.beats).range(0.0625..=64.0).speed(0.05).suffix(" 拍")).changed();
                                ui.label(RichText::new(format!("= {:.2} Hz ({:.0} BPM)", bpm / 60.0 / e.beats.max(0.0625), bpm)).weak());
                            } else {
                                changed |= ui.add(egui::DragValue::new(&mut e.rate_hz).range(0.005..=20.0).speed(0.01).suffix(" Hz")).changed();
                            }
                        });
                        ui.end_row();

                        ui.label("最小 / 最大");
                        ui.horizontal(|ui| {
                            changed |= ui.add(egui::DragValue::new(&mut e.min).range(0.0..=255.0)).changed();
                            changed |= ui.add(egui::DragValue::new(&mut e.max).range(0.0..=255.0)).changed();
                        });
                        ui.end_row();

                        ui.label("位相 / 機材ごとのずれ");
                        ui.horizontal(|ui| {
                            changed |= ui.add(egui::DragValue::new(&mut e.phase).range(-360.0..=360.0).suffix("°")).changed();
                            changed |= ui.add(egui::DragValue::new(&mut e.spread).range(-360.0..=360.0).suffix("°")).changed();
                        });
                        ui.end_row();
                    });
                });
            }
            changed |= Self::pixel_fx_section(ui, core, &meta);
        });

        if let Some(i) = remove {
            core.project.effects.remove(i);
            changed = true;
        }
        if changed {
            core.dirty = true;
        }
    }

    /// LEDバー(ピクセルのある機材)用のエフェクト
    fn pixel_fx_section(ui: &mut egui::Ui, core: &mut crate::engine::Core, meta: &SelMeta) -> bool {
        use crate::pixel::{PixelColor, PixelFx, PixelKind};
        let has_bar = core.project.fixtures.iter().any(|f| core.lib.mode(&f.profile, f.mode).is_some_and(|m| !m.pixels().is_empty()));
        if !has_bar && core.project.pixel_fx.is_empty() {
            return false;
        }
        let mut changed = false;
        let mut remove = None;
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("ピクセルエフェクト(LEDバー)").strong().size(15.0));
            if ui.button("＋ ピクセルエフェクト").clicked() {
                let n = core.project.pixel_fx.len() + 1;
                core.project.pixel_fx.push(PixelFx { name: format!("ピクセル{n}"), ..PixelFx::default() });
                changed = true;
            }
            ui.menu_button("＋ メイン/サブの模様プリセット", |ui| {
                ui.label(RichText::new("メイン色とサブ色を混ぜる模様です(OFFの状態で追加)").weak());
                for (k, (name, kind, beats, width, mirror)) in crate::pixel::PRESETS.iter().enumerate() {
                    let speed = if *beats > 0.0 { format!("{beats}拍で1周") } else { "止まったまま".into() };
                    let extra = if *mirror { " / 中央から左右対称" } else { "" };
                    if ui.button(format!("{name}   ({} / {speed} / 幅{width}{extra})", kind.label())).clicked() {
                        if let Some(mut fx) = crate::pixel::from_preset(k) {
                            // 同じ名前があれば番号を付ける(同じ名前はまとめてON/OFFされるため)
                            let base = fx.name.clone();
                            let mut n = 2;
                            while core.project.pixel_fx.iter().any(|x| x.name == fx.name) {
                                fx.name = format!("{base}{n}");
                                n += 1;
                            }
                            core.project.pixel_fx.push(fx);
                            changed = true;
                        }
                        ui.close();
                    }
                }
                ui.separator();
                if ui.button("全部まとめて追加(同じ名前があるものは飛ばす)").clicked() {
                    changed |= crate::pixel::add_presets(&mut core.project, crate::pixel::PRESETS.len()) > 0;
                    ui.close();
                }
            });
        });
        ui.label(RichText::new("LEDバーの各ピクセルに、流れる色やレインボーを出します。色は「メイン色/サブ色」(入れ替えにも追従)か指定の色。速さはBPMに同期します。1本のバーに出る模様は1つずつ(ONにすると、同じバーの他の模様はOFFになります)").weak());
        let mut turned_on: Option<String> = None;
        let color_combo = |ui: &mut egui::Ui, salt: String, c: &mut PixelColor| -> bool {
            let mut ch = false;
            egui::ComboBox::from_id_salt(salt).selected_text(c.label()).width(90.0).show_ui(ui, |ui| {
                for o in PixelColor::CHOICES {
                    if ui.selectable_label(c.same_kind(o), o.label()).clicked() && !c.same_kind(o) {
                        *c = o;
                        ch = true;
                    }
                }
            });
            if let PixelColor::Custom(rgb) = c {
                ch |= ui.color_edit_button_rgb(rgb).changed();
            }
            ch
        };
        for (i, fx) in core.project.pixel_fx.iter_mut().enumerate() {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut fx.enabled, "有効").changed() {
                        changed = true;
                        if fx.enabled {
                            turned_on = Some(fx.name.clone());
                        }
                    }
                    changed |= ui.add(egui::TextEdit::singleline(&mut fx.name).desired_width(160.0)).changed();
                    if ui.button("削除").clicked() {
                        remove = Some(i);
                    }
                });
                egui::Grid::new(("pxfx", i)).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label("対象");
                    changed |= selector_combo(ui, &format!("pxsel{i}"), &mut fx.sel, meta);
                    ui.end_row();

                    ui.label("種類");
                    egui::ComboBox::from_id_salt(format!("pxkind{i}")).selected_text(fx.kind.label()).show_ui(ui, |ui| {
                        for k in PixelKind::ALL {
                            changed |= ui.selectable_value(&mut fx.kind, k, k.label()).changed();
                        }
                    });
                    ui.end_row();

                    if fx.kind.uses_colors() {
                        ui.label("色A / 色B");
                        ui.horizontal(|ui| {
                            changed |= color_combo(ui, format!("pxa{i}"), &mut fx.a);
                            ui.label("/");
                            changed |= color_combo(ui, format!("pxb{i}"), &mut fx.b);
                        });
                        ui.end_row();
                    }

                    ui.label("速さ");
                    ui.horizontal(|ui| {
                        let label = |b: f32| if b <= 0.0 { "止めたまま".to_string() } else { format!("{b}拍で1周") };
                        egui::ComboBox::from_id_salt(format!("pxbeats{i}")).selected_text(label(fx.beats)).width(110.0).show_ui(ui, |ui| {
                            for b in [0.0, 16.0, 8.0, 4.0, 2.0, 1.0, 0.5] {
                                if ui.selectable_label((fx.beats - b).abs() < 1e-6, label(b)).clicked() {
                                    fx.beats = b;
                                    changed = true;
                                }
                            }
                        });
                        if fx.beats > 0.0 {
                            changed |= ui.add(egui::DragValue::new(&mut fx.beats).range(0.125..=64.0).speed(0.05).suffix(" 拍")).changed();
                        }
                    });
                    ui.end_row();

                    if let Some(w) = fx.kind.width_label() {
                        ui.label(w);
                        changed |= ui.add(egui::DragValue::new(&mut fx.width).range(0.0..=64.0).speed(0.1)).changed();
                        ui.end_row();
                    }

                    ui.label("向き・つなぎ方");
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut fx.reverse, "逆向き").changed();
                        changed |= ui.checkbox(&mut fx.span, "複数台を1本につなげる").on_hover_text("OFFなら、各台で同じ動きをします").changed();
                        changed |= ui.checkbox(&mut fx.mirror, "中央から左右対称").on_hover_text("模様が中央から外へ広がります").changed();
                    });
                    ui.end_row();
                });
            });
        }
        if let Some(name) = turned_on {
            core.set_pixel_fx(&name, true);
        }
        if let Some(i) = remove {
            core.project.pixel_fx.remove(i);
            changed = true;
        }
        changed
    }
}
