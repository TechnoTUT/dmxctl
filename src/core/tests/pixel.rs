//! LEDバー(RGBピクセル)のテスト

use crate::core::color::{ColorRole, ColorTarget, NamedColor};
use crate::core::engine::*;
use crate::core::fixture::{self, split_pixel, Library};
use crate::core::pixel::{pixel_color, PixelAxis, PixelColor, PixelFx, PixelKind};
use crate::core::project::*;
use std::path::Path;
use std::sync::Arc;

const BAR: &str = "generic/led-bar-rgb";
const R: [f32; 3] = [1.0, 0.0, 0.0];
const B: [f32; 3] = [0.0, 0.0, 1.0];

fn core() -> Core {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    Core::new(Project::starter(&lib), lib)
}

/// `pixels`ピクセルのLEDバーを足す。戻り値=機材id
fn add_bar(c: &mut Core, pixels: usize) -> u32 {
    let lib = c.lib.clone();
    let id = c.project.add_fixture(&lib, BAR).unwrap();
    let mode = lib.get(BAR).unwrap().modes.iter().position(|m| m.pixels().len() == pixels).unwrap();
    let i = c.project.fixture_index(id).unwrap();
    let f = &mut c.project.fixtures[i];
    f.mode = mode;
    f.values = lib.mode(BAR, mode).unwrap().default_values();
    id
}

/// 出力(DMX)のピクセルの色 (r, g, b)
fn out_px(c: &Core, id: u32) -> Vec<(u8, u8, u8)> {
    let f = &c.project.fixtures[c.project.fixture_index(id).unwrap()];
    let base = f.address as usize - 1;
    let n = c.lib.mode(&f.profile, f.mode).unwrap().pixels().len();
    let fr = &c.rt.frames[f.universe];
    (0..n).map(|k| (fr[base + k * 3], fr[base + k * 3 + 1], fr[base + k * 3 + 2])).collect()
}

/// ベース値のピクセルの色
fn base_px(c: &Core, id: u32) -> Vec<(u8, u8, u8)> {
    let f = &c.project.fixtures[c.project.fixture_index(id).unwrap()];
    f.values.chunks(3).map(|x| (x[0], x[1], x[2])).collect()
}

#[test]
fn builtin_bar_profile_has_pixel_modes() {
    let (lib, _) = Library::load(Path::new("none"));
    let p = lib.get(BAR).unwrap();
    assert!(p.validate().errors.is_empty(), "{:?}", p.validate().errors);
    let counts: Vec<(usize, usize)> = p.modes.iter().map(|m| (m.channels.len(), m.pixels().len())).collect();
    assert!(counts.contains(&(3, 1)) && counts.contains(&(24, 8)) && counts.contains(&(96, 32)), "{counts:?}");
    let m = p.modes.iter().find(|m| m.pixels().len() == 8).unwrap();
    // RGBRGBRGB... の順
    let attrs: Vec<&str> = m.channels.iter().take(6).map(|c| c.attr.as_str()).collect();
    assert_eq!(attrs, ["red.1", "green.1", "blue.1", "red.2", "green.2", "blue.2"]);
    assert!(m.channels.iter().all(|c| c.safe == Some(0)), "ブラックアウトで消える");
}

#[test]
fn pixel_attr_names() {
    assert_eq!(split_pixel("red.12"), ("red", Some(12)));
    assert_eq!(split_pixel("dimmer"), ("dimmer", None));
    assert_eq!(split_pixel("pan_fine"), ("pan_fine", None));
    assert_eq!(split_pixel("red.0"), ("red.0", None));
    assert_eq!(split_pixel("a.b"), ("a.b", None));
}

#[test]
fn other_orders_and_rgbw() {
    let grb = fixture::pixel_channels(1, 2, "GRB");
    assert_eq!(grb.iter().map(|c| c.attr.as_str()).collect::<Vec<_>>(), ["green.1", "red.1", "blue.1", "green.2", "red.2", "blue.2"]);
    let m = fixture::Mode { name: "w".into(), channels: fixture::pixel_channels(1, 1, "RGBW") };
    let px = m.pixels();
    let mut v = vec![0u8; 4];
    m.set_pixel(&mut v, &px[0], [1.0, 0.5, 0.5]);
    assert_eq!(v, [127, 0, 0, 127], "共通成分は白へ");
}

