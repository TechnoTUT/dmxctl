//! 同期(Ableton Link / MIDIクロック)・ページ・位置プリセット・OSC のテスト

use crate::engine::*;
use crate::fixture::Library;
use crate::osc::{self, Arg, Message};
use crate::project::*;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

const SPOT: u32 = 1;

fn core() -> Core {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    Core::new(Project::starter(&lib), lib)
}

fn note(n: u8, v: u8) -> MidiMsg {
    MidiMsg { kind: TriggerKind::Note, channel: 0, number: n, value: v }
}

fn cc(n: u8, v: u8) -> MidiMsg {
    MidiMsg { kind: TriggerKind::Cc, channel: 0, number: n, value: v }
}

fn bind(c: &mut Core, kind: TriggerKind, num: u8, page: Option<u8>, action: Action) {
    c.project.bindings.push(Binding { name: "b".into(), trigger: Some(MidiTrigger { kind, channel: None, number: num }), action, page, osc: None });
}

fn dimmer(c: &Core) -> f32 {
    let i = c.project.fixture_index(SPOT).unwrap();
    let fx = &c.project.fixtures[i];
    let m = c.lib.mode(&fx.profile, fx.mode).unwrap();
    m.get_f(&fx.values, m.find_attr("dimmer").unwrap())
}

fn attr(c: &Core, id: u32, a: &str) -> f32 {
    let i = c.project.fixture_index(id).unwrap();
    let fx = &c.project.fixtures[i];
    let m = c.lib.mode(&fx.profile, fx.mode).unwrap();
    m.get_f(&fx.values, m.find_attr(a).unwrap())
}

fn msg(addr: &str, args: Vec<Arg>) -> Message {
    Message { addr: addr.into(), args }
}

// ------------------------------------------------------------ MIDIクロック

/// 120BPM のクロック(1拍24回 = 20.833ms間隔)を`n`回送る
fn send_ticks(c: &mut Core, start: Instant, from: u32, n: u32, bpm: f64) -> Instant {
    let dt = 60.0 / bpm / 24.0;
    let mut t = start;
    for k in from..from + n {
        t = start + Duration::from_secs_f64(k as f64 * dt);
        c.clock_msg(ClockMsg::Tick, t);
    }
    t
}

#[test]
fn midi_clock_sets_bpm_and_beat() {
    let mut c = core();
    c.set_tempo_source(TempoSource::MidiClock);
    let t0 = Instant::now();
    c.clock_msg(ClockMsg::Start, t0);
    let last = send_ticks(&mut c, t0, 0, 49, 128.0); // 2拍ぶん + 1
    assert!((c.project.bpm - 128.0).abs() < 0.2, "BPM {}", c.project.bpm);
    // Start直後の最初のクロックが拍0。48回後 = 2拍目の頭
    let b = c.clock_beat(last).unwrap();
    assert!((b - 2.0).abs() < 1e-9, "拍 {b}");
    // 次のクロックまでの間は滑らかに進むが、次のクロックの位置は越えない
    let later = c.clock_beat(last + Duration::from_millis(15)).unwrap();
    assert!(later > 2.0 && later <= 2.0 + 1.0 / 24.0 + 1e-9, "{later}");
    // 途切れたら None(内部のテンポで走る)
    assert!(c.clock_beat(last + Duration::from_secs(1)).is_none());
    assert!(!c.clock_alive(last + Duration::from_secs(1)));
}

#[test]
fn midi_clock_resumes_without_jumping_and_resyncs() {
    let mut c = core();
    c.set_tempo_source(TempoSource::MidiClock);
    c.rt.beat = 10.25;
    let t0 = Instant::now();
    // Startなしで途中から来たクロックは、今の拍の位置から数え始める
    let last = send_ticks(&mut c, t0, 0, 13, 120.0);
    let b = c.clock_beat(last).unwrap();
    assert!((b - 10.75).abs() < 1e-9, "{b}");
    // 頭出しで小節の頭(0)へ
    c.resync();
    assert!(c.clock_beat(last).unwrap().abs() < 1e-9);
    let last = send_ticks(&mut c, t0, 13, 24, 120.0);
    assert!((c.clock_beat(last).unwrap() - 1.0).abs() < 1e-9);
}

#[test]
fn midi_clock_is_ignored_unless_selected() {
    let mut c = core();
    let t0 = Instant::now();
    c.clock_msg(ClockMsg::Start, t0);
    send_ticks(&mut c, t0, 0, 30, 90.0);
    assert_eq!(c.project.bpm, 120.0);
    assert!(c.clock_beat(t0).is_none());
}

