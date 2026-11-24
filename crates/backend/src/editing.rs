//! Project edits are built on a copy, checked as a whole, then committed once.
use crate::controls::CoreControls;
use dmxctl_core::{engine::Core, project::*, fixture::Library};
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

pub(crate) fn get(core: &Core, entity: &str, selection: &Value) -> anyhow::Result<Value> {
    let field = collection(entity)?;
    let project = serde_json::to_value(&core.project)?;
    let items = array(&project, field)?;
    let index = select(items, selection, entity)?;
    Ok(json!({"index": index, "data": redact(&items[index])}))
}

fn array<'a>(project:&'a Value,field:&str)->anyhow::Result<&'a Vec<Value>> {
    let value=if field=="auto.presets" {&project["auto"]["presets"]}else{&project[field]};
    value.as_array().context("collection is not an array")
}
fn array_mut<'a>(project:&'a mut Value,field:&str)->anyhow::Result<&'a mut Vec<Value>> {
    let value=if field=="auto.presets" {&mut project["auto"]["presets"]}else{&mut project[field]};
    value.as_array_mut().context("collection is not an array")
}

/// 一覧の中から、`index` か `key`(ID・名前)で1つ選ぶ。`entity` はエラーの文に使う
pub(crate) fn select(items:&[Value],operation:&Value,entity:&str)->anyhow::Result<usize> {
    if let Some(index)=operation.get("index") {
        let index=usize::try_from(index.as_u64().context("index must be an integer")?)?;
        ensure!(index<items.len(),"index is out of range"); return Ok(index);
    }
    let key=operation.get("key").context("key or index is required")?;
    let by=|matches:&dyn Fn(&Value)->bool|->Vec<usize> {items.iter().enumerate().filter(|(_,item)|matches(item)).map(|(i,_)|i).collect()};
    let hits=if let Some(id)=key.as_u64() {by(&|item|item["id"]==id)}
    else if let Some(text)=key.as_str() {
        let binding_id=(entity=="binding").then(||text.strip_prefix("binding:").and_then(|s|s.parse::<u64>().ok())).flatten();
        if let Some(id)=binding_id {by(&|item|item.get("id").is_some() && item["id"]==id)}
        else {
            // 数字だけの文字列は、ID を持つ一覧(機材など)では ID として探す。CLI の位置引数と同じ
            let by_id=text.parse::<u64>().ok().map(|id|by(&|item|item.get("id").is_some() && item["id"]==id)).unwrap_or_default();
            if by_id.is_empty() {by(&|item|item["name"]==text)} else {by_id}
        }
    } else {bail!("key must be a number or a string")};
    let entity=if entity.is_empty() {"item"} else {entity};
    ensure!(!hits.is_empty(),"unknown {entity}: {key}");
    ensure!(hits.len()==1,"key matches more than one {entity}: {key} (use index)");
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
        "pixel"=>serde_json::to_value(dmxctl_core::pixel::PixelFx::default())?,
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
        let end=start.saturating_add(mode.channels.len()).min(512);
        if let Some(slots)=used.get_mut(start..end) {slots.fill(true);}
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
        if pressed && core.project.bindings.get(i).is_some_and(|b|!crate::controls::is_slider(&b.action)) {
            bail!("release held GUI/MIDI controls before editing");
        }
    }
    Ok(())
}

mod apply;
mod gui;
mod host;
mod profile;
#[cfg(test)]
mod tests;
mod validate;

pub use apply::*;
pub use gui::*;
pub use profile::*;
pub use validate::*;
