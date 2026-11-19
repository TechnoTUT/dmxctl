//! 性能の目安(普段は実行しない)。実行: cargo test --release perf -- --ignored --nocapture

use crate::engine::*;
use crate::fixture::Library;
use crate::project::*;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

/// 機材16台(スポット8 + レーザー8)、割り当て64、エフェクト4、チェイス2、ペア3、全体光量あり
fn heavy_core() -> Core {
    let (lib, _) = Library::load(Path::new("none"));
    let lib = Arc::new(lib);
    let mut p = Project::default();
    for i in 0..16 {
        let id = p.add_fixture(&lib, if i % 2 == 0 { "stage-evolution/minispot30" } else { "stage-evolution/laserscan300" }).unwrap();
        // 512スロットを超えないように2つ目のユニバースへ
        if i >= 8 {
            let f = p.fixtures.iter_mut().find(|f| f.id == id).unwrap();
            f.universe = 1;
        }
    }
    // 2つ目のユニバースは先頭から詰め直す
    let mut addr = 1u16;
    for f in p.fixtures.iter_mut().filter(|f| f.universe == 1) {
        f.address = addr;
        addr += lib.mode(&f.profile, f.mode).unwrap().channels.len() as u16;
    }
    // LEDバー 32ピクセル×2本(2つ目のユニバースの後ろ)
    let bar_mode = lib.get("generic/led-bar-rgb").unwrap().modes.iter().position(|m| m.pixels().len() == 32).unwrap();
    for _ in 0..2 {
        let id = p.add_fixture(&lib, "generic/led-bar-rgb").unwrap();
        let f = p.fixtures.iter_mut().find(|f| f.id == id).unwrap();
        f.universe = 1;
        f.mode = bar_mode;
        f.address = addr;
        f.values = lib.mode("generic/led-bar-rgb", bar_mode).unwrap().default_values();
        addr += 96;
    }
    for kind in [crate::pixel::PixelKind::Rainbow, crate::pixel::PixelKind::Chase] {
        p.pixel_fx.push(crate::pixel::PixelFx { enabled: true, kind, ..Default::default() });
    }
    p.master_seeded = true;
    for n in 0..64u8 {
        p.bindings.push(Binding { id: 0,
            name: "b".into(),
            trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: None, number: n }),
            action: match n % 6 {
                0 => Action::SetValue { sel: Selector::Profile("stage-evolution/minispot30".into()), attr: "color".into(), mode: PadMode::Trigger, on: (n % 14) as f32 * 10.0 + 5.0, off: 0.0 },
                1 => Action::Effect { name: "e".into(), mode: PadMode::Toggle },
                2 => Action::Color { color: Some(crate::color::NamedColor::ALL[(n % 10) as usize]), target: crate::color::ColorTarget::All },
                3 => Action::Chase { name: "c1".into(), mode: PadMode::Toggle },
                4 => Action::Tempo { op: TempoOp::Tap },
                _ => Action::SetValue { sel: Selector::All, attr: "dimmer".into(), mode: PadMode::Toggle, on: 255.0, off: 0.0 },
            },
            page: None,
            osc: None,
        });
    }
    for (i, attr) in ["pan", "tilt", "dimmer", "pan"].iter().enumerate() {
        let mut e = Effect::new("e", attr);
        e.enabled = true;
        e.rate_hz = 0.3 + i as f32 * 0.1;
        e.beats = if i % 2 == 0 { 4.0 } else { 0.0 };
        p.effects.push(e);
    }
    for n in ["A", "B", "C", "D"] {
        p.scenes.push(Scene { name: n.into(), fade: 0.0, values: vec![] });
    }
    for (i, name) in ["c1", "c2"].iter().enumerate() {
        p.chases.push(Chase { name: (*name).into(), enabled: true, steps: vec!["A".into(), "B".into(), "C".into(), "D".into()], step_beats: 1.0 + i as f32, ..Chase::default() });
    }
    for i in 0..3u32 {
        p.pairs.push(Pair { name: format!("p{i}"), leader: 1 + i * 2, follower: 3 + i * 2, ..Pair::default() });
    }
    let mut c = Core::new(p, lib);
    c.rt.master = 0.8;
    c
}

fn per_call_us(iters: u32, mut f: impl FnMut()) -> f64 {
    for _ in 0..50 {
        f(); // ウォームアップ
    }
    let t = Instant::now();
    for _ in 0..iters {
        f();
    }
    t.elapsed().as_secs_f64() * 1e6 / iters as f64
}

#[test]
#[ignore]
fn perf_render_led_and_midi() {
    let mut c = heavy_core();
    let render = per_call_us(3000, || c.render());
    let led = per_call_us(3000, || {
        std::hint::black_box(crate::led::compute(&c));
    });
    let mut n = 0u8;
    let midi = per_call_us(20000, || {
        n = n.wrapping_add(1);
        c.handle_midi(MidiMsg { kind: TriggerKind::Cc, channel: 0, number: 0x30 + (n % 9), value: n % 128 });
    });
    let view = per_call_us(3000, || {
        for id in 1..=16 {
            std::hint::black_box(c.paired_view(id));
        }
    });
    println!(
        "\n[性能] 機材{}台 / 割り当て{}件 / エフェクト{}件 / チェイス{}件 / ペア{}件\n  render(1フレーム合成)   : {:8.1} µs  (40Hz=25000µs の {:.2}%)\n  LED表示の計算(80個)      : {:8.1} µs\n  MIDI 1メッセージの処理    : {:8.2} µs\n  ペア表示の計算(16台)     : {:8.1} µs",
        c.project.fixtures.len(),
        c.project.bindings.len(),
        c.project.effects.len(),
        c.project.chases.len(),
        c.project.pairs.len(),
        render,
        render / 25000.0 * 100.0,
        led,
        midi,
        view
    );
    // 目安: 1フレームの合成は周期の5%未満、MIDIは1msに1000件さばける
    assert!(render < 1250.0, "renderが遅い: {render}µs");
    assert!(midi < 100.0, "MIDI処理が遅い: {midi}µs");
}
