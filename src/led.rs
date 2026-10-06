//! APC mini mk2 のパッド/ボタンのLED表示(コントローラーへのMIDI出力)。
//!
//! 通信仕様書(Communications Protocol v1.0):
//! - パッド(RGB): Note On、ノート番号=パッド(0x00..=0x3F)、ベロシティ=色(128色のパレット)、
//!   **MIDIチャンネル=明るさ/点滅**(ch0=10% ch1=25% ch2=50% ch3=65% ch4=75% ch5=90% ch6=100% ...)
//! - ボタン(単色): 常にch0のNote On、ベロシティ 0=消灯 / 1=点灯 / 2=点滅

use crate::apcmini::Ctl;
use crate::engine::{lock, Core, Shared};
use crate::project::*;
use midir::MidiOutputConnection;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub type Out = Arc<Mutex<MidiOutputConnection>>;

// パレットの色(ベロシティ)。仕様書の「Velocity to RGB Color Chart」より
pub const C_WHITE: u8 = 3;
pub const C_RED: u8 = 5;
pub const C_ORANGE: u8 = 9;
pub const C_YELLOW: u8 = 13;
pub const C_GREEN: u8 = 21;
pub const C_LIGHT_BLUE: u8 = 37;
pub const C_BLUE: u8 = 45;
pub const C_VIOLET: u8 = 49;
pub const C_MAGENTA: u8 = 53;
pub const C_PINK: u8 = 57;

/// 明るさ(MIDIチャンネル)
pub const LV_100: u8 = 6;
/// 待機中の明るさの選択肢: 10% / 25% / 50%
pub const IDLE_LEVELS: [u8; 3] = [0, 1, 2];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Led {
    Off,
    /// パッド: 色(パレット番号)と明るさ(MIDIチャンネル)
    Rgb { color: u8, level: u8 },
    /// ボタン: 1=点灯 / 2=点滅
    Single(u8),
    /// パッド: 実際の色(RGB各0〜255。明るさは値に含める)。full=効いている(最大の明るさ)
    True { rgb: [u8; 3], full: bool },
}

/// 待機中の明るさ(IDLE_LEVELS に対応)を、フルカラー表示用の倍率にしたもの
const IDLE_SCALE: [f32; 3] = [0.10, 0.25, 0.50];

impl Led {
    /// 送るMIDIメッセージ。パレットの色はNote On、実際の色はSysEx(RGB LED Color Lighting, 0x24)
    pub fn message(self, note: u8) -> Vec<u8> {
        match self {
            Led::Off => vec![0x90, note, 0],
            Led::Rgb { color, level } => vec![0x90 | (level & 0x0F), note, color],
            Led::Single(v) => vec![0x90, note, v],
            Led::True { rgb, .. } => {
                // 開始パッド・終了パッド + RGBを各8bit(上位1bit/下位7bit)= 8バイト
                let mut m = vec![0xF0, 0x47, 0x7F, 0x4F, 0x24, 0x00, 0x08, note, note];
                for c in rgb {
                    m.push(c >> 7);
                    m.push(c & 0x7F);
                }
                m.push(0xF7);
                m
            }
        }
    }

    /// 効いている(最大の明るさで光っている)か
    #[cfg(test)]
    pub fn is_full(self) -> bool {
        match self {
            Led::Rgb { level, .. } => level == LV_100,
            Led::True { full, .. } => full,
            Led::Single(v) => v == 2,
            Led::Off => false,
        }
    }
}

/// 割り当ての「実際の色」(色を指定するパッド、機材の選択、メイン/サブ入れ替え)。
/// `looks`=機材ごとの今の出力の色
pub fn action_rgb(core: &Core, action: &Action, looks: &HashMap<u32, [f32; 3]>) -> Option<[f32; 3]> {
    match action {
        Action::Color { color: Some(c), .. } => Some(c.rgb()),
        Action::SetValue { sel, attr, on, .. } => set_value_range(core, sel, attr, *on).and_then(|r| crate::color::range_rgb(&r)),
        Action::Select { fixture, .. } => looks.get(fixture).copied(),
        // 入れ替えのパッドは、今メインの機材に出ている色
        Action::Palette { .. } => core.role_rgb(crate::color::ColorRole::Main),
        _ => None,
    }
}

