//! メイン色/サブ色(色の2系統)のテスト

use crate::color::{self, ColorRole, ColorTarget, NamedColor, PaletteOp};
use crate::engine::*;
use crate::fixture::Library;
use crate::osc::{Arg, Message};
use crate::project::*;
use std::path::Path;
use std::sync::Arc;

const SPOT: u32 = 1;
const LASER: u32 = 2;

fn core() -> Core {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    Core::new(Project::starter(&lib), lib)
}

/// 機材の「color」チャンネルの今の値
fn color_of(c: &Core, id: u32) -> f32 {
    let fx = &c.project.fixtures[c.project.fixture_index(id).unwrap()];
    let m = c.lib.mode(&fx.profile, fx.mode).unwrap();
    m.get_f(&fx.values, m.find_attr("color").unwrap())
}

/// この機材に`rgb`を出したときの「color」チャンネルの値
fn expected(c: &Core, id: u32, rgb: [f32; 3]) -> f32 {
    let fx = &c.project.fixtures[c.project.fixture_index(id).unwrap()];
    let m = c.lib.mode(&fx.profile, fx.mode).unwrap();
    color::plan_for_mode(m, rgb).into_iter().find(|(a, _)| a == "color").map(|(_, v)| v).unwrap()
}

fn shows(c: &Core, id: u32, col: NamedColor) -> bool {
    color_of(c, id) == expected(c, id, col.rgb())
}

fn note(n: u8, v: u8) -> MidiMsg {
    MidiMsg { kind: TriggerKind::Note, channel: 0, number: n, value: v }
}

fn bind(c: &mut Core, kind: TriggerKind, num: u8, action: Action) {
    c.project.bindings.push(Binding { id: 0, name: "b".into(), trigger: Some(MidiTrigger { kind, channel: None, number: num }), action, page: None, osc: None });
}

/// スポット=メイン、レーザー=サブ、メイン=赤、サブ=青
fn split() -> Core {
    let mut c = core();
    c.set_color_role(LASER, ColorRole::Sub);
    c.set_palette_color(ColorRole::Main, NamedColor::Red.rgb());
    c.set_palette_color(ColorRole::Sub, NamedColor::Blue.rgb());
    c
}

#[test]
fn each_fixture_type_shows_its_own_color() {
    let c = split();
    // 前提: 2機種で、赤と青が別の値になる
    assert_ne!(expected(&c, SPOT, NamedColor::Red.rgb()), expected(&c, SPOT, NamedColor::Blue.rgb()));
    assert!(shows(&c, SPOT, NamedColor::Red), "スポット(メイン)は赤");
    assert!(shows(&c, LASER, NamedColor::Blue), "レーザー(サブ)は青");
}

#[test]
fn swap_flips_main_and_sub_instantly() {
    let mut c = split();
    c.swap_palette();
    assert!(shows(&c, SPOT, NamedColor::Blue));
    assert!(shows(&c, LASER, NamedColor::Red));
    c.swap_palette();
    assert!(shows(&c, SPOT, NamedColor::Red));
    assert!(shows(&c, LASER, NamedColor::Blue));
}

#[test]
fn all_main_fixtures_switch_between_two_color_systems() {
    // 全部メインなら、入れ替えで「全体がメイン色 ⇄ 全体がサブ色」の切り替えになる
    let mut c = core();
    c.set_palette_color(ColorRole::Main, NamedColor::Green.rgb());
    c.set_palette_color(ColorRole::Sub, NamedColor::Magenta.rgb());
    assert!(shows(&c, SPOT, NamedColor::Green));
    assert!(shows(&c, LASER, NamedColor::Green));
    c.swap_palette();
    assert_eq!(color_of(&c, SPOT), expected(&c, SPOT, NamedColor::Magenta.rgb()));
    assert_eq!(color_of(&c, LASER), expected(&c, LASER, NamedColor::Magenta.rgb()));
}

