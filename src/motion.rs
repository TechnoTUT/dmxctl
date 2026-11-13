//! ムービングの動き(SPOT4と同じ考え方): 「形」×「1周の長さ」×「揃え方」×「向き」を別々に選んで組み合わせる。
//! 床置きのムービングを「真上からどちらへ何度傾けるか」で動かすので、真上を通る動きでもPanが急に半回転しない。
//! 動かすのは、パン/チルトを持つ機材(レーザーなどARMが必要な機材は除く)。並びはパッチの順(左から)

use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

/// Panの角度の範囲(MINI SPOT 30 は540°)
pub const PAN_DEG: f64 = 540.0;
/// Tiltの角度の範囲(MINI SPOT 30 は270°)
pub const TILT_DEG: f64 = 270.0;

/// 動きの形
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Shape {
    /// 動かさない(向きだけ)
    #[default]
    Still,
    Circle,
    Eight,
    /// 左右に振る
    PanSweep,
    /// 前後に振る
    TiltSweep,
    /// 隣どうしが逆に振れて、ビームがハサミのように交差する
    Cross,
    Random,
    /// 前後の波が、左の台から右の台へ伝わる
    Wave,
}

pub const SHAPES: [(Shape, &str); 8] = [
    (Shape::Still, "止める"),
    (Shape::Circle, "円"),
    (Shape::Eight, "8の字"),
    (Shape::PanSweep, "左右"),
    (Shape::TiltSweep, "前後"),
    (Shape::Cross, "クロス"),
    (Shape::Random, "ランダム"),
    (Shape::Wave, "ウェーブ"),
];

/// 台どうしの揃え方
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Spread {
    /// 全台同じ動き
    Sync,
    /// 台ごとに少しずつ遅らせる
    #[default]
    Fan,
    /// 左半分と右半分で、左右の向きが逆(鏡写し)
    Mirror,
}

pub const SPREADS: [(Spread, &str); 3] = [(Spread::Sync, "揃える"), (Spread::Fan, "ずらす"), (Spread::Mirror, "左右対称")];

/// 1周にかける拍数と表示名
pub const RATES: [(f32, &str); 5] = [(16.0, "4小節"), (8.0, "2小節"), (4.0, "1小節"), (2.0, "2拍"), (1.0, "1拍")];

/// 基準の向き(床に横一列で置いたとき)
pub const POSITIONS: [&str; 4] = ["真上", "扇形", "交差", "客席"];

/// 今の動き(保存する)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Motion {
    pub shape: Shape,
    /// 1周の拍数
    pub beats: f32,
    pub spread: Spread,
    /// 基準の向き(POSITIONS の番号)。Noneなら、パン/チルトの今の値(位置プリセットなど)を中心にする
    pub position: Option<u8>,
    /// 動きの大きさ 0..1(1で中心から45°)
    pub size: f32,
    /// 客席(正面)を向くときのPanの値 0..1(本体の置き方で決まる)
    pub pan_front: f32,
    /// 真上を向くときのTiltの値 0..1
    pub tilt_up: f32,
}

impl Default for Motion {
    fn default() -> Self {
        Motion { shape: Shape::Still, beats: 4.0, spread: Spread::Fan, position: None, size: 0.5, pan_front: 0.5, tilt_up: 0.5 }
    }
}

impl Motion {
    /// パン/チルトを書き換えるか(止めていて向きも選んでいなければ、今の値のまま)
    pub fn active(&self) -> bool {
        self.shape != Shape::Still || self.position.is_some()
    }
}

/// 動きの操作(パッド・キーボード・画面のボタン共通)
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum MotionOp {
    Shape(Shape),
    /// 1周の拍数
    Beats(f32),
    Spread(Spread),
    /// 基準の向き。選んでいるものをもう一度押すと、今の値(位置プリセット)に戻る
    Position(u8),
    /// フェーダーで動きの大きさ
    Size,
    /// 動きの大きさを足し引きする(キーボード用)
    SizeStep(f32),
}

