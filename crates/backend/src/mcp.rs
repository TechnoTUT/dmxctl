//! MCP (Model Context Protocol) のサーバー。`dmxctl mcp` で起動する。
//! AI のクライアント(Claude Code など)と標準入出力で JSON-RPC をやりとりし、受けた操作を、
//! 動いている本体(`dmxctl serve`、または `--control` を付けた画面版)の制御の窓口へ中継する。
//!
//! - 操作の検証・安全の処理は本体の窓口が行う。ここは窓口の操作を、ツールの形に並べ直すだけ。
//!   ARM・安全設定の変更・レーザーの合図は、窓口と同じく実行できない
//! - 本体の停止・記録のファイル・設定の読み込み直しは、ツールにしない
//! - 実機へ送るかどうかは本体の起動オプション(`--output`)で決まり、ここからは変えられない
//! - MCP の接続を閉じても、本体は止まらない
use crate::diagnostics::Requester;
use anyhow::{Context, bail, ensure};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;
use std::time::Duration;

/// 対応する MCP の版。クライアントが求めた版がこの中にあれば、それで答える
const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2024-11-05"];
const MAX_WAITS: usize = 4;

const INSTRUCTIONS: &str = "dmxctl の照明を操作する。まず status で状態を、fixture_list で機材と属性を確認する。\
値は DMX(0〜255)。変更の後は output_check で、安全の処理を通った最終の出力を確かめる。\
ARM・安全設定の変更・レーザーの合図はできない。実機へ送るかは本体の起動オプションで決まる。";

/// `dmxctl mcp [--connect 127.0.0.1:9190]`
pub fn run(args: &[String]) -> anyhow::Result<()> {
    let address = match args {
        [] => crate::control::DEFAULT_ADDRESS.to_string(),
        [flag, address] if flag == "--connect" => address.clone(),
        _ => bail!("usage: dmxctl mcp [--connect 127.0.0.1:9190]"),
    };
    let address: std::net::SocketAddr = address.parse().context("--connect needs IP:PORT")?;
    ensure!(address.ip().is_loopback(), "control address must be loopback");
    let request = move |payload: &Value, timeout: Duration| crate::control::request_with_timeout(address, payload, timeout);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    serve(&request, stdin.lock(), stdout)
}

/// 1行に1つの JSON-RPC のメッセージを読み、答えを1行ずつ書く。入力が閉じたら終わる
pub fn serve(
    request: &(impl Fn(&Value, Duration) -> anyhow::Result<Value> + Sync),
    input: impl BufRead,
    output: impl Write + Send,
) -> anyhow::Result<()> {
    let output = Mutex::new(output);
    let pending = Mutex::new(BTreeMap::<String, Arc<AtomicBool>>::new());
    thread::scope(|scope| {
        let mut workers: Vec<thread::ScopedJoinHandle<'_, std::io::Result<()>>> = Vec::new();
        let input_result = (|| -> anyhow::Result<()> {
            for line in input.lines() {
                let line = line?;
                for worker in workers.extract_if(.., |worker| worker.is_finished()) {
                    worker.join().expect("MCP wait worker panicked")?;
                }
                if line.trim().is_empty() {
                    continue;
                }
                let message = match serde_json::from_str::<Value>(&line) {
                    Ok(message) => message,
                    Err(e) => {
                        write_reply(&output, error(Value::Null, -32700, &format!("parse error: {e}")))?;
                        continue;
                    }
                };
                if message["jsonrpc"] == "2.0" && message["method"] == "notifications/cancelled" && message.get("id").is_none() {
                    let request_id = message["params"]["requestId"].to_string();
                    if let Some(cancelled) = pending.lock().unwrap().get(&request_id) {
                        cancelled.store(true, Ordering::Relaxed);
                    }
                    continue;
                }
                if message["jsonrpc"] == "2.0" && message["method"] == "tools/call" && is_waiting_tool(&message["params"]["name"])
                    && let Some(id) = message.get("id")
                {
                    let key = id.to_string();
                    let mut waits = pending.lock().unwrap();
                    if waits.contains_key(&key) {
                        write_reply(&output, error(id.clone(), -32600, "request ID is already in use"))?;
                        continue;
                    }
                    if waits.len() >= MAX_WAITS {
                        write_reply(&output, tool_reply(id.clone(), Err(anyhow::anyhow!("too many pending wait/explain requests"))))?;
                        continue;
                    }
                    let cancelled = Arc::new(AtomicBool::new(false));
                    waits.insert(key.clone(), Arc::clone(&cancelled));
                    let pending = &pending;
                    let output = &output;
                    // 待機中も、後続の操作やキャンセルを標準入力から受け付ける
                    workers.push(thread::Builder::new().spawn_scoped(scope, move || {
                        let poll = |payload: &Value, timeout: Duration| {
                            // キャンセル後も、本体の枠を空ける後片付け(trace_cancel)だけは送る
                            ensure!(!cancelled.load(Ordering::Relaxed) || payload["op"] == "trace_cancel", "wait cancelled");
                            // 本体のトレースの枠は1つ。キャンセル済みのワーカーが後片付けを終える前に始めると、
                            // another trace is pending になるので、終わるのを待ってから始める
                            if payload["op"] == "trace_start" {
                                wait_for_cancelled_workers(pending, &key);
                            }
                            request(payload, timeout)
                        };
                        let reply = handle(&poll, &message);
                        let mut waits = pending.lock().unwrap();
                        waits.remove(&key);
                        if let Some(reply) = reply && !cancelled.load(Ordering::Relaxed) {
                            write_reply(output, reply)?;
                        }
                        Ok(())
                    })?);
                } else if let Some(reply) = handle(request, &message) {
                    write_reply(&output, reply)?;
                }
            }
            Ok(())
        })();
        for cancelled in pending.lock().unwrap().values() {
            cancelled.store(true, Ordering::Relaxed);
        }
        input_result?;
        for worker in workers {
            worker.join().expect("MCP wait worker panicked")?;
        }
        Ok(())
    })
}

