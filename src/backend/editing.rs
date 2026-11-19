//! Project edits are built on a copy, checked as a whole, then committed once.
use crate::backend::controls::CoreControls;
use crate::core::{engine::Core, project::*, fixture::Library};
use anyhow::{Context, ensure, bail};
use serde_json::{Value, json};
use std::collections::{HashSet, BTreeMap};

fn collection(entity:&str)->anyhow::Result<&'static str> {
    Ok(match entity {
        "fixture"=>"fixtures","group"=>"groups","binding"=>"bindings","scene"=>"scenes",
        "position"=>"positions","effect"=>"effects","pixel"=>"pixel_fx","chase"=>"chases",
        "pair"=>"pairs","output"=>"outputs","auto_preset"=>"auto.presets",
        _=>bail!("unknown entity: {entity}"),
    })
}

pub fn list(core:&Core,entity:&str)->anyhow::Result<Value> {
    let project=serde_json::to_value(&core.project)?;
    if entity=="settings" {return Ok(redact(&project));}
    Ok(json!(array(&project,collection(entity)?)?.iter().enumerate().map(|(index,value)|json!({"index":index,"data":redact(value)})).collect::<Vec<_>>()))
}

fn array<'a>(project:&'a Value,field:&str)->anyhow::Result<&'a Vec<Value>> {
    let value=if field=="auto.presets" {&project["auto"]["presets"]}else{&project[field]};
    value.as_array().context("collection is not an array")
}
fn array_mut<'a>(project:&'a mut Value,field:&str)->anyhow::Result<&'a mut Vec<Value>> {
    let value=if field=="auto.presets" {&mut project["auto"]["presets"]}else{&mut project[field]};
    value.as_array_mut().context("collection is not an array")
}

fn select(items:&[Value],operation:&Value)->anyhow::Result<usize> {
    if let Some(index)=operation.get("index") {
        let index=usize::try_from(index.as_u64().context("index must be an integer")?)?;
        ensure!(index<items.len(),"index is out of range"); return Ok(index);
    }
    let key=operation.get("key").context("key or index is required")?;
    let hits:Vec<_>=items.iter().enumerate().filter(|(_,item)| {
        if let Some(id)=key.as_u64() {item["id"]==id}
        else if let Some(key)=key.as_str() {
            if let Some(id)=key.strip_prefix("binding:").and_then(|s|s.parse::<u64>().ok()).filter(|_|item.get("id").is_some()) {item["id"]==id}
            else {item["name"]==key}
        } else {false}
    }).map(|(i,_)|i).collect();
    ensure!(hits.len()==1,"key must identify exactly one item (use index for grouped effects)");
    Ok(hits[0])
}

fn numbers(value:&Value)->anyhow::Result<()> {
    match value {
        Value::Number(n)=>{if let Some(n)=n.as_f64(){ensure!((n as f32).is_finite(),"numeric value is too large");}},
        Value::Array(values)=>for value in values {numbers(value)?;},
        Value::Object(values)=>for value in values.values(){numbers(value)?;},
        _=>{},
    }
    Ok(())
}

fn patch(target:&mut Value,data:&Value,extra:&[&str])->anyhow::Result<()> {
    let object=data.as_object().context("data must be an object")?;
    for (key,value) in object {
        ensure!(target.get(key).is_some() || extra.contains(&key.as_str()),"unknown field: {key}");
        if value.is_object() && target[key].is_object() && !["action","sel","trigger","kind","looks"].contains(&key.as_str()) {
            patch(&mut target[key],value,&[])?;
        }else {target[key]=value.clone();}
    }
    Ok(())
}

fn template(entity:&str)->anyhow::Result<Value> {
    Ok(match entity {
        "binding"=>json!({"id":0,"name":"Binding","trigger":null,"page":null,"osc":null,"action":{"Blackout":{"mode":"Toggle"}}}),
        "group"=>json!({"name":"Group","fixtures":[]}),
        "scene"|"position"=>json!({"name":"Scene","fade":0,"values":[]}),
        "effect"=>serde_json::to_value(Effect::new("Effect","dimmer"))?,
        "pixel"=>serde_json::to_value(crate::core::pixel::PixelFx::default())?,
        "chase"=>serde_json::to_value(Chase::default())?,
        "pair"=>serde_json::to_value(Pair::default())?,
        "auto_preset"=>serde_json::to_value(AutoPreset::default())?,
        "output"=>json!({"enabled":false,"universe":0,"kind":{"ArtNet":{"ip":"127.0.0.1","art_universe":0}}}),
        _=>bail!("no default for {entity}"),
    })
}

fn new_fixture(project:&mut Value,data:&Value,lib:&Library)->anyhow::Result<Value> {
    ensure!(data.get("id").is_none() && data.get("armed").is_none() && data.get("limits").is_none(),"fixture identity and safety fields are read-only");
    let profile=data["profile"].as_str().context("fixture profile is required")?;
    let mode=data.get("mode").map(|v|v.as_u64().context("mode must be an integer")).transpose()?.unwrap_or(0) as usize;
    let definition=lib.mode(profile,mode).context("unknown fixture profile/mode")?;
    let universe=data.get("universe").map(|v|v.as_u64().context("universe must be an integer")).transpose()?.unwrap_or(0) as usize;
    let items=array(project,"fixtures")?;
    let highest=items.iter().filter_map(|f|f["id"].as_u64()).max().unwrap_or(0);
    let id=project["next_id"].as_u64().unwrap_or(1).max(highest.checked_add(1).context("fixture ID exhausted")?);
    ensure!(id<u32::MAX as u64,"fixture ID exhausted");
    let mut used=[false;512];
    for f in items.iter().filter(|f|f["universe"]==universe as u64) {
        let Some(mode)=f["profile"].as_str().and_then(|p|lib.mode(p,f["mode"].as_u64().unwrap_or(0) as usize)) else{continue;};
        let start=f["address"].as_u64().unwrap_or(1).saturating_sub(1) as usize;
        for slot in start..start.saturating_add(mode.channels.len()).min(512) {used[slot]=true;}
    }
    let n=definition.channels.len();
    ensure!(n>0 && n<=512,"invalid channel count");
    let address=if let Some(address)=data.get("address") {address.as_u64().context("address must be an integer")?}
        else {(0..=512-n).find(|&start|!used[start..start+n].iter().any(|&v|v)).context("no free DMX address")? as u64+1};
    let mut fixture=json!({"id":id,"name":lib.get(profile).unwrap().name,"profile":profile,"mode":mode,
        "universe":universe,"address":address,"values":definition.default_values(),"color_role":"Main","enabled":true});
    patch(&mut fixture,data,&[])?;
    project["next_id"]=json!(id+1);
    Ok(fixture)
}

