//! BPM・タップテンポ・拍同期エフェクト・チェイスのテスト

use crate::engine::*;
use crate::fixture::Library;
use crate::project::*;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn core() -> Core {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    Core::new(Project::starter(&lib), lib)
}

fn note(n: u8, v: u8) -> MidiMsg {
    MidiMsg { kind: TriggerKind::Note, channel: 0, number: n, value: v }
}

fn bind(c: &mut Core, num: u8, action: Action) {
    c.project.bindings.push(Binding { name: "b".into(), trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: num }), action, page: None, osc: None });
}

#[test]
fn beat_advances_with_bpm() {
    let mut c = core();
    assert_eq!(c.project.bpm, 120.0);
    c.advance_beat(1.0);
    assert!((c.rt.beat - 2.0).abs() < 1e-9, "120BPMなら1秒で2拍");
    c.set_bpm(60.0);
    c.advance_beat(1.0);
    assert!((c.rt.beat - 3.0).abs() < 1e-9, "テンポを変えても、それまでの位置は飛ばない");
}

#[test]
fn bpm_is_clamped() {
    let mut c = core();
    c.set_bpm(5.0);
    assert_eq!(c.project.bpm, BPM_MIN);
    c.set_bpm(999.0);
    assert_eq!(c.project.bpm, BPM_MAX);
}

#[test]
fn tap_tempo_averages_intervals_and_aligns_the_beat() {
    let mut c = core();
    let t0 = Instant::now();
    c.tap_at(t0);
    assert_eq!(c.project.bpm, 120.0, "1回目ではまだテンポは変わらない");
    c.tap_at(t0 + Duration::from_millis(500));
    assert!((c.project.bpm - 120.0).abs() < 0.01);
    c.tap_at(t0 + Duration::from_millis(1000));
    c.tap_at(t0 + Duration::from_millis(1500));
    assert!((c.project.bpm - 120.0).abs() < 0.01);

    // 別のテンポで数え直し(2秒以上空くとリセット)
    let t1 = t0 + Duration::from_secs(10);
    c.tap_at(t1);
    c.tap_at(t1 + Duration::from_millis(400));
    c.tap_at(t1 + Duration::from_millis(800));
    assert!((c.project.bpm - 150.0).abs() < 0.01, "0.4秒間隔 = 150BPM: {}", c.project.bpm);

    // 叩いた瞬間が拍の頭になる(最も近い拍に丸める)
    c.rt.beat = 7.4;
    c.tap_at(t1 + Duration::from_millis(1200));
    assert_eq!(c.rt.beat, 7.0);
    c.rt.beat = 7.6;
    c.tap_at(t1 + Duration::from_millis(1600));
    assert_eq!(c.rt.beat, 8.0);
}

#[test]
fn resync_returns_to_bar_start() {
    let mut c = core();
    c.rt.beat = 13.7;
    c.resync();
    assert_eq!(c.rt.beat, 0.0);
}

#[test]
fn effects_follow_the_beat_not_the_clock() {
    let mut c = core();
    let mut e = Effect::new("saw", "dimmer");
    e.enabled = true;
    e.sel = Selector::Fixture(1);
    e.wave = Wave::Saw;
    e.min = 0.0;
    e.max = 200.0;
    e.beats = 4.0; // 4拍で1周期
    c.project.effects.push(e);
    c.set_attr(&Selector::Fixture(1), "dimmer", 255.0);
    c.rt.master = 1.0;

    for (beat, expect) in [(0.0, 0.0), (1.0, 50.0), (2.0, 100.0), (3.0, 150.0), (4.0, 0.0), (5.0, 50.0)] {
        c.rt.beat = beat;
        c.rt.last_beat_t = None; // 時間経過ぶんの加算を止める
        c.render();
        let v = c.rt.frames[0][5] as f32;
        assert!((v - expect).abs() <= 1.5, "拍{beat}: 期待{expect} 実際{v}");
    }
    // テンポを倍にしても、拍の位置が同じなら同じ値(=拍に同期している)
    c.set_bpm(240.0);
    c.rt.beat = 2.0;
    c.rt.last_beat_t = None;
    c.render();
    assert!((c.rt.frames[0][5] as f32 - 100.0).abs() <= 1.5);
}

