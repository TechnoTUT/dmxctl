//! 割り当ての実行(apply_binding): パッドの動作(連続/押している間/トグル/ワンショット)ごとに、
//! 押した瞬間・離した瞬間・押しっぱなしで何が起きるか

use crate::core::color::{ColorRole, ColorTarget, PaletteOp, ScopeOp};
use crate::core::engine::*;
use crate::core::fixture::Library;
use crate::core::io::ndi::LiveSource;
use crate::core::project::*;
use std::path::Path;
use std::sync::Arc;

/// Spot 1 (id=1) と Laser 1 (id=2)
fn core() -> Core {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    Core::new(Project::starter(&lib), lib)
}

/// 割り当てを足して、その番号を返す
fn bind(c: &mut Core, action: Action) -> usize {
    c.project.bindings.push(Binding { id: 0, name: "t".into(), trigger: None, action, page: None, osc: None });
    c.project.bindings.len() - 1
}

fn press(c: &mut Core, i: usize) {
    c.apply_binding(i, 1.0, true);
}

fn release(c: &mut Core, i: usize) {
    c.apply_binding(i, 0.0, false);
}

fn spot_dimmer(c: &Core) -> u8 {
    let fx = &c.project.fixtures[0];
    let m = c.lib.mode(&fx.profile, fx.mode).unwrap();
    fx.values[m.find_attr("dimmer").unwrap()]
}

fn set_value(mode: PadMode, on: f32, off: f32) -> Action {
    Action::SetValue { sel: Selector::Fixture(1), attr: "dimmer".into(), mode, on, off }
}

#[test]
fn set_value_modes() {
    let mut c = core();
    let fader = bind(&mut c, set_value(PadMode::Continuous, 200.0, 0.0));
    c.apply_binding(fader, 0.5, false);
    assert_eq!(spot_dimmer(&c), 100, "連続: off〜onの間");

    let hold = bind(&mut c, set_value(PadMode::Momentary, 200.0, 10.0));
    press(&mut c, hold);
    assert_eq!(spot_dimmer(&c), 200, "押している間: 押したらON値");
    release(&mut c, hold);
    assert_eq!(spot_dimmer(&c), 10, "押している間: 離したらOFF値");

    let once = bind(&mut c, set_value(PadMode::Trigger, 150.0, 0.0));
    release(&mut c, once);
    assert_eq!(spot_dimmer(&c), 10, "ワンショット: 離しただけでは何もしない");
    press(&mut c, once);
    assert_eq!(spot_dimmer(&c), 150);

    let toggle = bind(&mut c, set_value(PadMode::Toggle, 255.0, 0.0));
    press(&mut c, toggle);
    assert_eq!(spot_dimmer(&c), 255, "トグル: 1回目でON");
    release(&mut c, toggle);
    assert_eq!(spot_dimmer(&c), 255, "トグル: 離してもそのまま");
    press(&mut c, toggle);
    assert_eq!(spot_dimmer(&c), 0, "トグル: 2回目でOFF");
}

#[test]
fn rising_only_actions_ignore_release_and_held_repeats() {
    let mut c = core();
    let next = bind(&mut c, Action::Page { op: PageOp::Next });
    press(&mut c, next);
    assert_eq!(c.rt.page, 1);
    press(&mut c, next);
    assert_eq!(c.rt.page, 1, "押しっぱなしの再送では進まない");
    release(&mut c, next);
    assert_eq!(c.rt.page, 1, "離しても進まない");
    press(&mut c, next);
    assert_eq!(c.rt.page, 2);
}

#[test]
fn master_modes() {
    let mut c = core();
    let fader = bind(&mut c, Action::Master { mode: PadMode::Continuous, on: 100.0, off: 0.0 });
    c.apply_binding(fader, 0.5, false);
    assert!((c.rt.master - 0.5).abs() < 1e-6);

    let hold = bind(&mut c, Action::Master { mode: PadMode::Momentary, on: 100.0, off: 20.0 });
    press(&mut c, hold);
    assert!((c.rt.master - 1.0).abs() < 1e-6);
    release(&mut c, hold);
    assert!((c.rt.master - 0.2).abs() < 1e-6);

    let toggle = bind(&mut c, Action::Master { mode: PadMode::Toggle, on: 100.0, off: 30.0 });
    c.rt.master = 1.0;
    press(&mut c, toggle);
    assert!((c.rt.master - 0.3).abs() < 1e-6, "ON値のときに押すとOFF値へ");
    release(&mut c, toggle);
    assert!((c.rt.master - 0.3).abs() < 1e-6);
    press(&mut c, toggle);
    assert!((c.rt.master - 1.0).abs() < 1e-6, "それ以外のときはON値へ");

    let once = bind(&mut c, Action::Master { mode: PadMode::Trigger, on: 40.0, off: 0.0 });
    release(&mut c, once);
    assert!((c.rt.master - 1.0).abs() < 1e-6);
    press(&mut c, once);
    assert!((c.rt.master - 0.4).abs() < 1e-6);
}

