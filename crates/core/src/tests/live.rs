//! 「今の色」表示・フラッシュ・バックアップのテスト

use crate::color::{ColorTarget, NamedColor};
use crate::engine::*;
use crate::fixture::Library;
use crate::look::{looks, Look};
use crate::project::*;
use std::path::Path;
use std::sync::Arc;

const SPOT: u32 = 1;
const BAR: &str = "generic/led-bar-rgb";

fn core() -> Core {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    Core::new(Project::starter(&lib), lib)
}

fn add_bar(c: &mut Core, pixels: usize) -> u32 {
    let lib = c.lib.clone();
    let id = c.project.add_fixture(&lib, BAR).unwrap();
    let mode = lib.get(BAR).unwrap().modes.iter().position(|m| m.pixels().len() == pixels).unwrap();
    let i = c.project.fixture_index(id).unwrap();
    c.project.fixtures[i].mode = mode;
    c.project.fixtures[i].values = lib.mode(BAR, mode).unwrap().default_values();
    id
}

fn out(c: &Core, id: u32, attr: &str) -> u8 {
    let f = &c.project.fixtures[c.project.fixture_index(id).unwrap()];
    let m = c.lib.mode(&f.profile, f.mode).unwrap();
    c.rt.frames[f.universe][f.address as usize - 1 + m.find_attr(attr).unwrap()]
}

fn look_of(c: &Core, name: &str) -> (Look, Option<&'static str>) {
    let l = looks(c).into_iter().find(|l| l.name.starts_with(name)).unwrap();
    (l.look, l.note)
}

#[test]
fn look_reads_actual_output() {
    let mut c = core();
    let bar = add_bar(&mut c, 4);
    c.apply_named_color(NamedColor::Red, ColorTarget::All);
    c.set_attr(&Selector::Fixture(SPOT), "dimmer", 255.0);
    c.render();
    match look_of(&c, "MINI SPOT").0 {
        Look::One { rgb, level, label } => {
            assert_eq!(rgb, Some(NamedColor::Red.rgb()), "{label}");
            assert_eq!(level, 1.0);
        }
        l => panic!("{l:?}"),
    }
    let (laser, note) = look_of(&c, "LASER");
    assert_eq!(note, Some("SAFE"), "ARMしていないレーザーは出ていない");
    assert!(matches!(laser, Look::One { level, .. } if level == 0.0));
    assert!(matches!(look_of(&c, "LEDバー").0, Look::Strip(ref px) if px.len() == 4 && px.iter().all(|p| *p == [1.0, 0.0, 0.0])));

    // 全体光量は明るさに出る
    c.rt.master = 0.5;
    c.render();
    assert!(matches!(look_of(&c, "MINI SPOT").0, Look::One { level, .. } if (level - 0.5).abs() < 0.01));
    // ブラックアウト
    c.rt.blackout = true;
    c.render();
    let (spot, note) = look_of(&c, "MINI SPOT");
    assert_eq!(note, Some("BLACKOUT"));
    assert!(matches!(spot, Look::One { level, .. } if level == 0.0));
    assert!(matches!(look_of(&c, "LEDバー").0, Look::Strip(ref px) if px.iter().all(|p| *p == [0.0; 3])));
    let _ = bar;
}