#[test]
fn step_order_patterns() {
    let seq = |o, n| (0..8).map(|i| step_index(o, i, n)).collect::<Vec<_>>();
    assert_eq!(seq(ChaseOrder::Forward, 3), vec![0, 1, 2, 0, 1, 2, 0, 1]);
    assert_eq!(seq(ChaseOrder::PingPong, 3), vec![0, 1, 2, 1, 0, 1, 2, 1]);
    assert_eq!(seq(ChaseOrder::PingPong, 2), vec![0, 1, 0, 1, 0, 1, 0, 1]);
    assert_eq!(step_index(ChaseOrder::Forward, 5, 1), 0);
    assert_eq!(step_index(ChaseOrder::PingPong, 5, 1), 0);
    // ランダム: 範囲内で、同じステップが続かない
    let r: Vec<usize> = (0..200).map(|i| step_index(ChaseOrder::Random, i, 4)).collect();
    assert!(r.iter().all(|&s| s < 4));
    assert!(r.windows(2).all(|w| w[0] != w[1]), "同じステップが2回続いた");
    assert!((0..4).all(|s| r.contains(&s)), "偏りすぎ");
    // 負の通し番号でも壊れない
    assert!(step_index(ChaseOrder::Forward, -1, 3) < 3);
}

fn chase_setup() -> Core {
    let mut c = core();
    // シーンA/B/C: スポット(id1)のディマー 50 / 100 / 150
    for (name, v) in [("A", 50.0), ("B", 100.0), ("C", 150.0)] {
        c.set_attr(&Selector::Fixture(1), "dimmer", v);
        let sc = c.capture_scene(name, 0.0, &[1], &["動き".to_string(), "色".to_string(), "形".to_string(), "制御".to_string()].into());
        c.project.scenes.push(sc);
    }
    c.set_attr(&Selector::Fixture(1), "dimmer", 0.0);
    c.project.chases.push(Chase { name: "ch".into(), enabled: true, steps: vec!["A".into(), "B".into(), "C".into()], step_beats: 1.0, fade_beats: 0.0, order: ChaseOrder::Forward });
    c
}

fn dimmer(c: &Core) -> u8 {
    c.project.fixtures[0].values[5]
}

fn at_beat(c: &mut Core, beat: f64) {
    c.rt.beat = beat;
    c.rt.last_beat_t = None;
    c.render();
}

#[test]
fn chase_steps_on_beats() {
    let mut c = chase_setup();
    at_beat(&mut c, 0.0);
    assert_eq!(dimmer(&c), 50, "有効にした直後、最初のステップが出る");
    at_beat(&mut c, 0.5);
    assert_eq!(dimmer(&c), 50, "拍の途中では変わらない");
    at_beat(&mut c, 1.0);
    assert_eq!(dimmer(&c), 100);
    at_beat(&mut c, 2.2);
    assert_eq!(dimmer(&c), 150);
    at_beat(&mut c, 3.0);
    assert_eq!(dimmer(&c), 50, "最後まで行ったら最初に戻る");
}

#[test]
fn chase_step_length_and_manual_edits() {
    let mut c = chase_setup();
    c.project.chases[0].step_beats = 2.0; // 2拍で1ステップ
    at_beat(&mut c, 0.0);
    at_beat(&mut c, 1.9);
    assert_eq!(dimmer(&c), 50);
    at_beat(&mut c, 2.0);
    assert_eq!(dimmer(&c), 100);
    // ステップの途中で手動で触った値は、次のステップまで維持される
    c.set_attr(&Selector::Fixture(1), "dimmer", 7.0);
    at_beat(&mut c, 3.5);
    assert_eq!(dimmer(&c), 7);
    at_beat(&mut c, 4.0);
    assert_eq!(dimmer(&c), 150);
}

#[test]
fn chase_stops_and_restarts_cleanly() {
    let mut c = chase_setup();
    at_beat(&mut c, 0.0);
    c.project.chases[0].enabled = false;
    at_beat(&mut c, 1.0);
    assert_eq!(dimmer(&c), 50, "止めると動かない");
    assert!(c.rt.chase_idx.is_empty());
    c.project.chases[0].enabled = true;
    at_beat(&mut c, 1.3);
    assert_eq!(dimmer(&c), 100, "再開したら、その拍の位置のステップがすぐ出る");
}