#[test]
fn fade_to_black_modes() {
    let mut c = core();
    let hold = bind(&mut c, Action::Fade { secs: 0.0, mode: PadMode::Momentary });
    press(&mut c, hold);
    assert_eq!(c.rt.dim, 0.0, "押している間は暗転");
    release(&mut c, hold);
    assert_eq!(c.rt.dim, 1.0, "離すと戻る");

    let toggle = bind(&mut c, Action::Fade { secs: 0.0, mode: PadMode::Toggle });
    press(&mut c, toggle);
    release(&mut c, toggle);
    assert_eq!(c.rt.dim, 0.0, "トグル: 離しても暗いまま");
    press(&mut c, toggle);
    assert_eq!(c.rt.dim, 1.0);

    let once = bind(&mut c, Action::Fade { secs: 0.0, mode: PadMode::Trigger });
    press(&mut c, once);
    release(&mut c, once);
    assert_eq!(c.rt.dim, 0.0, "ワンショット: 暗転するだけ");
    press(&mut c, once);
    assert_eq!(c.rt.dim, 0.0);
}

#[test]
fn color_scope_ops() {
    let mut c = core();
    assert!(c.rt.color_global);
    let hold = bind(&mut c, Action::ColorScope { op: ScopeOp::HoldIndividual });
    press(&mut c, hold);
    assert!(!c.rt.color_global, "押している間は個別");
    release(&mut c, hold);
    assert!(c.rt.color_global);

    let toggle = bind(&mut c, Action::ColorScope { op: ScopeOp::Toggle });
    press(&mut c, toggle);
    release(&mut c, toggle);
    assert!(!c.rt.color_global);

    let global = bind(&mut c, Action::ColorScope { op: ScopeOp::Global });
    press(&mut c, global);
    assert!(c.rt.color_global);
    let individual = bind(&mut c, Action::ColorScope { op: ScopeOp::Individual });
    press(&mut c, individual);
    assert!(!c.rt.color_global);
}

#[test]
fn palette_hold_swap_returns_on_release() {
    let mut c = core();
    let hold = bind(&mut c, Action::Palette { op: PaletteOp::HoldSwap });
    press(&mut c, hold);
    assert!(c.palette_swapped());
    release(&mut c, hold);
    assert!(!c.palette_swapped());

    let auto = bind(&mut c, Action::Palette { op: PaletteOp::Auto });
    press(&mut c, auto);
    assert!(c.project.palette.auto);
    release(&mut c, auto);
    press(&mut c, auto);
    assert!(!c.project.palette.auto);
}

#[test]
fn hue_fader_and_named_color() {
    let mut c = core();
    let fader = bind(&mut c, Action::Color { color: None, target: ColorTarget::All });
    c.apply_binding(fader, 0.5, false);
    assert!((c.rt.hue - 180.0).abs() < 1e-3, "色相フェーダーは押していなくても効く");

    let pad = bind(&mut c, Action::Color { color: Some(crate::core::color::NamedColor::Red), target: ColorTarget::All });
    release(&mut c, pad);
    assert_eq!(c.rt.last_color, None, "色のパッドは離しただけでは何もしない");
    press(&mut c, pad);
    assert_eq!(c.rt.last_color, Some(crate::core::color::NamedColor::Red));
}

#[test]
fn tempo_source_toggles_back_to_internal() {
    let mut c = core();
    let audio = bind(&mut c, Action::Tempo { op: TempoOp::Audio });
    press(&mut c, audio);
    assert_eq!(c.project.tempo_source, TempoSource::Audio);
    release(&mut c, audio);
    assert_eq!(c.project.tempo_source, TempoSource::Audio);
    press(&mut c, audio);
    assert_eq!(c.project.tempo_source, TempoSource::Internal);

    let set = bind(&mut c, Action::Tempo { op: TempoOp::Set { min: 100.0, max: 140.0 } });
    c.apply_binding(set, 0.5, false);
    assert!((c.project.bpm - 120.0).abs() < 1e-3);
}

#[test]
fn live_color_toggles_ndi_source() {
    let mut c = core();
    let video = bind(&mut c, Action::LiveColor { role: ColorRole::Main, source: LiveSource::Video });
    press(&mut c, video);
    assert_eq!(c.project.ndi.main, LiveSource::Video);
    assert!(c.project.ndi.enabled, "選んだらNDIの受信を有効にする");
    release(&mut c, video);
    press(&mut c, video);
    assert_eq!(c.project.ndi.main, LiveSource::Off, "同じものをもう一度選ぶと使わない");
    assert_eq!(c.project.ndi.sub, LiveSource::Off, "サブ色は変わらない");
}
