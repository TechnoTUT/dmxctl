//! パッドLEDの表示内容のテスト

use crate::core::engine::*;
use crate::core::fixture::Library;
use crate::core::led::*;
use crate::core::project::*;
use std::path::Path;
use std::sync::Arc;

const SPOT: &str = "stage-evolution/minispot30";

fn core() -> Core {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    let mut c = Core::new(Project::starter(&lib), lib);
    c.project.bindings.clear();
    c
}

fn bind(c: &mut Core, kind: TriggerKind, num: u8, action: Action) {
    c.project.bindings.push(Binding { id: 0, name: "b".into(), trigger: Some(MidiTrigger { kind, channel: None, number: num }), action, page: None, osc: None });
}

fn led(c: &Core, note: u8) -> Led {
    compute(c).into_iter().find(|(n, _)| *n == note).unwrap().1
}

fn note(n: u8, v: u8) -> MidiMsg {
    MidiMsg { kind: TriggerKind::Note, channel: 0, number: n, value: v }
}

#[test]
fn messages_match_the_protocol_examples() {
    // 仕様書: 「パッド1を赤の点灯100%」= 96 00 05 / 「オレンジ」= 96 00 09 / 「ボリュームボタン点灯」= 90 64 01
    assert_eq!(Led::Rgb { color: C_RED, level: LV_100 }.message(0x00), [0x96, 0x00, 0x05]);
    assert_eq!(Led::Rgb { color: C_ORANGE, level: LV_100 }.message(0x00), [0x96, 0x00, 0x09]);
    assert_eq!(Led::Single(1).message(0x64), [0x90, 0x64, 0x01]);
    assert_eq!(Led::Single(2).message(0x70), [0x90, 0x70, 0x02], "点滅");
    assert_eq!(Led::Off.message(0x38), [0x90, 0x38, 0x00]);
    // 明るさはMIDIチャンネル: 10% = 0x90 / 25% = 0x91 / 50% = 0x92
    assert_eq!(Led::Rgb { color: C_BLUE, level: 0 }.message(1)[0], 0x90);
    assert_eq!(Led::Rgb { color: C_BLUE, level: 1 }.message(1)[0], 0x91);
    assert_eq!(Led::Rgb { color: C_BLUE, level: 2 }.message(1)[0], 0x92);
}

#[test]
fn covers_all_80_controls_once() {
    let c = core();
    let all = compute(&c);
    assert_eq!(all.len(), 64 + 8 + 8);
    let mut notes: Vec<u8> = all.iter().map(|(n, _)| *n).collect();
    notes.sort_unstable();
    notes.dedup();
    assert_eq!(notes.len(), 80);
    assert!(all.iter().all(|(_, l)| *l == Led::Off), "割り当てが無ければ全部消灯");
}

#[test]
fn assigned_pads_are_dim_and_bright_when_active() {
    let mut c = core();
    bind(&mut c, TriggerKind::Note, 0x38, Action::SetValue { sel: Selector::Fixture(1), attr: "dimmer".into(), mode: PadMode::Toggle, on: 255.0, off: 0.0 });
    assert_eq!(led(&c, 0x38), Led::Rgb { color: C_BLUE, level: 1 }, "待機中は25%");
    assert_eq!(led(&c, 0x39), Led::Off, "割り当て無しは消灯");
    c.set_attr(&Selector::Fixture(1), "dimmer", 255.0);
    assert_eq!(led(&c, 0x38), Led::Rgb { color: C_BLUE, level: LV_100 }, "ONの間は100%");
    c.set_attr(&Selector::Fixture(1), "dimmer", 0.0);
    assert_eq!(led(&c, 0x38), Led::Rgb { color: C_BLUE, level: 1 });

    c.project.led_idle = 0;
    assert_eq!(led(&c, 0x38), Led::Rgb { color: C_BLUE, level: 0 });
    c.project.led_idle = 2;
    assert_eq!(led(&c, 0x38), Led::Rgb { color: C_BLUE, level: 2 });
}

#[test]
fn pressed_pad_shows_white() {
    let mut c = core();
    bind(&mut c, TriggerKind::Note, 0x10, Action::Blackout { mode: PadMode::Momentary });
    c.handle_midi(note(0x10, 100));
    assert_eq!(led(&c, 0x10), Led::Rgb { color: C_WHITE, level: LV_100 });
    c.handle_midi(note(0x10, 0));
    assert_eq!(led(&c, 0x10), Led::Rgb { color: C_RED, level: 1 });
}

