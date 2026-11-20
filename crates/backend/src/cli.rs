//! JSON automation with isolated project state and explicit device output.
use crate::commands::{apply, validate};
use dmxctl_core::{engine::{self, Core, Engine, TICK}, fixture::Library, project::{Project, TempoSource}};
use crate::recording::Recording;
use anyhow::{Context, bail, ensure};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::{BufRead, Write}, path::{Path, PathBuf}, sync::{Arc, Mutex, mpsc}, time::{Duration, Instant}};

/// `dmxctl` の最初の引数がこれなら、CLIとして動かす
pub const COMMANDS: &[&str] = &["serve","ctl","mcp","inspect","simulate","run","watch","log","diff","carabiner"];

pub const HELP: &str = r#"dmxctl carabiner install
dmxctl inspect --project FILE [--fixtures DIR]
dmxctl serve --project FILE [--listen 127.0.0.1:9190] [--output none|live] [--inputs none|configured] [--autosave]
dmxctl ctl [--connect 127.0.0.1:9190] status|inspect|stop
dmxctl mcp [--connect 127.0.0.1:9190]   (MCP のサーバー。AI のクライアントから標準入出力で使う。本体は先に serve か --control で起動しておく)
dmxctl ctl fixture list
dmxctl ctl button list
dmxctl ctl button click|press|release|hold binding:ID [--hold-ms N]
dmxctl ctl button set binding:ID --value 0.5
dmxctl ctl input midi --kind Note --channel 9 --number 40 --value 127
dmxctl ctl input key --json '{"number":16,"pressed":true}'
dmxctl ctl input osc --json '{"address":"/dmx/bpm","args":[128]}'
dmxctl ctl fixture set ID --attr pan --value 80
dmxctl ctl command --json '{"cmd":"motion","params":{"shape":"Circle"}}'
dmxctl ctl project save
dmxctl ctl edit --file operations.json [--dry-run]
dmxctl ctl fixture|group|binding|scene|position|effect|pixel|chase|pair|output|auto_preset list
dmxctl ctl ENTITY create --json '{...}'
dmxctl ctl ENTITY update KEY --json '{...}'
dmxctl ctl ENTITY delete KEY
dmxctl ctl ENTITY update --index N --json '{...}'
dmxctl ctl scene capture NAME [--json '{"fade":1,"fixtures":[1]}']
dmxctl ctl settings update --json '{"bpm":128}'
dmxctl ctl profile list|save|delete [--file FILE] [--dry-run]
dmxctl ctl project diff
dmxctl ctl project reload [--file FILE] [--dry-run]
dmxctl ctl project restore --file FILE [--dry-run]
dmxctl ctl output snapshot [--fixture ID] [--attrs pan,tilt]
dmxctl ctl watch [--fixture ID] [--attrs pan,tilt] [--duration SECONDS]
dmxctl ctl record start --file NEW.jsonl
dmxctl ctl record stop
dmxctl ctl schema [ENTITY]
dmxctl ctl version
dmxctl ctl explain --fixture ID --attr pan
dmxctl ctl assert --fixture ID --attr pan --equals 80 [--tolerance 0.1]
dmxctl ctl wait --frames 2 | --beats 4 | --ms 100
dmxctl ctl wait --fixture ID --attr pan --min 80 [--timeout-ms 5000]
dmxctl ctl fixture set ID --attr pan --value 80 --wait
dmxctl simulate --project FILE [--scenario FILE] [--duration SECONDS] [--log NEW.jsonl]
dmxctl run --project FILE [--scenario FILE] [--duration SECONDS] [--stdin] [--log NEW.jsonl] [--output]
dmxctl watch --project FILE [--duration SECONDS] [--stdin] [--log NEW.jsonl]
dmxctl log summarize FILE
dmxctl log scenario FILE --out NEW_DIRECTORY
dmxctl log query FILE [--from-ms N] [--to-ms N] [--universe N] [--address N] [--fixture N] [--attrs pan,tilt]
dmxctl diff BEFORE.jsonl AFTER.jsonl

