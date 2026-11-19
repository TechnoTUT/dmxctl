//! パッチタブ: 機材の追加・アドレス設定・グループ。

use crate::app::{App, RED};
use dmxctl_core::engine::lock;
use dmxctl_core::project::*;
use eframe::egui::{self, RichText};

impl App {
    pub fn tab_patch(&mut self, ui: &mut egui::Ui) {
        let shared = self.host.core.clone();
        let mut guard = lock(&shared);
        let core = &mut *guard;
        let lib = core.lib.clone();
        let mut changed = false;

        ui.horizontal(|ui| {
            self.add_profile = self.add_profile.min(lib.profiles.len().saturating_sub(1));
            let cur = lib.profiles.get(self.add_profile).map(|p| p.name.clone()).unwrap_or_default();
            egui::ComboBox::from_id_salt("add_profile").selected_text(cur).show_ui(ui, |ui| {
                for (i, p) in lib.profiles.iter().enumerate() {
                    ui.selectable_value(&mut self.add_profile, i, format!("{} {}", p.manufacturer, p.name));
                }
            });
            if ui.button("＋ 機材を追加").clicked()
                && let Some(p) = lib.profiles.get(self.add_profile) {
                let count=core.project.fixtures.iter().filter(|f|f.profile==p.id).count()+1;
                match dmxctl_backend::editing::gui_apply(core,&serde_json::json!([{"entity":"fixture","op":"create",
                    "data":{"profile":p.id,"name":format!("{} {}",p.name,count)}}])) {
                    Ok(_)=>changed=true,Err(error)=>self.host.notice=format!("機材を追加できません: {error}"),
                }
            }
            crate::theme::help(ui, &format!("機材定義は「機材定義」タブで作れます。{} に .json を置いても増やせます", dmxctl_core::project::config_dir().join("fixtures").display()));
        });
        ui.separator();

        // 重複チェック用に各機材の占有範囲を先に計算
        let spans: Vec<(u32, usize, usize, usize)> = core
            .project
            .fixtures
            .iter()
            .map(|f| {
                let n = lib.mode(&f.profile, f.mode).map(|m| m.channels.len()).unwrap_or(0);
                (f.id, f.universe, f.address as usize, f.address as usize + n)
            })
            .collect();

        let mut remove: Option<usize> = None;
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            egui::Grid::new("patch").striped(true).spacing([12.0, 6.0]).show(ui, |ui| {
                for h in ["使う", "名前", "機種", "モード", "ユニバース", "開始アドレス", "ch", "状態", ""] {
                    ui.label(RichText::new(h).strong());
                }
                ui.end_row();

                for (i, fx) in core.project.fixtures.iter_mut().enumerate() {
                    changed |= ui.checkbox(&mut fx.enabled, "").on_hover_text("OFFにすると出力しません(操作・エフェクトの対象からも外れます)").changed();
                    changed |= ui.add_sized([150.0, 24.0], egui::TextEdit::singleline(&mut fx.name)).changed();
                    let profile = lib.get(&fx.profile);
                    ui.label(profile.map(|p| p.name.as_str()).unwrap_or("(不明)"));

                    if let Some(p) = profile {
                        let cur = p.modes.get(fx.mode).map(|m| m.name.clone()).unwrap_or_default();
                        egui::ComboBox::from_id_salt(("mode", fx.id)).selected_text(cur).show_ui(ui, |ui| {
                            for (mi, m) in p.modes.iter().enumerate() {
                                if ui.selectable_label(fx.mode == mi, format!("{} ({}ch)", m.name, m.channels.len())).clicked() && fx.mode != mi {
                                    fx.mode = mi;
                                    fx.values = m.default_values();
                                    changed = true;
                                }
                            }
                        });
                    } else {
                        ui.label("-");
                    }

                    let mut u = fx.universe as u32 + 1;
                    if ui.add(egui::DragValue::new(&mut u).range(1..=NUM_UNIVERSES as u32)).changed() {
                        fx.universe = u as usize - 1;
                        changed = true;
                    }
                    changed |= ui.add(egui::DragValue::new(&mut fx.address).range(1..=512)).changed();

                    let n = spans[i].3 - spans[i].2;
                    ui.label(n.to_string());

                    let overlap = spans.iter().any(|s| s.0 != fx.id && s.1 == spans[i].1 && s.2 < spans[i].3 && spans[i].2 < s.3);
                    if spans[i].3 > 513 {
                        ui.label(RichText::new("512を超過").color(RED));
                    } else if overlap {
                        ui.label(RichText::new("アドレス重複").color(RED));
                    } else {
                        ui.label(format!("{}-{}", spans[i].2, spans[i].3.saturating_sub(1)));
                    }
                    if ui.button("削除").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });

            ui.add_space(12.0);
            ui.horizontal(|ui| {
                crate::theme::heading(ui, "グループ");
                crate::theme::help(ui, "MIDI割り当てやエフェクトの対象に「グループ」を選べます");
            });
            let fx_names: Vec<(u32, String)> = core.project.fixtures.iter().map(|f| (f.id, f.name.clone())).collect();
            let mut del_group: Option<String> = None;
            let mut rename: Option<(String, String)> = None;
            for (gi, g) in core.project.groups.iter_mut().enumerate() {
                ui.horizontal_wrapped(|ui| {
                    // 割り当てやエフェクトが名前で指しているので、1文字ごとには変えず、確定したときに参照ごと書き換える
                    let mut text = match &self.group_rename {
                        Some((i, t)) if *i == gi => t.clone(),
                        _ => g.name.clone(),
                    };
                    let response = ui.add(egui::TextEdit::singleline(&mut text).desired_width(120.0));
                    if response.changed() {
                        self.group_rename = Some((gi, text));
                    }
                    if response.lost_focus()
                        && let Some((_, t)) = self.group_rename.take_if(|(i, _)| *i == gi)
                        && t.trim() != g.name
                    {
                        rename = Some((g.name.clone(), t));
                    }
                    for (id, name) in &fx_names {
                        let mut on = g.fixtures.contains(id);
                        if ui.checkbox(&mut on, name).changed() {
                            if on {
                                g.fixtures.push(*id);
                            } else {
                                g.fixtures.retain(|x| x != id);
                            }
                            changed = true;
                        }
                    }
                    if ui.button("グループ削除").clicked() {
                        del_group = Some(g.name.clone());
                    }
                });
            }
            if let Some((old, new)) = rename {
                if let Err(error) = dmxctl_backend::editing::gui_rename_group(core, &old, &new) {
                    self.host.notice = format!("グループの名前を変えられません: {error}");
                }
            }
            if let Some(name) = del_group {
                self.group_rename = None;
                // 消したグループを指す割り当て・エフェクトも片付ける。残すと検証に通らず、以後の編集がすべて失敗する
                match dmxctl_backend::editing::gui_delete_group(core, &name) {
                    Ok(removed) if removed.is_empty() => self.host.notice = format!("グループ「{name}」を削除しました"),
                    Ok(removed) => self.host.notice = format!("グループ「{name}」を削除しました。このグループを対象にしていた {} も削除しました", removed.join("、")),
                    Err(error) => self.host.notice = format!("グループを削除できません: {error}"),
                }
            }
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut self.new_group).hint_text("新しいグループ名").desired_width(160.0));
                if ui.button("＋ グループ追加").clicked() && !self.new_group.trim().is_empty() {
                    match dmxctl_backend::editing::gui_apply(core,&serde_json::json!([{"entity":"group","op":"create","data":{"name":self.new_group.trim()}}])) {
                        Ok(_)=>{self.new_group.clear();changed=true;},Err(error)=>self.host.notice=format!("グループを追加できません: {error}"),
                    }
                }
            });
        });

        if let Some(i) = remove {
            let (id, name) = (core.project.fixtures[i].id, core.project.fixtures[i].name.clone());
            // 消した機材を指す割り当て・エフェクトも片付ける。残すと検証に通らず、以後の編集がすべて失敗する
            match dmxctl_backend::editing::gui_delete_fixture(core, id) {
                Ok(removed) if removed.is_empty() => self.host.notice = format!("「{name}」を削除しました"),
                Ok(removed) => self.host.notice = format!("「{name}」を削除しました。この機材を対象にしていた {} も削除しました", removed.join("、")),
                Err(error) => self.host.notice = format!("機材を削除できません: {error}"),
            }
        }
        if changed {
            core.dirty = true;
        }
    }
}
