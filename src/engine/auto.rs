//! オート: Beat Link Trigger(BLT)などから曲の情報(色・明るさ・フレーズ)が届いている間は、それに合わせて自動で照明を動かす。
//! 届かなくなったら(DJ機器が止まった・つながっていない)、全体を薄い水色で静かに光らせて待つ(待機)。
//! 画面の無いPC(`dmxctl --headless`)で、BLTと一緒に置いておけば、人が操作しなくても照明が動く

use super::*;
use crate::motion::{MotionOp, Shape, Spread};

/// 待機の見た目と、本番の見た目を行き来する時間(秒)
const IDLE_FADE: f32 = 1.5;

impl Core {
    /// BLTなどから曲の情報が届いた(オートの「つながっている」の目印)
    pub(crate) fn note_live_input(&mut self) {
        self.rt.auto_last = Some(Instant::now());
    }

    /// MIDIコントローラー・キーボードで操作した(手動優先)
    pub(crate) fn note_manual(&mut self) {
        self.rt.manual_last = Some(Instant::now());
    }

    /// 手動優先の間か(オートがONで、さっき人が操作した)。この間はオートは照明を変えない
    pub fn manual_active(&self) -> bool {
        let secs = self.project.auto.manual_secs;
        self.project.auto.enabled && secs > 0.0 && self.rt.manual_last.is_some_and(|t| t.elapsed().as_secs_f32() < secs)
    }

