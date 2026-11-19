//! On-demand inspection of actual render frames. Idle operation allocates no trace.
use crate::core::{engine::Core, fixture::{Library, Mode}, project::*};
use anyhow::{Context, ensure, bail};
use serde_json::{Value,json};
use std::{hash::{Hash,Hasher}, time::{Duration,Instant}};

pub struct Trace {
    pub ticket:u64,
    pub fixture:u32,
    attr:String,
    started:Instant,
    frame:Option<u64>,
    stages:Vec<Value>,
    pub result:Option<Value>,
}

impl Trace {
    pub fn capture_single(&mut self,id:u32,mode:&Mode,values:&[u8],stage:&str,details:Value) {
        if self.fixture!=id || self.result.is_some() {return;}
        let channels:Vec<_>=mode.attr_channels(&self.attr).into_iter().filter(|&ch|ch<values.len())
            .map(|ch|json!({"channel":ch+1,"value":mode.get_f(values,ch),"coarse":values[ch],
                "fine":mode.fine_of(ch).and_then(|fine|values.get(fine).copied())})).collect();
        let changed=self.stages.last().is_none_or(|previous|previous["channels"]!=json!(channels));
        self.stages.push(json!({"stage":stage,"changed":changed,"channels":channels,"details":details}));
    }
    fn capture(&mut self,project:&Project,lib:&Library,values:Option<&[Vec<u8>]>,stage:&str,details:Value) {
        let Some((i,f))=project.fixtures.iter().enumerate().find(|(_,f)|f.id==self.fixture) else{return;};
        let Some(mode)=lib.mode(&f.profile,f.mode) else{return;};
        let values=values.and_then(|v|v.get(i)).unwrap_or(&f.values);
        self.capture_single(f.id,mode,values,stage,details);
    }
}

impl Core {
    pub fn version(&mut self)->Value {
        let encoded=serde_json::to_vec(&self.project).expect("serializable project");
        let mut hash=std::collections::hash_map::DefaultHasher::new();encoded.hash(&mut hash);
        let hash=hash.finish();
        if self.rt.revision_hash.is_some_and(|previous|previous!=hash) {self.rt.revision+=1;}
        self.rt.revision_hash=Some(hash);
        json!({"session":self.rt.session,"revision":self.rt.revision})
    }
    pub fn check_version(&mut self,request:&Value)->anyhow::Result<()> {
        if let Some(session)=request.get("expected_session") {
            ensure!(session.as_str()==Some(self.rt.session.as_str()),"application session changed; inspect the current state again");
        }
        if let Some(revision)=request.get("expected_revision") {
            let revision=revision.as_u64().context("expected_revision must be an integer")?;
            let current=self.version();
            ensure!(current["revision"]==revision,"configuration changed; expected revision {revision}, current {}",self.rt.revision);
        }
        Ok(())
    }
    pub fn start_trace(&mut self,fixture:u32,attr:&str)->anyhow::Result<Value> {
        ensure!(!crate::core::engine::EMERGENCY.load(std::sync::atomic::Ordering::SeqCst),"explain is unavailable during emergency stop; use output snapshot");
        let f=self.project.fixtures.iter().find(|f|f.id==fixture).context("unknown fixture")?;
        let mode=self.lib.mode(&f.profile,f.mode).context("unknown profile/mode")?;
        ensure!(!mode.attr_channels(attr).is_empty(),"unknown attribute");
        let now=self.now();
        ensure!(self.rt.trace.as_ref().is_none_or(|t|now.saturating_duration_since(t.started)>Duration::from_secs(5)),"another trace is pending");
        self.rt.trace_ticket=self.rt.trace_ticket.checked_add(1).context("trace ticket exhausted")?;
        let ticket=self.rt.trace_ticket;
        self.rt.trace=Some(Trace {ticket,fixture,attr:attr.into(),started:now,frame:None,stages:Vec::new(),result:None});
        Ok(json!({"ticket":ticket,"from_frame":self.rt.frame_id+1}))
    }
    pub fn trace_result(&mut self,ticket:u64)->anyhow::Result<Value> {
        let trace=self.rt.trace.as_ref().context("trace is no longer available")?;
        ensure!(trace.ticket==ticket,"trace ticket does not match");
        if trace.result.is_none() {return Ok(json!({"ready":false}));}
        let mut trace=self.rt.trace.take().unwrap();
        Ok(json!({"ready":true,"trace":trace.result.take()}))
    }
    pub(crate) fn trace_begin(&mut self) {
        if let Some(trace)=self.rt.trace.as_mut() {
            if trace.result.is_none() {trace.stages.clear();trace.frame=Some(self.rt.frame_id+1);}
        }
        self.trace_stage("base",None,Value::Null);
    }
    pub(crate) fn trace_stage(&mut self,stage:&str,values:Option<&[Vec<u8>]>,details:Value) {
        if let Some(trace)=self.rt.trace.as_mut() {trace.capture(&self.project,&self.lib,values,stage,details);}
    }
    pub(crate) fn trace_finish(&mut self) {
        let t_ms=self.now().saturating_duration_since(self.rt.t0).as_millis() as u64;
        let Some(trace)=self.rt.trace.as_mut().filter(|t|t.result.is_none()) else{return;};
        let Some(f)=self.project.fixtures.iter().find(|f|f.id==trace.fixture) else {
            trace.result=Some(json!({"frame":self.rt.frame_id,"error":"fixture removed"}));return;
        };
        let Some(mode)=self.lib.mode(&f.profile,f.mode) else {
            trace.result=Some(json!({"frame":self.rt.frame_id,"error":"profile/mode removed"}));return;
        };
        let start=(f.address as usize).saturating_sub(1);
        let output=self.rt.frames.get(f.universe).and_then(|frame|frame.get(start..start+mode.channels.len()));
        if let Some(values)=output {trace.capture_single(f.id,mode,values,"output",json!({"enabled":f.enabled,"universe":f.universe+1,"address":f.address}));}
        trace.result=Some(json!({"fixture":trace.fixture,"attr":trace.attr,"frame":trace.frame,"t_ms":t_ms,
            "stages":trace.stages,"safety":self.rt.safety_events,"output_enabled":self.rt.physical_output_enabled}));
    }
}

