//! On-demand inspection of actual render frames. Idle operation allocates no trace.
use dmxctl_core::{engine::Core, project::*};
use anyhow::{Context, ensure, bail};
use serde_json::{Value,json};
use std::time::{Duration,Instant};
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

/// schema で選べる対象
pub(crate) const SCHEMA_ENTITIES: [&str; 16] = [
    "project", "fixture", "binding", "group", "scene", "position", "effect", "pixel", "chase", "pair", "output", "auto", "auto_preset", "motion", "profile", "action",
];

const PROJECT_SCHEMA_READ_ONLY: &[&str] = &["next_id", "next_binding_id", "master_seeded"];

pub(crate) fn project_schema_summary(core: &mut Core) -> Value {
    let mut result = schema_metadata("project", core.version(), PROJECT_SCHEMA_READ_ONLY);
    summarize_project_schema(&mut result);
    result
}

pub(crate) fn summarize_project_schema(result: &mut Value) {
    if let Some(result) = result.as_object_mut() {
        result.remove("schema");
        result.remove("current");
        result.insert("entities".into(), json!(&SCHEMA_ENTITIES[1..]));
        result.insert("note".into(), json!("形と今の値は entity ごとに schema で取得する。設定全体が要るときは full: true"));
    }
}

fn schema_value<T:schemars::JsonSchema>()->Value {serde_json::to_value(schemars::schema_for!(T)).unwrap()}

pub fn schema(core:&mut Core,entity:&str)->anyhow::Result<Value> {
    let version=core.version();
    let project=crate::editing::redact(&serde_json::to_value(&core.project)?);
    let (schema,current,read_only)=match entity {
        "project"=>(schema_value::<Project>(),project,PROJECT_SCHEMA_READ_ONLY.to_vec()),
        "fixture"=>(schema_value::<FixtureInstance>(),project["fixtures"].clone(),vec!["id","armed","limits"]),
        "binding"=>(schema_value::<Binding>(),project["bindings"].clone(),vec!["id"]),
        "group"=>(schema_value::<Group>(),project["groups"].clone(),vec![]),
        "scene"|"position"=>(schema_value::<Scene>(),project[if entity=="scene"{"scenes"}else{"positions"}].clone(),vec![]),
        "effect"=>(schema_value::<Effect>(),project["effects"].clone(),vec![]),
        "pixel"=>(schema_value::<dmxctl_core::pixel::PixelFx>(),project["pixel_fx"].clone(),vec![]),
        "chase"=>(schema_value::<Chase>(),project["chases"].clone(),vec![]),
        "pair"=>(schema_value::<Pair>(),project["pairs"].clone(),vec![]),
        "output"=>(schema_value::<OutputCfg>(),project["outputs"].clone(),vec![]),
        "auto"=>(schema_value::<AutoCfg>(),project["auto"].clone(),vec![]),
        "auto_preset"=>(schema_value::<AutoPreset>(),project["auto"]["presets"].clone(),vec![]),
        "motion"=>(schema_value::<dmxctl_core::motion::Motion>(),project["motion"].clone(),vec!["pan_front","tilt_up"]),
        "profile"=>(schema_value::<dmxctl_core::fixture::Profile>(),json!(core.lib.profiles.iter().map(|p|p.as_ref()).collect::<Vec<_>>()),vec!["existing safety definitions"]),
        "action"=>(schema_value::<Action>(),Value::Null,vec!["Arm is not remotely executable"]),
        _=>bail!("unknown schema entity"),
    };
    let mut result = schema_metadata(entity, version, &read_only);
    result["schema"] = schema;
    result["current"] = current;
    Ok(result)
}

fn schema_metadata(entity: &str, version: Value, read_only: &[&str]) -> Value {
    json!({"entity":entity,"read_only":read_only,"version":version,
        "rules":{"bpm":[BPM_MIN,BPM_MAX],"slider":[0,1],"dmx":[0,255.99609375],"hold_ms":[1,5000],"scene_fade_seconds":[0,120],
            "indices":"zero-based; DMX addresses one-based","validation":"edit --dry-run also checks references, patch overlap and safety policy"},
        "commands":command_catalog()})
}