pub(crate) fn ensure_idle(core:&Core)->anyhow::Result<()> {
    ensure!(core.rt.learn.is_none() && core.rt.osc_learn.is_none(),"finish MIDI/OSC learn before editing");
    ensure!(core.rt.remote_holds.is_empty(),"release held controls before editing");
    for (&i,&pressed) in &core.rt.pad_state {
        if pressed && core.project.bindings.get(i).is_some_and(|b|!crate::backend::controls::is_slider(&b.action)) {
            bail!("release held GUI/MIDI controls before editing");
        }
    }
    Ok(())
}

pub fn apply(core:&mut Core,operations:&Value,dry_run:bool)->anyhow::Result<Value> {
    apply_with_policy(core,operations,dry_run,true)
}

pub(crate) fn gui_apply(core:&mut Core,operations:&Value)->anyhow::Result<Value> {
    apply_with_policy(core,operations,false,false)
}

fn apply_with_policy(core:&mut Core,operations:&Value,dry_run:bool,remote:bool)->anyhow::Result<Value> {
    ensure_idle(core)?;
    let operations=operations.as_array().context("operations must be an array")?;
    ensure!(!operations.is_empty() && operations.len()<=256,"expected 1..256 edit operations");
    let mut data=serde_json::to_value(&core.project)?;
    let mut results=Vec::new();
    for operation in operations {
        numbers(operation)?;
        let entity=operation["entity"].as_str().context("entity is required")?;
        let op=operation["op"].as_str().context("op is required")?;
        if entity=="settings" {
            ensure!(op=="update","settings supports update only");
            let settings=operation["data"].as_object().context("data must be an object")?;
            for key in settings.keys() {
                ensure!(["bpm","tempo_source","midi_input","clock_input","audio_input","osc_enabled","osc_port","web_enabled","web_port","web_pin",
                    "osc_rgb_secs","osc_level_depth","auto","palette","fade_secs","led_enabled","led_idle","led_true_color","ndi","motion"].contains(&key.as_str()),"setting is read-only or unknown: {key}");
            }
            patch(&mut data,&operation["data"],&[])?;
            results.push(json!({"entity":entity,"op":op}));continue;
        }
        let field=collection(entity)?;
        if op=="capture" {
            ensure!(entity=="scene" || entity=="position","capture supports scenes and positions only");
            let name=operation["data"]["name"].as_str().context("capture name is required")?;
            let draft:Project=serde_json::from_value(data.clone())?;
            let ids:Vec<u32>=operation["data"].get("fixtures").map(|v|serde_json::from_value(v.clone())).transpose()?.unwrap_or_else(||draft.fixtures.iter().map(|f|f.id).collect());
            ensure!(ids.iter().all(|id|draft.fixture_index(*id).is_some()),"unknown capture fixture");
            let fade=operation["data"].get("fade").map(|v|v.as_f64().context("fade must be numeric")).transpose()?.unwrap_or(0.0) as f32;
            let skip:HashSet<String>=operation["data"].get("skip_categories").map(|v|serde_json::from_value(v.clone())).transpose()?.unwrap_or_default();
            let temporary=Core::new(draft,core.lib.clone());
            let mut scene=temporary.capture_scene(name,fade,&ids,&skip);
            if entity=="position" {scene.values.retain(|v|v.attr=="pan" || v.attr=="tilt");}
            let items=array_mut(&mut data,field)?;
            if let Some(i)=items.iter().position(|s|s["name"]==name) {items[i]=serde_json::to_value(scene)?;}else{items.push(serde_json::to_value(scene)?);}
            results.push(json!({"entity":entity,"name":name}));continue;
        }
        let index=match op {
            "create"=>{
                let mut item=if entity=="fixture" {new_fixture(&mut data,&operation["data"],&core.lib)?}
                    else {let mut item=template(entity)?;patch(&mut item,&operation["data"],&[])?;item};
                if entity=="binding" {ensure!(operation["data"].get("id").is_none(),"binding IDs are assigned by the application");item["id"]=json!(0);}
                let items=array_mut(&mut data,field)?;items.push(item);items.len()-1
            }
            "update"=>{
                let index=select(array(&data,field)?,operation)?;
                let changes=&operation["data"];
                if entity=="fixture" {ensure!(changes.get("id").is_none() && changes.get("armed").is_none() && changes.get("limits").is_none(),"fixture identity and safety fields are read-only");}
                if entity=="binding" {ensure!(changes.get("id").is_none(),"binding ID is read-only");}
                if entity=="fixture" && (changes.get("profile").is_some() || changes.get("mode").is_some()) {
                    let item=&mut array_mut(&mut data,field)?[index];
                    let profile=changes.get("profile").unwrap_or(&item["profile"]).as_str().context("profile must be a string")?;
                    let mode=changes.get("mode").unwrap_or(&item["mode"]).as_u64().context("mode must be an integer")? as usize;
                    item["values"]=json!(core.lib.mode(profile,mode).context("unknown profile/mode")?.default_values());
                }
                patch(&mut array_mut(&mut data,field)?[index],changes,if entity=="binding"{&["page","osc"]}else{&[]})?;
                index
            }
            "delete"=>{
                let index=select(array(&data,field)?,operation)?;array_mut(&mut data,field)?.remove(index);index
            }
            _=>bail!("op must be create, update, delete or capture"),
        };
        results.push(json!({"entity":entity,"op":op,"index":index}));
    }
    let mut project:Project=serde_json::from_value(data)?;
    project.ensure_binding_ids();
    if remote {safety_unchanged(&core.project,&project,&core.lib)?;}
    validate(&project,&core.lib)?;
    if !dry_run {commit(core,project);}
    Ok(json!({"applied":!dry_run,"operations":results,"bindings":if dry_run{None}else{Some(core.controls())}}))
}

/// 画面から機材を消す。その機材を指す割り当てやエフェクトも一緒に片付けてから検証し、まとめて反映する。
/// 一緒に消したものの名前を返す
pub(crate) fn gui_delete_fixture(core:&mut Core,id:u32)->anyhow::Result<Vec<String>> {
    gui_change(core,|project|remove_fixture(project,id))
}

/// 画面からグループを消す。そのグループを指す割り当てやエフェクトも一緒に片付ける。
/// 一緒に消したものの名前を返す
pub(crate) fn gui_delete_group(core:&mut Core,name:&str)->anyhow::Result<Vec<String>> {
    gui_change(core,|project|{
        let index=project.groups.iter().position(|g|g.name==name).context("unknown group")?;
        project.groups.remove(index);
        Ok(remove_targeting(project,&Selector::Group(name.to_string()),None))
    })
}

