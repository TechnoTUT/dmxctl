//! Replay GUI bindings and external inputs through the existing engine actions.
use crate::core::{engine::MidiMsg, project::{Action, PadMode, TriggerKind}};
use crate::core::hold::{Feedback, Hold};
use crate::core::io::osc::{Arg, Message};
use anyhow::{Context, ensure, bail};
use serde_json::{Value, json};
use std::time::Duration;

fn continuous(action: &Action) -> bool {
    use crate::core::{motion::MotionOp, project::TempoOp};
    matches!(action,Action::SetValue{mode:PadMode::Continuous,..}|Action::Master{mode:PadMode::Continuous,..}
        |Action::Color{color:None,..}|Action::Tempo{op:TempoOp::Set{..}}|Action::Level{..}|Action::Pixel{..}
        |Action::Motion{op:MotionOp::Size|MotionOp::Glide})
}

pub(crate) fn is_slider(action:&Action)->bool {continuous(action)}

fn duration(value: &Value, default: u64) -> anyhow::Result<Duration> {
    let ms=value.get("hold_ms").map(|v|v.as_u64().context("hold_ms must be an integer")).transpose()?.unwrap_or(default);
    ensure!((1..=5000).contains(&ms),"hold_ms must be in 1..5000");
    Ok(Duration::from_millis(ms))
}

fn permitted(action: &Action) -> anyhow::Result<()> {
    ensure!(!matches!(action,Action::Arm{..}),"ARM cannot be operated through remote controls");
    Ok(())
}

pub trait CoreControls {
    fn controls(&mut self) -> Value;
    fn remote_button(&mut self, value:&Value) -> anyhow::Result<()>;
    fn remote_input(&mut self, value:&Value) -> anyhow::Result<()>;
    fn preflight(&self,hits:&[usize]) -> anyhow::Result<()>;
    fn track_input(&mut self,hits:&[usize],pressed:bool,duration:Duration,feedback:Feedback,skip_sliders:bool);
}

impl CoreControls for crate::core::engine::Core {
    fn controls(&mut self) -> Value {
        self.project.ensure_binding_ids();
        json!(self.project.bindings.iter().enumerate().map(|(index,b)|json!({
            "id":format!("binding:{}",b.id),"name":b.name,"kind":if continuous(&b.action){"slider"}else{"button"},
            "page":b.page,"trigger":b.trigger,"osc":b.osc,"action":b.action,
            "pressed":self.rt.pad_state.get(&index).copied().unwrap_or(false),
            "remote_held":self.rt.remote_holds.contains_key(&b.id),"remote_allowed":permitted(&b.action).is_ok()
        })).collect::<Vec<_>>())
    }

    fn remote_button(&mut self, value:&Value) -> anyhow::Result<()> {
        self.project.ensure_binding_ids();
        let control=value["control"].as_str().context("control is required")?;
        let id:u64=control.strip_prefix("binding:").context("expected binding:ID")?.parse()?;
        let index=self.project.bindings.iter().position(|b|b.id==id).context("unknown control")?;
        let action=self.project.bindings[index].action.clone();
        permitted(&action)?;
        let event=value["event"].as_str().context("event is required")?;
        if event=="release" {
            if let Some(hold)=self.rt.remote_holds.remove(&id) { self.release_hold(id,hold,"release"); }
            return Ok(());
        }
        if event=="set" {
            ensure!(continuous(&action),"set requires a slider control");
            let value=value["value"].as_f64().context("numeric value is required")?;
            ensure!((0.0..=1.0).contains(&value),"slider value must be in 0..1");
            self.note_manual();
            self.apply_binding(index,value as f32,false);
            return Ok(());
        }
        ensure!(["click","press","hold"].contains(&event),"unsupported control event");
        ensure!(!continuous(&action),"use set for a slider control");
        ensure!(!self.rt.pad_state.get(&index).copied().unwrap_or(false) || self.rt.remote_holds.contains_key(&id),"control is held by another input");
        ensure!(event!="click" || !self.rt.remote_holds.contains_key(&id),"control is already held");
        let duration=duration(value,if event=="click"{50}else{1000})?;
        self.note_manual();
        self.apply_binding(index,1.0,true);
        self.rt.remote_holds.insert(id,Hold {action,until:self.now()+duration,feedback:Feedback::None});
        Ok(())
    }