// ------------------------------------------------------------ Ableton Link

#[test]
fn link_drives_beat_and_receives_tempo_changes() {
    let mut c = core();
    c.set_tempo_source(TempoSource::Link);
    assert!(c.rt.link.is_some());
    c.set_bpm(100.0);
    c.resync();
    c.render();
    let b0 = c.rt.beat;
    assert!(b0.abs() < 0.1, "頭出し直後 {b0}");
    std::thread::sleep(Duration::from_millis(300));
    c.render();
    // 100BPM で 0.3秒 = 0.5拍
    assert!((c.rt.beat - b0 - 0.5).abs() < 0.1, "{b0} -> {}", c.rt.beat);
    assert_eq!(c.project.bpm, 100.0);

    // 同じPCの別のLink参加者(映像ソフトの代わり)がテンポを変えると、こちらも追従する
    let other = crate::link::Link::start(100.0).unwrap();
    let deadline = Instant::now() + Duration::from_secs(8);
    while other.peers() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(other.peers() > 0, "Linkの相手が見つからない");
    other.set_tempo(140.0);
    let deadline = Instant::now() + Duration::from_secs(5);
    while (c.project.bpm - 140.0).abs() > 0.01 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        c.render();
    }
    assert!((c.project.bpm - 140.0).abs() < 0.01, "相手のテンポに追従しない: {}", c.project.bpm);
    assert!(c.rt.link_peers > 0);
    // こちらのTAP/BPM変更は相手に伝わる
    c.set_bpm(90.0);
    let deadline = Instant::now() + Duration::from_secs(5);
    while (other.state(4.0).unwrap().0 - 90.0).abs() > 0.01 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!((other.state(4.0).unwrap().0 - 90.0).abs() < 0.01);
    // 拍の位置も揃っている(1小節の中での位置)
    c.render();
    let mine = c.rt.beat.rem_euclid(4.0);
    let theirs = other.state(4.0).unwrap().1.rem_euclid(4.0);
    let diff = (mine - theirs).abs().min(4.0 - (mine - theirs).abs());
    assert!(diff < 0.1, "拍がずれている: {mine} / {theirs}");

    c.set_tempo_source(TempoSource::Internal);
    assert!(c.rt.link.is_none());
}

// ------------------------------------------------------------ ページ

#[test]
fn bindings_follow_the_current_page() {
    let mut c = core();
    let on = |v| Action::SetValue { sel: Selector::Fixture(SPOT), attr: "dimmer".into(), mode: PadMode::Trigger, on: v, off: 0.0 };
    bind(&mut c, TriggerKind::Note, 10, Some(0), on(50.0));
    bind(&mut c, TriggerKind::Note, 10, Some(1), on(200.0));
    bind(&mut c, TriggerKind::Note, 0x70, None, Action::Page { op: PageOp::Go(1) });
    bind(&mut c, TriggerKind::Note, 0x71, None, Action::Page { op: PageOp::Prev });

    c.handle_midi(note(10, 127));
    c.handle_midi(note(10, 0));
    assert_eq!(dimmer(&c), 50.0, "ページ1の割り当てだけが効く");

    c.handle_midi(note(0x70, 127));
    c.handle_midi(note(0x70, 0));
    assert_eq!(c.rt.page, 1);
    c.handle_midi(note(10, 127));
    c.handle_midi(note(10, 0));
    assert_eq!(dimmer(&c), 200.0, "ページ2の割り当てに切り替わる");

    c.handle_midi(note(0x71, 127));
    assert_eq!(c.rt.page, 0);
    c.handle_midi(note(0x71, 0));
    c.handle_midi(note(0x71, 127));
    assert_eq!(c.rt.page, PAGES - 1, "前のページは端で一周する");
}

#[test]
fn held_pad_is_released_after_page_change() {
    let mut c = core();
    bind(&mut c, TriggerKind::Note, 5, Some(0), Action::SetValue { sel: Selector::Fixture(SPOT), attr: "dimmer".into(), mode: PadMode::Momentary, on: 255.0, off: 0.0 });
    c.handle_midi(note(5, 127));
    assert_eq!(dimmer(&c), 255.0);
    c.rt.page = 3;
    c.handle_midi(note(5, 0));
    assert_eq!(dimmer(&c), 0.0, "押している間の動作が戻らなくならない");
    // 別のページでは反応しない
    c.handle_midi(note(5, 127));
    assert_eq!(dimmer(&c), 0.0);
}

