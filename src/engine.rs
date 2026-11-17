//! 共有状態(Core)と、DMX値の合成・送出ループ。
//!
//! `Core`の操作は分野ごとに子モジュールに分けてある:
//! - `palette`: 色の全体制御、メイン色/サブ色
//! - `pair`: ペアの連動
//! - `tempo`: BPM・Link・MIDIクロック・音声検出・チェイス
//! - `scene`: シーン・位置プリセット・フェード
//! - `binding`: MIDIの受信と割り当ての実行

mod auto;
mod binding;
mod motion;
mod pair;
mod palette;
mod scene;
mod tempo;

pub use pair::{apply_pair, PairView};
pub use auto::BltState;
pub use scene::Fade;
pub use tempo::{ClockIn, ClockMsg};

use crate::color::{self, ColorRole, ColorTarget, NamedColor, PaletteOp, ScopeOp, SelectOp};
use crate::fixture::{Library, Mode};
use crate::output::{self, Output};
use crate::project::*;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
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

#[derive(Debug, Clone, Copy)]
pub struct MidiMsg {
    pub kind: TriggerKind,
    /// 0..15
    pub channel: u8,
    pub number: u8,
    /// ベロシティ/CC値。ノートオフは0
    pub value: u8,
}

pub struct Runtime {
    pub(crate) audit_depth: usize,
    pub trace: Option<crate::diagnostics::Trace>,
    pub trace_ticket: u64,
    pub revision: u64,
    pub revision_hash: Option<u64>,
    pub session: String,
    pub physical_output_enabled: bool,
    pub project_path: Option<std::path::PathBuf>,
    pub remote_holds: HashMap<u64, crate::controls::Hold>,
    pub frame_id: u64,
    /// Deadline for remote momentary controls; enforced by the output thread.
    pub flash_deadline: Option<Instant>,
    /// Automation can supply a monotonic clock without changing the GUI clock.
    pub virtual_now: Option<Instant>,
    pub recorder: Option<crate::recording::Recorder>,
    pub safety_events: Vec<serde_json::Value>,
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
    pub audio_status: String,
    /// 音声検出の確からしさ(0〜1)と、最後に採用した時刻
    pub audio_conf: f32,
    pub audio_pending: f32,
    pub audio_seen: Option<Instant>,
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
    /// NDIの受信結果(映像の平均色・音の色)
    pub ndi: crate::ndi::NdiState,
    /// NDIから作った、今のメイン色/サブ色(NDIを使っていなければNone)。保存しない
    pub live_main: Option<[f32; 3]>,
    pub live_sub: Option<[f32; 3]>,
    /// OSCのRGB(/dmx/main/rgb・/dmx/sub/rgb)で受けた色。[メイン, サブ]。保存しない
    pub osc_rgb: [Option<OscRgb>; 2],
    /// OSCで受けた明るさ(/dmx/level。波形の大きさなど)。全体光量に掛ける。保存しない
    pub osc_level: Option<OscGlide<1>>,
    /// オート: 最後に曲の情報(色・明るさ・フレーズ)が届いた時刻
    pub auto_last: Option<Instant>,
    /// オート: 前のフレームで、つながっていたか
    pub auto_was_online: bool,
    /// オート: 待機の見た目(薄い水色)の混ぜ具合(0=本番〜1=待機)
    pub idle_mix: f32,
    /// オート: 最後にMIDIコントローラー・キーボードで操作した時刻(手動優先)
    pub manual_last: Option<Instant>,
    /// オート: 今使っている組み込みの動き(フレーズの名前)
    pub auto_key: String,
    /// オート: 最後にフレーズが届いたときの拍(フレーズの情報が来ない曲かどうか)
    pub phrase_beat: Option<f64>,
    /// オート: 盛り上がり(OSCの明るさを数秒でならしたもの。0〜1)と、最後に見た16拍のかたまり
    pub energy: f32,
    pub energy_block: i64,
    /// オート: 最後に受けた明るさ(盛り上がりの元。手動優先の間も覚えておく)
    pub energy_input: Option<f32>,
    /// オート: サビに入った瞬間の全開を、この拍で戻す
    pub accent_until: Option<f64>,
    /// オート: プリセットの明るさ(全体光量に掛ける)
    pub auto_bright: Option<OscGlide<1>>,
    /// オート: プリセットで「波形の色を使わない」にしているフレーズの間
    pub wave_color_off: bool,
    /// オート: プリセットでONにしたエフェクト(次のフレーズでOFFに戻すため)
    pub auto_fx: Vec<String>,
    /// オート: 最後に Beat Link Trigger の気配(Linkの仲間がいる・曲の情報が届いた)があった時刻。起動時は起動した時刻
    pub blt_seen: Instant,
    /// 最後に受けたフレーズ(/dmx/phrase。Beat Link Triggerなどから)
    pub phrase: String,
    /// ムービングの動きの位置(1で1周)と、それを進めるための前回の拍
    pub motion_phase: f64,
    pub motion_beat: Option<f64>,
    /// ムービングごとの、前回のPanの角度(度)。次の向きの選び方に使う
    pub motion_pan: HashMap<u32, f64>,
    /// キーボードで押しているキー(crate::keys の番号)
    pub keys_down: Vec<u8>,
    /// キーボードの数字の段に出す、LEDバーの模様のセット
    pub key_page: u8,
    /// サーバーにつないだクライアント(`dmxctl connect`)では、操作をサーバーへ送る口。
    /// このときの Core はサーバーの写しで、出力ループも外部との連携も動かさない
    pub remote: Option<crate::remote::Link>,
    /// クライアントでの表示用: サーバーで実行中のフェードの数・Ableton Linkが動いているか
    pub remote_fades: usize,
    pub remote_link: bool,
}