#[test]
fn flash_goes_full_without_touching_base_values() {
    let mut c = core();
    let bar = add_bar(&mut c, 4);
    c.apply_named_color(NamedColor::Blue, ColorTarget::All);
    c.set_attr(&Selector::Fixture(SPOT), "dimmer", 30.0);
    // バーは暗めの青
    c.set_attr(&Selector::Fixture(bar), "blue", 100.0);
    c.rt.master = 0.4;
    c.project.bindings.push(Binding { id: 0,
        name: "f".into(),
        trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: 7 }),
        action: Action::Flash { white: false, mode: PadMode::Momentary },
        page: None,
        osc: None,
    });
    let base = c.project.fixtures.clone();
    let laser_mode_before = {
        c.render();
        out(&c, 2, "mode")
    };

    c.handle_midi(MidiMsg { kind: TriggerKind::Note, channel: 0, number: 7, value: 127 });
    c.render();
    assert_eq!(out(&c, SPOT, "dimmer"), 255, "全体光量より優先して全開");
    assert_eq!(out(&c, bar, "blue.1"), 255, "今の色(青)のまま全開");
    assert_eq!(out(&c, bar, "red.1"), 0);
    assert_eq!(out(&c, 2, "mode"), laser_mode_before, "レーザーには掛からない");
    let led = crate::led::compute(&c).into_iter().find(|(n, _)| *n == 7).unwrap().1;
    assert!(matches!(led, crate::led::Led::Rgb { level: crate::led::LV_100, .. }));

    c.handle_midi(MidiMsg { kind: TriggerKind::Note, channel: 0, number: 7, value: 0 });
    c.render();
    assert_eq!(out(&c, SPOT, "dimmer"), 12, "離すと元の明るさ(30×40%)に戻る");
    assert_eq!(c.project.fixtures.iter().map(|f| f.values.clone()).collect::<Vec<_>>(), base.iter().map(|f| f.values.clone()).collect::<Vec<_>>(), "ベース値は変えない");
}

#[test]
fn white_flash_and_blackout_priority() {
    let mut c = core();
    let bar = add_bar(&mut c, 4);
    c.apply_named_color(NamedColor::Red, ColorTarget::All);
    c.rt.flash = Some(true);
    c.render();
    assert_eq!((out(&c, bar, "red.2"), out(&c, bar, "green.2"), out(&c, bar, "blue.2")), (255, 255, 255));
    let spot = &c.project.fixtures[0];
    let m = c.lib.mode(&spot.profile, spot.mode).unwrap();
    let white = crate::color::plan_for_mode(m, [1.0; 3]).into_iter().find(|(a, _)| a == "color").unwrap().1;
    assert_eq!(out(&c, SPOT, "color") as f32, white, "カラーホイールは白");
    // ブラックアウトはフラッシュより優先
    c.rt.blackout = true;
    c.render();
    assert_eq!(out(&c, SPOT, "dimmer"), 0);
    assert_eq!(out(&c, bar, "red.1"), 0);
}

#[test]
fn flash_toggle_and_osc() {
    let mut c = core();
    c.project.bindings.push(Binding { id: 0,
        name: "f".into(),
        trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: 7 }),
        action: Action::Flash { white: true, mode: PadMode::Toggle },
        page: None,
        osc: None,
    });
    let hit = |c: &mut Core| {
        c.handle_midi(MidiMsg { kind: TriggerKind::Note, channel: 0, number: 7, value: 127 });
        c.handle_midi(MidiMsg { kind: TriggerKind::Note, channel: 0, number: 7, value: 0 });
    };
    hit(&mut c);
    assert_eq!(c.rt.flash, Some(true));
    hit(&mut c);
    assert_eq!(c.rt.flash, None);
    let m = |a: &str, v: f64| crate::io::osc::Message { addr: a.into(), args: vec![crate::io::osc::Arg::Float(v)] };
    c.handle_osc(&m("/dmx/flash/color", 1.0));
    assert_eq!(c.rt.flash, Some(false));
    c.handle_osc(&m("/dmx/flash", 0.0));
    assert_eq!(c.rt.flash, None);
}

