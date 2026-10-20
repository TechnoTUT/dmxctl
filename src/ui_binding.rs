//! 割り当て1件分の編集UI。MIDIタブとマップタブで共用する。

use crate::app::{selector_combo, SelMeta};
use crate::color::{ColorTarget, NamedColor, PaletteOp, ScopeOp, SelectOp};
use crate::fixture::Library;
use crate::project::*;
use eframe::egui::{self, RichText};

/// 編集UIが参照する、Coreから複製した選択肢の情報
pub struct EditCtx<'a> {
    pub meta: &'a SelMeta,
    pub lib: &'a Library,
    pub scenes: &'a [String],
    pub effects: &'a [String],
    pub chases: &'a [String],
    pub pairs: &'a [String],
    pub positions: &'a [String],
}

impl<'a> EditCtx<'a> {
    pub fn new(meta: &'a SelMeta, lib: &'a Library, n: &'a Names) -> Self {
        EditCtx { meta, lib, scenes: &n.scenes, effects: &n.effects, chases: &n.chases, pairs: &n.pairs, positions: &n.positions }
    }
}

/// 割り当て1件ぶんの「MIDIラーン」「OSCラーン」の待ち状態
pub struct Learn<'a> {
    pub midi: &'a mut Option<usize>,
    pub osc: &'a mut Option<usize>,
}

#[derive(Default)]
pub struct EditOut {
    pub changed: bool,
    pub remove: bool,
}

/// 何も割り当てていないコントロール用の、最初の動作。フェーダーは連続値、ボタン/パッドはトグル
pub fn default_action(continuous: bool) -> Action {
    Action::SetValue {
        sel: Selector::All,
        attr: "dimmer".into(),
        mode: if continuous { PadMode::Continuous } else { PadMode::Toggle },
        on: 255.0,
        off: 0.0,
    }
}

/// マップの升目に出す短い説明(2〜3行)
/// 対象の短い名前(マップの升目用)
pub fn target_short(sel: &Selector, meta: &SelMeta) -> String {
    match sel {
        Selector::All => "全機材".to_string(),
        Selector::Fixture(_) => meta.label(sel),
        Selector::Group(g) => format!("G:{g}"),
        Selector::Profile(p) => format!("機種:{}", meta.profiles.iter().find(|(id, _)| id == p).map(|(_, n)| n.as_str()).unwrap_or(p)),
    }
}