#[test]
fn whole_bar_follows_attr_color_and_palette() {
    let mut c = core();
    let bar = add_bar(&mut c, 8);
    c.set_attr(&Selector::Fixture(bar), "red", 200.0);
    assert!(base_px(&c, bar).iter().all(|p| p.0 == 200), "「red」で全ピクセルの赤");
    let i = c.project.fixture_index(bar).unwrap();
    let m = c.lib.mode(BAR, c.project.fixtures[i].mode).unwrap();
    assert_eq!(m.find_attr("red"), Some(0));

    c.apply_named_color(NamedColor::Blue, ColorTarget::All);
    assert!(base_px(&c, bar).iter().all(|p| *p == (0, 0, 255)), "{:?}", base_px(&c, bar));

    // メイン/サブ: バーをサブにする
    c.set_color_role(bar, ColorRole::Sub);
    c.set_palette_color(ColorRole::Sub, NamedColor::Green.rgb());
    assert!(base_px(&c, bar).iter().all(|p| *p == (0, 255, 0)));
}

#[test]
fn master_and_blackout_reach_pixels() {
    let mut c = core();
    let bar = add_bar(&mut c, 4);
    c.apply_named_color(NamedColor::White, ColorTarget::All);
    c.rt.master = 0.5;
    c.render();
    assert!(out_px(&c, bar).iter().all(|p| *p == (127, 127, 127)), "{:?}", out_px(&c, bar));
    c.rt.master = 1.0;
    c.rt.blackout = true;
    c.render();
    assert!(out_px(&c, bar).iter().all(|p| *p == (0, 0, 0)));
}

#[test]
fn chase_moves_one_pixel_per_beat() {
    // 8ピクセル・8拍で1周 = 1拍で1ピクセル進む
    for (beat, lit) in [(0.0, 0usize), (2.0, 2), (7.5, 7), (8.0, 0)] {
        let px: Vec<bool> = (0..8).map(|i| pixel_color(PixelKind::Chase, i, 8, beat / 8.0, 1.0, R, B) == R).collect();
        let on: Vec<usize> = px.iter().enumerate().filter(|(_, x)| **x).map(|(i, _)| i).collect();
        assert_eq!(on, vec![lit], "拍 {beat}");
    }
    // 幅2
    let on = (0..8).filter(|&i| pixel_color(PixelKind::Chase, i, 8, 3.0 / 8.0, 2.0, R, B) == R).count();
    assert_eq!(on, 2);
}

#[test]
fn other_patterns() {
    // 交互(幅2): AABBAABB、次の周期で入れ替わる
    let row = |phase: f64| (0..8).map(|i| if pixel_color(PixelKind::Alternate, i, 8, phase, 2.0, R, B) == R { 'A' } else { 'B' }).collect::<String>();
    assert_eq!(row(0.2), "AABBAABB");
    assert_eq!(row(1.2), "BBAABBAA");
    // ワイプ: 前半はAで埋めていく、後半はBで埋め戻す
    let wipe = |phase: f64| (0..8).map(|i| if pixel_color(PixelKind::Wipe, i, 8, phase, 0.0, R, B) == R { 'A' } else { 'B' }).collect::<String>();
    assert_eq!(wipe(0.25), "AAAABBBB");
    assert_eq!(wipe(0.75), "BBBBAAAA");
    // レインボー: 隣どうしで色が違い、全体で一周する
    let rb: Vec<[f32; 3]> = (0..6).map(|i| pixel_color(PixelKind::Rainbow, i, 6, 0.0, 1.0, R, B)).collect();
    assert_eq!(rb[0], [1.0, 0.0, 0.0]);
    assert!(rb.windows(2).all(|w| w[0] != w[1]));
    // きらめき: 光る数はだいたい「幅」の数
    let lit: usize = (0..200).map(|s| (0..32).filter(|&i| pixel_color(PixelKind::Sparkle, i, 32, s as f64 / 8.0, 4.0, R, B) == R).count()).sum();
    let avg = lit as f32 / 200.0;
    assert!((2.5..5.5).contains(&avg), "{avg}");
    // グラデーション: 端はA、真ん中あたりはB寄り
    assert_eq!(pixel_color(PixelKind::Gradient, 0, 8, 0.0, 1.0, R, B), R);
    assert!(pixel_color(PixelKind::Gradient, 4, 8, 0.0, 1.0, R, B)[2] > 0.9);
}

