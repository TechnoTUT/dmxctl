//! プロジェクトの検証と、検証済みのプロジェクトの反映。
use super::*;

/// ARM・レーザーの合図の割り当て(押すと ARM されたり、レーザーが出たりする)。id ごとに、中身を JSON で比べる
fn safety_bindings(project:&Project)->anyhow::Result<std::collections::BTreeMap<u64,Value>> {
    project.bindings.iter().filter(|b|matches!(b.action,Action::Arm{..}|Action::Laser{..}))
        .map(|b|Ok((b.id,serde_json::to_value(b)?))).collect()
}

pub fn safety_unchanged(old:&Project,new:&Project,lib:&Library)->anyhow::Result<()> {
    ensure!(old.motion.pan_front==new.motion.pan_front && old.motion.tilt_up==new.motion.tilt_up,"motion calibration is read-only from CLI");
    // ARM・レーザーの合図の割り当ては、作成・変更・削除のどれもできない。よく押すパッドを差し替えると、
    // 人が押したときに意図せず ARM されたり、レーザーが出たりするため。画面の割り当ての編集で扱う
    ensure!(safety_bindings(old)?==safety_bindings(new)?,"ARM and laser cue bindings are read-only from CLI (edit them in the GUI)");
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
    crate::commands::validate(project,lib)?;
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
    // 選んだ機材(使う・使わないに関係なく)。属性やピクセルがあるかを確かめる。1台も選ばないときは確かめない
    let chosen=|sel:&Selector|->Vec<&FixtureInstance> {project.fixtures.iter().filter(|f|match sel {
        Selector::All=>true,Selector::Fixture(id)=>f.id==*id,
        Selector::Group(name)=>project.groups.iter().any(|g|g.name==*name && g.fixtures.contains(&f.id)),
        Selector::Profile(p)=>f.profile==*p,
    }).collect()};
    let has_attr=|sel:&Selector,attr:&str|{let fs=chosen(sel);fs.is_empty() || fs.iter().any(|f|lib.info(&f.profile,f.mode).is_some_and(|i|!i.channels(attr).is_empty()))};
    let has_pixels=|sel:&Selector|{let fs=chosen(sel);fs.is_empty() || fs.iter().any(|f|lib.info(&f.profile,f.mode).is_some_and(|i|!i.pixels.is_empty()))};
    for f in &project.fixtures {ensure!(!f.name.trim().is_empty(),"fixture name is empty");}
    // 名前で機材を選べるように、名前は重ねない
    unique(project.fixtures.iter().map(|f|f.name.as_str()),"fixture")?;
    for group in &project.groups {
        ensure!(group.fixtures.iter().all(|id|fixtures.contains(id)),"group references unknown fixture");
        ensure!(group.fixtures.iter().collect::<HashSet<_>>().len()==group.fixtures.len(),"group {} lists the same fixture more than once",group.name);
    }
    for scene in project.scenes.iter().chain(&project.positions) {
        ensure!((0.0..=120.0).contains(&scene.fade),"invalid scene fade");
        for v in &scene.values {
            let f=project.fixtures.iter().find(|f|f.id==v.fixture).context("scene references unknown fixture")?;
            ensure!(!lib.mode(&f.profile,f.mode).unwrap().attr_channels(&v.attr).is_empty() && (0.0..256.0).contains(&v.value),"invalid scene attribute/value");
        }
    }
    for effect in &project.effects {
        selector(&effect.sel)?;ensure!(!effect.name.trim().is_empty() && !effect.attr.trim().is_empty(),"effect name/attr is empty");
        let name=&effect.name;
        // 使うエフェクトだけ確かめる(使わないものは、機材を入れ替えたあとにも残っていてよい)
        ensure!(!effect.enabled || has_attr(&effect.sel,&effect.attr),"effect {name}: no selected fixture has attribute {}",effect.attr);
        ensure!((0.0..=255.0).contains(&effect.min) && (0.0..=255.0).contains(&effect.max),"effect {name}: min and max must be 0..255");
        ensure!(effect.rate_hz.is_finite() && (0.0..=MAX_RATE_HZ).contains(&effect.rate_hz),"effect {name}: rate_hz must be 0..{MAX_RATE_HZ}");
        ensure!(effect.beats.is_finite() && (0.0..=MAX_BEATS).contains(&effect.beats),"effect {name}: beats must be 0..{MAX_BEATS}");
        ensure!(effect.beats>0.0 || effect.rate_hz>0.0,"effect {name}: beats or rate_hz must be greater than 0");
        ensure!(effect.phase.is_finite() && effect.spread.is_finite(),"effect {name}: phase and spread must be finite");
    }
    unique(project.pixel_fx.iter().map(|s|s.name.as_str()),"pixel effect")?;
    for effect in &project.pixel_fx {
        selector(&effect.sel)?;
        let name=&effect.name;
        ensure!(!effect.enabled || has_pixels(&effect.sel),"pixel effect {name}: no selected fixture has pixels");
        ensure!(effect.beats.is_finite() && (0.0..=MAX_BEATS).contains(&effect.beats),"pixel effect {name}: beats must be 0..{MAX_BEATS}");
        ensure!(effect.width.is_finite() && (0.0..=MAX_PIXEL_WIDTH).contains(&effect.width),"pixel effect {name}: width must be 0..{MAX_PIXEL_WIDTH}");
        ensure!(effect.skew.is_finite() && effect.skew.abs()<=MAX_PIXEL_SKEW,"pixel effect {name}: skew must be -{MAX_PIXEL_SKEW}..{MAX_PIXEL_SKEW}");
        for color in [&effect.a,&effect.b] {
            if let dmxctl_core::pixel::PixelColor::Custom(rgb)=color {ensure!(rgb.iter().all(|v|(0.0..=1.0).contains(v)),"pixel effect {name}: Custom color components must be 0..1");}
        }
    }
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
            Action::SetValue{sel,attr,on,off,..}=>{
                selector(sel)?;ensure!(!attr.is_empty() && (0.0..256.0).contains(on) && (0.0..256.0).contains(off),"invalid value binding");
                ensure!(has_attr(sel,attr),"binding {}: no selected fixture has attribute {attr}",binding.name);
            }
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
            OutputKind::Sacn{ip,sacn_universe,priority}=>{
                if !ip.trim().is_empty() {let _:std::net::Ipv4Addr=ip.trim().parse().context("invalid sACN IPv4 address")?;}
                ensure!((1..=63999).contains(sacn_universe),"invalid sACN universe");
                ensure!(*priority<=200,"invalid sACN priority");
            }
            OutputKind::OpenDmxSerial{port}=>ensure!(!port.trim().is_empty(),"empty serial port"),_=>{},
        }
    }
    let motion=&project.motion;
    ensure!(motion.beats.is_finite() && motion.beats>0.0 && motion.beats<=MAX_BEATS,"motion beats must be greater than 0 and at most {MAX_BEATS}");
    ensure!((0.0..=1.0).contains(&motion.size),"motion size must be 0..1");
    ensure!((0.0..=1.0).contains(&motion.glide),"motion glide must be 0..1");
    ensure!(motion.position.is_none_or(|p|p<4),"motion position must be 0..3");
    ensure!(motion.points.iter().flatten().all(|v|v.is_finite() && v.abs()<=90.0),"motion points must be -90..90");
    Ok(())
}

/// エフェクトの周期の上限(拍)と速さの上限(Hz)、ピクセル効果の幅・ずれの上限。画面で選べる範囲より十分に広い
const MAX_BEATS:f32=1024.0;
const MAX_RATE_HZ:f32=50.0;
const MAX_PIXEL_WIDTH:f32=1024.0;
const MAX_PIXEL_SKEW:f32=64.0;

pub(super) fn unique<'a>(names:impl Iterator<Item=&'a str>,kind:&str)->anyhow::Result<HashSet<&'a str>> {
    let mut set=HashSet::new();for name in names {ensure!(!name.trim().is_empty() && set.insert(name),"empty or duplicate {kind} name");}Ok(set)
}

pub use dmxctl_core::recorder::redact;