#[test]
fn color_pads_light_in_the_color_they_select_and_show_the_current_one() {
    let mut c = core();
    // パレット(128色)で光らせるとき
    c.project.led_true_color = false;
    let spot = Selector::Profile(SPOT.into());
    for (num, on) in [(0x00u8, 15.0f32), (0x01, 25.0), (0x02, 35.0), (0x03, 45.0), (0x04, 65.0)] {
        bind(&mut c, TriggerKind::Note, num, Action::SetValue { sel: spot.clone(), attr: "color".into(), mode: PadMode::Trigger, on, off: 0.0 });
    }
    // 赤・緑・青・黄・オレンジ
    let colors: Vec<u8> = (0..5u8).map(|n| match led(&c, n) { Led::Rgb { color, .. } => color, other => panic!("{other:?}") }).collect();
    assert_eq!(colors, vec![C_RED, C_GREEN, C_BLUE, C_YELLOW, C_ORANGE]);

    // いま選ばれている色のパッドだけが明るい(範囲のどこでも同じ色)
    c.set_attr(&spot, "color", 32.0); // 青(30〜39)
    let level = |n: u8| match led(&c, n) { Led::Rgb { level, .. } => level, _ => 99 };
    assert_eq!((level(0), level(1), level(2), level(3), level(4)), (1, 1, LV_100, 1, 1));
}

#[test]
fn state_based_colors_for_each_action_kind() {
    let mut c = core();
    c.project.scenes.push(Scene { name: "A".into(), fade: 0.0, values: vec![] });
    c.project.scenes.push(Scene { name: "B".into(), fade: 0.0, values: vec![] });
    c.project.effects.push(Effect::new("fx", "pan"));
    c.project.chases.push(Chase { name: "ch".into(), ..Chase::default() });
    c.project.pairs.push(Pair { name: "LR".into(), leader: 1, follower: 2, enabled: false, ..Pair::default() });
    bind(&mut c, TriggerKind::Note, 0x20, Action::Scene { name: "A".into() });
    bind(&mut c, TriggerKind::Note, 0x21, Action::Scene { name: "B".into() });
    bind(&mut c, TriggerKind::Note, 0x22, Action::Effect { name: "fx".into(), mode: PadMode::Toggle });
    bind(&mut c, TriggerKind::Note, 0x23, Action::Chase { name: "ch".into(), mode: PadMode::Toggle });
    bind(&mut c, TriggerKind::Note, 0x24, Action::Pair { name: "LR".into(), op: PairOp::Link, mode: PadMode::Toggle });
    bind(&mut c, TriggerKind::Note, 0x25, Action::Blackout { mode: PadMode::Toggle });

    let col = |c: &Core, n: u8| match led(c, n) { Led::Rgb { color, .. } => color, _ => 255 };
    let lvl = |c: &Core, n: u8| match led(c, n) { Led::Rgb { level, .. } => level, _ => 255 };
    // ムービングなどのエフェクトはオレンジ(LEDバーの模様は雰囲気ごとの色。下のテスト)
    assert_eq!([0x20, 0x21, 0x22, 0x23, 0x24, 0x25].map(|n| col(&c, n)), [C_GREEN, C_GREEN, C_ORANGE, C_YELLOW, C_PINK, C_RED]);
    assert!([0x20, 0x21, 0x22, 0x23, 0x24, 0x25].iter().all(|n| lvl(&c, *n) == 1), "最初は全部待機の明るさ");

    // シーン: 最後に呼んだものだけが光る(ラジオボタンのように)
    c.recall_scene("A");
    assert_eq!((lvl(&c, 0x20), lvl(&c, 0x21)), (LV_100, 1));
    c.recall_scene("B");
    assert_eq!((lvl(&c, 0x20), lvl(&c, 0x21)), (1, LV_100));

    c.project.effects[0].enabled = true;
    c.project.chases[0].enabled = true;
    c.project.pairs[0].enabled = true;
    c.rt.blackout = true;
    assert!([0x22, 0x23, 0x24, 0x25].iter().all(|n| lvl(&c, *n) == LV_100), "エフェクト・チェイス・ペア・ブラックアウトが効いている間は最大");
}