#[test]
fn pixel_fx_render_spans_bars_and_uses_palette() {
    let mut c = core();
    let a = add_bar(&mut c, 4);
    let b = add_bar(&mut c, 4);
    c.set_palette_color(ColorRole::Main, NamedColor::Red.rgb());
    c.set_palette_color(ColorRole::Sub, NamedColor::Blue.rgb());
    c.project.pixel_fx.push(PixelFx { name: "流れる".into(), enabled: true, kind: PixelKind::Chase, width: 1.0, beats: 8.0, ..PixelFx::default() });
    // 2本(計8ピクセル)を1本につなげる: 5拍目 → 2本目の2番目
    c.rt.beat = 5.0;
    c.render();
    let lit = |c: &Core, id| out_px(c, id).iter().map(|p| p.0 == 255 && p.2 == 0).collect::<Vec<_>>();
    assert_eq!(lit(&c, a), [false; 4]);
    assert_eq!(lit(&c, b), [false, true, false, false]);
    // 各台で同じ動き
    c.project.pixel_fx[0].span = false;
    c.render();
    assert_eq!(lit(&c, a), lit(&c, b));
    // 逆向き
    c.project.pixel_fx[0].span = true;
    c.project.pixel_fx[0].reverse = true;
    c.render();
    assert_eq!(lit(&c, a), [false, false, true, false]);
    // メインとサブの入れ替えにも追従する
    c.swap_palette();
    c.render();
    assert!(out_px(&c, a).iter().filter(|p| p.0 == 255).count() == 3, "入れ替え後は赤が背景 {:?}", out_px(&c, a));
    // ベース値は書き換えない
    assert!(base_px(&c, a).iter().all(|p| *p == base_px(&c, a)[0]));
}

#[test]
fn pixel_fx_toggle_by_effect_pad() {
    let mut c = core();
    add_bar(&mut c, 8);
    c.project.pixel_fx.push(PixelFx { name: "虹".into(), kind: PixelKind::Rainbow, ..PixelFx::default() });
    c.project.bindings.push(Binding { id: 0,
        name: "b".into(),
        trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: 3 }),
        action: Action::Effect { name: "虹".into(), mode: PadMode::Toggle },
        page: None,
        osc: None,
    });
    c.handle_midi(MidiMsg { kind: TriggerKind::Note, channel: 0, number: 3, value: 127 });
    assert!(c.project.pixel_fx[0].enabled);
    let led = crate::core::led::compute(&c).into_iter().find(|(n, _)| *n == 3).unwrap().1;
    assert!(matches!(led, crate::core::led::Led::Rgb { level: crate::core::led::LV_100, .. }));
    c.handle_osc(&crate::core::io::osc::Message { addr: "/dmx/effect".into(), args: vec![crate::core::io::osc::Arg::Str("虹".into()), crate::core::io::osc::Arg::Float(0.0)] });
    assert!(!c.project.pixel_fx[0].enabled);
}

#[test]
fn scenes_store_each_pixel() {
    let mut c = core();
    let bar = add_bar(&mut c, 4);
    let i = c.project.fixture_index(bar).unwrap();
    c.project.fixtures[i].values = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120];
    let s = c.capture_scene("s", 0.0, &[bar], &Default::default());
    assert_eq!(s.values.len(), 12);
    c.project.scenes.push(s);
    c.apply_named_color(NamedColor::White, ColorTarget::All);
    c.recall_scene("s");
    assert_eq!(c.project.fixtures[i].values, vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120]);
}