All commands return JSON/JSONL. simulate advances a virtual 40 Hz clock without waiting.
run/watch stream at real time; --stdin accepts one command JSON object per line.
simulate/run/watch open no MIDI/OSC/NDI/Link input and never save the source project.
Only run --output or serve --output live sends to configured DMX outputs.
serve defaults to configured inputs; use --inputs none for an isolated session.
ARM and safety definitions remain read-only through CLI controls.
Scenario: [{"at_ms":0,"command":{"cmd":"set","fixture":1,"attr":"pan","value":50}}]
Commands: set, effect, motion, blackout, master, fade, flash, bpm, auto, auto_resume,
preset, look, color, swap, scene, tap, resync. id is optional and echoed in the log.
"#;

struct Args { positional: Vec<String>, options: BTreeMap<String, String>, stdin: bool, output: bool }
impl Args {
    fn parse(args: Vec<String>) -> anyhow::Result<Self> {
        let mut result = Self { positional: Vec::new(), options: BTreeMap::new(), stdin:false, output:false };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--stdin" => result.stdin = true,
                "--output" => result.output = true,
                s if s.starts_with("--") => {
                    ensure!(["--project","--fixtures","--scenario","--duration","--log","--out","--from-ms","--to-ms","--universe","--address","--fixture","--attrs"].contains(&s), "unknown option: {s}");
                    let value = args.next().context("option needs a value")?;
                    ensure!(result.options.insert(arg, value).is_none(), "duplicate option");
                }
                _ => result.positional.push(arg),
            }
        }
        Ok(result)
    }
    fn path(&self, key: &str) -> Option<&Path> { self.options.get(key).map(Path::new) }
    fn number(&self, key: &str, default: u64) -> anyhow::Result<u64> {
        self.options.get(key).map(|v| v.parse().map_err(anyhow::Error::from)).unwrap_or(Ok(default))
    }
    fn allow(&self, keys: &[&str]) -> anyhow::Result<()> {
        for key in self.options.keys() { ensure!(keys.contains(&key.as_str()), "option is not supported by this command: {key}"); }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Event { at_ms: u64, command: Value }

/// 標準出力を読む側がパイプを閉じたかを判定する
pub(crate) fn stdout_closed(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::BrokenPipe
        || (cfg!(windows) && error.raw_os_error() == Some(232))
}

pub(crate) fn stdout_line(line: std::fmt::Arguments<'_>) -> std::io::Result<()> {
    let mut out = std::io::stdout().lock();
    match writeln!(out, "{line}").and_then(|_| out.flush()) {
        Err(error) if stdout_closed(&error) => Ok(()),
        result => result,
    }
}

pub fn run(args: Vec<String>) -> anyhow::Result<()> {
    if args.is_empty() || args.iter().any(|s| s == "--help" || s == "-h") {
        stdout_line(format_args!("{HELP}"))?;
        return Ok(());
    }
    if args[0] == "serve" {
        let launch = crate::control::Launch::parse(&args[1..],true)?;
        engine::install_safety_hooks();
        return crate::host::Host::serve(launch);
    }
    if args[0] == "carabiner" {
        ensure!(args.len() == 2 && args[1] == "install", "usage: dmxctl carabiner install");
        let path = dmxctl_core::io::carabiner::install()?;
        stdout_line(format_args!("{}", serde_json::json!({"installed": path.display().to_string(), "version": dmxctl_core::io::carabiner::VERSION})))?;
        return Ok(());
    }
    if args[0] == "ctl" { return crate::control::client(&args[1..]); }
    if args[0] == "mcp" { return crate::mcp::run(&args[1..]); }
    let args = Args::parse(args)?;
    let command = args.positional.first().context("missing command")?.as_str();
    if command == "log" { return log_command(&args); }
    if command == "diff" { return diff(&args); }
    ensure!(["inspect","simulate","run","watch"].contains(&command), "unknown command");
    if command == "inspect" { args.allow(&["--project","--fixtures"])?; }
    else { args.allow(&["--project","--fixtures","--scenario","--duration","--log"])?; }
    ensure!(args.positional.len() == 1, "unexpected argument");
    ensure!(!args.output || command == "run", "--output is only available with run");
    ensure!(!args.stdin || command == "run" || command == "watch", "--stdin requires real time run/watch");
    let project_path = args.path("--project").context("--project is required")?;
    let dir = args.path("--fixtures").map(PathBuf::from).unwrap_or_else(|| project_path.parent().unwrap_or(Path::new(".")).join("fixtures"));
    let (lib, errors) = Library::load(&dir);
    ensure!(errors.is_empty(), "fixture errors: {}", errors.join("; "));
    let source = std::fs::read_to_string(project_path)?;
    let mut project: Project = serde_json::from_str(source.trim_start_matches('\u{feff}'))?;
    validate(&project, &lib)?;
    // Do not start external clocks or inputs, even if saved as enabled.
    project.tempo_source = TempoSource::Internal;
    let mut core = Core::new(project, Arc::new(lib));
    if command == "inspect" {
        let fixtures: Vec<Value> = core.project.fixtures.iter().map(|f| {
            let mode = core.lib.mode(&f.profile, f.mode).unwrap();
            json!({"id":f.id,"name":f.name,"profile":f.profile,"universe":f.universe+1,
                "address":f.address,"armed":false,"limits":f.limits,"channels":mode.channels})
        }).collect();
        stdout_line(format_args!("{}", json!({"fixtures":fixtures,"project":crate::editing::redact(&serde_json::to_value(&core.project)?),"commands":["set","effect","motion","blackout","master","fade","flash","bpm","auto","auto_resume","preset","look","color","swap","scene","tap","resync"]})))?;
        return Ok(());
    }
    let seconds: f64 = args.options.get("--duration").map(|s| s.parse()).transpose()?.unwrap_or(10.0);
    ensure!(seconds.is_finite() && seconds > 0.0 && seconds <= 86400.0, "duration must be in (0, 86400] seconds");
    let ticks = (seconds * 40.0).ceil() as u64;
    let events: Vec<Event> = args.path("--scenario").map(|p| -> anyhow::Result<Vec<Event>> {
        let text = std::fs::read_to_string(p)?;
        Ok(serde_json::from_str(text.trim_start_matches('\u{feff}'))?)
    }).transpose()?.unwrap_or_default();
    ensure!(events.windows(2).all(|w| w[0].at_ms <= w[1].at_ms), "scenario must be ordered by at_ms");
    ensure!(events.iter().all(|e| e.at_ms < ticks * 25), "scenario event is beyond duration");
    let mut check = Core::new(core.project.clone(), core.lib.clone());
    let check_origin=check.rt.t0;
    for event in &events {
        check.rt.virtual_now=Some(check_origin+Duration::from_millis(event.at_ms));
        check.expire_remote_controls(check.now());
        apply(&mut check, &event.command)?;
    }
    let recording = Recording::start(args.path("--log"), true, command == "simulate")?;
    let mut metadata = serde_json::to_value(&core.project)?;
    metadata["web_pin"] = json!("");
    recording.recorder.emit(json!({"type":"start","version":1,"hz":40,"output":args.output,
        "clock":if command == "simulate" {"virtual"} else {"real"},
        "project":metadata,"profiles":core.lib.profiles.iter().map(|p|p.as_ref()).collect::<Vec<_>>(),
        "started_unix_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64}));
    let origin = Instant::now();
    core.rt.t0 = origin;
    core.rt.blt_seen = origin;
    core.rt.last_beat_t = Some(origin);
    core.rt.observe.set_recorder(Some(recording.recorder.clone()));
    let shared = Arc::new(Mutex::new(core));
    let (tx, rx) = mpsc::sync_channel(256);
    if args.stdin {
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines() {
                // 空行(と空白だけの行)は読み飛ばす
                if line.as_ref().is_ok_and(|s| s.trim().is_empty()) { continue; }
                let message = line.map_err(|e| e.to_string()).and_then(|s| serde_json::from_str::<Value>(&s).map_err(|e|e.to_string()));
                if tx.send(message).is_err() { break; }
            }
        });
    }
    let mut next_event = 0;
    // Apply time-zero operations before the real output thread can render.
    {
        let mut core = engine::lock(&shared);
        if command == "simulate" { core.rt.virtual_now = Some(origin); }
        while let Some(event) = events.get(next_event).filter(|e|e.at_ms == 0) {
            record_command(&mut core, &event.command, 0);
            next_event += 1;
        }
    }
    let mut engine = if args.output {
        engine::install_safety_hooks();
        Some(Engine::spawn(shared.clone()))
    } else { None };
    for tick in 0..ticks {
        let elapsed = if command == "simulate" { tick * 25 } else { origin.elapsed().as_millis() as u64 };
        {
            let mut core = engine::lock(&shared);
            if command == "simulate" { core.rt.virtual_now = Some(origin + Duration::from_millis(elapsed)); }
            while let Some(event) = events.get(next_event).filter(|e|e.at_ms <= elapsed) {
                record_command(&mut core, &event.command, elapsed);
                next_event += 1;
            }
            // Bounded work per tick, even if stdin is continuously supplied.
            for message in rx.try_iter().take(64) {
                match message {
                    Ok(value) => record_command(&mut core, &value, elapsed),
                    Err(error) => recording.recorder.emit(json!({"type":"command_error","t_ms":elapsed,"error":error})),
                }
            }
            if !args.output { core.render(); }
        }
        if command != "simulate" {
            std::thread::sleep((origin + TICK * (tick + 1) as u32).saturating_duration_since(Instant::now()));
        }
    }
    if let Some(engine) = engine.as_mut() {
        engine::emergency_stop(false);
        std::thread::sleep(Duration::from_millis(350));
        engine.stop();
    }
    {
        let mut core = engine::lock(&shared);
        recording.recorder.emit(json!({"type":"complete","t_ms":if command == "simulate" { ticks*25 } else { origin.elapsed().as_millis() as u64 },"outputs":core.rt.output_status}));
        core.rt.observe.set_recorder(None);
    }
    recording.finish()
}

fn record_command(core: &mut Core, value: &Value, elapsed: u64) {
    let result = apply(core, value);
    if let Some(recorder) = core.rt.observe.recorder() {
        recorder.emit(json!({"type":if result.is_ok(){"command"}else{"command_error"},
            "id":value.get("id").cloned().unwrap_or_else(||json!(format!("command-{}",recorder.next_sequence()))),"t_ms":elapsed,
            "applies_from_frame":recorder.next_frame(),
            "command":crate::editing::redact(value),"error":result.err().map(|e| e.to_string())}));
    }
}

fn read_log(path: &str) -> anyhow::Result<Vec<Value>> {
    std::io::BufReader::new(std::fs::File::open(path)?).lines().enumerate().map(|(i,line)| {
        let line = line?;
        serde_json::from_str(&line).with_context(|| format!("invalid log line {}",i+1))
    }).collect()
}

fn log_command(args: &Args) -> anyhow::Result<()> {
    ensure!(!args.output && !args.stdin, "invalid log flags");
    let mode = args.positional.get(1).context("summarize or query required")?;
    let path = args.positional.get(2).context("log path required")?;
    ensure!(args.positional.len()==3, "unexpected argument");
    let events = read_log(path)?;
    if mode == "scenario" {
        args.allow(&["--out"])?;
        return export_scenario(&events,args.path("--out").context("--out NEW_DIRECTORY is required")?);
    } else if mode == "summarize" {
        args.allow(&[])?;
        let frames: Vec<_> = events.iter().filter(|e|e["type"]=="frame").collect();
        stdout_line(format_args!("{}",json!({"frames":frames.len(),"last_t_ms":frames.last().map(|v|&v["t_ms"]),
            "commands":events.iter().filter(|e|e["type"]=="command").count(),
            "command_errors":events.iter().filter(|e|e["type"]=="command_error").count(),
            "state_changes":events.iter().filter(|e|e["type"]=="state_change").count(),
            "safety_corrections":frames.iter().map(|v|v["safety"].as_array().map_or(0,Vec::len)).sum::<usize>(),
            "complete":events.iter().any(|e|e["type"]=="complete"),
            "closed":events.last().is_some_and(|e|e["type"]=="end"),
            "sequence_gaps":events.iter().filter_map(|v|v["seq"].as_u64()).scan(0u64,|next,seq| {
                let gap=seq.saturating_sub(*next); *next=seq+1; Some(gap)
            }).sum::<u64>(),
            "dropped":events.last().filter(|e|e["type"]=="end").map(|e|&e["dropped"])})))?;
    } else if mode == "query" {
        args.allow(&["--from-ms","--to-ms","--universe","--address","--fixture","--attrs"])?;
        let from = args.number("--from-ms",0)?;
        let to = args.number("--to-ms",u64::MAX)?;
        ensure!(from <= to, "invalid time interval");
        let universe = args.number("--universe",0)?;
        let address = args.number("--address",0)?;
        let fixture = args.number("--fixture",0)?;
        let attrs:Vec<&str> = args.options.get("--attrs").map(|s|s.split(',').collect()).unwrap_or_default();
        let stdout = std::io::stdout();
        let mut out = stdout.lock();
        for mut event in events {
            let Some(t) = event["t_ms"].as_u64() else {continue};
            if t < from || t > to {continue;}
            if universe != 0 || address != 0 || fixture != 0 || !attrs.is_empty() {
                if event["type"] != "frame" {continue;}
                if let Some(changes) = event["changes"].as_array_mut() {
                    changes.retain(|v| (universe==0 || v["universe"]==universe) && (address==0 || v["address"]==address)
                        && (fixture==0 || v["fixture"]==fixture)
                        && (attrs.is_empty() || v["attr"].as_str().is_some_and(|s|attrs.contains(&s))));
                }
                event["filtered"] = json!(true);
            }
            if let Err(error) = writeln!(out, "{event}").and_then(|_| out.flush()) {
                if stdout_closed(&error) { break; }
                return Err(error.into());
            }
        }
    } else {bail!("unknown log command");}
    Ok(())
}

fn export_scenario(events:&[Value],out:&Path)->anyhow::Result<()> {
    ensure!(events.last().is_some_and(|v|v["type"]=="end" && v["dropped"]==0)
        && events.iter().any(|v|v["type"]=="complete"),"scenario export requires a completed recording without dropped events");
    ensure!(events.iter().filter(|v|v.get("seq").is_some()).enumerate().all(|(i,v)|v["seq"]==i as u64),"log sequence gap");
    ensure!(!events.iter().any(|v|v["type"]=="state_change" || v["type"]=="command_error" || v["learning"]==true),
        "recording contains non-replayable state changes, input learning or failed commands; inspect the history first");
    let start=events.first().filter(|v|v["type"]=="start" && v["version"]==1).context("unsupported log header")?;
    let project:Project=serde_json::from_value(start["project"].clone())?;
    let profiles:Vec<dmxctl_core::fixture::Profile>=serde_json::from_value(start["profiles"].clone())?;
    let mut library=Library::default();
    for profile in &profiles {library.add(profile.clone());}
    validate(&project,&library)?;
    let mut core=Core::new(project,Arc::new(library));
    let origin=core.rt.t0;
    let started=start["t_ms"].as_u64().unwrap_or(0);
    let mut scenario=Vec::new();
    let mut last=0;
    for event in events.iter().filter(|e|e["type"]=="command") {
        if ["lease_expired","project_edit"].iter().any(|s|event["source"]==*s) {continue;}
        let at=event["t_ms"].as_u64().context("command has no timestamp")?.checked_sub(started).context("command precedes recording start")?;
        ensure!(at>=last,"command timestamps are not ordered");last=at;
        core.rt.virtual_now=Some(origin+Duration::from_millis(at));
        core.expire_remote_controls(core.now());
        apply(&mut core,&event["command"]).context("recorded command cannot be safely replayed")?;
        scenario.push(json!({"at_ms":at,"command":crate::editing::redact(&event["command"])}));
    }
    let duration=events.iter().filter_map(|v|v["t_ms"].as_u64()).max().unwrap_or(started).saturating_sub(started).max(last+25);
    // Validate everything before creating a new, self-contained directory.
    std::fs::create_dir(out).context("export directory must not already exist")?;
    std::fs::create_dir(out.join("fixtures"))?;
    for (i,profile) in profiles.iter().enumerate() {
        std::fs::write(out.join("fixtures").join(format!("{i}.json")),serde_json::to_vec_pretty(profile)?)?;
    }
    std::fs::write(out.join("project.json"),serde_json::to_vec_pretty(&start["project"])?)?;
    std::fs::write(out.join("scenario.json"),serde_json::to_vec_pretty(&scenario)?)?;
    let manifest=json!({"commands":scenario.len(),"duration_seconds":duration as f64/1000.0,
        "source_clock":start["clock"],"runtime":"fresh internal clock; live beat phase, active fades and input service state are not restored",
        "project":"project.json","scenario":"scenario.json","fixtures":"fixtures"});
    std::fs::write(out.join("manifest.json"),serde_json::to_vec_pretty(&manifest)?)?;
    stdout_line(format_args!("{}",json!({"directory":out,"manifest":manifest})))?;
    Ok(())
}

fn diff(args: &Args) -> anyhow::Result<()> {
    args.allow(&[])?;
    ensure!(args.positional.len()==3 && !args.output && !args.stdin, "diff needs two logs");
    let a = read_log(&args.positional[1])?;
    let b = read_log(&args.positional[2])?;
    for log in [&a,&b] {
        ensure!(log.last().is_some_and(|v|v["type"]=="end" && v["dropped"]==0)
            && log.iter().any(|v|v["type"]=="complete"), "diff requires complete logs with no dropped events");
        ensure!(log.iter().filter(|v|v.get("seq").is_some()).enumerate().all(|(i,v)|v["seq"]==i as u64), "log sequence gap");
    }
    let frames = |events: Vec<Value>| -> Vec<Value> { events.into_iter().filter(|e|e["type"]=="frame").collect() };
    let (a,b)=(frames(a),frames(b));
    let mut state_a = BTreeMap::new();
    let mut state_b = BTreeMap::new();
    let mut differences = Vec::new();
    for i in 0..a.len().max(b.len()) {
        for (frame,state) in [(a.get(i),&mut state_a),(b.get(i),&mut state_b)] {
            if let Some(changes) = frame.and_then(|f|f["changes"].as_array()) {
                for c in changes { state.insert((c["universe"].as_u64().unwrap_or(0),c["address"].as_u64().unwrap_or(0)),c["value"].clone()); }
            }
        }
        if state_a != state_b || a.get(i).is_none() || b.get(i).is_none() {
            let keys:std::collections::BTreeSet<_> = state_a.keys().chain(state_b.keys()).copied().collect();
            let changes:Vec<_> = keys.into_iter().filter(|k|state_a.get(k)!=state_b.get(k)).map(|(u,ch)|json!({
                "universe":u,"address":ch,"before":state_a.get(&(u,ch)),"after":state_b.get(&(u,ch))})).collect();
            differences.push(json!({"frame_index":i,"before_t_ms":a.get(i).map(|f|&f["t_ms"]),"after_t_ms":b.get(i).map(|f|&f["t_ms"]),"changes":changes}));
        }
    }
    stdout_line(format_args!("{}",json!({"alignment":"recorded_frame_index","differences":differences})))?;
    Ok(())
}

#[cfg(test)]
mod closed_output_tests {
    #[test]
    fn only_closed_output_is_quiet() {
        use std::io::{Error, ErrorKind};
        assert!(super::stdout_closed(&Error::from(ErrorKind::BrokenPipe)));
        assert!(!super::stdout_closed(&Error::from(ErrorKind::PermissionDenied)));
        #[cfg(windows)]
        assert!(super::stdout_closed(&Error::from_raw_os_error(232)));
    }
}