#[test]
fn chase_fade_length_comes_from_bpm() {
    let mut c = chase_setup();
    c.project.chases[0].fade_beats = 1.0; // 1拍かけて切り替え
    c.set_bpm(120.0); // 1拍 = 0.5秒
    at_beat(&mut c, 0.0);
    at_beat(&mut c, 1.0); // Aの50 → Bの100 へのフェード開始
    assert!(!c.rt.fades.is_empty());
    let f = &c.rt.fades[0];
    assert!((f.dur - 0.5).abs() < 1e-4, "フェード時間: {}", f.dur);
}

#[test]
fn missing_scene_step_is_skipped_safely() {
    let mut c = chase_setup();
    c.project.chases[0].steps[1] = "無いシーン".into();
    at_beat(&mut c, 0.0);
    at_beat(&mut c, 1.0);
    assert_eq!(dimmer(&c), 50, "見つからないシーンは何もしない");
    at_beat(&mut c, 2.0);
    assert_eq!(dimmer(&c), 150);
}

#[test]
fn chases_can_run_together_with_partial_scenes() {
    // 「動きだけ」のシーンと「光量だけ」のシーンを、別々のチェイスで同時に回す
    let mut c = core();
    c.set_attr(&Selector::Fixture(1), "pan", 30.0);
    c.set_attr(&Selector::Fixture(1), "dimmer", 200.0);
    let only = |keep: &str| -> std::collections::HashSet<String> { ["動き", "色", "光量", "形", "制御", "その他"].iter().filter(|c| **c != keep).map(|s| s.to_string()).collect() };
    let s1 = c.capture_scene("move1", 0.0, &[1], &only("動き"));
    let s2 = c.capture_scene("bright", 0.0, &[1], &only("光量"));
    c.set_attr(&Selector::Fixture(1), "pan", 200.0);
    c.set_attr(&Selector::Fixture(1), "dimmer", 20.0);
    let s3 = c.capture_scene("move2", 0.0, &[1], &only("動き"));
    let s4 = c.capture_scene("dark", 0.0, &[1], &only("光量"));
    c.project.scenes.extend([s1, s2, s3, s4]);
    c.project.chases.push(Chase { name: "move".into(), enabled: true, steps: vec!["move1".into(), "move2".into()], step_beats: 1.0, ..Chase::default() });
    c.project.chases.push(Chase { name: "light".into(), enabled: true, steps: vec!["bright".into(), "dark".into()], step_beats: 2.0, ..Chase::default() });

    let pan = |c: &Core| c.project.fixtures[0].values[0];
    at_beat(&mut c, 0.0);
    assert_eq!((pan(&c), dimmer(&c)), (30, 200));
    at_beat(&mut c, 1.0);
    assert_eq!((pan(&c), dimmer(&c)), (200, 200), "動きだけが変わる");
    at_beat(&mut c, 2.0);
    assert_eq!((pan(&c), dimmer(&c)), (30, 20), "2拍ごとの光量チェイスと、1拍ごとの動きチェイスが別々に回る");
}

#[test]
fn midi_tempo_actions() {
    let mut c = core();
    c.project.bindings.clear();
    bind(&mut c, 1, Action::Tempo { op: TempoOp::Tap });
    bind(&mut c, 2, Action::Tempo { op: TempoOp::Resync });
    bind(&mut c, 3, Action::Tempo { op: TempoOp::Nudge { delta: 5.0 } });
    bind(&mut c, 4, Action::Tempo { op: TempoOp::Nudge { delta: -10.0 } });
    c.project.bindings.push(Binding {
        name: "fader".into(),
        trigger: Some(MidiTrigger { kind: TriggerKind::Cc, channel: None, number: 9 }),
        action: Action::Tempo { op: TempoOp::Set { min: 60.0, max: 187.0 } },
        page: None,
        osc: None,
    });

    c.rt.beat = 9.6;
    c.handle_midi(note(1, 100));
    c.handle_midi(note(1, 0));
    assert_eq!(c.rt.beat, 10.0, "TAPで拍が合う");
    assert_eq!(c.rt.taps.len(), 1);

    c.handle_midi(note(2, 100));
    assert_eq!(c.rt.beat, 0.0);

    c.handle_midi(note(3, 100));
    c.handle_midi(note(3, 0));
    assert_eq!(c.project.bpm, 125.0);
    c.handle_midi(note(4, 100));
    c.handle_midi(note(4, 0));
    assert_eq!(c.project.bpm, 115.0);

    c.handle_midi(MidiMsg { kind: TriggerKind::Cc, channel: 0, number: 9, value: 0 });
    assert_eq!(c.project.bpm, 60.0);
    c.handle_midi(MidiMsg { kind: TriggerKind::Cc, channel: 0, number: 9, value: 127 });
    assert_eq!(c.project.bpm, 187.0);
}