#[test]
fn old_projects_have_no_pixel_fx() {
    let p: Project = serde_json::from_str("{}").unwrap();
    assert!(p.pixel_fx.is_empty());
    let fx: PixelFx = serde_json::from_str(r#"{"name":"x","kind":"Rainbow"}"#).unwrap();
    assert_eq!(fx.a, PixelColor::Main);
}


fn row(kind: PixelKind, phase: f64, width: f32) -> String {
    (0..8).map(|i| if pixel_color(kind, i, 8, phase, width, R, B) == R { 'A' } else if pixel_color(kind, i, 8, phase, width, R, B) == B { 'B' } else { '~' }).collect()
}

#[test]
fn stripes_scroll_seamlessly() {
    assert_eq!(row(PixelKind::Stripes, 0.0, 2.0), "AABBAABB");
    assert_eq!(row(PixelKind::Stripes, 0.25, 2.0), "BAABBAAB", "1ピクセル進む");
    assert_eq!(row(PixelKind::Stripes, 0.5, 2.0), "BBAABBAA");
    assert_eq!(row(PixelKind::Stripes, 1.0, 2.0), row(PixelKind::Stripes, 0.0, 2.0), "1周でつなぎ目なく元に戻る");
}

#[test]
fn wave_blends_main_and_sub_and_travels() {
    // 山(A)と谷(B)がなめらかにつながる
    assert_eq!(pixel_color(PixelKind::Wave, 0, 8, 0.0, 1.0, R, B), R);
    assert_eq!(pixel_color(PixelKind::Wave, 4, 8, 0.0, 1.0, R, B), B);
    let mid = pixel_color(PixelKind::Wave, 2, 8, 0.0, 1.0, R, B);
    assert!((mid[0] - 0.5).abs() < 1e-4 && (mid[2] - 0.5).abs() < 1e-4, "{mid:?}");
    // 1/4周でAの山が2ピクセル進む
    assert_eq!(pixel_color(PixelKind::Wave, 2, 8, 0.25, 1.0, R, B), R);
}

#[test]
fn alt_fade_crossfades_neighbours_in_opposite_directions() {
    assert_eq!(row(PixelKind::AltFade, 0.0, 1.0), "ABABABAB");
    assert_eq!(row(PixelKind::AltFade, 0.5, 1.0), "BABABABA");
    let c = pixel_color(PixelKind::AltFade, 0, 8, 0.25, 1.0, R, B);
    assert!((c[0] - 0.5).abs() < 1e-4, "途中は半分ずつ混ざる {c:?}");
    assert_eq!(row(PixelKind::AltFade, 0.0, 2.0), "AABBAABB");
}

#[test]
fn mirror_spreads_from_the_center() {
    let mut c = core();
    let bar = add_bar(&mut c, 8);
    c.set_palette_color(ColorRole::Main, NamedColor::Red.rgb());
    c.set_palette_color(ColorRole::Sub, NamedColor::Blue.rgb());
    c.project.pixel_fx.push(PixelFx { name: "m".into(), enabled: true, kind: PixelKind::Chase, width: 1.0, beats: 4.0, mirror: true, ..PixelFx::default() });
    let lit = |c: &Core| out_px(c, bar).iter().map(|p| if p.0 == 255 { 'A' } else { '.' }).collect::<String>();
    c.rt.beat = 0.0;
    c.render();
    assert_eq!(lit(&c), "...AA...", "中央から");
    c.rt.beat = 3.0;
    c.render();
    assert_eq!(lit(&c), "A......A", "端へ広がる");
}

#[test]
fn presets_mix_main_and_sub() {
    for i in 0..crate::core::pixel::PRESETS.len() {
        let fx = crate::core::pixel::from_preset(i).unwrap();
        assert_eq!((fx.a, fx.b), (PixelColor::Main, PixelColor::Sub));
        assert!(!fx.enabled);
    }
    let mut p = Project::default();
    assert_eq!(crate::core::pixel::add_presets(&mut p, 99), crate::core::pixel::PRESETS.len());
    assert_eq!(crate::core::pixel::add_presets(&mut p, 99), 0, "同じ名前は足さない");
    // 縞々は止まったまま
    let stripes = p.pixel_fx.iter().find(|f| f.name == "縞々").unwrap();
    assert_eq!(stripes.beats, 0.0);
}

#[test]
fn one_pattern_per_bar() {
    let mut c = core();
    let a = add_bar(&mut c, 8);
    let b = add_bar(&mut c, 8);
    c.project.pixel_fx.push(PixelFx { name: "縞".into(), sel: Selector::Fixture(a), ..PixelFx::default() });
    c.project.pixel_fx.push(PixelFx { name: "波".into(), sel: Selector::Fixture(a), kind: PixelKind::Wave, ..PixelFx::default() });
    c.project.pixel_fx.push(PixelFx { name: "別のバー".into(), sel: Selector::Fixture(b), ..PixelFx::default() });
    c.set_pixel_fx("別のバー", true);
    c.set_pixel_fx("縞", true);
    c.set_pixel_fx("波", true);
    let on: Vec<(&str, bool)> = c.project.pixel_fx.iter().map(|f| (f.name.as_str(), f.enabled)).collect();
    assert_eq!(on, [("縞", false), ("波", true), ("別のバー", true)], "同じバーの模様だけが切り替わる");
}

#[test]
fn pad_template_places_patterns_on_top_row() {
    let mut c = core();
    add_bar(&mut c, 8);
    let (added, skipped) = crate::core::pixel::add_pad_template(&mut c.project, None);
    assert_eq!((added, skipped), (8, 0));
    assert_eq!(c.project.pixel_fx.len(), 8);
    let press = |c: &mut Core, n: u8| {
        c.handle_midi(MidiMsg { kind: TriggerKind::Note, channel: 0, number: n, value: 127 });
        c.handle_midi(MidiMsg { kind: TriggerKind::Note, channel: 0, number: n, value: 0 });
    };
    press(&mut c, 0x38); // 左上 = 縞々
    press(&mut c, 0x3C); // 5番目 = ウェーブ
    let on: Vec<&str> = c.project.pixel_fx.iter().filter(|f| f.enabled).map(|f| f.name.as_str()).collect();
    assert_eq!(on, ["ウェーブ"], "パッドで切り替えると、前の模様はOFFになる");
    let led = |c: &Core, n: u8| crate::core::led::compute(c).into_iter().find(|(x, _)| *x == n).unwrap().1;
    assert!(matches!(led(&c, 0x3C), crate::core::led::Led::Rgb { level: crate::core::led::LV_100, .. }));
    assert!(!matches!(led(&c, 0x38), crate::core::led::Led::Rgb { level: crate::core::led::LV_100, .. }));
    // もう一度押しても、置いてある模様は二重に置かない
    assert_eq!(crate::core::pixel::add_pad_template(&mut c.project, None), (0, 0));
    assert_eq!(c.project.bindings.iter().filter(|b| matches!(b.action, Action::Effect { .. })).count(), 8);
}

#[test]
fn pad_template_uses_the_next_free_pad() {
    let mut c = core();
    // 左上のパッドが使用中なら、そこを飛ばして詰めて置く(最上段の残り7個 + 2段目の左端)
    c.project.bindings.push(Binding { id: 0, name: "x".into(), trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: 0x38 }), action: Action::Blackout { mode: PadMode::Toggle }, page: None, osc: None });
    assert_eq!(crate::core::pixel::add_pad_template(&mut c.project, None), (8, 0));
    let pad_of = |name: &str| c.project.bindings.iter().find(|b| matches!(&b.action, Action::Effect { name: n, .. } if n == name)).and_then(|b| b.trigger).map(|t| t.number);
    assert_eq!(pad_of("縞々"), Some(0x39));
    assert_eq!(pad_of("センター縞"), Some(0x30));
    // ページ専用: 別のページの割り当てとは重ならない扱い
    let mut d = core();
    d.project.bindings.push(Binding { id: 0, name: "y".into(), trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: 0x38 }), action: Action::Blackout { mode: PadMode::Toggle }, page: Some(0), osc: None });
    assert_eq!(crate::core::pixel::add_pad_template(&mut d.project, Some(1)), (8, 0));
    assert!(d.project.bindings.iter().any(|b| b.page == Some(1) && b.trigger.is_some_and(|t| t.number == 0x38)));
}
// ---------------------------------------------------------------- 90ピクセルRGBWのバー8本(面)