/// キャンセル済みのワーカー(自分以外)が後片付けを終えて抜けるまで待つ。
/// ワーカーは次の問い合わせの前にキャンセルに気づき、後片付けを送って抜けるので、普通はすぐ終わる。
/// 本体が応答しないときに固まらないよう、上限を設ける(本体への1回の問い合わせの上限と同じ6秒)
fn wait_for_cancelled_workers(pending: &Mutex<BTreeMap<String, Arc<AtomicBool>>>, me: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(6);
    while pending.lock().unwrap().iter().any(|(key, cancelled)| key != me && cancelled.load(Ordering::Relaxed)) {
        if std::time::Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(2));
    }
}

/// 待つことのあるツール。別のスレッドで処理し、その間も後続の要求とキャンセルを受け付ける
fn is_waiting_tool(name: &Value) -> bool {
    name == "wait" || name == "explain"
}

fn write_reply(output: &Mutex<impl Write>, reply: Value) -> std::io::Result<()> {
    let mut output = output.lock().unwrap();
    writeln!(output, "{reply}")?;
    output.flush()
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_reply(id: Value, result: anyhow::Result<Value>) -> Value {
    let result = match result {
        Ok(value) => json!({"content": [{"type": "text", "text": value.to_string()}], "isError": false}),
        Err(e) => json!({"content": [{"type": "text", "text": format!("{e:#}")}], "isError": true}),
    };
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

/// 1つのメッセージを処理する。通知(id の無いもの)には答えない
pub fn handle(request: Requester, message: &Value) -> Option<Value> {
    if !message.is_object() {
        return Some(error(Value::Null, -32600, "invalid request"));
    }
    let id = message.get("id").cloned();
    let method = message["method"].as_str().unwrap_or("");
    let Some(id) = id else {
        // 通知には応答しない
        return None;
    };
    if message["jsonrpc"] != "2.0" || method.is_empty() {
        return Some(error(id, -32600, "invalid request"));
    }
    let result = match method {
        "initialize" => {
            let wanted = message["params"]["protocolVersion"].as_str().unwrap_or("");
            let version = PROTOCOL_VERSIONS.iter().find(|v| **v == wanted).unwrap_or(&PROTOCOL_VERSIONS[0]);
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "dmxctl", "version": env!("CARGO_PKG_VERSION")},
                "instructions": INSTRUCTIONS,
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let name = match message["params"].get("name") {
                None => return Some(error(id, -32602, "params.name is required")),
                Some(v) => match v.as_str() {
                    Some(name) => name,
                    None => return Some(error(id, -32602, "params.name must be a string")),
                },
            };
            let args = message["params"].get("arguments").cloned().unwrap_or_else(|| json!({}));
            if !tools().iter().any(|t| t["name"] == name) {
                return Some(error(id, -32602, &format!("unknown tool: {name}")));
            }
            if !args.is_object() {
                return Some(error(id, -32602, "arguments must be an object"));
            }
            // 本体の窓口が拒んだ・つながらないなどは、ツールの失敗として返す(AI が読んで直せるように)
            return Some(tool_reply(id, call(request, name, &args)));
        }
        _ => return Some(error(id, -32601, &format!("method not found: {method}"))),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {"type": "object", "properties": properties, "required": required, "additionalProperties": false},
    })
}

