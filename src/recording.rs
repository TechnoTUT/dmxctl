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
