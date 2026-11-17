//! MIDIタブ: 入力デバイスの接続と、割り当ての一覧編集。

use crate::app::{App, SelMeta};
use crate::engine::lock;
use crate::midi;
use crate::project::*;
use crate::ui_binding::{binding_editor, default_action, names, EditCtx, Learn};
use eframe::egui::{self, RichText};

impl App {
    pub fn tab_midi(&mut self, ui: &mut egui::Ui) {
        self.midi_port_row(ui);
        ui.separator();
        self.osc_row(ui);
        ui.separator();

        let shared = self.host.core.clone();
        let mut guard = lock(&shared);
        let core = &mut *guard;
        let lib = core.lib.clone();
        let meta = SelMeta::new(&core.project, &lib);
        let nm = names(&core.project);
        let cx = EditCtx::new(&meta, &lib, &nm);
        let mut changed = false;
        let mut remove: Option<usize> = None;

        ui.horizontal(|ui| {
            if ui.button("＋ 割り当てを追加").clicked() {
                core.project.bindings.push(Binding { id: 0,
                    name: format!("割り当て{}", core.project.bindings.len() + 1),
                    trigger: None,
                    action: default_action(false),
                    page: None,
                    osc: None,
                });
                changed = true;
            }
            crate::theme::help(ui, "「MIDIラーン」を押してからパッドを叩く/ノブを回すと、そのボタンに割り当てられます。図で見たいときは「MIDI」タブへ");
        });

        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            for (i, b) in core.project.bindings.iter_mut().enumerate() {
                let out = binding_editor(ui, i, b, Learn { midi: &mut core.rt.learn, osc: &mut core.rt.osc_learn }, &cx);
                changed |= out.changed;
                if out.remove {
                    remove = Some(i);
                }
            }
        });

        if let Some(i) = remove {
            core.release_remote_controls();
            match crate::editing::gui_apply(core,&serde_json::json!([{"entity":"binding","op":"delete","index":i}])) {
                Ok(_)=>changed=true,Err(error)=>self.host.notice=format!("割り当てを削除できません: {error}"),
            }
        }
        if changed {
            core.dirty = true;
        }
    }

    fn osc_row(&mut self, ui: &mut egui::Ui) {
        let (mut on, port, status, last, mut smooth, phrase, osc_rgb, mut depth, level) = {
            let c = lock(&self.host.core);
            (
                c.project.osc_enabled,
                c.project.osc_port,
                c.rt.osc_status.clone(),
                c.rt.last_osc.clone(),
                c.project.osc_rgb_secs,
                c.rt.phrase.clone(),
                c.rt.osc_rgb.map(|o| o.map(|o| o.current)),
                c.project.osc_level_depth,
                c.rt.osc_level.map(|o| o.current[0]),
            )
        };
        let mut apply = false;
        ui.horizontal_wrapped(|ui| {
            if ui.checkbox(&mut on, "OSCを受信").on_hover_text("TouchDesignerなどからOSC(UDP)で操作します").changed() {
                lock(&self.host.core).project.osc_enabled = on;
                apply = true;
            }
            ui.label("ポート");
            ui.add(egui::DragValue::new(&mut self.osc_port_edit).range(1024..=65535));
            if self.osc_port_edit != port && ui.button("適用").clicked() {
                lock(&self.host.core).project.osc_port = self.osc_port_edit;
                apply = true;
            }
            ui.label(RichText::new(format!("状態: {status}")).weak());
            if !last.is_empty() {
                ui.label(RichText::new(format!("最後の受信: {last}")).weak());
            }
        });
        // Beat Link Trigger などから色(RGB)・フレーズを受けているときの様子
        ui.horizontal_wrapped(|ui| {
            ui.label("RGBで受けた色へ近づける速さ");
            if ui.add(egui::Slider::new(&mut smooth, 0.0..=5.0).suffix(" 秒").fixed_decimals(1)).on_hover_text("/dmx/main/rgb・/dmx/sub/rgb で受けた色へ、この時間をかけてなめらかに近づけます(0ですぐ)").changed() {
                let mut c = lock(&self.host.core);
                c.project.osc_rgb_secs = smooth;
                c.dirty = true;
            }
            for (name, rgb) in ["メイン", "サブ"].iter().zip(osc_rgb) {
                if let Some([r, g, b]) = rgb {
                    ui.label(RichText::new(format!("{name}:")).weak());
                    let (rect, _) = ui.allocate_exact_size(egui::vec2(18.0, 14.0), egui::Sense::hover());
                    ui.painter().rect_filled(rect, 3.0, egui::Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8));
                }
            }
            if osc_rgb.iter().any(Option::is_some) && ui.small_button("RGBをやめる").on_hover_text("パレットの色に戻します").clicked() {
                lock(&self.host.core).live(crate::live::LiveOp::ClearOscColor);
            }
            if !phrase.is_empty() {
                ui.label(RichText::new(format!("フレーズ: {phrase}")).weak());
            }
        });
        // /dmx/level(波形の大きさなど)で明るさを変えるときの効き具合
        ui.horizontal_wrapped(|ui| {
            ui.label("OSCの明るさ(/dmx/level)の効き具合");
            if ui.add(egui::Slider::new(&mut depth, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0))).on_hover_text("受けた明るさが0のとき、全体光量をどこまで下げるか(100%で真っ暗、0%で効かない)").changed() {
                let mut c = lock(&self.host.core);
                c.project.osc_level_depth = depth;
                c.dirty = true;
            }
            if let Some(l) = level {
                ui.add(egui::ProgressBar::new(l.clamp(0.0, 1.0)).desired_width(80.0).text(format!("{:.0}%", l * 100.0)));
                if ui.small_button("明るさをやめる").clicked() {
                    lock(&self.host.core).live(crate::live::LiveOp::ClearOscLevel);
                }
            }
        });
        ui.label(RichText::new("オート(曲の情報で自動・待機の色・フレーズのプリセット)の設定は「オート」タブ").small().weak());
        self.web_row(ui);
        egui::CollapsingHeader::new("OSCで使えるアドレス").id_salt("osc_help").show(ui, |ui| {
            ui.label(
                RichText::new(
                    "割り当てに「OSC」アドレスを付けると(OSCラーンで登録)、その動作をOSCで実行できます。値は0〜1(ボタン系は0.5以上で押した扱い、値なしは1回押した扱い)。\
                     下の決まったアドレスは、割り当てなしでそのまま使えます",
                )
                .weak(),
            );
            egui::Grid::new("osc_addrs").num_columns(2).spacing([16.0, 4.0]).striped(true).show(ui, |ui| {
                for (addr, desc) in crate::osc::ADDRESSES {
                    ui.label(RichText::new(*addr).monospace());
                    ui.label(*desc);
                    ui.end_row();
                }
            });
        });
        if apply {
            lock(&self.host.core).dirty = true;
            self.apply_services();
        }
    }

    /// ブラウザ(スマホなど)の操作パネル
    fn web_row(&mut self, ui: &mut egui::Ui) {
        let (mut on, mut port, mut pin) = {
            let c = lock(&self.host.core);
            (c.project.web_enabled, c.project.web_port, c.project.web_pin.clone())
        };
        let before = (on, port, pin.clone());
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut on, "スマホから操作(操作パネル)").on_hover_text("同じネットワークのスマホ・PCのブラウザで、下のアドレスを開くと操作パネルが出ます(ブラックアウト・全体光量・オート・プリセット・色・シーン)");
            ui.label("ポート");
            ui.add(egui::DragValue::new(&mut port).range(1024..=65535));
            ui.label("合言葉");
            ui.add(egui::TextEdit::singleline(&mut pin).password(true).desired_width(90.0).hint_text("なし"))
                .on_hover_text("決めると、操作パネルを開いたときに入力を求めます。空なら、同じネットワークの誰でも操作できます");
            if on && self.host.web.is_some() {
                ui.hyperlink_to(&self.host.web_status, &self.host.web_status);
            } else if on && !self.host.web_status.is_empty() {
                ui.label(RichText::new(&self.host.web_status).color(crate::theme::WARN));
            }
        });
        if (on, port, pin.clone()) != before {
            {
                let mut c = lock(&self.host.core);
                c.project.web_enabled = on;
                c.project.web_port = port;
                c.project.web_pin = pin;
                c.dirty = true;
            }
            self.apply_services();
        }
    }

    fn midi_port_row(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.label("入力デバイス");
            let cur = if self.midi_choice.is_empty() { "(選択してください)".to_string() } else { self.midi_choice.clone() };
            egui::ComboBox::from_id_salt("midi_port").selected_text(cur).width(260.0).show_ui(ui, |ui| {
                for p in &self.midi_ports {
                    ui.selectable_value(&mut self.midi_choice, p.clone(), p);
                }
            });
            if ui.button("更新").clicked() {
                self.midi_ports = midi::list_ports();
            }
            let connected = self.host.midi.connected.is_some();
            if connected {
                if ui.button("切断").clicked() {
                    self.host.midi.disconnect();
                    let mut c = lock(&self.host.core);
                    c.rt.midi_status = "未接続".into();
                    c.project.midi_input.clear();
                    c.dirty = true;
                }
            } else if ui.add_enabled(!self.midi_choice.is_empty(), egui::Button::new("接続")).clicked() {
                let name = self.midi_choice.clone();
                match self.host.midi.connect(&self.host.core, ctx.clone(), &name) {
                    Ok(()) => {
                        let mut c = lock(&self.host.core);
                        c.project.midi_input = name;
                        c.dirty = true;
                        drop(c);
                        self.host.notice.clear();
                        // MIDIクロックの入力がメインと同じポートなら、専用の入力は閉じる
                        self.apply_services();
                    }
                    Err(e) => self.host.notice = format!("MIDI接続に失敗: {e}"),
                }
            }
        });
        // パッドのLED(APC mini mk2)
        ui.horizontal(|ui| {
            let (mut on, mut idle, status) = {
                let c = lock(&self.host.core);
                (c.project.led_enabled, c.project.led_idle, c.rt.led_status.clone())
            };
            let mut changed = false;
            changed |= ui.checkbox(&mut on, "パッドのLEDを点灯").changed();
            ui.label("待機中の明るさ");
            let names = ["暗め (10%)", "普通 (25%)", "明るめ (50%)"];
            egui::ComboBox::from_id_salt("led_idle").selected_text(names[(idle as usize).min(2)]).width(110.0).show_ui(ui, |ui| {
                for (i, n) in names.iter().enumerate() {
                    changed |= ui.selectable_value(&mut idle, i as u8, *n).changed();
                }
            });
            let mut true_color = lock(&self.host.core).project.led_true_color;
            if ui.checkbox(&mut true_color, "色のパッドは実際の色で").on_hover_text("色の指定・メイン/サブ色・機材の選択のパッドを、フルカラー(SysEx)で実際の色に光らせます。OFFなら128色のパレットから近い色").changed() {
                let mut c = lock(&self.host.core);
                c.project.led_true_color = true_color;
                c.dirty = true;
            }
            let note = if self.host.midi.led_active() { format!("LED: {status}") } else { "(APC mini mk2 に接続すると有効)".to_string() };
            ui.label(RichText::new(note).weak());
            if changed {
                let mut c = lock(&self.host.core);
                c.project.led_enabled = on;
                c.project.led_idle = idle;
                c.dirty = true;
            }
        });
    }
}
