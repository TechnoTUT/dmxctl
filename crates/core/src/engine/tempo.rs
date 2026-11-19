//! テンポ(BPM・タップ・Link・MIDIクロック・音声検出)と、拍に合わせて進むチェイス。

use super::*;

/// MIDIクロック(0xF8)は1拍に24回
pub const CLOCK_PPQ: f64 = 24.0;
/// これ以上クロックが来なければ途切れたとみなし、内部のテンポで走り続ける
pub const CLOCK_TIMEOUT: Duration = Duration::from_millis(500);
/// 音声検出を採用する確からしさの下限
pub const AUDIO_MIN_CONF: f32 = 0.25;

pub use crate::io::midi::ClockMsg;

/// MIDIクロックの受信状態
#[derive(Default)]
pub struct ClockIn {
    /// 数え始めたときの拍の位置
    base: f64,
    ticks: i64,
    pub last: Option<Instant>,
    times: VecDeque<Instant>,
    /// Startの直後: 次のクロックを拍0にする
    start_pending: bool,
}

impl Core {
    /// 画面用の、今この瞬間の拍の位置。`rt.beat` は出力のたび(25msごと)にしか進まないので、
    /// そのまま描くと拍で動く点などがガタガタに見える。最後に進めた時刻からの経過ぶんを足す
    pub fn beat_now(&self) -> f64 {
        let since = self.rt.last_beat_t.map(|t| self.now().saturating_duration_since(t).as_secs_f64().min(0.1)).unwrap_or(0.0);
        self.rt.beat + since * self.project.bpm as f64 / 60.0
    }

    pub fn advance_beat(&mut self, dt: f64) {
        self.rt.beat += dt * self.project.bpm as f64 / 60.0;
    }

    pub fn set_bpm(&mut self, v: f32) {
        let v = v.clamp(BPM_MIN, BPM_MAX);
        if (v - self.project.bpm).abs() > 1e-4 {
            self.project.bpm = v;
            self.dirty = true;
        }
        // Link中は、仲間(映像ソフトなど)のテンポも一緒に変える
        if let Some(l) = self.link() {
            l.set_tempo(v as f64);
        }
    }

    /// Link中なら、そのLink
    fn link(&self) -> Option<&crate::io::link::Link> {
        self.rt.link.as_ref().filter(|_| self.project.tempo_source == TempoSource::Link)
    }

    pub fn set_tempo_source(&mut self, s: TempoSource) {
        if self.project.tempo_source != s {
            self.project.tempo_source = s;
            self.rt.audio_seen = None;
            self.dirty = true;
        }
        self.ensure_link();
    }

    /// 設定に合わせてLinkを開始/終了する
    pub fn ensure_link(&mut self) {
        if self.project.tempo_source == TempoSource::Link {
            if self.rt.link.is_none() {
                self.rt.link = crate::io::link::Link::start(self.project.bpm as f64);
            }
        } else {
            self.rt.link = None;
            self.rt.link_peers = 0;
        }
    }

    /// 拍を進める。Link/MIDIクロックに合わせているときは、そちらの拍の位置を使う
    pub(super) fn sync_beat(&mut self, now: Instant, dt: f64) {
        match self.project.tempo_source {
            TempoSource::Link => {
                if let Some(l) = self.rt.link.as_ref()
                    && let Some((tempo, beat)) = l.state() {
                    self.rt.link_peers = l.peers();
                    self.rt.beat = beat;
                    // 仲間から来たテンポの変更は、保存のきっかけにしない
                    self.project.bpm = (tempo as f32).clamp(BPM_MIN, BPM_MAX);
                    return;
                }
            }
            TempoSource::MidiClock => {
                if let Some(b) = self.clock_beat(now) {
                    self.rt.beat = b;
                    return;
                }
            }
            TempoSource::Internal | TempoSource::Audio => {}
        }
        self.advance_beat(dt);
    }

    /// 音声検出の結果を受け取る。確からしいときだけ、BPMをなめらかに寄せ、拍のタイミングを少しずつ合わせる
    pub fn audio_report(&mut self, est: crate::io::audio::Estimate, now: Instant) {
        if self.project.tempo_source != TempoSource::Audio {
            return;
        }
        self.rt.audio_conf = est.confidence;
        if est.confidence < AUDIO_MIN_CONF {
            self.rt.audio_status = format!("リズムを探しています(確度 {:.0}%)", est.confidence * 100.0);
            return;
        }
        self.rt.audio_status = format!("検出中 {:.1} BPM(確度 {:.0}%)", est.bpm, est.confidence * 100.0);
        let locked = self.rt.audio_seen.is_some_and(|t| now.duration_since(t) < Duration::from_secs(5));
        let cur = self.project.bpm;
        let new = est.bpm.clamp(BPM_MIN, BPM_MAX);
        // 最初は一気に、以降は小さな揺れを無視してなめらかに寄せる(照明の動きが突然変わらないように)
        let next = if !locked {
            new
        } else if (new - cur).abs() < 0.4 {
            cur
        } else if (new - cur).abs() > 8.0 {
            // 大きく違うときは、2回続けて同じ値が出たときだけ従う
            if (new - self.rt.audio_pending).abs() < 1.0 { new } else { cur }
        } else {
            cur + (new - cur) * 0.35
        };
        self.rt.audio_pending = new;
        self.set_bpm((next * 10.0).round() / 10.0);
        // 拍のタイミング: いまの拍の端数と、音の拍の端数の差を、少しずつ詰める
        let mine = self.rt.beat.rem_euclid(1.0);
        let mut err = est.phase as f64 - mine;
        if err > 0.5 {
            err -= 1.0;
        } else if err < -0.5 {
            err += 1.0;
        }
        self.rt.beat += if locked { err * 0.2 } else { err };
        self.rt.audio_seen = Some(now);
    }

