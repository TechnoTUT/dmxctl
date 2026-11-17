//! 全体光量(マスター)のテスト

use crate::engine::*;
use crate::fixture::{ChannelDef, Library, Mode, Profile};
use crate::project::*;
use std::path::Path;
use std::sync::Arc;

const SPOT: &str = "stage-evolution/minispot30";

fn core() -> Core {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    Core::new(Project::starter(&lib), lib)
}

fn cc(number: u8, value: u8) -> MidiMsg {
    MidiMsg { kind: TriggerKind::Cc, channel: 0, number, value }
}

#[test]
fn starter_assigns_master_to_rightmost_fader() {
    let c = core();
    let b = c.project.bindings.iter().find(|b| matches!(b.action, Action::Master { .. })).expect("全体光量の割り当て");
    let t = b.trigger.unwrap();
    assert_eq!((t.kind, t.number), (TriggerKind::Cc, 0x38), "右端(マスター)フェーダー");
    assert!(matches!(b.action, Action::Master { mode: PadMode::Continuous, on, off } if on == 100.0 && off == 0.0));
}

#[test]
fn master_fader_scales_dimmer_only() {
    let mut c = core();
    c.set_attr(&Selector::Fixture(1), "dimmer", 200.0);
    c.set_attr(&Selector::Fixture(1), "pan", 50.0);
    c.set_attr(&Selector::Fixture(1), "strobe", 100.0);
    c.render();
    assert_eq!(c.rt.frames[0][5], 200, "起動時は全開");

    c.handle_midi(cc(0x38, 127));
    c.render();
    assert_eq!(c.rt.frames[0][5], 200);

    c.handle_midi(cc(0x38, 64));
    c.render();
    let d = c.rt.frames[0][5];
    assert!((99..=101).contains(&d), "約半分: {d}");
    assert_eq!(c.rt.frames[0][0], 50, "Panには影響しない");
    assert_eq!(c.rt.frames[0][4], 100, "ストロボにも影響しない");
    assert_eq!(c.project.fixtures[0].values[5], 200, "元の値(ベース値)は変えない");

    c.handle_midi(cc(0x38, 0));
    c.render();
    assert_eq!(c.rt.frames[0][5], 0);

    c.handle_midi(cc(0x38, 127));
    c.render();
    assert_eq!(c.rt.frames[0][5], 200, "戻せば元の値に戻る");
}

#[test]
fn master_leaves_lasers_without_a_dimmer_alone() {
    let mut c = core();
    c.project.fixtures[1].armed = true;
    c.handle_midi(cc(0x38, 0));
    c.render();
    // レーザー(アドレス10〜)にはディマーが無いので、全体光量ではモードなどが変わらない
    assert_eq!(c.rt.frames[0][9], 255);
    assert_eq!(c.rt.frames[0][9 + 13], 64);
}

#[test]
fn master_scales_rgb_when_there_is_no_dimmer_channel() {
    let (mut lib, _) = Library::load(Path::new("none"));
    let mut m = Mode { name: "rgb".into(), channels: vec![] };
    for (n, a) in [("R", "red"), ("G", "green"), ("B", "blue")] {
        m.channels.push(ChannelDef::blank(n, a, "色"));
    }
    lib.profiles.push(Arc::new(Profile { id: "t/rgb".into(), name: "RGB".into(), manufacturer: String::new(), needs_arm: false, modes: vec![m] }));
    let lib = Arc::new(lib);
    let mut p = Project::default();
    let id = p.add_fixture(&lib, "t/rgb").unwrap();
    p.bindings.push(Project::default_master_binding());
    let mut c = Core::new(p, lib);
    c.set_attr(&Selector::Fixture(id), "red", 200.0);
    c.set_attr(&Selector::Fixture(id), "blue", 100.0);
    c.handle_midi(cc(0x38, 64));
    c.render();
    assert!((99..=101).contains(&c.rt.frames[0][0]));
    assert_eq!(c.rt.frames[0][1], 0);
    assert!((49..=51).contains(&c.rt.frames[0][2]));
}

