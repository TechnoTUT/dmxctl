//! 出力の直前に値を直す。禁止値・動いてよい範囲・安全な値・フラッシュ。安全の要なので、ほかの経路を作らない。
use super::*;

/// 安全な値にする: 安全値のあるチャンネルは安全値(レーザーOFFなど)、明るさのチャンネルは0
pub(super) fn safe_values(mode: &Mode, info: &crate::fixture::ModeInfo, v: &mut [u8]) {
    for (c, ch) in mode.channels.iter().enumerate() {
        if let Some(s) = ch.safe {
            v[c] = s;
        }
    }
    for &ch in &info.master {
        info.set_f(v, ch, 0.0);
    }
}

/// 禁止の値を使える値に置き換え、機材ごとの動いてよい範囲に収める
pub fn guard_values(mode: &Mode, limits: &BTreeMap<String, [u8; 2]>, v: &mut [u8]) {
    for (c, ch) in mode.channels.iter().enumerate() {
        let Some(x) = v.get_mut(c) else { break };
        let [a, b] = limits.get(&ch.attr).copied().unwrap_or([0, 255]);
        let (lo, hi) = (a.min(b), a.max(b));
        let clamped = (*x).clamp(lo, hi);
        let allowed = ch.allowed(clamped);
        *x = if (lo..=hi).contains(&allowed) && !ch.is_forbidden(allowed) {
            allowed
        } else {
            // Preserve both constraints when the safe replacement is outside the limits.
            (lo..=hi).filter(|&n| !ch.is_forbidden(n))
                .min_by_key(|&n| n.abs_diff(clamped)).unwrap_or(allowed)
        };
    }
}

/// フラッシュの値にする。white=白で全開 / false=今の色のまま全開(色が無ければ白)
pub(super) fn flash_values(mode: &Mode, info: &crate::fixture::ModeInfo, v: &mut [u8], white: bool) {
    let full = |c: [f32; 3]| {
        let m = c[0].max(c[1]).max(c[2]);
        if m < 1e-3 { [1.0; 3] } else { c.map(|x| x / m) }
    };
    if white {
        for (attr, val) in color::plan_for_mode(mode, [1.0; 3]) {
            for &ch in info.channels(&attr) {
                info.set_f(v, ch, val);
            }
        }
    } else if !info.pixels.is_empty() {
        for p in &info.pixels {
            let c = full(mode.pixel_rgb(v, p));
            info.set_pixel(v, p, c);
        }
    } else if info.channels("dimmer").is_empty() {
        // ディマーの無いRGB灯具: 色の比率を保ったまま、いちばん強い色を最大に
        let chans: Vec<usize> = ["red", "green", "blue", "white"].iter().flat_map(|a| info.channels(a).to_vec()).collect();
        let max = chans.iter().map(|&c| v[c]).max().unwrap_or(0);
        for &c in &chans {
            v[c] = if max == 0 { 255 } else { (v[c] as f32 * 255.0 / max as f32).round() as u8 };
        }
    }
    for &ch in info.channels("dimmer") {
        info.set_f(v, ch, 255.0);
    }
}

impl Core {
    /// 異常時に送るフレーム: 最後に送ったフレームから、全機材を安全な値(レーザーOFF・明るさ0)にしたもの
    pub fn safe_frames(&self) -> Vec<[u8; 512]> {
        let mut frames = self.rt.frames.clone();
        for fx in &self.project.fixtures {
            let (Some(mode), Some(info)) = (self.lib.mode(&fx.profile, fx.mode), self.lib.info(&fx.profile, fx.mode)) else { continue };
            let n = mode.channels.len();
            let base = (fx.address as usize).saturating_sub(1);
            let Some(f) = frames.get_mut(fx.universe) else { continue };
            if fx.address == 0 || base + n > 512 {
                continue;
            }
            safe_values(mode, &info, &mut f[base..base + n]);
        }
        frames
    }
}
