//! 出力タブ(Art-Net / USB-DMX)とモニタータブ。

use crate::ui::app::{App, RED, WARN};
use crate::core::engine::lock;
use crate::core::io::opendmx;
use crate::core::project::*;
use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense};

impl App {
    pub fn tab_output(&mut self, ui: &mut egui::Ui) {
        if !self.host.output_enabled() {
            ui.colored_label(WARN, "シミュレーション中です。出力先の設定を変更しても、実機には送信しません。");
        }
        let shared = self.host.core.clone();
        let mut guard = lock(&shared);
        let core = &mut *guard;
        let mut apply = false;
        let mut remove: Option<usize> = None;
        // USB機器の検出は時間がかかることがあるため、共有ロックを離してから行う(その間もDMX出力を止めない)
        let (mut add_serial, mut add_d2xx, mut refresh_serial, mut refresh_d2xx) = (false, false, false, false);

        ui.horizontal(|ui| {
            crate::ui::theme::heading(ui, "出力");
            crate::ui::theme::help(ui, "FT232系の安価なUSB-DMXは、まず「COMポート」方式を試し、つながらなければ「D2XX」方式を使います。同時に複数の出力を有効にできます。\nArt-Netは、ノードのIPを指定するか、全体に流す場合は 255.255.255.255");
            ui.add_space(8.0);
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
        ui.add_space(6.0);

        if core.project.outputs.is_empty() {
            ui.colored_label(WARN, "出力が未設定です。上のボタンから追加してください。");
        }
        let statuses = core.rt.output_status.clone();
        let dim = |t: &str| RichText::new(t).small().color(crate::ui::theme::TEXT_DIM);

        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            crate::ui::theme::card().show(ui, |ui| {
                egui::Grid::new("outputs").num_columns(7).spacing([14.0, 8.0]).striped(true).show(ui, |ui| {
                    ui.label(dim("有効"));
                    ui.label(dim("種類"));
                    ui.label(dim("ユニバース"));
                    ui.label(dim("送り先"));
                    ui.label(dim("Art-Net U"));
                    ui.label(dim("状態"));
                    ui.label("");
                    ui.end_row();
                    for (i, o) in core.project.outputs.iter_mut().enumerate() {
                        apply |= ui.checkbox(&mut o.enabled, "").changed();
                        ui.label(RichText::new(o.kind.label()).strong());
                        let mut u = o.universe as u32 + 1;
                        if ui.add(egui::DragValue::new(&mut u).range(1..=NUM_UNIVERSES as u32)).on_hover_text("このアプリの何番のユニバースを送るか").changed() {
                            o.universe = u as usize - 1;
                            apply = true;
                        }
                        match &mut o.kind {
                            OutputKind::ArtNet { ip, art_universe } => {
                                let r = ui.add(egui::TextEdit::singleline(ip).desired_width(140.0)).on_hover_text("ノードのIP。全体に流す場合は 255.255.255.255");
                                if r.lost_focus() {
                                    apply = true;
                                }
                                apply |= ui.add(egui::DragValue::new(art_universe).range(0..=32767)).changed();
                            }
                            OutputKind::OpenDmxSerial { port } => {
                                ui.horizontal(|ui| {
                                    let cur = self.serial_ports.iter().find(|p| p.0 == *port).map(|p| p.1.clone()).unwrap_or_else(|| port.clone());
                                    egui::ComboBox::from_id_salt(format!("ser{i}")).selected_text(cur).width(240.0).show_ui(ui, |ui| {
                                        for (name, desc) in &self.serial_ports {
                                            if ui.selectable_label(port == name, desc).clicked() {
                                                *port = name.clone();
                                                apply = true;
                                            }
                                        }
                                    });
                                    refresh_serial |= ui.small_button("再検出").clicked();
                                });
                                ui.label("");
                            }
                            OutputKind::OpenDmxD2xx { index } => {
                                ui.horizontal(|ui| {
                                    match &self.d2xx {
                                        Ok(list) if !list.is_empty() => {
                                            let cur = list.get(*index as usize).cloned().unwrap_or_else(|| format!("{index}"));
                                            egui::ComboBox::from_id_salt(format!("d2{i}")).selected_text(cur).width(240.0).show_ui(ui, |ui| {
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
                                            ui.label(RichText::new(e.as_str()).color(RED)).on_hover_text(e.as_str());
                                        }
                                    }
                                    refresh_d2xx |= ui.small_button("再検出").clicked();
                                });
                                ui.label("");
                            }
                        }
                        let st = statuses.get(i).cloned().unwrap_or_default();
                        let color = if st == "無効" || st.is_empty() {
                            crate::ui::theme::TEXT_FAINT
                        } else if ["エラー", "失敗", "見つかりません", "開けません"].iter().any(|k| st.contains(k)) {
                            WARN
                        } else {
                            crate::ui::theme::OK
                        };
                        ui.horizontal(|ui| {
                            crate::ui::theme::dot(ui, color);
                            ui.add_sized([170.0, 18.0], egui::Label::new(RichText::new(&st).small().color(crate::ui::theme::TEXT_DIM)).truncate()).on_hover_text(&st);
                        });
                        if ui.small_button("削除").clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
            });
        });

        if let Some(i) = remove {
            core.project.outputs.remove(i);
            apply = true;
        }
        if apply {
            core.apply_outputs_changed();
        }
        drop(guard);

        if self.host.is_client() {
            // 出力はサーバーのPCで開くので、サーバーの一覧を頼み直す
            if refresh_serial || refresh_d2xx {
                self.host.request_devices();
            }
        } else {
            if add_serial || refresh_serial {
                self.serial_ports = opendmx::list_serial_ports();
            }
            if add_d2xx || refresh_d2xx {
                self.d2xx = opendmx::list_d2xx();
            }
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
        use crate::ui::theme;
        let u = self.monitor_universe;
        // チャンネルごとに、どの機材のどのチャンネルか
        let (frame, used, slots, in_use) = {
            let c = lock(&self.host.core);
            let mut slots: Vec<Option<Slot>> = vec![None; 512];
            for (k, f) in c.project.fixtures.iter().enumerate().filter(|(_, f)| f.universe == u && f.address > 0) {
                let Some(m) = c.lib.mode(&f.profile, f.mode) else { continue };
                for (ci, ch) in m.channels.iter().enumerate() {
                    let a = f.address as usize - 1 + ci;
                    if a < 512 {
                        let color = attr_color(crate::core::fixture::split_pixel(&ch.attr).0).unwrap_or(FIXTURE_COLORS[k % FIXTURE_COLORS.len()]);
                        let pixel = crate::core::fixture::split_pixel(&ch.attr).1;
                        slots[a] = Some(Slot { fixture: f.id, name: f.name.clone(), channel: ch.name.clone(), color, band: FIXTURE_COLORS[k % FIXTURE_COLORS.len()], pixel });
                    }
                }
            }
            let in_use: Vec<bool> = (0..NUM_UNIVERSES).map(|u| c.project.fixtures.iter().any(|f| f.universe == u) || c.project.outputs.iter().any(|o| o.enabled && o.universe == u)).collect();
            (c.rt.frames[u], c.rt.used_len[u], slots, in_use)
        };

        ui.horizontal(|ui| {
            theme::heading(ui, "モニター");
            theme::help(ui, "実際に出力されている値です(全体光量・ブラックアウト・ARM・エフェクト適用後)。バーにマウスを乗せると、チャンネル番号と値が出ます");
            ui.add_space(10.0);
            ui.label(RichText::new("ユニバース").small().color(theme::TEXT_DIM));
            // 使っているユニバースだけを並べる(32個あるので)
            for (k, used) in in_use.iter().enumerate() {
                if !*used && k != u {
                    continue;
                }
                let text = RichText::new((k + 1).to_string()).color(if *used || k == u { theme::TEXT } else { theme::TEXT_FAINT });
                ui.selectable_value(&mut self.monitor_universe, k, text);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.monitor_all, "512ch すべて");
                ui.label(RichText::new(format!("使用 {used}ch")).small().color(theme::TEXT_DIM));
            });
        });
        ui.add_space(6.0);

        // 1段に並べるチャンネル数(16の倍数)と、1本の幅
        let avail = ui.available_width() - 12.0;
        let cols = (((avail / 22.0) as usize) / 16 * 16).clamp(16, 64);
        let cell = avail / cols as f32;
        let shown = if self.monitor_all { 512 } else { used.max(1).div_ceil(cols) * cols }.min(512);
        let (band_h, fader_h, num_h, row_gap) = (16.0, 72.0, 14.0, 12.0);
        let row_h = band_h + fader_h + num_h + row_gap;

        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let rows = shown.div_ceil(cols);
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(avail, rows as f32 * row_h), Sense::hover());
            let p = ui.painter_at(rect);
            for row in 0..rows {
                let y0 = rect.min.y + row as f32 * row_h;
                let first = row * cols;
                // 機材の名前の帯(同じ機材のチャンネルをまとめる)
                let mut k = first;
                while k < (first + cols).min(512) {
                    let Some(s) = &slots[k] else {
                        k += 1;
                        continue;
                    };
                    let start = k;
                    while k < (first + cols).min(512) && slots[k].as_ref().is_some_and(|x| x.fixture == s.fixture) {
                        k += 1;
                    }
                    let x0 = rect.min.x + (start - first) as f32 * cell;
                    let band = egui::Rect::from_min_size(egui::pos2(x0 + 1.0, y0), egui::vec2((k - start) as f32 * cell - 2.0, band_h - 3.0));
                    p.rect_filled(band, 4.0, theme::alpha(s.band, 0.22));
                    // LEDバーは、この段に並んでいるピクセルの番号も出す
                    let label = match (slots[start].as_ref().and_then(|x| x.pixel), slots[k - 1].as_ref().and_then(|x| x.pixel)) {
                        (Some(a), Some(b)) => format!("{}  ピクセル {a}〜{b}", s.name),
                        _ => s.name.clone(),
                    };
                    p.with_clip_rect(band).text(band.left_center() + egui::vec2(5.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(10.5), theme::mix(s.band, Color32::WHITE, 0.45));
                }
                // フェーダー
                for c in 0..cols {
                    let ch = first + c;
                    if ch >= 512 {
                        break;
                    }
                    let v = frame[ch];
                    let x = rect.min.x + c as f32 * cell;
                    let track = egui::Rect::from_min_size(egui::pos2(x + 2.0, y0 + band_h), egui::vec2((cell - 4.0).max(2.0), fader_h));
                    let slot = slots[ch].as_ref();
                    p.rect_filled(track, 3.0, if slot.is_some() { Color32::from_rgb(0x1B, 0x1F, 0x27) } else { Color32::from_rgb(0x15, 0x18, 0x1E) });
                    if v > 0 {
                        let h = fader_h * v as f32 / 255.0;
                        let fill = egui::Rect::from_min_max(egui::pos2(track.min.x, track.max.y - h), track.max);
                        let color = slot.map(|s| s.color).unwrap_or(Color32::from_gray(140));
                        p.rect_filled(fill, 3.0, color);
                        // つまみの位置の線
                        p.hline(fill.x_range(), fill.min.y, egui::Stroke::new(1.5, theme::mix(color, Color32::WHITE, 0.6)));
                    }
                    // チャンネル番号(機材の先頭と10の倍数は明るく)
                    let strong = ch % 10 == 9 || slot.is_some_and(|s| ch == 0 || slots[ch - 1].as_ref().is_none_or(|x| x.fixture != s.fixture));
                    let num_color = if strong { theme::TEXT_DIM } else { theme::TEXT_FAINT };
                    let size = if cell >= 24.0 { 9.5 } else { 8.5 };
                    p.text(egui::pos2(track.center().x, track.max.y + num_h / 2.0 + 1.0), Align2::CENTER_CENTER, (ch + 1).to_string(), FontId::monospace(size), num_color);
                }
            }
            if let Some(pos) = resp.hover_pos() {
                let (c, row) = (((pos.x - rect.min.x) / cell) as usize, ((pos.y - rect.min.y) / row_h) as usize);
                let ch = row * cols + c;
                if c < cols && ch < shown {
                    let v = frame[ch];
                    let who = slots[ch].as_ref().map(|s| format!("\n{} / {}", s.name, s.channel)).unwrap_or_else(|| "\n(未使用)".into());
                    resp.on_hover_text(format!("ch {} = {} ({:.0}%){who}", ch + 1, v, v as f32 / 2.55));
                }
            }
        });
    }
}

/// モニターの1チャンネル分の情報
#[derive(Clone)]
struct Slot {
    fixture: u32,
    name: String,
    channel: String,
    /// バーの色(赤・緑・青などのチャンネルはその色)
    color: Color32,
    /// 機材の名前の帯の色
    band: Color32,
    /// LEDバーのピクセル番号
    pixel: Option<u16>,
}

/// 機材ごとの色(順番に使う)
const FIXTURE_COLORS: [Color32; 8] = [
    Color32::from_rgb(0x5E, 0xE6, 0xD0),
    Color32::from_rgb(0x38, 0xBD, 0xF8),
    Color32::from_rgb(0xA7, 0x8B, 0xFA),
    Color32::from_rgb(0xF4, 0x72, 0xB6),
    Color32::from_rgb(0xFB, 0xBF, 0x24),
    Color32::from_rgb(0xA3, 0xE6, 0x35),
    Color32::from_rgb(0xFB, 0x92, 0x3C),
    Color32::from_rgb(0x22, 0xD3, 0xEE),
];

/// 色のチャンネルは、その色でバーを塗る
fn attr_color(attr: &str) -> Option<Color32> {
    Some(match attr {
        "red" => Color32::from_rgb(0xEF, 0x44, 0x44),
        "green" => Color32::from_rgb(0x22, 0xC5, 0x5E),
        "blue" => Color32::from_rgb(0x3B, 0x82, 0xF6),
        "white" => Color32::from_rgb(0xE2, 0xE8, 0xF0),
        "amber" => Color32::from_rgb(0xF5, 0x9E, 0x0B),
        "uv" => Color32::from_rgb(0x8B, 0x5C, 0xF6),
        _ => return None,
    })
}
