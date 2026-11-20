//! 操作の列を検証して反映する。
use super::*;

pub fn apply(core:&mut Core,operations:&Value,dry_run:bool)->anyhow::Result<Value> {
    apply_with_policy(core,operations,dry_run,true)
}

pub fn gui_apply(core:&mut Core,operations:&Value)->anyhow::Result<Value> {
    apply_with_policy(core,operations,false,false)
}

pub(super) fn apply_with_policy(core:&mut Core,operations:&Value,dry_run:bool,remote:bool)->anyhow::Result<Value> {
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
                let index=select(array(&data,field)?,operation,true)?;
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
                let index=select(array(&data,field)?,operation,true)?;array_mut(&mut data,field)?.remove(index);index
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
