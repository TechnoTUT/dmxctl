//! 色の全体制御・全体/個別スイッチのテスト

use crate::core::color::*;
use crate::core::engine::*;
use crate::core::fixture::{ChannelDef, Library, Mode, Profile};
use crate::core::project::*;
use std::path::Path;
use std::sync::Arc;

const SPOT: &str = "stage-evolution/minispot30";
const LASER: &str = "stage-evolution/laserscan300";

/// スポット(id1) + レーザー(id2) + RGB(id3) + RGBW(id4)
fn core() -> Core {
    let (mut lib, _) = Library::load(Path::new("none"));
    for (id, attrs) in [("t/rgb", vec!["red", "green", "blue"]), ("t/rgbw", vec!["red", "green", "blue", "white"])] {
        let mut m = Mode { name: "m".into(), channels: vec![] };
        for a in attrs {
            m.channels.push(ChannelDef::blank(a, a, "色"));
        }
        lib.profiles.push(Arc::new(Profile { id: id.into(), name: id.into(), manufacturer: String::new(), needs_arm: false, modes: vec![m] }));
    }
    let lib = Arc::new(lib);
    let mut p = Project::default();
    for prof in [SPOT, LASER, "t/rgb", "t/rgbw"] {
        p.add_fixture(&lib, prof).unwrap();
    }
    p.master_seeded = true;
    Core::new(p, lib)
}

fn val(c: &Core, fixture: u32, attr: &str) -> u8 {
    let fx = c.project.fixtures.iter().find(|f| f.id == fixture).unwrap();
    let m = c.lib.mode(&fx.profile, fx.mode).unwrap();
    fx.values[m.find_attr(attr).unwrap()]
}

fn range_label(c: &Core, fixture: u32) -> String {
    let fx = c.project.fixtures.iter().find(|f| f.id == fixture).unwrap();
    let m = c.lib.mode(&fx.profile, fx.mode).unwrap();
    let ch = &m.channels[m.find_attr("color").unwrap()];
    ch.range_of(fx.values[m.find_attr("color").unwrap()]).unwrap().label.clone()
}

fn note(n: u8, v: u8) -> MidiMsg {
    MidiMsg { kind: TriggerKind::Note, channel: 0, number: n, value: v }
}

fn bind(c: &mut Core, num: u8, action: Action) {
    c.project.bindings.push(Binding { id: 0, name: "b".into(), trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: num }), action, page: None, osc: None });
}

#[test]
fn one_color_reaches_every_kind_of_fixture() {
    let mut c = core();
    c.apply_named_color(NamedColor::Blue, ColorTarget::All);
    assert_eq!(range_label(&c, 1), "青", "スポットのカラーホイール");
    assert_eq!(range_label(&c, 2), "青", "レーザーの色レンジ");
    assert_eq!((val(&c, 3, "red"), val(&c, 3, "green"), val(&c, 3, "blue")), (0, 0, 255), "RGB");

    c.apply_named_color(NamedColor::Red, ColorTarget::All);
    assert_eq!((range_label(&c, 1), range_label(&c, 2)), ("赤".to_string(), "赤".to_string()));
    assert_eq!((val(&c, 3, "red"), val(&c, 3, "green"), val(&c, 3, "blue")), (255, 0, 0));

    c.apply_named_color(NamedColor::Green, ColorTarget::All);
    assert_eq!((range_label(&c, 1), range_label(&c, 2)), ("緑".to_string(), "緑".to_string()));
}

#[test]
fn nearest_available_color_is_chosen() {
    let mut c = core();
    // ピンクは、どの機材にも無いので一番近い色になる(スポットにピンクは無い)
    c.apply_named_color(NamedColor::Pink, ColorTarget::All);
    let l1 = range_label(&c, 1);
    assert!(["赤", "マゼンタ", "オレンジ"].contains(&l1.as_str()), "スポットの近い色: {l1}");
    assert!((val(&c, 3, "red"), val(&c, 3, "green"), val(&c, 3, "blue")) == (255, 76, 140), "RGBはそのままの色");
    // 水色 → スポットは「ライトブルー」
    c.apply_named_color(NamedColor::Cyan, ColorTarget::All);
    assert_eq!(range_label(&c, 1), "ライトブルー");
    assert_eq!(range_label(&c, 2), "シアン");
}

