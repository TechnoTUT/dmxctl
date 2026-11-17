//! 機材定義タブ: 機材・モード・チャンネル・値の範囲(選択肢)・カテゴリをアプリ内で作る/編集する。

use crate::app::{App, RED, WARN};
use crate::engine::lock;
use crate::fixture::*;
use crate::ui_control::category_color;
use eframe::egui::{self, vec2, Align, Layout, RichText};

pub struct ProfileEd {
    pub draft: Option<Profile>,
    pub is_new: bool,
    pub dirty: bool,
    pub mode: usize,
    /// 追加したカテゴリ(まだどのチャンネルにも使っていなくても選べるように)
    pub extra_cats: Vec<String>,
    pub new_cat: String,
    /// 「N個に均等分割」のN
    pub split_n: u32,
    pub confirm_delete: bool,
    /// 「RGBピクセル」のまとめ追加: 数と色の並び(PIXEL_ORDERSの番号)
    pub px_count: u16,
    pub px_order: usize,
    /// (エラーか, 内容)
    pub msg: (bool, String),
}

impl Default for ProfileEd {
    fn default() -> Self {
        ProfileEd { draft: None, is_new: false, dirty: false, mode: 0, extra_cats: Vec::new(), new_cat: String::new(), split_n: 8, confirm_delete: false, px_count: 8, px_order: 0, msg: (false, String::new()) }
    }
}

enum Act {
    Select(String),
    New,
    Duplicate,
    Save,
    Revert,
    Delete,
}

enum ChOp {
    Up(usize),
    Down(usize),
    Del(usize),
    AddAfter(usize),
}

impl App {
    fn pe_load(&mut self, lib: &Library, id: &str) {
        self.pe.draft = lib.get(id).map(|p| (**p).clone());
        self.pe.is_new = false;
        self.pe.dirty = false;
        self.pe.mode = 0;
        self.pe.confirm_delete = false;
    }