/// 画面からグループの名前を変える。そのグループを指す割り当てとエフェクトも新しい名前に書き換えるので、何も消えない
pub(crate) fn gui_rename_group(core:&mut Core,old:&str,new:&str)->anyhow::Result<()> {
    let new=new.trim();
    ensure!(!new.is_empty(),"group name is empty");
    gui_change(core,|project|{
        let group=project.groups.iter_mut().find(|g|g.name==old).context("unknown group")?;
        group.name=new.to_string();
        let (from,to)=(Selector::Group(old.to_string()),Selector::Group(new.to_string()));
        for sel in project.effects.iter_mut().map(|e|&mut e.sel).chain(project.pixel_fx.iter_mut().map(|e|&mut e.sel))
            .chain(project.bindings.iter_mut().filter_map(|b|action_selector(&mut b.action))) {
            if *sel==from {*sel=to.clone();}
        }
        Ok(())
    }).map(|_|())
}

/// プロジェクトのコピーを書き換え、検証してからまとめて反映する
fn gui_change<T>(core:&mut Core,change:impl FnOnce(&mut Project)->anyhow::Result<T>)->anyhow::Result<T> {
    ensure_idle(core)?;
    let mut project=core.project.clone();
    let result=change(&mut project)?;
    validate(&project,&core.lib)?;
    commit(core,project);
    Ok(result)
}

/// 割り当てが機材の選び方を持っていれば、それを返す
fn action_selector(action:&mut Action)->Option<&mut Selector> {
    match action {
        Action::SetValue{sel,..}|Action::Level{sel}|Action::Mute{sel,..}|Action::Role{sel,..}|Action::Arm{sel,..}=>Some(sel),
        _=>None,
    }
}

/// 割り当ての対象が、消すものそのものか。fixture は「選択」の割り当てが指す機材
fn targets(action:&Action,gone:&Selector,fixture:Option<u32>)->bool {
    match action {
        Action::SetValue{sel,..}|Action::Level{sel}|Action::Mute{sel,..}|Action::Role{sel,..}|Action::Arm{sel,..}=>sel==gone,
        Action::Select{fixture:id,..}=>fixture==Some(*id),
        _=>false,
    }
}

/// 機材と、その機材への参照をプロジェクトから取り除く
fn remove_fixture(project:&mut Project,id:u32)->anyhow::Result<Vec<String>> {
    let index=project.fixture_index(id).context("unknown fixture")?;
    project.fixtures.remove(index);
    for g in &mut project.groups {g.fixtures.retain(|x|*x!=id);}
    project.pairs.retain(|p|p.leader!=id && p.follower!=id);
    for s in project.scenes.iter_mut().chain(project.positions.iter_mut()) {s.values.retain(|v|v.fixture!=id);}
    Ok(remove_targeting(project,&Selector::Fixture(id),Some(id)))
}

/// 消した機材やグループだけを対象にしたエフェクトと割り当ては、残しても動かないので消す。
/// 消したエフェクトを使う割り当てとオートの演出からも外す。消したものの名前を返す
fn remove_targeting(project:&mut Project,gone:&Selector,fixture:Option<u32>)->Vec<String> {
    let mut removed=Vec::new();
    let mut effect_names=Vec::new();
    project.effects.retain(|e|{let keep=e.sel!=*gone;if !keep {effect_names.push(e.name.clone());}keep});
    project.pixel_fx.retain(|e|{let keep=e.sel!=*gone;if !keep {effect_names.push(e.name.clone());}keep});
    // 同じ名前のエフェクトが残っていれば、名前での参照はそのまま使える
    effect_names.retain(|name|!project.effects.iter().any(|e|&e.name==name) && !project.pixel_fx.iter().any(|e|&e.name==name));
    let mut seen=HashSet::new();
    effect_names.retain(|name|seen.insert(name.clone()));
    removed.extend(effect_names.iter().map(|name|format!("エフェクト「{name}」")));
    project.bindings.retain(|b|{
        let keep=!targets(&b.action,gone,fixture) && !matches!(&b.action,Action::Effect{name,..} if effect_names.contains(name));
        if !keep {removed.push(format!("割り当て「{}」",b.name));}
        keep
    });
    for preset in &mut project.auto.presets {
        for look in preset.looks.values_mut() {look.effects.retain(|e|!effect_names.contains(e));}
    }
    removed
}

pub fn safety_unchanged(old:&Project,new:&Project,lib:&Library)->anyhow::Result<()> {
    ensure!(old.motion.pan_front==new.motion.pan_front && old.motion.tilt_up==new.motion.tilt_up,"motion calibration is read-only from CLI");
    for f in &old.fixtures {
        let mode=lib.mode(&f.profile,f.mode).context("unknown profile/mode")?;
        let protected=lib.get(&f.profile).is_some_and(|p|p.needs_arm) || !f.limits.is_empty() || mode.channels.iter().any(|c|!c.forbid.is_empty());
        if protected {
            let replacement=new.fixtures.iter().find(|n|n.id==f.id).context("protected fixtures cannot be deleted through CLI")?;
            ensure!(replacement.profile==f.profile && replacement.mode==f.mode && replacement.universe==f.universe
                && replacement.address==f.address && replacement.enabled==f.enabled && replacement.limits==f.limits,
                "protected fixture patch, enable state and safety limits are read-only from CLI");
        }
    }
    Ok(())
}

pub(crate) fn commit(core:&mut Core,project:Project) {
    commit_with_arm(core,project,true);
}

pub(crate) fn commit_with_arm(core:&mut Core,mut project:Project,keep_arm:bool) {
    project.ensure_binding_ids();
    for f in &mut project.fixtures {
        f.armed=keep_arm && core.project.fixtures.iter().find(|old|old.id==f.id && old.profile==f.profile && old.mode==f.mode).is_some_and(|old|old.armed);
    }
    let outputs_changed=project.outputs!=core.project.outputs;
    let bindings_changed=serde_json::to_value(&project.bindings).ok()!=serde_json::to_value(&core.project.bindings).ok();
    let bpm_changed=project.bpm!=core.project.bpm;
    core.rt.fades.retain(|fade| {
        let before=core.project.fixtures.iter().find(|f|f.id==fade.fixture);
        let after=project.fixtures.iter().find(|f|f.id==fade.fixture);
        match (before,after) {(Some(a),Some(b))=>a.profile==b.profile && a.mode==b.mode && a.values==b.values,_=>false}
    });
    core.rt.chase_idx.retain(|name,_| {
        let before=core.project.chases.iter().find(|c|&c.name==name);
        let after=project.chases.iter().find(|c|&c.name==name && c.enabled);
        matches!((before,after),(Some(a),Some(b)) if serde_json::to_value(a).ok()==serde_json::to_value(b).ok())
    });
    core.release_remote_controls();
    core.project=project;
    if bindings_changed {core.rt.pad_state.clear();core.rt.toggle_state.clear();}
    if !keep_arm {core.rt.fades.clear();core.rt.chase_idx.clear();core.rt.pad_state.clear();core.rt.toggle_state.clear();}
    if core.rt.last_scene.as_ref().is_some_and(|name|!core.project.scenes.iter().any(|s|&s.name==name)) {core.rt.last_scene=None;}
    if core.rt.last_position.as_ref().is_some_and(|name|!core.project.positions.iter().any(|s|&s.name==name)) {core.rt.last_position=None;}
    let ids:HashSet<_>=core.project.fixtures.iter().map(|f|f.id).collect();
    core.rt.selected.retain(|id|ids.contains(id));core.rt.muted.retain(|id|ids.contains(id));
    core.rt.level.retain(|id,_|ids.contains(id));core.rt.motion_pan.retain(|id,_|ids.contains(id));
    if outputs_changed {core.apply_outputs_changed();}
    core.dirty=true;
    if bpm_changed {core.set_bpm(core.project.bpm);}
}

