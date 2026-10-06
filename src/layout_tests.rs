//! おすすめ配置と、フェーダー/ボタン用の新しい動作(明るさ・ミュート・LEDバーの速さ・斜め)のテスト

use crate::apcmini::{pad_note, Ctl};
use crate::engine::*;
use crate::fixture::{self, ChannelDef, Library, Mode, Profile};
use crate::pixel::{PixelColor, PixelFx, PixelKind, PixelParam, FACE_SETS};
use crate::project::*;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

const BAR: &str = "test/bar90-rgbw";
const TWINKLE: &str = "test/twinkle";
const PAR: &str = "test/rgb-par";

fn ch(name: &str, attr: &str) -> ChannelDef {
    ChannelDef::blank(name, attr, "その他")
}

/// ユーザーの構成に近いもの: スポット4・Twinkle Ball・RGBの灯具2・レーザー2・90ピクセルRGBWのバー8本
fn rig() -> Core {
    let (mut lib, _) = Library::load(Path::new("none"));
    lib.add(Profile { id: BAR.into(), name: "LEDバー 90px RGBW".into(), manufacturer: String::new(), needs_arm: false, modes: vec![Mode { name: "360ch".into(), channels: fixture::pixel_channels(1, 90, "RGBW") }] });
    let tw = ["dimmer", "strobe", "red", "green", "blue", "rotation", "program", "program_speed"];
    lib.add(Profile { id: TWINKLE.into(), name: "Twinkle Ball".into(), manufacturer: String::new(), needs_arm: false, modes: vec![Mode { name: "8ch".into(), channels: tw.iter().map(|a| ch(a, a)).collect() }] });
    lib.add(Profile { id: PAR.into(), name: "RGBパー".into(), manufacturer: String::new(), needs_arm: false, modes: vec![Mode { name: "3ch".into(), channels: ["red", "green", "blue"].iter().map(|a| ch(a, a)).collect() }] });
    let lib = Arc::new(lib);
    let mut p = Project::default();
    let add = |p: &mut Project, profile: &str, universe: usize| {
        let id = p.add_fixture(&lib, profile).unwrap();
        let i = p.fixture_index(id).unwrap();
        p.fixtures[i].universe = universe;
        id
    };
    for _ in 0..4 {
        add(&mut p, "stage-evolution/minispot30", 0);
    }
    add(&mut p, TWINKLE, 0);
    add(&mut p, PAR, 0);
    add(&mut p, PAR, 0);
    add(&mut p, "stage-evolution/laserscan300", 0);
    add(&mut p, "stage-evolution/laserscan300", 0);
    for u in 1..=8 {
        let id = add(&mut p, BAR, u);
        let i = p.fixture_index(id).unwrap();
        p.fixtures[i].address = 1;
    }
    Core::new(p, lib)
}

fn bar_ids(c: &Core) -> Vec<u32> {
    c.project.fixtures.iter().filter(|f| f.profile == BAR).map(|f| f.id).collect()
}

fn binding_at(c: &Core, ctl: Ctl) -> Option<&Binding> {
    let hits: Vec<&Binding> = c.project.bindings.iter().filter(|b| b.trigger.is_some_and(|t| ctl.matches(&t))).collect();
    assert!(hits.len() <= 1, "{ctl:?} に割り当てが {} 件", hits.len());
    hits.first().copied()
}

/// 押して離す
fn press(c: &mut Core, ctl: Ctl) {
    let (kind, number) = ctl.trigger();
    c.handle_midi(MidiMsg { kind, channel: 0, number, value: 127 });
    c.handle_midi(MidiMsg { kind, channel: 0, number, value: 0 });
}

/// 割り当てを1回だけ実行する(フェーダーの値 v=0〜1)
fn run(c: &mut Core, action: Action, v: f32) {
    c.project.bindings.push(Binding { name: "t".into(), trigger: None, action, page: None, osc: None });
    let i = c.project.bindings.len() - 1;
    c.apply_binding(i, v, false);
    c.project.bindings.pop();
    c.rt.pad_state.remove(&i);
}

fn fader(c: &mut Core, i: u8, value: u8) {
    c.handle_midi(MidiMsg { kind: TriggerKind::Cc, channel: 0, number: 0x30 + i, value });
}

/// バーの出力(RGBW)。[バー][ピクセル]
fn bars_out(c: &Core) -> Vec<Vec<[u8; 4]>> {
    bar_ids(c)
        .iter()
        .map(|&id| {
            let f = &c.project.fixtures[c.project.fixture_index(id).unwrap()];
            let fr = &c.rt.frames[f.universe];
            (0..90).map(|k| [fr[k * 4], fr[k * 4 + 1], fr[k * 4 + 2], fr[k * 4 + 3]]).collect()
        })
        .collect()
}

