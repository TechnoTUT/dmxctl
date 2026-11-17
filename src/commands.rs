//! Validated operations shared by offline runs and the running application.
use crate::{engine::Core, fixture::Library, project::{Project, Selector}};
use anyhow::{Context, bail, ensure};
use serde_json::Value;

fn finite(value: &Value) -> anyhow::Result<f32> {
    let v = value.as_f64().context("value must be a number")?;
    ensure!(v.is_finite() && (v as f32).is_finite(), "value must be finite");
    Ok(v as f32)
}

pub(crate) fn flag(value:&Value,key:&str,default:bool)->anyhow::Result<bool> {
    value.get(key).map(|v|v.as_bool().with_context(||format!("{key} must be boolean"))).transpose().map(|v|v.unwrap_or(default))
}

pub(crate) fn apply(core: &mut Core, value: &Value) -> anyhow::Result<()> {
    core.rt.audit_depth += 1;
    let result=apply_inner(core,value);
    core.rt.audit_depth -= 1;
    result
}

fn apply_inner(core: &mut Core, value: &Value) -> anyhow::Result<()> {
    let cmd = value["cmd"].as_str().context("cmd is required")?;
    match cmd {
        "button" => core.remote_button(value)?,
        "input" => core.remote_input(value)?,
        "edit" => {crate::editing::apply(core,&value["operations"],flag(value,"dry_run",false)?)?;}
        "set" => {
            let id = u32::try_from(value["fixture"].as_u64().context("fixture id required")?)?;
            let index = core.project.fixture_index(id).context("unknown fixture")?;
            let f = &core.project.fixtures[index];
            let mode = core.lib.mode(&f.profile, f.mode).context("unknown mode")?;
            let attr = value["attr"].as_str().context("attr required")?;
            ensure!(!mode.attr_channels(attr).is_empty(), "unknown attr");
            let number = finite(&value["value"])?;
            core.note_manual();
            core.set_attr(&Selector::Fixture(id), attr, number);
        }
        "effect" => {
            let name = value["name"].as_str().context("name required")?;
            let mut effect = core.project.effects.iter().find(|e|e.name == name).context("unknown effect")?.clone();
            let patch = value["params"].as_object().context("params object required")?;
            let mut data = serde_json::to_value(&effect)?;
            for (key, value) in patch {
                ensure!(["enabled","min","max","rate_hz","beats","phase","spread","wave"].contains(&key.as_str()), "effect field is not writable: {key}");
                if value.is_number() { finite(value)?; }
                data[key] = value.clone();
            }
            effect = serde_json::from_value(data)?;
            ensure!(effect.beats >= 0.0 && effect.rate_hz >= 0.0
                && (0.0..=255.0).contains(&effect.min) && (0.0..=255.0).contains(&effect.max), "invalid effect range/period/rate");
            *core.project.effects.iter_mut().find(|e|e.name == name).unwrap() = effect;
        }
        "motion" => {
            let patch = value["params"].as_object().context("params object required")?;
            let mut data = serde_json::to_value(&core.project.motion)?;
            for (key, value) in patch {
                ensure!(["shape","beats","spread","position","size","points","glide"].contains(&key.as_str()), "motion field is not writable: {key}");
                data[key] = value.clone();
            }
            let motion: crate::motion::Motion = serde_json::from_value(data)?;
            ensure!(motion.beats.is_finite() && motion.beats > 0.0 && (0.0..=1.0).contains(&motion.size)
                && (0.0..=1.0).contains(&motion.glide) && motion.position.is_none_or(|p|p < 4)
                && motion.points.iter().flatten().all(|v|v.is_finite() && v.abs() <= 90.0), "invalid motion parameters");
            core.project.motion = motion;
        }
        "flash" => {
            let on=flag(value,"on",true)?;
            let ms=value.get("hold_ms").map(|v|v.as_u64().context("hold_ms must be an integer")).transpose()?.unwrap_or(500);
            ensure!((1..=5000).contains(&ms),"hold_ms must be in 1..5000");
            crate::web::command(core,value).map_err(anyhow::Error::msg)?;
            core.rt.flash_deadline=on.then(||core.now()+std::time::Duration::from_millis(ms));
        }
        "blackout"|"master"|"fade"|"bpm"|"auto"|"auto_resume"|"preset"|"look"|"color"|"swap"|"scene"|"tap"|"resync" => {
            if let Some(number) = value.get("value") { finite(number)?; }
            if let Some(on) = value.get("on") { ensure!(on.is_boolean(), "on must be boolean"); }
            crate::web::command(core, value).map_err(anyhow::Error::msg)?;
        }
        _ => bail!("unsupported command (safety configuration and ARM are read-only): {cmd}"),
    }
    Ok(())
}

pub(crate) fn validate(project: &Project, lib: &Library) -> anyhow::Result<()> {
    let mut occupied = std::collections::HashSet::new();
    let mut ids = std::collections::HashSet::new();
    for f in &project.fixtures {
        ensure!(ids.insert(f.id), "duplicate fixture id");
        let profile = lib.get(&f.profile).context("unknown profile")?;
        ensure!(profile.validate().errors.is_empty(), "invalid fixture profile: {}", f.profile);
        let mode = profile.modes.get(f.mode).context("unknown mode")?;
        let info = lib.info(&f.profile, f.mode).context("missing fixture channel information")?;
        ensure!(f.values.len() == mode.channels.len(), "fixture values length mismatch");
        ensure!(f.universe < crate::project::NUM_UNIVERSES && f.address > 0 && f.address as usize + mode.channels.len() <= 513, "invalid fixture address");
        for (index, channel) in mode.channels.iter().enumerate() {
            if f.enabled { ensure!(occupied.insert((f.universe, f.address as usize + index)), "overlapping fixture addresses"); }
            let [a,b] = f.limits.get(&channel.attr).copied().unwrap_or([0,255]);
            ensure!((a.min(b)..=a.max(b)).any(|v| !channel.is_forbidden(v)), "no permitted value for {} / {}", f.name, channel.attr);
            if info.master.contains(&index) && channel.safe.is_none() {
                ensure!(a.min(b) == 0 && !channel.is_forbidden(0), "blackout conflicts with limits/forbid");
            }
            if let Some(safe) = channel.safe {
                ensure!(!channel.is_forbidden(safe) && (a.min(b)..=a.max(b)).contains(&safe), "safe value conflicts with limits/forbid");
            }
        }
    }
    Ok(())
}
