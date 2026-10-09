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
    let range = |from: u8, to: u8, label: &str| fixture::Range { from, to, label: label.into(), sweep: false, warn: false, color: None };
    let mut tw_ch: Vec<ChannelDef> = tw.iter().map(|a| ch(a, a)).collect();
    tw_ch[5].ranges = vec![range(0, 129, "角度調整"), range(130, 139, "リセット"), range(140, 255, "回転(遅→速)")];
    tw_ch[6].ranges = vec![range(0, 9, "機能なし"), range(10, 39, "オートプログラム1"), range(40, 159, "オートプログラム2〜5"), range(160, 189, "サウンドプログラム1"), range(190, 255, "サウンドプログラム2")];
    lib.add(Profile { id: TWINKLE.into(), name: "Twinkle Ball".into(), manufacturer: String::new(), needs_arm: false, modes: vec![Mode { name: "8ch".into(), channels: tw_ch }] });
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

/// 1ページ目(演出)で効く割り当て
fn binding_at(c: &Core, ctl: Ctl) -> Option<&Binding> {
    binding_on(c, ctl, 0)
}

fn binding_on(c: &Core, ctl: Ctl, page: u8) -> Option<&Binding> {
    let hits: Vec<&Binding> = c.project.bindings.iter().filter(|b| b.on_page(page) && b.trigger.is_some_and(|t| ctl.matches(&t))).collect();
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

    // どのページでも、1つのコントロールに1つの割り当て
    for page in 0..2u8 {
        let mut seen = HashSet::new();
        for b in c.project.bindings.iter().filter(|b| b.on_page(page)) {
            let t = b.trigger.unwrap();
            assert!(seen.insert((t.kind == TriggerKind::Cc, t.number)), "ページ{} で重複: {}", page + 1, b.name);
        }
    }
    // 1〜4段は模様のセット、6・7段はメイン色・サブ色
    for (row, set) in FACE_SETS.iter().enumerate() {
        for (col, fx) in set.presets.iter().enumerate() {
            let b = binding_at(&c, Ctl::Pad(pad_note(row as u8, col as u8))).unwrap();
            assert!(matches!(&b.action, Action::Effect { name, .. } if name == fx.name), "{}", b.name);
        }
    }
    // 5段目=ムービングの動き、6段目=ムービングの見た目、7段目=その他の機材(色のパッドは置かない)
    let effect_at = |row: u8, col: u8| match &binding_at(&c, Ctl::Pad(pad_note(row, col))).unwrap().action {
        Action::Effect { name, .. } => name.clone(),
        a => panic!("{row}段{col}列: {a:?}"),
    };
    assert_eq!((0..8).map(|k| effect_at(4, k)).collect::<Vec<_>>(), ["円運動", "8の字", "パン揺れ", "チルト揺れ", "ワイパー", "クロス", "バラバラ", "高速円"]);
    assert_eq!(effect_at(5, 0), "点灯チェイス");
    assert!(matches!(&binding_at(&c, Ctl::Pad(pad_note(6, 0))).unwrap().action, Action::SetValue { attr, .. } if attr == "rotation"));
    assert!(!c.project.bindings.iter().any(|b| matches!(b.action, Action::Color { color: Some(_), .. })), "色のパッドは無い");
    // トラックボタン=機種ごとの消灯(LEDバーはまとめて1つ、レーザーも)
    let mutes: Vec<String> = (0..8u8)
        .filter_map(|i| binding_at(&c, Ctl::Track(i)))
        .map(|b| match &b.action {
            Action::Mute { sel: Selector::Group(g), .. } => g.clone(),
            Action::Mute { sel: Selector::Profile(p), .. } => p.clone(),
            a => panic!("{a:?}"),
        })
        .collect();
    assert_eq!(mutes, ["LEDバー", "stage-evolution/minispot30", TWINKLE, PAR, "stage-evolution/laserscan300"]);
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
    // レーザー(ARMが必要)は、明るさのフェーダーには入れない(消灯ボタンと、ARMしたときだけ効く動きのプリセットはある)
    for b in &c.project.bindings {
        if let Action::Level { sel: Selector::Profile(p) } = &b.action {
            assert_ne!(p, "stage-evolution/laserscan300", "{}", b.name);
        }
    }
    let laser: Vec<&str> = c
        .project
        .bindings
        .iter()
        .filter(|b| b.page == Some(0) && matches!(&b.action, Action::SetValue { sel: Selector::Profile(p), .. } if p == "stage-evolution/laserscan300"))
        .map(|b| b.name.as_str())
        .collect();
    assert_eq!(laser, ["オートショー", "回転", "ズーム", "描画"], "7段目の残り(8個まで)");
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

    // トラックボタン1でLEDバーが全部消える。もう一度押すと戻る
    press(&mut c, Ctl::Track(0));
    c.render();
    assert!(lit(&c).iter().all(|&x| !x));
    press(&mut c, Ctl::Track(0));
    c.render();
    assert!(lit(&c).iter().all(|&x| x));

    // レーザーの消灯ボタン: ARMしていても安全値(レーザーOFF)になる
    for f in c.project.fixtures.iter_mut().filter(|f| f.profile == "stage-evolution/laserscan300") {
        f.armed = true;
    }
    let laser = c.project.fixtures.iter().find(|f| f.profile == "stage-evolution/laserscan300").unwrap().address as usize - 1;
    c.render();
    assert_eq!(c.rt.frames[0][laser], 255, "ARM中はDMX制御");
    press(&mut c, Ctl::Track(4));
    c.render();
    assert_eq!(c.rt.frames[0][laser], 0, "消灯中はレーザーOFF");

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

#[test]
fn settings_page_switches_roles_arm_and_ndi() {
    let mut c = rig();
    let lib = c.lib.clone();
    crate::layout::apply(&mut c.project, &lib);
    // 右の列の5番目で 2ページ目へ、もう一度で戻る
    press(&mut c, Ctl::Scene(4));
    assert_eq!(c.rt.page, 1);
    // 1段目の先頭 = LEDバーの系統の切り替え(全部メイン → 全部サブ)
    let bars = bar_ids(&c);
    let role = |c: &Core, id: u32| c.project.fixtures[c.project.fixture_index(id).unwrap()].color_role;
    assert!(bars.iter().all(|&id| role(&c, id) == crate::color::ColorRole::Main));
    press(&mut c, Ctl::Pad(pad_note(0, 0)));
    assert!(bars.iter().all(|&id| role(&c, id) == crate::color::ColorRole::Sub));
    // 2段目 = LEDバー1本ずつ
    press(&mut c, Ctl::Pad(pad_note(1, 0)));
    assert_eq!(role(&c, bars[0]), crate::color::ColorRole::Main);
    assert_eq!(role(&c, bars[1]), crate::color::ColorRole::Sub);
    // レーザーのARM(5段目)
    let lasers: Vec<usize> = c.project.fixtures.iter().enumerate().filter(|(_, f)| f.profile == "stage-evolution/laserscan300").map(|(i, _)| i).collect();
    assert!(matches!(binding_on(&c, Ctl::Pad(pad_note(4, 0)), 1).unwrap().action, Action::Arm { .. }));
    press(&mut c, Ctl::Pad(pad_note(4, 0)));
    assert!(c.project.fixtures[lasers[0]].armed && !c.project.fixtures[lasers[1]].armed);
    // NDI(6段目): メイン=映像 を押すと受信がONになり、もう一度で使わない
    press(&mut c, Ctl::Pad(pad_note(5, 0)));
    assert!(c.project.ndi.enabled && c.project.ndi.main == crate::ndi::LiveSource::Video);
    c.rt.ndi.video = Some([0.0, 1.0, 0.0]);
    c.render();
    assert_eq!(c.role_rgb(crate::color::ColorRole::Main), Some([0.0, 1.0, 0.0]), "映像の色がメイン色に");
    press(&mut c, Ctl::Pad(pad_note(5, 0)));
    assert_eq!(c.project.ndi.main, crate::ndi::LiveSource::Off);
    c.render();
    assert_ne!(c.role_rgb(crate::color::ColorRole::Main), Some([0.0, 1.0, 0.0]));
    // Link(6段目の5番目)
    press(&mut c, Ctl::Pad(pad_note(5, 4)));
    assert_eq!(c.project.tempo_source, TempoSource::Link);
    press(&mut c, Ctl::Pad(pad_note(5, 4)));
    assert_eq!(c.project.tempo_source, TempoSource::Internal);
    // フェーダー1 = レーザー1のX
    fader(&mut c, 0, 127);
    let f = &c.project.fixtures[lasers[0]];
    let m = c.lib.mode(&f.profile, f.mode).unwrap();
    assert_eq!(f.values[m.find_attr("pan").unwrap()], 127);
    // 戻る
    press(&mut c, Ctl::Scene(4));
    assert_eq!(c.rt.page, 0);
}

/// 実際の設定での確認用(普段は実行しない)。保存してあるプロジェクトを読み(書き込まない)、エフェクトを1つONにして出力を見る。
/// 実行: DMXCTL_FX=円運動 cargo test --release real_effect_output -- --ignored --nocapture
#[test]
#[ignore]
fn real_effect_output() {
    let (lib, _) = Library::load(&config_dir().join("fixtures"));
    let p = Project::load(&default_project_path()).unwrap();
    let mut c = Core::new(p, Arc::new(lib));
    let name = std::env::var("DMXCTL_FX").unwrap_or_else(|_| "円運動".into());
    for e in c.project.effects.iter_mut() {
        e.enabled = e.name == name;
    }
    for f in c.project.pixel_fx.iter_mut() {
        f.enabled = false;
    }
    let spot = c.project.fixtures.iter().find(|f| f.profile == "stage-evolution/minispot30").unwrap().clone();
    let m = c.lib.mode(&spot.profile, spot.mode).unwrap().clone();
    println!("{} mode={} addr={} effects: {:?}", spot.name, m.name, spot.address, c.project.effects.iter().filter(|e| e.name == name).map(|e| (&e.attr, e.min, e.max, e.beats, &e.sel)).collect::<Vec<_>>());
    for beat in [0.0, 1.0, 2.0, 3.0, 4.0, 6.0] {
        c.rt.beat = beat;
        c.rt.last_beat_t = None;
        c.render();
        let base = spot.address as usize - 1;
        let vals: Vec<String> = m.channels.iter().enumerate().map(|(k, ch)| format!("{}={}", ch.attr, c.rt.frames[spot.universe][base + k])).collect();
        println!("拍{beat}: {}", vals.join(" "));
    }
}

/// 実機での確認用(普段は実行しない)。保存してあるプロジェクトのムービングの動きを、1つ6秒ずつ実機に流す(プロジェクトは書き換えない)。
/// 実行: DMXCTL_TEST_PORT=/dev/cu.usbserial-XXXX cargo test --release real_moves_demo -- --ignored --nocapture
#[test]
#[ignore]
fn real_moves_demo() {
    let port = std::env::var("DMXCTL_TEST_PORT").expect("DMXCTL_TEST_PORT");
    let (lib, _) = Library::load(&config_dir().join("fixtures"));
    let p = Project::load(&default_project_path()).unwrap();
    let mut c = Core::new(p, Arc::new(lib));
    let mut out = crate::output::create(&OutputCfg { enabled: true, universe: 0, kind: OutputKind::OpenDmxSerial { port } });
    let names = ["円運動", "8の字", "パン揺れ", "チルト揺れ", "ワイパー", "クロス", "バラバラ", "高速円"];
    for f in c.project.pixel_fx.iter_mut() {
        f.enabled = false;
    }
    for name in names {
        for e in c.project.effects.iter_mut() {
            e.enabled = e.name == name;
        }
        println!("{name}");
        let t0 = std::time::Instant::now();
        while t0.elapsed().as_secs_f64() < 6.0 {
            c.render();
            out.send(&c.rt.frames[0], c.rt.used_len[0]);
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
    }
    // 最後は全部0
    out.send(&[0u8; 512], 512);
    std::thread::sleep(std::time::Duration::from_millis(300));
    println!("終了");
}
