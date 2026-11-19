//! 実行の観測。記録 `Recorder` と、指定した機材の診断 `Trace` をまとめて持つ。
//! エンジンの合成はここを経由して観測し、観測の有無を `Runtime` の他の値と混ぜない。
use crate::recorder::Recorder;
use crate::trace::Trace;

#[derive(Default)]
pub struct Observe {
    recorder: Option<Recorder>,
    pub(crate) trace: Option<Trace>,
    pub(crate) trace_ticket: u64,
    /// コマンドを実行している入れ子の深さ。0 でないあいだの入力は、コマンドとして記録済みなので重ねて記録しない
    audit_depth: usize,
}

impl Observe {
    /// 記録の書き込み口。記録していなければ `None`
    pub fn recorder(&self) -> Option<&Recorder> {
        self.recorder.as_ref()
    }

    /// 記録を始める・終える
    pub fn set_recorder(&mut self, recorder: Option<Recorder>) {
        self.recorder = recorder;
    }

    pub fn is_recording(&self) -> bool {
        self.recorder.is_some()
    }

    /// 診断を待っている・取っている最中か
    pub fn is_tracing(&self) -> bool {
        self.trace.is_some()
    }

    /// コマンドの実行に入る。外から来た入力か、コマンドの中の入力かを見分けるのに使う
    pub fn enter_command(&mut self) {
        self.audit_depth += 1;
    }

    pub fn leave_command(&mut self) {
        self.audit_depth = self.audit_depth.saturating_sub(1);
    }

    /// コマンドの外から来た入力か。そうでなければ、記録しない
    pub fn is_root_input(&self) -> bool {
        self.audit_depth == 0
    }
}