#[test]
fn recommended_layout_fills_the_controller_without_conflicts() {
    let mut c = rig();
    let lib = c.lib.clone();
    let r = crate::layout::apply(&mut c.project, &lib);
    assert_eq!(r.presets, 32);
    assert!(r.effects >= 4, "{r:?}");

    // 1つのコントロールに1つの割り当て
    let mut seen = HashSet::new();
    for b in &c.project.bindings {
        let t = b.trigger.unwrap();
        assert!(seen.insert((t.kind == TriggerKind::Cc, t.number)), "重複: {}", b.name);
    }
    // 1〜4段は模様のセット、6・7段はメイン色・サブ色
    for (row, set) in FACE_SETS.iter().enumerate() {
        for (col, fx) in set.presets.iter().enumerate() {
            let b = binding_at(&c, Ctl::Pad(pad_note(row as u8, col as u8))).unwrap();
            assert!(matches!(&b.action, Action::Effect { name, .. } if name == fx.name), "{}", b.name);
        }
    }
    assert!(matches!(binding_at(&c, Ctl::Pad(pad_note(5, 0))).unwrap().action, Action::Color { target: crate::color::ColorTarget::Main, .. }));
    assert!(matches!(binding_at(&c, Ctl::Pad(pad_note(6, 7))).unwrap().action, Action::Color { target: crate::color::ColorTarget::Sub, .. }));
    // トラックボタン=LEDバー1〜8のミュート(パッチ順)
    for (i, id) in bar_ids(&c).into_iter().enumerate() {
        let b = binding_at(&c, Ctl::Track(i as u8)).unwrap();
        assert!(matches!(&b.action, Action::Mute { sel: Selector::Fixture(x), .. } if *x == id), "{}", b.name);
    }
    // フェーダー: LEDバーの明るさ・速さ・斜め / 機種ごとの明るさ / 色相 / 全体光量
    assert!(matches!(binding_at(&c, Ctl::Fader(0)).unwrap().action, Action::Level { .. }));
    assert!(matches!(binding_at(&c, Ctl::Fader(1)).unwrap().action, Action::Pixel { param: PixelParam::Speed }));
    assert!(matches!(binding_at(&c, Ctl::Fader(2)).unwrap().action, Action::Pixel { param: PixelParam::Skew }));
    assert!(matches!(&binding_at(&c, Ctl::Fader(3)).unwrap().action, Action::SetValue { attr, .. } if attr == "dimmer"), "スポットはディマー");
    assert!(matches!(binding_at(&c, Ctl::Fader(5)).unwrap().action, Action::Level { .. }), "ディマーの無いRGB灯具は明るさ");
    assert!(matches!(binding_at(&c, Ctl::Fader(8)).unwrap().action, Action::Master { .. }));

    // 割り当てが指す模様・エフェクトは全部ある
    let names = crate::ui_binding::names(&c.project);
    for b in &c.project.bindings {
        if let Action::Effect { name, .. } = &b.action {
            assert!(names.effects.contains(name), "{name} が無い");
        }
    }
    // レーザー(ARMが必要)には、明るさもミュートも割り当てない
    for b in &c.project.bindings {
        if let Action::SetValue { sel: Selector::Profile(p), .. } | Action::Level { sel: Selector::Profile(p) } | Action::Mute { sel: Selector::Profile(p), .. } = &b.action {
            assert_ne!(p, "stage-evolution/laserscan300", "{}", b.name);
        }
    }
}

#[test]
fn applying_twice_keeps_one_copy_and_other_devices() {
    let mut c = rig();
    // 他のMIDI機器(ch10)とOSCだけの割り当ては残す
    c.project.bindings.push(Binding { name: "外部".into(), trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: Some(9), number: 60 }), action: Action::Tempo { op: TempoOp::Tap }, page: None, osc: None });
    c.project.bindings.push(Binding { name: "OSC".into(), trigger: None, action: Action::Tempo { op: TempoOp::Tap }, page: None, osc: Some("/tap".into()) });
    let lib = c.lib.clone();
    let first = crate::layout::apply(&mut c.project, &lib);
    let (n, fx, px) = (c.project.bindings.len(), c.project.effects.len(), c.project.pixel_fx.len());
    let second = crate::layout::apply(&mut c.project, &lib);
    assert_eq!((c.project.bindings.len(), c.project.effects.len(), c.project.pixel_fx.len()), (n, fx, px));
    assert_eq!(second.removed, first.bindings);
    assert_eq!((second.presets, second.effects), (0, 0));
    assert!(c.project.bindings.iter().any(|b| b.name == "外部") && c.project.bindings.iter().any(|b| b.name == "OSC"));
}

