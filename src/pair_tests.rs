//! ペア(連動・左右反転)のテスト

use crate::engine::*;
use crate::fixture::Library;
use crate::project::*;
use std::path::Path;
use std::sync::Arc;

const SPOT: &str = "stage-evolution/minispot30";
const LASER: &str = "stage-evolution/laserscan300";

/// 9chスポット2台(id1=アドレス1, id2=アドレス10)。1→2のペア(左右反転ON、Panを反転)
fn two_spots() -> Core {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    let mut p = Project::default();
    p.add_fixture(&lib, SPOT).unwrap();
    p.add_fixture(&lib, SPOT).unwrap();
    p.master_seeded = true;
    p.pairs.push(Pair { name: "LR".into(), leader: 1, follower: 2, ..Pair::default() });
    Core::new(p, lib)
}

const F2: usize = 9; // 2台目の先頭スロット

#[test]
fn chained_pairs_follow_in_any_registration_order() {
    for reverse in [false, true] {
        let mut c = two_spots();
        c.project.add_fixture(&c.lib, SPOT).unwrap();
        c.project.pairs.push(Pair { name: "BC".into(), leader: 2, follower: 3, ..Pair::default() });
        if reverse { c.project.pairs.reverse(); }
        c.set_attr(&Selector::Fixture(1), "pan", 20.0);
        c.set_attr(&Selector::Fixture(2), "pan", 40.0);
        c.set_attr(&Selector::Fixture(3), "pan", 60.0);
        c.render();
        assert_eq!((c.rt.frames[0][0], c.rt.frames[0][9], c.rt.frames[0][18]), (20, 235, 20));
        let view = c.paired_view(3).unwrap();
        assert_eq!(view.values[0], 20, "画面も連鎖した値を表示する");
        assert_eq!(c.project.fixtures[2].values[0], 60, "保存する基準値は変えない");

        c.project.pairs.iter_mut().find(|p| p.leader == 1).unwrap().enabled = false;
        c.render();
        assert_eq!(c.rt.frames[0][18], 215, "上流を無効にすると直近のリーダーに追従する");
        assert_eq!(c.paired_view(3).unwrap().values[0], 215);
    }
}

#[test]
fn cyclic_pairs_do_not_block_independent_links() {
    let mut c = two_spots();
    for _ in 0..2 { c.project.add_fixture(&c.lib, SPOT).unwrap(); }
    c.project.pairs.push(Pair { name: "BA".into(), leader: 2, follower: 1, ..Pair::default() });
    c.project.pairs.push(Pair { name: "CD".into(), leader: 3, follower: 4, mirror: false, ..Pair::default() });
    c.set_attr(&Selector::Fixture(1), "pan", 20.0);
    c.set_attr(&Selector::Fixture(2), "pan", 40.0);
    c.set_attr(&Selector::Fixture(3), "pan", 60.0);
    c.render();
    assert_eq!((c.rt.frames[0][0], c.rt.frames[0][9], c.rt.frames[0][27]), (20, 40, 60));
}

fn note(n: u8, v: u8) -> MidiMsg {
    MidiMsg { kind: TriggerKind::Note, channel: 0, number: n, value: v }
}

#[test]
fn follower_mirrors_pan_and_copies_the_rest() {
    let mut c = two_spots();
    c.set_attr(&Selector::Fixture(1), "pan", 40.0);
    c.set_attr(&Selector::Fixture(1), "tilt", 100.0);
    c.set_attr(&Selector::Fixture(1), "dimmer", 200.0);
    c.set_attr(&Selector::Fixture(1), "color", 25.0);
    c.set_attr(&Selector::Fixture(1), "control", 250.0); // リセットのマクロ
    c.set_attr(&Selector::Fixture(2), "control", 3.0);
    c.render();
    let f = &c.rt.frames[0];
    assert_eq!((f[0], f[1], f[5], f[2]), (40, 100, 200, 25), "元の機材はそのまま");
    assert_eq!(f[F2], 215, "Panは反転(255-40)");
    assert_eq!(f[F2 + 1], 100, "Tiltは反転しない");
    assert_eq!(f[F2 + 5], 200, "ディマーは連動");
    assert_eq!(f[F2 + 2], 25, "同じ機種なら、カラーのような選択式も連動");
    assert_eq!(f[F2 + 8], 3, "制御カテゴリ(リセット等)は連動しない");
    assert_eq!(c.project.fixtures[1].values[0], 127, "ペア相手自身の値は書き換えない(ペアを外せば元に戻る)");
}

