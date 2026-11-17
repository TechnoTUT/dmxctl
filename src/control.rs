//! Local control channel. Requests are executed by the owning App, so GUI and
//! headless mode share state, service management, saving and shutdown.
use crate::{app::App, commands, engine, recording::Recording};
use anyhow::{Context, ensure, bail};
use serde_json::{Value, json};
use std::{io::{BufRead, BufReader, Read, Write}, net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf, sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}, mpsc},
    thread::JoinHandle, time::{Duration, Instant}};

pub const DEFAULT_ADDRESS: &str = "127.0.0.1:9190";
const MAX_REQUEST: u64 = 1024 * 1024;
const MAX_RESPONSE: u64 = 4 * 1024 * 1024;

#[derive(Clone)]
pub struct Launch {
    pub project: Option<PathBuf>,
    pub listen: Option<SocketAddr>,
    pub output: bool,
    pub inputs: bool,
    pub autosave: bool,
}

impl Default for Launch {
    fn default() -> Self { Self { project: None, listen: None, output: true, inputs: true, autosave: true } }
}

impl Launch {
    pub fn parse(args: &[String], serve: bool) -> anyhow::Result<Self> {
        let mut result = Self::default();
        // serve は本番のGUIと同じPCで試すことがあるので、既定では実機出力・自動保存・入力(MIDI/OSC/Link/操作パネル)をすべて切る。
        // 入力が要るときは --inputs configured
        if serve { result.output = false; result.autosave = false; result.inputs = false; result.listen = Some(DEFAULT_ADDRESS.parse()?); }
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            match arg.as_str() {
                "--control" => { if result.listen.is_none() { result.listen = Some(DEFAULT_ADDRESS.parse()?); } }
                "--project"|"--listen"|"--output"|"--inputs" => {
                    i += 1;
                    let value = args.get(i).context("startup option needs a value")?;
                    match arg.as_str() {
                        "--project" => result.project = Some(PathBuf::from(value)),
                        "--listen" => result.listen = Some(local_address(value)?),
                        "--output" => { ensure!(["none","live"].contains(&value.as_str()), "output must be none or live"); result.output = value == "live"; }
                        "--inputs" => { ensure!(["none","configured"].contains(&value.as_str()), "inputs must be none or configured"); result.inputs = value == "configured"; }
                        _ => unreachable!(),
                    }
                }
                "--autosave" => result.autosave = true,
                "--no-autosave" => result.autosave = false,
                "--headless"|"--kmsdrm"|"--apply-layout" if !serve => {},
                // GUIは今まで知らない引数を無視していた(ショートカットやランチャーが付ける引数で起動できなくならないように)
                _ if !serve => eprintln!("unknown startup option ignored: {arg}"),
                _ => bail!("unknown startup option: {arg}"),
            }
            i += 1;
        }
        if serve { ensure!(result.project.is_some(), "serve requires --project"); }
        Ok(result)
    }
}

fn local_address(value: &str) -> anyhow::Result<SocketAddr> {
    let address: SocketAddr = value.parse().context("expected a loopback IP:port")?;
    ensure!(address.ip().is_loopback(), "control connections must use a loopback address");
    Ok(address)
}

struct Request { value: Value, reply: mpsc::Sender<Value>, deadline:Instant }

pub(crate) struct Host {
    pub address: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    requests: mpsc::Receiver<Request>,
    recording: Option<Recording>,
}

