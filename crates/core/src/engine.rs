//! 共有状態(Core)と、DMX値の合成・送出ループ。
//!
//! `Core`の操作は分野ごとに子モジュールに分けてある:
//! - `palette`: 色の全体制御、メイン色/サブ色
//! - `pair`: ペアの連動
//! - `tempo`: BPM・Link・MIDIクロック・音声検出・チェイス
//! - `scene`: シーン・位置プリセット・フェード
//! - `binding`: MIDIの受信と割り当ての実行
//!
//! 合成と出力の部品も、ファイルを分けてある:
//! - `runtime`: 実行中の状態 `Runtime`
//! - `render`: ユニバースごとのフレームの合成
//! - `guard`: 出力の直前の禁止値・動いてよい範囲・安全な値
//! - `safety`: 異常時・終了のシグナル・シャットダウンの安全なフレーム
//! - `output_loop`: 40Hzの出力スレッド
//! - `glide`: OSCで受けた値の滑らかな追従

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
use crate::io::output::{self, Output};
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

}

mod glide;
mod guard;
mod output_loop;
mod pixel;
mod render;
mod runtime;
mod safety;

pub use glide::*;
pub use guard::*;
pub use output_loop::*;
pub use runtime::*;
pub use safety::*;