#[test]
fn faders_on_other_pages_do_not_react() {
    let mut c = core();
    bind(&mut c, TriggerKind::Cc, 0x30, Some(0), Action::SetValue { sel: Selector::Fixture(SPOT), attr: "dimmer".into(), mode: PadMode::Continuous, on: 255.0, off: 0.0 });
    c.handle_midi(cc(0x30, 127));
    assert_eq!(dimmer(&c), 255.0);
    c.rt.page = 1;
    c.handle_midi(cc(0x30, 0));
    assert_eq!(dimmer(&c), 255.0);
}

#[test]
fn leds_show_only_the_current_page() {
    let mut c = core();
    bind(&mut c, TriggerKind::Note, 0, Some(1), Action::Blackout { mode: PadMode::Toggle });
    bind(&mut c, TriggerKind::Note, 0x70, None, Action::Page { op: PageOp::Go(0) });
    bind(&mut c, TriggerKind::Note, 0x71, None, Action::Page { op: PageOp::Go(1) });
    let led = |c: &Core, n: u8| crate::led::compute(c).into_iter().find(|(x, _)| *x == n).unwrap().1;
    assert_eq!(led(&c, 0), crate::led::Led::Off);
    assert_eq!(led(&c, 0x70), crate::led::Led::Single(2), "今のページのボタンは点滅");
    assert_eq!(led(&c, 0x71), crate::led::Led::Single(1));
    c.rt.page = 1;
    assert!(matches!(led(&c, 0), crate::led::Led::Rgb { .. }));
    assert_eq!(led(&c, 0x71), crate::led::Led::Single(2));
}

// ------------------------------------------------------------ 位置プリセット

#[test]
fn position_presets_capture_and_recall_pan_tilt() {
    let mut c = core();
    c.set_attr(&Selector::Fixture(SPOT), "pan", 100.0);
    c.set_attr(&Selector::Fixture(SPOT), "tilt", 30.0);
    c.set_attr(&Selector::Fixture(SPOT), "dimmer", 77.0);
    let p = c.capture_position("客席", 0.0);
    assert!(p.values.iter().all(|v| v.attr == "pan" || v.attr == "tilt"), "パン/チルトだけを記録");
    assert!(p.values.iter().any(|v| v.fixture == SPOT));
    c.project.positions.push(p);

    c.set_attr(&Selector::Fixture(SPOT), "pan", 0.0);
    c.set_attr(&Selector::Fixture(SPOT), "tilt", 255.0);
    c.set_attr(&Selector::Fixture(SPOT), "dimmer", 10.0);
    bind(&mut c, TriggerKind::Note, 20, None, Action::Position { name: "客席".into() });
    c.handle_midi(note(20, 127));
    assert_eq!(attr(&c, SPOT, "pan"), 100.0);
    assert_eq!(attr(&c, SPOT, "tilt"), 30.0);
    assert_eq!(attr(&c, SPOT, "dimmer"), 10.0, "光量は変えない");
    assert_eq!(c.rt.last_position.as_deref(), Some("客席"));
    let lit = crate::led::compute(&c).into_iter().find(|(n, _)| *n == 20).unwrap().1;
    assert!(matches!(lit, crate::led::Led::Rgb { level: crate::led::LV_100, .. }), "呼び出し中の位置は明るく光る");
}

#[test]
fn position_recall_fades() {
    let mut c = core();
    c.set_attr(&Selector::Fixture(SPOT), "pan", 200.0);
    let p = c.capture_position("上", 1.0);
    c.project.positions.push(p);
    c.set_attr(&Selector::Fixture(SPOT), "pan", 0.0);
    c.recall_position("上");
    assert_eq!(attr(&c, SPOT, "pan"), 0.0, "すぐには飛ばない");
    assert!(!c.rt.fades.is_empty());
}

// ------------------------------------------------------------ OSC

