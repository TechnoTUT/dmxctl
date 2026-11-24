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

/// 1か所の変更。`value` が無ければ、そのキーを消す(None のときに書き出さない項目など)。
/// 知らないキー(`value` の打ち間違いなど)は、消す編集として通さずに拒む
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Change {
    pub path: Vec<Seg>,
    /// `"value": null` は「null にする」。キーが無いときだけ「消す」
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    pub value: Option<Value>,
    /// 変える前の値(無いキーは null)。あれば、当てる先の今の値がこれと同じときだけ当てる。
    /// `"base": null`(前は無かった)も、比べる値として残す
    #[serde(default, skip_serializing_if = "Option::is_none", deserialize_with = "present")]
    pub base: Option<Value>,
}

/// キーがあれば、値が null でも Some にする(serde の Option は null を None にしてしまう)
fn present<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
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

/// 設定(Project)の中で、空のときに書き出さない項目(`skip_serializing_if`)か。
/// 機材の limits の空の表と、割り当ての page・osc の null だけ
fn omitted_when_empty(path: &[Seg], value: &Value) -> bool {
    match path {
        [.., Seg::Key(list), Seg::Index(_), Seg::Key(key)] => match (list.as_str(), key.as_str()) {
            ("fixtures", "limits") => value.as_object().is_some_and(|o| o.is_empty()),
            ("bindings", "page" | "osc") => value.is_null(),
            _ => false,
        },
        _ => false,
    }
}