    pub fn tab_profiles(&mut self, ui: &mut egui::Ui) {
        let shared = self.core.clone();
        let mut guard = lock(&shared);
        let core = &mut *guard;
        let lib = core.lib.clone();
        let mut act: Option<Act> = None;
        let full = ui.available_size();
        let left_w = 250.0;
        let usage = |id: &str| core.project.fixtures.iter().filter(|f| f.profile == id).count();

        ui.horizontal_top(|ui| {
            // ---- 左: 機種の一覧 ----
            ui.allocate_ui_with_layout(vec2(left_w, full.y), Layout::top_down(Align::Min), |ui| {
                ui.set_width(left_w);
                ui.horizontal(|ui| {
                    if ui.button("＋ 新規作成").clicked() {
                        act = Some(Act::New);
                    }
                    if ui.add_enabled(self.pe.draft.is_some(), egui::Button::new("複製")).clicked() {
                        act = Some(Act::Duplicate);
                    }
                });
                ui.add_space(4.0);
                egui::ScrollArea::vertical().id_salt("plist").auto_shrink([false; 2]).show(ui, |ui| {
                    for p in &lib.profiles {
                        let sel = !self.pe.is_new && self.pe.draft.as_ref().is_some_and(|d| d.id == p.id);
                        let tag = match (lib.is_builtin(&p.id), lib.has_user_file(&p.id)) {
                            (true, true) => "内蔵・編集済",
                            (true, false) => "内蔵",
                            _ => "自作",
                        };
                        let n = usage(&p.id);
                        let text = format!("{}\n{}{}", p.name, tag, if n > 0 { format!(" / 使用中 {n}台") } else { String::new() });
                        if ui.selectable_label(sel, text).clicked() {
                            act = Some(Act::Select(p.id.clone()));
                        }
                    }
                    if self.pe.is_new {
                        let _ = ui.selectable_label(true, format!("{}\n(新規・未保存)", self.pe.draft.as_ref().map(|d| d.name.as_str()).unwrap_or("")));
                    }
                });
            });
            ui.separator();

            // ---- 右: エディタ ----
            ui.allocate_ui_with_layout(vec2((full.x - left_w - 24.0).max(420.0), full.y), Layout::top_down(Align::Min), |ui| {
                if self.pe.draft.is_none() {
                    ui.add_space(8.0);
                    ui.label("左の一覧から機材を選ぶか、「新規作成」で機材定義を作ります。");
                    ui.label(RichText::new("チャンネルごとに「動き」「色」などのカテゴリを付けると、コントロール画面がカテゴリ別にまとまり、シーンも「動きだけ」のように保存できます。").weak());
                    ui.label(RichText::new("内蔵の機材を編集した場合は、元の定義とは別に保存され、いつでも「初期状態に戻す」で戻せます。").weak());
                    if !self.pe.msg.1.is_empty() {
                        ui.colored_label(msg_color(self.pe.msg.0), &self.pe.msg.1);
                    }
                    return;
                }
                self.profile_editor(ui, &lib, &mut act, usage(&self.pe.draft.as_ref().unwrap().id));
            });
        });

        // ---- 操作の実行 ----
        let blocked = |pe: &ProfileEd| pe.dirty;
        const DIRTY_MSG: &str = "未保存の変更があります。「保存」するか「元に戻す」をしてから切り替えてください";
        match act {
            None => {}
            Some(Act::Select(id)) => {
                if blocked(&self.pe) {
                    self.pe.msg = (true, DIRTY_MSG.into());
                } else {
                    self.pe_load(&lib, &id);
                    self.pe.msg = (false, String::new());
                }
            }
            Some(Act::New) => {
                if blocked(&self.pe) {
                    self.pe.msg = (true, DIRTY_MSG.into());
                } else {
                    let mut m = Mode { name: "モード1".into(), channels: Vec::new() };
                    m.channels.push(ChannelDef::blank("ディマー", "dimmer", "光量"));
                    self.pe.draft = Some(Profile { id: lib.unique_id("new-fixture"), name: "新しい機材".into(), manufacturer: String::new(), needs_arm: false, modes: vec![m] });
                    self.pe.is_new = true;
                    self.pe.dirty = true;
                    self.pe.mode = 0;
                    self.pe.msg = (false, String::new());
                }
            }
            Some(Act::Duplicate) => {
                if let Some(d) = self.pe.draft.clone() {
                    let mut c = d;
                    c.id = lib.unique_id(&format!("{}-copy", c.name));
                    c.name = format!("{} (コピー)", c.name);
                    self.pe.draft = Some(c);
                    self.pe.is_new = true;
                    self.pe.dirty = true;
                    self.pe.msg = (false, "コピーしました。名前を変えて保存してください".into());
                }
            }
            Some(Act::Revert) => {
                let id = self.pe.draft.as_ref().map(|d| d.id.clone()).unwrap_or_default();
                if self.pe.is_new {
                    self.pe.draft = None;
                    self.pe.is_new = false;
                    self.pe.dirty = false;
                } else {
                    self.pe_load(&lib, &id);
                }
                self.pe.msg = (false, String::new());
            }
            Some(Act::Save) => {
                if self.pe.is_new
                    && let Some(d) = self.pe.draft.as_mut() {
                    d.id = lib.unique_id(&d.name);
                }
                if let Some(d) = self.pe.draft.clone() {
                    let report = d.validate();
                    if !report.errors.is_empty() {
                        self.pe.msg = (true, format!("保存できません: {}", report.errors.join(" / ")));
                    } else {
                        match core.save_profile(&d,false,false) {
                            Ok(result) => {
                                self.pe.is_new = false;
                                self.pe.dirty = false;
                                let warn = if report.warnings.is_empty() { String::new() } else { format!("(確認: {})", report.warnings.join(" / ")) };
                                self.pe.msg = (false, format!("保存しました → 機材にも反映済み {warn}\n{}", result["path"].as_str().unwrap_or("")));
                            }
                            Err(e) => self.pe.msg = (true, format!("保存に失敗しました: {e}")),
                        }
                    }
                }
            }
            Some(Act::Delete) => {
                if let Some(d) = self.pe.draft.clone() {
                    match core.delete_profile(&d.id,false,false) {
                        Ok(_) => {
                            let newlib=core.lib.clone();
                            if newlib.get(&d.id).is_some() {
                                self.pe_load(&newlib, &d.id);
                                self.pe.msg = (false, "内蔵の定義に戻しました".into());
                            } else {
                                self.pe.draft = None;
                                self.pe.dirty = false;
                                self.pe.msg = (false, format!("「{}」を削除しました", d.name));
                            }
                            self.pe.confirm_delete = false;
                        }
                        Err(e) => self.pe.msg = (true, format!("削除に失敗しました: {e}")),
                    }
                }
            }
        }
    }

