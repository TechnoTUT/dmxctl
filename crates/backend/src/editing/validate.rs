//! プロジェクトの検証と、検証済みのプロジェクトの反映。
use super::*;

/// ARM・レーザーの合図の割り当て(押すと ARM されたり、レーザーが出たりする)。並び順も比べる
/// (同じ入力に割り当てたトグルとワンショットは、並び順で結果が変わる)
fn safety_bindings(project:&Project)->anyhow::Result<Vec<(u64,Value)>> {
    project.bindings.iter().filter(|b|b.is_safety()).map(|b|Ok((b.id,serde_json::to_value(b)?))).collect()
}

/// ARM の割り当てごとに、実際に ARM される機材(ARM が要る機材)の ID。グループの中身を変えても、対象は変わる。
/// トグルが今の状態を見るのも、ARM が要る機材だけ(`Core` の `Action::Arm`)
fn arm_targets(project:&Project,lib:&Library)->BTreeMap<u64,Vec<u32>> {
    project.bindings.iter().filter_map(|b|match &b.action {Action::Arm{sel,..}=>Some((b.id,sel)),_=>None}).map(|(id,sel)| {
        let ids=project.fixtures.iter().filter(|f|lib.get(&f.profile).is_some_and(|p|p.needs_arm)).filter(|f|match sel {
            Selector::All=>true,
            Selector::Fixture(id)=>f.id==*id,
            Selector::Group(name)=>project.groups.iter().any(|g|g.name==*name && g.fixtures.contains(&f.id)),
            Selector::Profile(p)=>f.profile==*p,
        }).map(|f|f.id).collect();
        (id,ids)
    }).collect()
}

/// 保護された機材(ARM が要る・動いてよい範囲がある・禁止値がある)か。パッチ・削除・有効状態・出力の経路を守る
fn is_protected(f:&FixtureInstance,lib:&Library)->bool {
    lib.get(&f.profile).is_some_and(|p|p.needs_arm) || !f.limits.is_empty()
        || lib.mode(&f.profile,f.mode).is_some_and(|m|m.channels.iter().any(|c|!c.forbid.is_empty()))
}

/// 安全な値(ブラックアウト・異常時に出す値)がある機材か。ほとんどの機材が当てはまるので、パッチは変えられる。
/// ほかの機材を重ねて、その安全な値を上書きすることだけを拒む
fn has_safe_values(f:&FixtureInstance,lib:&Library)->bool {
    is_protected(f,lib) || lib.mode(&f.profile,f.mode).is_some_and(|m|m.channels.iter().any(|c|c.safe.is_some()))
}

/// 機材がフレームに書くスロット(0始まり)。書かないなら None。出力の処理と同じ条件にする。
/// 通常の出力(`Core::render`): 使う機材で、値の数がチャンネル数と同じもの。512 を超える分は書かない。
/// 異常時のフレーム(`Core::safe_frames`): 使わない機材も書く。512 を超える機材は丸ごと書かない
fn written_slots(f:&FixtureInstance,lib:&Library,emergency:bool)->Option<std::ops::Range<usize>> {
    let mode=lib.mode(&f.profile,f.mode)?;
    let n=mode.channels.len();
    let base=(f.address as usize).checked_sub(1)?;
    if f.universe>=NUM_UNIVERSES {return None;}
    if emergency {
        lib.info(&f.profile,f.mode)?;
        (base+n<=512).then_some(base..base+n)
    } else {
        (f.enabled && f.values.len()==n).then_some(base..(base+n).min(512))
    }
}

