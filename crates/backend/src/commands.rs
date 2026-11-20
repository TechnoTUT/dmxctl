//! Validated operations shared by offline runs and the running application.
use crate::controls::CoreControls;
use dmxctl_core::{color::NamedColor, engine::Core, fixture::{DMX_FINE_MAX, Library}, project::{BPM_MAX, BPM_MIN, Project, Selector}};
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
    core.rt.observe.enter_command();
    let result=apply_inner(core,value);
    core.rt.observe.leave_command();
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
            let channels = mode.attr_channels(attr);
            ensure!(!channels.is_empty(), "unknown attr");
            let max = if channels.iter().all(|&channel| mode.fine_of(channel).is_some()) {
                f64::from(DMX_FINE_MAX)
            } else {
                255.0
            };
            let number = value["value"].as_f64().context("value must be a number")?;
            ensure!((0.0..=max).contains(&number), "value must be in 0..{max} (DMX)");
            core.note_manual();
            core.set_attr(&Selector::Fixture(id), attr, number as f32);
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
            let motion: dmxctl_core::motion::Motion = serde_json::from_value(data)?;
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
            // web::command は操作パネル向けに日本語で失敗を返す。ctl・MCP の JSON は英語にするため、ここで先に確かめる
            check_panel_command(core,cmd,value)?;
            // 画面のスライダーは範囲内しか出さないので丸めているが、ctl・MCP では範囲外を黙って丸めずに知らせる
            let number = value.get("value").and_then(Value::as_f64);
            if cmd=="master" { ensure!(number.is_some_and(|v|(0.0..=1.0).contains(&v)),"master value must be in 0..1"); }
            if cmd=="bpm" { ensure!(number.is_some_and(|v|(BPM_MIN as f64..=BPM_MAX as f64).contains(&v)),"bpm value must be in {BPM_MIN}..{BPM_MAX}"); }
            if let Some(on) = value.get("on") { ensure!(on.is_boolean(), "on must be boolean"); }
            crate::web::command(core, value).map_err(anyhow::Error::msg)?;
        }
        _ => bail!("unsupported command (safety configuration and ARM are read-only): {cmd}"),
    }
    Ok(())
}

