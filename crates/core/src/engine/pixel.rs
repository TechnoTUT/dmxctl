//! ピクセルエフェクトの ON/OFF。
use super::*;
use crate::pixel::*;

impl Core {
    /// ピクセルエフェクトをON/OFFする。ONにしたときは、同じLEDバーに掛かっている他のピクセルエフェクトをOFFにする
    /// (模様は後のものが上書きするので、1本のバーには1つずつ)
    pub fn set_pixel_fx(&mut self, name: &str, on: bool) {
        let mut mine: Vec<usize> = Vec::new();
        for fx in self.project.pixel_fx.iter().filter(|f| f.name == name) {
            mine.extend(self.resolve(&fx.sel));
        }
        let others: Vec<usize> = (0..self.project.pixel_fx.len())
            .filter(|&k| {
                let fx = &self.project.pixel_fx[k];
                on && fx.enabled && fx.name != name && self.resolve(&fx.sel).iter().any(|i| mine.contains(i))
            })
            .collect();
        for fx in self.project.pixel_fx.iter_mut().filter(|f| f.name == name) {
            fx.enabled = on;
        }
        for k in others {
            self.project.pixel_fx[k].enabled = false;
        }
        self.dirty = true;
    }

    /// 模様のプレビュー(パッド用): rows本 × cols点 の色。実際のバーの本数・ピクセル数で計算して間引く
    pub fn face_preview(&self, fx: &PixelFx, rows: usize, cols: usize) -> Vec<Vec<[f32; 3]>> {
        let (a, b) = (self.pixel_rgb_of(fx.a), self.pixel_rgb_of(fx.b));
        let phase = if fx.beats > 0.0 { self.pixel_beat() / fx.beats as f64 } else { 0.0 };
        let skew = fx.skew + self.rt.px_skew;
        // 実際のバーの数とピクセル数(無ければ8本×90ピクセルとして描く)
        let lens: Vec<usize> = self.project.fixtures.iter().filter(|f| f.enabled).filter_map(|f| self.lib.info(&f.profile, f.mode)).map(|i| i.pixels.len()).filter(|n| *n > 0).collect();
        let len = lens.first().copied().unwrap_or(90);
        let bars = if lens.is_empty() { rows.min(8) } else { lens.len() };
        let total = len * bars;
        (0..rows.min(bars))
            .map(|bar| {
                (0..cols)
                    .map(|c| {
                        let k = ((c as f32 + 0.5) * len as f32 / cols as f32) as usize;
                        face_color(fx, skew, phase, FacePos { bar, bars, k: k.min(len - 1), len, offset: bar * len, total }, a, b)
                    })
                    .collect()
            })
            .collect()
    }

    /// ピクセルエフェクトの拍の位置(速さの倍率を掛けたもの)
    pub fn pixel_beat(&self) -> f64 {
        self.rt.beat * self.rt.px_speed as f64 + self.rt.px_offset
    }

    /// ピクセルエフェクト全体の速さの倍率を変える。変えた瞬間に模様が飛ばないよう、今の位置を保つ
    pub fn set_pixel_speed(&mut self, s: f32) {
        let (old, new) = (self.rt.px_speed as f64, s as f64);
        if (old - new).abs() < 1e-9 {
            return;
        }
        self.rt.px_offset += self.rt.beat * (old - new);
        self.rt.px_speed = s;
    }

    pub fn pixel_rgb_of(&self, c: PixelColor) -> [f32; 3] {
        match c {
            PixelColor::Main => self.role_rgb(ColorRole::Main).unwrap_or([1.0; 3]),
            PixelColor::Sub => self.role_rgb(ColorRole::Sub).unwrap_or([0.0; 3]),
            PixelColor::Off => [0.0; 3],
            PixelColor::White => [1.0; 3],
            PixelColor::Custom(c) => c,
        }
    }

    /// 動作中のピクセルエフェクトを、出力値(`vals`: 機材ごとの値)に書き込む
    pub fn render_pixel_fx(&self, vals: &mut [Vec<u8>]) {
        for fx in self.project.pixel_fx.iter().filter(|f| f.enabled) {
            let a = self.pixel_rgb_of(fx.a);
            let b = self.pixel_rgb_of(fx.b);
            let phase = if fx.beats > 0.0 { self.pixel_beat() / fx.beats as f64 } else { 0.0 };
            // フェーダーの「斜め」は、どの模様にも足す
            let skew = fx.skew + self.rt.px_skew;
            // 対象のうち、ピクセルを持つ機材(パッチの順)
            let targets: Vec<(usize, &crate::fixture::Mode, std::sync::Arc<crate::fixture::ModeInfo>)> = self
                .resolve(&fx.sel)
                .into_iter()
                .filter_map(|i| {
                    let f = &self.project.fixtures[i];
                    let m = self.lib.mode(&f.profile, f.mode)?;
                    let info = self.lib.info(&f.profile, f.mode)?;
                    (!info.pixels.is_empty() && vals[i].len() == m.channels.len()).then_some((i, m, info))
                })
                .collect();
            let total: usize = targets.iter().map(|t| t.2.pixels.len()).sum();
            let bars = targets.len();
            let mut offset = 0;
            for (bar, (i, _, info)) in targets.iter().enumerate() {
                let px = &info.pixels;
                for (k, p) in px.iter().enumerate() {
                    let at = FacePos { bar, bars, k, len: px.len(), offset, total };
                    info.set_pixel(&mut vals[*i], p, face_color(fx, skew, phase, at, a, b));
                }
                offset += px.len();
            }
        }
    }
}