#[test]
fn off_fixtures_are_left_alone() {
    let mut c = core();
    c.set_color_role(LASER, ColorRole::Off);
    let before = color_of(&c, LASER);
    c.set_palette_color(ColorRole::Main, NamedColor::Red.rgb());
    c.set_palette_color(ColorRole::Sub, NamedColor::Green.rgb());
    c.swap_palette();
    assert_eq!(color_of(&c, LASER), before);
    assert!(c.role_rgb(ColorRole::Off).is_none());
}

#[test]
fn midi_pads_set_colors_and_swap() {
    let mut c = core();
    c.set_color_role(LASER, ColorRole::Sub);
    bind(&mut c, TriggerKind::Note, 8, Action::Color { color: Some(NamedColor::Yellow), target: ColorTarget::Main });
    bind(&mut c, TriggerKind::Note, 16, Action::Color { color: Some(NamedColor::Violet), target: ColorTarget::Sub });
    bind(&mut c, TriggerKind::Note, 0x76, Action::Palette { op: PaletteOp::Swap });
    bind(&mut c, TriggerKind::Note, 0x75, Action::Palette { op: PaletteOp::HoldSwap });
    c.handle_midi(note(8, 127));
    c.handle_midi(note(8, 0));
    c.handle_midi(note(16, 127));
    c.handle_midi(note(16, 0));
    assert!(shows(&c, SPOT, NamedColor::Yellow));
    assert!(shows(&c, LASER, NamedColor::Violet));
    assert_eq!(c.rt.last_color, None, "メイン/サブの操作は、直接の色パッドの点灯には影響しない");

    // LED: 今のメイン色のパッドが明るく光る
    let led = |c: &Core, n: u8| crate::led::compute(c).into_iter().find(|(x, _)| *x == n).unwrap().1;
    assert!(led(&c, 8).is_full());

    c.handle_midi(note(0x76, 127));
    c.handle_midi(note(0x76, 0));
    assert!(shows(&c, SPOT, NamedColor::Violet));
    assert_eq!(led(&c, 0x76), crate::led::Led::Single(2), "入れ替え中は点滅");

    // 押している間だけ入れ替え(今は入れ替え中なので、押すと元に戻り、離すとまた入れ替わる)
    c.handle_midi(note(0x75, 127));
    assert!(shows(&c, SPOT, NamedColor::Yellow));
    c.handle_midi(note(0x75, 0));
    assert!(shows(&c, SPOT, NamedColor::Violet));
}

#[test]
fn hue_fader_sets_main_color() {
    let mut c = core();
    bind(&mut c, TriggerKind::Cc, 0x35, Action::Color { color: None, target: ColorTarget::Main });
    c.handle_midi(MidiMsg { kind: TriggerKind::Cc, channel: 0, number: 0x35, value: 127 });
    assert!(color::same_rgb(c.project.palette.main, NamedColor::Red.rgb()), "上端=360°=赤");
    c.apply_hue(240.0, ColorTarget::Sub);
    assert!(color::same_rgb(c.project.palette.sub, NamedColor::Blue.rgb()));
    assert_eq!(c.rt.hue, 0.0, "直接の色相スライダーの表示は変えない");
}

#[test]
fn auto_swap_follows_the_beat() {
    let mut c = split();
    c.project.palette.auto_beats = 1.0;
    c.set_palette_auto(true);
    c.dirty = false;
    c.rt.beat = 0.3;
    c.render();
    assert!(shows(&c, SPOT, NamedColor::Red));
    c.rt.beat = 1.3;
    c.render();
    assert!(shows(&c, SPOT, NamedColor::Blue), "1拍ごとに入れ替わる");
    assert!(shows(&c, LASER, NamedColor::Red));
    c.rt.beat = 2.3;
    c.render();
    assert!(shows(&c, SPOT, NamedColor::Red));
    assert!(!c.dirty, "拍による切り替えは自動保存のきっかけにしない");
    c.set_palette_auto(false);
    c.rt.beat = 3.3;
    c.render();
    assert!(shows(&c, SPOT, NamedColor::Red), "OFFにしたら止まる");
}