impl Runtime {
    fn new() -> Self {
        Runtime {
            audit_depth: 0,
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
            frame_id: 0,
            trace: None,
            trace_ticket: 0,
            revision: 0,
            revision_hash: None,
            session: format!("{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()),
            physical_output_enabled: false,
            project_path: None,
            remote_holds: HashMap::new(),
            remote: None,
            remote_fades: 0,
            remote_link: false,
            flash_deadline: None,
            virtual_now: None,
            recorder: None,
            safety_events: Vec::new(),
            t0: Instant::now(),
            page: 0,
            link: None,
            link_peers: 0,
            clock: ClockIn::default(),
            last_position: None,
            osc_learn: None,
            last_osc: String::new(),
            osc_status: "停止中".into(),
            audio_status: "停止中".into(),
            audio_conf: 0.0,
            audio_pending: 0.0,
            audio_seen: None,
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
            ndi: crate::ndi::NdiState::default(),
            live_main: None,
            live_sub: None,
            osc_rgb: [None, None],
            osc_level: None,
            auto_last: None,
            auto_was_online: false,
            idle_mix: 0.0,
            manual_last: None,
            auto_key: String::new(),
            phrase_beat: None,
            energy: 0.0,
            energy_block: i64::MIN,
            energy_input: None,
            accent_until: None,
            auto_bright: None,
            wave_color_off: false,
            auto_fx: Vec::new(),
            blt_seen: Instant::now(),
            phrase: String::new(),
            motion_phase: 0.0,
            motion_beat: None,
            motion_pan: HashMap::new(),
            keys_down: Vec::new(),
            key_page: 0,
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
    pub fn now(&self) -> Instant {
        self.rt.virtual_now.unwrap_or_else(Instant::now)
    }

    /// 手元の操作(MIDI・OSC・操作パネル)でフラッシュを変える。
    /// リモートのフラッシュの期限が残っていても、人が押しているフラッシュをその期限で切らないよう、期限を消す
    /// (リモートの経路は、このあとで期限を付け直す)
    pub fn set_flash(&mut self, flash: Option<bool>) {
        self.rt.flash = flash;
        self.rt.flash_deadline = None;
    }

    pub fn new(mut project: Project, lib: Arc<Library>) -> Self {
        project.ensure_binding_ids();
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
            // 使わない機材は、操作・エフェクトの対象にしない
            .filter(|(_, f)| f.enabled)
            .filter(|(_, f)| match sel {
                Selector::All => true,
                Selector::Fixture(id) => f.id == *id,
                Selector::Group(name) => self.project.groups.iter().any(|g| g.name == *name && g.fixtures.contains(&f.id)),
                Selector::Profile(p) => f.profile == *p,
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// セレクタに該当する機材のid
    pub fn resolve_ids(&self, sel: &Selector) -> Vec<u32> {
        self.resolve(sel).into_iter().map(|i| self.project.fixtures[i].id).collect()
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
            if let Some(om) = old_mode
                && om.channels.len() == fx.values.len() {
                for (i, ch) in new_mode.channels.iter().enumerate() {
                    if let Some(oi) = om.find_attr(&ch.attr) {
                        values[i] = fx.values[oi];
                    }
                }
            }
            fx.values = values;
        }
        self.rt.fades.clear();
        self.lib = new;
        self.dirty = true;
    }

    /// ベース値 + エフェクト + ブラックアウト/ARMゲートを合成して、ユニバースごとのフレームを作る
    pub fn render(&mut self) {
        let now = self.now();
        self.rt.safety_events.clear();
        self.expire_remote_controls(now);
        if self.rt.flash_deadline.is_some_and(|deadline|now >= deadline) {
            self.rt.flash = None;
            self.rt.flash_deadline = None;
            if let Some(recorder) = self.rt.recorder.as_ref() {
                recorder.emit(serde_json::json!({"type":"command","source":"lease_expired",
                    "command":{"cmd":"flash","on":false},"applies_from_frame":recorder.next_frame(),
                    "applies_from_engine_frame":self.rt.frame_id+1,
                    "t_ms":now.saturating_duration_since(self.rt.t0).as_millis() as u64}));
            }
        }
        self.trace_begin();
        // 拍を進める(ウィンドウ移動などで止まった直後に、拍が大きく飛ばないよう上限を付ける)
        let dt = self.rt.last_beat_t.map(|t| now.duration_since(t).as_secs_f64().min(0.5)).unwrap_or(0.0);
        self.rt.last_beat_t = Some(now);
        self.sync_beat(now, dt);
        self.run_auto(dt);
        self.run_palette_auto();
        self.run_live_colors();
        self.step_dim(dt);
        self.run_chases();
        self.advance_fades(now);
        self.trace_stage("automation_and_fades",None,serde_json::Value::Null);
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

        if self.rt.trace.as_ref().is_some_and(|trace|trace.result.is_none()) {
            let id=self.rt.trace.as_ref().unwrap().fixture;
            let effects:Vec<_>=self.project.effects.iter().filter(|e|e.enabled && self.resolve_ids(&e.sel).contains(&id)).collect();
            let details=serde_json::json!({"effects":effects});
            self.trace_stage("effects",Some(&vals),details);
        }
        // ムービングの動き(形・1周の長さ・揃え方・向き)
        self.render_motion(&mut vals);
        self.trace_stage("motion",Some(&vals),serde_json::Value::Null);

        // ピクセルエフェクト(LEDバー)
        self.render_pixel_fx(&mut vals);
        self.trace_stage("pixel",Some(&vals),serde_json::Value::Null);

        // ペア: フォロワーの値を、(エフェクト適用後の)リーダーの値から作る。反転もここで行う
        for p in self.project.pairs.iter().filter(|p| p.enabled) {
            let Some(plan) = self.pair_plan(p) else { continue };
            let (lf, ff) = (&self.project.fixtures[plan.li], &self.project.fixtures[plan.fi]);
            let (Some(lm), Some(fm)) = (lib.mode(&lf.profile, lf.mode), lib.mode(&ff.profile, ff.mode)) else { continue };
            let lvals = vals[plan.li].clone();
            apply_pair(&plan, lm, fm, &lvals, &mut vals[plan.fi]);
        }

        self.trace_stage("pairs",Some(&vals),serde_json::Value::Null);
        // 全体光量: 各機材のディマー(無ければ色の強さのチャンネル)に掛ける
        // 暗転フェードと機材ごとの明るさ(フェーダー)も、全体光量と同じところにかける
        // OSCで受けた明るさ(波形の大きさなど)も同じところにかける。効き具合(depth)が0なら効かない
        let level = self.rt.osc_level.as_mut().map(|o| o.step(now)[0]).map(|l| 1.0 - self.project.osc_level_depth.clamp(0.0, 1.0) * (1.0 - l.clamp(0.0, 1.0))).unwrap_or(1.0);
        // オートのプリセットの明るさ(曲の情報が届いている間)
        let bright = if self.project.auto.enabled && self.rt.auto_was_online { self.rt.auto_bright.as_mut().map(|o| o.step(now)[0]).unwrap_or(1.0) } else { 1.0 };
        let master = self.rt.master * self.rt.dim * level * bright;
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

        if self.rt.trace.as_ref().is_some_and(|t|t.result.is_none()) {
            let details=serde_json::json!({"master":self.rt.master,"dim":self.rt.dim,"osc_level":level,"auto_brightness":bright,"fixture_levels":self.rt.level});
            self.trace_stage("master",Some(&vals),details);
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

        self.trace_stage("flash",Some(&vals),serde_json::Value::Null);
        // オートで曲の情報が届いていないときは、薄い水色で待つ(ブラックアウトとARMは下で優先される)
        self.apply_idle(&mut vals);
        self.trace_stage("idle",Some(&vals),serde_json::Value::Null);

        for f in self.rt.frames.iter_mut() {
            *f = [0; 512];
        }
        let mut used = vec![0usize; NUM_UNIVERSES];

        for (i, fx) in self.project.fixtures.iter().enumerate() {
            // 使わない機材は出力しない(0のまま)
            if !fx.enabled {
                continue;
            }
            let Some(profile) = lib.get(&fx.profile) else { continue };
            let Some(mode) = profile.modes.get(fx.mode) else { continue };
            let v = &mut vals[i];
            let requested = (self.rt.recorder.is_some() || self.rt.trace.as_ref().is_some_and(|t|t.fixture==fx.id && t.result.is_none())).then(||v.clone());
            if v.len() != mode.channels.len() || fx.universe >= NUM_UNIVERSES || fx.address == 0 {
                continue;
            }
            // 暗転しきったら、ブラックアウトと同じ扱い(レーザーなど明るさのチャンネルが無い機材も止める)
            let dark = self.rt.blackout || self.rt.dim <= 0.001;
            // ミュート中も安全値を出す(明るさのチャンネルが無いレーザーも消えるように)
            let muted = self.rt.muted.contains(&fx.id);
            if dark || muted {
                if let Some(info) = lib.info(&fx.profile, fx.mode) {
                    safe_values(mode, &info, v);
                }
            } else if profile.needs_arm && !fx.armed {
                for (c, ch) in mode.channels.iter().enumerate() {
                    if let Some(s) = ch.safe {
                        v[c] = s;
                    }
                }
            }
            if let Some(trace)=self.rt.trace.as_mut().filter(|t|t.fixture==fx.id && t.result.is_none()) {
                trace.capture_single(fx.id,mode,v,"gates",serde_json::json!({"blackout":dark,"muted":muted,"armed":fx.armed,"needs_arm":profile.needs_arm}));
            }
            // 禁止の値(レーザーのオートショーなど)と、動いてよい範囲は、何をしていても最後に必ず直す
            let gated = requested.as_ref().map(|_| v.clone());
            guard_values(mode, &fx.limits, v);
            if let Some(trace)=self.rt.trace.as_mut().filter(|t|t.fixture==fx.id && t.result.is_none()) {
                trace.capture_single(fx.id,mode,v,"safety",serde_json::json!({"limits":fx.limits}));
            }
            if let Some(before) = requested {
                let gated = gated.unwrap();
                for (ch, (&requested, &applied)) in before.iter().zip(v.iter()).enumerate() {
                    if requested != applied {
                        let mut reasons = Vec::new();
                        if requested != gated[ch] {
                            reasons.push(if dark { "blackout" } else if muted { "mute" } else { "not_armed" });
                        }
                        let [a, b] = fx.limits.get(&mode.channels[ch].attr).copied().unwrap_or([0, 255]);
                        let clamped = gated[ch].clamp(a.min(b), a.max(b));
                        if gated[ch] != clamped { reasons.push("limits"); }
                        if mode.channels[ch].is_forbidden(clamped) { reasons.push("forbid"); }
                        self.rt.safety_events.push(serde_json::json!({
                            "fixture": fx.id, "attr": mode.channels[ch].attr,
                            "channel": ch + 1, "requested": requested, "applied": applied,
                            "reason": reasons.join("+"), "reasons": reasons
                        }));
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
        self.rt.frame_id += 1;
        self.trace_finish();
        if let Some(recorder) = self.rt.recorder.clone() {
            recorder.frame(self, false);
        }
    }
}

/// 安全な値にする: 安全値のあるチャンネルは安全値(レーザーOFFなど)、明るさのチャンネルは0
fn safe_values(mode: &Mode, info: &crate::fixture::ModeInfo, v: &mut [u8]) {
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

/// 異常時(画面や出力のスレッドがpanicした・終了のシグナルを受けた)に立てる。
/// 立つと出力スレッドは、以後は安全なフレームだけを送る(戻らない。アプリの再起動が必要)
pub static EMERGENCY: AtomicBool = AtomicBool::new(false);
/// 安全なフレームを送り終えたら、プロセスを終了する(終了のシグナルを受けたとき)
static EXIT_AFTER_SAFE: AtomicBool = AtomicBool::new(false);
/// 出力スレッドが動いている間だけ立つ(終了時の安全停止を、出力スレッドが送れるときだけ待つため)
static ENGINE_RUNNING: AtomicBool = AtomicBool::new(false);
/// 安全なフレームを送る回数(40Hzで約0.3秒)
const SAFE_TICKS: u32 = 12;

/// 安全停止に入る。`exit`=安全なフレームを送り終えたら終了する
pub fn emergency_stop(exit: bool) {
    if exit {
        EXIT_AFTER_SAFE.store(true, Ordering::SeqCst);
    }
    EMERGENCY.store(true, Ordering::SeqCst);
}

/// 画面のスレッドのpanicで安全停止する(出力スレッドが安全なフレームを送る時間を待ってから終了へ進む)。
/// Ctrl+C・終了のシグナルでも、安全なフレームを送ってから終了する
pub fn install_safety_hooks() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        // 画面(main)が落ちるとアプリが終わる。出力スレッドの中のpanicは、出力スレッド自身が受け止める
        if std::thread::current().name() == Some("main") {
            emergency_stop(false);
            std::thread::sleep(TICK * (SAFE_TICKS + 2));
        }
    }));
    #[cfg(unix)]
    unsafe {
        extern "C" fn on_signal(_: libc::c_int) {
            // シグナルの中ではフラグを立てるだけ(終了は出力スレッドが安全なフレームを送ってから行う)。
            // 2回目(出力スレッドが動いていないときなど)は、すぐに終了する
            if EMERGENCY.load(Ordering::SeqCst) {
                unsafe { libc::_exit(130) };
            }
            emergency_stop(true);
        }
        let h = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGINT, h);
        libc::signal(libc::SIGTERM, h);
        libc::signal(libc::SIGHUP, h);
        // 画面を通らずにプロセスが終わるとき(Linuxでログアウト時に画面のサーバーが先に落ちて、
        // ウィンドウのライブラリが exit() するなど)も、出力スレッドが動いていれば安全なフレームを送ってから終わる
        extern "C" fn on_exit() {
            if ENGINE_RUNNING.load(Ordering::SeqCst) && !EMERGENCY.load(Ordering::SeqCst) {
                emergency_stop(false);
                std::thread::sleep(TICK * (SAFE_TICKS + 2));
            }
        }
        libc::atexit(on_exit);
    }
    // Windows: Ctrl+C / Ctrl+Break / コンソールを閉じた / ログオフ / シャットダウン。
    // コンソールがあるとき(開発用のデバッグビルド)だけ届く。リリース版はウィンドウだけのアプリ
    // (main.rs の windows_subsystem)でコンソールが無いので、シャットダウン・ログオフは下の watch_windows_shutdown で受ける
    #[cfg(windows)]
    unsafe {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
            fn ExitProcess(code: u32) -> !;
        }
        unsafe extern "system" fn on_ctrl(kind: u32) -> i32 {
            // 2回目のCtrl+C / Ctrl+Break(出力スレッドが動いていないときなど)は、すぐに終了する。
            // シャットダウン等(2以上)は下の watch_windows_shutdown が先に安全停止を始めていることがあるので、待つほうにする
            if kind <= 1 && EMERGENCY.load(Ordering::SeqCst) {
                unsafe { ExitProcess(130) };
            }
            emergency_stop(true);
            // この関数から戻ると、コンソールを閉じたとき等はWindowsがプロセスを終わらせてしまうので、
            // 出力スレッドが安全なフレームを送り終えて終了するまで待つ
            std::thread::sleep(TICK * (SAFE_TICKS + 8));
            1
        }
        SetConsoleCtrlHandler(Some(on_ctrl), 1);
    }
    #[cfg(windows)]
    watch_windows_shutdown();
}

/// Windowsのシャットダウン・ログオフで安全停止する。
/// ウィンドウだけのアプリには、シャットダウンは WM_QUERYENDSESSION → WM_ENDSESSION として届く(on_exit は呼ばれない)。
/// 画面(egui)のウィンドウとは別に、見えないウィンドウを専用のスレッドで作って受ける
/// (画面のスレッドが止まっていても受けられるように)
#[cfg(windows)]
fn watch_windows_shutdown() {
    use std::ffi::c_void;
    use std::ptr::null_mut;
    type Hwnd = *mut c_void;
    #[repr(C)]
    struct WndClassW {
        style: u32,
        wnd_proc: unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize,
        cls_extra: i32,
        wnd_extra: i32,
        instance: *mut c_void,
        icon: *mut c_void,
        cursor: *mut c_void,
        background: *mut c_void,
        menu_name: *const u16,
        class_name: *const u16,
    }
    #[repr(C)]
    struct Msg {
        hwnd: Hwnd,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt: [i32; 2],
        private: u32,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn RegisterClassW(class: *const WndClassW) -> u16;
        fn CreateWindowExW(ex_style: u32, class: *const u16, name: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: Hwnd, menu: *mut c_void, instance: *mut c_void, param: *mut c_void) -> Hwnd;
        fn DefWindowProcW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn GetMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32) -> i32;
        fn TranslateMessage(msg: *const Msg) -> i32;
        fn DispatchMessageW(msg: *const Msg) -> isize;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }
    const WM_ENDSESSION: u32 = 0x0016;

    unsafe extern "system" fn wnd_proc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize {
        // wparam != 0 = 本当に終わる(キャンセルされたときは0)。
        // ここから戻るとWindowsはいつでもプロセスを終わらせるので、出力スレッドが安全なフレームを送り終えて終了するまで待つ
        if msg == WM_ENDSESSION && wparam != 0 {
            emergency_stop(true);
            std::thread::sleep(TICK * (SAFE_TICKS + 8));
            return 0;
        }
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    let spawned = std::thread::Builder::new().name("shutdown-watch".into()).spawn(|| {
        let class: Vec<u16> = "dmxctl-shutdown-watch\0".encode_utf16().collect();
        // SAFETY: Win32の公開APIを、宣言どおりの型で呼ぶ。ウィンドウはこのスレッドで作り、このスレッドでメッセージを処理する
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let wc = WndClassW {
                style: 0,
                wnd_proc,
                cls_extra: 0,
                wnd_extra: 0,
                instance,
                icon: null_mut(),
                cursor: null_mut(),
                background: null_mut(),
                menu_name: std::ptr::null(),
                class_name: class.as_ptr(),
            };
            if RegisterClassW(&wc) == 0 {
                return;
            }
            // 親なしのウィンドウ(表示はしない)。メッセージ専用ウィンドウ(HWND_MESSAGE)にはシャットダウンが届かないので使わない
            let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, null_mut(), null_mut(), instance, null_mut());
            if hwnd.is_null() {
                return;
            }
            let mut msg: Msg = std::mem::zeroed();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    });
    if let Err(e) = spawned {
        eprintln!("シャットダウンの監視を始められません: {e}");
    }
}

/// 出力スレッド。40Hzでフレームを合成して各出力へ送る
pub struct Engine {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn spawn(core: Shared) -> Engine {
        Self::spawn_with_output(core, true)
    }

    /// Keep rendering at 40 Hz even when physical output is disabled.
    pub fn spawn_with_output(core: Shared, output_enabled: bool) -> Engine {
        lock(&core).rt.physical_output_enabled=output_enabled;
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = std::thread::Builder::new()
            .name("dmx-engine".into())
            .spawn(move || run(core, s2, output_enabled))
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

impl Drop for Engine {
    fn drop(&mut self) { self.stop(); }
}

fn run(core: Shared, stop: Arc<AtomicBool>, output_enabled: bool) {
    struct Running;
    impl Drop for Running {
        fn drop(&mut self) {
            ENGINE_RUNNING.store(false, Ordering::SeqCst);
        }
    }
    ENGINE_RUNNING.store(true, Ordering::SeqCst);
    let _running = Running;
    let mut cfgs: Vec<OutputCfg> = Vec::new();
    let mut outs: Vec<Box<dyn Output>> = Vec::new();
    let mut gen_seen = u64::MAX;
    let mut next = Instant::now();
    // 異常時に送るフレーム(毎回、最新のものを作っておく)
    let mut safe: Option<(Vec<[u8; 512]>, Vec<usize>)> = None;
    let mut safe_sent = 0u32;

    while !stop.load(Ordering::SeqCst) {
        if EMERGENCY.load(Ordering::SeqCst) {
            // 安全停止中: Coreを待たずに(画面のスレッドが止まっていても)安全なフレームを送り続ける
            if let Some((frames, lens)) = &safe {
                send_frames(&cfgs, &mut outs, frames, lens);
                // Safety output uses a separate path from render: record it too.
                if let Ok(mut c) = core.try_lock() {
                    c.rt.frames = frames.clone();
                    c.rt.frame_id += 1;
                    c.rt.safety_events.clear();
                    if let Some(recorder) = c.rt.recorder.clone() {
                        recorder.frame(&c, true);
                    }
                }
            }
            safe_sent += 1;
            if safe_sent >= SAFE_TICKS && EXIT_AFTER_SAFE.load(Ordering::SeqCst) {
                std::process::exit(130);
            }
            wait_tick(&mut next);
            continue;
        }
        // 1回分の処理でpanicしても、出力を止めずに安全停止へ切り替える(最後のフレームが出っぱなしにならないように)
        let tick = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let Some(mut c) = lock_unless_emergency(&core) else { return };
            c.render();
            let (frames, lens, cfg_now, gen_now) = (c.rt.frames.clone(), c.rt.used_len.clone(), c.project.outputs.clone(), c.rt.output_gen);
            safe = Some((c.safe_frames(), lens.clone()));
            drop(c);
            if output_enabled && gen_now != gen_seen {
                // 先に古い出力を閉じてからポートを開き直す
                outs.clear();
                outs = cfg_now.iter().map(output::create).collect();
                cfgs = cfg_now;
                gen_seen = gen_now;
            }
            send_frames(&cfgs, &mut outs, &frames, &lens);
            let statuses: Vec<String> = outs.iter().map(|o| o.status()).collect();
            lock(&core).rt.output_status = statuses;
        }));
        if tick.is_err() {
            emergency_stop(false);
            continue;
        }
        wait_tick(&mut next);
    }
}

/// Coreを借りる。待っている間に安全停止になったら諦める(panicした画面のスレッドがロックを持ったままでも止まらないように)
fn lock_unless_emergency(core: &Shared) -> Option<MutexGuard<'_, Core>> {
    loop {
        match core.try_lock() {
            Ok(g) => return Some(g),
            Err(std::sync::TryLockError::Poisoned(e)) => return Some(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => {
                if EMERGENCY.load(Ordering::SeqCst) {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

fn send_frames(cfgs: &[OutputCfg], outs: &mut [Box<dyn Output>], frames: &[[u8; 512]], lens: &[usize]) {
    for (cfg, o) in cfgs.iter().zip(outs.iter_mut()) {
        if !cfg.enabled {
            continue;
        }
        if let (Some(f), Some(&len)) = (frames.get(cfg.universe), lens.get(cfg.universe)) {
            o.send(f, len);
        }
    }
}

fn wait_tick(next: &mut Instant) {
    *next += TICK;
    let now = Instant::now();
    if *next > now {
        std::thread::sleep(*next - now);
    } else {
        *next = now;
    }
}
