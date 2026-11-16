use dmxctl::{control, fixture::Library, project::{Project, OutputCfg, OutputKind}};
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
        let mut child=Command::new(env!("CARGO_BIN_EXE_dmxctl-cli"))
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
        Command::new(env!("CARGO_BIN_EXE_dmxctl-cli")).args(["ctl","--connect",&self.address.to_string()])
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
    host.request(json!({"op":"command","command":{"cmd":"flash","on":true,"hold_ms":150}}));
    std::thread::sleep(Duration::from_millis(50));
    let bright=host.request(json!({"op":"output","fixture":1,"attrs":["dimmer"]}));
    assert_eq!(bright["channels"][0]["value"],255);
    std::thread::sleep(Duration::from_millis(250));
    let normal=host.request(json!({"op":"output","fixture":1,"attrs":["dimmer"]}));
    assert_ne!(normal["channels"][0]["value"],255);
    host.stop();
}

#[test]
fn non_local_addresses_are_rejected() {
    assert!(control::Launch::parse(&["--listen".into(),"0.0.0.0:9190".into()],false).is_err());
    assert!(control::client(&["--connect".into(),"192.0.2.1:9190".into(),"status".into()]).is_err());
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
    let mut app=dmxctl::app::App::with_options(&ctx,false,control::Launch {
        project:Some(dir.join("project.json")),listen:Some(address),output:false,inputs:false,autosave:false,
    }).unwrap();
    for command in [json!({"cmd":"set","fixture":1,"attr":"pan","value":83}),
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
    assert_eq!(dmxctl::engine::lock(&app.core).project.fixtures[0].values[0],83);
    // No GUI frames or control polling: the engine must enforce the lease itself.
    std::thread::sleep(Duration::from_millis(200));
    assert!(dmxctl::engine::lock(&app.core).rt.flash.is_none());
    app.shutdown();
    drop(app);
    std::fs::remove_dir_all(dir).unwrap();
}
