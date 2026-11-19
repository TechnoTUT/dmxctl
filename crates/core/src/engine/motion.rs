//! ムービングの動き(crate::motion)を、ムービングのパン/チルトに書き込む。

use super::*;
use crate::motion::{self, MotionOp, Shape, PAN_DEG, TILT_DEG};

impl Core {
    /// 動きで動かす機材(パン/チルトがあり、ARMが要らない、使っている機材)。パッチの順 = 左から
    pub fn motion_heads(&self) -> Vec<usize> {
        self.project
            .fixtures
            .iter()
            .enumerate()
            .filter(|(_, f)| f.enabled && self.lib.get(&f.profile).is_some_and(|p| !p.needs_arm))
            .filter(|(_, f)| self.lib.mode(&f.profile, f.mode).is_some_and(|m| m.find_attr("pan").is_some() && m.find_attr("tilt").is_some()))
            .map(|(i, _)| i)
            .collect()
    }

    /// 左から`k`台目のムービングが、今(動きを掛ける前の値で)向いている傾き [右, 前](度)。「4点」の向きを登録するとき用
    pub fn head_lean(&self, k: usize) -> Option<[f32; 2]> {
        let i = *self.motion_heads().get(k)?;
        let fx = &self.project.fixtures[i];
        let info = self.lib.info(&fx.profile, fx.mode)?;
        let (pc, tc) = (*info.channels("pan").first()?, *info.channels("tilt").first()?);
        let m = &self.project.motion;
        let pan_deg = (info.get_f(&fx.values, pc) as f64 - m.pan_front.clamp(0.0, 1.0) as f64 * 255.0) / 255.0 * PAN_DEG;
        let tilt_deg = (info.get_f(&fx.values, tc) as f64 - m.tilt_up.clamp(0.0, 1.0) as f64 * 255.0) / 255.0 * TILT_DEG;
        Some(motion::to_lean(pan_deg, tilt_deg).map(|v| v as f32))
    }

    /// 動きの操作。`v`はフェーダーの値(0..1)
    pub fn motion_op(&mut self, op: MotionOp, v: f32) {
        let m = &mut self.project.motion;
        match op {
            MotionOp::Shape(s) => m.shape = s,
            MotionOp::Beats(b) => m.beats = b.max(0.25),
            MotionOp::Spread(s) => m.spread = s,
            MotionOp::Position(p) => m.position = if m.position == Some(p) { None } else { Some(p) },
            MotionOp::Size => m.size = v.clamp(0.0, 1.0),
            MotionOp::SizeStep(d) => m.size = ((m.size + d) * 20.0).round().clamp(0.0, 20.0) / 20.0,
            MotionOp::Glide => m.glide = v.clamp(0.0, 1.0),
        }
        // 動きを使うときは、ムービングのパン/チルトを動かしているエフェクトを止める(上書きされて効かないので)
        if self.project.motion.active() && matches!(op, MotionOp::Shape(_) | MotionOp::Position(_)) {
            for id in self.motion_heads().into_iter().map(|i| self.project.fixtures[i].id).collect::<Vec<_>>() {
                for attr in ["pan", "tilt"] {
                    self.stop_effects_on(&Selector::Fixture(id), attr);
                }
            }
        }
        self.dirty = true;
    }

    /// パン/チルトのエフェクトや位置プリセットを使い始めたら、動きのほうを止める(後から選んだほうが効くように)
    pub(crate) fn yield_motion(&mut self, position_only: bool) {
        let m = &mut self.project.motion;
        if !position_only {
            m.shape = Shape::Still;
        }
        m.position = None;
    }

    /// 動きを、エフェクト適用後の値`vals`のパン/チルトに書き込む
    pub(crate) fn render_motion(&mut self, vals: &mut [Vec<u8>]) {
        let m = self.project.motion.clone();
        // 動きの位置は拍の進みぶんだけ進める(1周の長さを変えても、位置が飛ばない)
        let db = self.rt.motion_beat.map(|b| (self.rt.beat - b).clamp(0.0, 1.0)).unwrap_or(0.0);
        self.rt.motion_beat = Some(self.rt.beat);
        self.rt.motion_phase = (self.rt.motion_phase + db / m.beats.max(0.25) as f64).rem_euclid(1024.0);
        if !m.active() {
            self.rt.motion_pan.clear();
            return;
        }
        let heads = self.motion_heads();
        let n = heads.len();
        let front = m.pan_front.clamp(0.0, 1.0) as f64 * 255.0;
        let up = m.tilt_up.clamp(0.0, 1.0) as f64 * 255.0;
        let lim = (-front / 255.0 * PAN_DEG, (255.0 - front) / 255.0 * PAN_DEG);
        for (k, i) in heads.into_iter().enumerate() {
            let fx = &self.project.fixtures[i];
            let Some(info) = self.lib.info(&fx.profile, fx.mode) else { continue };
            let (Some(&pc), Some(&tc)) = (info.channels("pan").first(), info.channels("tilt").first()) else { continue };
            let v = &mut vals[i];
            let pan_deg = (info.get_f(v, pc) as f64 - front) / 255.0 * PAN_DEG;
            let tilt_deg = (info.get_f(v, tc) as f64 - up) / 255.0 * TILT_DEG;
            let center = match m.position {
                Some(p) => motion::base_lean(p, motion::row_pos(k, n)),
                None => motion::to_lean(pan_deg, tilt_deg),
            };
            let lean = motion::lean(&m, center, self.rt.motion_phase, k, n);
            let prev = self.rt.motion_pan.get(&fx.id).copied().unwrap_or(pan_deg);
            let (p, t) = motion::aim(lean, prev, lim);
            info.set_f(v, pc, (front + p / PAN_DEG * 255.0) as f32);
            info.set_f(v, tc, (up + t / TILT_DEG * 255.0) as f32);
            self.rt.motion_pan.insert(fx.id, p);
        }
    }
}