#[test]
fn mirror_and_link_switches() {
    let mut c = two_spots();
    c.set_attr(&Selector::Fixture(1), "pan", 40.0);
    c.set_attr(&Selector::Fixture(2), "pan", 77.0);

    c.project.pairs[0].mirror = false;
    c.render();
    assert_eq!(c.rt.frames[0][F2], 40, "反転OFFなら、そのまま追従");

    c.project.pairs[0].mirror = true;
    c.project.pairs[0].enabled = false;
    c.render();
    assert_eq!(c.rt.frames[0][F2], 77, "連動OFFなら、ペア相手は自分の値");

    c.project.pairs[0].enabled = true;
    c.project.pairs[0].invert = vec!["pan".into(), "tilt".into()];
    c.set_attr(&Selector::Fixture(1), "tilt", 10.0);
    c.render();
    assert_eq!((c.rt.frames[0][F2], c.rt.frames[0][F2 + 1]), (215, 245), "反転する機能は選べる");
}

#[test]
fn leader_effects_are_mirrored_too() {
    let mut c = two_spots();
    let mut e = Effect::new("sweep", "pan");
    e.enabled = true;
    e.sel = Selector::Fixture(1); // 元の機材だけを動かす
    e.wave = Wave::Saw;
    e.min = 64.0;
    e.max = 192.0;
    e.beats = 4.0;
    c.project.effects.push(e);
    for beat in [0.0, 1.0, 2.0, 3.0] {
        c.rt.beat = beat;
        c.rt.last_beat_t = None;
        c.render();
        let (l, f) = (c.rt.frames[0][0] as i32, c.rt.frames[0][F2] as i32);
        assert!((l + f - 255).abs() <= 1, "拍{beat}: 元{l} ペア相手{f} は左右対称(合計255)のはず");
    }
    assert_eq!(c.rt.frames[0][0] as i32, 64 + 96, "元は拍3で64+128*0.75");
}

#[test]
fn sixteen_bit_pan_mirrors_smoothly() {
    let mut c = two_spots();
    for i in 0..2 {
        c.project.fixtures[i].mode = 1; // 11ch: Pan 16bit
        c.project.fixtures[i].values = c.lib.mode(SPOT, 1).unwrap().default_values();
    }
    c.project.fixtures[1].address = 12;
    c.set_attr(&Selector::Fixture(1), "pan", 100.5);
    c.render();
    let lib = c.lib.clone();
    let m = lib.mode(SPOT, 1).unwrap();
    let follower = m.get_f(&c.rt.frames[0][11..22], 0);
    assert!((follower - (255.996 - 100.5)).abs() < 0.02, "16bitの精度で反転: {follower}");
    // 元の下位バイトが動くと、ペア相手の下位バイトも動く
    c.set_attr(&Selector::Fixture(1), "pan", 100.75);
    c.render();
    let follower2 = m.get_f(&c.rt.frames[0][11..22], 0);
    assert!(follower2 < follower, "元が増えたら、ペア相手は減る");
}

#[test]
fn ranged_channels_invert_only_inside_their_continuous_range() {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    let mut p = Project::default();
    p.add_fixture(&lib, LASER).unwrap(); // id1 アドレス1(17ch)
    p.add_fixture(&lib, LASER).unwrap(); // id2 アドレス18
    p.master_seeded = true;
    // レーザーのパンの128以上(自動の動き)は出力しないので、同じ形のX軸ロールで確かめる
    p.pairs.push(Pair { name: "LR".into(), leader: 1, follower: 2, invert: vec!["x_roll".into()], ..Pair::default() });
    let mut c = Core::new(p, lib);
    let roll_slot = 17 + 7; // 2台目のX軸ロール(ch8)
    c.set_attr(&Selector::Fixture(1), "x_roll", 10.0); // 「角度」0〜127
    c.render();
    assert_eq!(c.rt.frames[0][roll_slot], 117, "0〜127の中で反転(127-10)");
    c.set_attr(&Selector::Fixture(1), "x_roll", 150.0); // 「ロール(A)」のマクロ
    c.render();
    assert_eq!(c.rt.frames[0][roll_slot], 150, "マクロ(選択式)の値は反転しない");
}

