//! OSCで受けた値を、少しずつ近づけて滑らかにする。
use super::*;

/// OSCで受けた値(色・明るさ)。受けた値(target)へ、今の値(current)を少しずつ近づける
#[derive(Debug, Clone, Copy)]
pub struct OscGlide<const N: usize> {
    pub target: [f32; N],
    pub current: [f32; N],
    /// 近づける速さ(秒。だいたいこの時間で6割ほど近づく。0ならすぐ)
    pub secs: f32,
    /// この時刻にちょうど target になるよう、まっすぐ近づける(/dmx/main/rgb/at。先読みした色を遅れなく出す)。
    /// None なら secs の速さで近づく
    pub due: Option<Instant>,
    pub last: Instant,
}

/// OSCで受けた色(RGB)
pub type OscRgb = OscGlide<3>;

impl<const N: usize> OscGlide<N> {
    /// 受けた値を、`secs`の速さ(または`due`の時刻にちょうど)で目指す。初めてなら`from`から
    pub fn aim(slot: &mut Option<Self>, target: [f32; N], from: [f32; N], secs: f32, due: Option<Instant>, now: Instant) {
        let o = slot.get_or_insert(OscGlide { target, current: from, secs, due, last: now });
        o.step(now);
        o.target = target;
        o.secs = secs;
        o.due = due;
    }

    /// `now`まで進めた今の値
    pub fn step(&mut self, now: Instant) -> [f32; N] {
        let dt = now.saturating_duration_since(self.last).as_secs_f32();
        let remain = self.due.map(|d| d.saturating_duration_since(self.last).as_secs_f32());
        self.last = now;
        let k = match remain {
            // 着く時刻までの残りのうち、今回進んだぶんだけ近づく(着く時刻を過ぎたらその色)
            Some(r) => if r <= dt { 1.0 } else { dt / r },
            None if self.secs <= 0.0 => 1.0,
            None => 1.0 - (-dt / self.secs).exp(),
        };
        for c in 0..N {
            self.current[c] += (self.target[c] - self.current[c]) * k;
        }
        self.current
    }
}