pub fn summary(a: &Action, meta: &SelMeta) -> String {
    match a {
        Action::SetValue { sel, attr, mode, .. } => {
            let target = target_short(sel, meta);
            let m = match mode {
                PadMode::Continuous => "連続",
                PadMode::Momentary => "押中",
                PadMode::Toggle => "切替",
                PadMode::Trigger => "1回",
            };
            format!("{target}\n{attr} {m}")
        }
        Action::Scene { name } => format!("シーン\n{name}"),
        Action::Effect { name, mode } => format!("エフェクト\n{name} {}", if *mode == PadMode::Toggle { "切替" } else { "押中" }),
        Action::Blackout { .. } => "BLACK\nOUT".into(),
        Action::Master { mode, .. } => format!("全体光量\n{}", if *mode == PadMode::Continuous { "連続" } else { "切替" }),
        Action::Tempo { op } => match op {
            TempoOp::Tap => "TAP\nテンポ".into(),
            TempoOp::Resync => "頭出し\nテンポ".into(),
            TempoOp::Set { .. } => "BPM\n連続".into(),
            TempoOp::Nudge { delta } => format!("BPM\n{delta:+}"),
            TempoOp::Link => "Ableton Link\nON/OFF".into(),
            TempoOp::Audio => "音声BPM\nON/OFF".into(),
        },
        Action::Chase { name, mode } => format!("チェイス\n{name} {}", if *mode == PadMode::Toggle { "切替" } else { "押中" }),
        Action::Pair { name, op, .. } => format!("ペア\n{name} {}", if *op == PairOp::Link { "連動" } else { "反転" }),
        Action::Color { color, target } => {
            let head = match target {
                ColorTarget::Main => "メイン色",
                ColorTarget::Sub => "サブ色",
                _ => "色",
            };
            format!("{head}\n{}", color.map(|c| c.label()).unwrap_or("色相"))
        }
        Action::Fade { secs, mode } => format!("暗転\n{secs}秒{}", if *mode == PadMode::Toggle { " 切替" } else { " 押中" }),
        Action::Flash { white, mode } => format!("FLASH\n{}{}", if *white { "白" } else { "今の色" }, if *mode == PadMode::Toggle { " 切替" } else { "" }),
        Action::Palette { op } => match op {
            PaletteOp::Swap => "メイン⇄サブ\n入れ替え".into(),
            PaletteOp::HoldSwap => "メイン⇄サブ\n押中".into(),
            PaletteOp::Auto => "メイン⇄サブ\n自動".into(),
            PaletteOp::Fade => "メイン⇄サブ\nグラデーション".into(),
        },
        Action::ColorScope { op } => format!("色\n{}", match op { ScopeOp::Toggle => "全体/個別", ScopeOp::HoldIndividual => "押中=個別", ScopeOp::Global => "全体へ", ScopeOp::Individual => "個別へ" }),
        Action::Select { fixture, .. } => format!("選択\n{}", meta.fixtures.iter().find(|f| f.id == *fixture).map(|f| f.name.as_str()).unwrap_or("?")),
        Action::Position { name } => format!("位置\n{name}"),
        Action::Page { op } => match op {
            PageOp::Go(p) => format!("ページ\n{}", p + 1),
            PageOp::Next => "ページ\n次へ".into(),
            PageOp::Prev => "ページ\n前へ".into(),
        },
        Action::Level { sel } => format!("明るさ\n{}", target_short(sel, meta)),
        Action::Mute { sel, .. } => format!("ミュート\n{}", target_short(sel, meta)),
        Action::Pixel { param } => match param {
            crate::pixel::PixelParam::Speed => "LEDバー\n速さ".into(),
            crate::pixel::PixelParam::Skew => "LEDバー\n斜め".into(),
        },
        Action::Role { sel, op } => format!("色の系統\n{} {}", target_short(sel, meta), op.label()),
        Action::Arm { sel, .. } => format!("ARM\n{}", target_short(sel, meta)),
        Action::LiveColor { role, source } => format!("NDI {}\n{}", role.label(), source.label()),
        Action::SwapBeats { beats } => format!("入れ替え\n{beats}拍ごと"),
        Action::BarCount { count } => format!("LEDバー\n{count}本"),
    }
}

pub fn page_label(page: Option<u8>) -> String {
    match page {
        None => "全ページ共通".into(),
        Some(p) => format!("ページ{}だけ", p + 1),
    }
}