impl MotionOp {
    /// 割り当ての編集で選べる操作(フェーダーの大きさは別)
    pub fn all() -> Vec<MotionOp> {
        let mut v: Vec<MotionOp> = SHAPES.iter().map(|x| MotionOp::Shape(x.0)).collect();
        v.extend(RATES.iter().map(|x| MotionOp::Beats(x.0)));
        v.extend(SPREADS.iter().map(|x| MotionOp::Spread(x.0)));
        v.extend((0..POSITIONS.len() as u8).map(MotionOp::Position));
        v.extend([MotionOp::Size, MotionOp::SizeStep(-0.1), MotionOp::SizeStep(0.1)]);
        v
    }

    /// 操作の種類(パッドの小さい見出し)
    pub fn group(self) -> &'static str {
        match self {
            MotionOp::Shape(_) | MotionOp::Size | MotionOp::SizeStep(_) => "動き",
            MotionOp::Beats(_) => "1周",
            MotionOp::Spread(_) => "揃え方",
            MotionOp::Position(_) => "向き",
        }
    }

    pub fn label(self) -> String {
        match self {
            MotionOp::Shape(s) => SHAPES.iter().find(|x| x.0 == s).map(|x| x.1).unwrap_or("?").to_string(),
            MotionOp::Beats(b) => RATES.iter().find(|x| (x.0 - b).abs() < 0.01).map(|x| x.1.to_string()).unwrap_or_else(|| format!("{b}拍")),
            MotionOp::Spread(s) => SPREADS.iter().find(|x| x.0 == s).map(|x| x.1).unwrap_or("?").to_string(),
            MotionOp::Position(p) => POSITIONS.get(p as usize).copied().unwrap_or("?").to_string(),
            MotionOp::Size => "動きの大きさ".into(),
            MotionOp::SizeStep(d) => if d < 0.0 { "動き 小さく" } else { "動き 大きく" }.into(),
        }
    }
}

/// 基準の向き(傾き: [右, 前] 度。0,0 = 真上)。k は並びの位置(左端 -1 〜 右端 +1)
pub fn base_lean(position: u8, k: f64) -> [f64; 2] {
    match position % 4 {
        0 => [0.0, 0.0],
        1 => [k * 35.0, 10.0],
        2 => [-k * 30.0, 10.0],
        _ => [k * 12.0, 50.0],
    }
}

/// Pan/Tiltの角度(度。Pan 0 = 正面、Tilt 0 = 真上)を、傾き [右, 前](度)にする
pub fn to_lean(pan_deg: f64, tilt_deg: f64) -> [f64; 2] {
    let p = pan_deg.to_radians();
    [tilt_deg * p.sin(), tilt_deg * p.cos()]
}

/// 傾き(右, 前。度)を、Pan/Tiltの角度にする。
/// 同じ向きになるPan/Tiltの組は2通り(Panを180°回してTiltを逆に)あるので、今のPanに近いほうを選ぶ。
/// `pan_lim` はPanの角度の範囲
pub fn aim(lean: [f64; 2], prev_pan: f64, pan_lim: (f64, f64)) -> (f64, f64) {
    let [x, y] = lean;
    let r = x.hypot(y).min(TILT_DEG / 2.0);
    if r < 1.0 {
        // ほぼ真上: Panはそのままにして、Tiltだけで寄せる
        let p = prev_pan.to_radians();
        return (prev_pan, y * p.cos() + x * p.sin());
    }
    let phi = x.atan2(y).to_degrees();
    let mut best: Option<(f64, f64, f64)> = None;
    for (base, t) in [(phi, r), (phi + 180.0, -r)] {
        for k in -2..=2 {
            let p = base + 360.0 * k as f64;
            if p < pan_lim.0 || p > pan_lim.1 {
                continue;
            }
            let d = (p - prev_pan).abs();
            if best.is_none_or(|b| d < b.2) {
                best = Some((p, t, d));
            }
        }
    }
    best.map(|(p, t, _)| (p, t)).unwrap_or((prev_pan, 0.0))
}