/// AI に見せる操作の一覧
fn tools() -> Vec<Value> {
    let fixture = json!({"type": "integer", "minimum": 0, "description": "機材の ID(fixture_list の id)"});
    let attr = json!({"type": "string", "description": "属性(pan・tilt・dimmer・red など。fixture_list で確認)"});
    let condition = json!({
        "fixture": fixture,
        "attr": attr,
        "equals": {"type": "number", "description": "この値と等しい(DMX。16bit は粗値+細値/256)"},
        "min": {"type": "number"},
        "max": {"type": "number"},
        "tolerance": {"type": "number", "minimum": 0},
    });
    let mut wait_props = condition.clone();
    for (k, v) in [
        ("frames", json!({"type": "integer", "minimum": 1, "description": "今から何フレーム(40Hz)待つか"})),
        ("beats", json!({"type": "number", "exclusiveMinimum": 0, "description": "今から何拍待つか"})),
        ("ms", json!({"type": "integer", "minimum": 0, "description": "何ミリ秒待つか"})),
        ("timeout_ms", json!({"type": "integer", "minimum": 1, "maximum": 300000, "description": "上限(既定 5000)"})),
    ] {
        wait_props[k] = v;
    }
    vec![
        tool("status", "本体の状態(BPM・拍・出力先・入力・送信するかどうか・設定の版など)", json!({}), &[]),
        tool(
            "fixture_list",
            "機材の一覧(ID・名前・機種・属性)。LED バーなどのピクセルの属性は、red・green などのまとめた名前と、pixels(ピクセル数)で返す。まとめた名前は fixture_set・output_check・output_snapshot でそのまま使える",
            json!({}),
            &[],
        ),
        tool(
            "fixture_set",
            "機材1台の属性の基準値を変える(DMX 0〜255)。禁止の値・動いてよい範囲は、出力の直前に本体が必ず直す",
            json!({"fixture": fixture, "attr": attr, "value": {"type": "number", "description": "DMX の値"}}),
            &["fixture", "attr", "value"],
        ),
        tool(
            "control_list",
            "画面・MIDI と同じ割り当て(ボタン・スライダー)の一覧。ID は binding:番号。既定は ID・名前・種類・ページだけ。動作の中身も見るときは detail: true",
            json!({
                "page": {"type": "integer", "minimum": 0, "description": "このページ(0始まり)の割り当てと、全ページ共通の割り当てだけ"},
                "query": {"type": "string", "description": "名前にこの文字を含むものだけ"},
                "detail": {"type": "boolean", "description": "動作・きっかけ(MIDI・OSC)の中身も返す"},
            }),
            &[],
        ),
        tool(
            "control_press",
            "割り当てを押す・離す・スライダーを動かす。click=少し押して離す、press/hold=hold_ms の間押す(期限で必ず離れる)、release=離す、set=スライダーを value(0〜1)にする",
            json!({
                "control": {"type": "string", "description": "binding:番号(control_list の id)"},
                "event": {"type": "string", "enum": ["click", "press", "hold", "release", "set"]},
                "hold_ms": {"type": "integer", "minimum": 1, "maximum": 5000},
                "value": {"type": "number", "minimum": 0, "maximum": 1},
            }),
            &["control", "event"],
        ),
        tool(
            "command",
            "本体の操作を1つ実行する(ctl command と同じ)。例: {\"cmd\":\"master\",\"value\":0.5}、{\"cmd\":\"scene\",\"name\":\"サビ\"}、{\"cmd\":\"flash\",\"on\":true,\"hold_ms\":500}、{\"cmd\":\"bpm\",\"value\":128}。使える操作と範囲は schema の commands",
            json!({"command": {"type": "object", "description": "cmd を含む操作"}}),
            &["command"],
        ),
        tool(
            "output_snapshot",
            "機材1台の、安全の処理を通った最終の出力の値(DMX)。values は属性名と値の対応。attrs で絞れる(LED バーの red などのまとめた名前も可)",
            json!({"fixture": fixture, "attrs": {"type": "array", "items": {"type": "string"}}}),
            &["fixture"],
        ),
        tool("output_check", "最終の出力が条件を満たすか確かめる(passed と実際の値を返す)", condition, &["fixture", "attr"]),
        tool("wait", "フレーム数・拍数・時間、または出力の条件(fixture・attr と equals/min/max)を満たすまで待つ", wait_props, &[]),
        tool(
            "explain",
            "次のフレームで、その属性の値がどう決まったか(基準値・エフェクト・動き・光量・安全の処理など)",
            json!({"fixture": fixture, "attr": attr, "timeout_ms": {"type": "integer", "minimum": 1, "maximum": 300000}}),
            &["fixture", "attr"],
        ),
        tool(
            "schema",
            "設定の形(JSON Schema)・今の値・読み取り専用の項目・使える操作の一覧",
            json!({"entity": {"type": "string", "enum": ["project", "fixture", "binding", "group", "scene", "position", "effect", "pixel", "chase", "pair", "output", "auto", "auto_preset", "motion", "profile", "action"]}}),
            &[],
        ),
        tool(
            "edit",
            "設定を編集する(機材・割り当て・シーンなど)。まず dry_run: true で確かめてから実行する。operations は ctl edit と同じ形",
            json!({
                "operations": {"type": "array", "items": {"type": "object"}, "description": "{entity, op: create|update|delete|capture, key?, data?, index?} の並び"},
                "dry_run": {"type": "boolean"},
                "expected_revision": {"type": "integer", "minimum": 0, "description": "status の version.revision。違えば失敗する"},
                "expected_session": {"type": "string"},
            }),
            &["operations"],
        ),
        tool("project_save", "今の設定をプロジェクトのファイルに保存する", json!({}), &[]),
    ]
}

// ピクセルの属性の判定(`red.12` → (`red`, Some(12)))は、本体の `Mode::attr_channels` と同じものを使う。
// 番号は1以上の u16 だけ(`red.0` はピクセルではなく普通の属性名)
use dmxctl_core::fixture::split_pixel;

