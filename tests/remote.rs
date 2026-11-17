//! サーバー(Host + --remote-listen)と、画面(App)のクライアント(connect)を同じプロセスでつなぐ
use dmxctl::{control::Launch, engine::lock, fixture::Library, host::Host, live::LiveOp, project::Project};
use std::time::{Duration, Instant};

fn project_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dmxctl-remote-{}-{}", std::process::id(), Instant::now().elapsed().as_nanos()));
    std::fs::create_dir_all(&dir).unwrap();
    let (lib, _) = Library::load(&dir.join("fixtures"));
    Project::starter(&lib).save(&dir.join("project.json")).unwrap();
    dir
}

#[test]
fn a_gui_client_follows_the_server_and_sends_operations() {
    let dir = project_dir();
    let server_ctx = eframe::egui::Context::default();
    let mut server = Host::start(&server_ctx, Launch {
        project: Some(dir.join("project.json")), output: false, inputs: false, autosave: false,
        remote_listen: Some("127.0.0.1:0".parse().unwrap()), ..Default::default()
    }).unwrap();
    let address = server.remote.as_ref().unwrap().address;

    let ctx = eframe::egui::Context::default();
    let mut app = dmxctl::app::App::with_options(&ctx, false, Launch {
        inputs: false, connect: Some(address.to_string()), ..Default::default()
    }).unwrap();
    assert!(app.host.is_client());
    let frames = |app: &mut dmxctl::app::App, until: &mut dyn FnMut(&mut dmxctl::app::App) -> bool| {
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