    fn profile_editor(&mut self, ui: &mut egui::Ui, lib: &Library, act: &mut Option<Act>, used_by: usize) {
        let pe = &mut self.pe;
        let mut cats = lib.all_categories();
        for c in &pe.extra_cats {
            if !cats.contains(c) {
                cats.push(c.clone());
            }
        }
        let attr_suggestions = lib.all_attrs();
        let is_builtin;
        let has_user;
        let mut changed = false;
        {
            let d = pe.draft.as_mut().unwrap();
            for m in &d.modes {
                for c in &m.channels {
                    let n = c.category_name().to_string();
                    if !cats.contains(&n) {
                        cats.push(n);
                    }
                }
            }
            is_builtin = lib.is_builtin(&d.id);
            has_user = lib.has_user_file(&d.id);

            ui.horizontal_wrapped(|ui| {
                ui.label("機材名");
                changed |= ui.add(egui::TextEdit::singleline(&mut d.name).desired_width(200.0)).changed();
                ui.label("メーカー");
                changed |= ui.add(egui::TextEdit::singleline(&mut d.manufacturer).desired_width(150.0)).changed();
                ui.label(RichText::new(format!("id: {}", d.id)).weak().small());
            });
            changed |= ui.checkbox(&mut d.needs_arm, "ARM必須(レーザーなど。ARMするまで、チャンネルの「安全値」を出力し続けます)").changed();
        }

        // 保存・元に戻す・削除
        ui.horizontal(|ui| {
            if ui.add(egui::Button::new(RichText::new("保存").strong())).clicked() {
                *act = Some(Act::Save);
            }
            if ui.add_enabled(pe.dirty, egui::Button::new("元に戻す")).clicked() {
                *act = Some(Act::Revert);
            }
            if !pe.is_new && has_user {
                let label = if is_builtin { "初期状態に戻す" } else { "この機材定義を削除" };
                if pe.confirm_delete {
                    ui.colored_label(WARN, if used_by > 0 { format!("使用中の機材が{used_by}台あります。本当に?") } else { "本当に?".to_string() });
                    if ui.button("はい").clicked() {
                        *act = Some(Act::Delete);
                    }
                    if ui.button("やめる").clicked() {
                        pe.confirm_delete = false;
                    }
                } else if ui.button(label).clicked() {
                    pe.confirm_delete = true;
                }
            }
            if pe.dirty {
                ui.label(RichText::new("(未保存の変更あり)").color(WARN));
            }
        });
        if !pe.msg.1.is_empty() {
            ui.colored_label(msg_color(pe.msg.0), &pe.msg.1);
        }
        if used_by > 0 {
            ui.label(RichText::new(format!("この機材は現在 {used_by}台 で使われています。保存すると、値は機能名(attr)で引き継がれます")).weak());
        }

        // 検証結果(保存前でも見える)
        let report = pe.draft.as_ref().unwrap().validate();
        for e in &report.errors {
            ui.colored_label(RED, format!("× {e}"));
        }
        for w in &report.warnings {
            ui.colored_label(WARN, format!("△ {w}"));
        }
        ui.separator();

        // カテゴリの追加
        ui.horizontal(|ui| {
            ui.label("カテゴリ:");
            for c in &cats {
                ui.label(RichText::new(c.as_str()).color(category_color(c)).strong());
            }
            ui.add(egui::TextEdit::singleline(&mut pe.new_cat).hint_text("新しいカテゴリ名").desired_width(130.0));
            if ui.button("追加").clicked() && !pe.new_cat.trim().is_empty() {
                pe.extra_cats.push(pe.new_cat.trim().to_string());
                pe.new_cat.clear();
            }
        });
        ui.add_space(4.0);

        // ---- モード ----
        let d = pe.draft.as_mut().unwrap();
        pe.mode = pe.mode.min(d.modes.len().saturating_sub(1));
        let mut new_mode: Option<Mode> = None;
        let mut del_mode = false;
        ui.horizontal_wrapped(|ui| {
            ui.label("モード:");
            for (i, m) in d.modes.iter().enumerate() {
                if ui.selectable_label(pe.mode == i, format!("{} ({}ch)", m.name, m.channels.len())).clicked() {
                    pe.mode = i;
                }
            }
            if ui.button("＋ 追加").clicked() {
                let n = d.modes.len() + 1;
                new_mode = Some(Mode { name: format!("モード{n}"), channels: vec![ChannelDef::blank("ディマー", "dimmer", "光量")] });
            }
            if ui.button("複製").clicked() {
                let mut m = d.modes[pe.mode].clone();
                m.name = format!("{} (コピー)", m.name);
                new_mode = Some(m);
            }
            if d.modes.len() > 1 && ui.button("このモードを削除").clicked() {
                del_mode = true;
            }
        });
        if let Some(m) = new_mode {
            d.modes.push(m);
            pe.mode = d.modes.len() - 1;
            changed = true;
        }
        if del_mode {
            d.modes.remove(pe.mode);
            pe.mode = pe.mode.saturating_sub(1);
            changed = true;
        }
        let mode_idx = pe.mode.min(d.modes.len() - 1);
        let m = &mut d.modes[mode_idx];
        ui.horizontal(|ui| {
            ui.label("モード名");
            changed |= ui.add(egui::TextEdit::singleline(&mut m.name).desired_width(160.0)).changed();
            ui.label(RichText::new(format!("{}ch(DMXアドレスは「パッチ」タブで設定)", m.channels.len())).weak());
        });

        // 定番チャンネルのまとめ追加
        ui.horizontal_wrapped(|ui| {
            ui.label("まとめて追加:");
            if ui.button("Pan/Tilt (16bit)").clicked() {
                let base = m.channels.len();
                for (n, a, fine) in [("Pan", "pan", None), ("Pan Fine", "pan_fine", Some(base + 1)), ("Tilt", "tilt", None), ("Tilt Fine", "tilt_fine", Some(base + 3))] {
                    let mut c = ChannelDef::blank(n, a, "動き");
                    c.fine_of = fine;
                    c.default = if fine.is_none() { 127 } else { 0 };
                    m.channels.push(c);
                }
                changed = true;
            }
            if ui.button("RGB").clicked() {
                for (n, a) in [("赤", "red"), ("緑", "green"), ("青", "blue")] {
                    m.channels.push(ChannelDef::blank(n, a, "色"));
                }
                changed = true;
            }
            ui.menu_button("RGBピクセル (LEDバー)…", |ui| {
                ui.label(RichText::new("RGBが並んだLEDバー用。ピクセルごとに 赤/緑/青 のチャンネルを作ります").weak());
                ui.horizontal(|ui| {
                    ui.label("ピクセル数");
                    ui.add(egui::DragValue::new(&mut pe.px_count).range(1..=170));
                    ui.label("色の並び");
                    egui::ComboBox::from_id_salt("px_order").selected_text(PIXEL_ORDERS[pe.px_order]).width(70.0).show_ui(ui, |ui| {
                        for (i, o) in PIXEL_ORDERS.iter().enumerate() {
                            ui.selectable_value(&mut pe.px_order, i, *o);
                        }
                    });
                });
                let per = PIXEL_ORDERS[pe.px_order].len() as u16;
                // すでにピクセルがあれば、続きの番号から
                let next = m.pixels().len() as u16 + 1;
                if ui.button(format!("{}個({}ch)を追加", pe.px_count, pe.px_count * per)).clicked() {
                    m.channels.extend(pixel_channels(next, pe.px_count, PIXEL_ORDERS[pe.px_order]));
                    changed = true;
                    ui.close();
                }
            });
            if ui.button("ディマー").clicked() {
                let mut c = ChannelDef::blank("ディマー", "dimmer", "光量");
                c.safe = Some(0);
                m.channels.push(c);
                changed = true;
            }
            if ui.button("空のチャンネル").clicked() {
                let n = m.channels.len() + 1;
                m.channels.push(ChannelDef::blank(&format!("チャンネル{n}"), &format!("ch{n}"), "その他"));
                changed = true;
            }
        });
        ui.add_space(4.0);

        // ---- チャンネル一覧 ----
        let snap: Vec<(String, Option<usize>)> = m.channels.iter().map(|c| (c.name.clone(), c.fine_of)).collect();
        let n = m.channels.len();
        let mut op: Option<ChOp> = None;
        let split_n = &mut pe.split_n;
        egui::ScrollArea::vertical().id_salt("chlist").auto_shrink([false; 2]).show(ui, |ui| {
            for ci in 0..n {
                crate::theme::card().show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let ch = &mut m.channels[ci];
                    let is_fine = snap[ci].1.is_some();
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(format!("ch{}", ci + 1)).monospace().strong());
                        changed |= ui.add(egui::TextEdit::singleline(&mut ch.name).desired_width(140.0).hint_text("名前")).changed();
                        ui.label("機能名");
                        changed |= ui.add(egui::TextEdit::singleline(&mut ch.attr).desired_width(90.0)).changed();
                        egui::ComboBox::from_id_salt(format!("attrsug{ci}")).selected_text("候補").width(48.0).show_ui(ui, |ui| {
                            for a in &attr_suggestions {
                                if ui.selectable_label(ch.attr == *a, a.as_str()).clicked() {
                                    ch.attr = a.clone();
                                    changed = true;
                                }
                            }
                        });
                        ui.label("カテゴリ");
                        egui::ComboBox::from_id_salt(format!("cat{ci}")).selected_text(RichText::new(ch.category_name()).color(category_color(ch.category_name()))).width(80.0).show_ui(ui, |ui| {
                            for c in &cats {
                                if ui.selectable_label(ch.category_name() == c.as_str(), RichText::new(c.as_str()).color(category_color(c))).clicked() {
                                    ch.category = c.clone();
                                    changed = true;
                                }
                            }
                        });
                        if ui.small_button("上へ").clicked() {
                            op = Some(ChOp::Up(ci));
                        }
                        if ui.small_button("下へ").clicked() {
                            op = Some(ChOp::Down(ci));
                        }
                        if ui.small_button("下に追加").clicked() {
                            op = Some(ChOp::AddAfter(ci));
                        }
                        if ui.small_button("削除").clicked() {
                            op = Some(ChOp::Del(ci));
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.label("初期値");
                        changed |= ui.add(egui::DragValue::new(&mut ch.default).range(0..=255)).changed();
                        let mut has_safe = ch.safe.is_some();
                        if ui.checkbox(&mut has_safe, "安全値").on_hover_text("ブラックアウト時・ARM前にこの値を出力します(ディマー=0、レーザーのモード=OFFなど)").changed() {
                            ch.safe = if has_safe { Some(0) } else { None };
                            changed = true;
                        }
                        if let Some(s) = &mut ch.safe {
                            changed |= ui.add(egui::DragValue::new(s).range(0..=255)).changed();
                        }
                        // 16bit(上位/下位)
                        let cur = match ch.fine_of {
                            Some(k) if k >= 1 && k <= snap.len() => format!("ch{k} {} の下位バイト", snap[k - 1].0),
                            Some(_) => "(不正)".to_string(),
                            None => "通常(8bit)".to_string(),
                        };
                        egui::ComboBox::from_id_salt(format!("fine{ci}")).selected_text(cur).width(190.0).show_ui(ui, |ui| {
                            if ui.selectable_label(ch.fine_of.is_none(), "通常(8bit)").clicked() {
                                ch.fine_of = None;
                                changed = true;
                            }
                            for (j, (nm, jf)) in snap.iter().enumerate() {
                                let taken = snap.iter().enumerate().any(|(k, (_, f))| k != ci && *f == Some(j + 1));
                                if j != ci && jf.is_none() && !taken && ui.selectable_label(ch.fine_of == Some(j + 1), format!("ch{} {nm} の下位バイト", j + 1)).clicked() {
                                    ch.fine_of = Some(j + 1);
                                    ch.ranges.clear();
                                    changed = true;
                                }
                            }
                        });
                    });

                    if is_fine {
                        return;
                    }
                    // レンジ(選択肢)
                    let issues = range_issues(&ch.ranges);
                    let head = format!("値の範囲・選択肢 ({}件){}", ch.ranges.len(), if issues.is_empty() { "" } else { "  要確認" });
                    egui::CollapsingHeader::new(RichText::new(head).color(if issues.is_empty() { ui.visuals().text_color() } else { WARN })).id_salt(("rg", mode_idx, ci)).show(ui, |ui| {
                        ui.label(RichText::new("値の範囲ごとに名前を付けると、コントロール画面で選択式(色・ゴボなど)になります。何も無ければ0〜255のスライダーです。「連続」は範囲内の値が速度などの意味を持つ場合にチェックします").weak().small());
                        let mut rdel: Option<usize> = None;
                        egui::Grid::new(("rgg", mode_idx, ci)).spacing([8.0, 4.0]).show(ui, |ui| {
                            for (ri, r) in ch.ranges.iter_mut().enumerate() {
                                changed |= ui.add(egui::DragValue::new(&mut r.from).range(0..=255)).changed();
                                changed |= ui.add(egui::DragValue::new(&mut r.to).range(0..=255).prefix("〜 ")).changed();
                                changed |= ui.add_sized([220.0, 20.0], egui::TextEdit::singleline(&mut r.label).hint_text("表示名")).changed();
                                // この範囲が出す色。チェックを入れて色を選ぶと、名前に関係なくその色として扱う
                                ui.horizontal(|ui| {
                                    let auto = crate::color::label_rgb(&r.label);
                                    let mut has = r.color.is_some();
                                    if ui
                                        .checkbox(&mut has, "色")
                                        .on_hover_text("この範囲が出す色を指定します(色の全体制御・色相フェーダー・パッドのLEDが使います)。チェックなしなら、表示名の色の名前(赤・青など)から判定します")
                                        .changed()
                                    {
                                        let to8 = |c: [f32; 3]| [(c[0] * 255.0).round() as u8, (c[1] * 255.0).round() as u8, (c[2] * 255.0).round() as u8];
                                        r.color = if has { Some(auto.map(to8).unwrap_or([255, 255, 255])) } else { None };
                                        changed = true;
                                    }
                                    match &mut r.color {
                                        Some(rgb) => {
                                            changed |= ui.color_edit_button_srgb(rgb).changed();
                                        }
                                        None => {
                                            // 表示名から判定された色の見本(判定できなければ「−」)
                                            match auto {
                                                Some(c) => {
                                                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(24.0, 18.0), egui::Sense::hover());
                                                    ui.painter().rect_filled(rect, 3.0, egui::Color32::from_rgb((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8));
                                                    resp.on_hover_text("表示名から判定した色");
                                                }
                                                None => {
                                                    ui.label(RichText::new("色なし").weak().small()).on_hover_text("表示名から色を判定できないので、色の全体制御の対象になりません。「色」にチェックして色を指定できます");
                                                }
                                            }
                                        }
                                    }
                                });
                                changed |= ui.checkbox(&mut r.sweep, "連続").changed();
                                changed |= ui.checkbox(&mut r.warn, "注意").on_hover_text("リセットなど、誤って選ぶと困る項目").changed();
                                if ui.small_button("削除").clicked() {
                                    rdel = Some(ri);
                                }
                                ui.end_row();
                            }
                        });
                        if let Some(ri) = rdel {
                            ch.ranges.remove(ri);
                            changed = true;
                        }
                        ui.horizontal(|ui| {
                            if ui.button("＋ 範囲を追加").clicked() {
                                let start = ch.ranges.last().map(|r| r.to.saturating_add(1)).unwrap_or(0);
                                let end = (start as u16 + 9).min(255) as u8;
                                let label = format!("項目{}", ch.ranges.len() + 1);
                                ch.ranges.push(Range { from: start, to: end, label, sweep: false, warn: false, color: None });
                                changed = true;
                            }
                            ui.add(egui::DragValue::new(split_n).range(2..=64).suffix(" 個"));
                            if ui.button("均等に分割(今の範囲は置き換え)").clicked() {
                                let k = (*split_n).max(2) as usize;
                                ch.ranges = (0..k)
                                    .map(|i| Range { from: (i * 256 / k) as u8, to: ((i + 1) * 256 / k - 1) as u8, label: format!("{}", i + 1), sweep: false, warn: false, color: None })
                                    .collect();
                                changed = true;
                            }
                            if !ch.ranges.is_empty() && ui.button("すべて消す").clicked() {
                                ch.ranges.clear();
                                changed = true;
                            }
                        });
                        for i in issues {
                            ui.colored_label(WARN, format!("△ {i}"));
                        }
                    });
                });
            }
        });

        match op {
            Some(ChOp::Up(i)) => m.move_channel(i, -1),
            Some(ChOp::Down(i)) => m.move_channel(i, 1),
            Some(ChOp::Del(i)) => {
                if m.channels.len() > 1 {
                    m.remove_channel(i)
                }
            }
            Some(ChOp::AddAfter(i)) => {
                let k = m.channels.len() + 1;
                let mut attr = format!("ch{k}");
                while m.find_attr(&attr).is_some() {
                    attr.push('_');
                }
                m.insert_channel(i + 1, ChannelDef::blank(&format!("チャンネル{k}"), &attr, "その他"));
            }
            None => {}
        }
        if op_changed(&op) {
            changed = true;
        }
        if changed {
            pe.dirty = true;
            pe.confirm_delete = false;
        }
    }
}

/// エラーは赤、成功などの通知は緑
fn msg_color(is_error: bool) -> eframe::egui::Color32 {
    if is_error { RED } else { eframe::egui::Color32::from_rgb(70, 160, 100) }
}

fn op_changed(op: &Option<ChOp>) -> bool {
    op.is_some()
}
