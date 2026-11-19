//! 画面を出さずに、全部のタブを1フレームずつ描く。
//! 描画の途中で落ちる配線の誤りを、実機の画面を見なくても見つけるためのテスト。
//! 見た目そのものは、`DMXCTL_SHOT` で撮って確かめる。

use dmxctl::core::{fixture::Library, project::Project};
use dmxctl_backend::control::Launch;
use dmxctl_ui::app::{App, Tab};

fn project_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dmxctl-ui-smoke-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (lib, _) = Library::load(&dir.join("fixtures"));
    Project::starter(&lib).save(&dir.join("project.json")).unwrap();
    dir
}

#[test]
fn every_tab_draws_a_frame() {
    let dir = project_dir();
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
