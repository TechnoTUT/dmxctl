//! オート: Beat Link Trigger(BLT)などから曲の情報(色・明るさ・フレーズ)が届いている間は、それに合わせて自動で照明を動かす。
//! 届かなくなったら(DJ機器が止まった・つながっていない)、全体を薄い水色で静かに光らせて待つ(待機)。
//! 画面の無いPC(`dmxctl headless`)で、BLTと一緒に置いておけば、人が操作しなくても照明が動く

use super::*;
use crate::motion::{MotionOp, Shape};

/// 待機の見た目と、本番の見た目を行き来する時間(秒)
const IDLE_FADE: f32 = 1.5;

/// Beat Link Trigger の様子
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BltState {
    /// 曲の情報(色・明るさ・フレーズ)が届いている
    Receiving,
    /// Linkの仲間はいる(BLTは動いていて、DJ機器が再生していない)
    Waiting,
    /// しばらく気配が無い(BLTが止まっている・ネットワークが届いていないかもしれない)。気配が無くなってからの時間
    Missing(Duration),
    /// まだ分からない(起動直後など)・知らせない設定
    Unknown,
}

impl BltState {
    /// 知らせる文(画面・ログ)
    pub fn message(self) -> String {
        match self {
            BltState::Receiving => "BLT: 曲の情報を受信中".into(),
            BltState::Waiting => "BLT: つながっています(再生待ち)".into(),
            BltState::Missing(d) => {
                let s = d.as_secs();
                let t = if s < 120 { format!("{s}秒") } else { format!("{}分", s / 60) };
                format!("⚠ Beat Link Trigger が見つかりません({t})。BLT が止まっていないか確かめて、止まっていたら起動してください(TechnoTUT ネットワークの BLT なら blt-start.sh)")
            }
            BltState::Unknown => "BLT: 待っています".into(),
        }
    }
}

impl Core {
    /// BLTなどから曲の情報が届いた(オートの「つながっている」の目印)
    pub(crate) fn note_live_input(&mut self) {
        self.rt.auto_last = Some(self.now());
    }

    /// MIDIコントローラー・キーボードで操作した(手動優先)
    pub(crate) fn note_manual(&mut self) {
        self.rt.manual_last = Some(self.now());
    }

    /// 手動優先の間か(オートがONで、さっき人が操作した)。この間はオートは照明を変えない
    pub fn manual_active(&self) -> bool {
        let secs = self.project.auto.manual_secs;
        self.project.auto.enabled && secs > 0.0 && self.rt.manual_last.is_some_and(|t| self.now().saturating_duration_since(t).as_secs_f32() < secs)
    }

    /// 手動優先の残りの秒数
    pub fn manual_remaining(&self) -> Option<f32> {
        self.manual_active().then(|| self.project.auto.manual_secs - self.rt.manual_last.map(|t| self.now().saturating_duration_since(t).as_secs_f32()).unwrap_or(0.0))
    }

    /// Beat Link Trigger の様子(画面・ログの表示用)
    pub fn blt_state(&self) -> BltState {
        let cfg = &self.project.auto;
        if self.auto_online() {
            return BltState::Receiving;
        }
        if self.rt.link_peers > 0 && self.project.tempo_source == TempoSource::Link {
            return BltState::Waiting;
        }
        let gone = self.now().saturating_duration_since(self.rt.blt_seen);
        if cfg.enabled && cfg.alert_secs > 0.0 && gone.as_secs_f32() >= cfg.alert_secs {
            BltState::Missing(gone)
        } else {
            BltState::Unknown
        }
    }

    /// オートで、曲の情報が届いているか
    pub fn auto_online(&self) -> bool {
        let timeout = Duration::from_secs_f32(self.project.auto.timeout_secs.clamp(1.0, 600.0));
        self.rt.auto_last.is_some_and(|t| self.now().saturating_duration_since(t) < timeout)
    }

