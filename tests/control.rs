use dmxctl::backend::control;
use dmxctl::core::{fixture::Library, project::{Project, OutputCfg, OutputKind}};
use serde_json::{Value, json};
use std::{io::{BufRead, BufReader}, net::SocketAddr, path::PathBuf, process::{Child, Command, Stdio},
    sync::{atomic::{AtomicU64, Ordering}, mpsc}, time::{Duration, Instant}};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Running { child: Child, dir: PathBuf, address: SocketAddr }
impl Running {
    fn start() -> Self {
        let dir = std::env::temp_dir().join(format!("dmxctl-control-{}-{}",std::process::id(),NEXT.fetch_add(1,Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).unwrap();
        let (lib,_) = Library::load(&dir.join("fixtures"));
        let mut project=Project::starter(&lib);
        project.fixtures[1].limits.insert("pan".into(),[20,90]);
        project.outputs.push(OutputCfg {enabled:true,universe:0,kind:OutputKind::ArtNet{ip:"127.0.0.1".into(),art_universe:0}});
        project.midi_input="preserve this device".into();
        project.save(&dir.join("project.json")).unwrap();
        let mut child=Command::new(env!("CARGO_BIN_EXE_dmxctl"))
            .args(["serve","--project","project.json","--listen","127.0.0.1:0","--inputs","none"])
            .current_dir(&dir).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let stdout=child.stdout.take().unwrap();
        let (tx,rx)=mpsc::channel();
        std::thread::spawn(move || {
            let mut line=String::new();
            let result=BufReader::new(stdout).read_line(&mut line).map(|_|line);
            let _=tx.send(result);
        });
        let line=rx.recv_timeout(Duration::from_secs(20));
        let ready:Value=match line {
            Ok(Ok(line)) => serde_json::from_str(&line).unwrap_or_else(|error| {let _=child.kill(); panic!("bad ready message {line}: {error}")}),
            error => {let _=child.kill(); panic!("host did not start: {error:?}")},
        };
        assert_eq!(ready["ready"],true);
        assert_eq!(ready["output_enabled"],false);
        assert_eq!(ready["autosave"],false);
        let address=ready["listen"].as_str().unwrap().parse().unwrap();
        Self {child,dir,address}
    }
    fn request(&self,value:Value) -> Value { control::request(self.address,&value).unwrap() }
    fn cli(&self,args:&[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_dmxctl")).args(["ctl","--connect",&self.address.to_string()])
            .args(args).current_dir(&self.dir).output().unwrap()
    }
    fn stop(&mut self) {
        self.request(json!({"op":"stop"}));
        let deadline=Instant::now()+Duration::from_secs(3);
        loop {
            if let Some(status)=self.child.try_wait().unwrap() {assert!(status.success());break;}
            assert!(Instant::now()<deadline,"host did not shut down");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
impl Drop for Running {
    fn drop(&mut self) {let _=self.child.kill();let _=self.child.wait();let _=std::fs::remove_dir_all(&self.dir);}
}

#[test]
fn concurrent_readers_reconnect_while_commands_keep_rendering() {
    let mut host=Running::start();
    let start=host.request(json!({"op":"clock"}))["frame"].as_u64().unwrap();
    let readers:Vec<_>=(0..4).map(|_|{
        let address=host.address;
        std::thread::spawn(move || {for _ in 0..15 {
            assert!(control::request(address,&json!({"op":"output","fixture":1})).unwrap()["frame"].as_u64().is_some());
        }})
    }).collect();
    for n in 0..12 {host.request(json!({"op":"command","command":{"cmd":"set","fixture":1,"attr":"pan","value":n}}));}
    for reader in readers {reader.join().unwrap();}
    assert!(host.request(json!({"op":"clock"}))["frame"].as_u64().unwrap()>start);
    host.request(json!({"op":"save"}));
    host.request(json!({"op":"reload"}));
    assert_eq!(host.request(json!({"op":"inspect"}))["project"]["fixtures"][0]["values"][0],11);
    host.stop();
}

#[test]
fn persistent_host_changes_the_live_core_and_saves_only_on_request() {
    let mut host=Running::start();
    let original=std::fs::read(host.dir.join("project.json")).unwrap();
    let before=host.request(json!({"op":"status"}));
    assert_eq!(before["inputs_enabled"],false);
    let result=host.request(json!({"op":"command","command":{"cmd":"set","fixture":1,"attr":"pan","value":77}}));
    assert!(result["takes_effect_from_frame"].as_u64().unwrap()>0);
    std::thread::sleep(Duration::from_millis(80));
    let snapshot=host.request(json!({"op":"output","fixture":1,"attrs":["pan"]}));
    assert_eq!(snapshot["channels"][0]["value"],77);
    assert!(snapshot["frame"].as_u64().unwrap()>before["frame"].as_u64().unwrap());
    assert_eq!(std::fs::read(host.dir.join("project.json")).unwrap(),original);
    host.request(json!({"op":"save"}));
    let saved=Project::load(&host.dir.join("project.json")).unwrap();
    assert_eq!(saved.fixtures[0].values[0],77);
    assert!(saved.outputs[0].enabled,"output none must not rewrite saved output settings");
    assert_eq!(saved.midi_input,"preserve this device");
    host.stop();
    assert_eq!(Project::load(&host.dir.join("project.json")).unwrap().midi_input,"preserve this device");
}

#[test]
fn recording_observes_live_commands_and_safety_and_closes_on_shutdown() {
    let mut host=Running::start();
    let path=host.dir.join("live.jsonl");
    host.request(json!({"op":"record_start","file":path}));
    host.request(json!({"op":"command","command":{"cmd":"set","fixture":1,"attr":"dimmer","value":180}}));
    host.request(json!({"op":"command","command":{"cmd":"set","fixture":2,"attr":"pan","value":200}}));
    std::thread::sleep(Duration::from_millis(100));
    assert!(control::request(host.address,&json!({"op":"command","command":{"cmd":"arm","fixture":2,"on":true}})).is_err());
    let snapshot=host.request(json!({"op":"output","fixture":2,"attrs":["pan","mode"]}));
    assert!(snapshot["channels"].as_array().unwrap().iter().any(|v|v["attr"]=="pan" && v["value"]==90));
    assert!(snapshot["channels"].as_array().unwrap().iter().any(|v|v["attr"]=="mode" && v["value"]==0));
    host.stop();
    let events:Vec<Value>=std::fs::read_to_string(&path).unwrap().lines().map(|l|serde_json::from_str(l).unwrap()).collect();
    assert_eq!(events.last().unwrap()["type"],"end");
    assert_eq!(events.last().unwrap()["dropped"],0);
    assert!(events.iter().any(|v|v["type"]=="command"));
    assert!(events.iter().any(|v|v["type"]=="command_error"));
    assert!(events.iter().any(|v|v["type"]=="frame" && v["safety"].as_array().unwrap().iter().any(|s|s["reason"]=="limits" && s["applied"]==90)));
    let dimmers:Vec<_>=events.iter().filter(|v|v["type"]=="frame")
        .flat_map(|v|v["changes"].as_array().unwrap()).filter(|v|v["address"]==6).map(|v|v["value"].as_u64().unwrap()).collect();
    assert!(dimmers.contains(&180));
    assert_eq!(dimmers.last(),Some(&0),"safe shutdown must be recorded");
}

#[test]
fn client_commands_and_watch_attach_to_the_same_host() {
    let mut host=Running::start();
    for args in [&["status"][..],&["inspect"][..],&["fixture","list"][..],&["fixture","set","1","--attr","pan","--value","81"][..]] {
        let result=host.cli(args);
        assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));
        serde_json::from_slice::<Value>(&result.stdout).unwrap();
    }
    let result=host.cli(&["watch","--fixture","1","--attrs","pan","--duration","0.16","--interval-ms","50"]);
    assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));
    let samples:Vec<Value>=String::from_utf8_lossy(&result.stdout).lines().map(|l|serde_json::from_str(l).unwrap()).collect();
    assert!(samples.len()>=2);
    assert!(samples.iter().all(|s|s["channels"][0]["value"]==81));
    assert!(samples.last().unwrap()["frame"].as_u64().unwrap()>samples[0]["frame"].as_u64().unwrap());
    assert!(!host.cli(&["status","--duration","2"]).status.success());
    host.stop();
}

#[test]
fn momentary_flash_expires_even_when_the_client_disconnects() {
    let mut host=Running::start();
    // 決まった時間だけ待つと、PCが混んでいるとき(テストを並べて動かすときなど)に間に合わないので、
    // 光るまで・戻るまでを、それぞれ待つ。押している時間は、混んでいても光ったのを見られる長さにする
    let started=std::time::Instant::now();
    host.request(json!({"op":"command","command":{"cmd":"flash","on":true,"hold_ms":1000}}));
    let until_bright=|bright:bool|{
        let deadline=std::time::Instant::now()+Duration::from_secs(5);
        while std::time::Instant::now()<deadline {
            let output=host.request(json!({"op":"output","fixture":1,"attrs":["dimmer"]}));
            let value=output["channels"][0]["value"].as_u64().expect("missing dimmer output");
            if (value==255)==bright {return true;}
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    };
    assert!(until_bright(true),"the flash did not light up");
    assert!(started.elapsed()<Duration::from_millis(1000),"the flash was seen too late to check that it expires");
    assert!(until_bright(false),"the flash did not expire");
    host.stop();
}

#[test]
fn non_local_addresses_are_rejected() {
    assert!(control::Launch::parse(&["--listen".into(),"0.0.0.0:9190".into()],false).is_err());
    assert!(control::client(&["--connect".into(),"192.0.2.1:9190".into(),"status".into()]).is_err());
}

#[test]
fn diagnostics_clients_wait_for_render_assert_and_explain_actual_output() {
    let mut host=Running::start();
    let result=host.cli(&["fixture","set","1","--attr","pan","--value","77","--wait","--timeout-ms","1000"]);
    assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));
    let result:Value=serde_json::from_slice(&result.stdout).unwrap();assert_eq!(result["observed"]["matched"],true);
    assert!(host.cli(&["assert","--fixture","1","--attr","pan","--equals","77"]).status.success());
    let failed=host.cli(&["assert","--fixture","1","--attr","pan","--equals","78"]);
    assert!(!failed.status.success());assert_eq!(serde_json::from_slice::<Value>(&failed.stdout).unwrap()["passed"],false);
    assert!(host.cli(&["wait","--frames","2","--timeout-ms","1000"]).status.success());
    assert!(host.cli(&["wait","--fixture","1","--attr","pan","--equals","77","--timeout-ms","1000"]).status.success());
    assert!(!host.cli(&["wait","--fixture","1","--attr","pan","--equals","78","--timeout-ms","80"]).status.success());
    let explained=host.cli(&["explain","--fixture","1","--attr","pan","--timeout-ms","1000"]);
    assert!(explained.status.success(),"{}",String::from_utf8_lossy(&explained.stderr));
    let explained:Value=serde_json::from_slice(&explained.stdout).unwrap();
    assert_eq!(explained["stages"].as_array().unwrap().last().unwrap()["channels"][0]["value"],77.0);
    assert!(!host.cli(&["fixture","set","1","--attr","pan","--value","99","--wait","--timeout-ms","0"]).status.success());
    assert_eq!(host.request(json!({"op":"output","fixture":1,"attrs":["pan"]}))["channels"][0]["value"],77);
    host.stop();
}

#[test]
fn schema_and_version_guard_clients_prevent_stale_edits() {
    let mut host=Running::start();
    let schema=host.cli(&["schema","motion"]);assert!(schema.status.success());
    let schema:Value=serde_json::from_slice(&schema.stdout).unwrap();assert_eq!(schema["entity"],"motion");
    let revision=schema["version"]["revision"].as_u64().unwrap().to_string();
    let session=schema["version"]["session"].as_str().unwrap();
    let first=host.cli(&["group","create","--json",r#"{"name":"First"}"#,"--expected-revision",&revision,"--expected-session",session]);
    assert!(first.status.success(),"{}",String::from_utf8_lossy(&first.stderr));
    assert!(!host.cli(&["group","create","--json",r#"{"name":"Stale"}"#,"--expected-revision",&revision]).status.success());
    let groups=host.request(json!({"op":"entity_list","entity":"group"}));assert_eq!(groups.as_array().unwrap().len(),1);
    host.stop();
}

#[test]
fn button_and_input_clients_use_persistent_controls() {
    let mut host=Running::start();
    let output=host.cli(&["button","list"]);assert!(output.status.success());
    let controls:Value=serde_json::from_slice(&output.stdout).unwrap();
    let master=controls.as_array().unwrap().iter().find(|v|v["kind"]=="slider").unwrap();
    let id=master["id"].as_str().unwrap();
    assert!(host.cli(&["button","set",id,"--value","0.3"]).status.success());
    assert!((host.request(json!({"op":"status"}))["master"].as_f64().unwrap()-0.3).abs()<0.001);
    for value in ["100","20"] {
        let output=host.cli(&["input","midi","--kind","Cc","--channel","0","--number","56","--value",value]);
        assert!(output.status.success(),"{}",String::from_utf8_lossy(&output.stderr));
    }
    assert!((host.request(json!({"op":"status"}))["master"].as_f64().unwrap()-20.0/127.0).abs()<0.001);
    host.stop();
}

#[test]
fn editing_clients_commit_atomically_and_reload_backs_up_unsaved_work() {
    let mut host=Running::start();
    let dry=host.cli(&["group","create","--json",r#"{"name":"Spots","fixtures":[1]}"#,"--dry-run"]);
    assert!(dry.status.success(),"{}",String::from_utf8_lossy(&dry.stderr));
    assert_eq!(host.request(json!({"op":"entity_list","entity":"group"})),json!([]));
    assert!(control::request(host.address,&json!({"op":"edit","dry_run":"true","operations":[{"entity":"group","op":"create","data":{"name":"Must not apply"}}]})).is_err());
    assert_eq!(host.request(json!({"op":"entity_list","entity":"group"})),json!([]));
    assert!(host.cli(&["group","create","--json",r#"{"name":"Spots","fixtures":[1]}"#]).status.success());
    let invalid=host.request(json!({"op":"inspect"}));
    assert!(control::request(host.address,&json!({"op":"edit","operations":[
        {"entity":"settings","op":"update","data":{"bpm":150}},
        {"entity":"fixture","op":"delete","key":1}
    ]})).is_err());
    assert_eq!(host.request(json!({"op":"inspect"}))["project"],invalid["project"]);
    assert!(host.cli(&["settings","update","--json",r#"{"bpm":128}"#]).status.success());
    let diff=host.cli(&["project","diff"]);assert!(diff.status.success());
    let diff:Value=serde_json::from_slice(&diff.stdout).unwrap();
    assert!(diff["changes"].as_array().unwrap().iter().any(|v|v["path"]=="/bpm"));
    let restored=host.request(json!({"op":"reload"}));
    assert_eq!(host.request(json!({"op":"status"}))["bpm"],120.0);
    let backup=Project::load(std::path::Path::new(restored["backup"].as_str().unwrap())).unwrap();
    assert_eq!(backup.bpm,128.0);assert_eq!(backup.groups[0].name,"Spots");
    assert!(host.cli(&["scene","capture","Intro","--json",r#"{"fixtures":[1],"fade":1}"#]).status.success());
    assert_eq!(host.request(json!({"op":"entity_list","entity":"scene"}))[0]["data"]["name"],"Intro");
    let before=host.request(json!({"op":"inspect"}))["project"].clone();
    std::fs::write(host.dir.join("broken.json"),"{broken").unwrap();
    assert!(!host.cli(&["project","restore","--file","broken.json"]).status.success());
    assert_eq!(host.request(json!({"op":"inspect"}))["project"],before);
    host.stop();
}

#[test]
fn profiles_are_saved_validated_and_cannot_weaken_laser_guards() {
    use dmxctl::core::fixture::{Profile,Mode,ChannelDef};
    let mut host=Running::start();
    let profile=Profile{id:"user/test".into(),name:"Test".into(),manufacturer:"Test".into(),needs_arm:false,
        modes:vec![Mode{name:"1ch".into(),channels:vec![ChannelDef::blank("Dimmer","dimmer","光量")]}]};
    let text=serde_json::to_string(&profile).unwrap();
    let result=host.cli(&["profile","save","--json",&text,"--dry-run"]);assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));
    assert!(!host.dir.join("fixtures/user_test.json").exists());
    let result=host.cli(&["profile","save","--json",&text]);assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));
    assert!(host.dir.join("fixtures/user_test.json").exists());
    let result=host.cli(&["fixture","create","--json",r#"{"profile":"user/test"}"#]);assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));
    let mut changed=profile.clone();
    changed.modes[0].channels.push(ChannelDef::blank("Red","red","色"));
    assert!(control::request(host.address,&json!({"op":"profile_save","data":changed})).is_err(),"in-use channel layout must remain consistent with saved values");
    host.request(json!({"op":"save"}));
    assert!(!host.cli(&["profile","delete","user/test"]).status.success(),"in-use profile must not disappear");
    assert!(host.cli(&["fixture","delete","3"]).status.success());
    assert!(!host.cli(&["profile","delete","user/test"]).status.success(),"saved project still references this profile");
    host.request(json!({"op":"save"}));
    assert!(host.cli(&["profile","delete","user/test"]).status.success());
    assert!(!host.dir.join("fixtures/user_test.json").exists());
    let profiles=host.request(json!({"op":"profile_list"}));
    let mut laser=profiles.as_array().unwrap().iter().find(|p|p["id"]=="stage-evolution/laserscan300").unwrap().clone();
    laser["needs_arm"]=json!(false);
    assert!(control::request(host.address,&json!({"op":"profile_save","data":laser})).is_err());
    assert!(!host.dir.join("fixtures/stage_evolution_laserscan300.json").exists());
    host.stop();
}

#[test]
fn disabled_output_keeps_rendering_without_sending_artnet() {
    let receiver=std::net::UdpSocket::bind("127.0.0.1:6454").unwrap();
    receiver.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let mut host=Running::start();
    let first=host.request(json!({"op":"status"}));
    host.request(json!({"op":"command","command":{"cmd":"set","fixture":1,"attr":"dimmer","value":200}}));
    let mut buffer=[0u8;600];
    let error=receiver.recv_from(&mut buffer).unwrap_err();
    assert!([std::io::ErrorKind::WouldBlock,std::io::ErrorKind::TimedOut].contains(&error.kind()));
    let later=host.request(json!({"op":"status"}));
    assert!(later["frame"].as_u64().unwrap()>first["frame"].as_u64().unwrap());
    host.stop();
}

#[test]
fn gui_frame_processes_commands_against_its_own_core() {
    let dir=std::env::temp_dir().join(format!("dmxctl-gui-control-{}",std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (lib,_)=Library::load(&dir.join("fixtures"));
    Project::starter(&lib).save(&dir.join("project.json")).unwrap();
    let reservation=std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address=reservation.local_addr().unwrap();
    drop(reservation);
    let ctx=eframe::egui::Context::default();
    let mut app=dmxctl::ui::app::App::with_options(&ctx,false,control::Launch {
        project:Some(dir.join("project.json")),listen:Some(address),output:false,inputs:false,autosave:false,..Default::default()
    }).unwrap();
    for command in [json!({"cmd":"set","fixture":1,"attr":"pan","value":83}),
        json!({"cmd":"edit","operations":[{"entity":"group","op":"create","data":{"name":"GUI shared edit","fixtures":[1]}}]}),
        json!({"cmd":"flash","on":true,"hold_ms":100})] {
        let (tx,rx)=mpsc::channel();
        let client=std::thread::spawn(move || {let _=tx.send(control::request(address,&json!({
            "op":"command","command":command
        })));});
        let deadline=Instant::now()+Duration::from_secs(3);
        loop {
            let mut frame=ctx.run_ui(Default::default(),|ui|app.frame(ui,None));
            // This test exercises GUI state without a GPU backend.
            frame.textures_delta.clear();
            if let Ok(response)=rx.try_recv() {response.unwrap(); break;}
            assert!(Instant::now()<deadline,"GUI frame did not process control request");
            std::thread::sleep(Duration::from_millis(10));
        }
        client.join().unwrap();
    }
    assert_eq!(dmxctl::core::engine::lock(&app.host.core).project.fixtures[0].values[0],83);
    assert_eq!(dmxctl::core::engine::lock(&app.host.core).project.groups[0].name,"GUI shared edit");
    // No GUI frames or control polling: the engine must enforce the lease itself.
    std::thread::sleep(Duration::from_millis(200));
    assert!(dmxctl::core::engine::lock(&app.host.core).rt.flash.is_none());
    app.shutdown();
    drop(app);
    std::fs::remove_dir_all(dir).unwrap();
}
