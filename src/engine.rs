//! 共有状態(Core)と、DMX値の合成・送出ループ。

use crate::color::{self, ColorRole, ColorTarget, NamedColor, PaletteOp, ScopeOp, SelectOp};
use crate::fixture::{Library, Mode};
use crate::output::{self, Output};
use crate::project::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub type Shared = Arc<Mutex<Core>>;

/// スレッドがpanicしても他が動き続けられるよう、毒化したロックも取り出す
pub fn lock(s: &Shared) -> MutexGuard<'_, Core> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

/// 出力の更新周期(40Hz)
pub const TICK: Duration = Duration::from_millis(25);
/// 出力スロット数の下限(DMX規格上の最小)
const MIN_SLOTS: usize = 24;

pub struct Fade {
    pub fixture: u32,
    pub ch: usize,
    pub from: f32,
    pub to: f32,
    pub start: Instant,
    pub dur: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct MidiMsg {
    pub kind: TriggerKind,
    /// 0..15
    pub channel: u8,
    pub number: u8,
    /// ベロシティ/CC値。ノートオフは0
    pub value: u8,
}

/// MIDIクロック(0xF8)は1拍に24回
pub const CLOCK_PPQ: f64 = 24.0;
/// これ以上クロックが来なければ途切れたとみなし、内部のテンポで走り続ける
pub const CLOCK_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClockMsg {
    Tick,
    Start,
    Continue,
    Stop,
}

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

pub struct Runtime {
    pub blackout: bool,
    pub fades: Vec<Fade>,
    /// バインディングごとの「押されている」状態(エッジ検出用)
    pub pad_state: HashMap<usize, bool>,
    /// トグル動作のバインディングの現在状態
    pub toggle_state: HashMap<usize, bool>,
    /// 全体光量(0.0〜1.0)。ディマーにかかる。保存しない(起動時は全開)
    pub master: f32,
    /// 拍の位置(0から連続して増える。`BEATS_PER_BAR`拍で1小節)
    pub beat: f64,
    pub last_beat_t: Option<Instant>,
    /// タップテンポ用に、直近のタップの時刻
    pub taps: Vec<Instant>,
    /// 動作中のチェイスが今何ステップ目(通し番号)か。ステップ切り替えの検出と、画面の表示に使う
    pub chase_idx: HashMap<String, i64>,
    /// 最後に呼び出したシーン(パッドの点灯表示用)
    pub last_scene: Option<String>,
    pub led_status: String,
    /// 色の操作先: true=全体(全機材) / false=個別(選択中の機材)。保存しない
    pub color_global: bool,
    /// 個別モードで色を掛ける機材(id)
    pub selected: HashSet<u32>,
    /// 最後に選んだ色(パッドの点灯表示用)
    pub last_color: Option<NamedColor>,
    /// 色相フェーダー(HSVのH)の現在値(0〜360度)。画面のスライダーと表示用
    pub hue: f32,
    pub learn: Option<usize>,
    pub last_midi: String,
    pub midi_status: String,
    /// 今押されているノート(マップ表示用。チャンネルは区別しない)
    pub notes_down: [bool; 128],
    /// CCの最後に受け取った値(マップ表示用。まだ動かしていないフェーダーはNone)
    pub cc_value: [Option<u8>; 128],
    pub frames: Vec<[u8; 512]>,
    pub used_len: Vec<usize>,
    pub output_status: Vec<String>,
    /// 出力設定を作り直す合図
    pub output_gen: u64,
    pub t0: Instant,
    /// 今のパッドのページ(0始まり)
    pub page: u8,
    pub link: Option<crate::link::Link>,
    pub link_peers: u64,
    pub clock: ClockIn,
    /// 最後に呼び出した位置プリセット(パッドの点灯表示用)
    pub last_position: Option<String>,
    /// OSCラーン: 次に来たOSCアドレスを、この割り当てに登録する
    pub osc_learn: Option<usize>,
    pub last_osc: String,
    pub osc_status: String,
    /// メイン色とサブ色を入れ替えているか(自動入れ替えとは別。保存しない)
    pub color_swapped: bool,
    /// 最後にメイン/サブの色を機材に出したときの「入れ替え」の状態。まだ一度も出していなければNone
    pub palette_shown: Option<bool>,
    /// フラッシュ中なら Some(白で全開か)
    pub flash: Option<bool>,
    /// 暗転フェードの明るさ(1=通常 / 0=暗転)。全体光量にかかる
    pub dim: f32,
    /// 暗転フェードの向かう先(0か1)と、かける秒数
    pub dim_target: f32,
    pub dim_secs: f32,
    /// 機材ごとの明るさ(0.0〜1.0。「明るさ」のフェーダーで変える)。無い機材は1.0。全体光量と同じところに掛かる。保存しない
    pub level: HashMap<u32, f32>,
    /// ミュート中の機材(明るさのチャンネルを0にする)。保存しない
    pub muted: HashSet<u32>,
    /// ピクセルエフェクト全体の速さの倍率と、倍率を変えても模様が飛ばないようにするための拍のずれ
    pub px_speed: f32,
    pub px_offset: f64,
    /// ピクセルエフェクトに足す斜めのずれ(周)
    pub px_skew: f32,
}

impl Runtime {
    fn new() -> Self {
        Runtime {
            blackout: false,
            fades: Vec::new(),
            pad_state: HashMap::new(),
            toggle_state: HashMap::new(),
            master: 1.0,
            beat: 0.0,
            last_beat_t: None,
            taps: Vec::new(),
            chase_idx: HashMap::new(),
            last_scene: None,
            led_status: String::new(),
            color_global: true,
            selected: HashSet::new(),
            last_color: None,
            hue: 0.0,
            learn: None,
            last_midi: String::new(),
            midi_status: "未接続".into(),
            notes_down: [false; 128],
            cc_value: [None; 128],
            frames: vec![[0; 512]; NUM_UNIVERSES],
            used_len: vec![MIN_SLOTS; NUM_UNIVERSES],
            output_status: Vec::new(),
            output_gen: 0,
            t0: Instant::now(),
            page: 0,
            link: None,
            link_peers: 0,
            clock: ClockIn::default(),
            last_position: None,
            osc_learn: None,
            last_osc: String::new(),
            osc_status: "停止中".into(),
            color_swapped: false,
            palette_shown: None,
            flash: None,
            dim: 1.0,
            dim_target: 1.0,
            dim_secs: 3.0,
            level: HashMap::new(),
            muted: HashSet::new(),
            px_speed: 1.0,
            px_offset: 0.0,
            px_skew: 0.0,
        }
    }
}

pub struct Core {
    pub project: Project,
    pub lib: Arc<Library>,
    pub rt: Runtime,
    /// 保存されていない変更がある
    pub dirty: bool,
}

impl Core {
    pub fn new(project: Project, lib: Arc<Library>) -> Self {
        let mut rt = Runtime::new();
        // 個別モードの初期の選択は、先頭の機材
        if let Some(f) = project.fixtures.first() {
            rt.selected.insert(f.id);
        }
        Core { project, lib, rt, dirty: false }
    }

