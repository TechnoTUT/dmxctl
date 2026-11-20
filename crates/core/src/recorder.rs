//! 実行の記録を取る口 `Recorder` と、記録用の状態の比較。
//! ディスクや端末への書き込みは `backend::recording` が受け持ち、ここはエンジンを止めない。
use crate::engine::Core;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex, mpsc};
use std::sync::atomic::{AtomicU64, Ordering};

struct State {
    seq: u64,
    frame: u64,
    previous: Vec<Vec<u8>>,
}

#[derive(Clone)]
pub struct Recorder {
    tx: mpsc::SyncSender<Value>,
    state: Arc<Mutex<State>>,
    dropped: Arc<AtomicU64>,
    lossless: bool,
}

impl Recorder {
    /// 書き込み側へ送る口を受け取る。`dropped` は送れなかった数を数える
    pub fn new(tx: mpsc::SyncSender<Value>, dropped: Arc<AtomicU64>, lossless: bool) -> Self {
        Recorder { tx, state: Arc::new(Mutex::new(State { seq: 0, frame: 0, previous: Vec::new() })), dropped, lossless }
    }

    pub fn next_frame(&self) -> u64 {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).frame
    }

    pub fn next_sequence(&self) -> u64 {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).seq
    }

    pub fn emit(&self, mut event: Value) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        event["seq"] = json!(state.seq);
        state.seq += 1;
        let failed = if self.lossless { self.tx.send(event).is_err() } else { self.tx.try_send(event).is_err() };
        if failed { self.dropped.fetch_add(1, Ordering::Relaxed); }
    }

    pub fn frame(&self, core: &Core, emergency: bool) {
        let frames: Vec<Vec<u8>> = core.rt.frames.iter().map(|f| f.to_vec()).collect();
        let (frame, full, changes) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let frame = state.frame;
            let full = frame.is_multiple_of(40) || emergency || state.previous.is_empty();
            let mut changes = Vec::new();
            for (u, data) in frames.iter().enumerate() {
                let used = core.project.fixtures.iter().any(|f| f.universe == u)
                    || core.project.outputs.iter().any(|o| o.enabled && o.universe == u)
                    || state.previous.get(u).is_some_and(|f| f.iter().any(|&v| v != 0));
                if !used { continue; }
                for (ch, &value) in data.iter().enumerate() {
                    if full || state.previous.get(u).and_then(|v| v.get(ch)).copied() != Some(value) {
                        let fixture = core.project.fixtures.iter().find(|f| f.enabled && f.universe == u
                            && ch + 1 >= f.address as usize
                            && ch + 1 < f.address as usize + f.values.len());
                        let attr = fixture.and_then(|f| core.lib.mode(&f.profile,f.mode)
                            .and_then(|m|m.channels.get(ch + 1 - f.address as usize))).map(|c|c.attr.as_str());
                        changes.push(json!({"universe":u+1, "address":ch+1, "value":value,
                            "fixture":fixture.map(|f|f.id), "attr":attr}));
                    }
                }
            }
            state.previous = frames;
            state.frame += 1;
            (frame, full, changes)
        };
        self.emit(json!({"type":"frame", "frame":frame, "engine_frame":core.rt.frame_id,
            "t_ms":core.now().saturating_duration_since(core.rt.t0).as_millis() as u64,
            "full":full, "emergency":emergency, "beat":core.rt.beat,
            "changes":changes, "safety":core.rt.safety_events}));
    }
}

impl Core {
    /// External inputs are root operations; remotely replayed inputs already
    /// have a command event, so do not record their dispatch a second time.
    pub(crate) fn record_input(&self,command:Value) {
        if !self.rt.observe.is_root_input() {return;}
        if let Some(recorder)=self.rt.observe.recorder() {
            recorder.emit(json!({"type":"command","source":"input","command":command,
                "learning":self.rt.learn.is_some() || self.rt.osc_learn.is_some(),
                "t_ms":self.now().saturating_duration_since(self.rt.t0).as_millis() as u64,
                "applies_from_frame":recorder.next_frame(),"applies_from_engine_frame":self.rt.frame_id+1}));
        }
    }