pub fn check(core:&Core,request:&Value)->anyhow::Result<Value> {
    let fixture=u32::try_from(request["fixture"].as_u64().context("fixture is required")?)?;
    let attr=request["attr"].as_str().context("attr is required")?;
    let f=core.project.fixtures.iter().find(|f|f.id==fixture).context("unknown fixture")?;
    let mode=core.lib.mode(&f.profile,f.mode).context("unknown profile/mode")?;
    let start=(f.address as usize).checked_sub(1).context("invalid fixture address")?;
    let values=core.rt.frames.get(f.universe).and_then(|v|v.get(start..start+mode.channels.len())).context("invalid fixture patch")?;
    let channels=mode.attr_channels(attr);ensure!(!channels.is_empty(),"unknown attribute");
    let number=|key:&str|->anyhow::Result<Option<f64>> {
        request.get(key).map(|v| {let n=v.as_f64().with_context(||format!("{key} must be numeric"))?;ensure!(n.is_finite(),"{key} must be finite");Ok(n)}).transpose()
    };
    let equals=number("equals")?;let min=number("min")?;let max=number("max")?;let tolerance=number("tolerance")?.unwrap_or(0.0);
    ensure!(tolerance>=0.0,"tolerance must be nonnegative");
    ensure!(equals.is_some() || min.is_some() || max.is_some(),"equals, min or max is required");
    if let (Some(min),Some(max))=(min,max) {ensure!(min<=max,"min exceeds max");}
    let actual:Vec<_>=channels.iter().map(|&ch|mode.get_f(values,ch) as f64).collect();
    let after=request.get("after_frame").map(|v|v.as_u64().context("after_frame must be an integer")).transpose()?;
    let passed=core.rt.frame_id>0 && after.is_none_or(|frame|core.rt.frame_id>=frame) && actual.iter().all(|&n|
        equals.is_none_or(|e|(n-e).abs()<=tolerance) && min.is_none_or(|min|n+tolerance>=min) && max.is_none_or(|max|n-tolerance<=max));
    Ok(json!({"passed":passed,"fixture":fixture,"attr":attr,"frame":core.rt.frame_id,"beat":core.rt.beat,
        "actual":actual,"condition":request,"unit":"DMX (coarse + fine/256)"}))
}

fn schema_value<T:schemars::JsonSchema>()->Value {serde_json::to_value(schemars::schema_for!(T)).unwrap()}