#[test]
fn white_uses_open_or_white_and_rgbw_white_channel() {
    let mut c = core();
    c.apply_named_color(NamedColor::Blue, ColorTarget::All);
    c.apply_named_color(NamedColor::White, ColorTarget::All);
    assert_eq!(range_label(&c, 1), "オープン(白)");
    assert_eq!(range_label(&c, 2), "白");
    assert_eq!((val(&c, 3, "red"), val(&c, 3, "green"), val(&c, 3, "blue")), (255, 255, 255));
    // RGBW: 共通成分は白チャンネルへ
    assert_eq!((val(&c, 4, "white"), val(&c, 4, "red"), val(&c, 4, "green"), val(&c, 4, "blue")), (255, 0, 0, 0));
    c.apply_named_color(NamedColor::Orange, ColorTarget::All);
    assert_eq!((val(&c, 4, "white"), val(&c, 4, "red"), val(&c, 4, "green"), val(&c, 4, "blue")), (0, 255, 127, 0));
}

#[test]
fn global_vs_individual_scope() {
    let mut c = core();
    assert!(c.rt.color_global, "起動時は全体");
    assert_eq!(c.color_targets(ColorTarget::Scope), vec![1, 2, 3, 4]);

    c.rt.color_global = false;
    assert_eq!(c.color_targets(ColorTarget::Scope), vec![1], "個別の初期選択は先頭の機材");
    c.apply_named_color(NamedColor::Green, ColorTarget::Scope);
    assert_eq!(range_label(&c, 1), "緑");
    assert_eq!(val(&c, 3, "green"), 0, "選んでいない機材は変わらない");

    c.rt.selected.insert(3);
    c.apply_named_color(NamedColor::Blue, ColorTarget::Scope);
    assert_eq!((range_label(&c, 1), val(&c, 3, "blue")), ("青".to_string(), 255));
    assert_eq!(range_label(&c, 2), "プリプログラム", "レーザーは選ばれていない");

    // 対象の固定指定は、スイッチに関係なく効く
    c.apply_named_color(NamedColor::Red, ColorTarget::All);
    assert_eq!(range_label(&c, 2), "赤");
    c.rt.selected.clear();
    c.rt.selected.insert(4);
    c.apply_named_color(NamedColor::Yellow, ColorTarget::Selected);
    assert_eq!((val(&c, 4, "red"), val(&c, 4, "green")), (255, 255));
    assert_eq!(val(&c, 3, "green"), 0);
}

#[test]
fn hue_fader_sweeps_through_the_colors() {
    let mut c = core();
    c.apply_hue(0.0, ColorTarget::All);
    assert_eq!((val(&c, 3, "red"), val(&c, 3, "green"), val(&c, 3, "blue")), (255, 0, 0), "端は赤");
    c.apply_hue(60.0, ColorTarget::All); // 黄
    assert_eq!((val(&c, 3, "red"), val(&c, 3, "green")), (255, 255));
    c.apply_hue(120.0, ColorTarget::All); // 緑
    assert_eq!((val(&c, 3, "red"), val(&c, 3, "green"), val(&c, 3, "blue")), (0, 255, 0));
    assert_eq!(range_label(&c, 1), "緑");
    c.apply_hue(240.0, ColorTarget::All); // 青
    assert_eq!((val(&c, 3, "green"), val(&c, 3, "blue")), (0, 255));
    assert_eq!(range_label(&c, 1), "青");
    assert!(c.rt.last_color.is_none());
}