    /// 手動優先の残りの秒数
    pub fn manual_remaining(&self) -> Option<f32> {
        self.manual_active().then(|| self.project.auto.manual_secs - self.rt.manual_last.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0))
    }

    /// ビルドアップ(アップ・ブリッジ)からサビに入った瞬間、1拍だけ今の色で全開にする
    pub(crate) fn accent_on_drop(&mut self, prev: &str, key: &str) {
        if self.project.auto.enabled && self.project.auto.accent && key == "chorus" && matches!(prev, "up" | "bridge") && self.rt.flash.is_none() {
            self.rt.flash = Some(false);
            self.rt.accent_until = Some(self.rt.beat.floor() + 1.0);
        }
    }

    /// オートで、曲の情報が届いているか
    pub fn auto_online(&self) -> bool {
        let timeout = Duration::from_secs_f32(self.project.auto.timeout_secs.clamp(1.0, 600.0));
        self.rt.auto_last.is_some_and(|t| t.elapsed() < timeout)
    }

    /// 毎フレーム: つながった/切れたときの切り替えと、待機の見た目へのフェード
    pub(super) fn run_auto(&mut self, dt: f64) {
        let enabled = self.project.auto.enabled;
        let online = enabled && self.auto_online();
        let manual = self.manual_active();
        // サビの頭の全開は1拍で戻す(オートを切った・手動になったときも)
        if let Some(u) = self.rt.accent_until
            && (self.rt.beat >= u || !enabled || manual) {
            self.rt.flash = None;
            self.rt.accent_until = None;
        }
        // 盛り上がり: 受けた明るさを約4秒でならす
        if let Some(l) = self.rt.energy_input {
            self.rt.energy += (l - self.rt.energy) * (1.0 - (-(dt as f32) / 4.0).exp());
        }
        // 手動優先の間は、つながった/切れたの切り替えもしない(人の操作をそのままにする。終わってから切り替える)
        if enabled && !manual && online != self.rt.auto_was_online {
            if online {
                // つながった: 今のフレーズ(まだ無ければバース相当)の動きで始める
                let key = if self.rt.phrase.is_empty() { "verse".to_string() } else { crate::osc::phrase_key(&self.rt.phrase) };
                self.auto_phrase(&key);
            } else {
                // 切れた: ムービングは止めて、色・明るさの受信もやめる
                self.motion_op(MotionOp::Shape(Shape::Still), 0.0);
                self.rt.osc_rgb = [None, None];
                self.rt.osc_level = None;
            }
        }
        if !manual {
            self.rt.auto_was_online = online;
        }
        if enabled && online && !manual {
            self.run_energy();
        }
        let target = if enabled && !online && !manual { 1.0 } else { 0.0 };
        let step = (dt as f32 / IDLE_FADE).max(0.0);
        self.rt.idle_mix = if self.rt.idle_mix < target { (self.rt.idle_mix + step).min(target) } else { (self.rt.idle_mix - step).max(target) };
    }

    /// フレーズの情報が来ない曲(rekordboxでフレーズを解析していない・64拍以上届かない)では、
    /// 16拍ごとに、盛り上がりで見た目を選ぶ(大きい: サビ / 中くらい: バース / 小さい: ダウン)
    fn run_energy(&mut self) {
        let cfg = &self.project.auto;
        if !cfg.energy || !cfg.builtin || self.rt.energy_input.is_none() {
            return;
        }
        if self.rt.phrase_beat.is_some_and(|b| self.rt.beat - b < 64.0) {
            return;
        }
        let block = (self.rt.beat / 16.0).floor() as i64;
        if block == self.rt.energy_block {
            return;
        }
        self.rt.energy_block = block;
        let key = match self.rt.energy {
            e if e > 0.7 => "chorus",
            e if e > 0.45 => "verse",
            _ => "down",
        };
        if key != self.rt.auto_key {
            self.auto_phrase(key);
        }
    }

    /// フレーズに合わせた、組み込みの動き(ムービングの動きと、メイン/サブの色の入れ替え方)。
    /// 盛り上がるところほど速く大きく、静かなところはゆっくり
    pub fn auto_phrase(&mut self, key: &str) {
        if !self.project.auto.enabled || !self.project.auto.builtin || self.manual_active() {
            return;
        }
        // (形, 1周の拍数, 揃え方, 色: true=拍で入れ替え / false=グラデーション, 色の間隔の拍数)
        let (shape, beats, spread, swap, every) = match key {
            "intro" | "outro" => (Shape::Circle, 16.0, Spread::Fan, false, 8.0),
            "verse" => (Shape::Eight, 8.0, Spread::Fan, true, 4.0),
            "bridge" => (Shape::PanSweep, 4.0, Spread::Mirror, false, 4.0),
            "up" => (Shape::TiltSweep, 2.0, Spread::Sync, true, 2.0),
            "chorus" => (Shape::Cross, 2.0, Spread::Mirror, true, 1.0),
            "down" => (Shape::Wave, 8.0, Spread::Fan, false, 8.0),
            _ => return,
        };
        self.rt.auto_key = key.to_string();
        self.motion_op(MotionOp::Shape(shape), 0.0);
        self.motion_op(MotionOp::Beats(beats), 0.0);
        self.motion_op(MotionOp::Spread(spread), 0.0);
        self.project.palette.auto_beats = every;
        if swap {
            self.set_palette_auto(true);
        } else {
            self.set_palette_fade(true);
        }
    }

    /// 待機の見た目(薄い水色・控えめの明るさ)を、出力の直前の値に重ねる。`idle_mix`=0〜1
    pub(super) fn apply_idle(&self, vals: &mut [Vec<u8>]) {
        let mix = self.rt.idle_mix;
        if mix <= 0.0 {
            return;
        }
        let cfg = &self.project.auto;
        let rgb = cfg.idle_rgb.map(|v| v.clamp(0.0, 1.0));
        // 全体光量のフェーダーと暗転フェードは、待機でも効く
        let level = cfg.idle_level.clamp(0.0, 1.0) * self.rt.master * self.rt.dim;
        let lib = &self.lib;
        for (i, fx) in self.project.fixtures.iter().enumerate() {
            let Some(profile) = lib.get(&fx.profile) else { continue };
            let (Some(mode), Some(info)) = (profile.modes.get(fx.mode), lib.info(&fx.profile, fx.mode)) else { continue };
            // レーザーなどARMが必要な機材は、待機では出さない(ブラックアウト・ARMの処理に任せる)
            if profile.needs_arm || vals[i].len() != mode.channels.len() {
                continue;
            }
            let mut idle = vals[i].clone();
            for (attr, val) in color::plan_for_mode(mode, rgb) {
                for &ch in info.channels(&attr) {
                    info.set_f(&mut idle, ch, val);
                }
            }
            for p in &info.pixels {
                info.set_pixel(&mut idle, p, rgb);
            }
            // ストロボ・シャッターは機材の既定値(開いた状態)に
            for (c, ch) in mode.channels.iter().enumerate() {
                if matches!(ch.attr.as_str(), "strobe" | "shutter") {
                    idle[c] = ch.default;
                }
            }
            for &ch in info.channels("dimmer") {
                info.set_f(&mut idle, ch, 255.0);
            }
            for &ch in &info.master {
                let v = info.get_f(&idle, ch);
                info.set_f(&mut idle, ch, v * level);
            }
            for (o, n) in vals[i].iter_mut().zip(idle) {
                *o = (*o as f32 + (n as f32 - *o as f32) * mix).round() as u8;
            }
        }
    }
}
