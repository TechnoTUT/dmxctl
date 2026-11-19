//! One JSONL stream for live observation and post-run analysis. Disk/terminal IO
//! never blocks the engine. Sequence gaps and a final dropped count expose loss.
use crate::core::recorder::Recorder;
use serde_json::{json, Value};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::{Arc, mpsc};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;

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
            recorder: Recorder::new(tx, dropped, lossless),
            writer,
        })
    }

    /// Detach all Core handles before finishing, so the channel can close.
    pub fn finish(self) -> anyhow::Result<()> {
        drop(self.recorder);
        self.writer.join().map_err(|_| anyhow::anyhow!("log writer panicked"))?.map_err(anyhow::Error::msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::engine::Core;
    #[test]
    fn input_history_suppresses_remote_duplicates_and_redacts_observations() {
        let path=std::env::temp_dir().join(format!("dmxctl-history-{}.jsonl",std::process::id()));
        let _=std::fs::remove_file(&path);
        let recording=Recording::start(Some(&path),false,true).unwrap();
        let (lib,_)=crate::core::fixture::Library::load(Path::new("none"));
        let mut c=Core::new(crate::core::project::Project::starter(&lib),Arc::new(lib));
        c.project.web_pin="secret-history-pin".into();
        c.rt.recorder=Some(recording.recorder.clone());
        c.handle_key(16,true);
        crate::backend::commands::apply(&mut c,&json!({"cmd":"input","source":"key","number":16,"pressed":false})).unwrap();
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