#[test]
fn different_models_only_follow_continuous_channels() {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    let mut p = Project::default();
    p.add_fixture(&lib, SPOT).unwrap();
    p.add_fixture(&lib, LASER).unwrap();
    p.master_seeded = true;
    let mut c = Core::new(p, lib);
    let plan = c.pair_plan(&Pair { leader: 1, follower: 2, ..Pair::default() }).unwrap();
    assert!(plan.chans.is_empty(), "スポット→レーザー: レーザーの機能は選択式かスポットに無い");
    let plan = c.pair_plan(&Pair { leader: 2, follower: 1, ..Pair::default() }).unwrap();
    assert!(plan.chans.is_empty(), "レーザー→スポット: 選択式は機種が違うと連動しない");
    // 存在しない機材・同じ機材
    assert!(c.pair_plan(&Pair { leader: 1, follower: 99, ..Pair::default() }).is_none());
    assert!(c.pair_plan(&Pair { leader: 1, follower: 1, ..Pair::default() }).is_none());
    c.project.pairs.push(Pair { leader: 1, follower: 99, ..Pair::default() });
    c.render(); // 壊れたペアがあっても落ちない
}

#[test]
fn control_tab_view_shows_followed_values() {
    let mut c = two_spots();
    c.set_attr(&Selector::Fixture(1), "pan", 40.0);
    assert!(c.paired_view(1).is_none(), "元の機材は連動していない");
    let v = c.paired_view(2).expect("ペア相手");
    assert_eq!(v.leader, c.project.fixtures[0].name);
    assert!(v.mask[0] && v.mask[5], "Pan・ディマーは連動");
    assert!(!v.mask[8], "制御は連動しない");
    assert_eq!(v.values[0], 215);
    assert_eq!(v.inverted, vec!["pan".to_string()]);
    c.project.pairs[0].enabled = false;
    assert!(c.paired_view(2).is_none());
}

#[test]
fn master_dimmer_applies_equally_to_both() {
    let mut c = two_spots();
    c.set_attr(&Selector::Fixture(1), "dimmer", 200.0);
    c.rt.master = 0.5;
    c.render();
    assert_eq!(c.rt.frames[0][5], 100);
    assert_eq!(c.rt.frames[0][F2 + 5], 100);
}

#[test]
fn midi_can_switch_link_and_mirror() {
    let mut c = two_spots();
    c.project.bindings.clear();
    for (n, op, mode) in [(1, PairOp::Mirror, PadMode::Toggle), (2, PairOp::Link, PadMode::Momentary)] {
        c.project.bindings.push(Binding { id: 0,
            name: "b".into(),
            trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: n }),
            action: Action::Pair { name: "LR".into(), op, mode },
            page: None,
            osc: None,
        });
    }
    c.handle_midi(note(1, 100));
    c.handle_midi(note(1, 0));
    assert!(!c.project.pairs[0].mirror, "左右反転をOFF");
    c.handle_midi(note(1, 100));
    c.handle_midi(note(1, 0));
    assert!(c.project.pairs[0].mirror, "もう一度でON");

    c.handle_midi(note(2, 100));
    assert!(c.project.pairs[0].enabled);
    c.handle_midi(note(2, 0));
    assert!(!c.project.pairs[0].enabled, "押している間だけ連動、離すとOFF");
}

#[test]
fn invert_helper_math() {
    let (lib, _) = Library::load(Path::new("none"));
    let spot9 = lib.mode(SPOT, 0).unwrap();
    assert_eq!(spot9.invert_f(0, 0.0), 255.0);
    assert_eq!(spot9.invert_f(0, 255.0), 0.0);
    assert_eq!(spot9.invert_f(0, 127.5), 127.5);
    let spot11 = lib.mode(SPOT, 1).unwrap();
    assert!((spot11.invert_f(0, 0.0) - 255.996).abs() < 0.01, "16bitは最大255.996");
    let laser = lib.mode(LASER, 0).unwrap();
    let pan = laser.find_attr("pan").unwrap();
    assert_eq!(laser.invert_f(pan, 0.0), 127.0);
    assert_eq!(laser.invert_f(pan, 64.0), 63.0);
    assert_eq!(laser.invert_f(pan, 200.0), 200.0, "選択式(マクロ)の範囲は変えない");
}

#[test]
fn pairs_persist_and_old_files_load() {
    let c = two_spots();
    let json = serde_json::to_string(&c.project).unwrap();
    let q: Project = serde_json::from_str(&json).unwrap();
    assert_eq!(q.pairs.len(), 1);
    assert_eq!((q.pairs[0].leader, q.pairs[0].follower), (1, 2));
    assert!(q.pairs[0].mirror && q.pairs[0].invert == vec!["pan".to_string()]);

    let old: Project = serde_json::from_str(r#"{"fixtures":[],"bindings":[]}"#).unwrap();
    assert!(old.pairs.is_empty());
    // 項目が欠けたペアも読める
    let p: Pair = serde_json::from_str(r#"{"name":"x","leader":1,"follower":2}"#).unwrap();
    assert!(p.enabled && p.mirror && p.skip_cats == vec!["制御".to_string()]);
}