#[test]
fn midi_chase_toggle() {
    let mut c = chase_setup();
    c.project.chases[0].enabled = false;
    c.project.bindings.clear();
    bind(&mut c, 7, Action::Chase { name: "ch".into(), mode: PadMode::Toggle });
    c.handle_midi(note(7, 100));
    c.handle_midi(note(7, 0));
    assert!(c.project.chases[0].enabled);
    c.handle_midi(note(7, 100));
    c.handle_midi(note(7, 0));
    assert!(!c.project.chases[0].enabled);
}

#[test]
fn bpm_and_chases_persist_and_old_files_load() {
    let mut p = Project { bpm: 133.5, ..Project::default() };
    p.chases.push(Chase { name: "x".into(), steps: vec!["A".into()], order: ChaseOrder::PingPong, ..Chase::default() });
    let q: Project = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
    assert_eq!(q.bpm, 133.5);
    assert_eq!(q.chases[0].order, ChaseOrder::PingPong);

    let old = r#"{"fixtures":[],"effects":[{"name":"e","enabled":true,"sel":"All","attr":"pan","wave":"Sine","rate_hz":0.5,"min":0.0,"max":255.0,"phase":0.0,"spread":0.0}]}"#;
    let o: Project = serde_json::from_str(old).unwrap();
    assert_eq!(o.bpm, 120.0);
    assert_eq!(o.effects[0].beats, 0.0, "古い保存ファイルのエフェクトは自由(Hz)のまま");
    assert!(o.chases.is_empty());
}

#[test]
fn chase_steps_do_not_mark_the_project_dirty() {
    let mut c = chase_setup();
    c.dirty = false;
    at_beat(&mut c, 0.0);
    at_beat(&mut c, 1.0);
    at_beat(&mut c, 2.0);
    assert!(!c.dirty, "動作中のチェイスのために、毎拍自動保存されてはいけない");
}

#[test]
fn audio_source_follows_detected_tempo_and_phase() {
    use crate::audio::Estimate;
    let mut c = core();
    let now = Instant::now();
    // 音声以外のテンポの元では無視される
    c.audio_report(Estimate { bpm: 140.0, phase: 0.0, confidence: 0.9 }, now);
    assert_eq!(c.project.bpm, 120.0);
    c.set_tempo_source(TempoSource::Audio);
    // 確からしさが低いうちは変えない
    c.audio_report(Estimate { bpm: 140.0, phase: 0.0, confidence: 0.1 }, now);
    assert_eq!(c.project.bpm, 120.0);
    // 最初は一気に合い、拍の端数も揃う
    c.rt.beat = 3.3;
    c.audio_report(Estimate { bpm: 140.0, phase: 0.1, confidence: 0.8 }, now);
    assert!((c.project.bpm - 140.0).abs() < 0.11);
    assert!((c.rt.beat.rem_euclid(1.0) - 0.1).abs() < 1e-6);
    // 小さな揺れは無視、大きな食い違いは1回では従わない
    c.audio_report(Estimate { bpm: 140.2, phase: 0.1, confidence: 0.8 }, now);
    assert!((c.project.bpm - 140.0).abs() < 0.11);
    c.audio_report(Estimate { bpm: 70.0, phase: 0.1, confidence: 0.8 }, now);
    assert!((c.project.bpm - 140.0).abs() < 0.11);
}

#[test]
fn beat_now_fills_in_between_output_ticks() {
    // 画面用の拍は、出力が拍を進めた(25msごと)あとの経過ぶんを足して、なめらかに進む
    let mut c = core();
    c.rt.beat = 3.0;
    c.rt.last_beat_t = Some(Instant::now() - Duration::from_millis(20));
    let b = c.beat_now();
    assert!(b > 3.03 && b < 3.1, "120BPMで20ms経てば約0.04拍進む: {b}");
    // 出力が止まっていても、先へ進みすぎない(最大0.1秒ぶん)
    c.rt.last_beat_t = Some(Instant::now() - Duration::from_secs(5));
    assert!((c.beat_now() - 3.2).abs() < 1e-6);
    // まだ一度も進めていなければ、そのまま
    c.rt.last_beat_t = None;
    assert_eq!(c.beat_now(), 3.0);
}