pub fn validate(project:&Project,lib:&Library)->anyhow::Result<()> {
    crate::backend::commands::validate(project,lib)?;
    ensure!((BPM_MIN..=BPM_MAX).contains(&project.bpm),"BPM is out of range");
    ensure!(project.osc_port>0 && project.web_port>0,"ports must be nonzero");
    ensure!((0.0..=120.0).contains(&project.fade_secs) && (0.0..=1.0).contains(&project.osc_level_depth)
        && project.osc_rgb_secs.is_finite() && project.osc_rgb_secs>=0.0,"invalid fade/OSC settings");
    let fixtures:HashSet<_>=project.fixtures.iter().map(|f|f.id).collect();
    let groups=unique(project.groups.iter().map(|g|g.name.as_str()),"group")?;
    let scenes=unique(project.scenes.iter().map(|s|s.name.as_str()),"scene")?;
    let positions=unique(project.positions.iter().map(|s|s.name.as_str()),"position")?;
    let chases=unique(project.chases.iter().map(|s|s.name.as_str()),"chase")?;
    let pairs=unique(project.pairs.iter().map(|s|s.name.as_str()),"pair")?;
    unique(project.auto.presets.iter().map(|s|s.name.as_str()),"auto preset")?;
    let effects:HashSet<_>=project.effects.iter().map(|s|s.name.as_str()).chain(project.pixel_fx.iter().map(|s|s.name.as_str())).collect();
    let selector=|sel:&Selector|->anyhow::Result<()> {
        ensure!(match sel {Selector::All=>true,Selector::Fixture(id)=>fixtures.contains(id),Selector::Group(name)=>groups.contains(name.as_str()),Selector::Profile(id)=>lib.get(id).is_some()},"dangling selector: {sel:?}");Ok(())
    };
    for f in &project.fixtures {ensure!(!f.name.trim().is_empty(),"fixture name is empty");}
    for group in &project.groups {ensure!(group.fixtures.iter().all(|id|fixtures.contains(id)),"group references unknown fixture");}
    for scene in project.scenes.iter().chain(&project.positions) {
        ensure!((0.0..=120.0).contains(&scene.fade),"invalid scene fade");
        for v in &scene.values {
            let f=project.fixtures.iter().find(|f|f.id==v.fixture).context("scene references unknown fixture")?;
            ensure!(!lib.mode(&f.profile,f.mode).unwrap().attr_channels(&v.attr).is_empty() && (0.0..256.0).contains(&v.value),"invalid scene attribute/value");
        }
    }
    for effect in &project.effects {
        selector(&effect.sel)?;ensure!(!effect.name.trim().is_empty() && !effect.attr.trim().is_empty(),"effect name/attr is empty");
        ensure!((0.0..=255.0).contains(&effect.min) && (0.0..=255.0).contains(&effect.max)
            && effect.rate_hz.is_finite() && effect.rate_hz>=0.0 && effect.beats.is_finite() && effect.beats>=0.0
            && effect.phase.is_finite() && effect.spread.is_finite(),"invalid effect parameters");
    }
    for effect in &project.pixel_fx {selector(&effect.sel)?;ensure!(!effect.name.trim().is_empty() && effect.beats.is_finite() && effect.beats>=0.0 && effect.width.is_finite() && effect.width>=0.0 && effect.skew.is_finite(),"invalid pixel effect");}
    for chase in &project.chases {ensure!(chase.steps.iter().all(|s|scenes.contains(s.as_str())) && chase.step_beats.is_finite() && chase.step_beats>0.0 && chase.fade_beats.is_finite() && chase.fade_beats>=0.0,"invalid chase or dangling scene reference");}
    let mut followers=HashSet::new();let mut links=BTreeMap::new();
    for pair in &project.pairs {
        ensure!(fixtures.contains(&pair.leader) && fixtures.contains(&pair.follower) && pair.leader!=pair.follower,"invalid pair endpoints");
        if pair.enabled {ensure!(followers.insert(pair.follower),"multiple active leaders for one follower");links.insert(pair.follower,pair.leader);}
    }
    for &start in links.keys() {let mut seen=HashSet::new();let mut at=start;while let Some(&leader)=links.get(&at) {ensure!(seen.insert(at),"pair cycle");at=leader;}}
    for binding in &project.bindings {
        ensure!(!binding.name.trim().is_empty(),"binding name is empty");
        ensure!(binding.page.is_none_or(|p|p<PAGES),"binding page is out of range");
        if let Some(t)=binding.trigger {ensure!(t.channel.is_none_or(|c|c<16) && (t.kind==TriggerKind::Key || t.number<128),"invalid MIDI trigger");}
        if let Some(osc)=&binding.osc {ensure!(osc.starts_with('/'),"invalid OSC binding address");}
        match &binding.action {
            Action::SetValue{sel,attr,on,off,..}=>{selector(sel)?;ensure!(!attr.is_empty() && (0.0..256.0).contains(on) && (0.0..256.0).contains(off),"invalid value binding");}
            Action::Level{sel}|Action::Mute{sel,..}|Action::Role{sel,..}|Action::Arm{sel,..}=>selector(sel)?,
            Action::Scene{name}=>ensure!(scenes.contains(name.as_str()),"binding references unknown scene"),
            Action::Position{name}=>ensure!(positions.contains(name.as_str()),"binding references unknown position"),
            Action::Effect{name,..}=>ensure!(effects.contains(name.as_str()),"binding references unknown effect"),
            Action::Chase{name,..}=>ensure!(chases.contains(name.as_str()),"binding references unknown chase"),
            Action::Pair{name,..}=>ensure!(pairs.contains(name.as_str()),"binding references unknown pair"),
            Action::Select{fixture,..}=>ensure!(fixtures.contains(fixture),"binding references unknown fixture"),
            Action::Master{on,off,..}=>ensure!((0.0..=100.0).contains(on) && (0.0..=100.0).contains(off),"invalid master binding"),
            Action::Fade{secs,..}=>ensure!((0.0..=120.0).contains(secs),"invalid fade binding"),
            _=>{},
        }
    }
    ensure!(!project.auto.presets.is_empty() && project.auto.preset<project.auto.presets.len(),"invalid auto preset index");
    let auto=&project.auto;
    ensure!((0.0..=1.0).contains(&auto.idle_level) && auto.idle_rgb.iter().all(|v|(0.0..=1.0).contains(v))
        && auto.timeout_secs.is_finite() && auto.timeout_secs>0.0 && auto.manual_secs.is_finite() && auto.manual_secs>=0.0
        && auto.alert_secs.is_finite() && auto.alert_secs>=0.0,"invalid auto parameters");
    for preset in &auto.presets {for look in preset.looks.values() {
        ensure!(look.scene.is_empty() || scenes.contains(look.scene.as_str()),"look references unknown scene");
        ensure!(look.effects.iter().all(|e|effects.contains(e.as_str()) || chases.contains(e.as_str())),"look references unknown effect/chase");
        ensure!(look.beats.is_finite() && look.beats>0.0 && look.color_beats.is_finite() && look.color_beats>0.0
            && (0.0..=1.0).contains(&look.brightness) && look.position.is_none_or(|p|p<4),"invalid phrase look");
    }}
    for output in &project.outputs {
        ensure!(output.universe<NUM_UNIVERSES,"output universe is out of range");
        match &output.kind {
            OutputKind::ArtNet{ip,art_universe}=>{let _:std::net::Ipv4Addr=ip.parse().context("invalid Art-Net IPv4 address")?;ensure!(*art_universe<32768,"invalid Art-Net universe");}
            OutputKind::OpenDmxSerial{port}=>ensure!(!port.trim().is_empty(),"empty serial port"),_=>{},
        }
    }
    let motion=&project.motion;
    ensure!(motion.beats.is_finite() && motion.beats>0.0 && (0.0..=1.0).contains(&motion.size)
        && (0.0..=1.0).contains(&motion.glide) && motion.position.is_none_or(|p|p<4)
        && motion.points.iter().flatten().all(|v|v.is_finite() && v.abs()<=90.0),"invalid motion parameters");
    Ok(())
}

