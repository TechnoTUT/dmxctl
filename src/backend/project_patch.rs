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

use crate::core::project::Project;
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
    fn apply_project_patch(&mut self, changes: &[Change]) -> anyhow::Result<Applied>;
}

impl CoreProjectPatch for crate::core::engine::Core {
    /// 画面の編集の差分を当てる(厳しい検証はしない)。当てられなければ何も変えない
    fn apply_project_patch(&mut self, changes: &[Change]) -> anyhow::Result<Applied> {
        if changes.is_empty() {
            return Ok(Applied::default());
        }
        for c in changes {
            if let Some(Seg::Key(k)) = c.path.first() {
                anyhow::ensure!(!LOCAL_KEYS.contains(&k.as_str()), "{k} is local to each machine");
            }
        }
        let mut value = serde_json::to_value(&self.project)?;
        apply(&mut value, changes)?;
        let project: Project = serde_json::from_value(value)?;
        let services = changes.iter().any(|c| matches!(c.path.first(), Some(Seg::Key(k)) if SERVICE_KEYS.contains(&k.as_str())));
        // ARM(保存しない値)は機材IDで引き継ぎ、フェード・チェイス・出力などの後始末をする
        crate::backend::editing::commit_with_arm(self, project, true);
        Ok(Applied { services })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::engine::Core;
    use crate::core::fixture::Library;
    use serde_json::json;
    use std::sync::Arc;

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
        let applied = server.apply_project_patch(&changes).unwrap();
        assert_eq!(applied, Applied { services: false });
        assert_eq!(server.project.fixtures[0].values[0], 200);
        assert!(server.project.fixtures[0].armed, "ARM must survive a patch");
        assert!(server.project.midi_input.is_empty());
    }

    #[test]
    fn service_settings_ask_for_apply_services() {
        let mut c = core();
        let before = serde_json::to_value(&c.project).unwrap();
        let mut p = c.project.clone();
        p.osc_port = 9100;
        let changes = diff_project(&before, &serde_json::to_value(&p).unwrap());
        assert!(c.apply_project_patch(&changes).unwrap().services);
        assert_eq!(c.project.osc_port, 9100);
        let local = [Change { path: vec![Seg::Key("midi_input".into())], value: Some(json!("x")), base: None }];
        assert!(c.apply_project_patch(&local).is_err());
    }
}