pub fn schema(core:&mut Core,entity:&str)->anyhow::Result<Value> {
    let version=core.version();
    let project=crate::backend::editing::redact(&serde_json::to_value(&core.project)?);
    let (schema,current,read_only)=match entity {
        "project"=>(schema_value::<Project>(),project,vec!["next_id","next_binding_id","master_seeded"]),
        "fixture"=>(schema_value::<FixtureInstance>(),project["fixtures"].clone(),vec!["id","armed","limits"]),
        "binding"=>(schema_value::<Binding>(),project["bindings"].clone(),vec!["id"]),
        "group"=>(schema_value::<Group>(),project["groups"].clone(),vec![]),
        "scene"|"position"=>(schema_value::<Scene>(),project[if entity=="scene"{"scenes"}else{"positions"}].clone(),vec![]),
        "effect"=>(schema_value::<Effect>(),project["effects"].clone(),vec![]),
        "pixel"=>(schema_value::<crate::core::pixel::PixelFx>(),project["pixel_fx"].clone(),vec![]),
        "chase"=>(schema_value::<Chase>(),project["chases"].clone(),vec![]),
        "pair"=>(schema_value::<Pair>(),project["pairs"].clone(),vec![]),
        "output"=>(schema_value::<OutputCfg>(),project["outputs"].clone(),vec![]),
        "auto"=>(schema_value::<AutoCfg>(),project["auto"].clone(),vec![]),
        "auto_preset"=>(schema_value::<AutoPreset>(),project["auto"]["presets"].clone(),vec![]),
        "motion"=>(schema_value::<crate::core::motion::Motion>(),project["motion"].clone(),vec!["pan_front","tilt_up"]),
        "profile"=>(schema_value::<crate::core::fixture::Profile>(),json!(core.lib.profiles.iter().map(|p|p.as_ref()).collect::<Vec<_>>()),vec!["existing safety definitions"]),
        "action"=>(schema_value::<Action>(),Value::Null,vec!["Arm is not remotely executable"]),
        _=>bail!("unknown schema entity"),
    };
    Ok(json!({"entity":entity,"schema":schema,"current":current,"read_only":read_only,"version":version,
        "rules":{"bpm":[BPM_MIN,BPM_MAX],"slider":[0,1],"dmx":[0,255.99609375],"hold_ms":[1,5000],"scene_fade_seconds":[0,120],
            "indices":"zero-based; DMX addresses one-based","validation":"edit --dry-run also checks references, patch overlap and safety policy"},
        "commands":command_catalog()}))
}

fn command_catalog()->Value {
    json!({"set":{"required":["fixture","attr","value"],"value_unit":"DMX"},
        "button":{"required":["control","event"],"event":["click","press","release","hold","set"],"value_range":[0,1],"hold_ms":[1,5000]},
        "input":{"source":["midi","key","osc"],"midi":{"kind":["Note","Cc"],"channel":[0,15],"number":[0,127],"value":[0,127]}},
        "edit":{"required":["operations"],"operations":["create","update","delete","capture"],"dry_run":"boolean"},
        "motion":{"required":["params"],"fields":["shape","beats","spread","position","size","points","glide"]},
        "effect":{"required":["name","params"]},"blackout":{"on":"boolean"},"master":{"value_range":[0,1]},
        "bpm":{"value_range":[BPM_MIN,BPM_MAX]},"scene":{"required":["name"]},"color":{"required":["name"],"slot":["main","sub"]},
        "flash":{"on":"boolean","hold_ms":[1,5000]},"other":["fade","auto","auto_resume","preset","look","swap","tap","resync"]})
}

pub fn wait_for(address:std::net::SocketAddr,target:&Value,timeout_ms:u64)->anyhow::Result<Value> {
    ensure!((1..=300000).contains(&timeout_ms),"timeout-ms must be in 1..300000");
    let started=Instant::now();let timeout=Duration::from_millis(timeout_ms);
    let mut last=Value::Null;
    loop {
        let remaining=timeout.saturating_sub(started.elapsed());
        ensure!(!remaining.is_zero(),"wait timed out; last observation: {last}");
        let mut request=target.get("condition").cloned().unwrap_or_else(||json!({"op":"clock"}));
        if target.get("condition").is_some(){request["op"]=json!("check");}
        last=crate::backend::control::request_with_timeout(address,&request,remaining.min(Duration::from_secs(6)))?;
        let clock=if request["op"]=="check" {
            crate::backend::control::request_with_timeout(address,&json!({"op":"clock"}),timeout.saturating_sub(started.elapsed()).min(Duration::from_secs(6)))?
        }else{last.clone()};
        if let Some(session)=target.get("session") {ensure!(clock["session"]==*session,"application session changed while waiting");}
        ensure!(clock["emergency"]!=true,"application entered emergency stop while waiting");
        let observed=if request["op"]=="check"{&last}else{&clock};
        let frame_ok=target["frame"].as_u64().is_none_or(|frame|observed["frame"].as_u64().is_some_and(|current|current>=frame));
        let beat_ok=target["beat"].as_f64().is_none_or(|beat|observed["beat"].as_f64().is_some_and(|current|current>=beat));
        let time_ok=target["ms"].as_u64().is_none_or(|ms|started.elapsed()>=Duration::from_millis(ms));
        if frame_ok && beat_ok && time_ok && (request["op"]!="check" || last["passed"]==true) {
            return Ok(json!({"matched":true,"elapsed_ms":started.elapsed().as_millis() as u64,"observation":last,"clock":clock}));
        }
        std::thread::sleep(Duration::from_millis(25).min(timeout.saturating_sub(started.elapsed())));
    }
}