    pub fn history_state(&self)->Option<Value> {
        self.history_snapshot().map(HistorySnapshot::state)
    }

    /// 記録中だけ、比較用の状態を複製する(ロック中は複製だけ。JSONにするのはロックの外)
    pub fn history_snapshot(&self)->Option<HistorySnapshot> {
        let recorder=self.rt.observe.recorder().cloned()?;
        Some(HistorySnapshot {
            project:self.project.clone(),
            profiles:self.lib.profiles.clone(),
            runtime:json!({"master":self.rt.master,"blackout":self.rt.blackout,"page":self.rt.page,"key_page":self.rt.key_page,
                "armed":self.project.fixtures.iter().map(|f|json!({"fixture":f.id,"armed":f.armed})).collect::<Vec<_>>(),
                "muted":self.rt.muted,"flash":self.rt.flash,"selected":self.rt.selected,"color_global":self.rt.color_global}),
            recorder,
            frame:self.rt.frame_id,
            t_ms:self.now().saturating_duration_since(self.rt.t0).as_millis() as u64,
        })
    }

    pub fn record_observation(&self,before:Option<Value>,source:&str) {
        let (Some(before),Some(after),Some(recorder))=(before,self.history_state(),self.rt.observe.recorder()) else{return;};
        if before==after {return;}
        // An observation spans a GUI frame, not an atomic engine operation.
        // Keep it separate from executable commands rather than inventing one.
        recorder.emit(json!({"type":"state_change","source":source,"before":before,"after":after,
            "t_ms":self.now().saturating_duration_since(self.rt.t0).as_millis() as u64,
            "observed_at_engine_frame":self.rt.frame_id,"replayable":false}));
    }
}

/// GUIの1フレームの前後を比べるための状態。Coreのロックを持たずにJSONにできる
pub struct HistorySnapshot {
    project:crate::project::Project,
    profiles:Vec<Arc<crate::fixture::Profile>>,
    runtime:Value,
    recorder:Recorder,
    frame:u64,
    t_ms:u64,
}

impl HistorySnapshot {
    fn state(self)->Value {
        json!({"project":redact(&serde_json::to_value(&self.project).unwrap()),
            "profiles":self.profiles.iter().map(|p|p.as_ref()).collect::<Vec<_>>(),"runtime":self.runtime})
    }

    /// 前後で変わっていれば state_change を記録する。Coreのロックの外で呼ぶ(40Hzの出力を止めない)
    pub fn observe(before:Option<Self>,after:Option<Self>,source:&str) {
        let (Some(before),Some(after))=(before,after) else {return;};
        // 機材定義は Arc を共有したままなら同じもの。プロジェクトと実行状態が同じなら比べるまでもない
        let same_profiles=before.profiles.len()==after.profiles.len()
            && before.profiles.iter().zip(&after.profiles).all(|(a,b)|Arc::ptr_eq(a,b));
        if same_profiles && before.runtime==after.runtime
            && serde_json::to_value(&before.project).ok()==serde_json::to_value(&after.project).ok() {return;}
        let (recorder,frame,t_ms)=(after.recorder.clone(),after.frame,after.t_ms);
        let (before,after)=(before.state(),after.state());
        if before==after {return;}
        recorder.emit(json!({"type":"state_change","source":source,"before":before,"after":after,
            "t_ms":t_ms,"observed_at_engine_frame":frame,"replayable":false}));
    }
}


/// 設定に入っている操作パネルの合言葉を、ログや差分に出さないように消す
pub fn redact(value:&Value)->Value {
    match value {
        Value::Object(map)=>Value::Object(map.iter().map(|(key,value)|(key.clone(),if key=="web_pin"{json!("")}else{redact(value)})).collect()),
        Value::Array(values)=>Value::Array(values.iter().map(redact).collect()),
        _=>value.clone(),
    }
}