    pub fn apply_outputs_changed(&mut self) {
        self.rt.output_gen += 1;
        self.dirty = true;
    }

    /// セレクタに該当する機材のインデックス
    pub fn resolve(&self, sel: &Selector) -> Vec<usize> {
        self.project
            .fixtures
            .iter()
            .enumerate()
            .filter(|(_, f)| match sel {
                Selector::All => true,
                Selector::Fixture(id) => f.id == *id,
                Selector::Group(name) => self.project.groups.iter().any(|g| g.name == *name && g.fixtures.contains(&f.id)),
                Selector::Profile(p) => f.profile == *p,
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// 機能名(attr)を持つチャンネルに値を設定する(上位/下位を含む)。LEDバーで「red」なら全ピクセルの赤
    pub fn set_attr(&mut self, sel: &Selector, attr: &str, v: f32) {
        let lib = self.lib.clone();
        for i in self.resolve(sel) {
            let fx = &mut self.project.fixtures[i];
            let Some(mode) = lib.mode(&fx.profile, fx.mode) else { continue };
            if fx.values.len() != mode.channels.len() {
                continue;
            }
            let id = fx.id;
            for ch in mode.attr_channels(attr) {
                mode.set_f(&mut fx.values, ch, v);
                self.rt.fades.retain(|f| !(f.fixture == id && f.ch == ch));
            }
        }
        self.dirty = true;
    }

    // ---- 色の全体制御 ----

    /// 色を掛ける機材(id)。`Scope`は「全体/個別」スイッチに従う。`Main`/`Sub`は、その系統に割り当てた機材
    pub fn color_targets(&self, target: ColorTarget) -> Vec<u32> {
        if let Some(role) = target.slot() {
            return self.project.fixtures.iter().filter(|f| f.color_role == role).map(|f| f.id).collect();
        }
        let all = matches!(target, ColorTarget::All) || (matches!(target, ColorTarget::Scope) && self.rt.color_global);
        self.project.fixtures.iter().filter(|f| all || self.rt.selected.contains(&f.id)).map(|f| f.id).collect()
    }

    // ---- メイン色/サブ色 ----

    /// 今、メインとサブが入れ替わっているか(入れ替えボタン + 拍による自動入れ替え)
    pub fn palette_swapped(&self) -> bool {
        let p = &self.project.palette;
        let auto = p.auto && p.auto_beats > 0.0 && (self.rt.beat / p.auto_beats as f64).floor().rem_euclid(2.0) >= 1.0;
        self.rt.color_swapped ^ auto
    }

    /// この系統の機材に、今出す色
    pub fn role_rgb(&self, role: ColorRole) -> Option<[f32; 3]> {
        let p = &self.project.palette;
        let main_side = match role {
            ColorRole::Main => true,
            ColorRole::Sub => false,
            ColorRole::Off => return None,
        };
        Some(if main_side != self.palette_swapped() { p.main } else { p.sub })
    }

    /// メイン/サブに割り当てた機材に、今の色を出す
    pub fn apply_palette(&mut self) {
        let plan: Vec<(u32, [f32; 3])> = self.project.fixtures.iter().filter_map(|f| self.role_rgb(f.color_role).map(|c| (f.id, c))).collect();
        for (id, c) in plan {
            self.apply_rgb(&[id], c);
        }
        self.rt.palette_shown = Some(self.palette_swapped());
    }

    pub fn set_palette_color(&mut self, slot: ColorRole, rgb: [f32; 3]) {
        match slot {
            ColorRole::Main => self.project.palette.main = rgb,
            ColorRole::Sub => self.project.palette.sub = rgb,
            ColorRole::Off => return,
        }
        self.dirty = true;
        self.apply_palette();
    }

    pub fn set_color_role(&mut self, id: u32, role: ColorRole) {
        if let Some(i) = self.project.fixture_index(id) {
            self.project.fixtures[i].color_role = role;
            self.dirty = true;
            self.apply_palette();
        }
    }

    // ---- 暗転フェード ----

    /// 暗くする(true)/戻す(false)。secs秒かけて変わる(0なら即座に)
    pub fn fade_to_black(&mut self, dark: bool, secs: f32) {
        self.rt.dim_target = if dark { 0.0 } else { 1.0 };
        self.rt.dim_secs = secs.max(0.0);
        if self.rt.dim_secs <= 0.0 {
            self.rt.dim = self.rt.dim_target;
        }
    }

    /// 暗転中(または暗転済み)なら戻す、そうでなければ暗転する
    pub fn toggle_fade(&mut self, secs: f32) {
        let dark = self.rt.dim_target > 0.5;
        self.fade_to_black(dark, secs);
    }

    /// 暗転フェードで明るさが変わっている途中か
    pub fn fading(&self) -> bool {
        (self.rt.dim - self.rt.dim_target).abs() > 1e-4
    }

    pub(crate) fn step_dim(&mut self, dt: f64) {
        if !self.fading() {
            return;
        }
        let step = if self.rt.dim_secs <= 0.0 { 1.0 } else { (dt as f32 / self.rt.dim_secs).max(0.0) };
        self.rt.dim = if self.rt.dim_target < self.rt.dim { (self.rt.dim - step).max(self.rt.dim_target) } else { (self.rt.dim + step).min(self.rt.dim_target) };
    }

    /// メインとサブを入れ替える
    pub fn swap_palette(&mut self) {
        self.rt.color_swapped = !self.rt.color_swapped;
        self.apply_palette();
    }

    pub fn set_palette_auto(&mut self, on: bool) {
        self.project.palette.auto = on;
        self.dirty = true;
        self.apply_palette();
    }

    /// 自動入れ替え中は、拍の区切りで色を出し直す
    fn run_palette_auto(&mut self) {
        if !self.project.palette.auto || self.rt.palette_shown == Some(self.palette_swapped()) {
            return;
        }
        // 拍による切り替えは一時的なものなので、自動保存のきっかけにしない
        let was = self.dirty;
        self.apply_palette();
        self.dirty = was;
    }

    /// 各機材の仕組み(RGB / カラーホイール / 色レンジ)に合わせて、その色に設定する
    pub fn apply_rgb(&mut self, ids: &[u32], rgb: [f32; 3]) {
        let lib = self.lib.clone();
        for &id in ids {
            let Some(i) = self.project.fixture_index(id) else { continue };
            let fx = &self.project.fixtures[i];
            let Some(mode) = lib.mode(&fx.profile, fx.mode) else { continue };
            for (attr, v) in color::plan_for_mode(mode, rgb) {
                self.set_attr(&Selector::Fixture(id), &attr, v);
            }
        }
    }

    pub fn apply_named_color(&mut self, c: NamedColor, target: ColorTarget) {
        if let Some(slot) = target.slot() {
            self.set_palette_color(slot, c.rgb());
            return;
        }
        let ids = self.color_targets(target);
        self.apply_rgb(&ids, c.rgb());
        self.rt.last_color = Some(c);
    }

    /// HSVの色相H(度。0〜360で一周。0=赤 / 60=黄 / 120=緑 / 180=水色 / 240=青 / 300=マゼンタ / 360=赤に戻る)を、
    /// 彩度・明度を最大にした色として、対象の機材に掛ける
    pub fn apply_hue(&mut self, degrees: f32, target: ColorTarget) {
        let h = degrees.clamp(0.0, 360.0);
        if let Some(slot) = target.slot() {
            self.set_palette_color(slot, color::hsv(h, 1.0, 1.0));
            return;
        }
        let ids = self.color_targets(target);
        self.apply_rgb(&ids, color::hsv(h, 1.0, 1.0));
        self.rt.hue = h;
        self.rt.last_color = None;
    }

    pub fn toggle_selected(&mut self, id: u32) {
        if !self.rt.selected.remove(&id) {
            self.rt.selected.insert(id);
        }
    }

    // ---- ペア ----

    /// ペアの連動計画。同じ機能名(attr)のチャンネルどうしを対応させる。
    /// 選択式(レンジ付き)のチャンネルは、機種が違うと値の意味が違うので、同じ機種のときだけ連動する
    pub fn pair_plan(&self, p: &Pair) -> Option<PairPlan> {
        let li = self.project.fixture_index(p.leader)?;
        let fi = self.project.fixture_index(p.follower)?;
        if li == fi {
            return None;
        }
        let (lf, ff) = (&self.project.fixtures[li], &self.project.fixtures[fi]);
        let lm = self.lib.mode(&lf.profile, lf.mode)?;
        let fm = self.lib.mode(&ff.profile, ff.mode)?;
        let same = lf.profile == ff.profile;
        let mut chans = Vec::new();
        for (fch, fdef) in fm.channels.iter().enumerate() {
            if fdef.fine_of.is_some() || p.skip_cats.iter().any(|c| c == fdef.category_name()) {
                continue;
            }
            let Some(lch) = lm.find_attr(&fdef.attr) else { continue };
            if lm.channels[lch].fine_of.is_some() {
                continue;
            }
            if !same && (!fdef.ranges.is_empty() || !lm.channels[lch].ranges.is_empty()) {
                continue;
            }
            chans.push((fch, lch, p.mirror && p.invert.contains(&fdef.attr)));
        }
        Some(PairPlan { li, fi, chans })
    }

    /// コントロール画面用: この機材がペアで連動している場合、連動後の値などを返す
    pub fn paired_view(&self, fixture_id: u32) -> Option<PairView> {
        let fi = self.project.fixture_index(fixture_id)?;
        let fx = &self.project.fixtures[fi];
        let fm = self.lib.mode(&fx.profile, fx.mode)?;
        if fx.values.len() != fm.channels.len() {
            return None;
        }
        let mut mask = vec![false; fm.channels.len()];
        let mut values = fx.values.clone();
        let mut inverted = Vec::new();
        let mut leader = None;
        for p in self.project.pairs.iter().filter(|p| p.enabled && p.follower == fixture_id) {
            let Some(plan) = self.pair_plan(p) else { continue };
            let lf = &self.project.fixtures[plan.li];
            let Some(lm) = self.lib.mode(&lf.profile, lf.mode) else { continue };
            apply_pair(&plan, lm, fm, &lf.values, &mut values);
            for &(fch, _, inv) in &plan.chans {
                mask[fch] = true;
                if inv && !inverted.contains(&fm.channels[fch].attr) {
                    inverted.push(fm.channels[fch].attr.clone());
                }
            }
            leader = Some(lf.name.clone());
        }
        leader.map(|leader| PairView { leader, mask, values, inverted })
    }

    // ---- BPM ----

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
    fn link(&self) -> Option<&crate::link::Link> {
        self.rt.link.as_ref().filter(|_| self.project.tempo_source == TempoSource::Link)
    }

    pub fn set_tempo_source(&mut self, s: TempoSource) {
        if self.project.tempo_source != s {
            self.project.tempo_source = s;
            self.dirty = true;
        }
        self.ensure_link();
    }

    /// 設定に合わせてLinkを開始/終了する
    pub fn ensure_link(&mut self) {
        if self.project.tempo_source == TempoSource::Link {
            if self.rt.link.is_none() {
                self.rt.link = crate::link::Link::start(self.project.bpm as f64);
            }
        } else {
            self.rt.link = None;
            self.rt.link_peers = 0;
        }
    }

    /// 拍を進める。Link/MIDIクロックに合わせているときは、そちらの拍の位置を使う
    fn sync_beat(&mut self, now: Instant, dt: f64) {
        match self.project.tempo_source {
            TempoSource::Link => {
                if let Some(l) = self.rt.link.as_ref() {
                    if let Some((tempo, beat)) = l.state(BEATS_PER_BAR as f64) {
                        self.rt.link_peers = l.peers();
                        self.rt.beat = beat;
                        // 仲間から来たテンポの変更は、保存のきっかけにしない
                        self.project.bpm = (tempo as f32).clamp(BPM_MIN, BPM_MAX);
                        return;
                    }
                }
            }
            TempoSource::MidiClock => {
                if let Some(b) = self.clock_beat(now) {
                    self.rt.beat = b;
                    return;
                }
            }
            TempoSource::Internal => {}
        }
        self.advance_beat(dt);
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
        self.tap_at(Instant::now());
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
                    l.force_beat(rounded, BEATS_PER_BAR as f64);
                }
            }
            // 拍の位置はクロックが決める
            TempoSource::MidiClock => {}
        }
    }

