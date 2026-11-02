use crate::engine::*;
use crate::fixture::Library;
use crate::project::*;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

const SPOT: &str = "stage-evolution/minispot30";

/// Spot 1 (アドレス1, 9ch) と Laser 1 (アドレス10, 17ch)
fn core() -> Core {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    Core::new(Project::starter(&lib), lib)
}

fn msg(kind: TriggerKind, number: u8, value: u8) -> MidiMsg {
    MidiMsg { kind, channel: 9, number, value }
}

#[test]
fn starter_patch_is_contiguous() {
    let c = core();
    assert_eq!(c.project.fixtures[0].address, 1);
    assert_eq!(c.project.fixtures[1].address, 10);
}

#[test]
fn render_maps_channels_to_addresses() {
    let mut c = core();
    c.set_attr(&Selector::Fixture(1), "dimmer", 200.0);
    c.set_attr(&Selector::Fixture(1), "pan", 50.0);
    c.render();
    // Spot 9ch: ch1=Pan, ch6=Dimmer
    assert_eq!(c.rt.frames[0][0], 50);
    assert_eq!(c.rt.frames[0][5], 200);
    // 使用スロット: レーザーは10..26まで
    assert_eq!(c.rt.used_len[0], 26);
}

#[test]
fn laser_is_off_until_armed_and_blackout_wins() {
    let mut c = core();
    c.render();
    let laser_mode_slot = 9; // アドレス10のch1
    assert_eq!(c.rt.frames[0][laser_mode_slot], 0, "ARM前はレーザーOFF");

    c.project.fixtures[1].armed = true;
    c.render();
    assert_eq!(c.rt.frames[0][laser_mode_slot], 255, "ARM後はDMX制御");

    c.set_attr(&Selector::Fixture(1), "dimmer", 255.0);
    c.rt.blackout = true;
    c.render();
    assert_eq!(c.rt.frames[0][laser_mode_slot], 0, "ブラックアウトはARM中でもレーザーを止める");
    assert_eq!(c.rt.frames[0][5], 0, "ブラックアウトでディマー0");
}

#[test]
fn armed_state_is_not_persisted() {
    let mut c = core();
    c.project.fixtures[1].armed = true;
    let json = serde_json::to_string(&c.project).unwrap();
    let back: Project = serde_json::from_str(&json).unwrap();
    assert!(!back.fixtures[1].armed);
}

#[test]
fn midi_learn_then_toggle_pad() {
    let mut c = core();
    c.project.bindings.clear(); // 初期の全体光量の割り当てを外し、先頭を試験対象にする
    c.project.bindings.push(Binding {
        name: "dimmer toggle".into(),
        trigger: None,
        action: Action::SetValue { sel: Selector::Profile(SPOT.into()), attr: "dimmer".into(), mode: PadMode::Toggle, on: 255.0, off: 0.0 },
        page: None,
        osc: None,
    });
    c.rt.learn = Some(0);
    c.handle_midi(msg(TriggerKind::Note, 36, 100));
    assert_eq!(c.project.bindings[0].trigger.unwrap().number, 36);
    assert!(c.rt.learn.is_none());
    // ラーン中の入力ではアクションは実行されない
    assert_eq!(c.project.fixtures[0].values[5], 0);

    c.handle_midi(msg(TriggerKind::Note, 36, 100));
    c.handle_midi(msg(TriggerKind::Note, 36, 0)); // 離す
    assert_eq!(c.project.fixtures[0].values[5], 255);
    c.handle_midi(msg(TriggerKind::Note, 36, 100));
    c.handle_midi(msg(TriggerKind::Note, 36, 0));
    assert_eq!(c.project.fixtures[0].values[5], 0);
}