/// 書き込んだ値のうち、設定に読み直したときに捨てられた項目(打ち間違い・古い版の項目)の場所。
/// 書き出すときに空なら省く項目(`omitted_when_empty`)だけは、無くてもよい
fn dropped_key(given: &Value, kept: &Value, path: &mut Vec<Seg>) -> bool {
    match (given, kept) {
        (Value::Object(g), Value::Object(k)) => g.iter().any(|(key, gv)| {
            path.push(Seg::Key(key.clone()));
            let dropped = match k.get(key) {
                Some(kv) => dropped_key(gv, kv, path),
                None => !omitted_when_empty(path, gv),
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
            // 全体の置き換え(空のパス)は、各機の項目の確認とサービスの設定し直しの判定を飛ばしてしまうので受け付けない
            let Some(Seg::Key(k)) = c.path.first() else {
                return Err(Rejected("編集の場所(path)が正しくありません".into()).into());
            };
            anyhow::ensure!(!LOCAL_KEYS.contains(&k.as_str()), "{k} is local to each machine");
        }
        // 存在しない項目(打ち間違い・古い版の項目)は、書き込んでも設定に読み直すときに捨てられ、
        // 黙って何も変わらないので、ここで見つけて拒む
        let missing = |path: &[Seg]| Err(Rejected(format!("存在しない項目です: {}", path_text(path))).into());
        let mut value = serde_json::to_value(&self.project)?;
        for c in changes {
            // 途中の場所が無いとき: 前の値(base)があれば、ほかの編集との行き違い。無ければ存在しない項目
            if lookup(&value, &c.path[..c.path.len() - 1]).is_none() {
                return if c.base.is_some() { Err(Conflict.into()) } else { missing(&c.path) };
            }
            // 消す編集は、今その項目があること。消す前の値(base)があるときは、行き違い(Conflict)として返す
            if c.value.is_none() && c.base.as_ref().is_none_or(|b| b.is_null()) && lookup(&value, &c.path).is_none() {
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
        // 機材定義が無い機材を足すと、それ以降の編集でモードを引けなくなる。もとからある機材(すでに壊れた参照)はそのまま認め、消せるようにする
        if let Some(f) = project.fixtures.iter().find(|f| self.lib.mode(&f.profile, f.mode).is_none()
            && !self.project.fixtures.iter().any(|o| o.id == f.id && o.profile == f.profile && o.mode == f.mode)) {
            return Err(Rejected(format!("機材定義が見つかりません: {} ({} のモード {})", f.name, f.profile, f.mode)).into());
        }
        // 割り当ての ID の重複・欠けは、反映するときに付け直す。安全の確認は付け直した後の設定で行う
        project.next_binding_id = project.next_binding_id.max(self.project.next_binding_id);
        project.ensure_binding_ids();
        // 自動操作の窓口(ctl・MCP)と同じ安全の確認。レーザーの範囲・保護された機材・校正・ARM の割り当てを守る
        // 送れない出力(範囲外の Art-Net のユニバースなど)は、送るときに番号が変わってしまうので先に拒む
        crate::editing::validate_outputs(&project).map_err(|e| Rejected(format!("出力の設定が正しくありません: {e}")))?;
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

    /// 3回目のレビューの経路: 出力の経路・重なりの並び順・安全な値がある機材・ARM の対象のグループ
    #[test]
    fn routes_order_safe_values_and_arm_targets_are_guarded() {
        let at = |path: &[&str], i: Option<usize>| {
            let mut p: Vec<Seg> = path.iter().map(|k| Seg::Key((*k).into())).collect();
            if let Some(i) = i { p.insert(1, Seg::Index(i)); }
            p
        };
        let set = |path: Vec<Seg>, value: serde_json::Value| vec![Change { path, value: Some(value), base: None }];
        let mut c = core();
        let laser = c.project.fixtures.iter().position(|f| f.profile == "stage-evolution/laserscan300").unwrap();
        let universe = c.project.fixtures[laser].universe;
        // 保護された機材のユニバースを送る出力は変えられない。別のユニバースの出力は足せる
        let mut outputs = serde_json::to_value(&c.project.outputs).unwrap();
        outputs.as_array_mut().unwrap().push(json!({"enabled": true, "universe": universe, "kind": {"ArtNet": {"ip": "10.0.0.9", "art_universe": 5}}}));
        let err = c.apply_project_patch(&set(at(&["outputs"], None), outputs), true).unwrap_err();
        assert!(err.to_string().contains("outputs that reach protected fixtures"), "{err}");
        let mut outputs = serde_json::to_value(&c.project.outputs).unwrap();
        outputs.as_array_mut().unwrap().push(json!({"enabled": true, "universe": 3, "kind": {"ArtNet": {"ip": "10.0.0.9", "art_universe": 5}}}));
        c.apply_project_patch(&set(at(&["outputs"], None), outputs), true).unwrap();

        // 安全な値がある機材(ムービング)のアドレスに、別の機材を重ねられない。パッチ自体は変えられる
        let spot = c.project.fixtures.iter().position(|f| f.profile == "stage-evolution/minispot30").unwrap();
        let lib = c.lib.clone();
        c.project.add_fixture(&lib, "glint/glp-185l").unwrap();
        let other = c.project.fixtures.len() - 1;
        let (u, a) = (c.project.fixtures[spot].universe, c.project.fixtures[spot].address);
        let onto_spot = [set(at(&["fixtures", "universe"], Some(other)), json!(u)), set(at(&["fixtures", "address"], Some(other)), json!(a))].concat();
        let err = c.apply_project_patch(&onto_spot, true).unwrap_err();
        assert!(err.to_string().contains("overlap"), "{err}");
        c.apply_project_patch(&set(at(&["fixtures", "name"], Some(spot)), json!("Spot")), true).unwrap();

        // もともと重なっていても、並び順を入れ替えると上書きする側が変わるので拒む
        c.apply_project_patch(&onto_spot, false).unwrap();
        let mut fixtures = serde_json::to_value(&c.project.fixtures).unwrap();
        fixtures.as_array_mut().unwrap().swap(spot, other);
        let err = c.apply_project_patch(&set(at(&["fixtures"], None), fixtures), true).unwrap_err();
        assert!(err.to_string().contains("overlap"), "{err}");

        // 4回目のレビュー: 安全な値がある機材の値の数、使わない機材を使い始めて重ねる、sACN の同じ送り先
        let err = c.apply_project_patch(&set(at(&["fixtures", "values"], Some(spot)), json!([0])), true).unwrap_err();
        assert!(err.to_string().contains("values length"), "{err}");
        let mut c2 = core();
        c2.project.add_fixture(&lib, "glint/glp-185l").unwrap();
        let other2 = c2.project.fixtures.len() - 1;
        let spot2 = c2.project.fixtures.iter().position(|f| f.profile == "stage-evolution/minispot30").unwrap();
        let (u2, a2) = (c2.project.fixtures[spot2].universe, c2.project.fixtures[spot2].address);
        c2.apply_project_patch(&[set(at(&["fixtures", "enabled"], Some(other2)), json!(false)), set(at(&["fixtures", "universe"], Some(other2)), json!(u2)), set(at(&["fixtures", "address"], Some(other2)), json!(a2))].concat(), false).unwrap();
        let err = c2.apply_project_patch(&set(at(&["fixtures", "enabled"], Some(other2)), json!(true)), true).unwrap_err();
        assert!(err.to_string().contains("overlap"), "{err}");
        let laser_universe = c2.project.fixtures.iter().find(|f| f.profile == "stage-evolution/laserscan300").unwrap().universe;
        let mut outputs = serde_json::to_value(&c2.project.outputs).unwrap();
        outputs.as_array_mut().unwrap().push(json!({"enabled": true, "universe": laser_universe, "kind": {"Sacn": {"ip": "10.0.0.7", "sacn_universe": 1, "priority": 100}}}));
        c2.apply_project_patch(&set(at(&["outputs"], None), outputs), false).unwrap();
        let mut outputs = serde_json::to_value(&c2.project.outputs).unwrap();
        outputs.as_array_mut().unwrap().push(json!({"enabled": true, "universe": 3, "kind": {"Sacn": {"ip": "10.0.0.7", "sacn_universe": 1, "priority": 50}}}));
        let err = c2.apply_project_patch(&set(at(&["outputs"], None), outputs), true).unwrap_err();
        assert!(err.to_string().contains("outputs that reach protected fixtures"), "{err}");

        // グループで ARM の対象を選ぶ割り当てがあるとき、グループの中身を変えて対象を増やせない
        let mut bindings = serde_json::to_value(&c.project.bindings).unwrap();
        bindings.as_array_mut().unwrap().push(json!({"id": 0, "name": "ARM", "trigger": null, "action": {"Arm": {"sel": {"Group": "G"}, "mode": "Toggle"}}}));
        c.apply_project_patch(&[set(at(&["groups"], None), json!([{"name": "G", "fixtures": []}])), set(at(&["bindings"], None), bindings)].concat(), false).unwrap();
        let laser_id = c.project.fixtures[laser].id;
        let err = c.apply_project_patch(&set(at(&["groups", "fixtures"], Some(0)), json!([laser_id])), true).unwrap_err();
        assert!(err.to_string().contains("armed by ARM bindings"), "{err}");
    }

    /// 機材定義の無い機材・場所の違う省略・先に消された項目の扱い
    #[test]
    fn unknown_profiles_misplaced_omissions_and_deleted_items() {
        let mut c = core();
        // 機材定義の無い機材は足せない
        let mut fixtures = serde_json::to_value(&c.project.fixtures).unwrap();
        let mut ghost = fixtures[0].clone();
        ghost["id"] = json!(999);
        ghost["profile"] = json!("nobody/ghost");
        fixtures.as_array_mut().unwrap().push(ghost);
        let err = c.apply_project_patch(&[Change { path: vec![Seg::Key("fixtures".into())], value: Some(fixtures.clone()), base: None }], true).unwrap_err();
        assert!(err.to_string().contains("nobody/ghost"), "{err}");
        // すでに壊れた参照の機材は、そのままほかの編集ができて、消すこともできる
        let mut broken = c.project.fixtures[0].clone();
        broken.id = 999;
        broken.profile = "nobody/ghost".into();
        c.project.fixtures.push(broken);
        c.apply_project_patch(&[Change { path: vec![Seg::Key("bpm".into())], value: Some(json!(130.0)), base: None }], true).unwrap();
        let mut fixtures = serde_json::to_value(&c.project.fixtures).unwrap();
        fixtures.as_array_mut().unwrap().pop();
        c.apply_project_patch(&[Change { path: vec![Seg::Key("fixtures".into())], value: Some(fixtures), base: None }], true).unwrap();
        assert!(c.project.fixtures.iter().all(|f| f.id != 999));

        // 省略してよいのは、割り当ての page・osc の null と、機材の limits の空の表だけ
        let mut fixture = serde_json::to_value(&c.project.fixtures[0]).unwrap();
        fixture["page"] = json!(null);
        let err = c.apply_project_patch(&[Change { path: vec![Seg::Key("fixtures".into()), Seg::Index(0)], value: Some(fixture), base: None }], true).unwrap_err();
        assert!(err.to_string().contains("fixtures.0.page"), "{err}");
        if !c.project.bindings.is_empty() {
            let mut binding = serde_json::to_value(&c.project.bindings[0]).unwrap();
            binding["page"] = json!(null);
            binding["osc"] = json!(null);
            c.apply_project_patch(&[Change { path: vec![Seg::Key("bindings".into()), Seg::Index(0)], value: Some(binding), base: None }], true).unwrap();
        }

        // 消す前の値があるのに、ほかのクライアントが先に消していたら、行き違い(Conflict)として返す
        let gone = [Change { path: vec![Seg::Key("fixtures".into()), Seg::Index(0), Seg::Key("limits".into())], value: None, base: Some(json!({"pan": [0, 10]})) }];
        assert!(c.apply_project_patch(&gone, true).unwrap_err().is::<Conflict>());
    }

    /// 5回目のレビュー: 送り先の規則・範囲の新設・実際に書く機材・ARM の並び順・差分の形式
    #[test]
    fn destinations_limits_writers_order_and_change_format() {
        let k = |path: &[&str], i: Option<usize>| {
            let mut p: Vec<Seg> = path.iter().map(|k| Seg::Key((*k).into())).collect();
            if let Some(i) = i { p.insert(1, Seg::Index(i)); }
            p
        };
        let set = |path: Vec<Seg>, value: serde_json::Value| vec![Change { path, value: Some(value), base: None }];
        let mut c = core();
        let lib = c.lib.clone();
        let laser_u = c.project.fixtures.iter().find(|f| f.profile == "stage-evolution/laserscan300").unwrap().universe;
        // sACN: マルチキャスト(IP なし)と、同じマルチキャストの IP を指定したものは同じ送り先
        let mut outputs = serde_json::to_value(&c.project.outputs).unwrap();
        outputs.as_array_mut().unwrap().push(json!({"enabled": true, "universe": laser_u, "kind": {"Sacn": {"ip": "", "sacn_universe": 1, "priority": 100}}}));
        c.apply_project_patch(&set(k(&["outputs"], None), outputs), false).unwrap();
        let mut outputs = serde_json::to_value(&c.project.outputs).unwrap();
        outputs.as_array_mut().unwrap().push(json!({"enabled": true, "universe": 3, "kind": {"Sacn": {"ip": "239.255.0.1", "sacn_universe": 1, "priority": 100}}}));
        let err = c.apply_project_patch(&set(k(&["outputs"], None), outputs), true).unwrap_err();
        assert!(err.to_string().contains("outputs that reach protected fixtures"), "{err}");
        // Art-Net: 範囲外のユニバースは、許可があっても拒む(送るときに番号が変わるため)
        let mut outputs = serde_json::to_value(&c.project.outputs).unwrap();
        outputs.as_array_mut().unwrap().push(json!({"enabled": true, "universe": 3, "kind": {"ArtNet": {"ip": "10.0.0.1", "art_universe": 40000}}}));
        assert!(c.apply_project_patch(&set(k(&["outputs"], None), outputs), false).unwrap_err().to_string().contains("Art-Net"));

        // 範囲が無い機材に、最初の範囲を付けることもできない
        let spot = c.project.fixtures.iter().position(|f| f.profile == "stage-evolution/minispot30").unwrap();
        let err = c.apply_project_patch(&set(k(&["fixtures", "limits"], Some(spot)), json!({"dimmer": [10, 255]})), true).unwrap_err();
        assert!(err.to_string().contains("limits"), "{err}");

        // もともと重なっていて、値の数が違うので書いていなかった機材を直すと、書き始めるので拒む
        c.project.add_fixture(&lib, "glint/glp-185l").unwrap();
        let bar = c.project.fixtures.len() - 1;
        let (u, a) = (c.project.fixtures[spot].universe, c.project.fixtures[spot].address);
        let good = c.project.fixtures[bar].values.clone();
        c.apply_project_patch(&[set(k(&["fixtures", "universe"], Some(bar)), json!(u)), set(k(&["fixtures", "address"], Some(bar)), json!(a)),
            set(k(&["fixtures", "values"], Some(bar)), json!([0]))].concat(), false).unwrap();
        let err = c.apply_project_patch(&set(k(&["fixtures", "values"], Some(bar)), json!(good)), true).unwrap_err();
        assert!(err.to_string().contains("overlap") || err.to_string().contains("values length"), "{err}");

        // ARM の割り当ての並び替えも拒む
        let mut c = core();
        let mut bindings = serde_json::to_value(&c.project.bindings).unwrap();
        for (name, mode) in [("ARM トグル", "Toggle"), ("ARM ワンショット", "Trigger")] {
            bindings.as_array_mut().unwrap().push(json!({"id": 0, "name": name, "trigger": null, "osc": "/arm", "action": {"Arm": {"sel": "All", "mode": mode}}}));
        }
        c.apply_project_patch(&set(k(&["bindings"], None), bindings), false).unwrap();
        let mut bindings = serde_json::to_value(&c.project.bindings).unwrap();
        let n = bindings.as_array().unwrap().len();
        bindings.as_array_mut().unwrap().swap(n - 1, n - 2);
        assert!(c.apply_project_patch(&set(k(&["bindings"], None), bindings), true).is_err());

        // 差分の形式: 知らないキー・空のパスを拒む。base: null は比べる値として残り、行き違いになる
        assert!(serde_json::from_value::<Change>(json!({"path": ["bpm"], "valeu": 120})).is_err());
        let whole = [Change { path: vec![], value: Some(serde_json::to_value(&c.project).unwrap()), base: None }];
        assert!(c.apply_project_patch(&whole, true).unwrap_err().is::<Rejected>());
        let change: Change = serde_json::from_value(json!({"path": ["bpm"], "value": 130.0, "base": null})).unwrap();
        assert_eq!(change.base, Some(json!(null)));
        assert!(c.apply_project_patch(&[change], true).unwrap_err().is::<Conflict>(), "bpm は前から有る");
        let err = c.apply_project_patch(&set(k(&["nowhere", "x"], None), json!(1)), true).unwrap_err();
        assert!(err.is::<Rejected>(), "{err}");
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