fn unique<'a>(names:impl Iterator<Item=&'a str>,kind:&str)->anyhow::Result<HashSet<&'a str>> {
    let mut set=HashSet::new();for name in names {ensure!(!name.trim().is_empty() && set.insert(name),"empty or duplicate {kind} name");}Ok(set)
}

pub use crate::core::recorder::redact;

impl crate::backend::host::Host {
    /// プロジェクトのMIDI機器(midi_input)に、つないでいる機器を合わせる
    pub(crate) fn sync_midi_choice(&mut self)->anyhow::Result<()> {
        let name=crate::core::engine::lock(&self.core).project.midi_input.clone();
        self.midi_synced+=1;
        if !self.launch.inputs {return Ok(());}
        if self.midi.connected.as_deref()==Some(name.as_str()) {return Ok(());}
        self.midi.disconnect();
        if !name.is_empty() {self.midi.connect(&self.core,self.ctx.clone(),&name).map_err(anyhow::Error::msg)?;}
        Ok(())
    }

    pub(crate) fn project_diff(&self)->anyhow::Result<Value> {
        let text=std::fs::read_to_string(&self.project_path)?;
        let mut saved:Project=serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
        saved.ensure_binding_ids();
        let current=crate::core::engine::lock(&self.core).project.clone();
        let pin_changed=saved.web_pin!=current.web_pin;
        let saved=redact(&serde_json::to_value(saved)?);
        let current=redact(&serde_json::to_value(&current)?);
        let mut changes=Vec::new();
        differences("",&saved,&current,&mut changes);
        Ok(json!({"changes":changes,"redacted_changes":{"web_pin":pin_changed},"project":self.project_path}))
    }

    pub(crate) fn reload_checked(&mut self,path:Option<&std::path::Path>,remote:bool,dry_run:bool)->anyhow::Result<Value> {
        let path=path.unwrap_or(&self.project_path).to_path_buf();
        let text=std::fs::read_to_string(&path)?;
        self.reload_text(&text,&path.display().to_string(),remote,dry_run)
    }

    /// プロジェクトの中身(JSON)を読み込む。保存先(project_path)は変えない。読み込む前の状態はバックアップに残す。
    /// `source` は結果に出す読み込み元(ファイルの場所や名前)
    pub(crate) fn reload_text(&mut self,text:&str,source:&str,remote:bool,dry_run:bool)->anyhow::Result<Value> {
        let path=source;
        let mut project:Project=serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
        let previous={
            let core=crate::core::engine::lock(&self.core);
            ensure_idle(&core)?;
            project.next_binding_id=project.next_binding_id.max(core.project.next_binding_id);
            project.next_id=project.next_id.max(core.project.next_id);
            project.ensure_binding_ids();
            // GUIの「読み込み」は今までどおり厳しい検証をしない(GUIで保存したファイルを読み戻せるように)
            if remote {safety_unchanged(&core.project,&project,&core.lib)?;validate(&project,&core.lib)?;}
            core.project.clone()
        };
        if dry_run {return Ok(json!({"applied":false,"valid":true,"source":path}));}
        let dir=self.project_path.parent().unwrap_or(std::path::Path::new(".")).join("backups");
        std::fs::create_dir_all(&dir)?;
        let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        let backup=dir.join(format!("before-load-{stamp}-{}.json",std::process::id()));
        let file=std::fs::OpenOptions::new().write(true).create_new(true).open(&backup)?;
        let mut writer=std::io::BufWriter::new(file);
        serde_json::to_writer_pretty(&mut writer,&previous)?;
        std::io::Write::flush(&mut writer)?;
        let old_midi=previous.midi_input;
        let new_midi=project.midi_input.clone();
        {
            let mut core=crate::core::engine::lock(&self.core);
            ensure_idle(&core)?;
            commit_with_arm(&mut core,project,false);
        }
        let midi_error=if old_midi!=new_midi {self.sync_midi_choice().err().map(|e|e.to_string())}else{None};
        self.apply_services();
        self.reloads+=1;
        Ok(json!({"applied":true,"source":path,"backup":backup,"project":self.project_path,"midi_error":midi_error}))
    }
}

fn differences(path:&str,before:&Value,after:&Value,out:&mut Vec<Value>) {
    if before==after {return;}
    if let (Some(a),Some(b))=(before.as_object(),after.as_object()) {
        let keys:std::collections::BTreeSet<_>=a.keys().chain(b.keys()).collect();
        for key in keys {differences(&format!("{path}/{}",key.replace('~',"~0").replace('/',"~1")),a.get(key).unwrap_or(&Value::Null),b.get(key).unwrap_or(&Value::Null),out);}
    }else if let (Some(a),Some(b))=(before.as_array(),after.as_array()) {
        if a.len()==b.len() {for (i,(a,b)) in a.iter().zip(b).enumerate(){differences(&format!("{path}/{i}"),a,b,out);}}
        else {out.push(json!({"path":path,"before":before,"after":after}));}
    }else {out.push(json!({"path":path,"before":before,"after":after}));}
}

