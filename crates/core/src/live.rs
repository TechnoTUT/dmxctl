//! 画面から実行中の状態を変える操作(TAP・シーンの呼び出し・色・ブラックアウトなど)。
//! 画面は `Core` のメソッドや `rt` を直接触らずに、`Core::live` にこの操作を渡す。
//! 1か所を通るので、あとでサーバーにつなぐクライアントでは、ここで操作をサーバーへ送ればよい。
//! (プロジェクトの編集(機材・割り当て・シーンの追加など)は editing の操作で行う)

use crate::color::{ColorRole, ColorTarget, NamedColor};
use crate::engine::Core;
use crate::motion::MotionOp;
use crate::engine::MidiMsg;
use crate::project::{PhraseLook, Selector, TempoSource, TriggerKind, PAGES};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "live", rename_all = "snake_case")]
pub enum LiveOp {
    /// 機材の値(手動)
    SetAttr { sel: Selector, attr: String, value: f32 },
    /// MIDIコントローラーからの入力
    Midi { kind: TriggerKind, channel: u8, number: u8, value: u8 },
    /// APC mini mk2 から読み取ったフェーダー1..9の位置(表示用。割り当ては実行しない)
    FaderPositions { positions: [u8; 9] },
    /// キーボードの割り当て(押した/離した)
    Key { number: u8, pressed: bool },
    SetBpm { bpm: f32 },
    Tap,
    /// 頭出し(小節の頭へ)
    Resync,
    Motion { op: MotionOp, value: f32 },
    /// テンポの元(反映には `Host::apply_services` が要る)
    SetTempoSource { source: TempoSource },
    SetColorRole { fixture: u32, role: ColorRole },
    /// 色を個別に変える機材の選択
    ToggleSelected { fixture: u32 },
    /// フェードイン/アウトの切り替え(秒)
    ToggleFade { secs: f32 },
    SwapPalette,
    SetPaletteColor { slot: ColorRole, rgb: [f32; 3] },
    SetPaletteAuto { on: bool },
    SetEffect { name: String, on: bool },
    SetPixelFx { name: String, on: bool },
    SetBarCount { count: usize },
    /// シーンを今の値で取り直す
    RefreshScene { index: usize },
    RecallScene { name: String },
    RecallPosition { name: String },
    NamedColor { color: NamedColor, target: ColorTarget },
    Look { key: String, look: PhraseLook },
    Hue { degrees: f32, target: ColorTarget },
    ToggleBlackout,
    SetBlackout { on: bool },
    /// APCのページ(0から)
    SetPage { page: u8 },
    /// 前(-1)/次(+1)のページ
    StepPage { delta: i8 },
    /// レーザーなどの出力の許可(ARM)。保存しない値なので、プロジェクトの編集ではなくこの操作で変える
    SetArm { fixture: u32, on: bool },
    /// レーザーの合図: 常時出力 / 押している間だけ
    SetLaserLatch { on: bool },
    SetLaserHold { on: bool },
    /// 全体光量(0..1)
    SetMaster { level: f32 },
    /// 色を全体で変えるか、選んだ機材だけか
    SetColorGlobal { on: bool },
    /// MIDI/OSCのラーン(次に届いた入力を、この番号の割り当てに覚えさせる)。
    /// 画面はラーンの状態を直接書き換えるので、クライアントでは1フレームの後で変化をこの操作にして送る
    SetLearn { midi: Option<usize>, osc: Option<usize> },
    /// OSCで受けた色・明るさを捨てて、画面の値に戻す
    ClearOscColor,
    ClearOscLevel,
    /// 手動操作の優先を解いて、オートに戻す
    ClearManual,
}

impl LiveOp {
    /// 反映にサービスの設定し直し(`Host::apply_services`)が要るか
    pub fn needs_services(&self) -> bool {
        matches!(self, LiveOp::SetTempoSource { .. })
    }
}

/// 操作を手元で実行せず、別の場所へ送る口。クライアントで、操作をサーバーへ送るのに使う。
/// 実装は `backend::remote` にある。
pub trait LiveForward: Send + Sync {
    fn live(&self, op: &LiveOp);
}