    /// 拍の位置を小節の頭に戻す
    pub fn resync(&mut self) {
        if let Some(l) = self.link() {
            l.force_beat(0.0, BEATS_PER_BAR as f64);
        }
        let c = &mut self.rt.clock;
        c.base = -(c.ticks as f64) / CLOCK_PPQ;
        self.rt.beat = 0.0;
        // 速さを変えてずれたLEDバーの模様も、小節の頭にそろえ直す
        self.rt.px_offset = 0.0;
    }

    /// 動作中のチェイスを、拍の位置に合わせて進める。ステップが変わった瞬間にシーンを呼び出す
    fn run_chases(&mut self) {
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

    /// 配置中の機材にあるカテゴリ(表示順)
    pub fn present_categories(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for fx in &self.project.fixtures {
            if let Some(m) = self.lib.mode(&fx.profile, fx.mode) {
                for c in m.categories() {
                    if !out.contains(&c) {
                        out.push(c);
                    }
                }
            }
        }
        out.sort_by_key(|c| crate::fixture::category_order(c));
        out
    }

    /// 選んだ機材の現在値をシーンにする。`skip_cats`に含まれるカテゴリ(動き/色など)のチャンネルは保存しない
    pub fn capture_scene(&self, name: &str, fade: f32, fixtures: &[u32], skip_cats: &HashSet<String>) -> Scene {
        let mut values = Vec::new();
        for fx in self.project.fixtures.iter().filter(|f| fixtures.contains(&f.id)) {
            let Some(mode) = self.lib.mode(&fx.profile, fx.mode) else { continue };
            if fx.values.len() != mode.channels.len() {
                continue;
            }
            for (ci, ch) in mode.channels.iter().enumerate() {
                if ch.fine_of.is_some() || skip_cats.contains(ch.category_name()) {
                    continue;
                }
                values.push(SceneValue { fixture: fx.id, attr: ch.attr.clone(), value: mode.get_f(&fx.values, ci) });
            }
        }
        Scene { name: name.into(), fade, values }
    }

    /// 既存シーンの各項目を現在の値で更新する。動きだけ等の部分シーンは、部分のまま更新される
    pub fn refresh_scene(&mut self, idx: usize) {
        let Some(scene) = self.project.scenes.get(idx).cloned() else { return };
        let mut values = scene.values;
        for sv in &mut values {
            let Some(fx) = self.project.fixtures.iter().find(|f| f.id == sv.fixture) else { continue };
            let Some(mode) = self.lib.mode(&fx.profile, fx.mode) else { continue };
            let Some(ch) = mode.find_attr(&sv.attr) else { continue };
            if fx.values.len() == mode.channels.len() {
                sv.value = mode.get_f(&fx.values, ch);
            }
        }
        self.project.scenes[idx].values = values;
        self.dirty = true;
    }

    /// 機材定義を差し替える(エディタで保存したとき)。既存の機材の値は機能名(attr)で引き継ぎ、
    /// 増えたチャンネルは初期値にする。モードが無くなっていたら先頭のモードにする
    pub fn replace_library(&mut self, new: Arc<Library>) {
        let old = self.lib.clone();
        for fx in &mut self.project.fixtures {
            let Some(profile) = new.get(&fx.profile) else { continue };
            if fx.mode >= profile.modes.len() {
                fx.mode = 0;
            }
            let Some(new_mode) = profile.modes.get(fx.mode) else { continue };
            let old_mode = old.mode(&fx.profile, fx.mode);
            let mut values = new_mode.default_values();
            if let Some(om) = old_mode {
                if om.channels.len() == fx.values.len() {
                    for (i, ch) in new_mode.channels.iter().enumerate() {
                        if let Some(oi) = om.find_attr(&ch.attr) {
                            values[i] = fx.values[oi];
                        }
                    }
                }
            }
            fx.values = values;
        }
        self.rt.fades.clear();
        self.lib = new;
        self.dirty = true;
    }

    pub fn recall_scene(&mut self, name: &str) {
        self.recall_scene_with(name, None);
    }

    /// `fade_override`(秒)があれば、シーン自身のフェード時間の代わりに使う(チェイスがBPMから決める)
    pub fn recall_scene_with(&mut self, name: &str, fade_override: Option<f32>) {
        let Some(scene) = self.project.scenes.iter().find(|s| s.name == name).cloned() else { return };
        let fade = fade_override.unwrap_or(scene.fade);
        self.rt.last_scene = Some(scene.name.clone());
        self.apply_values(&scene.values, fade);
    }

    // ---- 位置プリセット ----

    /// 今のパン/チルトを位置プリセットにする(パン/チルトを持つ全機材)
    pub fn capture_position(&self, name: &str, fade: f32) -> Scene {
        let mut values = Vec::new();
        for fx in &self.project.fixtures {
            let Some(mode) = self.lib.mode(&fx.profile, fx.mode) else { continue };
            if fx.values.len() != mode.channels.len() {
                continue;
            }
            for attr in POSITION_ATTRS {
                if let Some(ch) = mode.find_attr(attr) {
                    values.push(SceneValue { fixture: fx.id, attr: attr.into(), value: mode.get_f(&fx.values, ch) });
                }
            }
        }
        Scene { name: name.into(), fade, values }
    }

    pub fn recall_position(&mut self, name: &str) {
        let Some(p) = self.project.positions.iter().find(|s| s.name == name).cloned() else { return };
        self.apply_values(&p.values, p.fade);
        self.rt.last_position = Some(p.name);
    }

    /// シーン/位置プリセットの値を、フェード(秒)付きで適用する
    fn apply_values(&mut self, values: &[SceneValue], fade: f32) {
        let lib = self.lib.clone();
        let now = Instant::now();
        for sv in values {
            let Some(i) = self.project.fixture_index(sv.fixture) else { continue };
            let fx = &mut self.project.fixtures[i];
            let Some(mode) = lib.mode(&fx.profile, fx.mode) else { continue };
            let Some(ch) = mode.find_attr(&sv.attr) else { continue };
            if fx.values.len() != mode.channels.len() {
                continue;
            }
            let id = fx.id;
            self.rt.fades.retain(|f| !(f.fixture == id && f.ch == ch));
            let from = mode.get_f(&fx.values, ch);
            // カラー/ゴボ等の離散チャンネルはフェードせず即切り替える(途中の値で別の色が出てしまうため)
            if fade <= 0.0 || !mode.channels[ch].ranges.is_empty() {
                mode.set_f(&mut fx.values, ch, sv.value);
            } else {
                self.rt.fades.push(Fade { fixture: id, ch, from, to: sv.value, start: now, dur: fade });
            }
        }
        self.dirty = true;
    }

    fn advance_fades(&mut self, now: Instant) {
        if self.rt.fades.is_empty() {
            return;
        }
        let lib = self.lib.clone();
        let fades = std::mem::take(&mut self.rt.fades);
        let mut keep = Vec::with_capacity(fades.len());
        for f in fades {
            let Some(i) = self.project.fixture_index(f.fixture) else { continue };
            let fx = &mut self.project.fixtures[i];
            let Some(mode) = lib.mode(&fx.profile, fx.mode) else { continue };
            if fx.values.len() != mode.channels.len() {
                continue;
            }
            let p = if f.dur <= 0.0 { 1.0 } else { ((now - f.start).as_secs_f32() / f.dur).min(1.0) };
            mode.set_f(&mut fx.values, f.ch, f.from + (f.to - f.from) * p);
            if p < 1.0 {
                keep.push(f);
            }
        }
        self.rt.fades = keep;
    }

    /// ベース値 + エフェクト + ブラックアウト/ARMゲートを合成して、ユニバースごとのフレームを作る
    pub fn render(&mut self) {
        let now = Instant::now();
        // 拍を進める(ウィンドウ移動などで止まった直後に、拍が大きく飛ばないよう上限を付ける)
        let dt = self.rt.last_beat_t.map(|t| now.duration_since(t).as_secs_f64().min(0.5)).unwrap_or(0.0);
        self.rt.last_beat_t = Some(now);
        self.sync_beat(now, dt);
        self.run_palette_auto();
        self.step_dim(dt);
        self.run_chases();
        self.advance_fades(now);
        let t = (now - self.rt.t0).as_secs_f64();
        let lib = self.lib.clone();

        let mut vals: Vec<Vec<u8>> = self.project.fixtures.iter().map(|f| f.values.clone()).collect();

        for e in self.project.effects.iter().filter(|e| e.enabled) {
            for (n, i) in self.resolve(&e.sel).into_iter().enumerate() {
                let fx = &self.project.fixtures[i];
                let Some(mode) = lib.mode(&fx.profile, fx.mode) else { continue };
                let Some(info) = lib.info(&fx.profile, fx.mode) else { continue };
                let chans = info.channels(&e.attr);
                if chans.is_empty() || vals[i].len() != mode.channels.len() {
                    continue;
                }
                // 拍に同期するエフェクトは、拍の位置から位相を決める(テンポを変えても位相が飛ばない)
                let base = if e.beats > 0.0 { self.rt.beat / e.beats as f64 } else { t * e.rate_hz as f64 };
                let p = base + (e.phase as f64 + e.spread as f64 * n as f64) / 360.0;
                let cycle = p.floor();
                let frac = (p - cycle) as f32;
                let w = e.wave.eval(frac, cycle as i64 as u64, fx.id.wrapping_mul(31).wrapping_add(n as u32));
                for &ch in chans {
                    info.set_f(&mut vals[i], ch, e.min + (e.max - e.min) * w);
                }
            }
        }

        // ピクセルエフェクト(LEDバー)
        self.render_pixel_fx(&mut vals);

        // ペア: フォロワーの値を、(エフェクト適用後の)リーダーの値から作る。反転もここで行う
        for p in self.project.pairs.iter().filter(|p| p.enabled) {
            let Some(plan) = self.pair_plan(p) else { continue };
            let (lf, ff) = (&self.project.fixtures[plan.li], &self.project.fixtures[plan.fi]);
            let (Some(lm), Some(fm)) = (lib.mode(&lf.profile, lf.mode), lib.mode(&ff.profile, ff.mode)) else { continue };
            let lvals = vals[plan.li].clone();
            apply_pair(&plan, lm, fm, &lvals, &mut vals[plan.fi]);
        }

        // 全体光量: 各機材のディマー(無ければ色の強さのチャンネル)に掛ける
        // 暗転フェードと機材ごとの明るさ(フェーダー)も、全体光量と同じところにかける
        let master = self.rt.master * self.rt.dim;
        if master < 0.9999 || !self.rt.level.is_empty() {
            for (i, fx) in self.project.fixtures.iter().enumerate() {
                let master = master * self.rt.level.get(&fx.id).copied().unwrap_or(1.0);
                if master >= 0.9999 {
                    continue;
                }
                let Some(mode) = lib.mode(&fx.profile, fx.mode) else { continue };
                if vals[i].len() != mode.channels.len() {
                    continue;
                }
                let Some(info) = lib.info(&fx.profile, fx.mode) else { continue };
                for &ch in &info.master {
                    let v = info.get_f(&vals[i], ch);
                    info.set_f(&mut vals[i], ch, v * master);
                }
            }
        }

        // フラッシュ: 全開にする(全体光量より優先。ブラックアウトとARMは下で優先される)。レーザーなどARMが必要な機材には掛けない
        if let Some(white) = self.rt.flash {
            for (i, fx) in self.project.fixtures.iter().enumerate() {
                let Some(profile) = lib.get(&fx.profile) else { continue };
                let (Some(mode), Some(info)) = (profile.modes.get(fx.mode), lib.info(&fx.profile, fx.mode)) else { continue };
                if profile.needs_arm || vals[i].len() != mode.channels.len() {
                    continue;
                }
                flash_values(mode, &info, &mut vals[i], white);
            }
        }

        for f in self.rt.frames.iter_mut() {
            *f = [0; 512];
        }
        let mut used = vec![0usize; NUM_UNIVERSES];

        for (i, fx) in self.project.fixtures.iter().enumerate() {
            let Some(profile) = lib.get(&fx.profile) else { continue };
            let Some(mode) = profile.modes.get(fx.mode) else { continue };
            let v = &mut vals[i];
            if v.len() != mode.channels.len() || fx.universe >= NUM_UNIVERSES || fx.address == 0 {
                continue;
            }
            // 暗転しきったら、ブラックアウトと同じ扱い(レーザーなど明るさのチャンネルが無い機材も止める)
            let dark = self.rt.blackout || self.rt.dim <= 0.001;
            let gated = dark || (profile.needs_arm && !fx.armed);
            if gated {
                for (c, ch) in mode.channels.iter().enumerate() {
                    if let Some(s) = ch.safe {
                        v[c] = s;
                    }
                }
            }
            // ブラックアウトでは、安全値の無い機材(RGBの灯具など)も、明るさのチャンネルを0にする。ミュート中の機材も同じ
            if dark || self.rt.muted.contains(&fx.id) {
                if let Some(info) = lib.info(&fx.profile, fx.mode) {
                    for &ch in &info.master {
                        info.set_f(v, ch, 0.0);
                    }
                }
            }
            let base = fx.address as usize - 1;
            for (c, &b) in v.iter().enumerate() {
                let a = base + c;
                if a < 512 {
                    self.rt.frames[fx.universe][a] = b;
                    used[fx.universe] = used[fx.universe].max(a + 1);
                }
            }
        }
        for (u, n) in used.into_iter().enumerate() {
            self.rt.used_len[u] = n.clamp(MIN_SLOTS, 512);
        }
    }

    // ---- MIDI ----

    /// APC mini mk2 から読み取ったフェーダー1..9の位置を表示用に記録する。
    /// 動作(割り当て)は実行しない: 接続した瞬間に機材が動いたり、レーザーが出たりしないようにするため
    pub fn set_fader_positions(&mut self, positions: &[u8; 9]) {
        for (i, &v) in positions.iter().enumerate() {
            self.rt.cc_value[0x30 + i] = Some(v & 0x7F);
        }
        // 全体光量だけは、実機のフェーダー位置に合わせる(位置と出力がずれたままにならないように)
        let mut master = None;
        for b in &self.project.bindings {
            if let (Action::Master { mode: PadMode::Continuous, on, off }, Some(t)) = (&b.action, b.trigger) {
                if t.kind == TriggerKind::Cc && (0x30..=0x38).contains(&t.number) && t.channel.is_none_or(|c| c == 0) {
                    let v = positions[(t.number - 0x30) as usize] as f32;
                    master = Some(((off + (on - off) * v / 127.0) / 100.0).clamp(0.0, 1.0));
                }
            }
        }
        if let Some(m) = master {
            self.rt.master = m;
        }
        // 機材ごとの明るさとLEDバーの速さ・斜めも、全体光量と同じく実機のフェーダー位置に合わせる
        let sync: Vec<(usize, f32)> = self
            .project
            .bindings
            .iter()
            .enumerate()
            .filter(|(_, b)| matches!(b.action, Action::Level { .. } | Action::Pixel { .. }))
            .filter_map(|(i, b)| {
                let t = b.trigger?;
                (t.kind == TriggerKind::Cc && (0x30..=0x38).contains(&t.number) && t.channel.is_none_or(|c| c == 0)).then(|| (i, positions[(t.number - 0x30) as usize] as f32 / 127.0))
            })
            .collect();
        for (i, v) in sync {
            self.apply_binding(i, v, false);
        }
    }

    pub fn handle_midi(&mut self, m: MidiMsg) {
        let k = match m.kind {
            TriggerKind::Note => "Note",
            TriggerKind::Cc => "CC",
        };
        self.rt.last_midi = format!("{k} {} = {} (ch{})", m.number, m.value, m.channel + 1);
        let n = (m.number & 0x7F) as usize;
        match m.kind {
            TriggerKind::Note => self.rt.notes_down[n] = m.value > 0,
            TriggerKind::Cc => self.rt.cc_value[n] = Some(m.value),
        }

        // MIDIラーン: 次に来たノートオン/CCをトリガとして登録
        if let Some(idx) = self.rt.learn {
            if m.kind == TriggerKind::Cc || m.value > 0 {
                if let Some(b) = self.project.bindings.get_mut(idx) {
                    b.trigger = Some(MidiTrigger { kind: m.kind, channel: Some(m.channel), number: m.number });
                }
                self.rt.learn = None;
                self.dirty = true;
            }
            return;
        }

        // 今のページの割り当てと全ページ共通の割り当てに反応する。
        // ページを切り替える前に押したパッドは、離したときに元の割り当てへ届ける(押している間の動作が戻らなくならないように)
        let page = self.rt.page;
        let hits: Vec<usize> = self
            .project
            .bindings
            .iter()
            .enumerate()
            .filter(|(i, b)| {
                b.trigger.is_some_and(|t| t.kind == m.kind && t.number == m.number && t.channel.is_none_or(|c| c == m.channel))
                    && (b.on_page(page) || (m.kind == TriggerKind::Note && self.rt.pad_state.get(i) == Some(&true)))
            })
            .map(|(i, _)| i)
            .collect();
        let pressed = match m.kind {
            TriggerKind::Note => m.value > 0,
            TriggerKind::Cc => m.value >= 64,
        };
        for i in hits {
            self.apply_binding(i, m.value as f32 / 127.0, pressed);
        }
    }

    /// 割り当てを実行する。`v`=入力値(0.0〜1.0。フェーダー/ノブ用)、`pressed`=押されているか(ボタン用)
    pub fn apply_binding(&mut self, idx: usize, v: f32, pressed: bool) {
        let Some(action) = self.project.bindings.get(idx).map(|b| b.action.clone()) else { return };
        let v = v.clamp(0.0, 1.0);
        let was = self.rt.pad_state.insert(idx, pressed).unwrap_or(false);
        let rising = pressed && !was;
        let falling = !pressed && was;

        match action {
            Action::SetValue { sel, attr, mode, on, off } => match mode {
                PadMode::Continuous => self.set_attr(&sel, &attr, off + (on - off) * v),
                PadMode::Momentary => {
                    if rising {
                        self.set_attr(&sel, &attr, on);
                    } else if falling {
                        self.set_attr(&sel, &attr, off);
                    }
                }
                PadMode::Toggle => {
                    if rising {
                        let s = self.rt.toggle_state.entry(idx).or_insert(false);
                        *s = !*s;
                        let v = if *s { on } else { off };
                        self.set_attr(&sel, &attr, v);
                    }
                }
                PadMode::Trigger => {
                    if rising {
                        self.set_attr(&sel, &attr, on);
                    }
                }
            },
            Action::Scene { name } => {
                if rising {
                    self.recall_scene(&name);
                }
            }
            Action::Effect { name, mode } => {
                // 通常のエフェクトとピクセルエフェクトは、同じ名前ならまとめてON/OFF
                let cur = self.project.effects.iter().find(|e| e.name == name).map(|e| e.enabled).or_else(|| self.project.pixel_fx.iter().find(|e| e.name == name).map(|e| e.enabled));
                let new = match (mode, cur) {
                    (_, None) => None,
                    (PadMode::Toggle, Some(c)) => rising.then_some(!c),
                    (PadMode::Trigger, Some(_)) => rising.then_some(true),
                    (_, Some(_)) => (rising || falling).then_some(pressed),
                };
                if let Some(en) = new {
                    for e in self.project.effects.iter_mut().filter(|e| e.name == name) {
                        e.enabled = en;
                    }
                    if self.project.pixel_fx.iter().any(|e| e.name == name) {
                        self.set_pixel_fx(&name, en);
                    }
                    self.dirty = true;
                }
            }
            Action::Master { mode, on, off } => {
                let cur = self.rt.master;
                let new = match mode {
                    PadMode::Continuous => Some((off + (on - off) * v) / 100.0),
                    PadMode::Momentary => {
                        if rising {
                            Some(on / 100.0)
                        } else if falling {
                            Some(off / 100.0)
                        } else {
                            None
                        }
                    }
                    PadMode::Toggle => rising.then(|| if (cur - on / 100.0).abs() < 0.005 { off / 100.0 } else { on / 100.0 }),
                    PadMode::Trigger => rising.then(|| on / 100.0),
                };
                if let Some(v) = new {
                    self.rt.master = v.clamp(0.0, 1.0);
                }
            }
            Action::Color { color, target } => match color {
                Some(c) => {
                    if rising {
                        self.apply_named_color(c, target);
                    }
                }
                // フェーダーの全行程で色相環を一周する(下端=赤 → 上端=赤に戻る)
                None => self.apply_hue(v * 360.0, target),
            },
            Action::ColorScope { op } => match op {
                ScopeOp::Toggle => {
                    if rising {
                        self.rt.color_global = !self.rt.color_global;
                    }
                }
                ScopeOp::HoldIndividual => {
                    if rising || falling {
                        self.rt.color_global = !pressed;
                    }
                }
                ScopeOp::Global => {
                    if rising {
                        self.rt.color_global = true;
                    }
                }
                ScopeOp::Individual => {
                    if rising {
                        self.rt.color_global = false;
                    }
                }
            },
            Action::Select { fixture, op } => {
                if rising {
                    match op {
                        SelectOp::Only => {
                            self.rt.selected.clear();
                            self.rt.selected.insert(fixture);
                        }
                        SelectOp::Toggle => self.toggle_selected(fixture),
                    }
                }
            }
            Action::Pair { name, op, mode } => {
                let cur = self.project.pairs.iter().find(|p| p.name == name).map(|p| if op == PairOp::Link { p.enabled } else { p.mirror });
                let new = match (mode, cur) {
                    (_, None) => None,
                    (PadMode::Toggle, Some(c)) => rising.then_some(!c),
                    (PadMode::Trigger, Some(_)) => rising.then_some(true),
                    (_, Some(_)) => (rising || falling).then_some(pressed),
                };
                if let Some(v) = new {
                    for p in self.project.pairs.iter_mut().filter(|p| p.name == name) {
                        if op == PairOp::Link {
                            p.enabled = v;
                        } else {
                            p.mirror = v;
                        }
                    }
                    self.dirty = true;
                }
            }
            Action::Tempo { op } => match op {
                TempoOp::Tap => {
                    if rising {
                        self.tap();
                    }
                }
                TempoOp::Resync => {
                    if rising {
                        self.resync();
                    }
                }
                TempoOp::Set { min, max } => self.set_bpm(min + (max - min) * v),
                TempoOp::Nudge { delta } => {
                    if rising {
                        self.set_bpm(self.project.bpm + delta);
                    }
                }
            },
            Action::Chase { name, mode } => {
                let cur = self.project.chases.iter().find(|c| c.name == name).map(|c| c.enabled);
                let new = match (mode, cur) {
                    (_, None) => None,
                    (PadMode::Toggle, Some(c)) => rising.then_some(!c),
                    (PadMode::Trigger, Some(_)) => rising.then_some(true),
                    (_, Some(_)) => (rising || falling).then_some(pressed),
                };
                if let Some(en) = new {
                    for c in self.project.chases.iter_mut().filter(|c| c.name == name) {
                        c.enabled = en;
                    }
                    self.dirty = true;
                }
            }
            Action::Position { name } => {
                if rising {
                    self.recall_position(&name);
                }
            }
            Action::Fade { secs, mode } => match mode {
                PadMode::Toggle => {
                    if rising {
                        self.toggle_fade(secs);
                    }
                }
                PadMode::Trigger => {
                    if rising {
                        self.fade_to_black(true, secs);
                    }
                }
                // 押している間だけ暗転
                _ => {
                    if rising || falling {
                        self.fade_to_black(pressed, secs);
                    }
                }
            },
            Action::Flash { white, mode } => {
                let new = match mode {
                    PadMode::Toggle => rising.then(|| if self.rt.flash.is_some() { None } else { Some(white) }),
                    PadMode::Trigger => rising.then_some(Some(white)),
                    _ => (rising || falling).then(|| pressed.then_some(white)),
                };
                if let Some(f) = new {
                    self.rt.flash = f;
                }
            }
            Action::Palette { op } => match op {
                PaletteOp::Swap => {
                    if rising {
                        self.swap_palette();
                    }
                }
                // 押したときに入れ替え、離したときに戻す
                PaletteOp::HoldSwap => {
                    if rising || falling {
                        self.swap_palette();
                    }
                }
                PaletteOp::Auto => {
                    if rising {
                        self.set_palette_auto(!self.project.palette.auto);
                    }
                }
            },
            Action::Page { op } => {
                if rising {
                    self.rt.page = match op {
                        PageOp::Go(p) => p.min(PAGES - 1),
                        PageOp::Next => (self.rt.page + 1) % PAGES,
                        PageOp::Prev => (self.rt.page + PAGES - 1) % PAGES,
                    };
                }
            }
            Action::Level { sel } => {
                for i in self.resolve(&sel) {
                    let id = self.project.fixtures[i].id;
                    self.rt.level.insert(id, v);
                }
            }
            Action::Mute { sel, mode } => {
                let ids: Vec<u32> = self.resolve(&sel).into_iter().map(|i| self.project.fixtures[i].id).collect();
                let all = !ids.is_empty() && ids.iter().all(|id| self.rt.muted.contains(id));
                let new = match mode {
                    PadMode::Toggle => rising.then_some(!all),
                    PadMode::Trigger => rising.then_some(true),
                    _ => (rising || falling).then_some(pressed),
                };
                if let Some(m) = new {
                    for id in ids {
                        if m {
                            self.rt.muted.insert(id);
                        } else {
                            self.rt.muted.remove(&id);
                        }
                    }
                }
            }
            Action::Pixel { param } => match param {
                crate::pixel::PixelParam::Speed => self.set_pixel_speed(crate::pixel::speed_step(v)),
                crate::pixel::PixelParam::Skew => self.rt.px_skew = v * crate::pixel::SKEW_FADER_MAX,
            },
            Action::Blackout { mode } => {
                let cur = self.rt.blackout;
                let new = match mode {
                    PadMode::Toggle => rising.then_some(!cur),
                    PadMode::Trigger => rising.then_some(true),
                    _ => (rising || falling).then_some(pressed),
                };
                if let Some(b) = new {
                    self.rt.blackout = b;
                }
            }
        }
    }
}

/// ペアの連動計画: (フォロワーのチャンネル, リーダーのチャンネル, 反転するか)
pub struct PairPlan {
    pub li: usize,
    pub fi: usize,
    pub chans: Vec<(usize, usize, bool)>,
}

/// 連動を適用する: リーダーの値(`lvals`)を、計画に従ってフォロワーの値(`fvals`)へ写す
pub fn apply_pair(plan: &PairPlan, lm: &Mode, fm: &Mode, lvals: &[u8], fvals: &mut [u8]) {
    if lvals.len() != lm.channels.len() || fvals.len() != fm.channels.len() {
        return;
    }
    for &(fch, lch, inv) in &plan.chans {
        let mut v = lm.get_f(lvals, lch);
        if inv {
            v = lm.invert_f(lch, v);
        }
        fm.set_f(fvals, fch, v);
    }
}

/// フラッシュの値にする。white=白で全開 / false=今の色のまま全開(色が無ければ白)
fn flash_values(mode: &Mode, info: &crate::fixture::ModeInfo, v: &mut [u8], white: bool) {
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

/// 画面表示用: ペアに連動している機材の、連動後の値と、連動しているチャンネル
pub struct PairView {
    pub leader: String,
    pub mask: Vec<bool>,
    pub values: Vec<u8>,
    pub inverted: Vec<String>,
}

/// 出力スレッド。40Hzでフレームを合成して各出力へ送る
pub struct Engine {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn spawn(core: Shared) -> Engine {
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = std::thread::Builder::new()
            .name("dmx-engine".into())
            .spawn(move || run(core, s2))
            .expect("engine thread");
        Engine { stop, handle: Some(handle) }
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn run(core: Shared, stop: Arc<AtomicBool>) {
    let mut cfgs: Vec<OutputCfg> = Vec::new();
    let mut outs: Vec<Box<dyn Output>> = Vec::new();
    let mut gen_seen = u64::MAX;
    let mut next = Instant::now();

    while !stop.load(Ordering::SeqCst) {
        let (frames, lens, cfg_now, gen_now) = {
            let mut c = lock(&core);
            c.render();
            (c.rt.frames.clone(), c.rt.used_len.clone(), c.project.outputs.clone(), c.rt.output_gen)
        };
        if gen_now != gen_seen {
            // 先に古い出力を閉じてからポートを開き直す
            outs.clear();
            outs = cfg_now.iter().map(output::create).collect();
            cfgs = cfg_now;
            gen_seen = gen_now;
        }
        for (cfg, o) in cfgs.iter().zip(outs.iter_mut()) {
            if !cfg.enabled {
                continue;
            }
            if let (Some(f), Some(&len)) = (frames.get(cfg.universe), lens.get(cfg.universe)) {
                o.send(f, len);
            }
        }
        let statuses: Vec<String> = outs.iter().map(|o| o.status()).collect();
        lock(&core).rt.output_status = statuses;

        next += TICK;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else {
            next = now;
        }
    }
}