/// 機材id → 今の出力の色(LEDバーは1番目のピクセル)
pub fn look_colors(core: &Core) -> HashMap<u32, [f32; 3]> {
    crate::look::looks(core)
        .into_iter()
        .filter_map(|l| {
            let c = match l.look {
                crate::look::Look::One { rgb, .. } => rgb,
                crate::look::Look::Strip(px) => px.first().copied(),
            };
            c.map(|c| (l.id, c))
        })
        .collect()
}

/// レンジ名から、パッドに映す色を決める(赤・緑・青…)
pub fn label_color(label: &str) -> Option<u8> {
    const TABLE: [(&str, u8); 20] = [
        ("マゼンタ", C_MAGENTA),
        ("ライトブルー", C_LIGHT_BLUE),
        ("シアン", C_LIGHT_BLUE),
        ("オレンジ", C_ORANGE),
        ("ピンク", C_PINK),
        ("紫", C_VIOLET),
        ("赤", C_RED),
        ("緑", C_GREEN),
        ("青", C_BLUE),
        ("黄", C_YELLOW),
        ("白", C_WHITE),
        ("magenta", C_MAGENTA),
        ("cyan", C_LIGHT_BLUE),
        ("orange", C_ORANGE),
        ("purple", C_VIOLET),
        ("red", C_RED),
        ("green", C_GREEN),
        ("blue", C_BLUE),
        ("yellow", C_YELLOW),
        ("white", C_WHITE),
    ];
    let l = label.to_lowercase();
    TABLE.iter().find(|(k, _)| l.contains(k)).map(|(_, c)| *c)
}

/// 動作の種類ごとの色。チャンネル値で「色」を選ぶパッドは、選ぶ色そのものにする
fn base_color(core: &Core, action: &Action) -> u8 {
    match action {
        Action::SetValue { sel, attr, on, .. } => set_value_range(core, sel, attr, *on)
            .and_then(|r| r.color.map(|c| crate::color::nearest_named([c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0]).led()).or_else(|| label_color(&r.label)))
            .unwrap_or(C_BLUE),
        Action::Scene { .. } => C_GREEN,
        Action::Effect { .. } => C_VIOLET,
        Action::Blackout { .. } => C_RED,
        Action::Master { .. } => C_ORANGE,
        Action::Tempo { .. } => C_LIGHT_BLUE,
        Action::Chase { .. } => C_YELLOW,
        Action::Pair { .. } => C_PINK,
        Action::Color { color, .. } => color.map(|c| c.led()).unwrap_or(C_YELLOW),
        Action::ColorScope { .. } => C_ORANGE,
        Action::Select { .. } => C_LIGHT_BLUE,
        Action::Position { .. } => C_LIGHT_BLUE,
        Action::Page { .. } => C_WHITE,
        Action::Palette { .. } => C_MAGENTA,
        Action::Flash { .. } => C_WHITE,
        Action::Fade { .. } => C_ORANGE,
    }
}

/// 対象の最初の機材の、そのチャンネルのレンジ(onの値が入るもの)
fn set_value_range(core: &Core, sel: &Selector, attr: &str, on: f32) -> Option<crate::fixture::Range> {
    for i in core.resolve(sel) {
        let fx = &core.project.fixtures[i];
        let Some(m) = core.lib.mode(&fx.profile, fx.mode) else { continue };
        let Some(ch) = m.find_attr(attr) else { continue };
        return m.channels[ch].range_of(on.clamp(0.0, 255.0) as u8).cloned();
    }
    None
}

