//! 指定した機材の1フレームを、合成の段階ごとに記録する `Trace`。呼ばれたときだけ作るので、普段は何も確保しない。
use crate::{engine::Core, fixture::{Library, Mode}, project::*};
use anyhow::{Context, ensure};
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
        ensure!(!crate::engine::EMERGENCY.load(std::sync::atomic::Ordering::SeqCst),"explain is unavailable during emergency stop; use output snapshot");
        let f=self.project.fixtures.iter().find(|f|f.id==fixture).context("unknown fixture")?;
        let mode=self.lib.mode(&f.profile,f.mode).context("unknown profile/mode")?;
        ensure!(!mode.attr_channels(attr).is_empty(),"unknown attribute");
        let now=self.now();
        ensure!(self.rt.observe.trace.as_ref().is_none_or(|t|now.saturating_duration_since(t.started)>Duration::from_secs(5)),"another trace is pending");
        self.rt.observe.trace_ticket=self.rt.observe.trace_ticket.checked_add(1).context("trace ticket exhausted")?;
        let ticket=self.rt.observe.trace_ticket;
        self.rt.observe.trace=Some(Trace {ticket,fixture,attr:attr.into(),started:now,frame:None,stages:Vec::new(),result:None});
        Ok(json!({"ticket":ticket,"from_frame":self.rt.frame_id+1}))
    }
    pub fn trace_result(&mut self,ticket:u64)->anyhow::Result<Value> {
        let trace=self.rt.observe.trace.as_ref().context("trace is no longer available")?;
        ensure!(trace.ticket==ticket,"trace ticket does not match");
        if trace.result.is_none() {return Ok(json!({"ready":false}));}
        let mut trace=self.rt.observe.trace.take().unwrap();
        Ok(json!({"ready":true,"trace":trace.result.take()}))
    }
    /// 取りやめたトレースの枠を空ける(結果を取りに来ない呼び出し元のため)。チケットが違えば何もしない
    pub fn cancel_trace(&mut self,ticket:u64)->Value {
        let cancelled=self.rt.observe.trace.as_ref().is_some_and(|t|t.ticket==ticket);
        if cancelled {self.rt.observe.trace=None;}
        json!({"cancelled":cancelled})
    }
    pub(crate) fn trace_begin(&mut self) {
        if let Some(trace)=self.rt.observe.trace.as_mut() {
            if trace.result.is_none() {trace.stages.clear();trace.frame=Some(self.rt.frame_id+1);}
        }
        self.trace_stage("base",None,Value::Null);
    }
    pub(crate) fn trace_stage(&mut self,stage:&str,values:Option<&[Vec<u8>]>,details:Value) {
        if let Some(trace)=self.rt.observe.trace.as_mut() {trace.capture(&self.project,&self.lib,values,stage,details);}
    }
    pub(crate) fn trace_finish(&mut self) {
        let t_ms=self.now().saturating_duration_since(self.rt.t0).as_millis() as u64;
        let Some(trace)=self.rt.observe.trace.as_mut().filter(|t|t.result.is_none()) else{return;};
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