    fn remote_input(&mut self, value:&Value) -> anyhow::Result<()> {
        self.project.ensure_binding_ids();
        ensure!(self.rt.learn.is_none() && self.rt.osc_learn.is_none(),"finish MIDI/OSC learn before replaying input");
        let source=value["source"].as_str().context("source is required")?;
        let mut hits=Vec::new();
        match source {
            "midi" => {
                let kind=match value["kind"].as_str() {Some("Note")=>TriggerKind::Note,Some("Cc")=>TriggerKind::Cc,_=>bail!("kind must be Note or Cc")};
                let channel=byte(value,"channel",15)?;
                let number=byte(value,"number",127)?;
                let input=byte(value,"value",127)?;
                for (i,b) in self.project.bindings.iter().enumerate() {
                    if b.trigger.is_some_and(|t|t.kind==kind && t.number==number && t.channel.is_none_or(|c|c==channel))
                        && (b.on_page(self.rt.page) || (kind==TriggerKind::Note && self.rt.pad_state.get(&i)==Some(&true))) {hits.push(i);}
                }
                let pressed=if kind==TriggerKind::Note {input>0}else{input>=64};
                self.preflight(&hits)?;
                let duration=duration(value,1000)?;
                self.handle_midi(MidiMsg {kind,channel,number,value:input});
                self.track_input(&hits,pressed,duration,if kind==TriggerKind::Note {Feedback::Note(number)}else{Feedback::None},kind==TriggerKind::Cc);
                return Ok(());
            }
            "key" => {
                let number=byte(value,"number",u8::MAX)?;
                ensure!((number as u16)<crate::core::keys::SHIFT as u16*2,"unknown key number");
                ensure!(crate::core::keys::name(number)!="?" && crate::core::keys::name(number)!="Shift+?","unknown key number");
                let pressed=value["pressed"].as_bool().context("pressed must be boolean")?;
                for (i,b) in self.project.bindings.iter().enumerate() {
                    if b.trigger.is_some_and(|t|t.kind==TriggerKind::Key && t.number==number)
                        && (b.page.is_none_or(|p|p==self.rt.key_page) || self.rt.pad_state.get(&i)==Some(&true)) {hits.push(i);}
                }
                self.preflight(&hits)?;
                let duration=duration(value,1000)?;
                self.handle_key(number,pressed);
                self.track_input(&hits,pressed,duration,Feedback::Key(number),false);
                return Ok(());
            }
            "osc" => {
                let addr=value["address"].as_str().context("OSC address is required")?;
                ensure!(addr.starts_with('/') && addr.len()<=1024,"invalid OSC address");
                let args:Vec<Arg>=value["args"].as_array().context("args array is required")?.iter().map(|v| {
                    if let Some(v)=v.as_bool(){Ok(Arg::Bool(v))}
                    else if let Some(v)=v.as_i64(){Ok(Arg::Int(v))}
                    else if let Some(v)=v.as_f64(){ensure!((v as f32).is_finite(),"OSC number is too large");Ok(Arg::Float(v))}
                    else if let Some(v)=v.as_str(){Ok(Arg::Str(v.to_string()))}
                    else {bail!("OSC args must be numbers, booleans or strings")}
                }).collect::<anyhow::Result<_>>()?;
                for (i,b) in self.project.bindings.iter().enumerate() {
                    if b.osc.as_deref()==Some(addr) {hits.push(i);}
                    if addr.starts_with("/dmx/phrase") && b.osc.as_deref().is_some_and(|a|a.starts_with("/dmx/phrase/")) {permitted(&b.action)?;}
                }
                self.preflight(&hits)?;
                let first=args.iter().find_map(Arg::num);
                let duration=duration(value,1000)?;
                self.handle_osc(&Message {addr:addr.to_string(),args});
                if addr=="/dmx/flash" || addr=="/dmx/flash/color" {
                    self.rt.flash_deadline=self.rt.flash.map(|_|self.now()+duration);
                }
                if first.is_none() {return Ok(());}
                let pressed=first.unwrap()>=0.5;
                self.track_input(&hits,pressed,duration,Feedback::None,true);
            }
            _=>bail!("source must be midi, key or osc"),
        }
        Ok(())
    }