/// 安全な値がある機材のスロットのうち、最後に別の機材が書くもの(機材のID, ユニバース, スロット, 最後に書く機材のID)。
/// 出力は機材の並び順に書き、後の機材が上書きする。スロットごとに最後に書く機材だけを見るので、機材の数に比例する
fn overwritten_slots(project:&Project,lib:&Library,emergency:bool)->std::collections::BTreeSet<(u32,usize,usize,u32)> {
    let writers:Vec<_>=project.fixtures.iter().filter_map(|f|Some((f,written_slots(f,lib,emergency)?))).collect();
    let mut last:std::collections::HashMap<(usize,usize),u32>=std::collections::HashMap::new();
    for (f,slots) in &writers {
        for slot in slots.clone() {last.insert((f.universe,slot),f.id);}
    }
    let mut out=std::collections::BTreeSet::new();
    for (f,slots) in writers.iter().filter(|(f,_)|has_safe_values(f,lib)) {
        for slot in slots.clone() {
            if let Some(&id)=last.get(&(f.universe,slot)).filter(|&&id|id!=f.id) {out.insert((f.id,f.universe,slot,id));}
        }
    }
    out
}

/// 出力の送り先(届く機器)。送るときと同じ規則で決める。sACN の優先度のように、送り先を変えない設定は含めない
fn destination(kind:&OutputKind)->(&'static str,String,u32) {
    match kind {
        // Art-Net のパケットには、ユニバースの下位15ビットだけが入る(`artdmx_packet`)
        OutputKind::ArtNet{ip,art_universe}=>("artnet",ip.trim().to_string(),(*art_universe & 0x7FFF) as u32),
        // sACN で IP を指定しないときは、ユニバースから決まるマルチキャストに送る(`Sacn::new`)
        OutputKind::Sacn{ip,sacn_universe,..}=>{
            let ip=if ip.trim().is_empty() {dmxctl_core::io::output::sacn_multicast(*sacn_universe).to_string()} else {ip.trim().to_string()};
            ("sacn",ip,*sacn_universe as u32)
        }
        OutputKind::OpenDmxSerial{port}=>("serial",port.trim().to_string(),0),
        OutputKind::OpenDmxD2xx{index}=>("d2xx",String::new(),*index),
    }
}

/// 保護された機材に届く出力の経路。保護された機材のあるユニバースを送る出力と、その送り先へ送る出力(設定ごと比べる)
fn protected_outputs(old:&Project,new:&Project,universes:&HashSet<usize>)->anyhow::Result<(Vec<Value>,Vec<Value>)> {
    let destinations:HashSet<_>=old.outputs.iter().chain(&new.outputs).filter(|o|universes.contains(&o.universe)).map(|o|destination(&o.kind)).collect();
    let pick=|project:&Project|project.outputs.iter().filter(|o|universes.contains(&o.universe) || destinations.contains(&destination(&o.kind)))
        .map(|o|Ok(serde_json::to_value(o)?)).collect::<anyhow::Result<Vec<_>>>();
    Ok((pick(old)?,pick(new)?))
}

/// 出力の設定が送れる値か(Art-Net のユニバース・sACN のユニバースと優先度・IP)。`validate` と、LAN の差分の反映で使う
pub fn validate_outputs(project:&Project)->anyhow::Result<()> {
    for output in &project.outputs {
        ensure!(output.universe<NUM_UNIVERSES,"output universe is out of range");
        match &output.kind {
            OutputKind::ArtNet{ip,art_universe}=>{let _:std::net::Ipv4Addr=ip.trim().parse().context("invalid Art-Net IPv4 address")?;ensure!(*art_universe<32768,"invalid Art-Net universe");}
            OutputKind::Sacn{ip,sacn_universe,priority}=>{
                if !ip.trim().is_empty() {let _:std::net::Ipv4Addr=ip.trim().parse().context("invalid sACN IPv4 address")?;}
                ensure!((1..=63999).contains(sacn_universe),"invalid sACN universe");
                ensure!(*priority<=200,"invalid sACN priority");
            }
            OutputKind::OpenDmxSerial{port}=>ensure!(!port.trim().is_empty(),"empty serial port"),_=>{},
        }
    }
    Ok(())
}

