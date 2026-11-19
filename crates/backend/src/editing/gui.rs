//! 画面からの編集。機材・グループの削除と名前の変更で、割り当ての参照も一緒に直す。
use super::*;

/// 画面から機材を消す。その機材を指す割り当てやエフェクトも一緒に片付けてから検証し、まとめて反映する。
/// 一緒に消したものの名前を返す
pub fn gui_delete_fixture(core:&mut Core,id:u32)->anyhow::Result<Vec<String>> {
    gui_change(core,|project|remove_fixture(project,id))
}

/// 画面からグループを消す。そのグループを指す割り当てやエフェクトも一緒に片付ける。
/// 一緒に消したものの名前を返す
pub fn gui_delete_group(core:&mut Core,name:&str)->anyhow::Result<Vec<String>> {
    gui_change(core,|project|{
        let index=project.groups.iter().position(|g|g.name==name).context("unknown group")?;
        project.groups.remove(index);
        Ok(remove_targeting(project,&Selector::Group(name.to_string()),None))
    })
}

/// 画面からグループの名前を変える。そのグループを指す割り当てとエフェクトも新しい名前に書き換えるので、何も消えない
pub fn gui_rename_group(core:&mut Core,old:&str,new:&str)->anyhow::Result<()> {
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
pub(super) fn gui_change<T>(core:&mut Core,change:impl FnOnce(&mut Project)->anyhow::Result<T>)->anyhow::Result<T> {
    ensure_idle(core)?;
    let mut project=core.project.clone();
    let result=change(&mut project)?;
    validate(&project,&core.lib)?;
    commit(core,project);
    Ok(result)
}

/// 割り当てが機材の選び方を持っていれば、それを返す
pub(super) fn action_selector(action:&mut Action)->Option<&mut Selector> {
    match action {
        Action::SetValue{sel,..}|Action::Level{sel}|Action::Mute{sel,..}|Action::Role{sel,..}|Action::Arm{sel,..}=>Some(sel),
        _=>None,
    }
}

/// 割り当ての対象が、消すものそのものか。fixture は「選択」の割り当てが指す機材
pub(super) fn targets(action:&Action,gone:&Selector,fixture:Option<u32>)->bool {
    match action {
        Action::SetValue{sel,..}|Action::Level{sel}|Action::Mute{sel,..}|Action::Role{sel,..}|Action::Arm{sel,..}=>sel==gone,
        Action::Select{fixture:id,..}=>fixture==Some(*id),
        _=>false,
    }
}

/// 機材と、その機材への参照をプロジェクトから取り除く
pub(super) fn remove_fixture(project:&mut Project,id:u32)->anyhow::Result<Vec<String>> {
    let index=project.fixture_index(id).context("unknown fixture")?;
    project.fixtures.remove(index);
    for g in &mut project.groups {g.fixtures.retain(|x|*x!=id);}
    project.pairs.retain(|p|p.leader!=id && p.follower!=id);
    for s in project.scenes.iter_mut().chain(project.positions.iter_mut()) {s.values.retain(|v|v.fixture!=id);}
    Ok(remove_targeting(project,&Selector::Fixture(id),Some(id)))
}

/// 消した機材やグループだけを対象にしたエフェクトと割り当ては、残しても動かないので消す。
/// 消したエフェクトを使う割り当てとオートの演出からも外す。消したものの名前を返す
pub(super) fn remove_targeting(project:&mut Project,gone:&Selector,fixture:Option<u32>)->Vec<String> {
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