#[test]
fn track_and_scene_buttons_are_single_color_leds() {
    let mut c = core();
    c.project.effects.push(Effect::new("fx", "pan"));
    bind(&mut c, TriggerKind::Note, 0x64, Action::Effect { name: "fx".into(), mode: PadMode::Toggle }); // トラック1
    bind(&mut c, TriggerKind::Note, 0x70, Action::Blackout { mode: PadMode::Toggle }); // シーン1
    assert_eq!(led(&c, 0x64), Led::Single(1), "割り当てあり = 点灯");
    assert_eq!(led(&c, 0x65), Led::Off);
    c.project.effects[0].enabled = true;
    assert_eq!(led(&c, 0x64), Led::Single(2), "効いている間 = 点滅");
    assert_eq!(led(&c, 0x70), Led::Single(1));
    c.rt.blackout = true;
    assert_eq!(led(&c, 0x70), Led::Single(2));
}

#[test]
fn tap_pad_flashes_on_the_beat_and_marks_the_downbeat() {
    let mut c = core();
    bind(&mut c, TriggerKind::Note, 0x3F, Action::Tempo { op: TempoOp::Tap });
    let at = |c: &mut Core, beat: f64| {
        c.rt.beat = beat;
        led(c, 0x3F)
    };
    assert_eq!(at(&mut c, 4.05), Led::Rgb { color: C_ORANGE, level: LV_100 }, "小節の頭(1拍目)はオレンジで強く");
    assert_eq!(at(&mut c, 5.05), Led::Rgb { color: C_LIGHT_BLUE, level: LV_100 }, "他の拍は水色で強く");
    assert_eq!(at(&mut c, 5.6), Led::Rgb { color: C_LIGHT_BLUE, level: 1 }, "拍の途中は待機の明るさ");
}

#[test]
fn faders_have_no_leds_and_continuous_pads_stay_dim() {
    let mut c = core();
    bind(&mut c, TriggerKind::Cc, 0x30, Action::SetValue { sel: Selector::All, attr: "dimmer".into(), mode: PadMode::Continuous, on: 255.0, off: 0.0 });
    assert!(compute(&c).iter().all(|(_, l)| *l == Led::Off), "フェーダーにはLEDが無い");
    bind(&mut c, TriggerKind::Note, 0x05, Action::SetValue { sel: Selector::All, attr: "dimmer".into(), mode: PadMode::Continuous, on: 255.0, off: 0.0 });
    assert_eq!(led(&c, 0x05), Led::Rgb { color: C_BLUE, level: 1 });
}

#[test]
fn color_labels() {
    assert_eq!(label_color("赤"), Some(C_RED));
    assert_eq!(label_color("ライトブルー"), Some(C_LIGHT_BLUE));
    assert_eq!(label_color("オレンジ"), Some(C_ORANGE));
    assert_eq!(label_color("オープン(白)"), Some(C_WHITE));
    assert_eq!(label_color("マゼンタ/オレンジ"), Some(C_MAGENTA));
    assert_eq!(label_color("Green"), Some(C_GREEN));
    assert_eq!(label_color("レインボー(遅→速)"), None);
}

#[test]
fn led_settings_persist_with_sensible_defaults() {
    let p = Project::default();
    assert!(p.led_enabled);
    assert_eq!(p.led_idle, 1);
    let old: Project = serde_json::from_str(r#"{"fixtures":[]}"#).unwrap();
    assert!(old.led_enabled, "古い保存ファイルでも、LEDは既定でON");
}

#[test]
fn face_pattern_pads_are_colored_by_their_set() {
    let mut c = core();
    crate::core::pixel::add_face_presets(&mut c.project);
    let names: Vec<&str> = crate::core::pixel::FACE_SETS.iter().map(|s| s.presets[0].name).collect();
    for (k, name) in names.iter().enumerate() {
        bind(&mut c, TriggerKind::Note, 0x30 + k as u8, Action::Effect { name: name.to_string(), mode: PadMode::Toggle });
    }
    let col = |c: &Core, n: u8| match led(c, n) { Led::Rgb { color, .. } => color, _ => 255 };
    assert_eq!([0x30, 0x31, 0x32, 0x33].map(|n| col(&c, n)), [C_LIGHT_BLUE, C_VIOLET, C_MAGENTA, C_WHITE], "チル/グルーヴ/ピーク/モノクロ");
}