#[test]
fn momentary_and_continuous() {
    let mut c = core();
    c.project.bindings.push(Binding {
        name: "strobe flash".into(),
        trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: 37 }),
        action: Action::SetValue { sel: Selector::All, attr: "strobe".into(), mode: PadMode::Momentary, on: 200.0, off: 0.0 },
        page: None,
        osc: None,
    });
    c.project.bindings.push(Binding {
        name: "pan fader".into(),
        trigger: Some(MidiTrigger { kind: TriggerKind::Cc, channel: Some(9), number: 1 }),
        action: Action::SetValue { sel: Selector::Fixture(1), attr: "pan".into(), mode: PadMode::Continuous, on: 255.0, off: 0.0 },
        page: None,
        osc: None,
    });
    c.handle_midi(msg(TriggerKind::Note, 37, 90));
    assert_eq!(c.project.fixtures[0].values[4], 200);
    c.handle_midi(msg(TriggerKind::Note, 37, 0));
    assert_eq!(c.project.fixtures[0].values[4], 0);

    c.handle_midi(msg(TriggerKind::Cc, 1, 127));
    assert_eq!(c.project.fixtures[0].values[0], 255);
    c.handle_midi(msg(TriggerKind::Cc, 1, 0));
    assert_eq!(c.project.fixtures[0].values[0], 0);
    // 別チャンネルのCCには反応しない
    c.handle_midi(MidiMsg { kind: TriggerKind::Cc, channel: 3, number: 1, value: 127 });
    assert_eq!(c.project.fixtures[0].values[0], 0);
}

#[test]
fn scene_fade_moves_continuous_but_jumps_discrete() {
    let mut c = core();
    c.set_attr(&Selector::Fixture(1), "dimmer", 255.0);
    c.set_attr(&Selector::Fixture(1), "color", 25.0); // 緑
    let mut sc = c.capture_scene("A", 0.0, &[1], &Default::default());
    sc.fade = 10.0;
    c.project.scenes.push(sc);

    c.set_attr(&Selector::Fixture(1), "dimmer", 0.0);
    c.set_attr(&Selector::Fixture(1), "color", 65.0);
    c.recall_scene("A");
    assert_eq!(c.project.fixtures[0].values[2], 25, "カラーは即切替");
    assert_eq!(c.project.fixtures[0].values[5], 0, "ディマーはフェード開始直後は0");
    assert!(!c.rt.fades.is_empty());

    // 10秒フェードの半分まで進めた状態を作る
    for f in c.rt.fades.iter_mut() {
        f.start = Instant::now() - Duration::from_secs(5);
    }
    c.render();
    let d = c.project.fixtures[0].values[5];
    assert!((120..=135).contains(&d), "半分付近のはず: {d}");

    for f in c.rt.fades.iter_mut() {
        f.start = Instant::now() - Duration::from_secs(11);
    }
    c.render();
    assert_eq!(c.project.fixtures[0].values[5], 255);
    assert!(c.rt.fades.is_empty());
}

#[test]
fn effect_oscillates_within_range_and_16bit_is_smooth() {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    let mut p = Project::default();
    let id = p.add_fixture(&lib, SPOT).unwrap();
    p.fixtures[0].mode = 1; // 11ch: 16bit Pan/Tilt
    p.fixtures[0].values = lib.mode(SPOT, 1).unwrap().default_values();
    let mut e = Effect::new("pan sweep", "pan");
    e.enabled = true;
    e.sel = Selector::Fixture(id);
    e.min = 64.0;
    e.max = 192.0;
    e.rate_hz = 0.2;
    p.effects.push(e);
    let mut c = Core::new(p, lib);

    let mut seen_fine = std::collections::HashSet::new();
    for k in 0..40 {
        c.rt.t0 = Instant::now() - Duration::from_millis(k * 120);
        c.render();
        let coarse = c.rt.frames[0][0];
        assert!((64..=192).contains(&coarse), "範囲外: {coarse}");
        seen_fine.insert(c.rt.frames[0][1]);
    }
    assert!(seen_fine.len() > 10, "下位バイトが使われていない(16bitで滑らかに動いていない)");
}

#[test]
fn effect_toggle_via_midi() {
    let mut c = core();
    c.project.effects.push(Effect::new("fx", "pan"));
    c.project.bindings.push(Binding {
        name: "fx toggle".into(),
        trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: 40 }),
        action: Action::Effect { name: "fx".into(), mode: PadMode::Toggle },
        page: None,
        osc: None,
    });
    c.handle_midi(msg(TriggerKind::Note, 40, 100));
    c.handle_midi(msg(TriggerKind::Note, 40, 0));
    assert!(c.project.effects[0].enabled);
    c.handle_midi(msg(TriggerKind::Note, 40, 100));
    c.handle_midi(msg(TriggerKind::Note, 40, 0));
    assert!(!c.project.effects[0].enabled);
}

