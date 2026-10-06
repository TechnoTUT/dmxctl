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

        let shared = self.core.clone();
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
                core.project.bindings.push(Binding {
                    name: format!("割り当て{}", core.project.bindings.len() + 1),
                    trigger: None,
                    action: default_action(false),
                    page: None,
                    osc: None,
                });
                changed = true;
            }
            ui.label(RichText::new("「MIDIラーン」を押してからパッドを叩く/ノブを回すと、そのボタンに割り当てられます。図で見たいときは「マップ」タブへ").weak());
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
            core.project.bindings.remove(i);
            core.rt.pad_state.clear();
            core.rt.toggle_state.clear();
            core.rt.learn = None;
            changed = true;
        }
        if changed {
            core.dirty = true;
        }
    }

    fn osc_row(&mut self, ui: &mut egui::Ui) {
        let (mut on, port, status, last) = {
            let c = lock(&self.core);
            (c.project.osc_enabled, c.project.osc_port, c.rt.osc_status.clone(), c.rt.last_osc.clone())
        };
        let mut apply = false;
        ui.horizontal_wrapped(|ui| {
            if ui.checkbox(&mut on, "OSCを受信").on_hover_text("TouchDesignerなどからOSC(UDP)で操作します").changed() {
                lock(&self.core).project.osc_enabled = on;
                apply = true;
            }
            ui.label("ポート");
            ui.add(egui::DragValue::new(&mut self.osc_port_edit).range(1024..=65535));
            if self.osc_port_edit != port && ui.button("適用").clicked() {
                lock(&self.core).project.osc_port = self.osc_port_edit;
                apply = true;
            }
            ui.label(RichText::new(format!("状態: {status}")).weak());
            if !last.is_empty() {
                ui.label(RichText::new(format!("最後の受信: {last}")).weak());
            }
        });
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
            lock(&self.core).dirty = true;
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
            let connected = self.midi.connected.is_some();
            if connected {
                if ui.button("切断").clicked() {
                    self.midi.disconnect();
                    let mut c = lock(&self.core);
                    c.rt.midi_status = "未接続".into();
                    c.project.midi_input.clear();
                    c.dirty = true;
                }
            } else if ui.add_enabled(!self.midi_choice.is_empty(), egui::Button::new("接続")).clicked() {
                let name = self.midi_choice.clone();
                match self.midi.connect(&self.core, ctx.clone(), &name) {
                    Ok(()) => {
                        let mut c = lock(&self.core);
                        c.project.midi_input = name;
                        c.dirty = true;
                        drop(c);
                        self.notice.clear();
                        // MIDIクロックの入力がメインと同じポートなら、専用の入力は閉じる
                        self.apply_services();
                    }
                    Err(e) => self.notice = format!("MIDI接続に失敗: {e}"),
                }
            }
        });
        // パッドのLED(APC mini mk2)
        ui.horizontal(|ui| {
            let (mut on, mut idle, status) = {
                let c = lock(&self.core);
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
            let mut true_color = lock(&self.core).project.led_true_color;
            if ui.checkbox(&mut true_color, "色のパッドは実際の色で").on_hover_text("色の指定・メイン/サブ色・機材の選択のパッドを、フルカラー(SysEx)で実際の色に光らせます。OFFなら128色のパレットから近い色").changed() {
                let mut c = lock(&self.core);
                c.project.led_true_color = true_color;
                c.dirty = true;
            }
            let note = if self.midi.led_active() { format!("LED: {status}") } else { "(APC mini mk2 に接続すると有効)".to_string() };
            ui.label(RichText::new(note).weak());
            if changed {
                let mut c = lock(&self.core);
                c.project.led_enabled = on;
                c.project.led_idle = idle;
                c.dirty = true;
            }
        });
    }
}
