//! 演出の状態と、そこから4台ぶんの出力を毎フレーム計算するところ。

use crate::fixture::{self, Head, COLORS, GOBOS};
use crate::tempo::Clock;
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

pub const N: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Motion {
    Still,
    Circle,
    Eight,
    PanSweep,
    TiltSweep,
    Cross,
    Random,
    Wave,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum Spread {
    /// 全台同じ動き
    Sync,
    /// 1/4周ずつずらす
    Fan,
    /// 左右対称(1・2台目と3・4台目でPanが逆向き)
    Mirror,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum ColorMode {
    Fixed,
    /// 1拍ごとに次の色
    Beat,
    /// 1小節ごとに次の色
    Bar,
    /// 台ごとに違う色(固定)
    Split,
    /// カラーホイール連続回転
    Rainbow,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum GoboMode {
    Fixed,
    Beat,
    Bar,
    Shake,
    Rotate,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum DimFx {
    Steady,
    /// 拍ごとに点いて減衰
    Pulse,
    /// 1拍ごとに1台ずつ
    Chase,
    /// 1・3台目と2・4台目を拍ごとに交互
    Alternate,
    /// 拍ごとにランダムな台
    Random,
    /// なめらかな波(1小節で一周)
    Wave,
    /// 半拍ごとに1台ずつ
    FastChase,
    /// 1小節かけて明るくなる(盛り上げ)
    Build,
}

/// 動きの1周にかける拍数
pub const RATES: [(f64, &str); 5] = [(16.0, "4小節"), (8.0, "2小節"), (4.0, "1小節"), (2.0, "2拍"), (1.0, "1拍")];
pub const POSITION_NAMES: [&str; 3] = ["客席", "天井", "床"];

/// 照明の「見た目」一式。シーンにはこれを保存する
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
pub struct Look {
    pub motion: Motion,
    pub rate: u8,
    pub spread: Spread,
    pub color: u8,
    pub color_mode: ColorMode,
    pub gobo: u8,
    pub gobo_mode: GoboMode,
    pub position: u8,
    pub dim: DimFx,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            motion: Motion::Circle,
            rate: 2,
            spread: Spread::Fan,
            color: 0,
            color_mode: ColorMode::Fixed,
            gobo: 0,
            gobo_mode: GoboMode::Fixed,
            position: 0,
            dim: DimFx::Steady,
        }
    }
}

/// フェーダー(APC mini mk2 の9本と同じ並び)。すべて 0..1
pub const FADER_NAMES: [&str; 9] = ["動きの大きさ", "Pan位置", "Tilt位置", "モーター速度", "効果の深さ", "ストロボ速度", "回転速度", "減衰", "マスター"];
pub const F_SIZE: usize = 0;
pub const F_PAN: usize = 1;
pub const F_TILT: usize = 2;
pub const F_MOTOR: usize = 3;
pub const F_DEPTH: usize = 4;
pub const F_STROBE: usize = 5;
pub const F_WHEEL: usize = 6;
pub const F_DECAY: usize = 7;
pub const F_MASTER: usize = 8;
pub const FADER_DEFAULTS: [f32; 9] = [0.5, 0.5, 0.5, 0.0, 1.0, 0.7, 0.3, 0.5, 1.0];

/// 操作(APCのボタン・画面のボタン共通)
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Action {
    Motion(Motion),
    Rate(u8),
    Spread(Spread),
    Color(u8),
    ColorMode(ColorMode),
    Position(u8),
    Gobo(u8),
    GoboMode(GoboMode),
    Freeze,
    Auto,
    Dim(DimFx),
    Flash,
    Strobe,
    StrobeFast,
    Blackout,
    Tap,
    Downbeat,
    Half,
    Double,
    Fixture(u8),
    Scene(u8),
    Shift,
}

impl Action {
    /// 押している間だけ効くもの
    pub fn momentary(self) -> bool {
        matches!(self, Action::Flash | Action::Strobe | Action::StrobeFast | Action::Shift)
    }
}

/// 保存する設定(機材の取り付け・パッチ・シーン)
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Setup {
    pub mode: fixture::Mode,
    pub addresses: [u16; N],
    pub invert_pan: [bool; N],
    pub invert_tilt: [bool; N],
    /// 客席/天井/床 の基準位置 (pan, tilt)
    pub positions: [[f32; 2]; 3],
    pub scenes: [Option<Look>; 8],
}

impl Default for Setup {
    fn default() -> Self {
        Setup {
            mode: fixture::Mode::Ch9,
            addresses: [1, 10, 19, 28],
            invert_pan: [false; N],
            invert_tilt: [false; N],
            positions: [[0.5, 0.25], [0.5, 0.5], [0.5, 0.08]],
            scenes: [None; 8],
        }
    }
}

pub struct State {
    pub look: Look,
    pub faders: [f32; 9],
    pub enabled: [bool; N],
    pub auto: bool,
    pub freeze: bool,
    pub flash: bool,
    pub strobe: bool,
    pub strobe_fast: bool,
    pub blackout: bool,
    pub shift: bool,
    pub active_scene: Option<u8>,
    pub clock: Clock,
    pub setup: Setup,
    /// 直近に計算した出力
    pub heads: [Head; N],
    pub beat: f64,
    phase: f64,
    last_t: Option<f64>,
    random_step: i64,
    random_targets: [[f32; 2]; N],
    last_auto_bar: i64,
    rng: u64,
}

impl State {
    pub fn new(setup: Setup, bpm: f64) -> Self {
        State {
            look: Look::default(),
            faders: FADER_DEFAULTS,
            enabled: [true; N],
            auto: false,
            freeze: false,
            flash: false,
            strobe: false,
            strobe_fast: false,
            blackout: false,
            shift: false,
            active_scene: None,
            clock: Clock::new(bpm),
            setup,
            heads: [Head::default(); N],
            beat: 0.0,
            phase: 0.0,
            last_t: None,
            random_step: i64::MIN,
            random_targets: [[0.0; 2]; N],
            last_auto_bar: i64::MIN,
            rng: 0x2545_F491_4F6C_DD1D,
        }
    }

    pub fn press(&mut self, a: Action, down: bool, t: f64) {
        match a {
            Action::Flash => self.flash = down,
            Action::Strobe => self.strobe = down,
            Action::StrobeFast => self.strobe_fast = down,
            Action::Shift => self.shift = down,
            _ if !down => {}
            Action::Scene(i) => {
                let i = i as usize % 8;
                if self.shift {
                    self.setup.scenes[i] = Some(self.look);
                    self.active_scene = Some(i as u8);
                } else if let Some(l) = self.setup.scenes[i] {
                    self.look = l;
                    self.active_scene = Some(i as u8);
                }
            }
            Action::Freeze => self.freeze = !self.freeze,
            Action::Auto => self.auto = !self.auto,
            Action::Blackout => self.blackout = !self.blackout,
            Action::Tap => self.clock.tap(t),
            Action::Downbeat => self.clock.downbeat(t),
            Action::Half => self.clock.set_bpm(self.clock.bpm() / 2.0, t),
            Action::Double => self.clock.set_bpm(self.clock.bpm() * 2.0, t),
            Action::Fixture(i) => {
                let i = i as usize % N;
                self.enabled[i] = !self.enabled[i];
            }
            _ => {
                self.edit_look(a);
                self.active_scene = None;
            }
        }
    }

    fn edit_look(&mut self, a: Action) {
        let l = &mut self.look;
        match a {
            Action::Motion(m) => l.motion = m,
            Action::Rate(r) => l.rate = r.min(RATES.len() as u8 - 1),
            Action::Spread(s) => l.spread = s,
            Action::Color(c) => l.color = c % 8,
            Action::ColorMode(m) => l.color_mode = m,
            Action::Position(p) => l.position = p % 3,
            Action::Gobo(g) => l.gobo = g % 8,
            Action::GoboMode(m) => l.gobo_mode = m,
            Action::Dim(d) => l.dim = d,
            _ => {}
        }
    }

    /// ボタンが「選ばれている/効いている」か(LEDと画面の表示用)
    pub fn is_active(&self, a: Action) -> bool {
        let l = &self.look;
        match a {
            Action::Motion(m) => l.motion == m,
            Action::Rate(r) => l.rate == r,
            Action::Spread(s) => l.spread == s,
            Action::Color(c) => l.color == c,
            Action::ColorMode(m) => l.color_mode == m,
            Action::Position(p) => l.position == p,
            Action::Gobo(g) => l.gobo == g,
            Action::GoboMode(m) => l.gobo_mode == m,
            Action::Dim(d) => l.dim == d,
            Action::Freeze => self.freeze,
            Action::Auto => self.auto,
            Action::Flash => self.flash,
            Action::Strobe => self.strobe,
            Action::StrobeFast => self.strobe_fast,
            Action::Blackout => self.blackout,
            Action::Tap => self.beat.rem_euclid(1.0) < 0.15,
            Action::Downbeat => self.beat.rem_euclid(4.0) < 0.15,
            Action::Half | Action::Double => false,
            Action::Fixture(i) => self.enabled[i as usize % N],
            Action::Scene(i) => self.active_scene == Some(i),
            Action::Shift => self.shift,
        }
    }

    fn next_rand(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    fn rand01(&mut self) -> f32 {
        (self.next_rand() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// 自動モード: 16小節ごとに見た目を入れ替える
    fn auto_change(&mut self) {
        const MOTIONS: [Motion; 7] = [Motion::Circle, Motion::Eight, Motion::PanSweep, Motion::TiltSweep, Motion::Cross, Motion::Random, Motion::Wave];
        const SPREADS: [Spread; 3] = [Spread::Sync, Spread::Fan, Spread::Mirror];
        const CMODES: [ColorMode; 4] = [ColorMode::Fixed, ColorMode::Beat, ColorMode::Bar, ColorMode::Split];
        const GMODES: [GoboMode; 3] = [GoboMode::Fixed, GoboMode::Bar, GoboMode::Rotate];
        const DIMS: [DimFx; 6] = [DimFx::Steady, DimFx::Pulse, DimFx::Chase, DimFx::Alternate, DimFx::Wave, DimFx::FastChase];
        let pick = |s: &mut Self, n: usize| (s.next_rand() % n as u64) as usize;
        let l = Look {
            motion: MOTIONS[pick(self, MOTIONS.len())],
            rate: 1 + pick(self, 3) as u8,
            spread: SPREADS[pick(self, SPREADS.len())],
            color: pick(self, 8) as u8,
            color_mode: CMODES[pick(self, CMODES.len())],
            gobo: pick(self, 8) as u8,
            gobo_mode: GMODES[pick(self, GMODES.len())],
            position: self.look.position,
            dim: DIMS[pick(self, DIMS.len())],
        };
        self.look = l;
        self.active_scene = None;
    }

    /// 時刻 t(起動からの秒)の出力を計算して `heads` に入れる
    pub fn tick(&mut self, t: f64) {
        let beat = self.clock.beat(t);
        let dt_beats = match self.last_t {
            Some(lt) => ((t - lt) * self.clock.bpm() / 60.0).clamp(0.0, 1.0),
            None => 0.0,
        };
        self.last_t = Some(t);
        self.beat = beat;

        let bar = (beat / 4.0).floor() as i64;
        if self.auto && bar != self.last_auto_bar && bar.rem_euclid(16) == 0 {
            if self.last_auto_bar != i64::MIN {
                self.auto_change();
            }
            self.last_auto_bar = bar;
        } else if !self.auto {
            self.last_auto_bar = i64::MIN;
        }

        let rate = RATES[self.look.rate as usize % RATES.len()].0;
        if !self.freeze {
            self.phase = (self.phase + dt_beats / rate).rem_euclid(1024.0);
        }
        // ランダムな動き: 1周の1/4ごとに次の目標
        if self.look.motion == Motion::Random && !self.freeze {
            let step = (self.phase * 4.0).floor() as i64;
            if step != self.random_step {
                self.random_step = step;
                for i in 0..N {
                    self.random_targets[i] = [self.rand01() * 2.0 - 1.0, self.rand01() * 2.0 - 1.0];
                }
            }
        }

        for i in 0..N {
            self.heads[i] = self.head(i, beat);
        }
    }

    fn head(&self, i: usize, beat: f64) -> Head {
        let l = &self.look;
        let f = &self.faders;
        let s = &self.setup;

        // ---- 動き
        let fan = if l.spread == Spread::Fan { i as f64 / N as f64 } else { 0.0 };
        let p = self.phase + fan;
        let (sin, cos) = ((p * TAU).sin(), (p * TAU).cos());
        let (mut x, y) = match l.motion {
            Motion::Still => (0.0, 0.0),
            Motion::Circle => (cos, sin),
            Motion::Eight => (sin, (2.0 * p * TAU).sin() * 0.6),
            Motion::PanSweep => (sin, 0.0),
            Motion::TiltSweep => (0.0, sin),
            Motion::Cross => (sin, if i % 2 == 0 { sin } else { -sin }),
            Motion::Random => (self.random_targets[i][0] as f64, self.random_targets[i][1] as f64),
            Motion::Wave => ((i as f64 - 1.5) / 1.5, ((self.phase + i as f64 / N as f64) * TAU).sin()),
        };
        if l.spread == Spread::Mirror && i >= N / 2 {
            x = -x;
        }
        let base = s.positions[l.position as usize % 3];
        let size = f[F_SIZE] as f64;
        let mut pan = base[0] as f64 + (f[F_PAN] as f64 - 0.5) * 0.5 + x * size * 0.25;
        let mut tilt = base[1] as f64 + (f[F_TILT] as f64 - 0.5) * 0.5 + y * size * 0.25;
        if s.invert_pan[i] {
            pan = 1.0 - pan;
        }
        if s.invert_tilt[i] {
            tilt = 1.0 - tilt;
        }

        // ---- 色: 「次の色」は拍数ぶん進める。Fan/Mirrorのときは台ごとにずらす
        let offset = if l.spread == Spread::Sync { 0 } else { i as i64 };
        let wheel = f[F_WHEEL];
        let color = match l.color_mode {
            ColorMode::Fixed => COLORS[l.color as usize % 8].1,
            ColorMode::Beat => COLORS[(l.color as i64 + beat.floor() as i64 + offset).rem_euclid(8) as usize].1,
            ColorMode::Bar => COLORS[(l.color as i64 + (beat / 4.0).floor() as i64 + offset).rem_euclid(8) as usize].1,
            ColorMode::Split => COLORS[(l.color as usize + i * 2) % 8].1,
            ColorMode::Rainbow => fixture::rainbow(wheel),
        };

        // ---- ゴボ: 送るときはゴボ1〜7を回す(オープンは飛ばす)
        let step_gobo = |steps: i64| {
            let start = l.gobo.max(1) as i64 - 1;
            GOBOS[1 + (start + steps + offset).rem_euclid(7) as usize]
        };
        let gobo = match l.gobo_mode {
            GoboMode::Fixed => GOBOS[l.gobo as usize % 8],
            GoboMode::Beat => step_gobo(beat.floor() as i64),
            GoboMode::Bar => step_gobo((beat / 4.0).floor() as i64),
            GoboMode::Shake => fixture::gobo_shake(l.gobo.max(1), wheel),
            GoboMode::Rotate => fixture::gobo_rotate(wheel),
        };

        // ---- 明るさ
        let decay = 2.0 + (1.0 - f[F_DECAY] as f64) * 10.0;
        let env = |frac: f64| (-frac * decay).exp();
        let frac = beat.rem_euclid(1.0);
        let b = beat.floor() as i64;
        let fx = match l.dim {
            DimFx::Steady => 1.0,
            DimFx::Pulse => env(frac),
            DimFx::Chase => {
                if b.rem_euclid(N as i64) as usize == i { env(frac) } else { 0.0 }
            }
            DimFx::Alternate => {
                if b.rem_euclid(2) as usize == i % 2 { 1.0 } else { 0.0 }
            }
            DimFx::Random => {
                if hash(b, i) % 2 == 0 { env(frac) } else { 0.0 }
            }
            DimFx::Wave => 0.5 + 0.5 * ((beat / 4.0 - i as f64 / N as f64) * TAU).sin(),
            DimFx::FastChase => {
                let h = beat * 2.0;
                if (h.floor() as i64).rem_euclid(N as i64) as usize == i { env(h.rem_euclid(1.0)) } else { 0.0 }
            }
            DimFx::Build => (beat / 4.0).rem_euclid(1.0),
        };
        let depth = f[F_DEPTH] as f64;
        let mut dimmer = (1.0 - depth * (1.0 - fx)) * f[F_MASTER] as f64;
        if !self.enabled[i] || self.blackout {
            dimmer = 0.0;
        }

        // ---- フラッシュ/ストロボは、マスター・消灯・ブラックアウトより優先(曲のキメ用)
        let mut strobe = 0;
        if self.strobe_fast {
            strobe = fixture::strobe(1.0);
            dimmer = 1.0;
        } else if self.strobe {
            strobe = fixture::strobe(f[F_STROBE]);
            dimmer = 1.0;
        } else if self.flash {
            dimmer = 1.0;
        }

        Head {
            pan: pan as f32,
            tilt: tilt as f32,
            color,
            gobo,
            strobe,
            dimmer: dimmer as f32,
            pt_speed: (f[F_MOTOR].clamp(0.0, 1.0) * 255.0).round() as u8,
        }
    }
}

/// 拍と台番号から決まる疑似乱数(同じ拍の間は変わらない)
fn hash(b: i64, i: usize) -> u64 {
    let mut x = (b as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (i as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^ (x >> 29)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        State::new(Setup::default(), 120.0)
    }

    /// 120BPMなので 1拍 = 0.5秒
    fn run(s: &mut State, from: f64, to: f64) {
        let mut t = from;
        while t <= to {
            s.tick(t);
            t += 0.025;
        }
    }

    #[test]
    fn steady_output_is_safe_and_in_range() {
        let mut s = state();
        run(&mut s, 0.0, 10.0);
        for h in s.heads {
            assert!((0.0..=1.0).contains(&h.pan) && (0.0..=1.0).contains(&h.tilt));
            assert_eq!(h.dimmer, 1.0);
            assert_eq!(h.strobe, 0);
            assert_eq!(h.color, COLORS[0].1);
        }
    }

    #[test]
    fn fan_spreads_and_sync_aligns() {
        let mut s = state();
        run(&mut s, 0.0, 1.3);
        assert_ne!(s.heads[0].pan, s.heads[1].pan);
        s.press(Action::Spread(Spread::Sync), true, 1.3);
        s.tick(1.325);
        assert!(s.heads.iter().all(|h| h.pan == s.heads[0].pan && h.tilt == s.heads[0].tilt));
    }

    #[test]
    fn mirror_flips_pan_of_right_pair() {
        let mut s = state();
        s.press(Action::Spread(Spread::Mirror), true, 0.0);
        s.press(Action::Motion(Motion::PanSweep), true, 0.0);
        run(&mut s, 0.0, 0.7);
        let c = 0.5;
        assert!((s.heads[0].pan - c) * (s.heads[3].pan - c) < 0.0);
        assert!(((s.heads[0].pan - c) + (s.heads[3].pan - c)).abs() < 1e-5);
    }

    #[test]
    fn chase_lights_one_fixture_per_beat() {
        let mut s = state();
        s.press(Action::Dim(DimFx::Chase), true, 0.0);
        s.tick(0.0);
        s.tick(0.5 * 2.0 + 0.01); // 拍2の頭
        let lit: Vec<usize> = (0..N).filter(|&i| s.heads[i].dimmer > 0.5).collect();
        assert_eq!(lit, vec![2]);
    }

    #[test]
    fn master_blackout_and_flash() {
        let mut s = state();
        s.faders[F_MASTER] = 0.5;
        s.tick(0.0);
        assert_eq!(s.heads[0].dimmer, 0.5);
        s.press(Action::Blackout, true, 0.0);
        s.tick(0.1);
        assert!(s.heads.iter().all(|h| h.dimmer == 0.0));
        s.press(Action::Flash, true, 0.1);
        s.tick(0.2);
        assert!(s.heads.iter().all(|h| h.dimmer == 1.0));
        s.press(Action::Flash, false, 0.2);
        s.press(Action::Strobe, true, 0.2);
        s.tick(0.3);
        assert!(s.heads.iter().all(|h| h.strobe >= 10 && h.dimmer == 1.0));
        s.press(Action::Strobe, false, 0.3);
        s.tick(0.4);
        assert!(s.heads.iter().all(|h| h.strobe == 0 && h.dimmer == 0.0));
    }

    #[test]
    fn fixture_toggle_mutes_only_that_one() {
        let mut s = state();
        s.press(Action::Fixture(1), true, 0.0);
        s.tick(0.0);
        assert_eq!(s.heads[1].dimmer, 0.0);
        assert_eq!(s.heads[0].dimmer, 1.0);
    }

    #[test]
    fn color_chase_steps_on_beat() {
        let mut s = state();
        s.press(Action::Spread(Spread::Sync), true, 0.0);
        s.press(Action::ColorMode(ColorMode::Beat), true, 0.0);
        s.tick(0.1);
        let c0 = s.heads[0].color;
        s.tick(0.6);
        assert_ne!(c0, s.heads[0].color);
    }

    #[test]
    fn gobo_chase_skips_open() {
        let mut s = state();
        s.press(Action::GoboMode(GoboMode::Beat), true, 0.0);
        for k in 0..40 {
            s.tick(k as f64 * 0.25);
            assert!(s.heads.iter().all(|h| h.gobo != GOBOS[0]));
        }
    }

    #[test]
    fn scenes_store_with_shift_and_recall() {
        let mut s = state();
        s.press(Action::Motion(Motion::Eight), true, 0.0);
        s.press(Action::Shift, true, 0.0);
        s.press(Action::Scene(3), true, 0.0);
        s.press(Action::Shift, false, 0.0);
        s.press(Action::Motion(Motion::Still), true, 0.0);
        assert_eq!(s.active_scene, None);
        s.press(Action::Scene(3), true, 0.0);
        assert_eq!(s.look.motion, Motion::Eight);
        assert_eq!(s.active_scene, Some(3));
        // 空のシーンは何もしない
        s.press(Action::Scene(5), true, 0.0);
        assert_eq!(s.active_scene, Some(3));
    }

    #[test]
    fn freeze_holds_position() {
        let mut s = state();
        run(&mut s, 0.0, 1.0);
        s.press(Action::Freeze, true, 1.0);
        s.tick(1.025);
        let p = s.heads[0].pan;
        run(&mut s, 1.05, 3.0);
        assert_eq!(s.heads[0].pan, p);
    }

    #[test]
    fn auto_changes_look_every_16_bars() {
        let mut s = state();
        s.press(Action::Auto, true, 0.0);
        s.tick(0.0); // 120BPM: 16小節 = 32秒
        let before = s.look;
        run(&mut s, 0.0, 31.9);
        assert_eq!(s.look, before);
        let mut changed = false;
        for k in 0..4 {
            run(&mut s, 32.0 * (k + 1) as f64, 32.0 * (k + 1) as f64 + 0.1);
            changed |= s.look != before;
        }
        assert!(changed);
    }

    #[test]
    fn invert_mirrors_axis() {
        let mut s = state();
        s.press(Action::Motion(Motion::Still), true, 0.0);
        s.faders[F_PAN] = 0.8;
        s.setup.invert_pan[2] = true;
        s.tick(0.0);
        assert!((s.heads[0].pan + s.heads[2].pan - 1.0).abs() < 1e-6);
    }
}