fn command_catalog()->Value {
    json!({"set":{"required":["fixture","attr","value"],"value_unit":"DMX"},
        "button":{"required":["control","event"],"event":["click","press","release","hold","set"],"value_range":[0,1],"hold_ms":[1,5000]},
        "input":{"source":["midi","key","osc"],"midi":{"kind":["Note","Cc"],"channel":[0,15],"number":[0,127],"value":[0,127]}},
        "edit":{"required":["operations"],"operations":["create","update","delete","capture"],"dry_run":"boolean"},
        "motion":{"required":["params"],"fields":["shape","beats","spread","position","size","points","glide"]},
        "effect":{"required":["name","params"]},"blackout":{"on":"boolean"},"master":{"value_range":[0,1]},
        "bpm":{"value_range":[BPM_MIN,BPM_MAX]},"scene":{"required":["name"]},"color":{"required":["name"],"slot":["main","sub"]},
        "flash":{"on":"boolean","hold_ms":[1,5000]},"fade":{"on":"boolean","secs":[0,60]},
        "other":["auto","auto_resume","preset","look","swap","tap","resync"]})
}

/// 本体へ1件の要求を送る口。`ctl` は TCP、MCP も同じ口を使う。テストでは差し替えられる
pub type Requester<'a>=&'a dyn Fn(&Value,Duration)->anyhow::Result<Value>;

pub fn wait_for(address:std::net::SocketAddr,target:&Value,timeout_ms:u64)->anyhow::Result<Value> {
    wait_with(&|request:&Value,timeout:Duration|crate::control::request_with_timeout(address,request,timeout),target,timeout_ms)
}

/// 待機中の1回の問い合わせに与える時間。待機の残りが短くても1秒は待つ。
/// 残りの時間をそのまま与えると、締め切りの直前の問い合わせが数ミリ秒で時間切れになり、
/// 待機の時間切れではなく接続の失敗として返ってしまう。Windows はタイマーの粒度が粗く、起きやすい
fn poll_timeout(remaining:Duration)->Duration {
    remaining.clamp(Duration::from_secs(1),Duration::from_secs(6))
}