#[test]
fn osc_packets_roundtrip_including_bundles() {
    let m = osc::encode("/dmx/effect", &[Arg::Str("回転".into()), Arg::Float(1.0), Arg::Int(3), Arg::Bool(true)]);
    let got = osc::parse(&m);
    assert_eq!(got, vec![msg("/dmx/effect", vec![Arg::Str("回転".into()), Arg::Float(1.0), Arg::Int(3), Arg::Bool(true)])]);

    let a = osc::encode("/a", &[Arg::Float(0.25)]);
    let b = osc::encode("/b", &[]);
    let mut bundle = b"#bundle\0".to_vec();
    bundle.extend([0, 0, 0, 0, 0, 0, 0, 1]);
    for el in [&a, &b] {
        bundle.extend((el.len() as i32).to_be_bytes());
        bundle.extend(el.iter());
    }
    let got = osc::parse(&bundle);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0], msg("/a", vec![Arg::Float(0.25)]));
    assert_eq!(got[1].addr, "/b");

    // 壊れたパケットでも落ちない
    assert!(osc::parse(b"garbage").is_empty());
    assert!(osc::parse(&a[..a.len() - 2]).is_empty() || osc::parse(&a[..a.len() - 2])[0].args.is_empty());
    assert!(osc::parse(&[]).is_empty());
}

#[test]
fn osc_builtin_addresses() {
    let mut c = core();
    c.handle_osc(&msg("/dmx/master", vec![Arg::Float(0.25)]));
    assert_eq!(c.rt.master, 0.25);
    c.handle_osc(&msg("/dmx/bpm", vec![Arg::Float(128.0)]));
    assert_eq!(c.project.bpm, 128.0);
    c.handle_osc(&msg("/dmx/fixture/1/dimmer", vec![Arg::Float(1.0)]));
    assert_eq!(dimmer(&c), 255.0);
    c.handle_osc(&msg("/dmx/all/dimmer", vec![Arg::Float(0.0)]));
    assert_eq!(dimmer(&c), 0.0);
    c.handle_osc(&msg("/dmx/page", vec![Arg::Int(3)]));
    assert_eq!(c.rt.page, 2);
    c.handle_osc(&msg("/dmx/blackout", vec![Arg::Int(1)]));
    assert!(c.rt.blackout);
    c.handle_osc(&msg("/dmx/blackout", vec![Arg::Int(0)]));
    assert!(!c.rt.blackout);
    c.handle_osc(&msg("/dmx/color", vec![Arg::Str("blue".into())]));
    assert_eq!(c.rt.last_color, Some(crate::color::NamedColor::Blue));
    c.handle_osc(&msg("/dmx/color", vec![Arg::Str("赤".into())]));
    assert_eq!(c.rt.last_color, Some(crate::color::NamedColor::Red));
    c.handle_osc(&msg("/dmx/hue", vec![Arg::Float(0.5)]));
    assert_eq!(c.rt.hue, 180.0);
    c.rt.beat = 3.4;
    c.handle_osc(&msg("/dmx/resync", vec![Arg::Float(0.0)]));
    assert_eq!(c.rt.beat, 3.4, "ボタンを離した(0)ときは何もしない");
    c.handle_osc(&msg("/dmx/resync", vec![]));
    assert_eq!(c.rt.beat, 0.0);
    // 知らないアドレスは無視
    c.handle_osc(&msg("/other/thing", vec![Arg::Float(1.0)]));
    assert!(c.rt.last_osc.starts_with("/other/thing"));
}

#[test]
fn osc_triggers_bindings_and_learns_addresses() {
    let mut c = core();
    c.project.bindings.push(Binding {
        name: "t".into(),
        trigger: None,
        action: Action::SetValue { sel: Selector::Fixture(SPOT), attr: "dimmer".into(), mode: PadMode::Toggle, on: 255.0, off: 0.0 },
        page: Some(4),
        osc: None,
    });
    let idx = c.project.bindings.len() - 1;
    // OSCラーン
    c.rt.osc_learn = Some(idx);
    c.handle_osc(&msg("/td/button1", vec![Arg::Float(1.0)]));
    assert_eq!(c.project.bindings[idx].osc.as_deref(), Some("/td/button1"));
    assert_eq!(dimmer(&c), 0.0, "ラーンのときは実行しない");
    // トグル: 押す(1)→離す(0)で1回切り替わる。ページには関係なく反応する
    c.handle_osc(&msg("/td/button1", vec![Arg::Float(1.0)]));
    c.handle_osc(&msg("/td/button1", vec![Arg::Float(0.0)]));
    assert_eq!(dimmer(&c), 255.0);
    // 値なしも1回押した扱い
    c.handle_osc(&msg("/td/button1", vec![]));
    assert_eq!(dimmer(&c), 0.0);

    // 連続値(0〜1)はMIDIより細かく届く
    c.project.bindings.push(Binding { name: "m".into(), trigger: None, action: Action::Master { mode: PadMode::Continuous, on: 100.0, off: 0.0 }, page: None, osc: Some("/td/master".into()) });
    c.handle_osc(&msg("/td/master", vec![Arg::Float(0.333)]));
    assert!((c.rt.master - 0.333).abs() < 1e-4);
}