#[test]
fn dimmer_wins_over_rgb_on_fixtures_that_have_both() {
    let (mut lib, _) = Library::load(Path::new("none"));
    let mut m = Mode { name: "rgbd".into(), channels: vec![] };
    for (n, a) in [("Dim", "dimmer"), ("R", "red")] {
        m.channels.push(ChannelDef::blank(n, a, "色"));
    }
    lib.profiles.push(Arc::new(Profile { id: "t/rgbd".into(), name: "RGBD".into(), manufacturer: String::new(), needs_arm: false, modes: vec![m] }));
    let lib = Arc::new(lib);
    let mut p = Project::default();
    let id = p.add_fixture(&lib, "t/rgbd").unwrap();
    p.bindings.push(Project::default_master_binding());
    let mut c = Core::new(p, lib);
    c.set_attr(&Selector::Fixture(id), "dimmer", 200.0);
    c.set_attr(&Selector::Fixture(id), "red", 200.0);
    c.handle_midi(cc(0x38, 64));
    c.render();
    assert!((99..=101).contains(&c.rt.frames[0][0]), "ディマーは絞る");
    assert_eq!(c.rt.frames[0][1], 200, "ディマーがある機材では、赤は絞らない(二重にかからない)");
}

#[test]
fn fader_position_report_syncs_the_master() {
    let mut c = core();
    assert_eq!(c.rt.master, 1.0);
    // 実機のマスターフェーダーが下の方(0)にある状態で接続したら、出力も暗いまま(位置と一致)
    c.set_fader_positions(&[10, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(c.rt.master, 0.0);
    c.set_fader_positions(&[0, 0, 0, 0, 0, 0, 0, 0, 127]);
    assert_eq!(c.rt.master, 1.0);
    // 割り当て済みの他の機能(F1のディマー等)は、接続した瞬間には動かさない
    assert_eq!(c.project.fixtures[0].values[5], 0);
}

#[test]
fn master_modes_for_pads() {
    let mut c = core();
    c.project.bindings.clear();
    c.project.bindings.push(Binding { id: 0,
        name: "half".into(),
        trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: 1 }),
        action: Action::Master { mode: PadMode::Toggle, on: 30.0, off: 100.0 },
        page: None,
        osc: None,
    });
    c.project.bindings.push(Binding { id: 0,
        name: "dark while held".into(),
        trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: 2 }),
        action: Action::Master { mode: PadMode::Momentary, on: 0.0, off: 100.0 },
        page: None,
        osc: None,
    });
    let note = |n, v| MidiMsg { kind: TriggerKind::Note, channel: 0, number: n, value: v };
    c.handle_midi(note(1, 100));
    c.handle_midi(note(1, 0));
    assert!((c.rt.master - 0.3).abs() < 1e-4);
    c.handle_midi(note(1, 100));
    c.handle_midi(note(1, 0));
    assert!((c.rt.master - 1.0).abs() < 1e-4);
    c.handle_midi(note(2, 100));
    assert_eq!(c.rt.master, 0.0);
    c.handle_midi(note(2, 0));
    assert_eq!(c.rt.master, 1.0);
}

#[test]
fn master_binding_is_seeded_once_and_respects_existing_use() {
    // 既存のプロジェクト(全体光量なし)へ、一度だけ追加される
    let mut p = Project::default();
    assert_eq!(p.seed_master_binding(), Some(true));
    assert_eq!(p.bindings.len(), 1);
    p.bindings.clear(); // ユーザーが消した
    assert_eq!(p.seed_master_binding(), None);
    assert!(p.bindings.is_empty(), "消したものは復活しない");

    // 右端のフェーダーが別の用途で使われている場合は、上書きしない
    let mut q = Project::default();
    q.bindings.push(Binding { id: 0,
        name: "mine".into(),
        trigger: Some(MidiTrigger { kind: TriggerKind::Cc, channel: None, number: 0x38 }),
        action: Action::SetValue { sel: Selector::All, attr: "dimmer".into(), mode: PadMode::Continuous, on: 255.0, off: 0.0 },
        page: None,
        osc: None,
    });
    assert_eq!(q.seed_master_binding(), Some(false));
    assert_eq!(q.bindings.len(), 1);
}

#[test]
fn old_project_files_without_master_fields_still_load() {
    let json = r#"{"fixtures":[],"groups":[],"effects":[],"scenes":[],"bindings":[],"outputs":[],"midi_input":"","next_id":1}"#;
    let p: Project = serde_json::from_str(json).unwrap();
    assert!(!p.master_seeded);
    let _ = SPOT;
}