fn guard_signature(profile:&crate::core::fixture::Profile)->Value {
    json!({"needs_arm":profile.needs_arm,"modes":profile.modes.iter().map(|mode|json!({
        "name":mode.name,"channels":mode.channels.iter().map(|ch|json!({
            "attr":ch.attr,"fine_of":ch.fine_of,"safe":ch.safe,"forbid":ch.forbid
        })).collect::<Vec<_>>()
    })).collect::<Vec<_>>()})
}

fn guarded(profile:&crate::core::fixture::Profile)->bool {
    profile.needs_arm || profile.modes.iter().flat_map(|m|&m.channels).any(|ch|ch.safe.is_some() || !ch.forbid.is_empty())
}

fn profile_schema_compatible(project:&Project,old:&Library,new:&Library,id:&str)->anyhow::Result<()> {
    for fixture in project.fixtures.iter().filter(|f|f.profile==id) {
        let old=old.mode(id,fixture.mode).context("unknown existing fixture mode")?;
        let new=new.mode(id,fixture.mode).context("unpatch and save before removing an in-use profile/mode")?;
        let schema=|mode:&crate::core::fixture::Mode|json!(mode.channels.iter().map(|c|(&c.attr,c.fine_of)).collect::<Vec<_>>());
        ensure!(schema(old)==schema(new),"unpatch the fixture and save the project before changing its profile channel structure");
    }
    Ok(())
}

fn saved_profile_compatible(core:&Core,candidate:&Library,id:&str)->anyhow::Result<()> {
    profile_schema_compatible(&core.project,&core.lib,candidate,id)?;
    if let Some(path)=&core.rt.project_path {
        let text=std::fs::read_to_string(path)?;
        let saved:Project=serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
        profile_schema_compatible(&saved,&core.lib,candidate,id)?;
        crate::backend::commands::validate(&saved,candidate)?;
    }
    Ok(())
}

pub trait CoreEditing {
    fn save_profile(&mut self,profile:&crate::core::fixture::Profile,remote:bool,dry_run:bool)->anyhow::Result<Value>;
    fn delete_profile(&mut self,id:&str,remote:bool,dry_run:bool)->anyhow::Result<Value>;
}

impl CoreEditing for crate::core::engine::Core {
    fn save_profile(&mut self,profile:&crate::core::fixture::Profile,remote:bool,dry_run:bool)->anyhow::Result<Value> {
        ensure_idle(self)?;
        ensure!(!profile.id.trim().is_empty(),"profile ID is empty");
        let report=profile.validate();ensure!(report.errors.is_empty(),"invalid profile: {}",report.errors.join("; "));
        if let Some(old)=self.lib.get(&profile.id) {
            if remote && (guarded(old) || guarded(profile)) {ensure!(guard_signature(old)==guard_signature(profile),"profile safety definitions are read-only from CLI");}
        }
        let candidate=std::sync::Arc::new(self.lib.preview_user(profile));
        saved_profile_compatible(self,&candidate,&profile.id)?;
        let mut trial=Core::new(self.project.clone(),self.lib.clone());trial.replace_library(candidate.clone());
        validate(&trial.project,&candidate)?;
        if dry_run {return Ok(json!({"saved":false,"valid":true,"warnings":report.warnings}));}
        let path=self.lib.user_dir.join(format!("{}.json",crate::core::fixture::slug(&profile.id)));
        if path.exists() && !self.lib.has_user_file(&profile.id) {
            let existing:crate::core::fixture::Profile=serde_json::from_str(&std::fs::read_to_string(&path)?)?;
            ensure!(existing.id==profile.id,"profile filename collides with another ID");
        }
        let path=self.lib.save_user(profile).map_err(anyhow::Error::msg)?;
        let mut library=(*candidate).clone();
        library.register_user_file(&profile.id,path.clone());
        self.replace_library(std::sync::Arc::new(library));
        Ok(json!({"saved":true,"path":path,"warnings":report.warnings}))
    }