const BAR90: &str = "test/bar90-rgbw";

/// 90ピクセルRGBW(360ch)のバー8本を、ユニバース2〜9のアドレス1に置く。戻り値=機材id(パッチ順)
fn core_with_8_bars() -> (Core, Vec<u32>) {
    let (mut lib, _) = Library::load(Path::new("none"));
    lib.add(fixture::Profile {
        id: BAR90.into(),
        name: "LEDバー 90px RGBW".into(),
        manufacturer: String::new(),
        needs_arm: false,
        modes: vec![fixture::Mode { name: "360ch".into(), channels: fixture::pixel_channels(1, 90, "RGBW") }],
    });
    let lib = Arc::new(lib);
    let mut c = Core::new(Project::starter(&lib), lib.clone());
    let ids = (0..8)
        .map(|u| {
            let id = c.project.add_fixture(&lib, BAR90).unwrap();
            let i = c.project.fixture_index(id).unwrap();
            c.project.fixtures[i].universe = u + 1;
            c.project.fixtures[i].address = 1;
            id
        })
        .collect();
    (c, ids)
}

/// 出力(DMX)のRGBW。[バー][ピクセル]
fn out_rgbw(c: &Core, ids: &[u32]) -> Vec<Vec<[u8; 4]>> {
    ids.iter()
        .map(|&id| {
            let f = &c.project.fixtures[c.project.fixture_index(id).unwrap()];
            let fr = &c.rt.frames[f.universe];
            (0..90).map(|k| [fr[k * 4], fr[k * 4 + 1], fr[k * 4 + 2], fr[k * 4 + 3]]).collect()
        })
        .collect()
}

