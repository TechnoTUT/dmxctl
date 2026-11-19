//! 画面を出さずに、全部のタブを1フレームずつ描く。
//! 描画の途中で落ちる配線の誤りを、実機の画面を見なくても見つけるためのテスト。
//! 見た目そのものは、`DMXCTL_SHOT` で撮って確かめる。

use dmxctl::core::{fixture::Library, project::Project};
use dmxctl_backend::control::Launch;
use dmxctl_ui::app::{App, Tab};

/// テストごとに別のフォルダ(同時に走るテストが、同じ設定のファイルを書き合わないように)
fn project_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dmxctl-ui-smoke-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (lib, _) = Library::load(&dir.join("fixtures"));
    Project::starter(&lib).save(&dir.join("project.json")).unwrap();
    dir
}

#[test]
fn every_tab_draws_a_frame() {
    let dir = project_dir("tabs");
    let ctx = eframe::egui::Context::default();
    let mut app = App::with_options(&ctx, false, Launch {
        project: Some(dir.join("project.json")), output: false, inputs: false, autosave: false, ..Default::default()
    }).unwrap();
    for tab in Tab::ALL {
        app.tab = tab;
        for _ in 0..3 {
            let mut out = ctx.run_ui(Default::default(), |ui| app.frame(ui, None));
            // GPU無しで画面を回すので、テクスチャは捨てる
            out.textures_delta.clear();
        }
    }
}

/// 押下中にフォーカスを失い、そのまま隠れた(ui が呼ばれず logic だけが走る)ときも、レーザーの押下を解除する
#[test]
fn hidden_window_releases_the_laser_hold_through_logic() {
    use eframe::egui::{Context, RawInput};
    let dir = project_dir("laser");
    let ctx = Context::default();
    let mut app = App::with_options(&ctx, false, Launch {
        project: Some(dir.join("project.json")), output: false, inputs: false, autosave: false, ..Default::default()
    }).unwrap();
    let mut frame = eframe::Frame::_new_kittest();
    let shown = |focused: bool| RawInput { focused, ..Default::default() };
    // 手前で描いている間に、レーザータブの「押下中のみ出力」を押した
    let mut out = ctx.run_ui(shown(true), |ui| app.frame(ui, None));
    out.textures_delta.clear();
    app.laser_gui_hold = true;
    dmxctl::core::engine::lock(&app.host.core).set_laser_hold(true);
    // フォーカスがあるうちは、logic だけが走っても押したまま
    let _ = ctx.run_logic(&shown(true), |ctx| eframe::App::logic(&mut app, ctx, &mut frame));
    assert!(dmxctl::core::engine::lock(&app.host.core).rt.laser_hold);
    // フォーカスを失って隠れた: ui は呼ばれず logic だけ。InputState::focused は前回の true のまま残る
    let _ = ctx.run_logic(&shown(false), |ctx| {
        assert!(ctx.input(|i| i.focused), "前回の描画の値が残っている(この経路の前提)");
        eframe::App::logic(&mut app, ctx, &mut frame)
    });
    assert!(!app.laser_gui_hold);
    assert!(!dmxctl::core::engine::lock(&app.host.core).rt.laser_hold, "隠れたまま押下を解除する");
}
