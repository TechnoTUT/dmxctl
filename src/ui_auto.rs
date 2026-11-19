//! オートタブ: オートのON/OFF・待機の色と、フレーズ(イントロ・サビ・ビルドアップ…)ごとの見た目のプリセット

use crate::engine::lock;
use crate::motion::{POSITIONS, RATES, SHAPES, SPREADS};
use crate::project::*;
use crate::theme;
use eframe::egui::{self, RichText};

/// 色の動き方の名前
fn color_move_label(c: ColorMove) -> &'static str {
    match c {
        ColorMove::Keep => "そのまま",
        ColorMove::Swap => "入れ替え",
        ColorMove::Fade => "グラデーション",
    }
}

/// 拍数の名前(1周の長さ・色の間隔)
fn beats_label(b: f32) -> String {
    RATES.iter().find(|(v, _)| (*v - b).abs() < 1e-3).map(|(_, l)| l.to_string()).unwrap_or_else(|| format!("{b}拍"))
}

impl crate::app::App {
    pub fn tab_auto(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            theme::heading(ui, "オート");
            theme::help(
                ui,
                "Beat Link Trigger などから曲の情報(色・明るさ・フレーズ)が届いている間は、照明が自動で動きます。\
                 届かなくなったら待機の色で光らせて待ちます。下のプリセットで、フレーズごとの動きを決めます",
            );
        });
        self.auto_row(ui);
        ui.add_space(8.0);
        ui.separator();
        self.preset_editor(ui);
    }

    /// オート: Beat Link Triggerなどから曲の情報が届いている間は自動で動かし、届かなければ待機の色で光らせる
    fn auto_row(&mut self, ui: &mut egui::Ui) {
        let (mut cfg, online, manual, energy, blt) = {
            let c = lock(&self.host.core);
            (c.project.auto.clone(), c.auto_online(), c.manual_remaining(), c.rt.energy, c.blt_state())
        };
        // BLTが見つからないときは、いちばん上に目立つように
        if cfg.enabled && matches!(blt, crate::engine::BltState::Missing(_)) {
            theme::card().fill(theme::alpha(theme::WARN, 0.15)).show(ui, |ui| {
                ui.label(RichText::new(blt.message()).color(theme::WARN));
                ui.label(RichText::new("Linkの仲間も、曲の情報(OSC)も届いていません。BLT が動いているか、このPCと同じネットワークにつながっているか、BLTのトリガーの送り先(IP・ポート)が合っているかを確かめてください").small().color(theme::TEXT_DIM));
            });
            ui.add_space(4.0);
        }
        let before = cfg.clone();
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut cfg.enabled, "オート").on_hover_text(
                "Beat Link Trigger などから色・明るさ・フレーズ(/dmx/main/rgb・/dmx/level・/dmx/phrase)が届いている間は、それに合わせて自動で動かします。\
                 届かなくなったら(DJ機器が止まった・つながっていない)、待機の色で光らせて待ちます",
            );
            if cfg.enabled {
                if let Some(rest) = manual {
                    ui.label(RichText::new(format!("✋ 手動優先(あと{rest:.0}秒)")).color(crate::theme::WARN));
                    if ui.small_button("オートに戻す").clicked() {
                        lock(&self.host.core).live(crate::live::LiveOp::ClearManual);
                    }
                } else {
                    ui.label(RichText::new(if online { "● 曲の情報を受信中" } else { "○ 待機中" }).color(if online { crate::theme::ACCENT } else { crate::theme::TEXT_DIM }));
                }
                if online {
                    ui.label(RichText::new(format!("盛り上がり {:.0}%", energy * 100.0)).weak());
                }
            }
            ui.label("待機の色");
            ui.color_edit_button_rgb(&mut cfg.idle_rgb);
            ui.label("明るさ");
            ui.add(egui::Slider::new(&mut cfg.idle_level, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
            ui.label("待機までの秒数");
            ui.add(egui::DragValue::new(&mut cfg.timeout_secs).range(1.0..=600.0).suffix(" 秒"));
            ui.checkbox(&mut cfg.builtin, "フレーズでプリセットを使う").on_hover_text("フレーズが届いたら、下のプリセットのその行の見た目(ムービング・色・明るさ・シーン・エフェクト)にします");
            ui.checkbox(&mut cfg.energy, "フレーズが無い曲は盛り上がりで").on_hover_text("フレーズの情報が来ない曲では、16拍ごとに波形の大きさ(/dmx/level)で見た目を選びます(大きい: サビ / 中くらい: バース / 小さい: ダウン)");
            ui.checkbox(&mut cfg.accent, "入った瞬間の全開").on_hover_text("プリセットで「全開」にしたフレーズに入った瞬間、1拍だけ今の色で全開にします(レーザーは除く)。OFFにすると全部のフレーズで止めます");
            ui.label("BLTが見つからないと知らせるまで");
            ui.add(egui::DragValue::new(&mut cfg.alert_secs).range(0.0..=3600.0).suffix(" 秒")).on_hover_text("Linkの仲間も曲の情報も、この秒数届かなければ、下の帯とログに「Beat Link Trigger が見つかりません」と出します(0で知らせない)");
            ui.label("手動優先");
            ui.add(egui::DragValue::new(&mut cfg.manual_secs).range(0.0..=3600.0).suffix(" 秒")).on_hover_text("MIDIコントローラー・キーボードで操作したら、この秒数はオートが照明を変えません(0で手動優先なし)。画面のクリックは数えません");
        });
        if cfg != before {
            let mut c = lock(&self.host.core);
            c.project.auto = cfg;
            c.dirty = true;
        }
    }

    /// フレーズごとの見た目のプリセット
    fn preset_editor(&mut self, ui: &mut egui::Ui) {
        let shared = self.host.core.clone();
        let mut guard = lock(&shared);
        let core = &mut *guard;
        let mut changed = false;
        if core.project.auto.presets.is_empty() {
            core.project.auto.presets = vec![AutoPreset::standard()];
        }
        let n = core.project.auto.presets.len();
        let mut idx = core.project.auto.preset.min(n - 1);

        // プリセットを選ぶ・増やす・消す
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("プリセット").font(theme::bold(14.0)));
            egui::ComboBox::from_id_salt("auto_preset").selected_text(core.project.auto.presets[idx].name.clone()).show_ui(ui, |ui| {
                for (i, p) in core.project.auto.presets.iter().enumerate() {
                    if ui.selectable_label(i == idx, &p.name).clicked() {
                        idx = i;
                    }
                }
            });
            ui.label("名前");
            changed |= ui.add(egui::TextEdit::singleline(&mut core.project.auto.presets[idx].name).desired_width(140.0)).changed();
            if ui.button("複製").on_hover_text("今のプリセットを写して、新しいプリセットを作ります").clicked() {
                let mut p = core.project.auto.presets[idx].clone();
                p.name = format!("{} のコピー", p.name);
                core.project.auto.presets.push(p);
                idx = core.project.auto.presets.len() - 1;
                changed = true;
            }
            ui.menu_button("見本から作る", |ui| {
                for p in [AutoPreset::standard(), AutoPreset::intense()] {
                    if ui.button(&p.name).clicked() {
                        core.project.auto.presets.push(p);
                        idx = core.project.auto.presets.len() - 1;
                        changed = true;
                        ui.close();
                    }
                }
            });
            if n > 1 && ui.button("削除").clicked() {
                core.project.auto.presets.remove(idx);
                idx = idx.saturating_sub(1);
                changed = true;
            }
        });
        if idx != core.project.auto.preset {
            core.project.auto.preset = idx;
            changed = true;
        }
        let idx = idx.min(core.project.auto.presets.len() - 1);
        ui.label(
            RichText::new(
                "フレーズが届くと、その行の見た目になります(ムービング・色・明るさ・シーン・エフェクト)。「試す」で今すぐその見た目にできます。\
                 「エフェクト」でONにしたものは、次のフレーズでOFFに戻ります。/dmx/phrase/<名前> に割り当てがあるフレーズは、そちらが優先です",
            )
            .small()
            .color(theme::TEXT_DIM),
        );
        ui.add_space(4.0);

        let scenes: Vec<String> = core.project.scenes.iter().map(|s| s.name.clone()).collect();
        let mut fx_names: Vec<String> = core.project.effects.iter().map(|e| e.name.clone()).chain(core.project.pixel_fx.iter().map(|e| e.name.clone())).chain(core.project.chases.iter().map(|c| c.name.clone())).collect();
        fx_names.dedup();
        let now_key = core.rt.auto_key.clone();
        let mut try_look: Option<(String, PhraseLook)> = None;

        egui::ScrollArea::both().auto_shrink([false, true]).show(ui, |ui| {
            egui::Grid::new("auto_looks").num_columns(14).spacing([10.0, 6.0]).striped(true).show(ui, |ui| {
                for h in ["フレーズ", "動き", "1周", "揃え方", "向き", "色", "色の間隔", "明るさ", "波形の色", "入った瞬間", "レーザー", "シーン", "エフェクト", ""] {
                    ui.label(RichText::new(h).small().color(theme::TEXT_DIM));
                }
                ui.end_row();
                for (key, ja) in PHRASES {
                    let preset = &mut core.project.auto.presets[idx];
                    let mut look = preset.looks.get(key).cloned().unwrap_or_default();
                    let before = look.clone();
                    let current = now_key == key;
                    ui.label(RichText::new(if current { format!("● {ja}") } else { ja.to_string() }).font(theme::bold(13.0)).color(if current { theme::ACCENT } else { theme::TEXT }));
                    egui::ComboBox::from_id_salt(("shape", key)).width(80.0).selected_text(SHAPES.iter().find(|s| s.0 == look.shape).map(|s| s.1).unwrap_or("?")).show_ui(ui, |ui| {
                        for (s, l) in SHAPES {
                            ui.selectable_value(&mut look.shape, s, l);
                        }
                    });
                    egui::ComboBox::from_id_salt(("beats", key)).width(70.0).selected_text(beats_label(look.beats)).show_ui(ui, |ui| {
                        for (b, l) in RATES {
                            ui.selectable_value(&mut look.beats, b, l);
                        }
                    });
                    egui::ComboBox::from_id_salt(("spread", key)).width(80.0).selected_text(SPREADS.iter().find(|s| s.0 == look.spread).map(|s| s.1).unwrap_or("?")).show_ui(ui, |ui| {
                        for (s, l) in SPREADS {
                            ui.selectable_value(&mut look.spread, s, l);
                        }
                    });
                    egui::ComboBox::from_id_salt(("pos", key)).width(70.0).selected_text(look.position.and_then(|p| POSITIONS.get(p as usize).copied()).unwrap_or("そのまま")).show_ui(ui, |ui| {
                        ui.selectable_value(&mut look.position, None, "そのまま");
                        for (i, l) in POSITIONS.iter().enumerate() {
                            ui.selectable_value(&mut look.position, Some(i as u8), *l);
                        }
                    });
                    egui::ComboBox::from_id_salt(("color", key)).width(100.0).selected_text(color_move_label(look.color)).show_ui(ui, |ui| {
                        for c in [ColorMove::Keep, ColorMove::Swap, ColorMove::Fade] {
                            ui.selectable_value(&mut look.color, c, color_move_label(c));
                        }
                    });
                    ui.add_enabled_ui(look.color != ColorMove::Keep, |ui| {
                        egui::ComboBox::from_id_salt(("cbeats", key)).width(70.0).selected_text(beats_label(look.color_beats)).show_ui(ui, |ui| {
                            for (b, l) in RATES.iter().rev() {
                                ui.selectable_value(&mut look.color_beats, *b, *l);
                            }
                        });
                    });
                    ui.add(egui::Slider::new(&mut look.brightness, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
                    ui.checkbox(&mut look.wave_color, "").on_hover_text("波形の色(/dmx/main/rgb)を使う。OFFならパレットの色のまま");
                    ui.checkbox(&mut look.flash, "全開").on_hover_text("このフレーズに入った瞬間、1拍だけ今の色で全開(レーザーは除く)");
                    ui.checkbox(&mut look.laser, "出力").on_hover_text("このフレーズの間のみ、ARM済みのレーザーを出力する");
                    egui::ComboBox::from_id_salt(("scene", key)).width(100.0).selected_text(if look.scene.is_empty() { "なし".to_string() } else { look.scene.clone() }).show_ui(ui, |ui| {
                        ui.selectable_value(&mut look.scene, String::new(), "なし");
                        for s in &scenes {
                            ui.selectable_value(&mut look.scene, s.clone(), s);
                        }
                    });
                    let fx_label = if look.effects.is_empty() { "なし".to_string() } else { look.effects.join(", ") };
                    ui.menu_button(fx_label, |ui| {
                        if fx_names.is_empty() {
                            ui.label("エフェクト・チェイスがまだありません");
                        }
                        for name in &fx_names {
                            let mut on = look.effects.contains(name);
                            if ui.checkbox(&mut on, name).changed() {
                                if on {
                                    look.effects.push(name.clone());
                                } else {
                                    look.effects.retain(|e| e != name);
                                }
                            }
                        }
                    });
                    if ui.small_button("試す").on_hover_text("今すぐこの見た目にします(オートがOFFでも)").clicked() {
                        try_look = Some((key.to_string(), look.clone()));
                    }
                    ui.end_row();
                    if look != before {
                        preset.looks.insert(key.to_string(), look);
                        changed = true;
                    }
                }
            });
        });
        if let Some((key, look)) = try_look {
            core.live(crate::live::LiveOp::Look { key, look });
        }
        if changed {
            core.dirty = true;
        }
    }
}
