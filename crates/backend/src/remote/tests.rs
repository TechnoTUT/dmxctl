use super::*;
use dmxctl_core::engine::Core;
use dmxctl_core::project::{Action, Binding, PadMode, Selector};

fn wait(mut f: impl FnMut() -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(5) {
        if f() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

fn server() -> (Shared, RemoteServer) {
    let (lib, _) = Library::load(std::path::Path::new("/nonexistent"));
    let project = Project::starter(&lib);
    let core: Shared = Arc::new(Mutex::new(Core::new(project, Arc::new(lib))));
    let server = RemoteServer::start(core.clone(), dmxctl_core::wake::Wake::none(), "127.0.0.1:0".parse().unwrap(), false).unwrap();
    (core, server)
}

fn client(server: &RemoteServer) -> (Shared, Client) {
    let (mut client, initial) = Client::connect(&server.address.to_string(), dmxctl_core::wake::Wake::none()).unwrap();
    let core: Shared = Arc::new(Mutex::new(Core::new(initial.project, Arc::new(initial.lib))));
    client.attach(core.clone());
    (core, client)
}

/// 画面の1フレーム(edit で写しを書き換える)
fn frame(client: &mut Client, core: &Shared, edit: impl FnOnce(&mut Core)) {
    client.before_frame(core);
    edit(&mut lock(core));
    client.after_frame(core);
}

fn server_with(project: Project) -> (Shared, RemoteServer) {
    let (lib, _) = Library::load(std::path::Path::new("/nonexistent"));
    let core: Shared = Arc::new(Mutex::new(Core::new(project, Arc::new(lib))));
    let server = RemoteServer::start(core.clone(), dmxctl_core::wake::Wake::none(), "127.0.0.1:0".parse().unwrap(), false).unwrap();
    (core, server)
}

/// 手元の写しがサーバーと同じになるまでフレームを回す
fn converges(client: &mut Client, core: &Shared, server_core: &Shared) -> bool {
    wait(|| {
        frame(client, core, |_| {});
        let mut server = serde_json::to_value(&lock(server_core).project).unwrap();
        let mut local = serde_json::to_value(&lock(core).project).unwrap();
        server["midi_input"] = Value::Null;
        local["midi_input"] = Value::Null;
        server == local
    })
}

#[test]
fn held_pads_and_keys_are_released_when_a_client_drops() {
    use dmxctl_core::project::{Action, Binding, MidiTrigger, PadMode, TriggerKind};
    let (lib, _) = Library::load(std::path::Path::new("/nonexistent"));
    let mut project = Project::starter(&lib);
    project.bindings.clear();
    for (name, kind, number) in [("Flash pad", TriggerKind::Note, 40), ("Flash key", TriggerKind::Key, 16)] {
        project.bindings.push(Binding { id: 0, name: name.into(), action: Action::Flash { white: true, mode: PadMode::Momentary },
            trigger: Some(MidiTrigger { kind, channel: None, number }), page: None, osc: None });
    }
    project.ensure_binding_ids();
    let (server_core, server) = server_with(project);
    let (core, client) = client(&server);
    lock(&core).live(LiveOp::Midi { kind: TriggerKind::Note, channel: 0, number: 40, value: 127 });
    lock(&core).live(LiveOp::Key { number: 16, pressed: true });
    assert!(wait(|| {
        let c = lock(&server_core);
        c.rt.flash == Some(true) && c.rt.notes_down[40] && c.rt.keys_down.contains(&16)
    }));
    // 押したまま切れる(LANが切れた・クライアントが落ちた)
    drop(client);
    assert!(wait(|| {
        let c = lock(&server_core);
        c.rt.flash.is_none() && !c.rt.notes_down[40] && c.rt.keys_down.is_empty()
    }), "the server must release what the dropped client was holding");
}

#[test]
fn an_edit_on_a_shifted_list_does_not_hit_another_item() {
    let (server_core, server) = server();
    let (core_a, mut a) = client(&server);
    let (core_b, mut b) = client(&server);
    let n = lock(&server_core).project.fixtures.len();
    let old_second = lock(&server_core).project.fixtures[1].name.clone();
    assert_ne!(old_second, lock(&server_core).project.fixtures[0].name);
    // B は、A が先頭の機材を消す前の写しで、先頭の機材の名前を変える
    b.before_frame(&core_b);
    frame(&mut a, &core_a, |c| { c.project.fixtures.remove(0); });
    assert!(wait(|| lock(&server_core).project.fixtures.len() == n - 1));
    lock(&core_b).project.fixtures[0].name = "B edit".into();
    b.after_frame(&core_b);
    // B の編集は、詰めたあとの別の機材に当たらずに取り消され、B の写しはサーバーにそろう
    assert!(converges(&mut b, &core_b, &server_core));
    assert!(lock(&server_core).project.fixtures.iter().all(|f| f.name != "B edit"));
    assert_eq!(lock(&server_core).project.fixtures[0].name, old_second);
    assert!(b.take_errors().iter().any(|e| e.contains("重なった")));
}

#[test]
fn deleting_from_a_stale_list_does_not_drop_another_edit() {
    let (server_core, server) = server();
    let (core_a, mut a) = client(&server);
    let (core_b, mut b) = client(&server);
    let n = lock(&server_core).project.fixtures.len();
    // A は、B が名前を変える前の写しで機材を消す
    a.before_frame(&core_a);
    frame(&mut b, &core_b, |c| c.project.fixtures[n - 1].name = "B edit".into());
    assert!(wait(|| lock(&server_core).project.fixtures[n - 1].name == "B edit"));
    lock(&core_a).project.fixtures.remove(0);
    a.after_frame(&core_a);
    // A の削除は B の名前の変更を消さずに取り消され、A の写しはサーバーにそろう
    assert!(converges(&mut a, &core_a, &server_core));
    assert_eq!(lock(&server_core).project.fixtures.len(), n);
    assert_eq!(lock(&server_core).project.fixtures[n - 1].name, "B edit");
    assert!(a.take_errors().iter().any(|e| e.contains("重なった")));
}

#[test]
fn edits_while_disconnected_are_reported() {
    let (_server_core, server) = server();
    let (core, mut client) = client(&server);
    drop(server);
    assert!(wait(|| !client.connected()));
    client.take_errors();
    frame(&mut client, &core, |c| c.project.fixtures[0].values[0] = 9);
    frame(&mut client, &core, |c| c.project.fixtures[0].values[0] = 10);
    let errors = client.take_errors();
    assert_eq!(errors.iter().filter(|e| e.contains("つながっていない")).count(), 1, "{errors:?}");
}

#[test]
fn a_client_over_the_limit_is_told_why() {
    let (_core, server) = server();
    let mut clients: Vec<_> = (0..MAX_CLIENTS).map(|_| client(&server)).collect();
    assert!(wait(|| server.clients() == MAX_CLIENTS));
    let error = Client::connect(&server.address.to_string(), dmxctl_core::wake::Wake::none()).err().expect("over the limit");
    assert!(error.to_string().contains("多すぎます"), "{error}");
    clients.clear();
}

#[test]
fn live_ops_run_on_the_server_and_come_back() {
    let (server_core, server) = server();
    let (core, _client) = client(&server);
    lock(&core).live(LiveOp::SetBlackout { on: true });
    // 手元の写しは変えずに送る
    assert!(!lock(&core).rt.blackout);
    assert!(wait(|| lock(&server_core).rt.blackout));
    assert!(wait(|| lock(&core).rt.blackout));
    let id = lock(&server_core).project.fixtures[0].id;
    lock(&core).live(LiveOp::SetArm { fixture: id, on: true });
    assert!(wait(|| lock(&server_core).project.fixtures[0].armed));
    assert!(wait(|| lock(&core).project.fixtures[0].armed));
}

#[test]
fn edits_flow_both_ways() {
    let (server_core, server) = server();
    let (core, mut client) = client(&server);
    frame(&mut client, &core, |c| c.project.fixtures[0].values[0] = 123);
    assert!(wait(|| lock(&server_core).project.fixtures[0].values[0] == 123));
    // サーバーで変わったもの(自動操作など)は、次のフレームの境目で写しに入る
    lock(&server_core).project.fixtures[1].name = "from server".into();
    assert!(wait(|| {
        frame(&mut client, &core, |_| {});
        lock(&core).project.fixtures[1].name == "from server"
    }));
    assert_eq!(lock(&core).project.fixtures[0].values[0], 123);
}

#[test]
fn a_pending_edit_is_not_reverted_by_an_older_state() {
    let (server_core, server) = server();
    let (core, mut client) = client(&server);
    // サーバーで、クライアントの知らない変更をしておく
    lock(&server_core).project.fixtures[1].name = "server edit".into();
    // 同じころ、クライアントは別の値を書き換える
    frame(&mut client, &core, |c| c.project.fixtures[0].values[0] = 77);
    // 両方の編集が行き渡るまでフレームを回す。決まった回数だけ回すと、PCが混んでいるとき
    // (テストを並べて動かすときなど)にサーバーへ届く前に終わってしまうので、届くまで待つ
    let both_arrived = wait(|| {
        frame(&mut client, &core, |_| {});
        // 手元の編集は、どの時点でも戻らない
        assert_eq!(lock(&core).project.fixtures[0].values[0], 77);
        lock(&core).project.fixtures[1].name == "server edit" && lock(&server_core).project.fixtures[0].values[0] == 77
    });
    assert!(both_arrived, "the edits did not reach both sides");
}

#[test]
fn learn_and_midi_reach_the_server() {
    let (server_core, server) = server();
    let (core, mut client) = client(&server);
    frame(&mut client, &core, |c| c.rt.learn = Some(0));
    assert!(wait(|| lock(&server_core).rt.learn == Some(0)));
    // ラーンは、届いた状態に上書きされずに残る
    frame(&mut client, &core, |_| {});
    assert_eq!(lock(&core).rt.learn, Some(0));
    lock(&core).live(LiveOp::Midi { kind: dmxctl_core::project::TriggerKind::Note, channel: 0, number: 60, value: 100 });
    assert!(wait(|| lock(&server_core).rt.notes_down[60]));
}

#[test]
fn a_client_reconnects_to_a_restarted_server() {
    let (_old_core, server) = server();
    let address = server.address;
    let (core, mut client) = client(&server);
    drop(server);
    assert!(wait(|| !client.connected()));
    // 同じアドレスで、別のプロジェクトのサーバーを起動し直す
    let (lib, _) = Library::load(std::path::Path::new("/nonexistent"));
    let mut project = Project::starter(&lib);
    project.fixtures[0].name = "after restart".into();
    let new_core: Shared = Arc::new(Mutex::new(Core::new(project, Arc::new(lib))));
    let _server = RemoteServer::start(new_core.clone(), dmxctl_core::wake::Wake::none(), address, false).unwrap();
    assert!(wait(|| {
        frame(&mut client, &core, |_| {});
        client.connected() && lock(&core).project.fixtures[0].name == "after restart"
    }));
    // つなぎ直したあとの操作も届く
    lock(&core).live(LiveOp::SetBlackout { on: true });
    assert!(wait(|| lock(&new_core).rt.blackout));
}

#[test]
fn device_lists_and_profiles_go_through_the_server() {
    let dir = std::env::temp_dir().join(format!("dmxctl-remote-profiles-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (lib, _) = Library::load(&dir);
    let project = Project::starter(&lib);
    let server_core: Shared = Arc::new(Mutex::new(Core::new(project, Arc::new(lib))));
    let server = RemoteServer::start(server_core.clone(), dmxctl_core::wake::Wake::none(), "127.0.0.1:0".parse().unwrap(), false).unwrap();
    let (core, client) = client(&server);
    // つないだときに、サーバーのPCの機器の一覧が届く
    assert!(wait(|| client.devices().is_some()));
    let (count, _) = client.devices().unwrap();
    client.request_devices();
    assert!(wait(|| client.devices().is_some_and(|(n, _)| n > count)));
    // クライアントで保存した機材定義は、サーバーのフォルダに入り、クライアントにも配られる
    // 安全の設定が無い定義なら、クライアントから作って消せる
    let mut profile = (*lock(&core).lib.profiles[0]).clone();
    profile.needs_arm = false;
    for ch in profile.modes.iter_mut().flat_map(|m| &mut m.channels) {
        ch.safe = None;
        ch.forbid.clear();
    }
    profile.id = "test/from-client".into();
    profile.name = "From client".into();
    client.save_profile(&profile);
    assert!(wait(|| lock(&server_core).lib.get("test/from-client").is_some()));
    assert!(wait(|| lock(&core).lib.get("test/from-client").is_some()));
    assert!(std::fs::read_dir(&dir).unwrap().count() > 0, "saved into the server's folder");
    client.delete_profile("test/from-client");
    assert!(wait(|| lock(&server_core).lib.get("test/from-client").is_none()));
    assert!(wait(|| lock(&core).lib.get("test/from-client").is_none()));
    drop(client);
    drop(server);
    std::fs::remove_dir_all(dir).unwrap();
}

/// クライアントから、ARMが要る機材定義を作ってから ARM を外そうとする。外せたかを返す
fn client_turns_off_arm(allow_safety_edit: bool) -> bool {
    let dir = std::env::temp_dir().join(format!("dmxctl-remote-safety-{}-{allow_safety_edit}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (lib, _) = Library::load(&dir);
    let server_core: Shared = Arc::new(Mutex::new(Core::new(Project::starter(&lib), Arc::new(lib))));
    let server = RemoteServer::start(server_core.clone(), dmxctl_core::wake::Wake::none(), "127.0.0.1:0".parse().unwrap(), allow_safety_edit).unwrap();
    let (core, client) = client(&server);
    let mut laser = (*lock(&core).lib.profiles[0]).clone();
    laser.id = "test/laser".into();
    laser.needs_arm = true;
    // 新しく作るのは、どちらでもできる
    client.save_profile(&laser);
    assert!(wait(|| lock(&server_core).lib.get("test/laser").is_some_and(|p| p.needs_arm)));
    client.take_errors();
    laser.needs_arm = false;
    client.save_profile(&laser);
    let changed = wait(|| lock(&server_core).lib.get("test/laser").is_some_and(|p| !p.needs_arm));
    if !changed {
        assert!(client.take_errors().iter().any(|e| e.contains("--remote-allow-safety-edit")));
    }
    drop(client);
    drop(server);
    std::fs::remove_dir_all(dir).unwrap();
    changed
}

#[test]
fn safety_settings_change_only_when_the_server_allows_it() {
    assert!(!client_turns_off_arm(false), "safety settings are read-only from clients by default");
    assert!(client_turns_off_arm(true), "--remote-allow-safety-edit lets clients change them");
}

#[test]
fn listen_addresses_need_an_ip() {
    assert_eq!(parse_listen("192.168.1.10").unwrap(), "192.168.1.10:9300".parse().unwrap());
    assert_eq!(parse_listen("0.0.0.0:9400").unwrap(), "0.0.0.0:9400".parse().unwrap());
    assert_eq!(parse_listen("127.0.0.1:9401").unwrap(), "127.0.0.1:9401".parse().unwrap());
    // ポートだけでは、どこから受けるかが分からない
    assert!(parse_listen("9400").is_err());
    assert!(parse_listen(":9400").is_err());
    assert!(parse_listen("nope").is_err());
}

#[test]
fn probe_succeeds_only_while_the_server_listens() {
    let (_core, server) = server();
    let target = server.address.to_string();
    crate::host::Host::probe_server(&target, dmxctl_core::wake::Wake::none()).unwrap();
    // 待ち受けを止めたあとは、確認も失敗する
    drop(server);
    assert!(crate::host::Host::probe_server(&target, dmxctl_core::wake::Wake::none()).is_err());
}

/// 生の接続で hello を送り、送った patch への error を待つ(届かなければ None)
fn raw_patch_error(server: &RemoteServer, changes: serde_json::Value) -> Option<String> {
    raw_error(server, json!({"t": "patch", "changes": changes, "seq": 1}))
}

/// 1行の要求を送り、error が返ればその内容を返す
fn raw_error(server: &RemoteServer, message: serde_json::Value) -> Option<String> {
    use std::io::{BufRead, BufReader, Write};
    let stream = std::net::TcpStream::connect(server.address).unwrap();
    stream.set_read_timeout(Some(Duration::from_millis(1500))).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    writeln!(writer, "{}", json!({"t": "hello", "protocol": PROTOCOL, "seq": 0})).unwrap();
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    assert!(line.contains("welcome"), "{line}");
    writeln!(writer, "{message}").unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        line.clear();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let v: serde_json::Value = serde_json::from_str(&line).unwrap_or_default();
        if v["t"] == "error" {
            return Some(v["msg"].as_str().unwrap_or("").to_string());
        }
    }
    None
}

#[test]
fn malformed_patches_are_reported_to_the_client() {
    let (_core, server) = server();
    let e = raw_patch_error(&server, json!("oops")).expect("壊れた changes に error を返す");
    assert!(e.starts_with("編集の形が正しくありません"), "{e}");
    let e = raw_patch_error(&server, json!([{"path": ["nope"], "value": 1}])).expect("存在しない項目に error を返す");
    assert!(e.starts_with("存在しない項目です"), "{e}");
}

#[test]
fn remote_patches_cannot_widen_laser_limits_unless_allowed() {
    let laser = |core: &Shared| lock(core).project.fixtures.iter().position(|f| f.profile == "stage-evolution/laserscan300").unwrap();
    let widen = |i: usize| json!([{"path": ["fixtures", i, "limits"], "value": {"pan": [0, 255]}}]);
    // 既定では拒む
    let (core, server) = server();
    let i = laser(&core);
    let e = raw_patch_error(&server, widen(i)).expect("範囲の変更に error を返す");
    assert!(e.contains("--remote-allow-safety-edit"), "{e}");
    assert!(lock(&core).project.fixtures[i].limits.is_empty(), "範囲は変わらない");
    let e = raw_patch_error(&server, json!([{"path": ["motion", "pan_front"], "value": 90.0}])).expect("校正の変更に error を返す");
    assert!(e.contains("--remote-allow-safety-edit"), "{e}");
    // --remote-allow-safety-edit なら変えられる
    let (lib, _) = Library::load(std::path::Path::new("/nonexistent"));
    let allowed: Shared = Arc::new(Mutex::new(Core::new(Project::starter(&lib), Arc::new(lib))));
    let server = RemoteServer::start(allowed.clone(), dmxctl_core::wake::Wake::none(), "127.0.0.1:0".parse().unwrap(), true).unwrap();
    assert_eq!(raw_patch_error(&server, widen(i)), None);
    assert!(wait(|| lock(&allowed).project.fixtures[i].limits.get("pan") == Some(&[0, 255])));
}

#[test]
fn remote_learn_cannot_target_safety_bindings_unless_allowed() {
    let (core, server) = server();
    let arm = {
        let mut c = lock(&core);
        c.project.bindings.push(Binding { id: 0, name: "ARM".into(), trigger: None, action: Action::Arm { sel: Selector::All, mode: PadMode::Toggle }, page: None, osc: None });
        c.project.ensure_binding_ids();
        c.project.bindings.len() - 1
    };
    let learn = |i: usize| json!({"t": "live", "op": {"live": "set_learn", "midi": i, "osc": i}, "seq": 1});
    let e = raw_error(&server, learn(arm)).expect("ARM の割り当てのラーンに error を返す");
    assert!(e.contains("--remote-allow-safety-edit"), "{e}");
    assert!(lock(&core).rt.learn.is_none() && lock(&core).rt.osc_learn.is_none());
    // ほかの割り当てのラーンは始められる。書き込むときにも確かめるよう、印を付ける
    assert_eq!(raw_error(&server, learn(0)), None);
    assert!(wait(|| lock(&core).rt.learn == Some(0) && lock(&core).rt.learn_guarded));
}
