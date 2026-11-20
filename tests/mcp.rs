//! `dmxctl mcp` を本物の `dmxctl serve` につないで、AI のクライアントと同じやりとりで操作する
use dmxctl::core::{fixture::Library, project::Project};
use dmxctl_backend::control;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

struct Serve {
    child: Child,
    dir: std::path::PathBuf,
    address: String,
}

impl Serve {
    fn start() -> Self {
        let dir = std::env::temp_dir().join(format!("dmxctl-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (lib, _) = Library::load(&dir.join("fixtures"));
        let mut project = Project::starter(&lib);
        project.fixtures[0].limits.insert("pan".into(), [20, 90]);
        project.save(&dir.join("project.json")).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_dmxctl"))
            .args(["serve", "--project", "project.json", "--listen", "127.0.0.1:0", "--inputs", "none"])
            .current_dir(&dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        let ready: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(ready["ready"], true);
        assert_eq!(ready["output_enabled"], false, "テストでは実機へ送らない");
        Serve { child, dir, address: ready["listen"].as_str().unwrap().to_string() }
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Mcp {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Mcp {
    fn start(address: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dmxctl"))
            .args(["mcp", "--connect", address])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Mcp { child, stdin, stdout, next: 0 }
    }

    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    fn rpc(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], id);
        reply
    }

    /// ツールを呼び、(失敗か, 答えの JSON)を返す
    fn call(&mut self, name: &str, arguments: Value) -> (bool, Value) {
        let reply = self.rpc("tools/call", json!({"name": name, "arguments": arguments}));
        let result = &reply["result"];
        let text = result["content"][0]["text"].as_str().unwrap_or("");
        (result["isError"] == true, serde_json::from_str(text).unwrap_or_else(|_| json!(text)))
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn an_mcp_client_operates_the_running_application() {
    let serve = Serve::start();
    let mut mcp = Mcp::start(&serve.address);

    let init = mcp.rpc("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}));
    assert_eq!(init["result"]["serverInfo"]["name"], "dmxctl");
    mcp.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    let tools = mcp.rpc("tools/list", json!({}));
    assert!(tools["result"]["tools"].as_array().unwrap().len() >= 10);

    let (failed, status) = mcp.call("status", json!({}));
    assert!(!failed, "{status}");
    let (failed, fixtures) = mcp.call("fixture_list", json!({}));
    assert!(!failed, "{fixtures}");
    let attrs = fixtures[0]["attrs"].as_array().expect("機材の属性一覧");
    assert!(attrs.iter().any(|attr| attr == "dimmer"), "{fixtures}");
    // 出力の取得は機材1台ずつ。属性名と値の対応で返す
    let (failed, r) = mcp.call("output_snapshot", json!({}));
    assert!(failed && r.as_str().is_some_and(|m| m.contains("fixture is required")), "{r}");
    let (failed, r) = mcp.call("output_snapshot", json!({"fixture": 1, "attrs": ["dimmer"]}));
    assert!(!failed && r["values"]["dimmer"].is_number(), "{r}");
    let (failed, r) = mcp.call("control_list", json!({"query": "存在しない名前"}));
    assert!(!failed && r == json!([]), "{r}");

    // 値を変えて、反映を待ち、最終の出力を確かめる
    let (failed, r) = mcp.call("fixture_set", json!({"fixture": 1, "attr": "dimmer", "value": 200}));
    assert!(!failed, "{r}");
    let (failed, r) = mcp.call("wait", json!({"fixture": 1, "attr": "dimmer", "equals": 200, "timeout_ms": 5000}));
    assert!(!failed, "{r}");
    let (failed, r) = mcp.call("output_check", json!({"fixture": 1, "attr": "dimmer", "equals": 200}));
    assert!(!failed && r["passed"] == true, "{r}");

    // 動いてよい範囲(20〜90)の外は、本体が出力の直前に直す
    let (failed, _) = mcp.call("fixture_set", json!({"fixture": 1, "attr": "pan", "value": 200}));
    assert!(!failed);
    let (failed, r) = mcp.call("wait", json!({"frames": 2}));
    assert!(!failed, "{r}");
    let (_, r) = mcp.call("output_check", json!({"fixture": 1, "attr": "pan", "max": 90}));
    assert_eq!(r["passed"], true, "安全の処理を通った値: {r}");
    let (failed, r) = mcp.call("explain", json!({"fixture": 1, "attr": "pan"}));
    assert!(!failed, "{r}");

    // ARM とレーザーの合図は、本体の窓口が拒む
    let (failed, r) = mcp.call("edit", json!({"operations": [{"entity": "fixture", "op": "update", "key": 2, "data": {"armed": true}}]}));
    assert!(failed, "ARM は設定の編集からもできない: {r}");
    assert!(r.to_string().contains("safety fields are read-only"), "安全の項目として拒まれる: {r}");
    let (failed, r) = mcp.call("command", json!({"command": {"cmd": "input", "source": "osc", "address": "/dmx/laser", "args": [1]}}));
    assert!(failed, "レーザーの合図は送れない: {r}");
    assert!(r.as_str().unwrap_or("").contains("laser"), "{r}");

    // 編集は dry_run で確かめられる
    let (failed, r) = mcp.call("edit", json!({"operations": [{"entity": "scene", "op": "capture", "data": {"name": "MCP"}}], "dry_run": true}));
    assert!(!failed, "{r}");
    assert_eq!(r["applied"], false);
    // 割り当て以外の編集では、割り当ての一覧(本番では数十KB)を返さない
    let (failed, r) = mcp.call("edit", json!({"operations": [{"entity": "scene", "op": "capture", "data": {"name": "MCP保存"}}]}));
    assert!(!failed && r["applied"] == true && r.get("bindings").is_none(), "{r}");
    let (failed, r) = mcp.call("edit", json!({"operations": [{"entity": "scene", "op": "capture", "data": {"name": "MCP"}}], "dry_run": "true"}));
    assert!(failed && r.as_str().is_some_and(|msg| msg.contains("dry_run must be boolean")), "{r}");
    // schema は entity が必須。project の今の値は、頼まれたときだけ返す
    let (failed, r) = mcp.call("schema", json!({}));
    assert!(failed && r.as_str().is_some_and(|m| m.contains("entity is required")), "{r}");
    let (failed, r) = mcp.call("schema", json!({"entity": "project"}));
    assert!(!failed && r.get("current").is_none() && r.get("schema").is_none() && r["entities"].as_array().is_some_and(|e| e.iter().any(|e| e == "fixture")) && r["commands"].is_object(), "{r}");
    let (failed, r) = mcp.call("schema", json!({"entity": "project", "full": true}));
    assert!(!failed && r["current"].is_object() && r["schema"].is_object(), "{r}");
    let (failed, scenes) = mcp.call("schema", json!({"entity": "scene"}));
    assert!(!failed, "{scenes}");
    assert!(scenes["current"].as_array().unwrap().iter().all(|scene| scene["name"] != "MCP"));
    // 色は日本語の名前でも英語名でも選べる。範囲外の全体光量は丸めずに拒む
    for name in ["赤", "red"] {
        let (failed, r) = mcp.call("command", json!({"command": {"cmd": "color", "name": name, "slot": "main"}}));
        assert!(!failed, "{name}: {r}");
    }
    let (failed, r) = mcp.call("command", json!({"command": {"cmd": "master", "value": -0.1}}));
    assert!(failed && r.as_str().is_some_and(|m| m.contains("0..1")), "{r}");

    // 長い待機の途中でも、後続の要求とキャンセルを受け付ける
    mcp.next += 1;
    let wait_id = mcp.next;
    mcp.send(json!({"jsonrpc": "2.0", "id": wait_id, "method": "tools/call",
        "params": {"name": "wait", "arguments": {"ms": 15000, "timeout_ms": 1000}}}));
    assert_eq!(mcp.rpc("ping", json!({}))["result"], json!({}));
    mcp.send(json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": wait_id}}));
    let (failed, r) = mcp.call("wait", json!({"ms": 1500}));
    assert!(!failed && r["matched"] == true, "{r}");

    // explain も、待っている間に後続の要求とキャンセルを受け付ける
    mcp.next += 1;
    let explain_id = mcp.next;
    mcp.send(json!({"jsonrpc": "2.0", "id": explain_id, "method": "tools/call",
        "params": {"name": "explain", "arguments": {"fixture": 1, "attr": "pan", "timeout_ms": 5000}}}));
    mcp.send(json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": explain_id}}));
    // キャンセルより先に explain が終わっていれば、その返事が先に届くことがある(どちらでもよい)
    mcp.next += 1;
    let ping_id = mcp.next;
    mcp.send(json!({"jsonrpc": "2.0", "id": ping_id, "method": "ping"}));
    loop {
        let mut line = String::new();
        mcp.stdout.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        if reply["id"] == ping_id {
            assert_eq!(reply["result"], json!({}));
            break;
        }
        assert_eq!(reply["id"], explain_id, "{reply}");
    }
    let (failed, r) = mcp.call("explain", json!({"fixture": 1, "attr": "pan"}));
    assert!(!failed && r.is_object(), "{r}");


    // MCP を閉じても、本体は動き続ける
    drop(mcp.stdin.take());
    let deadline = Instant::now() + Duration::from_secs(5);
    while mcp.child.try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "入力を閉じたら mcp は終わる");
        std::thread::sleep(Duration::from_millis(20));
    }
    let address = serve.address.parse().unwrap();
    assert!(control::request(address, &json!({"op": "status"})).is_ok(), "本体は止まらない");
}

#[test]
fn tools_report_when_the_application_is_not_running() {
    // 誰も待っていないポート
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let mut mcp = Mcp::start(&format!("127.0.0.1:{port}"));
    mcp.rpc("initialize", json!({"protocolVersion": "2025-06-18"}));
    let (failed, r) = mcp.call("status", json!({}));
    assert!(failed);
    assert!(r.as_str().unwrap_or("").contains("cannot connect"), "{r}");
}