#[test]
fn midi_pads_fader_scope_and_selection() {
    let mut c = core();
    c.project.bindings.clear();
    bind(&mut c, 1, Action::Color { color: Some(NamedColor::Red), target: ColorTarget::Scope });
    bind(&mut c, 2, Action::ColorScope { op: ScopeOp::Toggle });
    bind(&mut c, 3, Action::Select { fixture: 2, op: SelectOp::Only });
    bind(&mut c, 4, Action::Select { fixture: 3, op: SelectOp::Toggle });
    bind(&mut c, 5, Action::ColorScope { op: ScopeOp::HoldIndividual });
    c.project.bindings.push(Binding { id: 0,
        name: "hue".into(),
        trigger: Some(MidiTrigger { kind: TriggerKind::Cc, channel: None, number: 9 }),
        action: Action::Color { color: None, target: ColorTarget::All },
        page: None,
        osc: None,
    });

    // 全体: 赤
    c.handle_midi(note(1, 100));
    c.handle_midi(note(1, 0));
    assert_eq!((range_label(&c, 1), range_label(&c, 2), val(&c, 3, "red")), ("赤".to_string(), "赤".to_string(), 255));
    assert_eq!(c.rt.last_color, Some(NamedColor::Red));

    // 個別に切り替え → 機材2(レーザー)だけを選んで青
    c.handle_midi(note(2, 100));
    c.handle_midi(note(2, 0));
    assert!(!c.rt.color_global);
    c.handle_midi(note(3, 100));
    c.handle_midi(note(3, 0));
    assert_eq!(c.rt.selected.iter().copied().collect::<Vec<_>>(), vec![2], "この機材だけを選ぶ");
    bind(&mut c, 6, Action::Color { color: Some(NamedColor::Blue), target: ColorTarget::Scope });
    c.handle_midi(note(6, 100));
    c.handle_midi(note(6, 0));
    assert_eq!((range_label(&c, 1), range_label(&c, 2)), ("赤".to_string(), "青".to_string()));

    // 選択に追加(複数)/もう一度で解除
    c.handle_midi(note(4, 100));
    c.handle_midi(note(4, 0));
    assert!(c.rt.selected.contains(&2) && c.rt.selected.contains(&3));
    c.handle_midi(note(4, 100));
    c.handle_midi(note(4, 0));
    assert!(!c.rt.selected.contains(&3));

    // 全体に戻して、押している間だけ個別
    c.handle_midi(note(2, 100));
    c.handle_midi(note(2, 0));
    assert!(c.rt.color_global);
    c.handle_midi(note(5, 100));
    assert!(!c.rt.color_global, "押している間は個別");
    c.handle_midi(note(5, 0));
    assert!(c.rt.color_global, "離すと全体に戻る");

    // フェーダーで色相
    c.handle_midi(MidiMsg { kind: TriggerKind::Cc, channel: 0, number: 9, value: 0 });
    assert_eq!(val(&c, 3, "red"), 255);
    c.handle_midi(MidiMsg { kind: TriggerKind::Cc, channel: 0, number: 9, value: 64 });
    let (r, g, b) = (val(&c, 3, "red"), val(&c, 3, "green"), val(&c, 3, "blue"));
    assert!(r < 20 && g > 200 && b > 200, "中央付近(約181度)は水色: {r},{g},{b}");
    c.handle_midi(MidiMsg { kind: TriggerKind::Cc, channel: 0, number: 9, value: 127 });
    let (r, g, b) = (val(&c, 3, "red"), val(&c, 3, "green"), val(&c, 3, "blue"));
    assert!(r > 200 && g < 20 && b < 20, "上端(360度)は赤に戻る: {r},{g},{b}");
}

#[test]
fn colors_ride_along_with_scenes_and_the_master_dimmer_is_untouched() {
    let mut c = core();
    c.set_attr(&Selector::Fixture(1), "dimmer", 200.0);
    c.apply_named_color(NamedColor::Blue, ColorTarget::All);
    assert_eq!(val(&c, 1, "dimmer"), 200, "色の操作は明るさに触れない");
    let sc = c.capture_scene("blue", 0.0, &[1, 3], &["動き".to_string(), "光量".to_string(), "形".to_string(), "制御".to_string()].into());
    assert!(sc.values.iter().any(|v| v.attr == "color") && sc.values.iter().any(|v| v.attr == "blue"), "「色」カテゴリだけのシーンにできる");
    assert!(!sc.values.iter().any(|v| v.attr == "dimmer"));
}

#[test]
fn fixtures_without_any_color_capability_are_skipped_safely() {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    let mut p = Project::default();
    p.add_fixture(&lib, SPOT).unwrap();
    let mut c = Core::new(p, lib);
    // ID 99 のような存在しない機材や、空の選択でも落ちない
    c.apply_rgb(&[99], [1.0, 0.0, 0.0]);
    c.rt.color_global = false;
    c.rt.selected.clear();
    c.apply_named_color(NamedColor::Red, ColorTarget::Scope);
    assert_eq!(range_label(&c, 1), "オープン(白)", "選択が空なら何も変えない");
}

#[test]
fn label_rgb_is_strict_about_mixed_labels() {
    assert!(label_rgb("マゼンタ/オレンジ").is_none());
    assert!(label_rgb("レインボー(遅→速)").is_none());
    assert!(label_rgb("カラーチェンジ(遅→速)").is_none());
    assert_eq!(label_rgb("ライトブルー"), Some(NamedColor::Cyan.rgb()));
    assert_eq!(label_rgb("青"), Some(NamedColor::Blue.rgb()));
    assert_eq!(label_rgb("オープン(白)"), Some(NamedColor::White.rgb()));
    assert!(label_rgb("プリプログラム").is_none());
}