#[test]
fn backups_are_kept_and_pruned() {
    let dir = std::env::temp_dir().join(format!("dmxctl-bk-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("project.json");
    let bk = dir.join("backups");
    assert!(backup_to(&file, &bk, 3).is_none(), "まだファイルが無ければ何もしない");
    for k in 0..5 {
        std::fs::write(&file, format!("{{\"v\":{k}}}")).unwrap();
        let b = backup_to(&file, &bk, 3).unwrap();
        assert_eq!(std::fs::read_to_string(&b).unwrap(), format!("{{\"v\":{k}}}"));
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let mut left: Vec<String> = std::fs::read_dir(&bk).unwrap().flatten().map(|e| std::fs::read_to_string(e.path()).unwrap()).collect();
    left.sort();
    assert_eq!(left, ["{\"v\":2}", "{\"v\":3}", "{\"v\":4}"], "新しい3つが残る");
    // 戻す画面の一覧: 新しい順で、このアプリが作ったバックアップだけ
    std::fs::write(bk.join("other.json"), "{}").unwrap();
    let listed: Vec<String> = list_backups(&bk, "project").iter().map(|p| std::fs::read_to_string(p).unwrap()).collect();
    assert_eq!(listed, ["{\"v\":4}", "{\"v\":3}", "{\"v\":2}"]);
    // 他のファイルは消さない
    std::fs::write(bk.join("memo.txt"), "x").unwrap();
    backup_to(&file, &bk, 1);
    assert!(bk.join("memo.txt").exists());
    std::fs::remove_dir_all(&dir).ok();
}

// ------------------------------------------------------------ パッドを実際の色で光らせる

#[test]
fn true_color_sysex_matches_protocol() {
    use crate::led::Led;
    // RGB LED Color Lighting (0x24): 開始/終了パッド + R,G,B を各 上位1bit/下位7bit
    let m = Led::True { rgb: [255, 128, 0], full: true }.message(0x10);
    assert_eq!(m, [0xF0, 0x47, 0x7F, 0x4F, 0x24, 0x00, 0x08, 0x10, 0x10, 0x01, 0x7F, 0x01, 0x00, 0x00, 0x00, 0xF7]);
}

#[test]
fn color_pads_light_in_their_actual_color() {
    use crate::color::{ColorRole, PaletteOp, SelectOp};
    use crate::led::{compute, Led};
    let mut c = core();
    c.project.led_idle = 1; // 待機中 25%
    let bind = |c: &mut Core, n: u8, action: Action| c.project.bindings.push(Binding { id: 0, name: "b".into(), trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: n }), action, page: None, osc: None });
    bind(&mut c, 0, Action::Color { color: Some(NamedColor::Orange), target: ColorTarget::Scope });
    bind(&mut c, 1, Action::Color { color: Some(NamedColor::Pink), target: ColorTarget::Sub });
    bind(&mut c, 2, Action::Select { fixture: SPOT, op: SelectOp::Only });
    bind(&mut c, 3, Action::Palette { op: PaletteOp::Swap });
    bind(&mut c, 4, Action::Scene { name: "x".into() });
    let led = |c: &Core, n: u8| compute(c).into_iter().find(|(x, _)| *x == n).unwrap().1;

    // 待機中は実際の色を暗めに(オレンジ = 255,127,0 の 25%)
    assert_eq!(led(&c, 0), Led::True { rgb: [64, 32, 0], full: false });
    c.handle_midi(MidiMsg { kind: TriggerKind::Note, channel: 0, number: 0, value: 127 });
    c.handle_midi(MidiMsg { kind: TriggerKind::Note, channel: 0, number: 0, value: 0 });
    assert_eq!(led(&c, 0), Led::True { rgb: [255, 128, 0], full: true }, "選んだ色は最大の明るさ");
    // サブ色のパッドも、その色そのもの(パレットに無いピンクでも)
    assert!(matches!(led(&c, 1), Led::True { rgb: [64, 19, 35], .. }), "{:?}", led(&c, 1));

    // 機材選択のパッドは、その機材が今出している色
    c.render();
    let spot_now = led(&c, 2);
    assert!(matches!(spot_now, Led::True { rgb: [r, g, _], .. } if r > g), "スポットはオレンジ系 {spot_now:?}");
    // 入れ替えのパッドは、今のメイン色
    c.set_palette_color(ColorRole::Main, NamedColor::Green.rgb());
    assert!(matches!(led(&c, 3), Led::True { rgb: [0, 64, 0], .. }));
    // 色と関係ないパッドは今までどおりパレットの色
    assert!(matches!(led(&c, 4), Led::Rgb { .. }));
    // 設定でOFFにすれば、全部パレットの色
    c.project.led_true_color = false;
    assert!(matches!(led(&c, 0), Led::Rgb { .. }));
    assert!(matches!(led(&c, 2), Led::Rgb { .. }));
}

// ------------------------------------------------------------ 暗転フェード

#[test]
fn fade_to_black_and_back() {
    let mut c = core();
    c.set_attr(&Selector::Fixture(SPOT), "dimmer", 200.0);
    c.render();
    assert_eq!(out(&c, SPOT, "dimmer"), 200);
    c.fade_to_black(true, 2.0);
    assert!(c.fading());
    c.step_dim(1.0);
    c.render();
    let half = out(&c, SPOT, "dimmer");
    assert!((99..=101).contains(&half), "2秒のうち1秒で半分 {half}");
    c.step_dim(1.0);
    assert!(!c.fading());
    c.render();
    assert_eq!(out(&c, SPOT, "dimmer"), 0);
    assert_eq!(look_of(&c, "MINI SPOT").1, Some("暗転"));
    // 暗転しきったら、ARM中のレーザーも止まる(ブラックアウトと同じ)
    let i = c.project.fixture_index(2).unwrap();
    c.project.fixtures[i].armed = true;
    c.render();
    let f = &c.project.fixtures[i];
    let m = c.lib.mode(&f.profile, f.mode).unwrap();
    let mode_ch = m.find_attr("mode").unwrap();
    assert_eq!(Some(out(&c, 2, "mode")), m.channels[mode_ch].safe);
    // フラッシュも効かない
    c.rt.flash = Some(true);
    c.render();
    assert_eq!(out(&c, SPOT, "dimmer"), 0);
    c.rt.flash = None;
    // 同じ秒数で戻す
    c.toggle_fade(2.0);
    c.step_dim(0.5);
    c.render();
    assert!((49..=51).contains(&out(&c, SPOT, "dimmer")));
    c.step_dim(5.0);
    c.render();
    assert_eq!(out(&c, SPOT, "dimmer"), 200, "元の明るさに戻る(ベース値は変わらない)");
    assert_eq!(c.project.fixtures[0].values[c.lib.mode(&c.project.fixtures[0].profile, 0).unwrap().find_attr("dimmer").unwrap()], 200);
}

#[test]
fn fade_pad_modes_and_osc() {
    let mut c = core();
    let press = |c: &mut Core, v: u8| c.handle_midi(MidiMsg { kind: TriggerKind::Note, channel: 0, number: 9, value: v });
    c.project.bindings.push(Binding { id: 0, name: "d".into(), trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: 9 }), action: Action::Fade { secs: 4.0, mode: PadMode::Toggle }, page: None, osc: None });
    press(&mut c, 127);
    press(&mut c, 0);
    assert_eq!((c.rt.dim_target, c.rt.dim_secs), (0.0, 4.0));
    assert!(crate::led::compute(&c).into_iter().find(|(n, _)| *n == 9).unwrap().1.is_full(), "暗転中はパッドが光る");
    press(&mut c, 127);
    press(&mut c, 0);
    assert_eq!(c.rt.dim_target, 1.0);
    // 押している間だけ
    c.project.bindings[1].action = Action::Fade { secs: 1.0, mode: PadMode::Momentary };
    press(&mut c, 127);
    assert_eq!(c.rt.dim_target, 0.0);
    press(&mut c, 0);
    assert_eq!(c.rt.dim_target, 1.0);
    // OSC: 秒数0なら即座に
    let m = |args: Vec<f64>| crate::io::osc::Message { addr: "/dmx/fade".into(), args: args.into_iter().map(crate::io::osc::Arg::Float).collect() };
    c.handle_osc(&m(vec![1.0, 0.0]));
    assert_eq!(c.rt.dim, 0.0);
    c.handle_osc(&m(vec![0.0]));
    assert_eq!((c.rt.dim_target, c.rt.dim_secs), (1.0, c.project.fade_secs));
}