    fn preflight(&self,hits:&[usize]) -> anyhow::Result<()> {
        for &i in hits {
            let binding=&self.project.bindings[i];
            permitted(&binding.action)?;
            ensure!(continuous(&binding.action) || !self.rt.pad_state.get(&i).copied().unwrap_or(false) || self.rt.remote_holds.contains_key(&binding.id),"control is held by another input");
        }
        Ok(())
    }

    fn track_input(&mut self,hits:&[usize],pressed:bool,duration:Duration,feedback:Feedback,skip_sliders:bool) {
        for &i in hits {
            let b=&self.project.bindings[i];
            if !pressed {self.rt.remote_holds.remove(&b.id);}
            else if !skip_sliders || !continuous(&b.action) {
                self.rt.remote_holds.insert(b.id,Hold {action:b.action.clone(),until:self.now()+duration,feedback:feedback.clone()});
            }
        }
    }
}

fn byte(value:&Value,key:&str,max:u8) -> anyhow::Result<u8> {
    let v=value[key].as_u64().with_context(||format!("{key} must be an integer"))?;
    ensure!(v<=max as u64,"{key} is out of range");
    Ok(v as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::engine::Core;
    use crate::core::{fixture::Library, project::*};
    use std::{sync::Arc,path::Path};
    fn core()->Core {
        let (lib,_)=Library::load(Path::new("none"));
        let mut project=Project::starter(&lib);
        project.bindings.clear();
        for (name,action,number) in [
            ("Flash",Action::Flash{white:true,mode:PadMode::Momentary},40),
            ("Arm",Action::Arm{sel:Selector::All,mode:PadMode::Toggle},41),
            ("Master",Action::Master{mode:PadMode::Continuous,on:100.0,off:0.0},42),
        ] {
            project.bindings.push(Binding{id:0,name:name.into(),action,
                trigger:Some(MidiTrigger{kind:TriggerKind::Note,channel:Some(9),number}),page:None,osc:Some(format!("/{name}"))});
        }
        Core::new(project,Arc::new(lib))
    }
    fn control(core:&Core,index:usize)->String {format!("binding:{}",core.project.bindings[index].id)}
    #[test]
    fn identities_survive_save_reorder_and_deletion() {
        let mut c=core();
        let first=c.project.bindings[0].id;
        let last=c.project.bindings.last().unwrap().id;
        let serialized=serde_json::to_string(&c.project).unwrap();
        let mut project:Project=serde_json::from_str(&serialized).unwrap();
        project.bindings.reverse();
        project.bindings.retain(|b|b.id!=last);
        let mut binding=project.bindings[0].clone(); binding.id=0;
        project.bindings.push(binding);project.ensure_binding_ids();
        assert!(project.bindings.last().unwrap().id>last);
        assert!(project.bindings.iter().any(|b|b.id==first && b.name=="Flash"));
        c.project=project;
        assert!(c.remote_button(&json!({"control":format!("binding:{last}"),"event":"click"})).is_err());
    }
    #[test]
    fn click_and_hold_release_on_the_engine_clock() {
        let mut c=core(); let id=control(&c,0);let start=c.now();c.rt.virtual_now=Some(start);
        c.remote_button(&json!({"control":id,"event":"click"})).unwrap();
        c.render();assert_eq!(c.rt.flash,Some(true));
        c.rt.virtual_now=Some(start+Duration::from_millis(60));c.render();assert_eq!(c.rt.flash,None);
        c.remote_button(&json!({"control":id,"event":"hold","hold_ms":100})).unwrap();
        c.project.bindings.clear();
        c.rt.virtual_now=Some(start+Duration::from_millis(170));c.render();assert_eq!(c.rt.flash,None);
        assert!(!c.rt.pad_state.contains_key(&usize::MAX));
    }
    #[test]
    fn arm_is_rejected_before_any_matching_action_runs() {
        let mut c=core();let arm=control(&c,1);
        assert!(c.remote_button(&json!({"control":arm,"event":"click"})).is_err());
        c.project.bindings[1].trigger=c.project.bindings[0].trigger;
        assert!(c.remote_input(&json!({"source":"midi","kind":"Note","channel":9,"number":40,"value":127})).is_err());
        assert_eq!(c.rt.flash,None);
        assert!(c.project.fixtures.iter().all(|f|!f.armed));
    }
    #[test]
    fn midi_key_and_osc_share_the_existing_binding_dispatch() {
        let mut c=core();let start=c.now();c.rt.virtual_now=Some(start);
        c.remote_input(&json!({"source":"midi","kind":"Note","channel":9,"number":40,"value":127,"hold_ms":50})).unwrap();
        assert_eq!(c.rt.flash,Some(true));
        c.rt.virtual_now=Some(start+Duration::from_millis(60));c.render();
        assert_eq!(c.rt.flash,None);assert!(!c.rt.notes_down[40]);
        c.project.bindings[0].trigger=Some(MidiTrigger{kind:TriggerKind::Key,channel:None,number:16});
        c.remote_input(&json!({"source":"key","number":16,"pressed":true})).unwrap();assert_eq!(c.rt.flash,Some(true));
        c.remote_input(&json!({"source":"key","number":16,"pressed":false})).unwrap();assert_eq!(c.rt.flash,None);
        c.remote_input(&json!({"source":"osc","address":"/Flash","args":[1]})).unwrap();assert_eq!(c.rt.flash,Some(true));
        c.remote_input(&json!({"source":"osc","address":"/Flash","args":[0]})).unwrap();assert_eq!(c.rt.flash,None);
    }
    #[test]
    fn sliders_validate_and_builtin_osc_flash_also_expires() {
        let mut c=core();let id=control(&c,2);let start=c.now();c.rt.virtual_now=Some(start);
        c.remote_button(&json!({"control":id,"event":"set","value":0.4})).unwrap();assert!((c.rt.master-0.4).abs()<0.001);
        assert!(c.remote_button(&json!({"control":id,"event":"set","value":2})).is_err());
        c.remote_input(&json!({"source":"osc","address":"/dmx/flash","args":[1],"hold_ms":50})).unwrap();
        c.rt.virtual_now=Some(start+Duration::from_millis(60));c.render();assert_eq!(c.rt.flash,None);
    }
    #[test]
    fn local_flash_is_not_cut_by_a_pending_remote_deadline() {
        let mut c=core();let start=c.now();c.rt.virtual_now=Some(start);
        crate::backend::commands::apply(&mut c,&json!({"cmd":"flash","on":true,"hold_ms":50})).unwrap();
        assert_eq!(c.rt.flash_deadline,Some(start+Duration::from_millis(50)),"deadline uses the engine clock");
        // 期限が来る前に、人が操作パネル/OSC/MIDIでフラッシュを押し直す
        crate::backend::web::command(&mut c,&json!({"cmd":"flash","on":true})).unwrap();
        c.rt.virtual_now=Some(start+Duration::from_millis(60));c.render();
        assert_eq!(c.rt.flash,Some(true));
        c.handle_osc(&crate::core::io::osc::Message{addr:"/dmx/flash".into(),args:vec![crate::core::io::osc::Arg::Int(1)]});
        c.render();assert_eq!(c.rt.flash,Some(true));
    }
}