/// 赤が点いているピクセルの番号
fn lit(bar: &[[u8; 4]]) -> Vec<usize> {
    bar.iter().enumerate().filter(|(_, p)| p[0] > 0).map(|(k, _)| k).collect()
}

fn red_chase(c: &mut Core) -> &mut PixelFx {
    c.project.pixel_fx.push(PixelFx { enabled: true, kind: PixelKind::Chase, a: PixelColor::Custom(R), b: PixelColor::Off, width: 1.0, beats: 8.0, span: false, ..PixelFx::default() });
    c.project.pixel_fx.last_mut().unwrap()
}

#[test]
fn eight_rgbw_bars_fill_their_own_universes() {
    let (mut c, ids) = core_with_8_bars();
    c.apply_named_color(NamedColor::White, ColorTarget::All);
    c.render();
    for (b, bar) in out_rgbw(&c, &ids).iter().enumerate() {
        assert!(bar.iter().all(|p| *p == [0, 0, 0, 255]), "バー{} 白はWチャンネルへ: {:?}", b + 1, &bar[..2]);
    }
    for u in 1..=8 {
        assert_eq!(c.rt.used_len[u], 360, "ユニバース{}", u + 1);
        assert!(c.rt.frames[u][360..].iter().all(|&v| v == 0), "361ch以降は空");
    }
}

#[test]
fn across_axis_steps_bar_by_bar() {
    let (mut c, ids) = core_with_8_bars();
    red_chase(&mut c).axis = PixelAxis::Across;
    // 8本・8拍で1周 = 1拍で1本進む。そのバーは全ピクセルが点く
    for beat in [0.0, 3.0, 7.5] {
        c.rt.beat = beat;
        c.render();
        let on: Vec<usize> = out_rgbw(&c, &ids).iter().enumerate().filter(|(_, bar)| !lit(bar).is_empty()).map(|(b, _)| b).collect();
        assert_eq!(on, vec![beat as usize], "拍 {beat}");
        assert_eq!(lit(&out_rgbw(&c, &ids)[beat as usize]).len(), 90);
    }
}

#[test]
fn skew_shifts_each_bar_for_a_diagonal() {
    let (mut c, ids) = core_with_8_bars();
    red_chase(&mut c).skew = 1.0;
    c.rt.beat = 0.0;
    c.render();
    // 端から端(8本)で1周ずれる = 1本ごとに90/8ピクセルずつ先へ
    let heads: Vec<usize> = out_rgbw(&c, &ids).iter().map(|bar| lit(bar)[0]).collect();
    assert_eq!(heads, vec![0, 11, 22, 33, 45, 56, 67, 78]);
    // ずれ0なら全部そろう
    c.project.pixel_fx[0].skew = 0.0;
    c.render();
    assert!(out_rgbw(&c, &ids).iter().all(|bar| lit(bar) == vec![0]));
}

/// 性能の目安(普段は実行しない)。実行: cargo test --release perf_eight_bars -- --ignored --nocapture
#[test]
#[ignore]
fn perf_eight_bars() {
    let (mut c, _) = core_with_8_bars();
    c.project.pixel_fx.push(PixelFx { enabled: true, kind: PixelKind::Wave, skew: 1.0, ..PixelFx::default() });
    let t = std::time::Instant::now();
    let n = 2000;
    for i in 0..n {
        c.rt.beat = i as f64 * 0.01;
        c.render();
    }
    let per = t.elapsed().as_secs_f64() * 1e6 / n as f64;
    println!("8本×90ピクセルRGBW: 1フレーム {per:.1} µs (40fpsで CPU {:.2}%)", per * 40.0 / 1e4);
}
