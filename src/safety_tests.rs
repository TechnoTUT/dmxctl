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

fn armed(c: &mut Core) {
    let i = laser(c);
    c.project.fixtures[i].armed = true;
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
    let mut json: serde_json::Value = serde_json::from_str(include_str!("../fixtures/stage-evolution_laserscan300.json")).unwrap();
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