    /// 毎フレーム: つながった/切れたときの切り替えと、待機の見た目へのフェード
    pub(super) fn run_auto(&mut self, dt: f64) {
        let enabled = self.project.auto.enabled;
        let online = enabled && self.auto_online();
        // BLTの気配: 曲の情報が届いている、または(テンポがLinkで)Linkの仲間がいる
        if self.auto_online() || (self.rt.link_peers > 0 && self.project.tempo_source == TempoSource::Link) {
            self.rt.blt_seen = self.now();
        }
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
                // 切れた: ムービングは止めて、色・明るさの受信もやめる。プリセットの明るさ・ONにしたエフェクトも戻す
                self.motion_op(MotionOp::Shape(Shape::Still), 0.0);
                self.rt.osc_rgb = [None, None];
                self.rt.osc_level = None;
                self.rt.auto_bright = None;
                self.rt.wave_color_off = false;
                self.rt.auto_key.clear();
                for name in std::mem::take(&mut self.rt.auto_fx) {
                    self.set_fx_by_name(&name, false);
                }
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

    /// フレーズに合わせて、今のプリセットの見た目にする(オートがONで、手動優先でないとき)。
    /// プリセットの見た目でシーンを呼び出したら true
    pub fn auto_phrase(&mut self, key: &str) -> bool {
        if !self.project.auto.enabled || !self.project.auto.builtin || self.manual_active() {
            return false;
        }
        let Some(look) = self.preset_look(key) else { return false };
        self.apply_look(key, &look);
        !look.scene.is_empty()
    }

    /// 今のプリセットの、そのフレーズの見た目
    pub fn preset_look(&self, key: &str) -> Option<PhraseLook> {
        let a = &self.project.auto;
        a.presets.get(a.preset).or(a.presets.first())?.looks.get(key).cloned()
    }

    /// フレーズの見た目にする(プリセットの画面の「試す」からも)
    pub fn apply_look(&mut self, key: &str, look: &PhraseLook) {
        let entering = self.rt.auto_key != key;
        self.rt.auto_key = key.to_string();
        // ムービング
        self.motion_op(MotionOp::Shape(look.shape), 0.0);
        self.motion_op(MotionOp::Beats(look.beats), 0.0);
        self.motion_op(MotionOp::Spread(look.spread), 0.0);
        if look.position != self.project.motion.position {
            self.project.motion.position = look.position;
            if look.position.is_some() {
                // 向きを選んだときと同じく、ムービングのパン/チルトのエフェクトは止める
                self.motion_op(MotionOp::Shape(look.shape), 0.0);
            }
        }
        // 色
        match look.color {
            ColorMove::Keep => {}
            ColorMove::Swap => {
                self.project.palette.auto_beats = look.color_beats.max(0.25);
                self.set_palette_auto(true);
            }
            ColorMove::Fade => {
                self.project.palette.auto_beats = look.color_beats.max(0.25);
                self.set_palette_fade(true);
            }
        }
        self.rt.wave_color_off = !look.wave_color;
        if !look.wave_color {
            self.rt.osc_rgb = [None, None];
        }
        // 明るさ(1秒かけて)
        let now = self.now();
        OscGlide::aim(&mut self.rt.auto_bright, [look.brightness.clamp(0.0, 1.0)], [1.0], 1.0, None, now);
        // シーン
        if !look.scene.is_empty() {
            self.recall_scene(&look.scene);
        }
        // エフェクト: 前のフレーズでONにしたもののうち、今回に無いものはOFFに戻す
        for name in std::mem::take(&mut self.rt.auto_fx) {
            if !look.effects.contains(&name) {
                self.set_fx_by_name(&name, false);
            }
        }
        for name in &look.effects {
            self.set_fx_by_name(name, true);
        }
        self.rt.auto_fx = look.effects.clone();
        // 入った瞬間に1拍だけ全開(レーザーは除く)
        if entering && look.flash && self.project.auto.accent && self.rt.flash.is_none() {
            self.rt.flash = Some(false);
            self.rt.accent_until = Some(self.rt.beat.floor() + 1.0);
        }
    }

    /// エフェクト・ピクセルエフェクト・チェイスを名前でON/OFF
    pub fn set_fx_by_name(&mut self, name: &str, on: bool) {
        self.set_effect(name, on);
        if self.project.pixel_fx.iter().any(|e| e.name == name) {
            self.set_pixel_fx(name, on);
        }
        for c in self.project.chases.iter_mut().filter(|c| c.name == name) {
            c.enabled = on;
        }
        self.dirty = true;
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
