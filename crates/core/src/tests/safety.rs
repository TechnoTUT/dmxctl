//! 安全処理: 禁止の値(レーザーのオートショーなど)・動いてよい範囲・異常時の安全なフレーム

use crate::engine::*;
use crate::fixture::Library;
use crate::project::*;
use std::path::Path;
use std::sync::Arc;

/// Spot 1 (id=1) と Laser 1 (id=2)
fn core() -> Core {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    Core::new(Project::starter(&lib), lib)
}

fn laser(c: &Core) -> usize {
    c.project.fixtures.iter().position(|f| f.profile == "stage-evolution/laserscan300").unwrap()
}

/// レーザーの、出力に出ている値
fn out(c: &Core, attr: &str) -> u8 {
    let f = &c.project.fixtures[laser(c)];
    let ch = c.lib.mode(&f.profile, f.mode).unwrap().find_attr(attr).unwrap();
    c.rt.frames[f.universe][f.address as usize - 1 + ch]
}

/// ARMして、常時出力の合図も出す(ARMだけでは出ない)
fn armed(c: &mut Core) {
    let i = laser(c);
    c.project.fixtures[i].armed = true;
    c.rt.laser_latch = true;
}

#[test]
fn laser_never_enters_auto_or_sound_mode() {
    let mut c = core();
    armed(&mut c);
    let sel = Selector::Fixture(c.project.fixtures[laser(&c)].id);
    c.render();
    assert_eq!(out(&c, "mode"), 255, "ARM中はDMX制御");
    for v in [21.0, 30.0, 100.0, 110.0, 150.0, 190.0, 210.0, 220.0] {
        c.set_attr(&sel, "mode", v);
        c.render();
        assert_eq!(out(&c, "mode"), 0, "オートショー/サウンド({v})は出さずにレーザーOFF");
    }
    c.set_attr(&sel, "mode", 230.0);
    c.render();
    assert_eq!(out(&c, "mode"), 230);
}

#[test]
fn auto_mode_blocked_even_from_effects() {
    let mut c = core();
    armed(&mut c);
    let id = c.project.fixtures[laser(&c)].id;
    c.project.effects.push(Effect { enabled: true, sel: Selector::Fixture(id), min: 40.0, max: 40.0, ..Effect::new("e", "mode") });
    c.render();
    assert_eq!(out(&c, "mode"), 0);
}

#[test]
fn laser_position_stays_in_limits() {
    let mut c = core();
    armed(&mut c);
    let i = laser(&c);
    let sel = Selector::Fixture(c.project.fixtures[i].id);
    c.project.fixtures[i].limits.insert("pan".into(), [20, 90]);
    c.project.fixtures[i].limits.insert("tilt".into(), [30, 60]);
    c.set_attr(&sel, "pan", 5.0);
    c.set_attr(&sel, "tilt", 120.0);
    c.render();
    assert_eq!((out(&c, "pan"), out(&c, "tilt")), (20, 60));
    // 自動で動く値(128以上)も使わない
    c.set_attr(&sel, "pan", 200.0);
    c.render();
    assert_eq!(out(&c, "pan"), 90);
    c.project.fixtures[i].limits.clear();
    c.render();
    assert_eq!(out(&c, "pan"), 127, "範囲が無くても、自動の動きには入らない");
}

