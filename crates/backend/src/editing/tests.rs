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
    let project:Project=serde_json::from_str(include_str!("../../../../config/project.json")).unwrap();
    let mut c=Core::new(project,Arc::new(lib));
    assert!(c.project.bindings.iter().any(|b|targets(&b.action,&Selector::Fixture(1),Some(1))));
    let removed=gui_delete_fixture(&mut c,1).unwrap();
    assert!(removed.iter().any(|r|r.contains("パッド56")));
    crate::editing::gui_apply(&mut c,&json!([{"entity":"fixture","op":"create","data":{"profile":"stage-evolution/minispot30"}}])).unwrap();
    crate::editing::gui_apply(&mut c,&json!([{"entity":"group","op":"create","data":{"name":"G"}}])).unwrap();
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

#[test]
fn sacn_outputs_are_validated() {
    let c=core();
    let check=|kind:OutputKind|{let mut p=c.project.clone();p.outputs=vec![OutputCfg{enabled:true,universe:0,kind}];validate::validate(&p,&c.lib)};
    let sacn=|ip:&str,sacn_universe:u16,priority:u8|OutputKind::Sacn{ip:ip.into(),sacn_universe,priority};
    assert!(check(sacn("",1,100)).is_ok(),"空ならマルチキャスト");
    assert!(check(sacn("192.168.1.20",63999,200)).is_ok());
    assert!(check(sacn("",0,100)).is_err(),"ユニバース0は無い");
    assert!(check(sacn("",64000,100)).is_err());
    assert!(check(sacn("nope",1,100)).is_err());
    assert!(check(sacn("",1,201)).is_err(),"優先度は200まで");
    // 優先度を省いた保存ファイルは標準の100で読む
    let kind:OutputKind=serde_json::from_value(json!({"Sacn":{"sacn_universe":3}})).unwrap();
    assert_eq!(kind,sacn("",3,100));
}

#[test]
fn arm_and_laser_cue_bindings_are_read_only_from_cli() {
    let mut c=core();
    let arm=json!({"name":"ARM","trigger":null,"action":{"Arm":{"sel":"All","mode":"Toggle"}}});
    let laser=json!({"name":"レーザー","trigger":null,"action":{"Laser":{"mode":"Momentary"}}});
    let flash=json!({"name":"フラッシュ","trigger":null,"action":{"Flash":{"white":true,"mode":"Momentary"}}});
    let rejected=|c:&mut Core,ops:Value| {
        let e=apply(c,&ops,true).unwrap_err().to_string();
        assert!(e.contains("ARM and laser cue bindings are read-only"),"{e}");
    };
    // 作れない(試すだけでも)
    rejected(&mut c,json!([{"entity":"binding","op":"create","data":arm}]));
    rejected(&mut c,json!([{"entity":"binding","op":"create","data":laser}]));
    // 画面の編集では作れる
    gui_apply(&mut c,&json!([{"entity":"binding","op":"create","data":arm},{"entity":"binding","op":"create","data":flash}])).unwrap();
    let id=|c:&Core,name:&str|format!("binding:{}",c.project.bindings.iter().find(|b|b.name==name).unwrap().id);
    let (arm_id,flash_id)=(id(&c,"ARM"),id(&c,"フラッシュ"));
    // 既存の割り当てを ARM・レーザーの合図に差し替えられない
    rejected(&mut c,json!([{"entity":"binding","op":"update","key":flash_id,"data":{"action":{"Laser":{"mode":"Momentary"}}}}]));
    // ARM の割り当ての名前・きっかけ・削除も変えられない
    rejected(&mut c,json!([{"entity":"binding","op":"update","key":arm_id,"data":{"name":"別名"}}]));
    rejected(&mut c,json!([{"entity":"binding","op":"delete","key":arm_id}]));
    // それ以外の割り当ては今までどおり編集できる
    apply(&mut c,&json!([{"entity":"binding","op":"update","key":flash_id,"data":{"name":"白フラッシュ"}}]),false).unwrap();
    assert!(c.project.bindings.iter().any(|b|b.name=="白フラッシュ"));
}