impl Host {
    pub fn start(address: SocketAddr, ctx: eframe::egui::Context) -> anyhow::Result<Self> {
        ensure!(address.ip().is_loopback(), "control server must bind to loopback");
        let listener = TcpListener::bind(address).context("cannot start control server")?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let (tx, requests) = mpsc::sync_channel::<Request>(32);
        let active = Arc::new(AtomicUsize::new(0));
        let thread = std::thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if active.fetch_add(1, Ordering::Relaxed) >= 8 {
                            active.fetch_sub(1, Ordering::Relaxed);
                            continue;
                        }
                        let (tx, ctx, active) = (tx.clone(), ctx.clone(), active.clone());
                        std::thread::spawn(move || {
                            let result = (|| -> anyhow::Result<Value> {
                                // macOS/BSD では accept した接続がリスナーのノンブロッキングを引き継ぐ。
                                // そのままだと要求が届く前に読み込みが WouldBlock で失敗し、接続がリセットされる
                                stream.set_nonblocking(false)?;
                                stream.set_read_timeout(Some(Duration::from_secs(2)))?;
                                stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                                let mut line = Vec::new();
                                BufReader::new((&stream).take(MAX_REQUEST + 1)).read_until(b'\n', &mut line)?;
                                ensure!(line.len() as u64 <= MAX_REQUEST && line.last() == Some(&b'\n'), "request is too large or incomplete");
                                let value: Value = serde_json::from_slice(&line)?;
                                let (reply, rx) = mpsc::channel();
                                tx.try_send(Request { value, reply, deadline:Instant::now()+Duration::from_secs(4) }).context("control queue is full or closed")?;
                                ctx.request_repaint();
                                Ok(rx.recv_timeout(Duration::from_secs(5)).context("application did not respond")?)
                            })();
                            let response = result.unwrap_or_else(|e| json!({"ok":false,"error":e.to_string()}));
                            let _ = writeln!(stream, "{response}");
                            active.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(10)),
                    Err(_) => break,
                }
            }
        });
        Ok(Self { address, stop, thread:Some(thread), requests, recording:None })
    }

    pub fn poll(&mut self, app: &mut App) -> bool {
        let mut shutdown = false;
        for _ in 0..16 {
            let Ok(request) = self.requests.try_recv() else { break; };
            let op = request.value["op"].as_str().unwrap_or("");
            let before=if ["profile_save","profile_delete","reload"].contains(&op) {
                engine::lock(&app.core).history_state()
            }else{None};
            let response = if Instant::now()>=request.deadline {Err(anyhow::anyhow!("queued request expired"))}
                else if shutdown { Err(anyhow::anyhow!("application is stopping")) }
                else { self.execute(app, &request.value) };
            engine::lock(&app.core).record_observation(before,op);
            let mut response=response;
            if let Ok(result)=&mut response {
                if ["reload","profile_save","profile_delete"].contains(&op) && request.value["dry_run"]!=true {
                    result["takes_effect_from_frame"]=json!(engine::lock(&app.core).rt.frame_id+1);
                }
            }
            let ok = response.is_ok();
            let response = match response {
                Ok(result) => json!({"ok":true,"result":result}),
                Err(error) => json!({"ok":false,"error":error.to_string()}),
            };
            let _ = request.reply.send(response);
            if op == "stop" && ok { shutdown = true; }
        }
        shutdown
    }

    fn execute(&mut self, app: &mut App, request: &Value) -> anyhow::Result<Value> {
        match request["op"].as_str().context("op is required")? {
            "status" => {
                let mut core = engine::lock(&app.core);
                let mut state = crate::web::state(&core);
                state["frame"] = json!(core.rt.frame_id);
                state["output_enabled"] = json!(app.launch.output);
                state["inputs_enabled"] = json!(app.launch.inputs);
                state["dirty"] = json!(core.dirty);
                state["project_path"] = json!(app.project_path);
                state["recording"] = json!(self.recording.is_some());
                state["version"]=core.version();
                Ok(state)
            }
            "version"=>Ok(engine::lock(&app.core).version()),
            "clock"=>{let core=engine::lock(&app.core);Ok(json!({"frame":core.rt.frame_id,"beat":core.rt.beat,"session":core.rt.session,
                "emergency":engine::EMERGENCY.load(Ordering::SeqCst)}))},
            "schema"=>crate::diagnostics::schema(&mut engine::lock(&app.core),request["entity"].as_str().unwrap_or("project")),
            "trace_start"=>engine::lock(&app.core).start_trace(u32::try_from(request["fixture"].as_u64().context("fixture is required")?)?,request["attr"].as_str().context("attr is required")?),
            "trace_result"=>engine::lock(&app.core).trace_result(request["ticket"].as_u64().context("ticket is required")?),
            "check"=>crate::diagnostics::check(&engine::lock(&app.core),request),
            "inspect" => {
                let core = engine::lock(&app.core);
                let mut project = serde_json::to_value(&core.project)?;
                project["web_pin"] = json!("");
                Ok(json!({"project":project,"frame":core.rt.frame_id,
                    "profiles":core.lib.profiles.iter().map(|p|p.as_ref()).collect::<Vec<_>>()}))
            }
            "fixtures" => {
                let core = engine::lock(&app.core);
                Ok(json!(core.project.fixtures.iter().map(|f|json!({
                    "id":f.id,"name":f.name,"profile":f.profile,"mode":f.mode,
                    "universe":f.universe+1,"address":f.address,"enabled":f.enabled,"armed":f.armed,"limits":f.limits
                })).collect::<Vec<_>>()))
            }
            "output" => snapshot(&engine::lock(&app.core), request),
            "controls" => Ok(engine::lock(&app.core).controls()),
            "entity_list" => crate::editing::list(&engine::lock(&app.core),request["entity"].as_str().context("entity is required")?),
            "profile_list" => Ok(json!(engine::lock(&app.core).lib.profiles.iter().map(|p|p.as_ref()).collect::<Vec<_>>())),
            "profile_save" => {
                let profile:crate::fixture::Profile=serde_json::from_value(request["data"].clone())?;
                engine::lock(&app.core).save_profile(&profile,true,commands::flag(request,"dry_run",false)?)
            }
            "profile_delete" => engine::lock(&app.core).delete_profile(request["id"].as_str().context("profile ID is required")?,true,commands::flag(request,"dry_run",false)?),
            "edit" => {
                let dry_run=commands::flag(request,"dry_run",false)?;
                let mut core=engine::lock(&app.core);
                ensure!(!engine::EMERGENCY.load(Ordering::SeqCst),"application is in emergency stop");
                core.check_version(request)?;
                let old_midi=core.project.midi_input.clone();
                let result=crate::editing::apply(&mut core,&request["operations"],dry_run);
                let command=json!({"cmd":"edit","operations":request["operations"],"dry_run":dry_run,"id":request.get("id")});
                log_command(&core,&command,result.as_ref().err().map(|e|e.to_string()));
                let mut result=result?;
                let changed_midi=old_midi!=core.project.midi_input;
                result["version"]=core.version();
                result["takes_effect_from_frame"]=if dry_run{Value::Null}else{json!(core.rt.frame_id+1)};
                drop(core);
                if !dry_run {
                    if changed_midi { result["midi_error"]=json!(app.sync_midi_choice().err().map(|e|e.to_string())); }
                    app.apply_services();
                }
                Ok(result)
            }
            "project_diff" => app.project_diff(),
            "reload" => {
                let file=request.get("file").map(|v|v.as_str().context("file must be a string")).transpose()?;
                app.reload_checked(file.map(std::path::Path::new),true,commands::flag(request,"dry_run",false)?)
            }
            "command" => {
                let command = &request["command"];
                if command["cmd"]=="edit" {
                    let mut delegated=json!({"op":"edit","operations":command["operations"],
                        "dry_run":command.get("dry_run").cloned().unwrap_or(json!(false)),"id":command.get("id")});
                    for key in ["expected_revision","expected_session"] {
                        if let Some(value)=request.get(key).or_else(||command.get(key)){delegated[key]=value.clone();}
                    }
                    return self.execute(app,&delegated);
                }
                let hold = if command["cmd"] == "flash" && command["on"] != false {
                    let ms = command.get("hold_ms").map(|v|v.as_u64().context("hold_ms must be an integer")).transpose()?.unwrap_or(500);
                    ensure!((1..=5000).contains(&ms), "hold_ms must be in 1..5000");
                    Some(Duration::from_millis(ms))
                } else { None };
                let mut core = engine::lock(&app.core);
                ensure!(!engine::EMERGENCY.load(Ordering::SeqCst), "application is in emergency stop");
                let result = commands::apply(&mut core, command);
                log_command(&core, command, result.as_ref().err().map(|e|e.to_string()));
                result?;
                // フラッシュの期限は commands::apply が core.now()(仮想時間でも同じ時計)で決める
                let frame = core.rt.frame_id;
                drop(core);
                if command["cmd"] == "flash" {
                    if let Some(duration) = hold { app.ctx.request_repaint_after(duration); }
                }
                app.apply_services();
                Ok(json!({"accepted_at_frame":frame,"takes_effect_from_frame":frame+1}))
            }
            "save" => {
                {
                    let core = engine::lock(&app.core);
                    crate::editing::validate(&core.project, &core.lib)?;
                }
                app.save_checked()?;
                Ok(json!({"saved":app.project_path}))
            }
            "record_start" => {
                ensure!(self.recording.is_none(), "recording is already active");
                let path = request["file"].as_str().context("file is required")?;
                let recording = Recording::start(Some(std::path::Path::new(path)), false, false)?;
                let mut core = engine::lock(&app.core);
                let mut project = serde_json::to_value(&core.project)?;
                project["web_pin"] = json!("");
                recording.recorder.emit(json!({"type":"start","version":1,"hz":40,"clock":"real",
                    "output":app.launch.output,"project":project,"frame":core.rt.frame_id,
                    "t_ms":core.now().saturating_duration_since(core.rt.t0).as_millis() as u64,
                    "configuration_version":core.version(),
                    "profiles":core.lib.profiles.iter().map(|p|p.as_ref()).collect::<Vec<_>>()}));
                core.rt.recorder = Some(recording.recorder.clone());
                self.recording = Some(recording);
                Ok(json!({"recording":true,"file":path}))
            }
            "record_stop" => { self.finish_recording(app)?; Ok(json!({"recording":false})) }
            "stop" => Ok(json!({"stopping":true})),
            op => bail!("unsupported control operation: {op}"),
        }
    }

    pub fn finish_recording(&mut self, app: &App) -> anyhow::Result<()> {
        let recording = self.recording.take().context("no active recording")?;
        {
            let mut core = engine::lock(&app.core);
            core.rt.recorder = None;
            recording.recorder.emit(json!({"type":"complete","frame":core.rt.frame_id,
                "t_ms":core.now().saturating_duration_since(core.rt.t0).as_millis() as u64,"outputs":core.rt.output_status}));
        }
        recording.finish()
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

fn log_command(core: &engine::Core, command: &Value, error: Option<String>) {
    if let Some(recorder) = &core.rt.recorder {
        recorder.emit(json!({"type":if error.is_none(){"command"}else{"command_error"},"command":crate::editing::redact(command),
            "id":command.get("id"),"error":error,"applies_from_frame":recorder.next_frame(),
            "applies_from_engine_frame":core.rt.frame_id+1,
            "t_ms":core.now().saturating_duration_since(core.rt.t0).as_millis() as u64}));
    }
}

fn snapshot(core: &engine::Core, request: &Value) -> anyhow::Result<Value> {
    let fixture = request.get("fixture").map(|v|v.as_u64().context("fixture must be an integer")).transpose()?;
    if let Some(id) = fixture { ensure!(core.project.fixtures.iter().any(|f|f.id as u64==id), "unknown fixture"); }
    let attrs: Vec<&str> = request.get("attrs").map(|v| -> anyhow::Result<Vec<&str>> {
        v.as_array().context("attrs must be an array")?.iter().map(|v|v.as_str().context("attr must be a string")).collect()
    }).transpose()?.unwrap_or_default();
    let mut channels = Vec::new();
    for f in &core.project.fixtures {
        if fixture.is_some_and(|id|f.id as u64!=id) { continue; }
        let Some(mode) = core.lib.mode(&f.profile,f.mode) else {continue;};
        for (i,ch) in mode.channels.iter().enumerate() {
            if !attrs.is_empty() && !attrs.contains(&ch.attr.as_str()) {continue;}
            let address = f.address as usize + i;
            if address==0 {continue;}
            let Some(value) = core.rt.frames.get(f.universe).and_then(|frame|frame.get(address-1)) else {continue;};
            channels.push(json!({"fixture":f.id,"attr":ch.attr,"universe":f.universe+1,"address":address,"value":value}));
        }
    }
    Ok(json!({"frame":core.rt.frame_id,"t_ms":core.now().saturating_duration_since(core.rt.t0).as_millis() as u64,"channels":channels}))
}

pub fn request(address: SocketAddr, value: &Value) -> anyhow::Result<Value> {
    request_with_timeout(address,value,Duration::from_secs(6))
}

pub fn request_with_timeout(address:SocketAddr,value:&Value,timeout:Duration)->anyhow::Result<Value> {
    ensure!(address.ip().is_loopback(), "control client must connect to loopback");
    ensure!(!timeout.is_zero(),"request timed out");
    let started=Instant::now();
    let mut stream = TcpStream::connect_timeout(&address,timeout.min(Duration::from_secs(2))).context("cannot connect to running application")?;
    let remaining=timeout.saturating_sub(started.elapsed());ensure!(!remaining.is_zero(),"request timed out");
    stream.set_write_timeout(Some(remaining.min(Duration::from_secs(2))))?;
    let line = serde_json::to_string(value)?;
    ensure!(line.len() as u64 + 1 <= MAX_REQUEST, "request is too large");
    writeln!(stream,"{line}")?;
    let remaining=timeout.saturating_sub(started.elapsed());ensure!(!remaining.is_zero(),"request timed out");
    stream.set_read_timeout(Some(remaining))?;
    let mut response = Vec::new();
    BufReader::new(stream.take(MAX_RESPONSE+1)).read_until(b'\n',&mut response)?;
    ensure!(response.len() as u64<=MAX_RESPONSE && response.last()==Some(&b'\n'), "response is too large or incomplete");
    let response:Value = serde_json::from_slice(&response)?;
    ensure!(response["ok"]==true,"{}",response["error"].as_str().unwrap_or("control request failed"));
    Ok(response["result"].clone())
}

/// CLI frontend to the local protocol. The protocol contains data operations,
/// not shell commands, and can later be adapted for MCP without another engine.
pub fn client(args: &[String]) -> anyhow::Result<()> {
    let mut positional = Vec::new();
    let mut options = std::collections::BTreeMap::new();
    let mut dry_run=false;
    let mut wait_after=false;
    let mut i = 0;
    while i < args.len() {
        if args[i]=="--wait" {ensure!(!wait_after,"duplicate --wait");wait_after=true;i+=1;continue;}
        if args[i]=="--dry-run" {ensure!(!dry_run,"duplicate --dry-run");dry_run=true;i+=1;continue;}
        if args[i].starts_with("--") {
            let key = args[i].as_str();
            ensure!(["--connect","--json","--file","--fixture","--attrs","--attr","--value","--duration","--interval-ms","--hold-ms","--channel","--number","--kind","--index",
                "--frames","--beats","--ms","--equals","--min","--max","--tolerance","--after-frame","--timeout-ms","--expected-revision","--expected-session"].contains(&key), "unknown control option: {key}");
            i += 1;
            let value = args.get(i).context("control option needs a value")?;
            ensure!(options.insert(key,value.as_str()).is_none(), "duplicate control option");
        } else { positional.push(args[i].as_str()); }
        i += 1;
    }
    let address = local_address(options.get("--connect").copied().unwrap_or(DEFAULT_ADDRESS))?;
    if positional.first().is_some_and(|p|["schema","version","explain","wait","assert"].contains(p)) {
        ensure!(!dry_run && !wait_after,"--dry-run/--wait is not supported by this command");
        return crate::diagnostics::client(address,&positional,&options);
    }
    let mut used = vec!["--connect"];
    let mut payload = match positional.as_slice() {
        ["status"] => json!({"op":"status"}),
        ["inspect"] => json!({"op":"inspect"}),
        ["stop"] => json!({"op":"stop"}),
        ["fixture","list"] => json!({"op":"fixtures"}),
        ["button","list"] => json!({"op":"controls"}),
        ["button",event,id] => {
            ensure!(["click","press","release","hold","set"].contains(event),"unknown button event");
            used.extend(["--hold-ms","--value"]);
            let mut command=json!({"cmd":"button","control":id,"event":event});
            if let Some(value)=options.get("--hold-ms") {command["hold_ms"]=json!(value.parse::<u64>()?);}
            if let Some(value)=options.get("--value") {command["value"]=json!(value.parse::<f64>()?);}
            json!({"op":"command","command":command})
        }
        ["input","midi"] => {
            used.extend(["--kind","--channel","--number","--value","--hold-ms"]);
            let mut command=json!({"cmd":"input","source":"midi","kind":options.get("--kind").context("--kind is required")?});
            for (option,key) in [("--channel","channel"),("--number","number"),("--value","value")] {
                command[key]=json!(options.get(option).with_context(||format!("{option} is required"))?.parse::<u64>()?);
            }
            if let Some(value)=options.get("--hold-ms") {command["hold_ms"]=json!(value.parse::<u64>()?);}
            json!({"op":"command","command":command})
        }
        ["input",source] if *source=="key" || *source=="osc" => {
            used.push("--json");
            let mut command:Value=serde_json::from_str(options.get("--json").context("--json is required")?)?;
            ensure!(command.is_object(),"input JSON must be an object");
            command["cmd"]=json!("input"); command["source"]=json!(source);
            json!({"op":"command","command":command})
        }
        ["fixture","set",id] => {
            used.extend(["--attr","--value"]);
            let fixture: u32 = id.parse()?;
            let attr = options.get("--attr").context("--attr is required")?;
            let number: f64 = options.get("--value").context("--value is required")?.parse()?;
            ensure!(number.is_finite(), "value must be finite");
            json!({"op":"command","command":{"cmd":"set","fixture":fixture,"attr":attr,"value":number}})
        }
        ["command"] => {
            used.push("--json");
            let command:Value = serde_json::from_str(options.get("--json").context("--json is required")?)?;
            json!({"op":"command","command":command})
        }
        ["project","save"] => json!({"op":"save"}),
        ["profile","list"] => json!({"op":"profile_list"}),
        ["profile","save"] => {used.extend(["--file","--json"]);json!({"op":"profile_save","data":client_data(&options)?})}
        ["profile","delete",id] => json!({"op":"profile_delete","id":id}),
        ["project","diff"] => json!({"op":"project_diff"}),
        ["project",op] if *op=="reload" || *op=="restore" => {
            used.push("--file");
            let mut payload=json!({"op":"reload"});
            if let Some(path)=options.get("--file") {payload["file"]=json!(absolute_path(path)?);}
            ensure!(*op!="restore" || options.contains_key("--file"),"restore requires --file");
            payload
        }
        ["edit"] => {
            used.extend(["--file","--json"]);
            let data=client_data(&options)?;
            if let Some(object)=data.as_object() {ensure!(object.keys().all(|key|key=="operations"),"edit file supports an operations array only; use --dry-run on the command line");}
            json!({"op":"edit","operations":if data.is_array(){data}else{data["operations"].clone()}})
        }
        [entity,"list"]|[entity,"get"] if editable(entity) => json!({"op":"entity_list","entity":entity}),
        [entity,op]|[entity,op,_] if editable(entity) && ["create","update","delete","capture"].contains(op) => {
            used.extend(["--file","--json","--index"]);
            let mut operation=json!({"entity":entity,"op":op});
            if *op!="delete" {
                let data=if options.contains_key("--json") || options.contains_key("--file") {client_data(&options)?}else{json!({})};
                operation["data"]=data;
            }
            if let Some(key)=positional.get(2) {
                if *op=="capture" {operation["data"]["name"]=json!(key);}
                else if *entity=="fixture" {operation["key"]=json!(key.parse::<u32>()?);}
                else {operation["key"]=json!(key);}
            }
            if let Some(index)=options.get("--index") {operation["index"]=json!(index.parse::<u64>()?);}
            json!({"op":"edit","operations":[operation]})
        }
        ["output","snapshot"]|["watch"] => {
            used.extend(["--fixture","--attrs"]);
            let mut payload = json!({"op":"output"});
            if let Some(id) = options.get("--fixture") { payload["fixture"] = json!(id.parse::<u32>()?); }
            if let Some(attrs) = options.get("--attrs") { payload["attrs"] = json!(attrs.split(',').collect::<Vec<_>>()); }
            payload
        }
        ["record","start"] => {
            used.push("--file");
            // Resolve client-side paths before sending them to the host.
            let path = PathBuf::from(options.get("--file").context("--file is required")?);
            let path = if path.is_absolute() {path} else {std::env::current_dir()?.join(path)};
            json!({"op":"record_start","file":path})
        }
        ["record","stop"] => json!({"op":"record_stop"}),
        _ => bail!("unknown control command; see dmxctl --help"),
    };
    let watching = positional.as_slice()==["watch"];
    if payload["op"]=="edit" || (payload["op"]=="command" && payload["command"]["cmd"]=="edit") {
        used.extend(["--expected-revision","--expected-session"]);
        if let Some(revision)=options.get("--expected-revision") {payload["expected_revision"]=json!(revision.parse::<u64>()?);}
        if let Some(session)=options.get("--expected-session") {payload["expected_session"]=json!(session);}
    }
    if wait_after {
        ensure!(!dry_run && ["command","edit","reload","profile_save","profile_delete"].contains(&payload["op"].as_str().unwrap_or("")),"--wait requires an operation that changes live state");
        used.push("--timeout-ms");
    }
    if dry_run {
        ensure!(["edit","reload","profile_save","profile_delete"].contains(&payload["op"].as_str().unwrap_or("")),"--dry-run requires edit, reload/restore or profile save/delete");
        payload["dry_run"]=json!(true);
    }
    if watching { used.extend(["--duration","--interval-ms"]); }
    for key in options.keys() { ensure!(used.contains(key),"option is not supported by this command: {key}"); }
    let wait_timeout=if wait_after {
        let timeout=options.get("--timeout-ms").map(|s|s.parse::<u64>()).transpose()?.unwrap_or(5000);
        ensure!((1..=300000).contains(&timeout),"timeout-ms must be in 1..300000");timeout
    }else{30000};
    if !watching {
        let before=if wait_after{Some(request(address,&json!({"op":"clock"}))?)}else{None};
        let mut result=request(address,&payload)?;
        if let Some(before)=before {
            let target=result["takes_effect_from_frame"].as_u64().unwrap_or(before["frame"].as_u64().unwrap_or(0)+1);
            let observed=crate::diagnostics::wait_for(address,&json!({"frame":target,"session":before["session"]}),wait_timeout)?;
            result["observed"]=observed;
        }
        println!("{result}");
        return Ok(());
    }
    let duration = options.get("--duration").map(|value|value.parse::<f64>()).transpose()?;
    if let Some(seconds)=duration { ensure!(seconds.is_finite() && seconds>0.0 && seconds<=86400.0,"invalid watch duration"); }
    let interval = options.get("--interval-ms").map(|value|value.parse::<u64>()).transpose()?.unwrap_or(100);
    ensure!((25..=10000).contains(&interval),"interval-ms must be in 25..10000");
    let started = Instant::now();
    let mut previous = Value::Null;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    while duration.is_none_or(|seconds| started.elapsed().as_secs_f64()<seconds) {
        let snapshot = request(address,&payload)?;
        let mut event = snapshot.clone();
        event["type"] = json!("snapshot");
        event["sample_interval_ms"] = json!(interval);
        event["changed"] = json!(snapshot["channels"]!=previous);
        previous = snapshot["channels"].clone();
        if writeln!(out,"{event}").and_then(|_|out.flush()).is_err() { break; }
        let mut delay = Duration::from_millis(interval);
        if let Some(seconds)=duration { delay=delay.min(Duration::from_secs_f64(seconds).saturating_sub(started.elapsed())); }
        std::thread::sleep(delay);
    }
    Ok(())
}

fn editable(entity:&str)->bool {
    ["fixture","group","binding","scene","position","effect","pixel","chase","pair","output","auto_preset","settings"].contains(&entity)
}

fn absolute_path(path:&str)->anyhow::Result<PathBuf> {
    let path=PathBuf::from(path);
    Ok(if path.is_absolute(){path}else{std::env::current_dir()?.join(path)})
}

fn client_data(options:&std::collections::BTreeMap<&str,&str>)->anyhow::Result<Value> {
    ensure!(options.contains_key("--json")!=options.contains_key("--file"),"provide exactly one of --json or --file");
    let text=if let Some(data)=options.get("--json") {data.to_string()}
        else {std::fs::read_to_string(absolute_path(options["--file"])?)?};
    Ok(serde_json::from_str(text.trim_start_matches('\u{feff}'))?)
}