pub fn binding_editor(ui: &mut egui::Ui, i: usize, b: &mut Binding, learn: Learn, cx: &EditCtx) -> EditOut {
    let mut out = EditOut::default();
    crate::theme::card().show(ui, |ui| {
        ui.horizontal(|ui| {
            let learning = *learn.midi == Some(i);
            let text = if learning { "待機中... パッドを押してください" } else { "MIDIラーン" };
            if ui.selectable_label(learning, text).clicked() {
                *learn.midi = if learning { None } else { Some(i) };
                *learn.osc = None;
            }
            let trig = b.trigger.map(|t| t.label()).unwrap_or_else(|| "(未設定)".into());
            ui.label(RichText::new(trig).strong());
            out.changed |= ui.add(egui::TextEdit::singleline(&mut b.name).desired_width(140.0)).changed();
            if ui.button("削除").clicked() {
                out.remove = true;
            }
        });
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt(format!("bpage{i}"))
                .selected_text(page_label(b.page))
                .width(110.0)
                .show_ui(ui, |ui| {
                    out.changed |= ui.selectable_value(&mut b.page, None, page_label(None)).changed();
                    for p in 0..PAGES {
                        out.changed |= ui.selectable_value(&mut b.page, Some(p), page_label(Some(p))).changed();
                    }
                })
                .response
                .on_hover_text("MIDIで反応するページ。パッドをページごとに使い分けるときに「ページNだけ」にします");
            ui.label("OSC");
            let mut addr = b.osc.clone().unwrap_or_default();
            if ui.add(egui::TextEdit::singleline(&mut addr).hint_text("/アドレス").desired_width(120.0)).changed() {
                let t = addr.trim();
                b.osc = (!t.is_empty()).then(|| t.to_string());
                out.changed = true;
            }
            let learning = *learn.osc == Some(i);
            if ui.selectable_label(learning, if learning { "待機中... OSCを送ってください" } else { "OSCラーン" }).clicked() {
                *learn.osc = if learning { None } else { Some(i) };
                *learn.midi = None;
            }
        });
        ui.horizontal_wrapped(|ui| {
            out.changed |= action_kind_combo(ui, i, &mut b.action, cx);
            match &mut b.action {
                Action::SetValue { sel, attr, mode, on, off } => {
                    out.changed |= selector_combo(ui, &format!("bsel{i}"), sel, cx.meta);
                    let mut attrs = cx.meta.attrs(cx.lib, sel);
                    if !attrs.contains(attr) {
                        attrs.push(attr.clone());
                    }
                    egui::ComboBox::from_id_salt(format!("battr{i}")).selected_text(attr.as_str()).width(110.0).show_ui(ui, |ui| {
                        for a in attrs {
                            if ui.selectable_label(*attr == a, &a).clicked() {
                                *attr = a;
                                out.changed = true;
                            }
                        }
                    });
                    // 幅の狭い編集パネルでも切れないよう、ここで改行する
                    ui.end_row();
                    out.changed |= pad_mode_combo(ui, &format!("bmode{i}"), mode, true);
                    let (on_label, off_label) = if *mode == PadMode::Continuous { ("最大", "最小") } else { ("ON値", "OFF値") };
                    ui.label(on_label);
                    out.changed |= ui.add(egui::DragValue::new(on).range(0.0..=255.0)).changed();
                    ui.label(off_label);
                    out.changed |= ui.add(egui::DragValue::new(off).range(0.0..=255.0)).changed();
                    // カラー/ゴボ等は、名前でON値を選べる
                    let ranges = cx.meta.ranges(cx.lib, sel, attr);
                    if !ranges.is_empty() {
                        egui::ComboBox::from_id_salt(format!("brange{i}")).selected_text("名前でON値を選ぶ").show_ui(ui, |ui| {
                            for r in &ranges {
                                if ui.selectable_label(false, &r.label).clicked() {
                                    *on = r.mid() as f32;
                                    out.changed = true;
                                }
                            }
                        });
                    }
                }
                Action::Scene { name } => {
                    egui::ComboBox::from_id_salt(format!("bscene{i}")).selected_text(name.as_str()).show_ui(ui, |ui| {
                        for s in cx.scenes {
                            if ui.selectable_label(name == s, s).clicked() {
                                *name = s.clone();
                                out.changed = true;
                            }
                        }
                    });
                }
                Action::Effect { name, mode } => {
                    egui::ComboBox::from_id_salt(format!("beffect{i}")).selected_text(name.as_str()).show_ui(ui, |ui| {
                        for s in cx.effects {
                            if ui.selectable_label(name == s, s).clicked() {
                                *name = s.clone();
                                out.changed = true;
                            }
                        }
                    });
                    out.changed |= pad_mode_combo(ui, &format!("bemode{i}"), mode, false);
                }
                Action::Blackout { mode } => {
                    out.changed |= pad_mode_combo(ui, &format!("bbmode{i}"), mode, false);
                }
                Action::Tempo { op } => {
                    let label = tempo_label(op);
                    egui::ComboBox::from_id_salt(format!("btempo{i}")).selected_text(label).width(190.0).show_ui(ui, |ui| {
                        let choices = [
                            TempoOp::Tap,
                            TempoOp::Resync,
                            TempoOp::Set { min: 60.0, max: 180.0 },
                            TempoOp::Nudge { delta: 1.0 },
                            TempoOp::Link,
                            TempoOp::Audio,
                        ];
                        for c in choices {
                            let same = std::mem::discriminant(op) == std::mem::discriminant(&c);
                            if ui.selectable_label(same, tempo_label(&c)).clicked() && !same {
                                *op = c;
                                out.changed = true;
                            }
                        }
                    });
                    match op {
                        TempoOp::Set { min, max } => {
                            ui.label("最小");
                            out.changed |= ui.add(egui::DragValue::new(min).range(BPM_MIN..=BPM_MAX).suffix(" BPM")).changed();
                            ui.label("最大");
                            out.changed |= ui.add(egui::DragValue::new(max).range(BPM_MIN..=BPM_MAX).suffix(" BPM")).changed();
                        }
                        TempoOp::Nudge { delta } => {
                            ui.label("1回で");
                            out.changed |= ui.add(egui::DragValue::new(delta).range(-50.0..=50.0).speed(0.1).suffix(" BPM")).changed();
                        }
                        _ => {}
                    }
                }
                Action::Chase { name, mode } => {
                    egui::ComboBox::from_id_salt(format!("bchase{i}")).selected_text(name.as_str()).show_ui(ui, |ui| {
                        for s in cx.chases {
                            if ui.selectable_label(name == s, s).clicked() {
                                *name = s.clone();
                                out.changed = true;
                            }
                        }
                    });
                    out.changed |= pad_mode_combo(ui, &format!("bcmode{i}"), mode, false);
                }
                Action::Color { color, target } => {
                    let cur = color.map(|c| c.label()).unwrap_or("色相(フェーダー)");
                    egui::ComboBox::from_id_salt(format!("bcol{i}")).selected_text(cur).width(140.0).show_ui(ui, |ui| {
                        out.changed |= ui.selectable_value(color, None, "色相(フェーダー)").changed();
                        for c in NamedColor::ALL {
                            out.changed |= ui.selectable_value(color, Some(c), c.label()).changed();
                        }
                    });
                    egui::ComboBox::from_id_salt(format!("bcolt{i}")).selected_text(target.label()).width(190.0).show_ui(ui, |ui| {
                        for t in ColorTarget::ALL {
                            out.changed |= ui.selectable_value(target, t, t.label()).changed();
                        }
                    });
                }
                Action::ColorScope { op } => {
                    egui::ComboBox::from_id_salt(format!("bscope{i}")).selected_text(op.label()).width(200.0).show_ui(ui, |ui| {
                        for o in ScopeOp::ALL {
                            out.changed |= ui.selectable_value(op, o, o.label()).changed();
                        }
                    });
                }
                Action::Select { fixture, op } => {
                    let cur = cx.meta.fixtures.iter().find(|f| f.id == *fixture).map(|f| f.name.clone()).unwrap_or_else(|| "(機材を選ぶ)".into());
                    egui::ComboBox::from_id_salt(format!("bsel_fx{i}")).selected_text(cur).width(150.0).show_ui(ui, |ui| {
                        for f in &cx.meta.fixtures {
                            out.changed |= ui.selectable_value(fixture, f.id, f.name.as_str()).changed();
                        }
                    });
                    egui::ComboBox::from_id_salt(format!("bselop{i}")).selected_text(op.label()).width(150.0).show_ui(ui, |ui| {
                        out.changed |= ui.selectable_value(op, SelectOp::Only, SelectOp::Only.label()).changed();
                        out.changed |= ui.selectable_value(op, SelectOp::Toggle, SelectOp::Toggle.label()).changed();
                    });
                }
                Action::Pair { name, op, mode } => {
                    egui::ComboBox::from_id_salt(format!("bpair{i}")).selected_text(name.as_str()).show_ui(ui, |ui| {
                        for s in cx.pairs {
                            if ui.selectable_label(name == s, s).clicked() {
                                *name = s.clone();
                                out.changed = true;
                            }
                        }
                    });
                    egui::ComboBox::from_id_salt(format!("bpop{i}")).selected_text(if *op == PairOp::Link { "連動 ON/OFF" } else { "左右反転 ON/OFF" }).width(130.0).show_ui(ui, |ui| {
                        out.changed |= ui.selectable_value(op, PairOp::Link, "連動 ON/OFF").changed();
                        out.changed |= ui.selectable_value(op, PairOp::Mirror, "左右反転 ON/OFF").changed();
                    });
                    out.changed |= pad_mode_combo(ui, &format!("bpm{i}"), mode, false);
                }
                Action::Position { name } => {
                    egui::ComboBox::from_id_salt(format!("bpos{i}")).selected_text(name.as_str()).show_ui(ui, |ui| {
                        for s in cx.positions {
                            if ui.selectable_label(name == s, s).clicked() {
                                *name = s.clone();
                                out.changed = true;
                            }
                        }
                    });
                    if cx.positions.is_empty() {
                        ui.label(RichText::new("位置プリセットは「コントロール」タブで作ります").weak());
                    }
                }
                Action::Fade { secs, mode } => {
                    ui.label("かける時間");
                    out.changed |= ui.add(egui::DragValue::new(secs).range(0.0..=60.0).speed(0.05).suffix(" 秒")).changed();
                    out.changed |= pad_mode_combo(ui, &format!("bfdm{i}"), mode, false);
                    ui.label(RichText::new("トグル=押すたびに暗転⇄復帰 / 押している間=離すと戻る").weak());
                }
                Action::Flash { white, mode } => {
                    out.changed |= ui.checkbox(white, "白で全開").on_hover_text("OFFなら、今の色のまま全開にします").changed();
                    out.changed |= pad_mode_combo(ui, &format!("bflm{i}"), mode, false);
                    ui.label(RichText::new("レーザーには掛かりません").weak());
                }
                Action::Palette { op } => {
                    egui::ComboBox::from_id_salt(format!("bpal{i}")).selected_text(op.label()).width(230.0).show_ui(ui, |ui| {
                        for o in PaletteOp::ALL {
                            out.changed |= ui.selectable_value(op, o, o.label()).changed();
                        }
                    });
                }
                Action::Page { op } => {
                    egui::ComboBox::from_id_salt(format!("bpgop{i}")).selected_text(op.label()).width(120.0).show_ui(ui, |ui| {
                        for p in 0..PAGES {
                            out.changed |= ui.selectable_value(op, PageOp::Go(p), PageOp::Go(p).label()).changed();
                        }
                        out.changed |= ui.selectable_value(op, PageOp::Next, PageOp::Next.label()).changed();
                        out.changed |= ui.selectable_value(op, PageOp::Prev, PageOp::Prev.label()).changed();
                    });
                }
                Action::Level { sel } => {
                    out.changed |= selector_combo(ui, &format!("blvl{i}"), sel, cx.meta);
                    ui.label(RichText::new("フェーダーで、対象の機材だけの明るさを変えます").weak());
                }
                Action::Mute { sel, mode } => {
                    out.changed |= selector_combo(ui, &format!("bmute{i}"), sel, cx.meta);
                    out.changed |= pad_mode_combo(ui, &format!("bmutem{i}"), mode, false);
                }
                Action::Pixel { param } => {
                    egui::ComboBox::from_id_salt(format!("bpx{i}")).selected_text(param.label()).width(170.0).show_ui(ui, |ui| {
                        for p in crate::pixel::PixelParam::ALL {
                            out.changed |= ui.selectable_value(param, p, p.label()).changed();
                        }
                    });
                    ui.label(RichText::new("動いているLEDバーの模様すべてに掛かります(フェーダー用)").weak());
                }
                Action::Role { sel, op } => {
                    out.changed |= selector_combo(ui, &format!("brole{i}"), sel, cx.meta);
                    egui::ComboBox::from_id_salt(format!("broleop{i}")).selected_text(op.label()).width(180.0).show_ui(ui, |ui| {
                        for o in RoleOp::ALL {
                            out.changed |= ui.selectable_value(op, o, o.label()).changed();
                        }
                    });
                }
                Action::Arm { sel, mode } => {
                    out.changed |= selector_combo(ui, &format!("barm{i}"), sel, cx.meta);
                    out.changed |= pad_mode_combo(ui, &format!("barmm{i}"), mode, false);
                    ui.label(RichText::new("レーザーなど、ARMが必要な機材の出力を許可します").weak());
                }
                Action::LiveColor { role, source } => {
                    use crate::color::ColorRole;
                    egui::ComboBox::from_id_salt(format!("blcr{i}")).selected_text(role.label()).width(80.0).show_ui(ui, |ui| {
                        out.changed |= ui.selectable_value(role, ColorRole::Main, ColorRole::Main.label()).changed();
                        out.changed |= ui.selectable_value(role, ColorRole::Sub, ColorRole::Sub.label()).changed();
                    });
                    egui::ComboBox::from_id_salt(format!("blcs{i}")).selected_text(source.label()).width(170.0).show_ui(ui, |ui| {
                        for s in [crate::ndi::LiveSource::Video, crate::ndi::LiveSource::Audio, crate::ndi::LiveSource::Off] {
                            out.changed |= ui.selectable_value(source, s, s.label()).changed();
                        }
                    });
                }
                Action::SwapBeats { beats } => {
                    out.changed |= ui.add(egui::DragValue::new(beats).range(0.25..=64.0).speed(0.05).suffix(" 拍ごと")).changed();
                }
                Action::BarCount { count } => {
                    out.changed |= ui.add(egui::DragValue::new(count).range(1..=32).suffix(" 本")).changed();
                    ui.label(RichText::new("パッチの順に先頭から使います(残りは消える)").weak());
                }
                Action::Master { mode, on, off } => {
                    out.changed |= pad_mode_combo(ui, &format!("bmm{i}"), mode, true);
                    let (a, c) = if *mode == PadMode::Continuous { ("最大", "最小") } else { ("ON時", "OFF時") };
                    ui.label(a);
                    out.changed |= ui.add(egui::DragValue::new(on).range(0.0..=100.0).suffix(" %")).changed();
                    ui.label(c);
                    out.changed |= ui.add(egui::DragValue::new(off).range(0.0..=100.0).suffix(" %")).changed();
                }
            }
        });
    });
    out
}

