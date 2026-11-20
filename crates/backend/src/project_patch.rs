//! プロジェクトの差分(画面の1フレームで変わったところ)。
//!
//! 画面はプロジェクトの値(スライダー・チェックボックス・名前の欄など)を直接書き換える。
//! サーバーにつなぐクライアントでは、1フレームの前後のプロジェクトを比べて、変わったところだけを
//! この差分にしてサーバーへ送り、サーバーで同じように当てる。
//!
//! - オブジェクトはキーごと、同じ長さの配列は要素ごとに比べる(スライダー1本なら、その値1つだけになる)
//! - 長さが変わった配列(追加・削除)は、配列ごと置き換える
//! - クライアントの編集には、変える前の値(`base`)を付ける。サーバーは、今の値がそれと違えば当てない。
//!   ほかのクライアントが先に同じ項目を変えた・一覧に追加や削除をして位置がずれた、という行き違いで、
//!   編集が別のものに当たったり、ほかの編集を消したりしないように
//!   (位置がずれても、たまたま同じ値の項目に当たったときは見分けられない)
//! - 当てるときは厳しい検証をしない(画面の編集では、名前を消して打ち直す途中なども許すため。保存・自動操作は従来どおり)

use dmxctl_core::project::Project;
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// その機械のものなので、つないだ先と同期しない項目(MIDI機器は画面とMIDIを持つ側のもの)
pub const LOCAL_KEYS: &[&str] = &["midi_input"];

/// これが変わったら、サービス(OSC・Link・NDI・音声・操作パネル)の設定し直しが要る
const SERVICE_KEYS: &[&str] = &["osc_enabled", "osc_port", "tempo_source", "clock_input", "ndi", "audio_input", "web_enabled", "web_port"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Seg {
    Index(usize),
    Key(String),
}

/// 1か所の変更。`value` が無ければ、そのキーを消す(None のときに書き出さない項目など)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub path: Vec<Seg>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    /// 変える前の値(無いキーは null)。あれば、当てる先の今の値がこれと同じときだけ当てる
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<Value>,
}

/// 当てる先の今の値が、編集の前の値と違った(ほかから先に変えられた)
#[derive(Debug)]
pub struct Conflict;

impl std::fmt::Display for Conflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ほかの画面の編集と重なったので、この編集は取り消しました")
    }
}

impl std::error::Error for Conflict {}

/// 当てられない編集(安全の設定に触れる・存在しない項目を指す など)。行き違いではないので、理由をそのまま知らせる
#[derive(Debug)]
pub struct Rejected(pub String);

impl std::fmt::Display for Rejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Rejected {}

/// 安全の設定に触れる編集を拒んだときの理由
pub const SAFETY_READ_ONLY: &str = "安全の設定(レーザーの動いてよい範囲・保護された機材のパッチ・ムービングの向きの校正・ARM とレーザーの合図の割り当て)は、サーバーのPCの画面か、サーバーを --remote-allow-safety-edit を付けて起動したときだけ変えられます";

/// `old` から `new` への差分(サーバーからクライアントへ配るもの。前の値は付けない)
pub fn diff(old: &Value, new: &Value) -> Vec<Change> {
    let mut out = Vec::new();
    walk(&mut Vec::new(), old, new, false, &mut out);
    out
}

/// クライアントの編集の差分。前の値を付け、その機械だけの項目は除く
pub fn diff_project(old: &Value, new: &Value) -> Vec<Change> {
    let mut out = Vec::new();
    walk(&mut Vec::new(), old, new, true, &mut out);
    out.retain(|c| !matches!(c.path.first(), Some(Seg::Key(k)) if LOCAL_KEYS.contains(&k.as_str())));
    out
}

fn walk(path: &mut Vec<Seg>, old: &Value, new: &Value, with_base: bool, out: &mut Vec<Change>) {
    let base = |v: &Value| with_base.then(|| v.clone());
    if old == new {
        return;
    }
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            for (k, va) in a {
                path.push(Seg::Key(k.clone()));
                match b.get(k) {
                    Some(vb) => walk(path, va, vb, with_base, out),
                    None => out.push(Change { path: path.clone(), value: None, base: base(va) }),
                }
                path.pop();
            }
            for (k, vb) in b {
                if !a.contains_key(k) {
                    path.push(Seg::Key(k.clone()));
                    out.push(Change { path: path.clone(), value: Some(vb.clone()), base: base(&Value::Null) });
                    path.pop();
                }
            }
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            for (i, (va, vb)) in a.iter().zip(b).enumerate() {
                path.push(Seg::Index(i));
                walk(path, va, vb, with_base, out);
                path.pop();
            }
        }
        _ => out.push(Change { path: path.clone(), value: Some(new.clone()), base: base(old) }),
    }
}