#[test]
fn hsv_primaries() {
    assert_eq!(hsv(0.0, 1.0, 1.0), [1.0, 0.0, 0.0]);
    assert_eq!(hsv(120.0, 1.0, 1.0), [0.0, 1.0, 0.0]);
    assert_eq!(hsv(240.0, 1.0, 1.0), [0.0, 0.0, 1.0]);
    assert_eq!(hsv(60.0, 1.0, 1.0), [1.0, 1.0, 0.0]);
    assert_eq!(hsv(0.0, 0.0, 1.0), [1.0, 1.0, 1.0]);
}

#[test]
fn template_places_free_buttons_only() {
    let mut c = core();
    // 0x00(下段の左端のパッド)は使用中
    c.project.bindings.push(Binding { id: 0,
        name: "mine".into(),
        trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: 0x00 }),
        action: Action::Blackout { mode: PadMode::Toggle },
        page: None,
        osc: None,
    });
    let (added, skipped) = add_template(&mut c.project);
    assert_eq!(skipped, 1);
    assert_eq!(added, 7 + 1 + 4 + 1, "色7 + スイッチ1 + 機材選択4(機材が4台) + 色相フェーダー1");
    let mine = c.project.bindings.iter().filter(|b| b.trigger.is_some_and(|t| t.number == 0x00)).count();
    assert_eq!(mine, 1, "使用中のボタンは上書きしない");
    // もう一度実行しても、増えない
    let (added2, _) = add_template(&mut c.project);
    assert_eq!(added2, 0);
}