/// 機材の一覧を軽くする: ピクセルの属性(red.1〜red.90 など)を、まとめた名前とピクセル数にする
fn compact_fixtures(mut list: Value) -> Value {
    for f in list.as_array_mut().into_iter().flatten() {
        let Some(attrs) = f["attrs"].as_array() else { continue };
        let mut names = std::collections::BTreeSet::new();
        let mut pixels = std::collections::BTreeSet::new();
        for a in attrs.iter().filter_map(Value::as_str) {
            match split_pixel(a) {
                (base, Some(n)) => {
                    names.insert(base.to_string());
                    pixels.insert(n);
                }
                (name, None) => {
                    names.insert(name.to_string());
                }
            }
        }
        f["attrs"] = json!(names);
        if !pixels.is_empty() {
            f["pixels"] = json!(pixels.len());
        }
    }
    list
}

/// 割り当ての一覧を軽くする。既定は ID・名前・種類・ページ・押しているかだけ
fn compact_controls(list: Value, page: Option<u64>, query: Option<&str>, detail: bool) -> Value {
    let items = list.as_array().into_iter().flatten().filter(|c| {
        page.is_none_or(|p| c["page"].is_null() || c["page"].as_u64() == Some(p)) && query.is_none_or(|q| c["name"].as_str().is_some_and(|n| n.contains(q)))
    });
    json!(items
        .map(|c| {
            if detail {
                return c.clone();
            }
            let action = c["action"].as_object().and_then(|o| o.keys().next().cloned()).or_else(|| c["action"].as_str().map(String::from));
            let mut short = json!({"id": c["id"], "name": c["name"], "kind": c["kind"], "action": action, "page": c["page"]});
            for k in ["pressed", "remote_held"] {
                if c[k] == true {
                    short[k] = json!(true);
                }
            }
            if c["remote_allowed"] == false {
                short["remote_allowed"] = json!(false);
            }
            short
        })
        .collect::<Vec<_>>())
}

/// 機材1台の出力を、属性名と値の対応にする(チャンネルごとの番地などを省く)
fn compact_snapshot(snap: Value) -> Value {
    let channels = snap["channels"].as_array().cloned().unwrap_or_default();
    let mut values = serde_json::Map::new();
    for c in &channels {
        let attr = c["attr"].as_str().unwrap_or("?").to_string();
        match values.get_mut(&attr) {
            // 同じ属性名のチャンネルが複数あるとき(16bit の細かい値など)は、並べる
            Some(Value::Array(list)) => list.push(c["value"].clone()),
            Some(prev) => *prev = json!([prev.clone(), c["value"].clone()]),
            None => {
                values.insert(attr, c["value"].clone());
            }
        }
    }
    let first = channels.first();
    json!({
        "frame": snap["frame"],
        "t_ms": snap["t_ms"],
        "universe": first.map(|c| c["universe"].clone()),
        "first_address": first.map(|c| c["address"].clone()),
        "values": values,
    })
}

fn condition_of(args: &Value) -> Value {
    let mut c = json!({});
    for k in ["fixture", "attr", "equals", "min", "max", "tolerance"] {
        if let Some(v) = args.get(k) {
            c[k] = v.clone();
        }
    }
    c
}

fn send(request: Requester, payload: Value) -> anyhow::Result<Value> {
    request(&payload, Duration::from_secs(6))
}

/// 引数を読む。無いときと、型が違うときで理由を分ける(AI が読んで直せるように)
fn arg<'a, T>(args: &'a Value, key: &str, kind: &str, get: impl Fn(&'a Value) -> Option<T>) -> anyhow::Result<T> {
    let v = args.get(key).with_context(|| format!("{key} is required"))?;
    get(v).with_context(|| format!("{key} must be {kind}"))
}

fn optional<'a, T>(args: &'a Value, key: &str, kind: &str, get: impl Fn(&'a Value) -> Option<T>) -> anyhow::Result<Option<T>> {
    args.get(key).map(|v| get(v).with_context(|| format!("{key} must be {kind}"))).transpose()
}