fn tempo_label(op: &TempoOp) -> &'static str {
    match op {
        TempoOp::Tap => "TAP(タップテンポ)",
        TempoOp::Resync => "頭出し(小節の頭へ)",
        TempoOp::Set { .. } => "BPM調整(フェーダー)",
        TempoOp::Nudge { .. } => "BPM ± (ボタン)",
        TempoOp::Link => "Ableton Link ON/OFF",
        TempoOp::Audio => "音声から自動BPM ON/OFF",
    }
}

fn action_kind_combo(ui: &mut egui::Ui, i: usize, action: &mut Action, cx: &EditCtx) -> bool {
    let (scenes, effects, chases) = (cx.scenes, cx.effects, cx.chases);
    let mut changed = false;
    egui::ComboBox::from_id_salt(format!("bkind{i}")).selected_text(action.kind_label()).width(120.0).show_ui(ui, |ui| {
        let is = |a: &Action, k: &str| a.kind_label() == k;
        if ui.selectable_label(is(action, "チャンネル値"), "チャンネル値").clicked() && !is(action, "チャンネル値") {
            *action = default_action(false);
            changed = true;
        }
        if ui.selectable_label(is(action, "シーン呼出"), "シーン呼出").clicked() && !is(action, "シーン呼出") {
            *action = Action::Scene { name: scenes.first().cloned().unwrap_or_default() };
            changed = true;
        }
        if ui.selectable_label(is(action, "エフェクト"), "エフェクト").clicked() && !is(action, "エフェクト") {
            *action = Action::Effect { name: effects.first().cloned().unwrap_or_default(), mode: PadMode::Toggle };
            changed = true;
        }
        if ui.selectable_label(is(action, "色(全体制御)"), "色(全体制御)").clicked() && !is(action, "色(全体制御)") {
            *action = Action::Color { color: Some(NamedColor::Red), target: ColorTarget::Scope };
            changed = true;
        }
        if ui.selectable_label(is(action, "メイン/サブ色"), "メイン/サブ色 入れ替え").clicked() && !is(action, "メイン/サブ色") {
            *action = Action::Palette { op: PaletteOp::Swap };
            changed = true;
        }
        if ui.selectable_label(is(action, "色:全体/個別"), "色:全体/個別 スイッチ").clicked() && !is(action, "色:全体/個別") {
            *action = Action::ColorScope { op: ScopeOp::Toggle };
            changed = true;
        }
        if ui.selectable_label(is(action, "機材選択"), "機材選択(個別モード用)").clicked() && !is(action, "機材選択") {
            *action = Action::Select { fixture: cx.meta.fixtures.first().map(|f| f.id).unwrap_or(0), op: SelectOp::Only };
            changed = true;
        }
        if ui.selectable_label(is(action, "ペア"), "ペア(連動・左右反転)").clicked() && !is(action, "ペア") {
            *action = Action::Pair { name: cx.pairs.first().cloned().unwrap_or_default(), op: PairOp::Mirror, mode: PadMode::Toggle };
            changed = true;
        }
        if ui.selectable_label(is(action, "テンポ"), "テンポ(BPM)").clicked() && !is(action, "テンポ") {
            *action = Action::Tempo { op: TempoOp::Tap };
            changed = true;
        }
        if ui.selectable_label(is(action, "チェイス"), "チェイス").clicked() && !is(action, "チェイス") {
            *action = Action::Chase { name: chases.first().cloned().unwrap_or_default(), mode: PadMode::Toggle };
            changed = true;
        }
        if ui.selectable_label(is(action, "位置プリセット"), "位置プリセット").clicked() && !is(action, "位置プリセット") {
            *action = Action::Position { name: cx.positions.first().cloned().unwrap_or_default() };
            changed = true;
        }
        if ui.selectable_label(is(action, "ページ切替"), "ページ切替").clicked() && !is(action, "ページ切替") {
            *action = Action::Page { op: PageOp::Go(0) };
            changed = true;
        }
        if ui.selectable_label(is(action, "全体光量"), "全体光量").clicked() && !is(action, "全体光量") {
            *action = Action::Master { mode: PadMode::Continuous, on: 100.0, off: 0.0 };
            changed = true;
        }
        if ui.selectable_label(is(action, "明るさ(機材ごと)"), "明るさ(機材ごと)").clicked() && !is(action, "明るさ(機材ごと)") {
            *action = Action::Level { sel: Selector::All };
            changed = true;
        }
        if ui.selectable_label(is(action, "ミュート"), "ミュート").clicked() && !is(action, "ミュート") {
            *action = Action::Mute { sel: Selector::All, mode: PadMode::Toggle };
            changed = true;
        }
        if ui.selectable_label(is(action, "LEDバー調整"), "LEDバー調整(速さ・斜め)").clicked() && !is(action, "LEDバー調整") {
            *action = Action::Pixel { param: crate::pixel::PixelParam::Speed };
            changed = true;
        }
        if ui.selectable_label(is(action, "色の系統"), "色の系統(メイン/サブ)").clicked() && !is(action, "色の系統") {
            *action = Action::Role { sel: Selector::All, op: RoleOp::Toggle };
            changed = true;
        }
        if ui.selectable_label(is(action, "ARM"), "ARM(レーザーなど)").clicked() && !is(action, "ARM") {
            *action = Action::Arm { sel: Selector::All, mode: PadMode::Toggle };
            changed = true;
        }
        if ui.selectable_label(is(action, "NDIの色"), "NDIの色(映像・音)").clicked() && !is(action, "NDIの色") {
            *action = Action::LiveColor { role: crate::color::ColorRole::Main, source: crate::ndi::LiveSource::Video };
            changed = true;
        }
        if ui.selectable_label(is(action, "LEDバーの本数"), "LEDバーの本数").clicked() && !is(action, "LEDバーの本数") {
            *action = Action::BarCount { count: 8 };
            changed = true;
        }
        if ui.selectable_label(is(action, "入れ替えの間隔"), "メイン⇄サブ 入れ替えの間隔").clicked() && !is(action, "入れ替えの間隔") {
            *action = Action::SwapBeats { beats: 4.0 };
            changed = true;
        }
        if ui.selectable_label(is(action, "暗転フェード"), "暗転フェード").clicked() && !is(action, "暗転フェード") {
            *action = Action::Fade { secs: 3.0, mode: PadMode::Toggle };
            changed = true;
        }
        if ui.selectable_label(is(action, "フラッシュ"), "フラッシュ(全開)").clicked() && !is(action, "フラッシュ") {
            *action = Action::Flash { white: true, mode: PadMode::Momentary };
            changed = true;
        }
        if ui.selectable_label(is(action, "ブラックアウト"), "ブラックアウト").clicked() && !is(action, "ブラックアウト") {
            *action = Action::Blackout { mode: PadMode::Momentary };
            changed = true;
        }
    });
    changed
}