#[test]
fn old_bindings_and_projects_still_load() {
    let b: Binding = serde_json::from_str(r#"{"name":"x","trigger":null,"action":{"Blackout":{"mode":"Toggle"}}}"#).unwrap();
    assert!(matches!(b.action, Action::Blackout { .. }));
    let a: Action = serde_json::from_str(r#"{"Color":{"color":"Red","target":"Scope"}}"#).unwrap();
    assert!(matches!(a, Action::Color { color: Some(NamedColor::Red), target: ColorTarget::Scope }));
    let h: Action = serde_json::from_str(r#"{"Color":{"color":null,"target":"All"}}"#).unwrap();
    assert!(matches!(h, Action::Color { color: None, .. }));
}

#[test]
fn hue_fader_covers_the_full_360_degree_circle() {
    let mut c = core();
    c.project.bindings.clear();
    c.project.bindings.push(Binding { id: 0,
        name: "H".into(),
        trigger: Some(MidiTrigger { kind: TriggerKind::Cc, channel: None, number: 0x37 }),
        action: Action::Color { color: None, target: ColorTarget::All },
        page: None,
        osc: None,
    });
    let rgb = |c: &Core| (val(c, 3, "red"), val(c, 3, "green"), val(c, 3, "blue"));
    let cc = |c: &mut Core, v: u8| c.handle_midi(MidiMsg { kind: TriggerKind::Cc, channel: 0, number: 0x37, value: v });

    cc(&mut c, 0);
    assert_eq!(rgb(&c), (255, 0, 0));
    assert_eq!(c.rt.hue, 0.0);
    // 三分の一 = 120度 = 緑、三分の二 = 240度 = 青(近似)
    cc(&mut c, 42);
    let (r, g, b) = rgb(&c);
    assert!(g > 240 && r < 20 && b < 20, "約119度は緑: {r},{g},{b}");
    assert!((c.rt.hue - 119.1).abs() < 0.5, "度数が記録される: {}", c.rt.hue);
    cc(&mut c, 85);
    let (r, g, b) = rgb(&c);
    assert!(b > 240 && r < 20 && g < 20, "約241度は青: {r},{g},{b}");
    // 上端は色相環を一周して赤に戻る
    cc(&mut c, 127);
    assert_eq!(rgb(&c), (255, 0, 0));
    assert_eq!(c.rt.hue, 360.0);
    // 途中を滑らかに通る: フェーダーを少しずつ動かしても、隣り合う値の変化は小さい
    let mut prev = None;
    for v in 0..=127u8 {
        cc(&mut c, v);
        let (r, g, b) = rgb(&c);
        if let Some((pr, pg, pb)) = prev {
            let d = (r as i32 - pr as i32).abs() + (g as i32 - pg as i32).abs() + (b as i32 - pb as i32).abs();
            assert!(d <= 12 * 3 + 10, "色が急に飛んだ: {v} の前後 {d}");
        }
        prev = Some((r, g, b));
    }
}

#[test]
fn hue_moves_wheel_fixtures_through_their_colors_in_order() {
    let mut c = core();
    let mut seen: Vec<String> = Vec::new();
    for deg in (0..=360).step_by(10) {
        c.apply_hue(deg as f32, ColorTarget::All);
        let l = range_label(&c, 1);
        if seen.last() != Some(&l) {
            seen.push(l);
        }
    }
    // 赤 → オレンジ → 黄 → 緑 → ライトブルー → 青 → 紫 → マゼンタ(→ 赤)
    assert_eq!(seen.first().map(String::as_str), Some("赤"));
    for want in ["黄", "緑", "ライトブルー", "青", "マゼンタ"] {
        assert!(seen.iter().any(|s| s == want), "{want} を通るはず: {seen:?}");
    }
    let pos = |name: &str| seen.iter().position(|s| s == name).unwrap();
    assert!(pos("黄") < pos("緑") && pos("緑") < pos("ライトブルー") && pos("ライトブルー") < pos("青") && pos("青") < pos("マゼンタ"));
}

// ---- 範囲ごとの色(カラーピッカー) ----

fn custom_range(from: u8, to: u8, label: &str, color: Option<[u8; 3]>) -> crate::core::fixture::Range {
    crate::core::fixture::Range { from, to, label: label.into(), sweep: false, warn: false, color }
}

/// 表示名がバラバラな「color」チャンネルを持つ機材
fn custom_core() -> Core {
    let (mut lib, _) = Library::load(Path::new("none"));
    let mut m = Mode { name: "m".into(), channels: vec![] };
    let mut ch = ChannelDef::blank("Color", "color", "色");
    ch.ranges = vec![
        custom_range(0, 9, "ネオングリーン", Some([0, 255, 0])),      // 名前では判定できない → 色の指定で使える
        custom_range(10, 19, "赤", Some([0, 0, 255])),                  // 名前は赤でも、色の指定(青)が優先
        custom_range(20, 29, "マゼンタ/オレンジ", Some([255, 128, 0])), // 複数色の名前でも、色の指定があれば使える
        custom_range(30, 39, "レインボー", None),                        // 色なし → 対象外
        custom_range(40, 49, "紫", None),                                // 指定なし → 名前から判定
        custom_range(50, 255, "ミステリー", None),
    ];
    m.channels.push(ch);
    lib.profiles.push(Arc::new(Profile { id: "t/custom".into(), name: "custom".into(), manufacturer: String::new(), needs_arm: false, modes: vec![m] }));
    let lib = Arc::new(lib);
    let mut p = Project::default();
    p.add_fixture(&lib, "t/custom").unwrap();
    p.master_seeded = true;
    Core::new(p, lib)
}

fn custom_val(c: &Core) -> u8 {
    c.project.fixtures[0].values[0]
}

#[test]
fn explicit_range_color_beats_the_label_and_works_with_any_name() {
    let mut c = custom_core();
    c.apply_named_color(NamedColor::Green, ColorTarget::All);
    assert_eq!(custom_val(&c), 4, "名前が「ネオングリーン」でも、色の指定(緑)で選ばれる");
    c.apply_named_color(NamedColor::Blue, ColorTarget::All);
    assert_eq!(custom_val(&c), 14, "名前は「赤」でも、色の指定(青)が優先される");
    c.apply_named_color(NamedColor::Orange, ColorTarget::All);
    assert_eq!(custom_val(&c), 24, "「マゼンタ/オレンジ」でも、色を指定してあれば使える");
    c.apply_named_color(NamedColor::Violet, ColorTarget::All);
    assert_eq!(custom_val(&c), 44, "色の指定がなければ名前(紫)から判定する");
    // 色がなく名前でも判定できない範囲(レインボー・ミステリー)は、どの色でも選ばれない
    for n in NamedColor::ALL {
        c.apply_named_color(n, ColorTarget::All);
        let v = custom_val(&c);
        assert!(!(30..=39).contains(&v) && v < 50, "{n:?} で色なしの範囲が選ばれた: {v}");
    }
}

#[test]
fn removing_the_explicit_color_falls_back_to_the_label() {
    let mut c = custom_core();
    let mut lib = Library::load(Path::new("none")).0;
    let mut p = (**c.lib.get("t/custom").unwrap()).clone();
    p.modes[0].channels[0].ranges[1].color = None; // 「赤」の範囲の色の指定を外す
    lib.profiles.push(Arc::new(p));
    c.replace_library(Arc::new(lib));
    c.apply_named_color(NamedColor::Blue, ColorTarget::All);
    assert_ne!(custom_val(&c), 14, "青はもう「赤」の範囲ではない");
    c.apply_named_color(NamedColor::Red, ColorTarget::All);
    assert_eq!(custom_val(&c), 14, "名前どおり、赤の範囲になる");
}

#[test]
fn hue_fader_uses_explicit_colors_too() {
    let mut c = custom_core();
    c.apply_hue(120.0, ColorTarget::All);
    assert_eq!(custom_val(&c), 4, "緑の色相 → ネオングリーン(色指定)");
    c.apply_hue(240.0, ColorTarget::All);
    assert_eq!(custom_val(&c), 14, "青の色相 → 色指定が青の範囲");
}

#[test]
fn led_uses_the_nearest_palette_color_of_the_explicit_color() {
    use crate::core::led::*;
    let mut c = custom_core();
    // パレット(128色)で光らせるとき
    c.project.led_true_color = false;
    c.project.bindings.clear();
    for (num, on) in [(0u8, 4.0f32), (1, 14.0), (2, 24.0), (3, 34.0)] {
        c.project.bindings.push(Binding { id: 0,
            name: "b".into(),
            trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: num }),
            action: Action::SetValue { sel: Selector::All, attr: "color".into(), mode: PadMode::Trigger, on, off: 0.0 },
            page: None,
            osc: None,
        });
    }
    let col = |n: u8| match compute(&c).into_iter().find(|(x, _)| *x == n).unwrap().1 {
        Led::Rgb { color, .. } => color,
        other => panic!("{other:?}"),
    };
    assert_eq!(col(0), C_GREEN, "緑を指定した範囲は緑で光る");
    assert_eq!(col(1), C_BLUE, "名前は赤でも、色の指定(青)で光る");
    assert_eq!(col(2), C_ORANGE, "オレンジを指定した範囲はオレンジで光る");
    assert_eq!(col(3), C_BLUE, "色の分からない範囲は、既定の青");
}

#[test]
fn nearest_named_color() {
    assert_eq!(nearest_named([0.0, 1.0, 0.0]), NamedColor::Green);
    assert_eq!(nearest_named([0.9, 0.1, 0.1]), NamedColor::Red);
    assert_eq!(nearest_named([1.0, 0.55, 0.05]), NamedColor::Orange);
    assert_eq!(nearest_named([0.95, 0.95, 0.95]), NamedColor::White);
}

#[test]
fn range_color_serialization_is_optional_and_backward_compatible() {
    // 古い定義(colorなし)も読める。colorなしの範囲は、保存してもキーを出さない
    let old: crate::core::fixture::Range = serde_json::from_str(r#"{"from":0,"to":9,"label":"赤"}"#).unwrap();
    assert!(old.color.is_none());
    assert!(!serde_json::to_string(&old).unwrap().contains("color"));
    let with: crate::core::fixture::Range = serde_json::from_str(r#"{"from":0,"to":9,"label":"x","color":[1,2,3]}"#).unwrap();
    assert_eq!(with.color, Some([1, 2, 3]));
    let back: crate::core::fixture::Range = serde_json::from_str(&serde_json::to_string(&with).unwrap()).unwrap();
    assert_eq!(back.color, Some([1, 2, 3]));
}

#[test]
fn builtin_laser_colors_are_recognized_from_their_labels() {
    // 内蔵のレーザー定義は、名前から色が判定できる(色の指定なしでも動く)
    let (lib, _) = Library::load(Path::new("none"));
    let m = lib.mode(LASER, 0).unwrap();
    let ch = &m.channels[m.find_attr("color").unwrap()];
    let found: Vec<&str> = ch.ranges.iter().filter(|r| range_rgb(r).is_some()).map(|r| r.label.as_str()).collect();
    assert_eq!(found, vec!["赤", "緑", "黄", "青", "紫", "シアン", "白"]);
}