#[test]
fn osc_server_receives_udp() {
    let core: Shared = Arc::new(std::sync::Mutex::new(core()));
    let port = 39000 + (std::process::id() % 500) as u16;
    let server = osc::OscServer::start(core.clone(), None, port).expect("待ち受けできない");
    let sock = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    sock.send_to(&osc::encode("/dmx/master", &[Arg::Float(0.5)]), ("127.0.0.1", port)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while lock(&core).rt.master != 0.5 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(lock(&core).rt.master, 0.5);
    drop(server);
    // 止めたらポートが空く
    assert!(std::net::UdpSocket::bind(("0.0.0.0", port)).is_ok());
}

#[test]
fn old_project_files_load_with_new_fields_defaulted() {
    let json = r#"{"fixtures":[],"bindings":[{"name":"x","trigger":null,"action":{"Blackout":{"mode":"Toggle"}}}]}"#;
    let p: Project = serde_json::from_str(json).unwrap();
    assert_eq!(p.bindings[0].page, None);
    assert_eq!(p.bindings[0].osc, None);
    assert_eq!(p.tempo_source, TempoSource::Internal);
    assert_eq!(p.osc_port, 9000);
    assert!(p.positions.is_empty());
    // 保存し直しても、使っていない項目は書かない
    let s = serde_json::to_string(&p.bindings[0]).unwrap();
    assert!(!s.contains("page") && !s.contains("osc"), "{s}");
}

#[test]
fn osc_rgb_glides_to_the_received_color_and_yields_to_manual_colors() {
    use crate::color::{ColorRole, ColorTarget, NamedColor};
    let mut c = core();
    // 速さ0: すぐその色。0〜255で送ってきても同じ
    c.handle_osc(&msg("/dmx/main/rgb", vec![Arg::Int(255), Arg::Int(0), Arg::Int(0), Arg::Float(0.0)]));
    c.render();
    assert_eq!(c.role_rgb(ColorRole::Main), Some([1.0, 0.0, 0.0]));
    // 速さ1秒: 1秒たつと6割ほど近づく(いきなり変わらない)
    c.handle_osc(&msg("/dmx/main/rgb", vec![Arg::Float(0.0), Arg::Float(0.0), Arg::Float(1.0), Arg::Float(1.0)]));
    let o = c.rt.osc_rgb[0].as_mut().unwrap();
    o.last -= Duration::from_secs(1);
    c.render();
    let [r, _, b] = c.role_rgb(ColorRole::Main).unwrap();
    assert!((b - 0.632).abs() < 0.02 && (r - 0.368).abs() < 0.02, "{r} {b}");
    // 標準の速さは /dmx/rgb/smooth で変えられる
    c.handle_osc(&msg("/dmx/rgb/smooth", vec![Arg::Float(2.0)]));
    assert_eq!(c.project.osc_rgb_secs, 2.0);
    // 手で色を選んだら、OSCの色はやめる
    c.apply_named_color(NamedColor::Green, ColorTarget::Main);
    c.render();
    assert!(c.rt.osc_rgb[0].is_none());
    assert_eq!(c.role_rgb(ColorRole::Main), Some(NamedColor::Green.rgb()));
    // /dmx/rgb/off でも戻る
    c.handle_osc(&msg("/dmx/sub/rgb", vec![Arg::Float(1.0), Arg::Float(1.0), Arg::Float(0.0), Arg::Float(0.0)]));
    c.render();
    assert_eq!(c.role_rgb(ColorRole::Sub), Some([1.0, 1.0, 0.0]));
    c.handle_osc(&msg("/dmx/rgb/off", vec![]));
    c.render();
    assert_eq!(c.role_rgb(ColorRole::Sub), Some(c.project.palette.sub));
}

#[test]
fn osc_phrase_runs_bindings_or_recalls_a_scene_of_the_same_name() {
    let mut c = core();
    c.project.scenes.push(Scene { name: "サビ".into(), fade: 0.0, values: Vec::new() });
    // 番号つき・大文字でも、日本語の名前のシーンを呼び出す
    c.handle_osc(&msg("/dmx/phrase", vec![Arg::Str("Chorus".into())]));
    assert_eq!(c.rt.last_scene.as_deref(), Some("サビ"));
    assert_eq!(c.rt.phrase, "Chorus");
    // /dmx/phrase/<名前> に割り当てた動作があれば、それを実行する(アドレスに名前を入れて送ってきても同じ)
    c.project.bindings.push(Binding {
        name: "intro".into(),
        trigger: None,
        action: Action::SetValue { sel: Selector::Fixture(SPOT), attr: "dimmer".into(), mode: PadMode::Toggle, on: 255.0, off: 0.0 },
        page: None,
        osc: Some("/dmx/phrase/intro".into()),
    });
    c.handle_osc(&msg("/dmx/phrase", vec![Arg::Str("intro".into())]));
    assert_eq!(dimmer(&c), 255.0);
    c.handle_osc(&msg("/dmx/phrase/verse2", vec![]));
    assert_eq!(c.rt.phrase, "verse2");
    // フレーズの終わりの0では何もしない
    c.handle_osc(&msg("/dmx/phrase", vec![Arg::Str("intro".into()), Arg::Float(0.0)]));
    assert_eq!(dimmer(&c), 255.0);
}

#[test]
fn osc_rgb_at_reaches_the_lookahead_color_exactly_on_time() {
    use crate::color::ColorRole;
    let mut c = core();
    c.handle_osc(&msg("/dmx/main/rgb", vec![Arg::Float(1.0), Arg::Float(0.0), Arg::Float(0.0), Arg::Float(0.0)]));
    c.render();
    // 1秒後に青: 0.5秒たてば半分
    c.handle_osc(&msg("/dmx/main/rgb/at", vec![Arg::Float(0.0), Arg::Float(0.0), Arg::Float(1.0), Arg::Float(1.0)]));
    let o = c.rt.osc_rgb[0].as_mut().unwrap();
    o.last -= Duration::from_millis(500);
    o.due = o.due.map(|d| d - Duration::from_millis(500));
    c.render();
    let [r, _, b] = c.role_rgb(ColorRole::Main).unwrap();
    assert!((r - 0.5).abs() < 0.02 && (b - 0.5).abs() < 0.02, "{r} {b}");
    // 着く時刻を過ぎたら、ちょうどその色のまま
    let o = c.rt.osc_rgb[0].as_mut().unwrap();
    o.last -= Duration::from_secs(2);
    o.due = o.due.map(|d| d - Duration::from_secs(2));
    c.render();
    assert_eq!(c.role_rgb(ColorRole::Main), Some([0.0, 0.0, 1.0]));
}

#[test]
fn osc_level_scales_the_overall_brightness() {
    let mut c = core();
    c.handle_osc(&msg("/dmx/fixture/1/dimmer", vec![Arg::Float(1.0)]));
    let out = |c: &mut Core| {
        c.render();
        let i = c.project.fixture_index(SPOT).unwrap();
        let fx = &c.project.fixtures[i];
        let info = c.lib.info(&fx.profile, fx.mode).unwrap();
        let ch = info.master[0];
        c.rt.frames[fx.universe][fx.address as usize - 1 + ch] as f32
    };
    assert_eq!(out(&mut c), 255.0);
    // 効き具合100%: 0.5 で半分。すぐ(速さ0)
    c.handle_osc(&msg("/dmx/level/depth", vec![Arg::Float(1.0)]));
    c.handle_osc(&msg("/dmx/level", vec![Arg::Float(0.5), Arg::Float(0.0)]));
    assert!((out(&mut c) - 127.5).abs() <= 1.0);
    // 効き具合50%: 0 でも半分まで
    c.handle_osc(&msg("/dmx/level/depth", vec![Arg::Float(0.5)]));
    c.handle_osc(&msg("/dmx/level", vec![Arg::Float(0.0), Arg::Float(0.0)]));
    assert!((out(&mut c) - 127.5).abs() <= 1.0);
    // 先読み: 1秒後に1.0 → 0.5秒で中間
    c.handle_osc(&msg("/dmx/level/at", vec![Arg::Float(1.0), Arg::Float(1.0)]));
    let o = c.rt.osc_level.as_mut().unwrap();
    o.last -= Duration::from_millis(500);
    o.due = o.due.map(|d| d - Duration::from_millis(500));
    let v = out(&mut c);
    assert!((v - 191.0).abs() <= 3.0, "{v}");
    // やめると元の明るさ
    c.handle_osc(&msg("/dmx/level/off", vec![]));
    assert_eq!(out(&mut c), 255.0);
}
