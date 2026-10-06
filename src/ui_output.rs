//! 出力タブ(Art-Net / USB-DMX)とモニタータブ。

use crate::app::{App, RED, WARN};
use crate::engine::lock;
use crate::opendmx;
use crate::project::*;
use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense};

impl App {
    pub fn tab_output(&mut self, ui: &mut egui::Ui) {
        let shared = self.core.clone();
        let mut guard = lock(&shared);
        let core = &mut *guard;
        let mut apply = false;
        let mut remove: Option<usize> = None;
        // USB機器の検出は時間がかかることがあるため、共有ロックを離してから行う(その間もDMX出力を止めない)
        let (mut add_serial, mut add_d2xx, mut refresh_serial, mut refresh_d2xx) = (false, false, false, false);

        ui.horizontal(|ui| {
            add_serial = ui.button("＋ USB-DMX (COMポート)").clicked();
            add_d2xx = ui.button("＋ USB-DMX (D2XX)").clicked();
            if ui.button("＋ Art-Net").clicked() {
                core.project.outputs.push(OutputCfg {
                    enabled: true,
                    universe: 0,
                    kind: OutputKind::ArtNet { ip: "255.255.255.255".into(), art_universe: 0 },
                });
                apply = true;
            }
        });
        ui.label(
            RichText::new("FT232系の安価なUSB-DMXは、まず「COMポート」方式を試し、つながらなければ「D2XX」方式を使います。同時に複数の出力を有効にできます")
                .weak(),
        );
        ui.separator();

        if core.project.outputs.is_empty() {
            ui.colored_label(WARN, "出力が未設定です。上のボタンから追加してください。");
        }
        let statuses = core.rt.output_status.clone();

        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            for (i, o) in core.project.outputs.iter_mut().enumerate() {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        apply |= ui.checkbox(&mut o.enabled, "有効").changed();
                        ui.label(RichText::new(o.kind.label()).strong());
                        ui.label("送るユニバース:");
                        let mut u = o.universe as u32 + 1;
                        if ui.add(egui::DragValue::new(&mut u).range(1..=NUM_UNIVERSES as u32)).changed() {
                            o.universe = u as usize - 1;
                            apply = true;
                        }
                        if ui.button("削除").clicked() {
                            remove = Some(i);
                        }
                    });
                    match &mut o.kind {
                        OutputKind::ArtNet { ip, art_universe } => {
                            ui.horizontal(|ui| {
                                ui.label("宛先IP");
                                let r = ui.add(egui::TextEdit::singleline(ip).desired_width(140.0));
                                if r.lost_focus() {
                                    apply = true;
                                }
                                ui.label("Art-Netユニバース");
                                apply |= ui.add(egui::DragValue::new(art_universe).range(0..=32767)).changed();
                            });
                            ui.label(RichText::new("ノードのIPを指定するか、全体に流す場合は 255.255.255.255").weak());
                        }
                        OutputKind::OpenDmxSerial { port } => {
                            ui.horizontal(|ui| {
                                ui.label("ポート");
                                let cur = self.serial_ports.iter().find(|p| p.0 == *port).map(|p| p.1.clone()).unwrap_or_else(|| port.clone());
                                egui::ComboBox::from_id_salt(format!("ser{i}")).selected_text(cur).width(300.0).show_ui(ui, |ui| {
                                    for (name, desc) in &self.serial_ports {
                                        if ui.selectable_label(port == name, desc).clicked() {
                                            *port = name.clone();
                                            apply = true;
                                        }
                                    }
                                });
                                refresh_serial |= ui.button("再検出").clicked();
                            });
                        }
                        OutputKind::OpenDmxD2xx { index } => {
                            ui.horizontal(|ui| {
                                ui.label("デバイス");
                                match &self.d2xx {
                                    Ok(list) if !list.is_empty() => {
                                        let cur = list.get(*index as usize).cloned().unwrap_or_else(|| format!("{index}"));
                                        egui::ComboBox::from_id_salt(format!("d2{i}")).selected_text(cur).width(300.0).show_ui(ui, |ui| {
                                            for (n, d) in list.iter().enumerate() {
                                                if ui.selectable_label(*index as usize == n, d).clicked() {
                                                    *index = n as u32;
                                                    apply = true;
                                                }
                                            }
                                        });
                                    }
                                    Ok(_) => {
                                        ui.label(RichText::new("FTDIデバイスが見つかりません").color(WARN));
                                    }
                                    Err(e) => {
                                        ui.label(RichText::new(e.as_str()).color(RED));
                                    }
                                }
                                refresh_d2xx |= ui.button("再検出").clicked();
                            });
                        }
                    }
                    if let Some(s) = statuses.get(i) {
                        ui.label(RichText::new(format!("状態: {s}")).weak());
                    }
                });
            }
        });

        if let Some(i) = remove {
            core.project.outputs.remove(i);
            apply = true;
        }
        if apply {
            core.apply_outputs_changed();
        }
        drop(guard);

        if add_serial || refresh_serial {
            self.serial_ports = opendmx::list_serial_ports();
        }
        if add_d2xx || refresh_d2xx {
            self.d2xx = opendmx::list_d2xx();
        }
        if add_serial {
            let port = self.serial_ports.first().map(|p| p.0.clone()).unwrap_or_default();
            let mut c = lock(&shared);
            c.project.outputs.push(OutputCfg { enabled: true, universe: 0, kind: OutputKind::OpenDmxSerial { port } });
            c.apply_outputs_changed();
        }
        if add_d2xx {
            let mut c = lock(&shared);
            c.project.outputs.push(OutputCfg { enabled: true, universe: 0, kind: OutputKind::OpenDmxD2xx { index: 0 } });
            c.apply_outputs_changed();
        }
    }

    pub fn tab_monitor(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("ユニバース");
            for u in 0..NUM_UNIVERSES {
                ui.selectable_value(&mut self.monitor_universe, u, (u + 1).to_string());
            }
        });
        let (frame, used) = {
            let c = lock(&self.core);
            (c.rt.frames[self.monitor_universe], c.rt.used_len[self.monitor_universe])
        };
        ui.label(RichText::new(format!("実際に出力されている値(ブラックアウト/ARM/エフェクト適用後)。使用スロット: {used}")).weak());

        let cell = 27.0;
        let cols = 32usize;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(cols as f32 * cell, (512 / cols) as f32 * cell), Sense::hover());
        let painter = ui.painter_at(rect);
        for (i, &v) in frame.iter().enumerate() {
            let (x, y) = ((i % cols) as f32, (i / cols) as f32);
            let r = egui::Rect::from_min_size(rect.min + egui::vec2(x * cell, y * cell), egui::vec2(cell - 2.0, cell - 2.0));
            let g = 24 + (v as u32 * 100 / 255) as u8;
            painter.rect_filled(r, 2.0, Color32::from_rgb(g / 2, g, g / 2 + 20));
            if v > 0 {
                painter.text(r.center(), Align2::CENTER_CENTER, v.to_string(), FontId::monospace(10.0), Color32::WHITE);
            }
        }
        if let Some(p) = resp.hover_pos() {
            let (x, y) = (((p.x - rect.min.x) / cell) as usize, ((p.y - rect.min.y) / cell) as usize);
            let i = y * cols + x;
            if x < cols && i < 512 {
                resp.on_hover_text(format!("ch {} = {}", i + 1, frame[i]));
            }
        }
    }
}
