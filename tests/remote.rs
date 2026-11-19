//! サーバー(Host + --remote-listen)と、画面(App)のクライアント(connect)を同じプロセスでつなぐ
use dmxctl::backend::{control::Launch, host::Host};
use dmxctl::core::live::LiveOp;
use dmxctl::core::{engine::lock, fixture::Library, project::Project};
use std::time::{Duration, Instant};

fn project_dir() -> std::path::PathBuf {
    // テストは並んで動くので、番号を振ってフォルダが重ならないようにする
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("dmxctl-remote-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (lib, _) = Library::load(&dir.join("fixtures"));
    Project::starter(&lib).save(&dir.join("project.json")).unwrap();
    dir
}

#[test]
fn a_gui_client_follows_the_server_and_sends_operations() {
    let dir = project_dir();
    let server_ctx = dmxctl::core::wake::Wake::none();
    let mut server = Host::start(&server_ctx, Launch {
        project: Some(dir.join("project.json")), output: false, inputs: false, autosave: false,
        remote_listen: Some("127.0.0.1:0".parse().unwrap()), ..Default::default()
    }).unwrap();
    let address = server.remote.as_ref().unwrap().address;

    let ctx = eframe::egui::Context::default();
    let mut app = dmxctl::ui::app::App::with_options(&ctx, false, Launch {
        inputs: false, connect: Some(address.to_string()), ..Default::default()
    }).unwrap();
    assert!(app.host.is_client());
    let frames = |app: &mut dmxctl::ui::app::App, until: &mut dyn FnMut(&mut dmxctl::ui::app::App) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut out = ctx.run_ui(Default::default(), |ui| app.frame(ui, None));
            // GPU無しで画面を回すので、テクスチャは捨てる
            out.textures_delta.clear();
            if until(app) { return; }
            assert!(Instant::now() < deadline, "client did not catch up");
            std::thread::sleep(Duration::from_millis(20));
        }
    };

    // サーバーでの変更が、画面のフレームの境目で写しに入る
    lock(&server.core).project.fixtures[0].name = "renamed on server".into();
    frames(&mut app, &mut |app| lock(&app.host.core).project.fixtures[0].name == "renamed on server");

    // クライアントの操作はサーバーで実行され、戻ってくる
    lock(&app.host.core).live(LiveOp::SetBlackout { on: true });
    frames(&mut app, &mut |app| lock(&app.host.core).rt.blackout);
    assert!(lock(&server.core).rt.blackout);

    // クライアントの保存はサーバーが行う
    lock(&server.core).dirty = true;
    app.save();
    let deadline = Instant::now() + Duration::from_secs(5);
    while lock(&server.core).dirty {
        server.control_tick();
        assert!(Instant::now() < deadline, "server did not save");
        std::thread::sleep(Duration::from_millis(20));
    }
    let saved = std::fs::read_to_string(dir.join("project.json")).unwrap();
    assert!(saved.contains("renamed on server"));

    // クライアントを閉じても、サーバーはブラックアウトしない(状態はそのまま)
    lock(&server.core).live(LiveOp::SetBlackout { on: false });
    app.shutdown();
    drop(app);
    std::thread::sleep(Duration::from_millis(100));
    assert!(!lock(&server.core).rt.blackout);
    server.shutdown();
    drop(server);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_gui_client_opens_server_files_and_files_from_its_own_pc() {
    let dir = project_dir();
    // サーバーのプロジェクトのフォルダに、別のファイルを置いておく
    let (lib, _) = Library::load(&dir.join("fixtures"));
    let mut other = Project::starter(&lib);
    other.fixtures[0].name = "from other.json".into();
    other.save(&dir.join("other.json")).unwrap();
    let server_ctx = dmxctl::core::wake::Wake::none();
    let mut server = Host::start(&server_ctx, Launch {
        project: Some(dir.join("project.json")), output: false, inputs: false, autosave: false,
        remote_listen: Some("127.0.0.1:0".parse().unwrap()), ..Default::default()
    }).unwrap();
    let address = server.remote.as_ref().unwrap().address;
    let ctx = eframe::egui::Context::default();
    let mut app = dmxctl::ui::app::App::with_options(&ctx, false, Launch {
        inputs: false, connect: Some(address.to_string()), ..Default::default()
    }).unwrap();
    // サーバーが Host のループで読み込み、クライアントは画面のフレームで受け取る
    let run = |app: &mut dmxctl::ui::app::App, server: &mut Host, until: &mut dyn FnMut(&mut dmxctl::ui::app::App, &mut Host) -> bool| {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            server.control_tick();
            let mut out = ctx.run_ui(Default::default(), |ui| app.frame(ui, None));
            out.textures_delta.clear();
            if until(app, server) { return; }
            assert!(Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(20));
        }
    };

    // サーバーのファイルの一覧から開く
    app.host.request_server_files();
    run(&mut app, &mut server, &mut |app, _| app.host.server_files().is_some_and(|f| f.projects.iter().any(|p| p.ends_with("other.json"))));
    let files = app.host.server_files().unwrap();
    assert!(files.current.ends_with("project.json"));
    let path = files.projects.iter().find(|p| p.ends_with("other.json")).unwrap().clone();
    app.host.open_server_file(&path);
    run(&mut app, &mut server, &mut |app, server| lock(&server.core).project.fixtures[0].name == "from other.json"
        && lock(&app.host.core).project.fixtures[0].name == "from other.json");
    // 保存先は変わらない(ローカルの「開く」と同じ)
    assert!(server.project_path.ends_with("project.json"));
    // 読み込む前の状態はサーバーのバックアップに残る
    app.host.request_server_files();
    run(&mut app, &mut server, &mut |app, _| app.host.server_files().is_some_and(|f| !f.backups.is_empty()));

    // このPCのファイルの中身を送って開く
    let local = std::env::temp_dir().join(format!("dmxctl-remote-local-{}.json", std::process::id()));
    let mut mine = Project::starter(&lib);
    mine.fixtures[0].name = "from client pc".into();
    mine.save(&local).unwrap();
    app.host.open_local_file(&local);
    run(&mut app, &mut server, &mut |app, server| lock(&server.core).project.fixtures[0].name == "from client pc"
        && lock(&app.host.core).project.fixtures[0].name == "from client pc");
    std::fs::remove_file(&local).unwrap();

    // 一覧に無いファイルは開けない
    app.host.open_server_file("/etc/passwd");
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        server.control_tick();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(lock(&server.core).project.fixtures[0].name, "from client pc");

    app.shutdown();
    drop(app);
    server.shutdown();
    drop(server);
    std::fs::remove_dir_all(dir).unwrap();
}