pub fn client(address:std::net::SocketAddr,args:&[&str],options:&std::collections::BTreeMap<&str,&str>)->anyhow::Result<()> {
    let command=*args.first().context("diagnostic command is required")?;
    let mut allowed=vec!["--connect"];
    let mut payload=json!({});
    let timeout=options.get("--timeout-ms").map(|s|s.parse::<u64>()).transpose()?.unwrap_or(5000);
    match args {
        ["version"]=>payload=json!({"op":"version"}),
        ["schema"]=>payload=json!({"op":"schema","entity":"project"}),
        ["schema",entity]=>payload=json!({"op":"schema","entity":entity}),
        ["explain"]|["assert"]|["wait"]=>{
            allowed.extend(["--fixture","--attr"]);
            if command=="wait" {allowed.extend(["--frames","--beats","--ms"]);}
            if command!="explain" {allowed.extend(["--equals","--min","--max","--tolerance","--after-frame"]);}
            if command!="assert" {allowed.push("--timeout-ms");}
            if command=="explain" || command=="assert" || ["--fixture","--attr","--equals","--min","--max","--tolerance"].iter().any(|key|options.contains_key(key)) {
                payload["fixture"]=json!(options.get("--fixture").context("--fixture is required")?.parse::<u32>()?);
                payload["attr"]=json!(options.get("--attr").context("--attr is required")?);
            }
            for (option,key) in [("--equals","equals"),("--min","min"),("--max","max"),("--tolerance","tolerance")] {
                if let Some(value)=options.get(option) {
                    let value=value.parse::<f64>()?;ensure!(value.is_finite(),"{option} must be finite");payload[key]=json!(value);
                }
            }
            if let Some(value)=options.get("--after-frame"){payload["after_frame"]=json!(value.parse::<u64>()?);}
        }
        _=>bail!("unknown diagnostic command"),
    }
    for option in options.keys(){ensure!(allowed.contains(option),"option is not supported by this command: {option}");}
    match command {
        "schema"|"version"=>println!("{}",crate::backend::control::request(address,&payload)?),
        "assert"=>{
            payload["op"]=json!("check");let result=crate::backend::control::request(address,&payload)?;
            println!("{result}");ensure!(result["passed"]==true,"output assertion failed");
        }
        "explain"=>{
            ensure!((1..=300000).contains(&timeout),"invalid timeout-ms");payload["op"]=json!("trace_start");
            let started=Instant::now();let timeout=Duration::from_millis(timeout);
            let start=crate::backend::control::request_with_timeout(address,&payload,timeout.min(Duration::from_secs(6)))?;
            loop {
                let remaining=timeout.saturating_sub(started.elapsed());ensure!(!remaining.is_zero(),"trace timed out");
                let result=crate::backend::control::request_with_timeout(address,&json!({"op":"trace_result","ticket":start["ticket"]}),remaining.min(Duration::from_secs(6)))?;
                if result["ready"]==true {println!("{}",result["trace"]);break;}
                std::thread::sleep(Duration::from_millis(25).min(timeout.saturating_sub(started.elapsed())));
            }
        }
        "wait"=>{
            let clock=crate::backend::control::request(address,&json!({"op":"clock"}))?;
            let mut target=json!({"session":clock["session"]});
            let mut criteria=false;
            if let Some(frames)=options.get("--frames") {
                let frames=frames.parse::<u64>()?;ensure!(frames>0,"frames must be positive");
                target["frame"]=json!(clock["frame"].as_u64().unwrap().checked_add(frames).context("frame target overflow")?);criteria=true;
            }
            if let Some(beats)=options.get("--beats") {
                let beats=beats.parse::<f64>()?;ensure!(beats.is_finite() && beats>0.0,"beats must be positive and finite");
                target["beat"]=json!(clock["beat"].as_f64().unwrap()+beats);criteria=true;
            }
            if let Some(ms)=options.get("--ms") {target["ms"]=json!(ms.parse::<u64>()?);criteria=true;}
            if payload.get("fixture").is_some() {
                target["condition"]=payload;criteria=true;
            }else if let Some(frame)=payload.get("after_frame") {target["frame"]=json!(target["frame"].as_u64().unwrap_or(0).max(frame.as_u64().unwrap()));criteria=true;}
            ensure!(criteria,"specify frames, beats, ms or an output condition");
            println!("{}",wait_for(address,&target,timeout)?);
        }
        _=>unreachable!(),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::Path,sync::Arc};
    fn core()->Core {let (lib,_)=Library::load(Path::new("none"));Core::new(Project::starter(&lib),Arc::new(lib))}
    #[test]
    fn trace_observes_real_effect_and_safety_stages_without_a_recorder() {
        let mut c=core();c.set_attr(&Selector::Fixture(1),"pan",50.0);
        c.project.fixtures[0].limits.insert("pan".into(),[20,90]);
        c.project.effects.push(Effect {enabled:true,sel:Selector::Fixture(1),min:140.0,max:140.0,..Effect::new("Test","pan")});
        let ticket=c.start_trace(1,"pan").unwrap()["ticket"].as_u64().unwrap();
        c.render();let result=c.trace_result(ticket).unwrap();let stages=result["trace"]["stages"].as_array().unwrap();
        let value=|stage:&str|stages.iter().find(|s|s["stage"]==stage).unwrap()["channels"][0]["value"].as_f64().unwrap();
        assert_eq!(value("base"),50.0);assert_eq!(value("effects"),140.0);assert_eq!(value("safety"),90.0);assert_eq!(value("output"),90.0);
        assert!(result["trace"]["safety"].as_array().unwrap().iter().any(|s|s["reason"]=="limits"));
        assert!(c.rt.trace.is_none());assert!(c.rt.recorder.is_none());
    }
    #[test]
    fn blackout_and_disabled_output_are_distinct_in_trace() {
        let mut c=core();c.rt.flash=Some(true);c.rt.blackout=true;
        let ticket=c.start_trace(1,"dimmer").unwrap()["ticket"].as_u64().unwrap();c.render();
        let result=c.trace_result(ticket).unwrap();
        let stages=result["trace"]["stages"].as_array().unwrap();
        assert_eq!(stages.iter().find(|s|s["stage"]=="flash").unwrap()["channels"][0]["value"],255.0);
        assert_eq!(stages.iter().find(|s|s["stage"]=="gates").unwrap()["channels"][0]["value"],0.0);
        c.rt.blackout=false;c.project.fixtures[0].enabled=false;
        let ticket=c.start_trace(1,"dimmer").unwrap()["ticket"].as_u64().unwrap();c.render();
        let result=c.trace_result(ticket).unwrap();let output=result["trace"]["stages"].as_array().unwrap().last().unwrap();
        assert_eq!(output["details"]["enabled"],false);assert_eq!(output["channels"][0]["value"],0.0);
    }
    #[test]
    fn conditions_use_sixteen_bit_values_and_require_a_rendered_frame() {
        let mut c=core();c.project.fixtures.truncate(1);c.project.fixtures[0].mode=1;
        c.project.fixtures[0].values=c.lib.mode(&c.project.fixtures[0].profile,1).unwrap().default_values();
        c.set_attr(&Selector::Fixture(1),"pan",64.5);
        let condition=json!({"fixture":1,"attr":"pan","equals":64.5});
        assert_eq!(check(&c,&condition).unwrap()["passed"],false);c.render();
        assert_eq!(check(&c,&condition).unwrap()["passed"],true);
        assert_eq!(check(&c,&json!({"fixture":1,"attr":"pan","equals":65.0,"tolerance":0.5})).unwrap()["passed"],true);
        assert!(check(&c,&json!({"fixture":1,"attr":"pan","min":80,"max":20})).is_err());
    }
    #[test]
    fn schemas_follow_serde_enums_and_hide_secrets() {
        let mut c=core();c.project.web_pin="private-schema-pin".into();
        let schema=schema(&mut c,"project").unwrap();
        assert!(!schema.to_string().contains("private-schema-pin"));
        assert_eq!(schema["schema"]["properties"]["bpm"]["type"],"number");
        let shape=&schema["schema"]["$defs"]["Shape"];
        assert!(shape.to_string().contains("Points"),"{shape}");
    }
    #[test]
    fn stale_versions_and_restarted_sessions_are_rejected() {
        let mut c=core();let version=c.version();c.project.bpm=128.0;
        assert!(c.check_version(&json!({"expected_revision":version["revision"]})).is_err());
        assert!(c.check_version(&json!({"expected_session":"previous process"})).is_err());
        let current=c.version();c.check_version(&json!({"expected_session":current["session"],"expected_revision":current["revision"]})).unwrap();
    }
}