#[test]
fn blackout_via_midi_momentary() {
    let mut c = core();
    c.project.bindings.push(Binding {
        name: "bo".into(),
        trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: 44 }),
        action: Action::Blackout { mode: PadMode::Momentary },
        page: None,
        osc: None,
    });
    c.handle_midi(msg(TriggerKind::Note, 44, 127));
    assert!(c.rt.blackout);
    c.handle_midi(msg(TriggerKind::Note, 44, 0));
    assert!(!c.rt.blackout);
}

#[test]
fn map_state_tracks_pressed_notes_and_fader_values() {
    let mut c = core();
    assert!(!c.rt.notes_down[0x38] && c.rt.cc_value[0x30].is_none());
    c.handle_midi(msg(TriggerKind::Note, 0x38, 100));
    assert!(c.rt.notes_down[0x38]);
    c.handle_midi(msg(TriggerKind::Note, 0x38, 0));
    assert!(!c.rt.notes_down[0x38]);
    c.handle_midi(msg(TriggerKind::Cc, 0x30, 77));
    assert_eq!(c.rt.cc_value[0x30], Some(77));
    // MIDIラーン中でも状態は追跡する(図が止まらないように)
    c.project.bindings.push(Binding { name: "x".into(), trigger: None, action: Action::Blackout { mode: PadMode::Toggle }, page: None, osc: None });
    c.rt.learn = Some(0);
    c.handle_midi(msg(TriggerKind::Note, 0x39, 100));
    assert!(c.rt.notes_down[0x39]);
}

#[test]
fn scene_capture_can_skip_categories_and_refresh_keeps_it_partial() {
    let mut c = core();
    c.set_attr(&Selector::Fixture(1), "pan", 40.0); // 動き
    c.set_attr(&Selector::Fixture(1), "dimmer", 200.0); // 光量
    c.set_attr(&Selector::Fixture(1), "color", 25.0); // 色

    let skip: std::collections::HashSet<String> = ["色".to_string(), "光量".to_string()].into();
    let sc = c.capture_scene("move", 0.0, &[1], &skip);
    assert!(sc.values.iter().any(|v| v.attr == "pan"));
    assert!(sc.values.iter().any(|v| v.attr == "tilt"));
    assert!(!sc.values.iter().any(|v| v.attr == "color" || v.attr == "dimmer"), "スキップしたカテゴリは含まれない");
    c.project.scenes.push(sc);

    // 動きだけのシーンを呼んでも、色と光量は変わらない
    c.set_attr(&Selector::Fixture(1), "pan", 99.0);
    c.recall_scene("move");
    assert_eq!(c.project.fixtures[0].values[0], 40);
    assert_eq!(c.project.fixtures[0].values[5], 200);
    assert_eq!(c.project.fixtures[0].values[2], 25);

    // 上書きしても部分のまま
    c.set_attr(&Selector::Fixture(1), "pan", 70.0);
    c.refresh_scene(0);
    let s = &c.project.scenes[0];
    assert_eq!(s.values.iter().find(|v| v.attr == "pan").unwrap().value, 70.0);
    assert!(!s.values.iter().any(|v| v.attr == "dimmer"));
}

#[test]
fn replacing_library_keeps_values_by_attr() {
    let mut c = core();
    c.set_attr(&Selector::Fixture(1), "dimmer", 180.0);
    c.set_attr(&Selector::Fixture(1), "gobo", 12.0);
    // ディマーの前にチャンネルを1つ挿入した定義に差し替える
    let mut lib = Library::load(Path::new("none")).0;
    let mut p = (**lib.get(SPOT).unwrap()).clone();
    p.modes[0].insert_channel(0, crate::fixture::ChannelDef::blank("Extra", "extra", "その他"));
    lib.profiles.retain(|x| x.id != SPOT);
    lib.profiles.push(Arc::new(p));
    c.replace_library(Arc::new(lib));

    let fx = &c.project.fixtures[0];
    assert_eq!(fx.values.len(), 10);
    assert_eq!(fx.values[0], 0, "新しいチャンネルは初期値");
    let m = c.lib.mode(SPOT, 0).unwrap();
    assert_eq!(fx.values[m.find_attr("dimmer").unwrap()], 180);
    assert_eq!(fx.values[m.find_attr("gobo").unwrap()], 12);
}
