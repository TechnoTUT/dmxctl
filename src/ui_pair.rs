//! ペアタブ: 2台の機材を連動させる(左右反転など)。

use crate::app::{App, RED, WARN};
use crate::engine::lock;
use crate::project::*;
use eframe::egui::{self, RichText};

impl App {
    pub fn tab_pairs(&mut self, ui: &mut egui::Ui) {
        let shared = self.host.core.clone();
        let mut guard = lock(&shared);
        let core = &mut *guard;
        let lib = core.lib.clone();
        let fixtures: Vec<(u32, String)> = core.project.fixtures.iter().map(|f| (f.id, f.name.clone())).collect();
        let mut changed = false;
        let mut remove: Option<usize> = None;

        ui.horizontal(|ui| {
            crate::theme::heading(ui, "ペア");
            crate::theme::help(
                ui,
                "ペア = 「元」の機材の動きに「ペア相手」が連動します(値・エフェクト・シーン・チェイスの結果がそのまま追従)。左右反転をONにすると、指定した機能(通常はPan)が反転します。\n\
                 連動・反転はMIDIパッドにも割り当てられます(割り当ての動作「ペア」)。ペア相手のチャンネルは、連動している間はコントロール画面で操作できません",
            );
        });
        ui.horizontal(|ui| {
            let ok = fixtures.len() >= 2;
            if ui.add_enabled(ok, egui::Button::new("＋ ペアを追加")).clicked() {
                core.project.pairs.push(Pair {
                    name: format!("ペア{}", core.project.pairs.len() + 1),
                    leader: fixtures[0].0,
                    follower: fixtures[1].0,
                    ..Pair::default()
                });
                changed = true;
            }
            if !ok {
                ui.colored_label(WARN, "機材が2台以上必要です(「パッチ」タブで追加。同じ機種を2台入れると左右反転が使えます)");
            }
        });
        ui.separator();

        // 連動するチャンネル数(ペアごと)と、元の機材で反転を選べる機能
        let counts: Vec<Option<usize>> = core.project.pairs.iter().map(|p| core.pair_plan(p).map(|pl| pl.chans.len())).collect();
        let name_of = |id: u32| fixtures.iter().find(|f| f.0 == id).map(|f| f.1.clone()).unwrap_or_else(|| "(削除された機材)".into());
        let cats = core.present_categories();

        // 反転の候補: 元の機材の「動き」カテゴリの機能
        let invert_candidates: Vec<Vec<String>> = core
            .project
            .pairs
            .iter()
            .map(|p| {
                let mut v: Vec<String> = Vec::new();
                if let Some(fx) = core.project.fixtures.iter().find(|f| f.id == p.leader)
                    && let Some(m) = lib.mode(&fx.profile, fx.mode) {
                    for c in m.channels.iter().filter(|c| c.fine_of.is_none() && c.category_name() == "動き") {
                        if !v.contains(&c.attr) {
                            v.push(c.attr.clone());
                        }
                    }
                }
                for a in &p.invert {
                    if !v.contains(a) {
                        v.push(a.clone());
                    }
                }
                v
            })
            .collect();

        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            if core.project.pairs.is_empty() {
                ui.label("ペアはまだありません。");
            }
            for (i, p) in core.project.pairs.iter_mut().enumerate() {
                crate::theme::card().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        changed |= ui.checkbox(&mut p.enabled, "連動").changed();
                        changed |= ui.checkbox(&mut p.mirror, "左右反転").changed();
                        changed |= ui.add(egui::TextEdit::singleline(&mut p.name).desired_width(140.0)).changed();
                        if ui.button("削除").clicked() {
                            remove = Some(i);
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label("元(操作する側)");
                        egui::ComboBox::from_id_salt(("pl", i)).selected_text(name_of(p.leader)).width(150.0).show_ui(ui, |ui| {
                            for (id, n) in &fixtures {
                                changed |= ui.selectable_value(&mut p.leader, *id, n).changed();
                            }
                        });
                        ui.label("→ ペア相手(追従する側)");
                        egui::ComboBox::from_id_salt(("pf", i)).selected_text(name_of(p.follower)).width(150.0).show_ui(ui, |ui| {
                            for (id, n) in &fixtures {
                                changed |= ui.selectable_value(&mut p.follower, *id, n).changed();
                            }
                        });
                        match counts[i] {
                            Some(n) => ui.label(RichText::new(format!("連動するチャンネル: {n}個")).weak()),
                            None => ui.label(RichText::new("この組み合わせでは連動できません").color(RED)),
                        };
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label("反転する機能:");
                        for a in &invert_candidates[i] {
                            let mut on = p.invert.contains(a);
                            if ui.checkbox(&mut on, a.as_str()).changed() {
                                if on {
                                    p.invert.push(a.clone());
                                } else {
                                    p.invert.retain(|x| x != a);
                                }
                                changed = true;
                            }
                        }
                        ui.label(RichText::new("(0↔255。レンジ付きの機能は、連続レンジの中で反転)").weak().small());
                    });
                    if !cats.is_empty() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("連動しない項目:");
                            for c in &cats {
                                let mut skip = p.skip_cats.contains(c);
                                if ui.checkbox(&mut skip, c.as_str()).changed() {
                                    if skip {
                                        p.skip_cats.push(c.clone());
                                    } else {
                                        p.skip_cats.retain(|x| x != c);
                                    }
                                    changed = true;
                                }
                            }
                        });
                    }
                    if p.leader == p.follower {
                        ui.colored_label(RED, "元とペア相手が同じ機材です");
                    }
                });
            }
        });

        if let Some(i) = remove {
            core.project.pairs.remove(i);
            changed = true;
        }
        if changed {
            core.dirty = true;
        }
    }
}