/// 操作パネルと共通の操作の入力を確かめる(失敗の理由は英語)
fn check_panel_command(core:&Core,cmd:&str,value:&Value)->anyhow::Result<()> {
    let text=|key:&str|->anyhow::Result<&str> {value.get(key).with_context(||format!("{key} is required"))?.as_str().with_context(||format!("{key} must be a string"))};
    match cmd {
        "master"|"bpm" => {value.get("value").context("value is required")?;}
        "fade" => if let Some(secs)=value.get("secs") {
            ensure!(secs.as_f64().is_some_and(|s|(0.0..=60.0).contains(&s)),"secs must be a number in 0..60");
        },
        "color" => {
            let name=text("name")?;
            ensure!(NamedColor::from_name(name).is_some(),"unknown color: {name} (use a Japanese name such as 赤 or an English name such as red)");
            if let Some(slot)=value.get("slot") {ensure!(slot=="main" || slot=="sub","slot must be main or sub");}
        }
        "scene" => {let name=text("name")?;ensure!(core.project.scenes.iter().any(|s|s.name==name),"unknown scene: {name}");}
        "preset" => {
            let index=value.get("index").context("index is required")?.as_u64().context("index must be a non-negative integer")?;
            ensure!((index as usize)<core.project.auto.presets.len(),"unknown preset index: {index}");
        }
        "look" => {let phrase=text("phrase")?;ensure!(core.preset_look(phrase).is_some(),"unknown phrase: {phrase}");}
        _ => {}
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
        ensure!(f.universe < dmxctl_core::project::NUM_UNIVERSES, "fixture universe must be in 0..{} (zero-based in project.json): {}", dmxctl_core::project::NUM_UNIVERSES - 1, f.name);
        ensure!(f.address > 0 && f.address as usize + mode.channels.len() <= 513, "fixture address must be in 1..={} so that all {} channels fit in 512: {}", 513 - mode.channels.len(), mode.channels.len(), f.name);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{path::Path, sync::Arc};

    fn core() -> Core {
        let (lib, _) = Library::load(Path::new("none"));
        Core::new(Project::starter(&lib), Arc::new(lib))
    }

    #[test]
    fn master_and_bpm_out_of_range_are_rejected_not_clamped() {
        let mut c = core();
        apply(&mut c, &json!({"cmd": "master", "value": 0.5})).unwrap();
        assert!(apply(&mut c, &json!({"cmd": "master", "value": -0.1})).is_err());
        assert!(apply(&mut c, &json!({"cmd": "master", "value": 1.5})).is_err());
        assert_eq!(c.rt.master, 0.5, "拒んだ値で変えない");
        apply(&mut c, &json!({"cmd": "bpm", "value": 128})).unwrap();
        assert!(apply(&mut c, &json!({"cmd": "bpm", "value": 10})).is_err());
        assert!(apply(&mut c, &json!({"cmd": "bpm", "value": 400})).is_err());
        assert_eq!(c.project.bpm, 128.0);
    }

    #[test]
    fn set_values_out_of_the_dmx_range_are_rejected() {
        let mut c = core();
        for v in [999.0, -50.0, 256.0, -1e-50, 255.5] {
            let e = apply(&mut c, &json!({"cmd": "set", "fixture": 1, "attr": "dimmer", "value": v})).unwrap_err().to_string();
            assert!(e.starts_with("value must be in 0.."), "{v}: {e}");
        }
        for v in [0.0, 255.0] {
            apply(&mut c, &json!({"cmd": "set", "fixture": 1, "attr": "pan", "value": v})).unwrap_or_else(|e| panic!("{v}: {e}"));
        }
    }

    #[test]
    fn set_preserves_the_fine_byte_at_the_dmx_maximum() {
        let mut c = core();
        let fixture = &mut c.project.fixtures[0];
        fixture.mode = 1;
        let mode = c.lib.mode(&fixture.profile, fixture.mode).unwrap();
        fixture.values.resize(mode.channels.len(), 0);
        let fixture = fixture.id;
        let error = apply(&mut c, &json!({"cmd": "set", "fixture": fixture, "attr": "pan", "value": 255.996094})).unwrap_err();
        assert!(error.to_string().starts_with("value must be in 0..255.99609375"));
        apply(&mut c, &json!({"cmd": "set", "fixture": fixture, "attr": "pan", "value": 255.99609375})).unwrap();
        let fixture = c.project.fixtures.iter().find(|item| item.id == fixture).unwrap();
        let mode = c.lib.mode(&fixture.profile, fixture.mode).unwrap();
        assert_eq!(mode.get_f(&fixture.values, mode.find_attr("pan").unwrap()), DMX_FINE_MAX);
    }

    #[test]
    fn universe_and_address_errors_are_told_apart() {
        let c = core();
        let mut p = c.project.clone();
        p.fixtures[0].universe = 5000;
        assert!(validate(&p, &c.lib).unwrap_err().to_string().starts_with("fixture universe must be in 0.."));
        let mut p = c.project.clone();
        p.fixtures[0].address = 510;
        assert!(validate(&p, &c.lib).unwrap_err().to_string().starts_with("fixture address must be in 1..="));
    }

    #[test]
    fn colors_are_found_by_japanese_or_english_name() {
        let mut c = core();
        for name in ["赤", "red", "Red", " RED "] {
            apply(&mut c, &json!({"cmd": "color", "name": name, "slot": "main"})).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        apply(&mut c, &json!({"cmd": "color", "name": "青", "slot": "sub"})).unwrap();
        assert!(apply(&mut c, &json!({"cmd": "color", "name": "存在しない色"})).is_err());
    }

    #[test]
    fn errors_returned_to_ctl_and_mcp_are_in_english() {
        let err = |v: serde_json::Value| apply(&mut core(), &v).unwrap_err().to_string();
        assert_eq!(err(json!({"cmd": "fade", "on": false, "secs": 100})), "secs must be a number in 0..60");
        assert_eq!(err(json!({"cmd": "fade", "on": false, "secs": "3"})), "secs must be a number in 0..60");
        assert!(err(json!({"cmd": "color", "name": "存在しない色"})).starts_with("unknown color: 存在しない色"));
        assert_eq!(err(json!({"cmd": "color"})), "name is required");
        assert_eq!(err(json!({"cmd": "color", "name": "赤", "slot": "left"})), "slot must be main or sub");
        assert_eq!(err(json!({"cmd": "scene", "name": "無い"})), "unknown scene: 無い");
        assert_eq!(err(json!({"cmd": "preset", "index": 99})), "unknown preset index: 99");
        assert_eq!(err(json!({"cmd": "look", "phrase": "nope"})), "unknown phrase: nope");
        assert_eq!(err(json!({"cmd": "master"})), "value is required");
        // 操作パネルの日本語の表示は今までどおり
        let mut c = core();
        assert_eq!(crate::web::command(&mut c, &json!({"cmd": "fade", "on": false, "secs": 100})).unwrap_err(), "secs は 0〜60 の数値です");
        assert_eq!(crate::web::command(&mut c, &json!({"cmd": "color", "name": "存在しない色"})).unwrap_err(), "その色はありません");
    }

    #[test]
    fn fade_takes_seconds() {
        let mut c = core();
        apply(&mut c, &json!({"cmd": "fade", "on": true, "secs": 0.0})).unwrap();
        c.render();
        assert!(c.rt.dim <= 0.001, "0秒ですぐ暗くなる: {}", c.rt.dim);
        assert!(apply(&mut c, &json!({"cmd": "fade", "on": false, "secs": 100})).is_err());
        assert!(apply(&mut c, &json!({"cmd": "fade", "on": false, "secs": "3"})).is_err());
    }
}