/// 対象の機材の現在値が、この割り当ての「ON値」になっているか(色選択・トグルの表示用)
fn set_value_active(core: &Core, sel: &Selector, attr: &str, on: f32) -> bool {
    let mut any = false;
    for i in core.resolve(sel) {
        let fx = &core.project.fixtures[i];
        let Some(m) = core.lib.mode(&fx.profile, fx.mode) else { continue };
        let Some(ch) = m.find_attr(attr) else { continue };
        if fx.values.len() != m.channels.len() {
            continue;
        }
        any = true;
        let v = m.get_f(&fx.values, ch);
        let def = &m.channels[ch];
        let hit = if def.ranges.is_empty() {
            (v - on).abs() < 1.0
        } else {
            // 選択式は、同じレンジに入っていればON(色の範囲のどこでも同じ色)
            match (def.range_of(v.floor() as u8), def.range_of(on.clamp(0.0, 255.0) as u8)) {
                (Some(a), Some(b)) => a.from == b.from && a.to == b.to,
                _ => false,
            }
        };
        if !hit {
            return false;
        }
    }
    any
}

/// この割り当てが「効いている」状態か
fn is_active(core: &Core, action: &Action) -> bool {
    match action {
        Action::Effect { name, .. } => core.project.effects.iter().any(|e| e.name == *name && e.enabled) || core.project.pixel_fx.iter().any(|e| e.name == *name && e.enabled),
        Action::Blackout { .. } => core.rt.blackout,
        Action::Chase { name, .. } => core.project.chases.iter().any(|c| c.name == *name && c.enabled),
        Action::Pair { name, op, .. } => core.project.pairs.iter().any(|p| p.name == *name && if *op == PairOp::Link { p.enabled } else { p.mirror }),
        Action::Scene { name } => core.rt.last_scene.as_deref() == Some(name.as_str()),
        Action::SetValue { sel, attr, mode, on, .. } => *mode != PadMode::Continuous && set_value_active(core, sel, attr, *on),
        // TAPは拍に合わせて光らせる(拍の頭のあいだ)
        Action::Tempo { op: TempoOp::Tap } => core.rt.beat.rem_euclid(1.0) < 0.25,
        // メイン/サブ色のパッドは、今のその系統の色なら点灯
        Action::Color { color: Some(c), target } => match target.slot() {
            Some(crate::color::ColorRole::Sub) => crate::color::same_rgb(core.project.palette.sub, c.rgb()),
            Some(_) => crate::color::same_rgb(core.project.palette.main, c.rgb()),
            None => core.rt.last_color == Some(*c),
        },
        Action::Palette { op: crate::color::PaletteOp::Auto } => core.project.palette.auto,
        Action::Palette { .. } => core.rt.color_swapped,
        Action::Flash { .. } => core.rt.flash.is_some(),
        Action::Fade { .. } => core.rt.dim_target < 0.5,
        Action::ColorScope { .. } => !core.rt.color_global,
        Action::Select { fixture, .. } => core.rt.selected.contains(fixture),
        Action::Position { name } => core.rt.last_position.as_deref() == Some(name.as_str()),
        Action::Page { op: PageOp::Go(p) } => core.rt.page == *p,
        Action::Page { .. } => false,
        Action::Color { color: None, .. } | Action::Tempo { .. } | Action::Master { .. } => false,
    }
}

fn control_led(core: &Core, ctl: Ctl, idle_level: u8, looks: &HashMap<u32, [f32; 3]>) -> Led {
    let (_, note) = ctl.trigger();
    let down = core.rt.notes_down[note as usize];
    let page = core.rt.page;
    let hits: Vec<&Binding> = core.project.bindings.iter().filter(|b| b.on_page(page) && b.trigger.is_some_and(|t| ctl.matches(&t))).collect();
    let Some(first) = hits.first() else { return Led::Off };
    let active = hits.iter().any(|b| is_active(core, &b.action));

    match ctl {
        Ctl::Pad(_) => {
            if down {
                return Led::Rgb { color: C_WHITE, level: LV_100 };
            }
            // 色に関係するパッドは、フルカラーで実際の色に光らせる(待機中は暗め)
            if core.project.led_true_color {
                if let Some(c) = action_rgb(core, &first.action, looks) {
                    let s = if active { 1.0 } else { IDLE_SCALE[(idle_level as usize).min(IDLE_SCALE.len() - 1)] };
                    let rgb = c.map(|x| (x.clamp(0.0, 1.0) * s * 255.0).round() as u8);
                    return Led::True { rgb, full: active };
                }
            }
            // 拍の頭は、1拍目だけオレンジにして小節の頭が分かるようにする
            let mut color = base_color(core, &first.action);
            if matches!(first.action, Action::Tempo { op: TempoOp::Tap }) && active && (core.rt.beat.floor() as i64).rem_euclid(BEATS_PER_BAR) == 0 {
                color = C_ORANGE;
            }
            Led::Rgb { color, level: if active { LV_100 } else { idle_level } }
        }
        // 単色LEDのボタン: 割り当てあり=点灯、効いている間=点滅
        _ => Led::Single(if active && !down { 2 } else { 1 }),
    }
}