pub fn safety_unchanged(old:&Project,new:&Project,lib:&Library)->anyhow::Result<()> {
    ensure!(old.motion.pan_front==new.motion.pan_front && old.motion.tilt_up==new.motion.tilt_up,"motion calibration is read-only from CLI");
    // ARM・レーザーの合図の割り当ては、作成・変更・削除・並べ替えのどれもできない。よく押すパッドを差し替えると、
    // 人が押したときに意図せず ARM されたり、レーザーが出たりするため。画面の割り当ての編集で扱う
    ensure!(safety_bindings(old)?==safety_bindings(new)?,"ARM and laser cue bindings are read-only from CLI (edit them in the GUI)");
    ensure!(arm_targets(old,lib)==arm_targets(new,lib),"fixtures armed by ARM bindings are read-only from CLI (check group membership)");
    // 機材の ID が重なると、下の比べ方で同じ ID の別の機材を見てしまう
    let mut ids=HashSet::new();
    ensure!(new.fixtures.iter().all(|f|ids.insert(f.id)),"duplicate fixture id");
    // 動いてよい範囲は、どの機材でも、付ける・外す・変えるのどれもできない(新しい機材に付けることも)。
    // 範囲はブラックアウトより後に当たるので、付けるだけで暗転中の値が変わることがある
    for f in &new.fixtures {
        let before=old.fixtures.iter().find(|o|o.id==f.id).map(|o|&o.limits);
        ensure!(before.map_or(f.limits.is_empty(),|l|*l==f.limits),"safety limits are read-only from CLI: {}",f.name);
    }
    // 保護された機材・安全な値がある機材の値の数がチャンネル数と違うと、出力でその機材ごと飛ばされ、安全の値も範囲も当たらない
    for f in new.fixtures.iter().filter(|f|has_safe_values(f,lib)) {
        if let Some(mode)=lib.mode(&f.profile,f.mode) {
            ensure!(f.values.len()==mode.channels.len(),"protected fixture values length mismatch: {}",f.name);
        }
    }
    for f in old.fixtures.iter().filter(|f|is_protected(f,lib)) {
        let replacement=new.fixtures.iter().find(|n|n.id==f.id).context("protected fixtures cannot be deleted through CLI")?;
        ensure!(replacement.profile==f.profile && replacement.mode==f.mode && replacement.universe==f.universe
            && replacement.address==f.address && replacement.enabled==f.enabled && replacement.limits==f.limits,
            "protected fixture patch, enable state and safety limits are read-only from CLI");
    }
    // 安全な値がある機材のスロットを、別の機材が最後に書くようにすると、安全の値が保たれない。
    // もともとそうなっているスロット(画面で作ったもの)は、そのままなら認める。
    // 通常の出力と異常時のフレームで、それぞれ実際に書く機材で比べる
    for emergency in [false,true] {
        ensure!(overwritten_slots(new,lib,emergency).is_subset(&overwritten_slots(old,lib,emergency)),"fixtures cannot overlap the addresses of protected fixtures from CLI");
    }
    // 出力の送り先・送るユニバースを変えると、保護された機材に別のフレームが届く
    let universes:HashSet<usize>=old.fixtures.iter().chain(&new.fixtures).filter(|f|is_protected(f,lib)).map(|f|f.universe).collect();
    let (before,after)=protected_outputs(old,new,&universes)?;
    ensure!(before==after,"outputs that reach protected fixtures are read-only from CLI");
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
    validate_outputs(project)?;
    let motion=&project.motion;
    ensure!(motion.beats.is_finite() && motion.beats>0.0 && (0.0..=1.0).contains(&motion.size)
        && (0.0..=1.0).contains(&motion.glide) && motion.position.is_none_or(|p|p<4)
        && motion.points.iter().flatten().all(|v|v.is_finite() && v.abs()<=90.0),"invalid motion parameters");
    Ok(())
}

pub(super) fn unique<'a>(names:impl Iterator<Item=&'a str>,kind:&str)->anyhow::Result<HashSet<&'a str>> {
    let mut set=HashSet::new();for name in names {ensure!(!name.trim().is_empty() && set.insert(name),"empty or duplicate {kind} name");}Ok(set)
}

pub use dmxctl_core::recorder::redact;