#[test]
fn palette_does_nothing_until_used() {
    let mut c = core();
    let (s, l) = (color_of(&c, SPOT), color_of(&c, LASER));
    for b in [0.5, 1.5, 2.5, 5.5] {
        c.rt.beat = b;
        c.render();
    }
    assert_eq!((color_of(&c, SPOT), color_of(&c, LASER)), (s, l));
}

#[test]
fn osc_controls_main_sub_and_swap() {
    let mut c = core();
    c.set_color_role(LASER, ColorRole::Sub);
    let m = |a: &str, args: Vec<Arg>| Message { addr: a.into(), args };
    c.handle_osc(&m("/dmx/main", vec![Arg::Str("green".into())]));
    c.handle_osc(&m("/dmx/sub", vec![Arg::Str("赤".into())]));
    assert!(shows(&c, SPOT, NamedColor::Green));
    assert!(shows(&c, LASER, NamedColor::Red));
    c.handle_osc(&m("/dmx/swap", vec![]));
    assert!(shows(&c, SPOT, NamedColor::Red));
    c.handle_osc(&m("/dmx/sub/hue", vec![Arg::Float(1.0 / 3.0)]));
    assert!(color::same_rgb(c.project.palette.sub, NamedColor::Green.rgb()));
    c.handle_osc(&m("/dmx/swap/auto", vec![Arg::Float(1.0)]));
    assert!(c.project.palette.auto);
    c.handle_osc(&m("/dmx/swap/auto", vec![Arg::Float(0.0)]));
    assert!(!c.project.palette.auto);
}

#[test]
fn template_places_main_sub_controls() {
    let mut p = Project::default();
    let (added, skipped) = color::add_palette_template(&mut p);
    assert_eq!((added, skipped), (20, 0));
    let find = |n: u8, k: TriggerKind| p.bindings.iter().find(|b| b.trigger.is_some_and(|t| t.kind == k && t.number == n)).map(|b| b.action.clone());
    assert!(matches!(find(8, TriggerKind::Note), Some(Action::Color { color: Some(NamedColor::Red), target: ColorTarget::Main })));
    assert!(matches!(find(16, TriggerKind::Note), Some(Action::Color { color: Some(NamedColor::Red), target: ColorTarget::Sub })));
    assert!(matches!(find(0x76, TriggerKind::Note), Some(Action::Palette { op: PaletteOp::Swap })));
    assert!(matches!(find(0x35, TriggerKind::Cc), Some(Action::Color { color: None, target: ColorTarget::Main })));
    // 使用中なら代わりのボタンへ(シーンボタン7 → トラックボタン8)
    let mut q = Project::default();
    q.bindings.push(Binding { id: 0, name: "x".into(), trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: 0x76 }), action: Action::Blackout { mode: PadMode::Toggle }, page: None, osc: None });
    color::add_palette_template(&mut q);
    assert!(q.bindings.iter().any(|b| b.trigger.is_some_and(|t| t.number == 0x6B) && matches!(b.action, Action::Palette { op: PaletteOp::Swap })));
}

#[test]
fn hue_of_matches_hsv() {
    for h in [0.0f32, 30.0, 120.0, 200.0, 300.0] {
        assert!((color::hue_of(color::hsv(h, 1.0, 1.0)) - h).abs() < 0.5, "{h}");
    }
    assert_eq!(color::hue_of([1.0, 1.0, 1.0]), 0.0);
}

#[test]
fn old_projects_default_to_main_role_and_default_palette() {
    let json = r#"{"fixtures":[{"id":1,"name":"a","profile":"stage-evolution/minispot30","mode":0,"universe":0,"address":1,"values":[]}]}"#;
    let p: Project = serde_json::from_str(json).unwrap();
    assert_eq!(p.fixtures[0].color_role, ColorRole::Main);
    assert_eq!(p.palette, color::Palette::default());
}