    /// MIDIクロックを受け取る(クロック中でなければ何もしない)
    pub fn clock_msg(&mut self, msg: ClockMsg, now: Instant) {
        if self.project.tempo_source != TempoSource::MidiClock {
            return;
        }
        let c = &mut self.rt.clock;
        match msg {
            ClockMsg::Start => {
                c.start_pending = true;
                c.times.clear();
            }
            ClockMsg::Continue | ClockMsg::Stop => {}
            ClockMsg::Tick => {
                let stale = c.last.is_none_or(|t| now.duration_since(t) > CLOCK_TIMEOUT);
                if c.start_pending {
                    c.start_pending = false;
                    c.base = 0.0;
                    c.ticks = 0;
                } else if stale {
                    // 途切れたあと(または最初)は、今の拍の位置から数え始める(拍が飛ばないように)
                    c.base = self.rt.beat;
                    c.ticks = 0;
                    c.times.clear();
                } else {
                    c.ticks += 1;
                }
                c.last = Some(now);
                c.times.push_back(now);
                // 1拍ぶん(25回)の間隔の平均からBPMを出す
                if c.times.len() > CLOCK_PPQ as usize + 1 {
                    c.times.pop_front();
                }
                if c.times.len() >= 7 {
                    let span = c.times.back().unwrap().duration_since(*c.times.front().unwrap()).as_secs_f64();
                    let per_tick = span / (c.times.len() - 1) as f64;
                    if per_tick > 0.0 {
                        let bpm = ((60.0 / (per_tick * CLOCK_PPQ)) * 10.0).round() / 10.0;
                        self.project.bpm = (bpm as f32).clamp(BPM_MIN, BPM_MAX);
                    }
                }
            }
        }
    }

    /// MIDIクロックから決まる今の拍の位置。クロックが途切れていたらNone
    pub fn clock_beat(&self, now: Instant) -> Option<f64> {
        let c = &self.rt.clock;
        let last = c.last?;
        let since = now.checked_duration_since(last).unwrap_or_default();
        if since > CLOCK_TIMEOUT {
            return None;
        }
        // 次のクロックまでの間は、BPMで補って滑らかに進める(次のクロックの位置は越えない)
        let between = (since.as_secs_f64() * self.project.bpm as f64 / 60.0).min(1.0 / CLOCK_PPQ);
        Some(c.base + c.ticks as f64 / CLOCK_PPQ + between)
    }

    pub fn clock_alive(&self, now: Instant) -> bool {
        self.rt.clock.last.is_some_and(|t| now.checked_duration_since(t).unwrap_or_default() <= CLOCK_TIMEOUT)
    }

    pub fn tap(&mut self) {
        self.tap_at(self.now());
    }

    /// タップテンポ: 直近のタップの間隔の平均からBPMを決め、拍の位置を叩いたタイミング(最も近い拍)に合わせる。
    /// 2秒以上空いたら数え直す
    pub fn tap_at(&mut self, now: Instant) {
        if self.rt.taps.last().is_some_and(|t| now.duration_since(*t) > Duration::from_millis(2000)) {
            self.rt.taps.clear();
        }
        self.rt.taps.push(now);
        if self.rt.taps.len() > 5 {
            self.rt.taps.remove(0);
        }
        if self.rt.taps.len() >= 2 {
            let span = now.duration_since(self.rt.taps[0]).as_secs_f64();
            let avg = span / (self.rt.taps.len() - 1) as f64;
            if avg > 0.0 {
                self.set_bpm((60.0 / avg) as f32);
            }
        }
        let rounded = self.rt.beat.round();
        match self.project.tempo_source {
            TempoSource::Internal => self.rt.beat = rounded,
            TempoSource::Link => {
                if let Some(l) = self.link() {
                    l.force_beat(rounded);
                }
            }
            // 拍の位置はクロックが決める
            TempoSource::MidiClock => {}
            TempoSource::Audio => self.rt.beat = rounded,
        }
    }

    /// 拍の位置を小節の頭に戻す
    pub fn resync(&mut self) {
        if let Some(l) = self.link() {
            l.force_beat(0.0);
        }
        let c = &mut self.rt.clock;
        c.base = -(c.ticks as f64) / CLOCK_PPQ;
        self.rt.beat = 0.0;
        // 速さを変えてずれたLEDバーの模様も、小節の頭にそろえ直す
        self.rt.px_offset = 0.0;
    }

    /// 動作中のチェイスを、拍の位置に合わせて進める。ステップが変わった瞬間にシーンを呼び出す
    pub(super) fn run_chases(&mut self) {
        let bpm = self.project.bpm.max(BPM_MIN);
        let mut todo: Vec<(String, f32)> = Vec::new();
        for c in &self.project.chases {
            if !c.enabled || c.steps.is_empty() {
                // 止めたら状態を忘れる(再開したときに、すぐ最初のステップが出る)
                self.rt.chase_idx.remove(&c.name);
                continue;
            }
            let idx = (self.rt.beat / (c.step_beats as f64).max(0.0625)).floor() as i64;
            if self.rt.chase_idx.get(&c.name) == Some(&idx) {
                continue;
            }
            self.rt.chase_idx.insert(c.name.clone(), idx);
            let step = step_index(c.order, idx, c.steps.len());
            todo.push((c.steps[step].clone(), c.fade_beats * 60.0 / bpm));
        }
        // チェイスによる値の変化は一時的なものなので、自動保存の対象(未保存の変更)にしない
        let was_dirty = self.dirty;
        for (scene, fade) in todo {
            self.recall_scene_with(&scene, Some(fade));
        }
        self.dirty = was_dirty;
    }
}