/// ランダムな動きの目標(-1..1)。1周の1/4ごとに変わる
fn random_target(step: i64, i: usize) -> [f64; 2] {
    let h = |salt: u64| {
        let mut x = (step as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (i as u64 + 1).wrapping_mul(0xBF58_476D_1CE4_E5B9) ^ salt;
        x ^= x >> 31;
        x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^= x >> 29;
        (x >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    [h(1), h(2)]
}

/// i 番目(左から。全n台)の傾き [右, 前](度)。`center` は動きの中心、`phase` は動きの位置(1で1周)
pub fn lean(m: &Motion, center: [f64; 2], phase: f64, i: usize, n: usize) -> [f64; 2] {
    let n = n.max(1);
    let fan = if m.spread == Spread::Fan { i as f64 / n as f64 } else { 0.0 };
    let p = phase + fan;
    let (sin, cos) = ((p * TAU).sin(), (p * TAU).cos());
    let a = m.size.clamp(0.0, 1.0) as f64 * 45.0;
    let mut center = center;
    let (mut x, y) = match m.shape {
        Shape::Still => (0.0, 0.0),
        Shape::Circle => {
            // 真上を囲む円は、Panが回り続けないと描けないので、円が真上にかからないところまで中心を倒す
            let d = center[0].hypot(center[1]);
            let need = a * 1.5;
            if d < need {
                center = if d < 1e-6 { [0.0, need] } else { [center[0] * need / d, center[1] * need / d] };
            }
            (cos * a, sin * a)
        }
        Shape::Eight => (sin * a, (2.0 * p * TAU).sin() * a * 0.5),
        Shape::PanSweep => (sin * a, 0.0),
        Shape::TiltSweep => (0.0, sin * a),
        Shape::Cross => (if i.is_multiple_of(2) { sin } else { -sin } * a, 0.0),
        Shape::Random => {
            let t = random_target((phase * 4.0).floor() as i64, i);
            (t[0] * a, t[1] * a)
        }
        Shape::Wave => (0.0, ((phase + i as f64 / (n * 2) as f64) * TAU).sin() * a),
    };
    if m.spread == Spread::Mirror && n > 1 && i >= n.div_ceil(2) {
        x = -x;
    }
    [center[0] + x, center[1] + y]
}

/// 並びの位置 i(全n台)を、左端 -1 〜 右端 +1 にする
pub fn row_pos(i: usize, n: usize) -> f64 {
    if n > 1 { i as f64 / (n - 1) as f64 * 2.0 - 1.0 } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(pan: f64, tilt: f64) -> [f64; 3] {
        let (p, t) = (pan.to_radians(), tilt.to_radians());
        [t.sin() * p.sin(), t.cos(), t.sin() * p.cos()]
    }

    #[test]
    fn aim_round_trips() {
        for lean in [[0.0, 30.0], [20.0, -10.0], [-40.0, 5.0], [10.0, 60.0]] {
            let (p, t) = aim(lean, 0.0, (-270.0, 270.0));
            let back = to_lean(p, t);
            assert!((back[0] - lean[0]).abs() < 1e-6 && (back[1] - lean[1]).abs() < 1e-6, "{lean:?} → {back:?}");
        }
    }

    #[test]
    fn sweep_through_top_keeps_pan() {
        // 真上を横切る左右の振り: Panは半回転しない
        let mut prev = 0.0;
        for k in -10..=10 {
            let (p, _) = aim([k as f64 * 3.0, 0.0], prev, (-270.0, 270.0));
            assert!((p - prev).abs() < 100.0 || k == -10, "Panが飛んだ: {prev} → {p}");
            prev = p;
        }
    }

    #[test]
    fn mirror_flips_right_half() {
        let m = Motion { shape: Shape::PanSweep, spread: Spread::Mirror, ..Motion::default() };
        let l = |i| lean(&m, [0.0, 0.0], 0.25, i, 4)[0];
        assert!(l(0) > 0.0 && l(1) > 0.0 && l(2) < 0.0 && l(3) < 0.0);
    }

    #[test]
    fn fan_position_points_outward() {
        let (l, r) = (base_lean(1, row_pos(0, 4)), base_lean(1, row_pos(3, 4)));
        assert!(l[0] < -30.0 && r[0] > 30.0);
        let (p, t) = aim(l, 0.0, (-270.0, 270.0));
        assert!(dir(p, t)[0] < -0.3, "扇形の左端は左へ傾く");
    }
}
