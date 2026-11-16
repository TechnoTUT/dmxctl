use dmxctl::{engine::{Core, guard_values}, fixture::Library, project::Project};
use serde_json::{Value, json};
use std::{path::{Path, PathBuf}, process::Command, sync::{Arc, atomic::{AtomicU64, Ordering}}};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("dmxctl-cli-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn file(&self, name: &str) -> PathBuf {self.0.join(name)}
    fn project(&self) -> Project {
        let (lib,_) = Library::load(&self.file("fixtures"));
        let project = Project::starter(&lib);
        project.save(&self.file("project.json")).unwrap();
        project
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_dmxctl-cli")).args(args).current_dir(&self.0).output().unwrap()
    }
}
impl Drop for Sandbox {fn drop(&mut self) {let _ = std::fs::remove_dir_all(&self.0);}}
fn events(output: &std::process::Output) -> Vec<Value> {
    assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).lines().map(|l|serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn simulation_is_deterministic_and_logs_safety_and_queries() {
    let dir = Sandbox::new();
    let mut project=dir.project();
    project.web_pin="private-test-pin".into();
    project.fixtures[1].limits.insert("pan".into(),[20,90]);
    project.save(&dir.file("project.json")).unwrap();
    std::fs::write(dir.file("scenario.json"),serde_json::to_vec(&json!([
        {"at_ms":0,"command":{"cmd":"set","id":"pan-change","fixture":1,"attr":"pan","value":50}},
        {"at_ms":25,"command":{"cmd":"set","fixture":2,"attr":"mode","value":30}},
        {"at_ms":25,"command":{"cmd":"set","fixture":2,"attr":"pan","value":200}},
        {"at_ms":50,"command":{"cmd":"motion","params":{"shape":"Circle","beats":2,"size":0.2}}},
        {"at_ms":75,"command":{"cmd":"blackout","on":true}}
    ])).unwrap()).unwrap();
    let first = dir.run(&["simulate","--project","project.json","--scenario","scenario.json","--duration","0.2","--log","a.jsonl"]);
    let a = events(&first);
    let b = events(&dir.run(&["simulate","--project","project.json","--scenario","scenario.json","--duration","0.2","--log","b.jsonl"]));
    let frames = |v: Vec<Value>| v.into_iter().filter(|v|v["type"]=="frame").collect::<Vec<_>>();
    assert_eq!(frames(a.clone()),frames(b));
    assert_eq!(frames(a.clone()).len(),8);
    assert!(!String::from_utf8_lossy(&first.stdout).contains("private-test-pin"));
    assert!(a.iter().any(|v|v["type"]=="command" && v["id"]=="pan-change"));
    assert!(frames(a.clone()).iter().any(|f|f["safety"].as_array().unwrap().iter().any(|s|s["reason"]=="not_armed")));
    assert!(frames(a).iter().any(|f|f["safety"].as_array().unwrap().iter().any(|s|s["reason"]=="limits" && s["attr"]=="pan")));
    assert_eq!(first.stdout,std::fs::read(dir.file("a.jsonl")).unwrap());
    let summary = events(&dir.run(&["log","summarize","a.jsonl"]));
    assert_eq!(summary[0]["dropped"],0);
    assert_eq!(summary[0]["complete"],true);
    assert_eq!(summary[0]["frames"],8);
    let query = events(&dir.run(&["log","query","a.jsonl","--from-ms","50","--to-ms","100","--universe","1","--address","1"]));
    assert!(!query.is_empty());
    for row in query {
        assert!((50..=100).contains(&row["t_ms"].as_u64().unwrap()));
        for change in row["changes"].as_array().unwrap() {assert_eq!(change["address"],1);}
    }
    assert_eq!(events(&dir.run(&["diff","a.jsonl","b.jsonl"]))[0]["differences"],json!([]));
    events(&dir.run(&["simulate","--project","project.json","--duration","0.2","--log","c.jsonl"]));
    let comparison=events(&dir.run(&["diff","a.jsonl","c.jsonl"]));
    assert!(comparison[0]["differences"].as_array().unwrap().iter()
        .any(|f| f["changes"].as_array().unwrap().iter().any(|c|c["address"]==1 && c["before"]!=c["after"])));
}

#[test]
fn safety_configuration_is_read_only_and_files_are_not_overwritten() {
    let dir = Sandbox::new();
    dir.project();
    let original = std::fs::read(dir.file("project.json")).unwrap();
    for cmd in [json!({"cmd":"arm","fixture":2,"on":true}),json!({"cmd":"motion","params":{"tilt_up":0.1}}),json!({"cmd":"limits","fixture":2})] {
        std::fs::write(dir.file("bad.json"),serde_json::to_vec(&json!([{"at_ms":0,"command":cmd}])).unwrap()).unwrap();
        assert!(!dir.run(&["simulate","--project","project.json","--scenario","bad.json"]).status.success());
    }
    assert!(!dir.run(&["simulate","--project","project.json","--output"]).status.success());
    assert!(!dir.run(&["simulate","--project","project.json","--log","project.json"]).status.success());
    assert_eq!(original,std::fs::read(dir.file("project.json")).unwrap());
}

#[test]
fn real_time_stdin_controls_next_frame_without_device_output() {
    use std::io::Write;
    use std::process::Stdio;
    let dir = Sandbox::new();
    dir.project();
    let mut child = Command::new(env!("CARGO_BIN_EXE_dmxctl-cli"))
        .args(["watch","--project","project.json","--duration","0.2","--stdin","--log","live.jsonl"])
        .current_dir(&dir.0).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"{\"cmd\":\"set\",\"id\":\"live\",\"fixture\":1,\"attr\":\"pan\",\"value\":77}\n").unwrap();
    let log = events(&child.wait_with_output().unwrap());
    assert_eq!(log[0]["output"],false);
    assert!(log.iter().any(|v|v["id"]=="live"));
    assert!(log.iter().filter(|v|v["type"]=="frame").flat_map(|v|v["changes"].as_array().unwrap()).any(|v|v["universe"]==1 && v["address"]==1 && v["value"]==77));
}