#[test]
fn layout_removes_old_template_patterns_only_when_unused() {
    let mut c = rig();
    crate::pixel::add_pad_template(&mut c.project, None);
    let mine = PixelFx { name: "自作".into(), ..PixelFx::default() };
    c.project.pixel_fx.push(mine);
    let lib = c.lib.clone();
    crate::layout::apply(&mut c.project, &lib);
    let names: Vec<&str> = c.project.pixel_fx.iter().map(|f| f.name.as_str()).collect();
    assert!(!names.contains(&"縞々スクロール"), "古いテンプレートの模様は片付ける");
    assert!(names.contains(&"自作"), "自分で作った模様は残す");
    assert_eq!(names.len(), 33);
}

#[test]
fn face_pads_run_the_patterns_and_mute_buttons_silence_bars() {
    let mut c = rig();
    let lib = c.lib.clone();
    crate::layout::apply(&mut c.project, &lib);
    c.apply_named_color(crate::color::NamedColor::Red, crate::color::ColorTarget::All);
    press(&mut c, Ctl::Pad(pad_note(1, 2))); // 縦スキャン
    assert!(c.project.pixel_fx.iter().any(|f| f.name == "縦スキャン" && f.enabled));
    c.rt.beat = 0.0;
    c.render();
    let lit = |c: &Core| bars_out(c).iter().map(|b| b.iter().any(|p| p.iter().any(|&v| v > 0))).collect::<Vec<bool>>();
    assert!(lit(&c).iter().all(|&x| x), "メイン色の帯 + サブ色の地");

    // トラックボタン1でバー1だけ消える。もう一度押すと戻る
    press(&mut c, Ctl::Track(0));
    c.render();
    assert_eq!(lit(&c), [false, true, true, true, true, true, true, true]);
    press(&mut c, Ctl::Track(0));
    c.render();
    assert!(lit(&c).iter().all(|&x| x));

    // 別の模様のパッドを押すと、前の模様は止まる(1本のバーに1つずつ)
    press(&mut c, Ctl::Pad(pad_note(0, 0)));
    let on: Vec<&str> = c.project.pixel_fx.iter().filter(|f| f.enabled).map(|f| f.name.as_str()).collect();
    assert_eq!(on, ["ブリーズ"]);
}

#[test]
fn level_fader_dims_only_its_fixtures() {
    let mut c = rig();
    let lib = c.lib.clone();
    crate::layout::apply(&mut c.project, &lib);
    c.apply_named_color(crate::color::NamedColor::White, crate::color::ColorTarget::All);
    c.render();
    assert!(bars_out(&c).iter().flatten().all(|p| *p == [0, 0, 0, 255]));
    fader(&mut c, 0, 0); // LEDバーの明るさ 0
    c.render();
    assert!(bars_out(&c).iter().flatten().all(|p| *p == [0, 0, 0, 0]));
    fader(&mut c, 0, 127);
    c.render();
    assert!(bars_out(&c).iter().flatten().all(|p| *p == [0, 0, 0, 255]));
    // RGB灯具の明るさは、バーに影響しない
    fader(&mut c, 5, 0);
    c.render();
    assert!(bars_out(&c).iter().flatten().all(|p| *p == [0, 0, 0, 255]));
    let par = c.project.fixtures.iter().find(|f| f.profile == PAR).unwrap();
    assert_eq!(&c.rt.frames[0][par.address as usize - 1..par.address as usize + 2], &[0, 0, 0]);
}

#[test]
fn speed_fader_steps_and_keeps_the_pattern_in_place() {
    let mut c = rig();
    c.rt.beat = 10.0;
    let before = c.pixel_beat();
    run(&mut c, Action::Pixel { param: PixelParam::Speed }, 1.0);
    assert_eq!(c.rt.px_speed, 4.0);
    assert!((c.pixel_beat() - before).abs() < 1e-9, "変えた瞬間は飛ばない");
    c.rt.beat = 11.0;
    assert!((c.pixel_beat() - (before + 4.0)).abs() < 1e-9, "1拍で4拍ぶん進む");
    for (v, s) in [(0.0, 0.25), (0.3, 0.5), (0.5, 1.0), (0.7, 2.0)] {
        run(&mut c, Action::Pixel { param: PixelParam::Speed }, v);
        assert_eq!(c.rt.px_speed, s, "{v}");
    }
    // 頭出しで小節の頭にそろえ直す
    c.resync();
    assert_eq!(c.pixel_beat(), 0.0);
}