/// 全パッド(64)とボタン(トラック8 + シーン8)の、今出すべき表示
pub fn compute(core: &Core) -> Vec<(u8, Led)> {
    let idle = IDLE_LEVELS[(core.project.led_idle as usize).min(IDLE_LEVELS.len() - 1)];
    // 機材の今の色は「機材選択」のパッドにだけ使う。無ければ調べない(毎回の計算を軽く)
    let looks = if core.project.led_true_color && core.project.bindings.iter().any(|b| matches!(b.action, Action::Select { .. })) { look_colors(core) } else { HashMap::new() };
    let mut out = Vec::with_capacity(80);
    for n in 0..64u8 {
        out.push((n, control_led(core, Ctl::Pad(n), idle, &looks)));
    }
    for i in 0..8u8 {
        out.push((Ctl::Track(i).trigger().1, control_led(core, Ctl::Track(i), idle, &looks)));
        out.push((Ctl::Scene(i).trigger().1, control_led(core, Ctl::Scene(i), idle, &looks)));
    }
    out
}

/// LED表示を実機へ送り続けるスレッド。変わったところだけ送る(3秒ごとに全部を送り直す)
pub struct LedSync {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl LedSync {
    pub fn spawn(core: Shared, out: Out) -> LedSync {
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = std::thread::Builder::new().name("apc-led".into()).spawn(move || run(core, out, s2)).ok();
        LedSync { stop, handle }
    }
}

impl Drop for LedSync {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn all_notes() -> Vec<u8> {
    let mut v: Vec<u8> = (0..64).collect();
    for i in 0..8u8 {
        v.push(Ctl::Track(i).trigger().1);
        v.push(Ctl::Scene(i).trigger().1);
    }
    v
}

fn run(core: Shared, out: Out, stop: Arc<AtomicBool>) {
    let mut sent: HashMap<u8, Vec<u8>> = HashMap::new();
    let mut last_full = Instant::now();
    let mut lit = false;
    let mut errors = 0u32;

    let send = |msg: Vec<u8>, errors: &mut u32| {
        if out.lock().unwrap_or_else(|e| e.into_inner()).send(&msg).is_err() {
            *errors += 1;
        }
    };

    while !stop.load(Ordering::SeqCst) {
        let (enabled, leds) = {
            let c = lock(&core);
            (c.project.led_enabled, if c.project.led_enabled { compute(&c) } else { Vec::new() })
        };
        if enabled {
            if last_full.elapsed() > Duration::from_secs(3) {
                sent.clear(); // 実機側の状態が変わっていても回復できるよう、定期的に全部送り直す
                last_full = Instant::now();
            }
            for (note, led) in leds {
                let msg = led.message(note);
                if sent.get(&note) != Some(&msg) {
                    send(msg.clone(), &mut errors);
                    sent.insert(note, msg);
                }
            }
            lit = true;
        } else if lit {
            for n in all_notes() {
                send(Led::Off.message(n), &mut errors);
            }
            sent.clear();
            lit = false;
        }
        lock(&core).rt.led_status = if !enabled { "消灯(設定でOFF)".into() } else if errors > 0 { format!("送信エラー {errors}回") } else { "点灯中".into() };
        std::thread::sleep(Duration::from_millis(40));
    }
    // 終了するとき(切断・アプリ終了)は、パッドを消す
    for n in all_notes() {
        send(Led::Off.message(n), &mut errors);
    }
}