#[test]
fn virtual_clock_advances_scene_fades_without_sleeping() {
    let (lib,_) = Library::load(Path::new("none"));
    let lib=Arc::new(lib);
    let mut core=Core::new(Project::starter(&lib),lib);
    let start=core.rt.t0;
    core.rt.virtual_now=Some(start);
    core.rt.last_beat_t=Some(start);
    core.fade_to_black(true,1.0);
    core.render();
    for tick in 1..=40 {
        core.rt.virtual_now=Some(start+std::time::Duration::from_millis(tick*25));
        core.render();
    }
    assert!(core.rt.dim <= 0.001);
    assert!((core.rt.beat-2.0).abs()<0.001);
}

#[test]
fn guard_preserves_limits_when_safe_replacement_is_outside() {
    let (lib,_) = Library::load(Path::new("none"));
    let mut mode=lib.mode("stage-evolution/laserscan300",0).unwrap().clone();
    mode.channels[0].forbid=vec![[30,40]];
    mode.channels[0].safe=Some(0);
    let mut limits=std::collections::BTreeMap::new();
    limits.insert(mode.channels[0].attr.clone(),[20,50]);
    let mut values=vec![35;mode.channels.len()];
    guard_values(&mode,&limits,&mut values);
    assert!((20..=50).contains(&values[0]));
    assert!(!mode.channels[0].is_forbidden(values[0]));
}

#[test]
fn explicit_output_sends_and_logs_safe_shutdown_frames() {
    use dmxctl::project::{OutputCfg, OutputKind};
    let receiver = std::net::UdpSocket::bind("127.0.0.1:6454").expect("loopback Art-Net test port is required");
    receiver.set_read_timeout(Some(std::time::Duration::from_millis(100))).unwrap();
    let dir = Sandbox::new();
    let mut project = dir.project();
    project.fixtures[0].values[5]=180;
    project.outputs.push(OutputCfg { enabled:true, universe:0, kind:OutputKind::ArtNet { ip:"127.0.0.1".into(), art_universe:0 } });
    project.save(&dir.file("project.json")).unwrap();
    let log = events(&dir.run(&["run","--project","project.json","--duration","0.1","--output","--log","output.jsonl"]));
    assert_eq!(log[0]["output"],true);
    let emergency:Vec<_>=log.iter().filter(|v|v["type"]=="frame" && v["emergency"]==true).collect();
    assert!(!emergency.is_empty());
    for frame in emergency {
        let changes=frame["changes"].as_array().unwrap();
        assert!(changes.iter().any(|v|v["address"]==6 && v["value"]==0));
        assert!(changes.iter().any(|v|v["address"]==10 && v["value"]==0));
    }
    let mut dimmers=Vec::new();
    let mut buffer=[0u8;600];
    while let Ok((n,_))=receiver.recv_from(&mut buffer) {
        assert!(n >= 44);
        assert_eq!(&buffer[..8],b"Art-Net\0");
        dimmers.push(buffer[18+5]);
        assert_eq!(buffer[18+9],0,"CLI must never ARM the laser");
    }
    assert!(dimmers.contains(&180));
    assert_eq!(dimmers.last(),Some(&0));
}

#[test]
fn impossible_limits_and_blackout_conflicts_are_rejected() {
    let dir=Sandbox::new();
    let mut project=dir.project();
    project.fixtures[1].limits.insert("pan".into(),[128,255]);
    project.save(&dir.file("project.json")).unwrap();
    assert!(!dir.run(&["simulate","--project","project.json"]).status.success());
    project.fixtures[1].limits.clear();
    project.fixtures[0].limits.insert("dimmer".into(),[100,255]);
    project.save(&dir.file("project.json")).unwrap();
    assert!(!dir.run(&["simulate","--project","project.json"]).status.success());
}
