//! One JSONL stream for live observation and post-run analysis. Disk/terminal IO
//! never blocks the engine. Sequence gaps and a final dropped count expose loss.
use crate::engine::Core;
use serde_json::{json, Value};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;

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

pub struct Recording {
    pub recorder: Recorder,
    writer: JoinHandle<Result<(), String>>,
}

impl Recording {
    pub fn start(path: Option<&Path>, live: bool, lossless: bool) -> anyhow::Result<Self> {
        // Never overwrite a project, scenario, or existing recording.
        let file = path.map(|p| std::fs::OpenOptions::new().write(true).create_new(true).open(p)).transpose()?;
        let (tx, rx) = mpsc::sync_channel::<Value>(256);
        let dropped = Arc::new(AtomicU64::new(0));
        let count = dropped.clone();
        let writer = std::thread::spawn(move || {
            let mut file = file.map(BufWriter::new);
            let mut live = live;
            let mut write_event = |event: &Value| -> Result<(), String> {
                let line = serde_json::to_vec(event).map_err(|e| e.to_string())?;
                if let Some(f) = file.as_mut() {
                    f.write_all(&line).and_then(|_| f.write_all(b"\n")).and_then(|_| f.flush()).map_err(|e| e.to_string())?;
                }
                if live {
                    let stdout = std::io::stdout();
                    let mut out = stdout.lock();
                    if out.write_all(&line).and_then(|_| out.write_all(b"\n")).and_then(|_| out.flush()).is_err() {
                        // Closing a live consumer must not interrupt file recording.
                        live = false;
                    }
                }
                Ok(())
            };
            for event in rx { write_event(&event)?; }
            write_event(&json!({"type":"end", "dropped":count.load(Ordering::Relaxed)}))?;
            Ok(())
        });
        Ok(Self {
            recorder: Recorder { tx, state: Arc::new(Mutex::new(State { seq: 0, frame: 0, previous: Vec::new() })), dropped, lossless },
            writer,
        })
    }

    /// Detach all Core handles before finishing, so the channel can close.
    pub fn finish(self) -> anyhow::Result<()> {
        drop(self.recorder);
        self.writer.join().map_err(|_| anyhow::anyhow!("log writer panicked"))?.map_err(anyhow::Error::msg)
    }
}

impl Recorder {
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
            let full = frame % 40 == 0 || emergency || state.previous.is_empty();
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
        if self.rt.audit_depth!=0 {return;}
        if let Some(recorder)=&self.rt.recorder {
            recorder.emit(json!({"type":"command","source":"input","command":command,
                "learning":self.rt.learn.is_some() || self.rt.osc_learn.is_some(),
                "t_ms":self.now().saturating_duration_since(self.rt.t0).as_millis() as u64,
                "applies_from_frame":recorder.next_frame(),"applies_from_engine_frame":self.rt.frame_id+1}));
        }
    }

    pub(crate) fn history_state(&self)->Option<Value> {
        self.history_snapshot().map(HistorySnapshot::state)
    }

    /// 記録中だけ、比較用の状態を複製する(ロック中は複製だけ。JSONにするのはロックの外)
    pub(crate) fn history_snapshot(&self)->Option<HistorySnapshot> {
        let recorder=self.rt.recorder.clone()?;
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

    pub(crate) fn record_observation(&self,before:Option<Value>,source:&str) {
        let (Some(before),Some(after),Some(recorder))=(before,self.history_state(),&self.rt.recorder) else{return;};
        if before==after {return;}
        // An observation spans a GUI frame, not an atomic engine operation.
        // Keep it separate from executable commands rather than inventing one.
        recorder.emit(json!({"type":"state_change","source":source,"before":before,"after":after,
            "t_ms":self.now().saturating_duration_since(self.rt.t0).as_millis() as u64,
            "observed_at_engine_frame":self.rt.frame_id,"replayable":false}));
    }
}

/// GUIの1フレームの前後を比べるための状態。Coreのロックを持たずにJSONにできる
pub(crate) struct HistorySnapshot {
    project:crate::project::Project,
    profiles:Vec<Arc<crate::fixture::Profile>>,
    runtime:Value,
    recorder:Recorder,
    frame:u64,
    t_ms:u64,
}

impl HistorySnapshot {
    fn state(self)->Value {
        json!({"project":crate::editing::redact(&serde_json::to_value(&self.project).unwrap()),
            "profiles":self.profiles.iter().map(|p|p.as_ref()).collect::<Vec<_>>(),"runtime":self.runtime})
    }

    /// 前後で変わっていれば state_change を記録する。Coreのロックの外で呼ぶ(40Hzの出力を止めない)
    pub(crate) fn observe(before:Option<Self>,after:Option<Self>,source:&str) {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn input_history_suppresses_remote_duplicates_and_redacts_observations() {
        let path=std::env::temp_dir().join(format!("dmxctl-history-{}.jsonl",std::process::id()));
        let _=std::fs::remove_file(&path);
        let recording=Recording::start(Some(&path),false,true).unwrap();
        let (lib,_)=crate::fixture::Library::load(Path::new("none"));
        let mut c=Core::new(crate::project::Project::starter(&lib),Arc::new(lib));
        c.project.web_pin="secret-history-pin".into();
        c.rt.recorder=Some(recording.recorder.clone());
        c.handle_key(16,true);
        crate::commands::apply(&mut c,&json!({"cmd":"input","source":"key","number":16,"pressed":false})).unwrap();
        let before=c.history_state();c.project.bpm=125.0;c.record_observation(before,"gui_frame");
        c.rt.recorder=None;recording.finish().unwrap();
        let text=std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("secret-history-pin"));
        let events:Vec<Value>=text.lines().map(|s|serde_json::from_str(s).unwrap()).collect();
        assert_eq!(events.iter().filter(|v|v["type"]=="command").count(),1);
        assert!(events.iter().any(|v|v["type"]=="state_change" && v["after"]["project"]["bpm"]==125.0));
        std::fs::remove_file(path).unwrap();
    }
}