#[test]
fn user_override_keeps_builtin_forbidden_ranges() {
    let dir = std::env::temp_dir().join(format!("dmxctl_forbid_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut json: serde_json::Value = serde_json::from_str(include_str!("../../../../fixtures/stage-evolution_laserscan300.json")).unwrap();
    for ch in json["modes"][0]["channels"].as_array_mut().unwrap() {
        ch.as_object_mut().unwrap().remove("forbid");
    }
    std::fs::write(dir.join("laser.json"), json.to_string()).unwrap();
    let (lib, errors) = Library::load(&dir);
    std::fs::remove_dir_all(&dir).ok();
    assert!(errors.is_empty(), "{errors:?}");
    assert!(lib.has_user_file("stage-evolution/laserscan300"));
    let m = lib.mode("stage-evolution/laserscan300", 0).unwrap();
    let mode = &m.channels[m.find_attr("mode").unwrap()];
    assert_eq!(mode.allowed(30), 0, "上書きした定義でも、オートショーには入らない");
}

#[test]
fn safe_frames_turn_laser_off_and_lights_dark() {
    let mut c = core();
    armed(&mut c);
    for f in c.project.fixtures.iter_mut() {
        f.values.iter_mut().for_each(|v| *v = 230);
    }
    c.render();
    let f = &c.project.fixtures[laser(&c)];
    let ch = c.lib.mode(&f.profile, f.mode).unwrap().find_attr("mode").unwrap();
    let addr = f.address as usize - 1 + ch;
    assert_ne!(c.rt.frames[0][addr], 0);
    let safe = c.safe_frames();
    assert_eq!(safe[0][addr], 0, "レーザーOFF");
    let spot = &c.project.fixtures[0];
    let dim = c.lib.mode(&spot.profile, spot.mode).unwrap().find_attr("dimmer").unwrap();
    assert_eq!(safe[0][spot.address as usize - 1 + dim], 0, "スポットのディマーは0");
}

/// レーザーの、出力に出ている動作モード(0=OFF / 255=DMX制御)
fn mode_out(c: &mut Core) -> u8 {
    c.render();
    out(c, "mode")
}

#[test]
fn armed_laser_waits_for_a_cue() {
    let mut c = core();
    let i = laser(&c);
    c.project.fixtures[i].armed = true;
    assert_eq!(mode_out(&mut c), 0, "ARMだけでは出さない");
    assert!(!c.laser_cue());

    // 押している間だけ
    c.set_laser_hold(true);
    assert_eq!(mode_out(&mut c), 255);
    c.set_laser_hold(false);
    assert_eq!(mode_out(&mut c), 0);

    // 常時出力
    c.rt.laser_latch = true;
    assert_eq!(mode_out(&mut c), 255);
    // 合図があっても、ARMしていなければ出さない
    c.project.fixtures[i].armed = false;
    assert_eq!(mode_out(&mut c), 0);
    // ブラックアウトは合図より強い
    c.project.fixtures[i].armed = true;
    c.rt.blackout = true;
    assert_eq!(mode_out(&mut c), 0);
}

#[test]
fn osc_laser_hold_stops_when_the_sender_goes_quiet() {
    use crate::io::osc::{Arg, Message};
    use std::time::{Duration, Instant};
    let mut c = core();
    let i = laser(&c);
    c.project.fixtures[i].armed = true;
    let start = Instant::now();
    c.rt.virtual_now = Some(start);
    let send = |c: &mut Core, v: f64| c.handle_osc(&Message { addr: "/dmx/laser".into(), args: vec![Arg::Float(v)] });
    send(&mut c, 1.0);
    assert_eq!(mode_out(&mut c), 255);
    // 送り続けている間は出し続ける
    c.rt.virtual_now = Some(start + Duration::from_millis(800));
    send(&mut c, 1.0);
    c.rt.virtual_now = Some(start + Duration::from_millis(1600));
    assert_eq!(mode_out(&mut c), 255);
    // 届かなくなったら止まる
    c.rt.virtual_now = Some(start + Duration::from_millis(1900));
    assert_eq!(mode_out(&mut c), 0);
    assert!(!c.rt.laser_hold);
    // 0 ですぐ止まる
    send(&mut c, 1.0);
    send(&mut c, 0.0);
    assert_eq!(mode_out(&mut c), 0);
    // 常時出力は期限なし
    c.handle_osc(&Message { addr: "/dmx/laser/latch".into(), args: vec![Arg::Float(1.0)] });
    c.rt.virtual_now = Some(start + Duration::from_secs(60));
    assert_eq!(mode_out(&mut c), 255);
}

#[test]
fn laser_pad_holds_and_toggles() {
    let mut c = core();
    let i = laser(&c);
    c.project.fixtures[i].armed = true;
    c.project.bindings.push(Binding { id: 0, name: "hold".into(), trigger: None, page: None, osc: Some("/t/hold".into()), action: Action::Laser { mode: PadMode::Momentary } });
    c.project.bindings.push(Binding { id: 0, name: "latch".into(), trigger: None, page: None, osc: Some("/t/latch".into()), action: Action::Laser { mode: PadMode::Toggle } });
    let n = c.project.bindings.len();
    c.apply_binding(n - 2, 1.0, true);
    assert_eq!(mode_out(&mut c), 255, "押している間");
    c.apply_binding(n - 2, 0.0, false);
    assert_eq!(mode_out(&mut c), 0, "離すと止まる");
    c.apply_binding(n - 1, 1.0, true);
    c.apply_binding(n - 1, 0.0, false);
    assert_eq!(mode_out(&mut c), 255, "トグルで常時出力");
    c.apply_binding(n - 1, 1.0, true);
    c.apply_binding(n - 1, 0.0, false);
    assert_eq!(mode_out(&mut c), 0, "もう一度押すと止まる");
}

#[test]
fn auto_phrase_cues_the_laser_only_while_online() {
    let mut c = core();
    let i = laser(&c);
    c.project.fixtures[i].armed = true;
    c.project.auto.enabled = true;
    let mut look = PhraseLook { laser: true, ..PhraseLook::default() };
    c.apply_look("chorus", &look);
    assert_eq!(mode_out(&mut c), 0, "曲の情報が届いていなければ出さない");
    // 曲の情報が届く(つながった瞬間はバースの見た目。標準のバースはレーザーを出さない)
    c.rt.auto_last = Some(c.now());
    assert_eq!(mode_out(&mut c), 0);
    c.apply_look("chorus", &look);
    assert_eq!(mode_out(&mut c), 255, "レーザーを出すフレーズ");
    look.laser = false;
    c.apply_look("verse", &look);
    assert_eq!(mode_out(&mut c), 0, "出さないフレーズ");
    // 切れたら止める
    c.apply_look("chorus", &PhraseLook { laser: true, ..PhraseLook::default() });
    c.rt.auto_last = None;
    assert_eq!(mode_out(&mut c), 0, "曲の情報が途切れたら出さない");
    assert!(!c.rt.auto_laser);
}