fn call(request: Requester, name: &str, args: &Value) -> anyhow::Result<Value> {
    let fixture = || arg(args, "fixture", "a non-negative integer (fixture_list の id)", Value::as_u64);
    let attr = || arg(args, "attr", "a string", Value::as_str);
    match name {
        "status" => send(request, json!({"op": "status"})),
        "fixture_list" => Ok(compact_fixtures(send(request, json!({"op": "fixtures", "include_attrs": true}))?)),
        "fixture_set" => {
            let value = arg(args, "value", "a number", Value::as_f64)?;
            ensure!(value.is_finite(), "value must be finite");
            send(request, json!({"op": "command", "command": {"cmd": "set", "fixture": fixture()?, "attr": attr()?, "value": value}}))
        }
        "control_list" => {
            let page = optional(args, "page", "a non-negative integer", Value::as_u64)?;
            let query = optional(args, "query", "a string", Value::as_str)?;
            let detail = optional(args, "detail", "a boolean", Value::as_bool)?.unwrap_or(false);
            Ok(compact_controls(send(request, json!({"op": "controls"}))?, page, query, detail))
        }
        "control_press" => {
            let mut command = json!({"cmd": "button", "control": arg(args, "control", "a string like binding:3", Value::as_str)?,
                "event": arg(args, "event", "one of click/press/hold/release/set", Value::as_str)?});
            for k in ["hold_ms", "value"] {
                if let Some(v) = args.get(k) {
                    command[k] = v.clone();
                }
            }
            send(request, json!({"op": "command", "command": command}))
        }
        "command" => {
            let command = args["command"].clone();
            ensure!(command.is_object() && command["cmd"].is_string(), "command must be an object with cmd");
            send(request, json!({"op": "command", "command": command}))
        }
        "output_snapshot" => {
            // 全機材の出力は本番の設定で数百KBになるので、MCP では機材1台ずつにする
            let mut payload = json!({"op": "output", "fixture": fixture()?});
            if let Some(a) = args.get("attrs") {
                payload["attrs"] = a.clone();
            }
            Ok(compact_snapshot(send(request, payload)?))
        }
        "output_check" => {
            fixture()?;
            attr()?;
            let mut payload = condition_of(args);
            payload["op"] = json!("check");
            send(request, payload)
        }
        "wait" => {
            let frames = optional(args, "frames", "a positive integer", Value::as_u64)?;
            let beats = optional(args, "beats", "a number", Value::as_f64)?;
            let ms = optional(args, "ms", "a non-negative integer", Value::as_u64)?;
            let timeout = optional(args, "timeout_ms", "a positive integer", Value::as_u64)?.unwrap_or(5000);
            crate::diagnostics::wait(request, frames, beats, ms, condition_of(args), timeout)
        }
        "explain" => {
            let timeout = optional(args, "timeout_ms", "a positive integer", Value::as_u64)?.unwrap_or(5000);
            crate::diagnostics::explain(request, json!({"fixture": fixture()?, "attr": attr()?}), timeout)
        }
        "schema" => send(request, json!({"op": "schema", "entity": args["entity"].as_str().unwrap_or("project")})),
        "edit" => {
            ensure!(args["operations"].is_array(), "operations must be an array");
            let dry_run = crate::commands::flag(args, "dry_run", false)?;
            let mut payload = json!({"op": "edit", "operations": args["operations"], "dry_run": dry_run});
            if let Some(v) = args.get("expected_revision") {
                payload["expected_revision"] = v.clone();
            }
            if let Some(v) = args.get("expected_session") {
                payload["expected_session"] = v.clone();
            }
            let mut result = send(request, payload)?;
            // 割り当ての一覧は、割り当てを編集したときだけ返す(本番の設定では数十KBになり、AI の文脈を圧迫するため)
            let touches_bindings = args["operations"].as_array().is_some_and(|ops| ops.iter().any(|op| op["entity"] == "binding"));
            if !touches_bindings && let Some(obj) = result.as_object_mut() {
                obj.remove("bindings");
            }
            Ok(result)
        }
        "project_save" => send(request, json!({"op": "save"})),
        _ => bail!("unknown tool: {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// 送られた要求を記録し、決まった答えを返す偽の本体
    fn fake() -> (RefCell<Vec<Value>>, impl Fn(&Value, Duration) -> anyhow::Result<Value> + Sync) {
        let log = RefCell::new(Vec::new());
        // 時計を聞くたびに1フレーム進む
        let frame = std::sync::atomic::AtomicU64::new(10);
        let reply = move |p: &Value, _: Duration| -> anyhow::Result<Value> {
            match p["op"].as_str().unwrap_or("") {
                "clock" => {
                    let frame = frame.fetch_add(1, Ordering::Relaxed) + 1;
                    Ok(json!({"frame": frame, "beat": 2.0, "session": "s", "emergency": false}))
                }
                "check" => Ok(json!({"passed": true, "frame": 11, "actual": [80.0]})),
                "command" if p["command"]["cmd"] == "input" => bail!("laser cues cannot be operated through remote controls"),
                _ => Ok(json!({"echo": p})),
            }
        };
        (log, reply)
    }

    fn rpc(id: u64, method: &str, params: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
    }

    #[test]
    fn initialize_lists_tools_and_ignores_notifications() {
        let (_, f) = fake();
        let init = handle(&f, &rpc(1, "initialize", json!({"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}))).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(init["result"]["serverInfo"]["name"], "dmxctl");
        assert!(init["result"]["capabilities"]["tools"].is_object());
        // 知らない版を求められたら、こちらの最新で答える
        let other = handle(&f, &rpc(2, "initialize", json!({"protocolVersion": "2025-03-26"}))).unwrap();
        assert_eq!(other["result"]["protocolVersion"], "2025-06-18");
        assert!(handle(&f, &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).is_none());
        let list = handle(&f, &rpc(3, "tools/list", json!({}))).unwrap();
        let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        for n in ["status", "fixture_set", "control_press", "output_check", "wait", "explain", "edit", "project_save"] {
            assert!(names.contains(&n), "{n}");
        }
        // 本体を止める・ARM する専用の操作は置かない
        assert!(!names.iter().any(|n| n.contains("stop") || n.contains("arm")));
        for t in list["result"]["tools"].as_array().unwrap() {
            assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        }
        assert_eq!(handle(&f, &rpc(4, "ping", json!({}))).unwrap()["result"], json!({}));
        assert_eq!(handle(&f, &rpc(5, "resources/list", json!({}))).unwrap()["error"]["code"], -32601);
        let batch = json!([rpc(6, "ping", json!({}))]);
        assert_eq!(handle(&f, &batch).unwrap()["error"]["code"], -32600);
    }

    #[test]
    fn tools_are_relayed_to_the_control_protocol() {
        let (log, f) = fake();
        let rec = |p: &Value, t: Duration| {
            log.borrow_mut().push(p.clone());
            f(p, t)
        };
        let call = |name: &str, args: Value| handle(&rec, &rpc(9, "tools/call", json!({"name": name, "arguments": args}))).unwrap();
        let r = call("fixture_set", json!({"fixture": 1, "attr": "pan", "value": 80}));
        assert_eq!(r["result"]["isError"], false);
        assert_eq!(log.borrow().last().unwrap()["command"], json!({"cmd": "set", "fixture": 1, "attr": "pan", "value": 80.0}));
        call("control_press", json!({"control": "binding:3", "event": "hold", "hold_ms": 300}));
        assert_eq!(log.borrow().last().unwrap()["command"], json!({"cmd": "button", "control": "binding:3", "event": "hold", "hold_ms": 300}));
        call("output_check", json!({"fixture": 1, "attr": "pan", "equals": 80}));
        assert_eq!(log.borrow().last().unwrap(), &json!({"op": "check", "fixture": 1, "attr": "pan", "equals": 80}));
        call("edit", json!({"operations": [{"entity": "scene", "op": "capture", "data": {"name": "A"}}], "dry_run": true}));
        assert_eq!(log.borrow().last().unwrap()["dry_run"], true);
        // 待つ: 今のフレームから数える
        let w = call("wait", json!({"frames": 1}));
        assert_eq!(w["result"]["isError"], false, "{w}");
        let text: Value = serde_json::from_str(w["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(text["matched"], true);
    }

    #[test]
    fn rejections_and_bad_calls_are_reported() {
        let (_, f) = fake();
        let call = |name: &str, args: Value| handle(&f, &rpc(1, "tools/call", json!({"name": name, "arguments": args}))).unwrap();
        // 本体が拒んだものは、ツールの失敗として理由を返す
        let r = call("command", json!({"command": {"cmd": "input", "source": "osc", "address": "/dmx/laser", "args": [1]}}));
        assert_eq!(r["result"]["isError"], true);
        assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("laser"));
        // 引数の不足と型の違いは、理由を分けて返す
        let text = |r: &Value| r["result"]["content"][0]["text"].as_str().unwrap().to_string();
        assert_eq!(text(&call("fixture_set", json!({"fixture": 1, "attr": "pan"}))), "value is required");
        assert_eq!(text(&call("fixture_set", json!({"fixture": 1, "attr": "pan", "value": "80"}))), "value must be a number");
        assert!(text(&call("fixture_set", json!({"fixture": -1, "attr": "pan", "value": 1}))).starts_with("fixture must be a non-negative integer"));
        assert_eq!(text(&call("wait", json!({"frames": "2"}))), "frames must be a positive integer");
        assert_eq!(call("command", json!({"command": "blackout"}))["result"]["isError"], true);
        // 知らないツール・おかしな引数は、プロトコルのエラー
        assert_eq!(call("stop", json!({}))["error"]["code"], -32602);
        assert_eq!(handle(&f, &rpc(2, "tools/call", json!({"name": "status", "arguments": [1]}))).unwrap()["error"]["code"], -32602);
        assert_eq!(handle(&f, &rpc(3, "tools/call", json!({}))).unwrap()["error"]["message"], "params.name is required");
        assert_eq!(handle(&f, &rpc(4, "tools/call", json!({"name": 7}))).unwrap()["error"]["message"], "params.name must be a string");
    }

    #[test]
    fn large_lists_are_compacted_for_ai_clients() {
        // LED バー: red.1〜red.90 などは、まとめた名前とピクセル数に
        let mut attrs: Vec<String> = vec!["dimmer".into(), "red".into()];
        for n in 1..=90 {
            for c in ["red", "green", "blue", "white"] {
                attrs.push(format!("{c}.{n}"));
            }
        }
        let list = compact_fixtures(json!([{"id": 11, "attrs": attrs}, {"id": 1, "attrs": ["pan", "tilt", "dimmer"]}]));
        assert_eq!(list[0]["attrs"], json!(["blue", "dimmer", "green", "red", "white"]));
        assert_eq!(list[0]["pixels"], 90);
        assert_eq!(list[1]["attrs"], json!(["dimmer", "pan", "tilt"]));
        assert!(list[1].get("pixels").is_none());
        assert_eq!(split_pixel("color.wheel"), ("color.wheel", None), "数字でない後ろは、ピクセルの番号ではない");
        // 本体と同じく、red.0 や 70000 番はピクセルではないので、そのままの名前で残す(まとめた red では解決できないため)
        let odd = compact_fixtures(json!([{"id": 2, "attrs": ["red.0", "green.70000", "blue.1", "blue.2"]}]));
        assert_eq!(odd[0]["attrs"], json!(["blue", "green.70000", "red.0"]));
        assert_eq!(odd[0]["pixels"], 2);

        let controls = json!([
            {"id": "binding:1", "name": "フラッシュ 白", "kind": "button", "page": null, "action": {"Flash": {"white": true}}, "pressed": false, "remote_allowed": true, "trigger": {"kind": "Note"}},
            {"id": "binding:2", "name": "ARM", "kind": "button", "page": 1, "action": {"Arm": {}}, "pressed": false, "remote_allowed": false},
            {"id": "binding:3", "name": "円", "kind": "button", "page": 0, "action": {"Motion": {}}, "pressed": true, "remote_allowed": true},
        ]);
        let short = compact_controls(controls.clone(), None, None, false);
        assert_eq!(short[0], json!({"id": "binding:1", "name": "フラッシュ 白", "kind": "button", "action": "Flash", "page": null}));
        assert_eq!(short[1]["remote_allowed"], false);
        assert_eq!(short[2]["pressed"], true);
        // ページ0: そのページと全ページ共通
        let page0: Vec<_> = compact_controls(controls.clone(), Some(0), None, false).as_array().unwrap().iter().map(|c| c["id"].clone()).collect();
        assert_eq!(page0, vec![json!("binding:1"), json!("binding:3")]);
        assert_eq!(compact_controls(controls.clone(), None, Some("フラッシュ"), false).as_array().unwrap().len(), 1);
        assert_eq!(compact_controls(controls.clone(), None, None, true), controls, "detail は今までどおり");

        let snap = compact_snapshot(json!({"frame": 5, "t_ms": 9, "channels": [
            {"attr": "pan", "universe": 1, "address": 1, "value": 128, "fixture": 1},
            {"attr": "pan", "universe": 1, "address": 2, "value": 7, "fixture": 1},
            {"attr": "dimmer", "universe": 1, "address": 6, "value": 255, "fixture": 1},
        ]}));
        assert_eq!(snap, json!({"frame": 5, "t_ms": 9, "universe": 1, "first_address": 1, "values": {"pan": [128, 7], "dimmer": 255}}));
    }

    /// trace_start が成立したあとに explain をキャンセルしても、本体のトレースの枠を空け、次の explain がすぐ成功する
    #[test]
    fn cancelling_explain_releases_the_trace_slot() {
        use std::io::Write as _;
        use std::sync::Mutex as M;
        // 枠が1つの偽の本体。1件目の結果はいつまでもできない。2件目からはすぐできる
        struct Host {
            slot: Option<u64>,
            next: u64,
            log: Vec<Value>,
        }
        let host = Arc::new(M::new(Host { slot: None, next: 0, log: Vec::new() }));
        let fake = {
            let host = host.clone();
            move |p: &Value, _: Duration| -> anyhow::Result<Value> {
                let mut h = host.lock().unwrap();
                h.log.push(p.clone());
                match p["op"].as_str().unwrap_or("") {
                    "trace_start" => {
                        ensure!(h.slot.is_none(), "another trace is pending");
                        h.next += 1;
                        let t = h.next;
                        h.slot = Some(t);
                        Ok(json!({"ticket": t}))
                    }
                    "trace_result" => {
                        let t = p["ticket"].as_u64().unwrap();
                        ensure!(h.slot == Some(t), "trace is no longer available");
                        if t == 1 {
                            return Ok(json!({"ready": false}));
                        }
                        h.slot = None;
                        Ok(json!({"ready": true, "trace": {"ticket": t}}))
                    }
                    "trace_cancel" => {
                        let t = p["ticket"].as_u64().unwrap();
                        let hit = h.slot == Some(t);
                        if hit {
                            h.slot = None;
                        }
                        Ok(json!({"cancelled": hit}))
                    }
                    _ => Ok(json!({})),
                }
            }
        };
        let (reader, mut writer) = std::io::pipe().unwrap();
        #[derive(Clone)]
        struct Out(Arc<M<Vec<u8>>>);
        impl std::io::Write for Out {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let out = Out(Arc::new(M::new(Vec::new())));
        let server = {
            let out = out.clone();
            std::thread::spawn(move || serve(&fake, std::io::BufReader::new(reader), out))
        };
        let explain = |id: u64| json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": "explain", "arguments": {"fixture": 1, "attr": "pan", "timeout_ms": 60000}}});
        writeln!(writer, "{}", explain(1)).unwrap();
        // trace_start が成立するまで待ってから、キャンセルする
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while host.lock().unwrap().slot.is_none() {
            assert!(std::time::Instant::now() < deadline, "trace_start が呼ばれない");
            std::thread::sleep(Duration::from_millis(5));
        }
        writeln!(writer, "{}", json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 1}})).unwrap();
        // 枠が空くまで待つ(キャンセルを受けたワーカーが trace_cancel を送る)
        while host.lock().unwrap().slot.is_some() {
            assert!(std::time::Instant::now() < deadline, "キャンセル後も枠が空かない");
            std::thread::sleep(Duration::from_millis(5));
        }
        writeln!(writer, "{}", explain(2)).unwrap();
        // 返事が届いてから入力を閉じる(閉じると、待っているものは取りやめになるため)
        while out.0.lock().unwrap().is_empty() {
            assert!(std::time::Instant::now() < deadline, "次の explain の返事が来ない");
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(writer);
        server.join().unwrap().unwrap();
        let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        let replies: Vec<Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(replies.len(), 1, "キャンセルした explain には答えない: {text}");
        assert_eq!(replies[0]["id"], 2);
        assert_eq!(replies[0]["result"]["isError"], false, "次の explain はすぐ成功する: {text}");
        let log = &host.lock().unwrap().log;
        assert!(log.iter().any(|p| p["op"] == "trace_cancel" && p["ticket"] == 1), "{log:?}");
    }

    /// キャンセル通知と次の explain を続けて送っても、後片付けが終わってから次の trace_start が走る
    #[test]
    fn explain_right_after_a_cancel_waits_for_the_cleanup() {
        use std::io::Write as _;
        use std::sync::Mutex as M;
        struct Host {
            slot: Option<u64>,
            next: u64,
            log: Vec<String>,
        }
        let host = Arc::new(M::new(Host { slot: None, next: 0, log: Vec::new() }));
        let fake = {
            let host = host.clone();
            move |p: &Value, _: Duration| -> anyhow::Result<Value> {
                let op = p["op"].as_str().unwrap_or("").to_string();
                let ticket = p["ticket"].as_u64();
                host.lock().unwrap().log.push(format!("{op}:{}", ticket.unwrap_or(0)));
                match op.as_str() {
                    "trace_start" => {
                        let mut h = host.lock().unwrap();
                        ensure!(h.slot.is_none(), "another trace is pending");
                        h.next += 1;
                        let t = h.next;
                        h.slot = Some(t);
                        Ok(json!({"ticket": t}))
                    }
                    "trace_result" if ticket == Some(1) => {
                        // 1件目の問い合わせは遅い(この間にキャンセルと次の explain が届く)
                        std::thread::sleep(Duration::from_millis(150));
                        Ok(json!({"ready": false}))
                    }
                    "trace_result" => {
                        let mut h = host.lock().unwrap();
                        ensure!(h.slot == ticket, "trace is no longer available");
                        h.slot = None;
                        Ok(json!({"ready": true, "trace": {}}))
                    }
                    "trace_cancel" => {
                        let mut h = host.lock().unwrap();
                        if h.slot == ticket {
                            h.slot = None;
                        }
                        Ok(json!({}))
                    }
                    _ => Ok(json!({})),
                }
            }
        };
        let (reader, mut writer) = std::io::pipe().unwrap();
        #[derive(Clone)]
        struct Out(Arc<M<Vec<u8>>>);
        impl std::io::Write for Out {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let out = Out(Arc::new(M::new(Vec::new())));
        let server = {
            let out = out.clone();
            std::thread::spawn(move || serve(&fake, std::io::BufReader::new(reader), out))
        };
        let explain = |id: u64| json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": "explain", "arguments": {"fixture": 1, "attr": "pan", "timeout_ms": 60000}}});
        writeln!(writer, "{}", explain(1)).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        // 1件目が遅い問い合わせ(trace_result)に入るまで待つ
        while !host.lock().unwrap().log.iter().any(|l| l == "trace_result:1") {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        // キャンセルと次の explain を続けて送る(枠が空くのを待たない)
        writeln!(writer, "{}\n{}", json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 1}}), explain(2)).unwrap();
        while out.0.lock().unwrap().is_empty() {
            assert!(std::time::Instant::now() < deadline, "次の explain の返事が来ない");
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(writer);
        server.join().unwrap().unwrap();
        let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        let reply: Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(reply["id"], 2);
        assert_eq!(reply["result"]["isError"], false, "another trace is pending にならない: {text}");
        let log = host.lock().unwrap().log.clone();
        let cancel = log.iter().position(|l| l == "trace_cancel:1").expect("後片付けを送る");
        let second = log.iter().rposition(|l| l.starts_with("trace_start")).unwrap();
        assert!(cancel < second, "後片付けのあとに次の trace_start: {log:?}");
    }

    #[test]
    fn serve_reads_lines_until_input_closes() {
        let (_, f) = fake();
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n",
            "\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "not json\n",
            "{\"jsonrpc\":\"2.0\",\"id\":\"b\",\"method\":\"tools/call\",\"params\":{\"name\":\"status\"}}\n",
        );
        let mut out = Vec::new();
        serve(&f, input.as_bytes(), &mut out).unwrap();
        let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 3, "通知には答えない");
        assert_eq!(lines[0]["id"], 1);
        assert_eq!(lines[1]["error"]["code"], -32700);
        assert_eq!(lines[2]["id"], "b");
        assert_eq!(lines[2]["result"]["isError"], false);
    }
}
