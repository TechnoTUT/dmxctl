//! 上部の「全体光量・BPM」バーと、BPM/チェイスタブ。

use crate::app::{App, WARN};
use crate::engine::lock;
use crate::project::*;
use eframe::egui::{self, vec2, Color32, RichText, Sense};

enum StepOp {
    Up(usize, usize),
    Down(usize, usize),
    Del(usize, usize),
}

impl App {
    /// 上部バーの2段目: 全体光量スライダーと、BPM・TAP・頭出し・拍の表示
    pub fn transport_row(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().slider_width = 170.0;
            let mut pct = lock(&self.core).rt.master * 100.0;
            if ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).fixed_decimals(0).suffix("%").text("全体光量")).changed() {
                lock(&self.core).rt.master = pct / 100.0;
            }
            ui.separator();

            let (mut bpm, beat, src, peers, clock_ok) = {
                let c = lock(&self.core);
                (c.project.bpm, c.rt.beat, c.project.tempo_source, c.rt.link_peers, c.clock_alive(std::time::Instant::now()))
            };
            ui.label("BPM");
            // MIDIクロック中は、BPMはクロックで決まる
            let by_clock = src == TempoSource::MidiClock && clock_ok;
            if ui.add_enabled(!by_clock, egui::DragValue::new(&mut bpm).range(BPM_MIN..=BPM_MAX).speed(0.2).fixed_decimals(1)).changed() {
                lock(&self.core).set_bpm(bpm);
            }
            match src {
                TempoSource::Link => {
                    let text = RichText::new(format!("Link {peers}")).strong().color(if peers > 0 { Color32::from_rgb(80, 195, 115) } else { Color32::from_gray(150) });
                    ui.label(text).on_hover_text(format!("Ableton Link: 接続中のアプリ {peers} 個"));
                }
                TempoSource::MidiClock => {
                    let text = RichText::new("CLK").strong().color(if clock_ok { Color32::from_rgb(80, 195, 115) } else { WARN });
                    ui.label(text).on_hover_text(if clock_ok { "MIDIクロックを受信中" } else { "MIDIクロックが来ていません(内部のテンポで走っています)" });
                }
                TempoSource::Internal => {}
            }
            if ui.button("TAP").on_hover_text("リズムに合わせて叩くとBPMが決まり、拍の位置も合います").clicked() {
                lock(&self.core).tap();
            }
            if ui.button("頭出し").on_hover_text("拍の位置を小節の頭に戻します").clicked() {
                lock(&self.core).resync();
            }
            // 暗転フェード
            ui.separator();
            let (dim, target, secs) = {
                let c = lock(&self.core);
                (c.rt.dim, c.rt.dim_target, c.project.fade_secs)
            };
            let text = if target < 0.5 {
                if dim > 0.001 { format!("暗転中 {:.0}%", dim * 100.0) } else { "暗転中 - 戻す (D)".to_string() }
            } else if dim < 0.999 {
                format!("復帰中 {:.0}%", dim * 100.0)
            } else {
                "暗転 (D)".to_string()
            };
            let mut btn = egui::Button::new(RichText::new(text).color(if target < 0.5 { Color32::WHITE } else { ui.visuals().text_color() }));
            if target < 0.5 {
                btn = btn.fill(Color32::from_rgb(130, 70, 25));
            }
            if ui.add(btn).on_hover_text("全体の明るさを、右の秒数をかけて0にします(暗転しきるとレーザーも止まります)。もう一度押すと、同じ秒数で戻ります").clicked() {
                lock(&self.core).toggle_fade(secs);
            }
            let mut s = secs;
            if ui.add(egui::DragValue::new(&mut s).range(0.0..=60.0).speed(0.05).suffix(" 秒")).on_hover_text("暗転・復帰にかける時間").changed() {
                let mut c = lock(&self.core);
                c.project.fade_secs = s;
                c.dirty = true;
            }
            ui.separator();

            // 拍の表示(1拍目はオレンジ)
            let in_bar = (beat.floor() as i64).rem_euclid(BEATS_PER_BAR);
            for b in 0..BEATS_PER_BAR {
                let (rect, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                let lit = b == in_bar;
                let color = match (lit, b) {
                    (true, 0) => Color32::from_rgb(255, 150, 30),
                    (true, _) => Color32::from_rgb(80, 195, 115),
                    _ => Color32::from_gray(155),
                };
                ui.painter().circle_filled(rect.center(), if lit { 6.5 } else { 4.0 }, color);
            }

            // 色の操作先: 全体(全機材) ⇄ 個別(選んだ機材だけ)
            ui.separator();
            let (global, selected, fixtures) = {
                let c = lock(&self.core);
                (c.rt.color_global, c.rt.selected.clone(), c.project.fixtures.iter().map(|f| (f.id, f.name.clone())).collect::<Vec<_>>())
            };
            let (label, fill) = if global { ("色: 全体", Color32::from_rgb(45, 110, 60)) } else { ("色: 個別", Color32::from_rgb(190, 120, 30)) };
            let btn = egui::Button::new(RichText::new(label).strong().color(Color32::WHITE)).fill(fill);
            if ui.add(btn).on_hover_text("色の操作を、全機材(全体)にかけるか、選んだ機材だけ(個別)にかけるかを切り替えます").clicked() {
                lock(&self.core).rt.color_global = !global;
            }
            if !global {
                for (id, name) in fixtures {
                    if ui.selectable_label(selected.contains(&id), name).on_hover_text("クリックで選択/解除(個別モードで色をかける機材)").clicked() {
                        lock(&self.core).toggle_selected(id);
                    }
                }
            }
            ui.separator();
            let help: String = crate::app::SHORTCUTS.iter().map(|(k, d)| format!("{k}: {d}")).collect::<Vec<_>>().join("\n");
            ui.label(RichText::new("⌨ キー").weak()).on_hover_text(format!("キーボードショートカット(文字の入力中は Esc と保存だけ)\n\n{help}"));
        });
    }

    pub fn tab_sequence(&mut self, ui: &mut egui::Ui) {
        let shared = self.core.clone();
        let mut guard = lock(&shared);
        let core = &mut *guard;
        let scenes: Vec<String> = core.project.scenes.iter().map(|s| s.name.clone()).collect();
        let running = core.rt.chase_idx.clone();
        let mut changed = false;
        let mut remove: Option<usize> = None;
        let mut step_op: Option<StepOp> = None;
        let mut services = false;

        // ---- 同期(映像ソフトなどとテンポ・拍を揃える) ----
        ui.label(RichText::new("同期").strong().size(15.0));
        ui.horizontal(|ui| {
            ui.label("テンポの元");
            for s in TempoSource::ALL {
                if ui.radio(core.project.tempo_source == s, s.label()).clicked() && core.project.tempo_source != s {
                    core.set_tempo_source(s);
                    services = true;
                }
            }
        });
        match core.project.tempo_source {
            TempoSource::Internal => {
                ui.label(RichText::new("このアプリのBPM(TAP・手入力)で動きます").weak());
            }
            TempoSource::Link => {
                let peers = core.rt.link_peers;
                let (text, color) = if core.rt.link.is_none() {
                    ("Linkを開始できませんでした".to_string(), WARN)
                } else if peers == 0 {
                    ("接続中のアプリ: 0(相手のLinkをONにしてください)".to_string(), WARN)
                } else {
                    (format!("接続中のアプリ: {peers}"), Color32::from_rgb(80, 195, 115))
                };
                ui.colored_label(color, text);
                ui.label(
                    RichText::new(
                        "同じPC・同じネットワークのLink対応アプリ(TouchDesignerの Ableton Link CHOP、Resolume、Ableton Live など)と、テンポと拍の位置が揃います。\
                         ここでのTAP・BPM変更・頭出しは相手にも伝わります。初回はWindowsのファイアウォールの確認が出ます(別のPCとつなぐときは「許可」)",
                    )
                    .weak(),
                );
            }
            TempoSource::MidiClock => {
                ui.horizontal(|ui| {
                    ui.label("クロックの入力");
                    let main = "(メインのMIDI入力)";
                    let cur = if core.project.clock_input.is_empty() { main.to_string() } else { core.project.clock_input.clone() };
                    egui::ComboBox::from_id_salt("clock_port").selected_text(cur).width(260.0).show_ui(ui, |ui| {
                        if ui.selectable_label(core.project.clock_input.is_empty(), main).clicked() {
                            core.project.clock_input.clear();
                            services = true;
                        }
                        for p in &self.midi_ports {
                            if ui.selectable_label(core.project.clock_input == *p, p).clicked() {
                                core.project.clock_input = p.clone();
                                services = true;
                            }
                        }
                    });
                    if services {
                        changed = true;
                    }
                    let alive = core.clock_alive(std::time::Instant::now());
                    if alive {
                        ui.colored_label(Color32::from_rgb(80, 195, 115), format!("受信中 {:.1} BPM", core.project.bpm));
                    } else {
                        ui.colored_label(WARN, "受信なし(内部のテンポで走ります)");
                    }
                });
                ui.label(
                    RichText::new("映像ソフトから送るときは loopMIDI などの仮想MIDIポートを使います。Start(0xFA)で小節の頭に揃います。クロック中はBPMの手入力とTAPは効きません").weak(),
                );
            }
        }
        ui.separator();

        // ---- テンポ ----
        ui.label(RichText::new("テンポ").strong().size(15.0));
        ui.horizontal(|ui| {
            let mut bpm = core.project.bpm;
            ui.label("BPM");
            if ui.add(egui::DragValue::new(&mut bpm).range(BPM_MIN..=BPM_MAX).speed(0.2).fixed_decimals(1)).changed() {
                core.set_bpm(bpm);
            }
            if ui.button("TAP").clicked() {
                core.tap();
            }
            if ui.button("頭出し").clicked() {
                core.resync();
            }
            if ui.button("÷2").clicked() {
                core.set_bpm(core.project.bpm / 2.0);
            }
            if ui.button("×2").clicked() {
                core.set_bpm(core.project.bpm * 2.0);
            }
            let beat = core.rt.beat;
            ui.label(RichText::new(format!("{}小節 {}拍目", beat.floor() as i64 / BEATS_PER_BAR + 1, (beat.floor() as i64).rem_euclid(BEATS_PER_BAR) + 1)).weak());
        });
        ui.label(
            RichText::new("TAP・頭出し・BPM調整は、MIDIのパッドやフェーダーにも割り当てられます(マップ/MIDIタブの動作「テンポ」)。エフェクトは「速さ」を拍数にするとBPMに同期します")
                .weak(),
        );
        ui.separator();

        // ---- チェイス ----
        ui.label(RichText::new("チェイス").strong().size(15.0));
        ui.label(
            RichText::new("シーンをBPMに合わせて順番に呼び出します。「色だけ」「動きだけ」のシーン(シーンタブの「保存する項目」)を使うと、色と動きを別々のテンポで回せます")
                .weak(),
        );
        ui.horizontal(|ui| {
            if ui.button("＋ チェイスを追加").clicked() {
                let mut c = Chase { name: format!("チェイス{}", core.project.chases.len() + 1), ..Chase::default() };
                c.steps = scenes.clone();
                core.project.chases.push(c);
                changed = true;
            }
            if scenes.is_empty() {
                ui.colored_label(WARN, "先に「シーン」タブでシーンを作ってください");
            }
        });
        ui.add_space(4.0);

        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            for (ci, c) in core.project.chases.iter_mut().enumerate() {
                let now_step = running.get(&c.name).map(|idx| step_index(c.order, *idx, c.steps.len()));
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut c.enabled, "動作中").changed();
                        changed |= ui.add(egui::TextEdit::singleline(&mut c.name).desired_width(160.0)).changed();
                        if ui.button("削除").clicked() {
                            remove = Some(ci);
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label("1ステップ");
                        changed |= ui.add(egui::DragValue::new(&mut c.step_beats).range(0.0625..=64.0).speed(0.05).suffix(" 拍")).changed();
                        ui.label("フェード");
                        changed |= ui.add(egui::DragValue::new(&mut c.fade_beats).range(0.0..=64.0).speed(0.05).suffix(" 拍")).on_hover_text("シーン切り替えにかける長さ。0=カット").changed();
                        ui.label("順序");
                        egui::ComboBox::from_id_salt(format!("cord{ci}")).selected_text(c.order.label()).width(90.0).show_ui(ui, |ui| {
                            for o in ChaseOrder::ALL {
                                changed |= ui.selectable_value(&mut c.order, o, o.label()).changed();
                            }
                        });
                    });
                    let n = c.steps.len();
                    for si in 0..n {
                        ui.horizontal(|ui| {
                            let mark = if now_step == Some(si) { "▶" } else { " " };
                            ui.label(RichText::new(format!("{mark}{:>2}", si + 1)).monospace().color(if now_step == Some(si) { Color32::from_rgb(255, 150, 30) } else { ui.visuals().text_color() }));
                            let missing = !scenes.contains(&c.steps[si]);
                            let text = if missing { RichText::new(format!("{} (見つかりません)", c.steps[si])).color(WARN) } else { RichText::new(c.steps[si].as_str()) };
                            egui::ComboBox::from_id_salt(format!("cstep{ci}_{si}")).selected_text(text).width(220.0).show_ui(ui, |ui| {
                                for s in &scenes {
                                    if ui.selectable_label(c.steps[si] == *s, s).clicked() {
                                        c.steps[si] = s.clone();
                                        changed = true;
                                    }
                                }
                            });
                            if ui.small_button("上へ").clicked() {
                                step_op = Some(StepOp::Up(ci, si));
                            }
                            if ui.small_button("下へ").clicked() {
                                step_op = Some(StepOp::Down(ci, si));
                            }
                            if ui.small_button("削除").clicked() {
                                step_op = Some(StepOp::Del(ci, si));
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        if ui.add_enabled(!scenes.is_empty(), egui::Button::new("＋ ステップ追加")).clicked() {
                            c.steps.push(scenes[0].clone());
                            changed = true;
                        }
                        if ui.add_enabled(!scenes.is_empty(), egui::Button::new("全シーンを並べる")).clicked() {
                            c.steps = scenes.clone();
                            changed = true;
                        }
                    });
                });
            }
        });

        if let Some(op) = step_op {
            match op {
                StepOp::Up(ci, si) if si > 0 => core.project.chases[ci].steps.swap(si, si - 1),
                StepOp::Down(ci, si) if si + 1 < core.project.chases[ci].steps.len() => core.project.chases[ci].steps.swap(si, si + 1),
                StepOp::Del(ci, si) => {
                    core.project.chases[ci].steps.remove(si);
                }
                _ => {}
            }
            changed = true;
        }
        if let Some(i) = remove {
            let name = core.project.chases.remove(i).name;
            core.rt.chase_idx.remove(&name);
            changed = true;
        }
        if changed {
            core.dirty = true;
        }
        drop(guard);
        if services {
            self.apply_services();
        }
    }
}