#[test]
fn skew_fader_adds_to_every_pattern() {
    let mut c = rig();
    c.project.pixel_fx.push(PixelFx { enabled: true, kind: PixelKind::Chase, a: PixelColor::Custom([1.0, 0.0, 0.0]), b: PixelColor::Off, width: 1.0, beats: 8.0, span: false, ..PixelFx::default() });
    c.rt.beat = 0.0;
    c.render();
    let heads = |c: &Core| bars_out(c).iter().map(|b| b.iter().position(|p| p[0] > 0).unwrap()).collect::<Vec<usize>>();
    assert_eq!(heads(&c), vec![0; 8]);
    run(&mut c, Action::Pixel { param: PixelParam::Skew }, 0.5); // 1周
    c.render();
    assert_eq!(heads(&c), vec![0, 11, 22, 33, 45, 56, 67, 78]);
}

#[test]
fn every_face_preset_moves_or_shows_a_pattern() {
    let mut c = rig();
    let lib = c.lib.clone();
    crate::layout::apply(&mut c.project, &lib);
    c.set_palette_color(crate::color::ColorRole::Main, [1.0, 0.0, 0.0]);
    c.set_palette_color(crate::color::ColorRole::Sub, [0.0, 0.0, 1.0]);
    let mut seen: Vec<(String, Vec<Vec<Vec<[u8; 4]>>>)> = Vec::new();
    for fx in FACE_SETS.iter().flat_map(|s| s.presets.iter()) {
        c.set_pixel_fx(fx.name, true);
        let frames: Vec<Vec<Vec<[u8; 4]>>> = [0.3, 1.1, 2.7, 5.2]
            .iter()
            .map(|&b| {
                c.rt.beat = b * fx.beats.max(1.0) as f64 / 8.0;
                c.render();
                bars_out(&c)
            })
            .collect();
        // 1コマの中に違う色があるか、コマごとに変わる(=動いている)
        let varied = frames.iter().any(|f| f.iter().flatten().any(|p| p != &f[0][0]));
        let moving = frames.windows(2).any(|w| w[0] != w[1]);
        assert!(varied || moving, "{} が単色のまま止まっている", fx.name);
        assert!(frames.iter().flatten().flatten().any(|p| p.iter().any(|&v| v > 0)), "{} が真っ暗", fx.name);
        for (name, other) in &seen {
            assert!(other[1] != frames[1] || other[2] != frames[2], "{} と {name} が同じ見た目", fx.name);
        }
        seen.push((fx.name.to_string(), frames));
    }
}

/// 模様の見た目の確認用(普段は実行しない)。セットごとに、8本×90ピクセルの面を6コマずつ並べた画像を書き出す。
/// 実行: DMXCTL_SHEET_DIR=保存先 cargo test --release preset_sheet -- --ignored
#[test]
#[ignore]
fn preset_sheet() {
    use eframe::egui::{Color32, ColorImage};
    let dir = std::path::PathBuf::from(std::env::var("DMXCTL_SHEET_DIR").unwrap_or_else(|_| ".".into()));
    let mut c = rig();
    let lib = c.lib.clone();
    crate::layout::apply(&mut c.project, &lib);
    c.set_palette_color(crate::color::ColorRole::Main, [1.0, 0.1, 0.4]);
    c.set_palette_color(crate::color::ColorRole::Sub, [0.1, 0.3, 1.0]);
    const PX_W: usize = 3;
    const BAR_H: usize = 6;
    const GAP: usize = 3;
    const FRAMES: usize = 6;
    let tile_w = 90 * PX_W;
    let tile_h = 8 * (BAR_H + GAP);
    for (si, set) in FACE_SETS.iter().enumerate() {
        let (w, h) = (FRAMES * (tile_w + 12) + 12, 8 * (tile_h + 12) + 12);
        let mut img = ColorImage::filled([w, h], Color32::from_gray(18));
        for (pi, fx) in set.presets.iter().enumerate() {
            c.set_pixel_fx(fx.name, true);
            for f in 0..FRAMES {
                // 1周期を6コマに分ける(止まった模様は拍で進めない)
                c.rt.beat = f as f64 * fx.beats.max(0.5) as f64 / FRAMES as f64;
                c.render();
                for (b, bar) in bars_out(&c).iter().enumerate() {
                    for (k, p) in bar.iter().enumerate() {
                        let rgb = [p[0], p[1], p[2]].map(|x| x.saturating_add(p[3]));
                        for dy in 0..BAR_H {
                            for dx in 0..PX_W {
                                let x = 12 + f * (tile_w + 12) + k * PX_W + dx;
                                let y = 12 + pi * (tile_h + 12) + b * (BAR_H + GAP) + dy;
                                img.pixels[y * w + x] = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
                            }
                        }
                    }
                }
            }
        }
        crate::shot::save_png(&dir.join(format!("presets_{si}.png")), &img).unwrap();
    }
}