/// 差分を当てる。途中の場所が無い(相手の配列が先に短くなったなど)・今の値が前の値と違う(`Conflict`)ときは、
/// エラーにして何も変えない
pub fn apply(target: &mut Value, changes: &[Change]) -> anyhow::Result<()> {
    let mut work = target.clone();
    for c in changes {
        apply_one(&mut work, c)?;
    }
    *target = work;
    Ok(())
}

fn apply_one(target: &mut Value, change: &Change) -> anyhow::Result<()> {
    let Some((last, parents)) = change.path.split_last() else {
        if change.base.as_ref().is_some_and(|b| b != target) {
            return Err(Conflict.into());
        }
        *target = change.value.clone().context("cannot remove the whole project")?;
        return Ok(());
    };
    let mut at = target;
    for seg in parents {
        at = match seg {
            Seg::Key(k) => at.get_mut(k.as_str()).with_context(|| format!("no such field: {k}"))?,
            Seg::Index(i) => at.get_mut(*i).with_context(|| format!("index out of range: {i}"))?,
        };
    }
    if let Some(base) = &change.base {
        let now = match last {
            Seg::Key(k) => at.get(k.as_str()).unwrap_or(&Value::Null),
            Seg::Index(i) => at.get(*i).ok_or(Conflict)?,
        };
        if now != base {
            return Err(Conflict.into());
        }
    }
    match (last, &change.value) {
        (Seg::Key(k), Some(v)) => {
            at.as_object_mut().context("not an object")?.insert(k.clone(), v.clone());
        }
        (Seg::Key(k), None) => {
            at.as_object_mut().context("not an object")?.remove(k);
        }
        (Seg::Index(i), Some(v)) => {
            *at.get_mut(*i).with_context(|| format!("index out of range: {i}"))? = v.clone();
        }
        (Seg::Index(_), None) => bail!("cannot remove an array element by path"),
    }
    Ok(())
}

/// 差分を当てた結果
#[derive(Debug, Default, PartialEq)]
pub struct Applied {
    /// サービスの設定し直し(`Host::apply_services`)が要る
    pub services: bool,
}

pub trait CoreProjectPatch {
    /// `guard_safety`: 安全の設定に触れる編集を拒む(リモートのクライアントから。`--remote-allow-safety-edit` なら false)
    fn apply_project_patch(&mut self, changes: &[Change], guard_safety: bool) -> anyhow::Result<Applied>;
}

fn lookup<'a>(mut at: &'a Value, path: &[Seg]) -> Option<&'a Value> {
    for seg in path {
        at = match seg {
            Seg::Key(k) => at.get(k.as_str())?,
            Seg::Index(i) => at.get(*i)?,
        };
    }
    Some(at)
}

fn path_text(path: &[Seg]) -> String {
    path.iter().map(|s| match s { Seg::Key(k) => k.clone(), Seg::Index(i) => i.to_string() }).collect::<Vec<_>>().join(".")
}

/// 設定(Project)の中で、空のときに書き出さない項目(`skip_serializing_if`)。機材の limits、割り当ての page・osc
const OMITTED_WHEN_EMPTY: &[&str] = &["limits", "page", "osc"];

/// 書き込んだ値のうち、設定に読み直したときに捨てられた項目(打ち間違い・古い版の項目)の場所。
/// 書き出すときに空なら省く項目(`OMITTED_WHEN_EMPTY`)だけは、空の値なら無くてもよい
fn dropped_key(given: &Value, kept: &Value, path: &mut Vec<Seg>) -> bool {
    match (given, kept) {
        (Value::Object(g), Value::Object(k)) => g.iter().any(|(key, gv)| {
            path.push(Seg::Key(key.clone()));
            let dropped = match k.get(key) {
                Some(kv) => dropped_key(gv, kv, path),
                None => !(OMITTED_WHEN_EMPTY.contains(&key.as_str()) && (gv.is_null() || gv.as_object().is_some_and(|o| o.is_empty()))),
            };
            if !dropped { path.pop(); }
            dropped
        }),
        (Value::Array(g), Value::Array(k)) if g.len() == k.len() => g.iter().zip(k).enumerate().any(|(i, (gv, kv))| {
            path.push(Seg::Index(i));
            let dropped = dropped_key(gv, kv, path);
            if !dropped { path.pop(); }
            dropped
        }),
        _ => false,
    }
}

