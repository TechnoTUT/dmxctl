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
                // .NET・PowerShell のクライアントは、UTF-8 の BOM を先頭に付けて送ることがある
                let line = line.strip_prefix('\u{feff}').unwrap_or(&line);
                for worker in workers.extract_if(.., |worker| worker.is_finished()) {
                    worker.join().expect("MCP wait worker panicked")?;
                }
                if line.trim().is_empty() {
                    continue;
                }
                let message = match serde_json::from_str::<Value>(line) {
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
                if message["jsonrpc"] == "2.0" && message["method"] == "tools/call" && message["params"]["name"] == "wait"
                    && let Some(id) = message.get("id")
                {
                    let key = id.to_string();
                    let mut waits = pending.lock().unwrap();
                    if waits.contains_key(&key) {
                        write_reply(&output, error(id.clone(), -32600, "request ID is already in use"))?;
                        continue;
                    }
                    if waits.len() >= MAX_WAITS {
                        write_reply(&output, tool_reply(id.clone(), Err(anyhow::anyhow!("too many pending wait requests"))))?;
                        continue;
                    }
                    let cancelled = Arc::new(AtomicBool::new(false));
                    waits.insert(key.clone(), Arc::clone(&cancelled));
                    let pending = &pending;
                    let output = &output;
                    // 待機中も、後続の操作やキャンセルを標準入力から受け付ける
                    workers.push(thread::Builder::new().spawn_scoped(scope, move || {
                        let poll = |payload: &Value, timeout: Duration| {
                            ensure!(!cancelled.load(Ordering::Relaxed), "wait cancelled");
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
        tool("fixture_list", "機材の一覧(ID・名前・機種・属性)", json!({}), &[]),
        tool(
            "fixture_set",
            "機材1台の属性の基準値を変える(DMX 0〜255)。禁止の値・動いてよい範囲は、出力の直前に本体が必ず直す",
            json!({"fixture": fixture, "attr": attr, "value": {"type": "number", "description": "DMX の値"}}),
            &["fixture", "attr", "value"],
        ),
        tool("control_list", "画面・MIDI と同じ割り当て(ボタン・スライダー)の一覧。ID は binding:番号", json!({}), &[]),
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
            "安全の処理を通った最終の出力の値(DMX)。fixture・attrs で絞れる",
            json!({"fixture": fixture, "attrs": {"type": "array", "items": {"type": "string"}}}),
            &[],
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
        "fixture_list" => send(request, json!({"op": "fixtures", "include_attrs": true})),
        "fixture_set" => {
            let value = arg(args, "value", "a number", Value::as_f64)?;
            ensure!(value.is_finite(), "value must be finite");
            send(request, json!({"op": "command", "command": {"cmd": "set", "fixture": fixture()?, "attr": attr()?, "value": value}}))
        }
        "control_list" => send(request, json!({"op": "controls"})),
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
            let mut payload = json!({"op": "output"});
            if let Some(f) = args.get("fixture") {
                payload["fixture"] = f.clone();
            }
            if let Some(a) = args.get("attrs") {
                payload["attrs"] = a.clone();
            }
            send(request, payload)
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
    fn serve_reads_lines_until_input_closes() {
        let (_, f) = fake();
        let input = concat!(
            "\u{feff}{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\r\n",
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