/// `target` の条件(frame・beat・ms・condition)を満たすまで待つ
pub fn wait_with(request_with_timeout:Requester,target:&Value,timeout_ms:u64)->anyhow::Result<Value> {
    ensure!((1..=300000).contains(&timeout_ms),"timeout-ms must be in 1..300000");
    let started=Instant::now();let timeout=Duration::from_millis(timeout_ms);
    let mut last=Value::Null;
    loop {
        let remaining=timeout.saturating_sub(started.elapsed());
        ensure!(!remaining.is_zero(),"wait timed out; last observation: {last}");
        let mut request=target.get("condition").cloned().unwrap_or_else(||json!({"op":"clock"}));
        if target.get("condition").is_some(){request["op"]=json!("check");}
        last=request_with_timeout(&request,poll_timeout(remaining))?;
        // 1回の問い合わせには最低1秒を与えるので、答えが締め切りの後に届くことがある。
        // 締め切りを過ぎていたら、次の問い合わせにも成功の判定にも進まず、待機の時間切れにする
        ensure!(started.elapsed()<timeout,"wait timed out; last observation: {last}");
        let clock=if request["op"]=="check" {
            let clock=request_with_timeout(&json!({"op":"clock"}),poll_timeout(timeout.saturating_sub(started.elapsed())))?;
            ensure!(started.elapsed()<timeout,"wait timed out; last observation: {last}");
            clock
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
        "schema"|"version"=>println!("{}",crate::control::request(address,&payload)?),
        "assert"=>{
            payload["op"]=json!("check");let result=crate::control::request(address,&payload)?;
            println!("{result}");ensure!(result["passed"]==true,"output assertion failed");
        }
        "explain"=>{
            let requester=|request:&Value,timeout:Duration|crate::control::request_with_timeout(address,request,timeout);
            println!("{}",explain(&requester,payload,timeout)?);
        }
        "wait"=>{
            let frames=options.get("--frames").map(|v|v.parse::<u64>()).transpose()?;
            let beats=options.get("--beats").map(|v|v.parse::<f64>()).transpose()?;
            let ms=options.get("--ms").map(|v|v.parse::<u64>()).transpose()?;
            let requester=|request:&Value,timeout:Duration|crate::control::request_with_timeout(address,request,timeout);
            println!("{}",wait(&requester,frames,beats,ms,payload,timeout)?);
        }
        _=>unreachable!(),
    }
    Ok(())
}

/// 待つ。`frames`・`beats` は今からの量、`ms` は経過時間。`condition` に fixture・attr があれば、出力がその条件を満たすまで待つ
/// (`after_frame` だけなら、そのフレームまで待つ)
pub fn wait(request:Requester,frames:Option<u64>,beats:Option<f64>,ms:Option<u64>,condition:Value,timeout_ms:u64)->anyhow::Result<Value> {
    let clock=request(&json!({"op":"clock"}),Duration::from_secs(6))?;
    let mut target=json!({"session":clock["session"]});
    let mut criteria=false;
    if let Some(frames)=frames {
        ensure!(frames>0,"frames must be positive");
        target["frame"]=json!(clock["frame"].as_u64().context("clock has no frame")?.checked_add(frames).context("frame target overflow")?);criteria=true;
    }
    if let Some(beats)=beats {
        ensure!(beats.is_finite() && beats>0.0,"beats must be positive and finite");
        target["beat"]=json!(clock["beat"].as_f64().context("clock has no beat")?+beats);criteria=true;
    }
    if let Some(ms)=ms {target["ms"]=json!(ms);criteria=true;}
    if condition.get("fixture").is_some() {
        target["condition"]=condition;criteria=true;
    }else if let Some(frame)=condition.get("after_frame").and_then(Value::as_u64) {target["frame"]=json!(target["frame"].as_u64().unwrap_or(0).max(frame));criteria=true;}
    ensure!(criteria,"specify frames, beats, ms or an output condition");
    wait_with(request,&target,timeout_ms)
}

/// 次の実フレームで、`payload` の fixture・attr の値がどう決まったかを返す
pub fn explain(request:Requester,mut payload:Value,timeout_ms:u64)->anyhow::Result<Value> {
    ensure!((1..=300000).contains(&timeout_ms),"invalid timeout-ms");payload["op"]=json!("trace_start");
    let started=Instant::now();let timeout=Duration::from_millis(timeout_ms);
    let start=request(&payload,timeout.min(Duration::from_secs(6)))?;
    let ticket=start["ticket"].clone();
    let result=(|| loop {
        let remaining=timeout.saturating_sub(started.elapsed());ensure!(!remaining.is_zero(),"trace timed out");
        let result=request(&json!({"op":"trace_result","ticket":ticket}),poll_timeout(remaining))?;
        // 締め切りの後に届いた答えは、結果ができていても時間切れにする(wait と同じ)。
        // このとき結果は本体から受け取り済みで、下の trace_cancel はチケットが合わず何もしない
        ensure!(started.elapsed()<timeout,"trace timed out");
        if result["ready"]==true {return Ok(result["trace"].clone());}
        std::thread::sleep(Duration::from_millis(25).min(timeout.saturating_sub(started.elapsed())));
    })();
    // 結果を受け取らずに終わる(キャンセル・時間切れ・失敗)ときは、本体のトレースの枠を空ける。
    // 空けないと、次の explain が最大5秒間 another trace is pending になる
    if result.is_err() {let _=request(&json!({"op":"trace_cancel","ticket":ticket}),Duration::from_secs(2));}
    result
}

#[cfg(test)]
mod tests {
    use std::{path::Path,sync::Arc};
    use super::*;
    use dmxctl_core::fixture::Library;
    fn core()->Core {let (lib,_)=Library::load(Path::new("none"));Core::new(Project::starter(&lib),Arc::new(lib))}
    #[test]
    fn cancelled_trace_frees_the_slot_for_the_next_one() {
        let mut c=core();
        let ticket=c.start_trace(1,"pan").unwrap()["ticket"].as_u64().unwrap();
        assert!(c.start_trace(1,"pan").unwrap_err().to_string().contains("another trace is pending"));
        // 違うチケットでは空けない
        assert_eq!(c.cancel_trace(ticket+1)["cancelled"],false);
        assert!(c.start_trace(1,"pan").is_err());
        assert_eq!(c.cancel_trace(ticket)["cancelled"],true);
        // すぐ次のトレースを始められる
        let next=c.start_trace(1,"pan").unwrap()["ticket"].as_u64().unwrap();
        assert!(next>ticket);
    }

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
        assert!(!c.rt.observe.is_tracing());assert!(!c.rt.observe.is_recording());
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
    /// 締め切りの後に届いた答えは、条件を満たしていても成功にしない。条件付きの待機では、
    /// 締め切りを過ぎた後に clock を問い合わせない
    #[test]
    fn answers_arriving_after_the_deadline_are_timeouts_even_if_they_match() {
        let log=std::sync::Mutex::new(Vec::new());
        // 1回の答えに 40ms かかり、条件はいつも満たしている本体
        let slow=|request:&Value,_timeout:Duration|->anyhow::Result<Value> {
            log.lock().unwrap().push(request["op"].as_str().unwrap_or("").to_string());
            std::thread::sleep(Duration::from_millis(40));
            Ok(match request["op"].as_str() {
                Some("check")=>json!({"passed":true,"frame":5}),
                Some("trace_start")=>json!({"ticket":1}),
                Some("trace_result")=>json!({"ready":true,"trace":{}}),
                _=>json!({"frame":5,"beat":9.0,"session":"s","emergency":false}),
            })
        };
        for target in [json!({"frame":1}),json!({"condition":{"fixture":1,"attr":"dimmer","equals":7}})] {
            log.lock().unwrap().clear();
            let error=wait_with(&slow,&target,30).unwrap_err().to_string();
            assert!(error.starts_with("wait timed out"),"{target}: {error}");
            assert!(!log.lock().unwrap().iter().any(|op|op=="clock" && target.get("condition").is_some()),"締め切り後に clock を問い合わせない: {:?}",log.lock().unwrap());
            // 締め切りに間に合えば成功する
            assert_eq!(wait_with(&slow,&target,1000).unwrap()["matched"],true);
        }
        // explain も同じ。trace_start(40ms)の後の trace_result が締め切り(60ms)を過ぎて届く
        let error=explain(&slow,json!({"fixture":1,"attr":"pan"}),60).unwrap_err().to_string();
        assert_eq!(error,"trace timed out");
        assert!(explain(&slow,json!({"fixture":1,"attr":"pan"}),1000).is_ok());
    }

    #[test]
    fn waits_report_their_own_timeout_even_when_the_last_poll_is_slow() {
        // 1回の答えに 40ms かかる本体。与えられた時間がそれより短ければ、接続の時間切れになる
        let slow=|request:&Value,timeout:Duration|->anyhow::Result<Value> {
            anyhow::ensure!(timeout>=Duration::from_millis(40),"os error 10060");
            std::thread::sleep(Duration::from_millis(40));
            Ok(match request["op"].as_str() {
                Some("check")=>json!({"passed":false,"frame":1}),
                _=>json!({"frame":1,"beat":0.0,"session":"s","emergency":false}),
            })
        };
        for target in [json!({"frame":1000}),json!({"condition":{"fixture":1,"attr":"dimmer","equals":7}})] {
            for timeout_ms in [1,30,95,130] {
                let error=wait_with(&slow,&target,timeout_ms).unwrap_err().to_string();
                assert!(error.starts_with("wait timed out"),"{target} {timeout_ms}ms: {error}");
            }
        }
    }
}