impl Core {
    /// 画面からの操作を実行する
    pub fn live(&mut self, op: LiveOp) {
        // クライアントでは、手元の写しを変えずにサーバーへ送る(結果はサーバーから届く状態で反映される)
        if let Some(link) = &self.rt.remote {
            link.live(&op);
            return;
        }
        match op {
            LiveOp::SetAttr { sel, attr, value } => self.set_attr(&sel, &attr, value),
            LiveOp::Midi { kind, channel, number, value } => self.handle_midi(MidiMsg { kind, channel: channel & 0x0F, number: number & 0x7F, value: value & 0x7F }),
            LiveOp::FaderPositions { positions } => self.set_fader_positions(&positions),
            LiveOp::Key { number, pressed } => self.handle_key(number, pressed),
            LiveOp::SetBpm { bpm } => self.set_bpm(bpm),
            LiveOp::Tap => self.tap(),
            LiveOp::Resync => self.resync(),
            LiveOp::Motion { op, value } => self.motion_op(op, value),
            LiveOp::SetTempoSource { source } => self.set_tempo_source(source),
            LiveOp::SetColorRole { fixture, role } => self.set_color_role(fixture, role),
            LiveOp::ToggleSelected { fixture } => self.toggle_selected(fixture),
            LiveOp::ToggleFade { secs } => self.toggle_fade(secs),
            LiveOp::SwapPalette => self.swap_palette(),
            LiveOp::SetPaletteColor { slot, rgb } => self.set_palette_color(slot, rgb),
            LiveOp::SetPaletteAuto { on } => self.set_palette_auto(on),
            LiveOp::SetEffect { name, on } => self.set_effect(&name, on),
            LiveOp::SetPixelFx { name, on } => self.set_pixel_fx(&name, on),
            LiveOp::SetBarCount { count } => self.set_bar_count(count),
            LiveOp::RefreshScene { index } => {
                if index < self.project.scenes.len() {
                    self.refresh_scene(index);
                }
            }
            LiveOp::RecallScene { name } => self.recall_scene(&name),
            LiveOp::RecallPosition { name } => self.recall_position(&name),
            LiveOp::NamedColor { color, target } => self.apply_named_color(color, target),
            LiveOp::Look { key, look } => self.apply_look(&key, &look),
            LiveOp::Hue { degrees, target } => self.apply_hue(degrees, target),
            LiveOp::ToggleBlackout => self.rt.blackout = !self.rt.blackout,
            LiveOp::SetBlackout { on } => self.rt.blackout = on,
            LiveOp::SetPage { page } => self.rt.page = page.min(PAGES - 1),
            LiveOp::StepPage { delta } => self.rt.page = (self.rt.page as i16 + delta as i16).rem_euclid(PAGES as i16) as u8,
            LiveOp::SetArm { fixture, on } => {
                if let Some(i) = self.project.fixture_index(fixture) {
                    self.project.fixtures[i].armed = on;
                }
            }
            LiveOp::SetLaserLatch { on } => self.rt.laser_latch = on,
            LiveOp::SetLaserHold { on } => self.set_laser_hold(on),
            LiveOp::SetMaster { level } => self.rt.master = level.clamp(0.0, 1.0),
            LiveOp::SetColorGlobal { on } => self.rt.color_global = on,
            LiveOp::SetLearn { midi, osc } => {
                self.rt.learn = midi.filter(|&i| i < self.project.bindings.len());
                self.rt.osc_learn = osc.filter(|&i| i < self.project.bindings.len());
            }
            LiveOp::ClearOscColor => self.rt.osc_rgb = [None, None],
            LiveOp::ClearOscLevel => self.rt.osc_level = None,
            LiveOp::ClearManual => self.rt.manual_last = None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::Library;
    use crate::project::Project;
    use std::sync::Arc;

    fn core() -> Core {
        let (lib, _) = Library::load(std::path::Path::new("/nonexistent"));
        let project = Project::starter(&lib);
        Core::new(project, Arc::new(lib))
    }

    #[test]
    fn ops_round_trip_through_json() {
        let ops = [
            LiveOp::Tap,
            LiveOp::Key { number: 3, pressed: true },
            LiveOp::SetAttr { sel: Selector::Fixture(1), attr: "pan".into(), value: 80.0 },
            LiveOp::Motion { op: MotionOp::Size, value: 0.5 },
            LiveOp::Hue { degrees: 120.0, target: ColorTarget::Scope },
            LiveOp::StepPage { delta: -1 },
        ];
        for op in ops {
            let text = serde_json::to_string(&op).unwrap();
            assert_eq!(serde_json::from_str::<LiveOp>(&text).unwrap(), op, "{text}");
        }
    }

    #[test]
    fn page_steps_wrap_and_blackout_toggles() {
        let mut c = core();
        c.rt.page = 0;
        c.live(LiveOp::StepPage { delta: -1 });
        assert_eq!(c.rt.page, PAGES - 1);
        c.live(LiveOp::StepPage { delta: 1 });
        assert_eq!(c.rt.page, 0);
        c.live(LiveOp::SetPage { page: 200 });
        assert_eq!(c.rt.page, PAGES - 1);
        let before = c.rt.blackout;
        c.live(LiveOp::ToggleBlackout);
        assert_eq!(c.rt.blackout, !before);
        c.live(LiveOp::SetMaster { level: 2.0 });
        assert_eq!(c.rt.master, 1.0);
    }
}