    fn delete_profile(&mut self,id:&str,remote:bool,dry_run:bool)->anyhow::Result<Value> {
        ensure_idle(self)?;
        let old=self.lib.get(id).context("unknown profile")?;
        ensure!(self.lib.has_user_file(id),"profile has no user override to delete");
        let candidate=std::sync::Arc::new(self.lib.preview_delete(id));
        saved_profile_compatible(self,&candidate,id)?;
        if remote && guarded(old) {
            let fallback=candidate.get(id).context("protected profiles cannot be deleted from CLI")?;
            ensure!(guard_signature(old)==guard_signature(fallback),"deleting this override would change safety definitions");
        }
        let mut trial=Core::new(self.project.clone(),self.lib.clone());trial.replace_library(candidate.clone());
        validate(&trial.project,&candidate)?;
        if !dry_run {self.lib.delete_user(id).map_err(anyhow::Error::msg)?;self.replace_library(candidate);}
        Ok(json!({"deleted":!dry_run,"id":id,"builtin_fallback":self.lib.is_builtin(id)}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{path::Path,sync::Arc};
    fn core()->Core {let (lib,_)=Library::load(Path::new("none"));Core::new(Project::starter(&lib),Arc::new(lib))}
    fn state(c:&Core)->Value {serde_json::to_value(&c.project).unwrap()}
    #[test]
    fn invalid_batch_changes_nothing_and_dry_run_does_not_allocate_ids() {
        let mut c=core();let before=state(&c);
        assert!(apply(&mut c,&json!([
            {"entity":"fixture","op":"create","data":{"profile":"stage-evolution/minispot30"}},
            {"entity":"group","op":"create","data":{"name":"bad","fixtures":[999]}}
        ]),false).is_err());
        assert_eq!(state(&c),before);
        apply(&mut c,&json!([{"entity":"fixture","op":"create","data":{"profile":"stage-evolution/minispot30"}}]),true).unwrap();
        assert_eq!(state(&c),before);
    }
    #[test]
    fn all_project_entities_can_be_created_in_one_checked_batch() {
        let mut c=core();
        apply(&mut c,&json!([
            {"entity":"fixture","op":"create","data":{"name":"Third","profile":"stage-evolution/minispot30","address":30}},
            {"entity":"group","op":"create","data":{"name":"Spots","fixtures":[1,3]}},
            {"entity":"effect","op":"create","data":{"name":"Pulse","sel":{"Group":"Spots"},"attr":"dimmer","beats":2}},
            {"entity":"pixel","op":"create","data":{"name":"Pixels","sel":{"Group":"Spots"}}},
            {"entity":"scene","op":"create","data":{"name":"Intro","values":[{"fixture":1,"attr":"dimmer","value":180}]}},
            {"entity":"position","op":"capture","data":{"name":"Up","fixtures":[1]}},
            {"entity":"chase","op":"create","data":{"name":"Chase","steps":["Intro"]}},
            {"entity":"pair","op":"create","data":{"name":"Pair","leader":1,"follower":3}},
            {"entity":"binding","op":"create","data":{"name":"Intro button","action":{"Scene":{"name":"Intro"}}}},
            {"entity":"output","op":"create","data":{"universe":0}},
            {"entity":"auto_preset","op":"create","data":{"name":"Custom"}},
            {"entity":"settings","op":"update","data":{"bpm":128,"auto":{"preset":2}}}
        ]),false).unwrap();
        assert_eq!(c.project.fixtures.len(),3);assert_eq!(c.project.groups.len(),1);
        assert_eq!(c.project.bindings.len(),2);assert_eq!(c.project.bpm,128.0);
        assert!(c.project.bindings.iter().all(|b|b.id>0));
    }
    #[test]
    fn intermediate_overlap_is_allowed_but_final_overlap_is_rejected() {
        let mut c=core();
        apply(&mut c,&json!([{"entity":"fixture","op":"create","data":{"profile":"stage-evolution/minispot30","address":30}}]),false).unwrap();
        let before=state(&c);
        assert!(apply(&mut c,&json!([{"entity":"fixture","op":"update","key":1,"data":{"address":30}}]),false).is_err());
        assert_eq!(state(&c),before);
        apply(&mut c,&json!([
            {"entity":"fixture","op":"update","key":1,"data":{"address":30}},
            {"entity":"fixture","op":"update","key":3,"data":{"address":1}}
        ]),false).unwrap();assert_eq!(c.project.fixtures[0].address,30);
    }
    #[test]
    fn deleting_referenced_entities_requires_fixing_the_references() {
        let mut c=core();
        apply(&mut c,&json!([{"entity":"group","op":"create","data":{"name":"G","fixtures":[1]}}]),false).unwrap();
        assert!(apply(&mut c,&json!([{"entity":"fixture","op":"delete","key":1}]),false).is_err());
        apply(&mut c,&json!([{"entity":"group","op":"delete","key":"G"},{"entity":"fixture","op":"delete","key":1}]),false).unwrap();
        assert_eq!(c.project.fixtures.len(),1);
    }
    #[test]
    fn gui_fixture_delete_clears_references_so_later_edits_still_work() {
        let mut c=core();
        apply(&mut c,&json!([
            {"entity":"group","op":"create","data":{"name":"G","fixtures":[1,2]}},
            {"entity":"effect","op":"create","data":{"name":"Solo","sel":{"Fixture":1},"attr":"dimmer"}},
            {"entity":"effect","op":"create","data":{"name":"Shared","sel":{"Fixture":1},"attr":"dimmer"}},
            {"entity":"effect","op":"create","data":{"name":"Shared","sel":{"Fixture":2},"attr":"dimmer"}},
            {"entity":"pixel","op":"create","data":{"name":"Px","sel":{"Fixture":1}}},
            {"entity":"binding","op":"create","data":{"name":"Dim","action":{"SetValue":{"sel":{"Fixture":1},"attr":"dimmer","mode":"Trigger","on":255,"off":0}}}},
            {"entity":"binding","op":"create","data":{"name":"Pick","action":{"Select":{"fixture":1,"op":"Toggle"}}}},
            {"entity":"binding","op":"create","data":{"name":"SoloFx","action":{"Effect":{"name":"Solo","mode":"Toggle"}}}},
            {"entity":"binding","op":"create","data":{"name":"SharedFx","action":{"Effect":{"name":"Shared","mode":"Toggle"}}}},
            {"entity":"binding","op":"create","data":{"name":"GroupDim","action":{"Level":{"sel":{"Group":"G"}}}}}
        ]),false).unwrap();
        let mut look=c.project.auto.presets[0].looks.values().next().unwrap().clone();
        look.effects=vec!["Solo".into(),"Shared".into()];
        let key=c.project.auto.presets[0].looks.keys().next().unwrap().clone();
        c.project.auto.presets[0].looks.insert(key.clone(),look);
        let before=c.project.bindings.len();

        let removed=gui_delete_fixture(&mut c,1).unwrap();
        assert!(c.project.fixture_index(1).is_none());
        assert_eq!(c.project.groups[0].fixtures,vec![2]);
        assert!(c.project.effects.iter().all(|e|e.name=="Shared" && e.sel==Selector::Fixture(2)));
        assert!(c.project.pixel_fx.is_empty());
        let names:Vec<_>=c.project.bindings.iter().map(|b|b.name.as_str()).collect();
        for gone in ["Dim","Pick","SoloFx"] {assert!(!names.contains(&gone),"{gone} should be removed");}
        for kept in ["SharedFx","GroupDim"] {assert!(names.contains(&kept),"{kept} should be kept");}
        assert_eq!(c.project.bindings.len(),before-3);
        assert_eq!(c.project.auto.presets[0].looks[&key].effects,vec!["Shared".to_string()]);
        assert!(removed.iter().any(|r|r.contains("Solo")) && removed.iter().any(|r|r.contains("Dim")));
        apply(&mut c,&json!([{"entity":"group","op":"create","data":{"name":"After"}}]),false).unwrap();
    }
    #[test]
    fn starter_fixture_can_be_deleted_from_the_patch_tab() {
        // 配布している初期プロジェクトには、機材1を指す割り当て「パッド56」が入っている
        let (lib,_)=Library::load(Path::new("none"));
        let project:Project=serde_json::from_str(include_str!("../../config/project.json")).unwrap();
        let mut c=Core::new(project,Arc::new(lib));
        assert!(c.project.bindings.iter().any(|b|targets(&b.action,&Selector::Fixture(1),Some(1))));
        let removed=gui_delete_fixture(&mut c,1).unwrap();
        assert!(removed.iter().any(|r|r.contains("パッド56")));
        crate::backend::editing::gui_apply(&mut c,&json!([{"entity":"fixture","op":"create","data":{"profile":"stage-evolution/minispot30"}}])).unwrap();
        crate::backend::editing::gui_apply(&mut c,&json!([{"entity":"group","op":"create","data":{"name":"G"}}])).unwrap();
    }
    #[test]
    fn gui_group_delete_clears_references_so_later_edits_still_work() {
        let mut c=core();
        gui_apply(&mut c,&json!([
            {"entity":"group","op":"create","data":{"name":"G","fixtures":[1]}},
            {"entity":"group","op":"create","data":{"name":"Keep","fixtures":[2]}},
            {"entity":"effect","op":"create","data":{"name":"GFx","sel":{"Group":"G"},"attr":"dimmer"}},
            {"entity":"pixel","op":"create","data":{"name":"GPx","sel":{"Group":"G"}}},
            {"entity":"binding","op":"create","data":{"name":"GLevel","action":{"Level":{"sel":{"Group":"G"}}}}},
            {"entity":"binding","op":"create","data":{"name":"GFxPad","action":{"Effect":{"name":"GFx","mode":"Toggle"}}}},
            {"entity":"binding","op":"create","data":{"name":"KeepLevel","action":{"Level":{"sel":{"Group":"Keep"}}}}}
        ])).unwrap();
        let removed=gui_delete_group(&mut c,"G").unwrap();
        assert_eq!(c.project.groups.iter().map(|g|g.name.as_str()).collect::<Vec<_>>(),vec!["Keep"]);
        assert!(c.project.effects.is_empty() && c.project.pixel_fx.is_empty());
        let names:Vec<_>=c.project.bindings.iter().map(|b|b.name.as_str()).collect();
        assert!(names.contains(&"KeepLevel") && !names.contains(&"GLevel") && !names.contains(&"GFxPad"));
        assert!(removed.iter().any(|r|r.contains("GFx")) && removed.iter().any(|r|r.contains("GLevel")));
        assert_eq!(c.project.fixtures.len(),2,"deleting a group must not delete its fixtures");
        gui_apply(&mut c,&json!([{"entity":"group","op":"create","data":{"name":"After"}}])).unwrap();
    }
    #[test]
    fn gui_group_rename_rewrites_references_and_rejects_bad_names() {
        let mut c=core();
        gui_apply(&mut c,&json!([
            {"entity":"group","op":"create","data":{"name":"G","fixtures":[1]}},
            {"entity":"group","op":"create","data":{"name":"Other"}},
            {"entity":"effect","op":"create","data":{"name":"E","sel":{"Group":"G"},"attr":"dimmer"}},
            {"entity":"pixel","op":"create","data":{"name":"P","sel":{"Group":"G"}}},
            {"entity":"binding","op":"create","data":{"name":"B","action":{"Mute":{"sel":{"Group":"G"},"mode":"Toggle"}}}}
        ])).unwrap();
        let before=state(&c);
        assert!(gui_rename_group(&mut c,"G","Other").is_err());
        assert!(gui_rename_group(&mut c,"G","  ").is_err());
        assert_eq!(state(&c),before);
        gui_rename_group(&mut c,"G"," G2 ").unwrap();
        let to=Selector::Group("G2".into());
        assert_eq!(c.project.groups[0].name,"G2");
        assert_eq!(c.project.effects[0].sel,to);assert_eq!(c.project.pixel_fx[0].sel,to);
        let b=c.project.bindings.iter_mut().find(|b|b.name=="B").unwrap();
        assert_eq!(action_selector(&mut b.action).cloned(),Some(to));
        gui_apply(&mut c,&json!([{"entity":"fixture","op":"create","data":{"profile":"stage-evolution/minispot30"}}])).unwrap();
    }
    #[test]
    fn safety_and_binding_identity_cannot_be_replaced_by_edits() {
        let mut c=core();c.project.fixtures[1].armed=true;let before=state(&c);
        for operations in [
            json!([{"entity":"fixture","op":"delete","key":2}]),
            json!([{"entity":"fixture","op":"update","key":2,"data":{"profile":"stage-evolution/minispot30"}}]),
            json!([{"entity":"fixture","op":"update","key":2,"data":{"limits":{}}}]),
            json!([{"entity":"binding","op":"update","index":0,"data":{"id":999}}]),
            json!([{"entity":"settings","op":"update","data":{"motion":{"pan_front":0.3}}}]),
        ] {assert!(apply(&mut c,&operations,false).is_err());assert_eq!(state(&c),before);}
        apply(&mut c,&json!([{"entity":"group","op":"create","data":{"name":"G"}}]),false).unwrap();
        assert!(c.project.fixtures[1].armed,"unrelated edits must preserve GUI ARM state");
    }
    #[test]
    fn pair_cycles_and_ambiguous_effect_updates_are_rejected() {
        let mut c=core();
        assert!(apply(&mut c,&json!([
            {"entity":"pair","op":"create","data":{"name":"A","leader":1,"follower":2}},
            {"entity":"pair","op":"create","data":{"name":"B","leader":2,"follower":1}}
        ]),false).is_err());
        apply(&mut c,&json!([
            {"entity":"effect","op":"create","data":{"name":"Circle","attr":"pan"}},
            {"entity":"effect","op":"create","data":{"name":"Circle","attr":"tilt"}}
        ]),false).unwrap();
        assert!(apply(&mut c,&json!([{"entity":"effect","op":"update","key":"Circle","data":{"beats":2}}]),false).is_err());
        apply(&mut c,&json!([{"entity":"effect","op":"update","index":1,"data":{"beats":2}}]),false).unwrap();
        assert_eq!(c.project.effects[1].beats,2.0);
    }
    #[test]
    fn active_holds_block_edits_without_changing_their_lifetime() {
        let mut c=core();
        c.project.bindings.push(Binding{id:0,name:"Flash".into(),trigger:None,page:None,osc:None,action:Action::Flash{white:true,mode:PadMode::Momentary}});
        c.project.ensure_binding_ids();let id=c.project.bindings.last().unwrap().id;
        c.remote_button(&json!({"control":format!("binding:{id}"),"event":"press"})).unwrap();
        assert!(apply(&mut c,&json!([{"entity":"group","op":"create","data":{"name":"G"}}]),false).is_err());
        assert_eq!(c.rt.flash,Some(true));
        c.remote_button(&json!({"control":format!("binding:{id}"),"event":"release"})).unwrap();
        apply(&mut c,&json!([{"entity":"group","op":"create","data":{"name":"G"}}]),false).unwrap();
    }
    #[test]
    fn unrelated_edits_keep_running_fades_but_explicit_values_override_them() {
        let mut c=core();
        c.project.scenes.push(Scene{name:"Fade".into(),fade:5.0,values:vec![SceneValue{fixture:1,attr:"dimmer".into(),value:180.0}]});
        c.recall_scene("Fade");assert!(!c.rt.fades.is_empty());let start=c.rt.fades[0].start;
        apply(&mut c,&json!([{"entity":"group","op":"create","data":{"name":"G"}}]),false).unwrap();
        assert_eq!(c.rt.fades[0].start,start);
        let mut values=c.project.fixtures[0].values.clone();values[5]=64;
        apply(&mut c,&json!([{"entity":"fixture","op":"update","key":1,"data":{"values":values}}]),false).unwrap();
        assert!(c.rt.fades.is_empty());
    }
}
