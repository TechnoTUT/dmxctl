//! 今の出力(DMX)から、各機材が実際に出している色と明るさを読み取る(マップ画面の表示用)。
//! 全体光量・ブラックアウト・エフェクト・ピクセルエフェクト・ARMの状態がすべて反映された値を見る

use crate::color;
use crate::engine::Core;
use crate::fixture::split_pixel;

#[derive(Debug, Clone, PartialEq)]
pub enum Look {
    /// 1色の機材。rgb=色(分からなければNone)、level=明るさ(0〜1)、label=色の名前など
    One { rgb: Option<[f32; 3]>, level: f32, label: String },
    /// LEDバー: ピクセルごとの色
    Strip(Vec<[f32; 3]>),
}

#[derive(Debug, Clone)]
pub struct FixtureLook {
    pub id: u32,
    pub name: String,
    pub look: Look,
    /// 出ていない理由(SAFE / BLACKOUT / 暗転)
    pub note: Option<&'static str>,
}

pub fn looks(core: &Core) -> Vec<FixtureLook> {
    let mut out = Vec::new();
    for fx in &core.project.fixtures {
        let Some(profile) = core.lib.get(&fx.profile) else { continue };
        let Some(mode) = profile.modes.get(fx.mode) else { continue };
        let Some(frame) = core.rt.frames.get(fx.universe) else { continue };
        if fx.address == 0 {
            continue;
        }
        let base = fx.address as usize - 1;
        if base + mode.channels.len() > 512 {
            continue;
        }
        let v = &frame[base..base + mode.channels.len()];
        let note = if core.rt.blackout {
            Some("BLACKOUT")
        } else if core.rt.dim <= 0.001 {
            Some("暗転")
        } else if profile.needs_arm && !fx.armed {
            Some("SAFE")
        } else {
            None
        };
        let pixels = mode.pixels();
        let look = if !pixels.is_empty() {
            Look::Strip(pixels.iter().map(|p| mode.pixel_rgb(v, p)).collect())
        } else {
            let level = match (note, mode.find_attr("dimmer")) {
                (Some(_), _) => 0.0,
                (None, Some(d)) => v[d] as f32 / 255.0,
                (None, None) => 1.0,
            };
            let rgb_ch = |a: &str| mode.channels.iter().position(|c| c.attr == a && c.fine_of.is_none() && split_pixel(&c.attr).1.is_none());
            match (rgb_ch("red"), rgb_ch("green"), rgb_ch("blue")) {
                (Some(r), Some(g), Some(b)) => {
                    let w = rgb_ch("white").map(|w| v[w] as f32 / 255.0).unwrap_or(0.0);
                    let c = [(v[r] as f32 / 255.0 + w).min(1.0), (v[g] as f32 / 255.0 + w).min(1.0), (v[b] as f32 / 255.0 + w).min(1.0)];
                    Look::One { rgb: Some(c), level, label: color::nearest_named(c).label().into() }
                }
                _ => match mode.find_attr("color") {
                    Some(ch) => {
                        let r = mode.channels[ch].range_of(v[ch]);
                        Look::One {
                            rgb: r.and_then(color::range_rgb),
                            level,
                            label: r.map(|r| r.label.clone()).unwrap_or_else(|| format!("color={}", v[ch])),
                        }
                    }
                    None => Look::One { rgb: None, level, label: "色の情報なし".into() },
                },
            }
        };
        out.push(FixtureLook { id: fx.id, name: fx.name.clone(), look, note });
    }
    out
}