fn pad_mode_combo(ui: &mut egui::Ui, salt: &str, mode: &mut PadMode, allow_continuous: bool) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(salt).selected_text(mode.label()).width(120.0).show_ui(ui, |ui| {
        for m in PadMode::ALL {
            if m == PadMode::Continuous && !allow_continuous {
                continue;
            }
            changed |= ui.selectable_value(mode, m, m.label()).changed();
        }
    });
    changed
}

/// エディタ用に、Coreから取り出した選択肢の名前
pub struct Names {
    pub scenes: Vec<String>,
    pub effects: Vec<String>,
    pub chases: Vec<String>,
    pub pairs: Vec<String>,
    pub positions: Vec<String>,
}

pub fn names(project: &Project) -> Names {
    let mut effects: Vec<String> = Vec::new();
    // ピクセルエフェクトも、同じ名前の通常のエフェクトとまとめてON/OFFする
    for name in project.effects.iter().map(|e| &e.name).chain(project.pixel_fx.iter().map(|e| &e.name)) {
        if !effects.contains(name) {
            effects.push(name.clone());
        }
    }
    Names {
        scenes: project.scenes.iter().map(|s| s.name.clone()).collect(),
        effects,
        chases: project.chases.iter().map(|c| c.name.clone()).collect(),
        pairs: project.pairs.iter().map(|p| p.name.clone()).collect(),
        positions: project.positions.iter().map(|p| p.name.clone()).collect(),
    }
}