impl CoreProjectPatch for dmxctl_core::engine::Core {
    /// 画面の編集の差分を当てる(厳しい検証はしない)。当てられなければ何も変えない
    fn apply_project_patch(&mut self, changes: &[Change], guard_safety: bool) -> anyhow::Result<Applied> {
        if changes.is_empty() {
            return Ok(Applied::default());
        }
        for c in changes {
            if let Some(Seg::Key(k)) = c.path.first() {
                anyhow::ensure!(!LOCAL_KEYS.contains(&k.as_str()), "{k} is local to each machine");
            }
        }
        // 存在しない項目(打ち間違い・古い版の項目)は、書き込んでも設定に読み直すときに捨てられ、
        // 黙って何も変わらないので、ここで見つけて拒む
        let missing = |path: &[Seg]| Err(Rejected(format!("存在しない項目です: {}", path_text(path))).into());
        let mut value = serde_json::to_value(&self.project)?;
        for c in changes {
            // 消す編集は、今その項目があること
            if c.value.is_none() && lookup(&value, &c.path).is_none() {
                return missing(&c.path);
            }
            apply_one(&mut value, c)?;
        }
        let mut project: Project = serde_json::from_value(value).map_err(|e| Rejected(format!("編集の値が正しくありません: {e}")))?;
        let applied = serde_json::to_value(&project)?;
        for c in changes {
            let Some(given) = &c.value else { continue };
            let Some(kept) = lookup(&applied, &c.path) else { return missing(&c.path) };
            let mut path = c.path.clone();
            if dropped_key(given, kept, &mut path) {
                return missing(&path);
            }
        }
        // 割り当ての ID の重複・欠けは、反映するときに付け直す。安全の確認は付け直した後の設定で行う
        project.next_binding_id = project.next_binding_id.max(self.project.next_binding_id);
        project.ensure_binding_ids();
        // 自動操作の窓口(ctl・MCP)と同じ安全の確認。レーザーの範囲・保護された機材・校正・ARM の割り当てを守る
        if guard_safety {
            crate::editing::safety_unchanged(&self.project, &project, &self.lib).map_err(|e| Rejected(format!("{SAFETY_READ_ONLY}({e})")))?;
        }
        let services = changes.iter().any(|c| matches!(c.path.first(), Some(Seg::Key(k)) if SERVICE_KEYS.contains(&k.as_str())));
        // ARM(保存しない値)は機材IDで引き継ぎ、フェード・チェイス・出力などの後始末をする
        crate::editing::commit_with_arm(self, project, true);
        Ok(Applied { services })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dmxctl_core::engine::Core;
    use dmxctl_core::fixture::Library;
    use serde_json::json;
    use std::sync::Arc;
    use dmxctl_core::project::Action;

    fn core() -> Core {
        let (lib, _) = Library::load(std::path::Path::new("/nonexistent"));
        let project = Project::starter(&lib);
        Core::new(project, Arc::new(lib))
    }

    #[test]
    fn diff_then_apply_reproduces_the_new_value() {
        let old = json!({"a": 1, "b": [1, 2, 3], "c": {"x": "keep", "y": null}, "gone": true, "list": [1]});
        let new = json!({"a": 2, "b": [1, 5, 3], "c": {"x": "keep", "y": [1]}, "added": {"k": 1}, "list": [1, 2]});
        let changes = diff(&old, &new);
        let mut v = old.clone();
        apply(&mut v, &changes).unwrap();
        assert_eq!(v, new);
        // 同じ長さの配列は要素だけ、長さが変わった配列は丸ごと
        assert!(changes.contains(&Change { path: vec![Seg::Key("b".into()), Seg::Index(1)], value: Some(json!(5)), base: None }));
        assert!(changes.contains(&Change { path: vec![Seg::Key("list".into())], value: Some(json!([1, 2])), base: None }));
        assert!(changes.contains(&Change { path: vec![Seg::Key("gone".into())], value: None, base: None }));
    }

    #[test]
    fn changes_round_trip_through_json() {
        let changes = diff(&json!({"f": [{"v": [0, 0]}]}), &json!({"f": [{"v": [0, 9]}], "n": 1}));
        let text = serde_json::to_string(&changes).unwrap();
        assert_eq!(serde_json::from_str::<Vec<Change>>(&text).unwrap(), changes);
    }

    #[test]
    fn failed_patch_changes_nothing() {
        let mut v = json!({"a": [1]});
        let bad = [Change { path: vec![Seg::Key("a".into()), Seg::Index(0)], value: Some(json!(7)), base: None },
            Change { path: vec![Seg::Key("a".into()), Seg::Index(5)], value: Some(json!(1)), base: None }];
        assert!(apply(&mut v, &bad).is_err());
        assert_eq!(v, json!({"a": [1]}));
    }

    #[test]
    fn a_client_edit_applies_only_on_top_of_what_it_saw() {
        let before = json!({"scenes": [{"name": "a"}, {"name": "b"}, {"name": "c"}], "n": 1});
        // B: 2番目の名前を変える / A: 先頭を消す(配列ごと置き換え)
        let rename = diff_project(&before, &json!({"scenes": [{"name": "a"}, {"name": "B"}, {"name": "c"}], "n": 1}));
        let delete = diff_project(&before, &json!({"scenes": [{"name": "b"}, {"name": "c"}], "n": 1}));
        assert_eq!(rename[0].base, Some(json!("b")));
        // A が先: B の編集は、詰めたあとの別のシーンに当たらない
        let mut v = before.clone();
        apply(&mut v, &delete).unwrap();
        assert!(apply(&mut v, &rename).unwrap_err().is::<Conflict>());
        assert_eq!(v, json!({"scenes": [{"name": "b"}, {"name": "c"}], "n": 1}));
        // B が先: A の削除は、B の変更を消さない
        let mut v = before.clone();
        apply(&mut v, &rename).unwrap();
        assert!(apply(&mut v, &delete).unwrap_err().is::<Conflict>());
        assert_eq!(v["scenes"][1]["name"], "B");
        // 続けて同じ値を動かす(スライダー)のは、前の編集の上に重なる
        let mut v = json!({"n": 1});
        apply(&mut v, &diff_project(&json!({"n": 1}), &json!({"n": 2}))).unwrap();
        apply(&mut v, &diff_project(&json!({"n": 2}), &json!({"n": 3}))).unwrap();
        assert_eq!(v, json!({"n": 3}));
        // サーバーから配る差分には前の値を付けない
        assert!(diff(&before, &json!({"n": 2})).iter().all(|c| c.base.is_none()));
    }

    #[test]
    fn a_slider_edit_becomes_one_change_and_keeps_arm() {
        let mut server = core();
        let mut client = core();
        server.project.fixtures[0].armed = true;
        let before = serde_json::to_value(&client.project).unwrap();
        client.project.fixtures[0].values[0] = 200;
        client.project.midi_input = "this machine only".into();
        let after = serde_json::to_value(&client.project).unwrap();
        let changes = diff_project(&before, &after);
        assert_eq!(changes.len(), 1, "{changes:?}");
        let applied = server.apply_project_patch(&changes, true).unwrap();
        assert_eq!(applied, Applied { services: false });
        assert_eq!(server.project.fixtures[0].values[0], 200);
        assert!(server.project.fixtures[0].armed, "ARM must survive a patch");
        assert!(server.project.midi_input.is_empty());
    }

    /// リモートの編集で、安全の設定(レーザーの範囲・保護された機材・校正・ARM の割り当て)は、
    /// 許されたときだけ変えられる
    #[test]
    fn safety_settings_are_guarded_unless_allowed() {
        let laser = |c: &Core| c.project.fixtures.iter().position(|f| f.profile == "stage-evolution/laserscan300").unwrap();
        let edits = |c: &Core| -> Vec<Vec<Change>> {
            let i = laser(c);
            let mut bindings = serde_json::to_value(&c.project.bindings).unwrap();
            bindings.as_array_mut().unwrap().push(json!({"id": 0, "name": "ARM", "trigger": null, "action": {"Arm": {"sel": "All", "mode": "Toggle"}}}));
            vec![
                vec![Change { path: vec![Seg::Key("fixtures".into()), Seg::Index(i), Seg::Key("limits".into())], value: Some(json!({"pan": [0, 255]})), base: None }],
                vec![Change { path: vec![Seg::Key("motion".into()), Seg::Key("pan_front".into())], value: Some(json!(90.0)), base: None }],
                vec![Change { path: vec![Seg::Key("bindings".into())], value: Some(bindings), base: None }],
                vec![Change { path: vec![Seg::Key("fixtures".into()), Seg::Index(i), Seg::Key("address".into())], value: Some(json!(300)), base: None }],
            ]
        };
        let mut c = core();
        let before = serde_json::to_value(&c.project).unwrap();
        for e in edits(&c) {
            let err = c.apply_project_patch(&e, true).unwrap_err();
            assert!(err.is::<Rejected>() && err.to_string().contains("--remote-allow-safety-edit"), "{e:?}: {err}");
        }
        assert_eq!(serde_json::to_value(&c.project).unwrap(), before, "拒んだ編集で何も変わらない");
        // 安全に関わらない編集(レーザーの名前・値)は今までどおり
        let i = laser(&c);
        c.apply_project_patch(&[Change { path: vec![Seg::Key("fixtures".into()), Seg::Index(i), Seg::Key("name".into())], value: Some(json!("L")), base: None }], true).unwrap();
        // 許されたときは変えられる
        let mut c = core();
        for e in edits(&c) {
            c.apply_project_patch(&e, false).unwrap_or_else(|err| panic!("{e:?}: {err}"));
        }
        assert_eq!(c.project.fixtures[laser(&c)].limits.get("pan"), Some(&[0, 255]));
        assert_eq!(c.project.motion.pan_front, 90.0);
    }

    /// 存在しない項目を指す編集は、黙って捨てずに拒む
    #[test]
    fn edits_to_missing_fields_are_rejected() {
        let mut c = core();
        let change = |path: Vec<Seg>, value: Option<serde_json::Value>| vec![Change { path, value, base: None }];
        for e in [change(vec![Seg::Key("nope".into())], Some(json!(1))), change(vec![Seg::Key("fixtures".into()), Seg::Index(0), Seg::Key("nope".into())], Some(json!(1)))] {
            let err = c.apply_project_patch(&e, true).unwrap_err();
            assert!(err.is::<Rejected>() && err.to_string().starts_with("存在しない項目です"), "{err}");
        }
        // 値の型が合わないものも、理由を付けて拒む
        let err = c.apply_project_patch(&change(vec![Seg::Key("bpm".into())], Some(json!("fast"))), true).unwrap_err();
        assert!(err.is::<Rejected>(), "{err}");
        // 実在する項目は通る
        c.apply_project_patch(&change(vec![Seg::Key("bpm".into())], Some(json!(128.0))), true).unwrap();
        assert_eq!(c.project.bpm, 128.0);
    }

    /// 保護された機材のアドレスに別の機材を重ねる編集と、割り当ての ID を重ねて ARM の割り当てを差し替える編集は拒む
    #[test]
    fn overlaps_and_duplicate_binding_ids_cannot_bypass_safety() {
        let mut c = core();
        let laser = c.project.fixtures.iter().position(|f| f.profile == "stage-evolution/laserscan300").unwrap();
        let other = c.project.fixtures.iter().position(|f| f.profile != c.project.fixtures[laser].profile).unwrap();
        let (universe, address) = (c.project.fixtures[laser].universe, c.project.fixtures[laser].address);
        let onto_laser = vec![
            Change { path: vec![Seg::Key("fixtures".into()), Seg::Index(other), Seg::Key("universe".into())], value: Some(json!(universe)), base: None },
            Change { path: vec![Seg::Key("fixtures".into()), Seg::Index(other), Seg::Key("address".into())], value: Some(json!(address)), base: None },
        ];
        let err = c.apply_project_patch(&onto_laser, true).unwrap_err();
        assert!(err.is::<Rejected>() && err.to_string().contains("overlap"), "{err}");

        // サーバーの画面で作った ARM の割り当てと同じ ID で、別の操作に割り当てた ARM を先に置く
        let mut bindings = serde_json::to_value(&c.project.bindings).unwrap();
        bindings.as_array_mut().unwrap().push(json!({"id": 0, "name": "ARM", "trigger": null, "osc": "/arm", "action": {"Arm": {"sel": "All", "mode": "Toggle"}}}));
        c.apply_project_patch(&[Change { path: vec![Seg::Key("bindings".into())], value: Some(bindings), base: None }], false).unwrap();
        let before = serde_json::to_value(&c.project.bindings).unwrap();
        let mut bindings = before.clone();
        let arm = c.project.bindings.iter().find(|b| matches!(b.action, Action::Arm { .. })).expect("ARM の割り当て");
        let mut fake = serde_json::to_value(arm).unwrap();
        fake["name"] = json!("差し替え");
        fake["osc"] = json!("/fake/arm");
        bindings.as_array_mut().unwrap().insert(0, fake);
        let err = c.apply_project_patch(&[Change { path: vec![Seg::Key("bindings".into())], value: Some(bindings), base: None }], true).unwrap_err();
        assert!(err.is::<Rejected>(), "{err}");
        assert_eq!(serde_json::to_value(&c.project.bindings).unwrap(), before);

        // 機材の ID の重なりと、保護された機材の値の数の違いも拒む
        let mut fixtures = serde_json::to_value(&c.project.fixtures).unwrap();
        let mut copy = fixtures[other].clone();
        copy["id"] = fixtures[laser]["id"].clone();
        fixtures.as_array_mut().unwrap().insert(0, copy);
        let err = c.apply_project_patch(&[Change { path: vec![Seg::Key("fixtures".into())], value: Some(fixtures), base: None }], true).unwrap_err();
        assert!(err.to_string().contains("duplicate fixture id"), "{err}");
        let short = [Change { path: vec![Seg::Key("fixtures".into()), Seg::Index(laser), Seg::Key("values".into())], value: Some(json!([0])), base: None }];
        let err = c.apply_project_patch(&short, true).unwrap_err();
        assert!(err.to_string().contains("values length"), "{err}");

        // 許されたときは変えられる
        c.apply_project_patch(&onto_laser, false).unwrap();
        // もともと重なっている組でも、重なるチャンネルを増やす編集は拒む
        let shift = [Change { path: vec![Seg::Key("fixtures".into()), Seg::Index(other), Seg::Key("address".into())], value: Some(json!(address + 1)), base: None }];
        c.apply_project_patch(&shift, false).unwrap();
        let back = [Change { path: vec![Seg::Key("fixtures".into()), Seg::Index(other), Seg::Key("address".into())], value: Some(json!(address)), base: None }];
        let err = c.apply_project_patch(&back, true).unwrap_err();
        assert!(err.to_string().contains("overlap"), "{err}");
    }

    /// 存在しない項目を消す編集と、置き換える値の中の存在しない項目も拒む
    #[test]
    fn removing_or_nesting_unknown_fields_is_rejected() {
        let mut c = core();
        let before = serde_json::to_value(&c.project).unwrap();
        let remove = [Change { path: vec![Seg::Key("fixtures".into()), Seg::Index(0), Seg::Key("nope".into())], value: None, base: None }];
        let err = c.apply_project_patch(&remove, true).unwrap_err();
        assert!(err.to_string().contains("fixtures.0.nope"), "{err}");
        let mut fixture = before["fixtures"][0].clone();
        fixture["nope"] = json!(1);
        let replace = [Change { path: vec![Seg::Key("fixtures".into()), Seg::Index(0)], value: Some(fixture), base: None }];
        let err = c.apply_project_patch(&replace, true).unwrap_err();
        assert!(err.is::<Rejected>() && err.to_string().contains("fixtures.0.nope"), "{err}");
        assert_eq!(serde_json::to_value(&c.project).unwrap(), before);
        // 未知の項目は、空の値でも拒む
        for empty in [json!(null), json!(false), json!([]), json!({})] {
            let mut fixture = before["fixtures"][0].clone();
            fixture["nope"] = empty;
            let err = c.apply_project_patch(&[Change { path: vec![Seg::Key("fixtures".into()), Seg::Index(0)], value: Some(fixture), base: None }], true).unwrap_err();
            assert!(err.to_string().contains("fixtures.0.nope"), "{err}");
        }
        // 書き出すときに省く空の項目(limits の空の表)は、あってもよい
        let mut fixture = before["fixtures"][0].clone();
        fixture["limits"] = json!({});
        c.apply_project_patch(&[Change { path: vec![Seg::Key("fixtures".into()), Seg::Index(0)], value: Some(fixture), base: None }], true).unwrap();
    }

    #[test]
    fn service_settings_ask_for_apply_services() {
        let mut c = core();
        let before = serde_json::to_value(&c.project).unwrap();
        let mut p = c.project.clone();
        p.osc_port = 9100;
        let changes = diff_project(&before, &serde_json::to_value(&p).unwrap());
        assert!(c.apply_project_patch(&changes, true).unwrap().services);
        assert_eq!(c.project.osc_port, 9100);
        let local = [Change { path: vec![Seg::Key("midi_input".into())], value: Some(json!("x")), base: None }];
        assert!(c.apply_project_patch(&local, true).is_err());
    }
}
