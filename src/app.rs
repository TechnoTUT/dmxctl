//! アプリ本体(ウィンドウ枠・タブ・保存)。各タブの中身は ui_*.rs。

use crate::engine::{lock, Core, Engine, Shared};
use crate::fixture::{Library, Range};
use crate::midi::{self, Midi};
use crate::project::*;
use eframe::egui::{self, Align, Color32, Layout, RichText};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Control,
    Map,
    Patch,
    Profiles,
    Pairs,
    Scenes,
    Effects,
    Sequence,
    Midi,
    Output,
    Monitor,
}

const TABS: [(Tab, &str); 11] = [
    (Tab::Control, "コントロール"),
    (Tab::Map, "マップ"),
    (Tab::Patch, "パッチ"),
    (Tab::Profiles, "機材定義"),
    (Tab::Pairs, "ペア"),
    (Tab::Scenes, "シーン"),
    (Tab::Effects, "エフェクト"),
    (Tab::Sequence, "BPM/チェイス"),
    (Tab::Midi, "MIDI/OSC"),
    (Tab::Output, "出力"),
    (Tab::Monitor, "モニター"),
];

pub const RED: Color32 = Color32::from_rgb(220, 70, 70);
pub const WARN: Color32 = Color32::from_rgb(255, 140, 90);

pub struct App {
    pub core: Shared,
    engine: Engine,
    pub midi: Midi,
    pub tab: Tab,
    pub project_path: PathBuf,
    last_save: Instant,
    pub notice: String,
    // 各タブのUI状態
    pub add_profile: usize,
    pub scene_skip: HashSet<u32>,
    pub scene_name: String,
    pub scene_fade: f32,
    pub monitor_universe: usize,
    pub midi_ports: Vec<String>,
    pub midi_choice: String,
    pub serial_ports: Vec<(String, String)>,
    pub d2xx: Result<Vec<String>, String>,
    pub new_group: String,
    pub map_sel: Option<crate::apcmini::Ctl>,
    /// コントロール画面で隠しているカテゴリ
    pub cat_hidden: HashSet<String>,
    /// シーン保存の対象から外すカテゴリ
    pub scene_skip_cats: HashSet<String>,
    pub pe: crate::ui_profile::ProfileEd,
    pub osc: Option<crate::osc::OscServer>,
    /// OSCの受信ポートの編集中の値(「適用」で反映)
    pub osc_port_edit: u16,
    /// 位置プリセットの保存欄
    pub pos_name: String,
    pub pos_fade: f32,
    /// コントロールタブの色ボタンの操作先(全体/個別スイッチ・メイン色・サブ色)
    pub color_edit: crate::color::ColorTarget,
    /// キーボードのF/Gでフラッシュ中(離したら止める)
    key_flash: bool,
    pub ctx: egui::Context,
    /// 診断表示(環境変数 DMXCTL_PERF=1): 描画1回あたりの時間
    perf: Option<Perf>,
}

struct Perf {
    frames: u32,
    ui_time: Duration,
    since: Instant,
    text: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);

        let (lib, lib_errors) = Library::load(&config_dir().join("fixtures"));
        let lib = Arc::new(lib);
        let path = default_project_path();
        let mut notice = lib_errors.join(" / ");
        // 起動のたびに、前回の保存ファイルをバックアップに残す(うっかり消した割り当てなどを戻せるように)
        backup_to(&path, &backup_dir(), BACKUP_KEEP);
        let mut project = match Project::load(&path) {
            Ok(p) => p,
            Err(_) if !path.exists() => Project::starter(&lib),
            Err(e) => {
                let saved = Project::backup_broken(&path).map(|p| format!(" 元のファイルは {} に退避しました。", p.display())).unwrap_or_default();
                notice = format!("プロジェクトを読めませんでした({e})。初期状態で起動します。{saved}");
                Project::starter(&lib)
            }
        };
        // 全体光量を右端のフェーダーに初期割り当て(既存のプロジェクトにも、一度だけ)
        if project.seed_master_binding() == Some(false) {
            notice = format!("{notice} 右端のフェーダーは別の用途で使われているため、全体光量は未割り当てです(マップで割り当てられます)");
        }
        let midi_choice = project.midi_input.clone();
        let core: Shared = Arc::new(Mutex::new(Core::new(project, lib)));
        // 初回のoutput_gen(0)でエンジンが出力を組み立てる
        let engine = Engine::spawn(core.clone());

        let mut app = App {
            core,
            engine,
            midi: Midi::default(),
            tab: Tab::Control,
            project_path: path,
            last_save: Instant::now(),
            notice,
            add_profile: 0,
            scene_skip: HashSet::new(),
            scene_name: String::new(),
            scene_fade: 1.0,
            monitor_universe: 0,
            midi_ports: midi::list_ports(),
            midi_choice,
            serial_ports: crate::opendmx::list_serial_ports(),
            d2xx: Ok(Vec::new()),
            new_group: String::new(),
            map_sel: None,
            cat_hidden: HashSet::new(),
            scene_skip_cats: HashSet::new(),
            pe: Default::default(),
            osc: None,
            osc_port_edit: 9000,
            pos_name: String::new(),
            pos_fade: 0.5,
            color_edit: crate::color::ColorTarget::Scope,
            key_flash: false,
            ctx: cc.egui_ctx.clone(),
            perf: std::env::var_os("DMXCTL_PERF").map(|_| Perf { frames: 0, ui_time: Duration::ZERO, since: Instant::now(), text: String::new() }),
        };
        // 前回のMIDI機器、なければAPC mini が見つかれば自動接続。
        // 環境変数 DMXCTL_NO_MIDI=1 なら自動接続しない(本番のアプリが使っているコントローラーを、2つ目のアプリが奪わないように)
        let auto_midi = std::env::var_os("DMXCTL_NO_MIDI").is_none();
        if !auto_midi {
            app.midi_choice.clear();
        } else if app.midi_choice.is_empty() || !app.midi_ports.contains(&app.midi_choice) {
            if let Some(p) = crate::apcmini::find_port(&app.midi_ports) {
                app.midi_choice = p;
            }
        }
        if !app.midi_choice.is_empty() && app.midi_ports.contains(&app.midi_choice) {
            let name = app.midi_choice.clone();
            match app.midi.connect(&app.core, cc.egui_ctx.clone(), &name) {
                Ok(()) => lock(&app.core).project.midi_input = name,
                Err(e) => app.notice = e,
            }
        }
        app.apply_services();
        app
    }

    /// 設定に合わせて、Ableton Link・OSC受信・MIDIクロック用の入力を開始/停止する
    pub fn apply_services(&mut self) {
        let (osc_on, port, src, clock_port) = {
            let mut c = lock(&self.core);
            c.ensure_link();
            (c.project.osc_enabled, c.project.osc_port, c.project.tempo_source, c.project.clock_input.clone())
        };
        self.osc_port_edit = port;
        let want = osc_on.then_some(port);
        if self.osc.as_ref().map(|s| s.port) != want {
            self.osc = None;
            match want {
                Some(port) => match crate::osc::OscServer::start(self.core.clone(), Some(self.ctx.clone()), port) {
                    Ok(s) => self.osc = Some(s),
                    Err(e) => {
                        self.notice = format!("OSC: {e}");
                        lock(&self.core).rt.osc_status = e;
                    }
                },
                None => lock(&self.core).rt.osc_status = "停止中".into(),
            }
        }
        // MIDIクロックを、メインの入力とは別のポートから受け取る
        let want_clock = (src == TempoSource::MidiClock && !clock_port.is_empty() && self.midi.connected.as_deref() != Some(clock_port.as_str())).then_some(clock_port);
        if self.midi.clock_connected != want_clock {
            self.midi.disconnect_clock();
            if let Some(name) = want_clock {
                if let Err(e) = self.midi.connect_clock(&self.core, &name) {
                    self.notice = format!("MIDIクロック: {e}");
                }
            }
        }
    }

    pub fn save(&mut self) {
        let project = {
            let mut c = lock(&self.core);
            c.dirty = false;
            c.project.clone()
        };
        if let Err(e) = project.save(&self.project_path) {
            self.notice = format!("保存に失敗しました: {e}");
            lock(&self.core).dirty = true;
        }
        self.last_save = Instant::now();
    }

    fn load_from(&mut self, path: PathBuf) {
        // 読み込むと今の内容は置き換わる(自動保存で上書きされる)ので、先に保存してバックアップに残す
        self.save();
        backup_to(&self.project_path, &backup_dir(), BACKUP_KEEP);
        match Project::load(&path) {
            Ok(p) => {
                let mut c = lock(&self.core);
                c.project = p;
                c.rt.fades.clear();
                c.rt.pad_state.clear();
                c.rt.toggle_state.clear();
                c.rt.learn = None;
                c.rt.osc_learn = None;
                c.rt.page = 0;
                c.apply_outputs_changed();
                drop(c);
                self.apply_services();
                self.notice = format!("読み込みました: {}", path.display());
            }
            Err(e) => self.notice = format!("読み込みに失敗しました: {e}"),
        }
    }

    fn autosave(&mut self) {
        if self.last_save.elapsed() > Duration::from_secs(5) {
            let dirty = lock(&self.core).dirty;
            if dirty {
                self.save();
            } else {
                self.last_save = Instant::now();
            }
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            for (t, label) in TABS {
                ui.selectable_value(&mut self.tab, t, label);
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let bo = lock(&self.core).rt.blackout;
                let (text, fill) = if bo {
                    ("BLACKOUT中 - 解除 (Esc)", RED)
                } else {
                    ("BLACKOUT (Esc)", Color32::from_rgb(90, 35, 35))
                };
                if ui.add(egui::Button::new(RichText::new(text).strong().color(Color32::WHITE)).fill(fill)).clicked() {
                    lock(&self.core).rt.blackout = !bo;
                }
                if ui.button("バックアップから戻す").on_hover_text("起動のたび・別のファイルを開く前に、自動でバックアップを残しています(最新20個)。選んだものに戻します(戻す前の状態もバックアップされます)").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("プロジェクト", &["json"]).set_directory(backup_dir()).pick_file() {
                        self.load_from(p);
                    }
                }
                if ui.button("開く").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("プロジェクト", &["json"]).pick_file() {
                        self.load_from(p);
                    }
                }
                if ui.button("別名で保存").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("プロジェクト", &["json"]).set_file_name("show.json").save_file() {
                        self.project_path = p;
                        self.save();
                    }
                }
                if ui.button("保存").clicked() {
                    self.save();
                }
            });
        });
        self.transport_row(ui);
        ui.add_space(4.0);
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let (midi, last, outs, dirty) = {
            let c = lock(&self.core);
            (c.rt.midi_status.clone(), c.rt.last_midi.clone(), c.rt.output_status.clone(), c.dirty)
        };
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("MIDI: {midi}"));
            if !last.is_empty() {
                ui.label(RichText::new(format!("最後の入力: {last}")).weak());
            }
            ui.separator();
            if outs.is_empty() {
                ui.label(RichText::new("出力: 未設定(「出力」タブで追加)").color(WARN));
            }
            for (i, s) in outs.iter().enumerate() {
                ui.label(format!("出力{}: {s}", i + 1));
            }
            if dirty {
                ui.label(RichText::new("(未保存)").weak());
            }
            if let Some(p) = &self.perf {
                ui.separator();
                ui.label(RichText::new(&p.text).weak());
            }
            if !self.notice.is_empty() {
                ui.separator();
                ui.label(RichText::new(&self.notice).color(WARN));
            }
        });
    }
}

impl App {
    fn ui_timed(&mut self, ui: &mut egui::Ui) {
        let t_frame = Instant::now();
        self.ui_body(ui);
        if let Some(p) = self.perf.as_mut() {
            p.frames += 1;
            p.ui_time += t_frame.elapsed();
            let secs = p.since.elapsed().as_secs_f64();
            if secs >= 2.0 {
                p.text = format!("描画 {:.1}回/秒, ui() {:.2}ms/回", p.frames as f64 / secs, p.ui_time.as_secs_f64() * 1000.0 / p.frames.max(1) as f64);
                p.frames = 0;
                p.ui_time = Duration::ZERO;
                p.since = Instant::now();
            }
        }
    }
}

/// キーボードショートカットの一覧(画面の説明用)
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("Space", "TAP(タップテンポ)"),
    ("R", "頭出し(小節の頭へ)"),
    ("1〜8", "パッドのページ"),
    ("← / →", "前 / 次のページ"),
    ("F(押している間)", "フラッシュ(白で全開)"),
    ("G(押している間)", "フラッシュ(今の色のまま全開)"),
    ("S", "メイン⇄サブ入れ替え"),
    ("A", "拍で自動入れ替え ON/OFF"),
    ("D", "暗転フェード(暗転⇄復帰)"),
    ("Esc", "ブラックアウト ON/OFF"),
    (if cfg!(target_os = "macos") { "⌘S" } else { "Ctrl+S" }, "保存"),
];

impl App {
    /// キーボードショートカット。文字を入力している間は、Escと Ctrl+S 以外は効かない
    fn shortcuts(&mut self, ui: &egui::Ui, focused: bool) {
        let typing = ui.ctx().egui_wants_keyboard_input();
        let events = ui.input(|i| i.events.clone());
        let mut save = false;
        {
            let mut c = lock(&self.core);
            for ev in events {
                let egui::Event::Key { key, physical_key, pressed, repeat, modifiers } = ev else { continue };
                // 日本語入力中でも効くよう、キーの位置で判定する
                let key = physical_key.unwrap_or(key);
                if !pressed {
                    if matches!(key, egui::Key::F | egui::Key::G) && self.key_flash {
                        c.rt.flash = None;
                        self.key_flash = false;
                    }
                    continue;
                }
                if repeat {
                    continue;
                }
                match key {
                    egui::Key::Escape => c.rt.blackout = !c.rt.blackout,
                    egui::Key::S if modifiers.command => save = true,
                    _ if typing || modifiers.command || modifiers.alt => {}
                    egui::Key::Space => c.tap(),
                    egui::Key::R => c.resync(),
                    egui::Key::ArrowLeft => c.rt.page = (c.rt.page + PAGES - 1) % PAGES,
                    egui::Key::ArrowRight => c.rt.page = (c.rt.page + 1) % PAGES,
                    egui::Key::F | egui::Key::G => {
                        c.rt.flash = Some(key == egui::Key::F);
                        self.key_flash = true;
                    }
                    egui::Key::S => c.swap_palette(),
                    egui::Key::A => {
                        let on = !c.project.palette.auto;
                        c.set_palette_auto(on);
                    }
                    egui::Key::D => {
                        let secs = c.project.fade_secs;
                        c.toggle_fade(secs);
                    }
                    k => {
                        let pages = [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5, egui::Key::Num6, egui::Key::Num7, egui::Key::Num8];
                        if let Some(p) = pages.iter().position(|x| *x == k) {
                            c.rt.page = (p as u8).min(PAGES - 1);
                        }
                    }
                }
            }
            // キーを押したまま他のウィンドウへ移ると離したことが届かないので、フラッシュを止める
            if !focused && self.key_flash {
                c.rt.flash = None;
                self.key_flash = false;
            }
        }
        if save {
            self.save();
            self.notice = "保存しました".into();
        }
    }

    fn ui_body(&mut self, ui: &mut egui::Ui) {
        // DMXの出力は別スレッドなので、画面の更新頻度は出力に影響しない。
        // 普段は操作時とMIDI入力時だけ描画し、フェード中/モニター表示中だけ30fpsで更新する(グラフィックPCと同居するためCPUを節約)
        // 拍の表示は、次の拍の頭で1回だけ更新する(毎フレーム描き直さない)
        let (busy, to_beat, animated) = {
            let c = lock(&self.core);
            let p = &c.project;
            // 色や値がひとりでに変わっているか(マップの「今の色」を追従させるため)
            let animated = p.effects.iter().any(|e| e.enabled) || p.pixel_fx.iter().any(|e| e.enabled) || p.chases.iter().any(|e| e.enabled) || p.palette.auto;
            (self.tab == Tab::Monitor || !c.rt.fades.is_empty() || c.fading(), (1.0 - c.rt.beat.rem_euclid(1.0)) * 60.0 / p.bpm.max(1.0) as f64, animated)
        };
        // 他のアプリ(映像など)を前に出して使っているとき(このウィンドウが前面でないとき)は、表示の更新をさらに減らす
        let focused = ui.input(|i| i.focused);
        let mut idle = if focused { Duration::from_millis(500).min(Duration::from_secs_f64(to_beat.max(0.0) + 0.01)) } else { Duration::from_millis(1000) };
        // マップ(操作画面)は、エフェクトなどで色が動いている間は4fpsで「今の色」を追従する
        if focused && animated && self.tab == Tab::Map {
            idle = idle.min(Duration::from_millis(250));
        }
        // モニター表示中/フェード中は10fps。描画1回ぶんのCPUは内容によらず約12〜16msかかるので、回数を減らすのが一番効く
        // (DMXの出力は画面と別スレッドで40Hz固定なので、画面の更新頻度は出力に影響しない)
        ui.ctx().request_repaint_after(if busy { Duration::from_millis(if focused { 100 } else { 250 }) } else { idle });
        self.shortcuts(ui, focused);
        self.autosave();

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Control => self.tab_control(ui),
            Tab::Map => self.tab_map(ui),
            Tab::Patch => self.tab_patch(ui),
            Tab::Profiles => self.tab_profiles(ui),
            Tab::Pairs => self.tab_pairs(ui),
            Tab::Scenes => self.tab_scenes(ui),
            Tab::Effects => self.tab_effects(ui),
            Tab::Sequence => self.tab_sequence(ui),
            Tab::Midi => self.tab_midi(ui),
            Tab::Output => self.tab_output(ui),
            Tab::Monitor => self.tab_monitor(ui),
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.ui_timed(ui);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        // 終了時は先にブラックアウトを数フレーム送ってから止める(レーザーを出しっぱなしにしない)
        {
            let mut c = lock(&self.core);
            c.rt.blackout = true;
            c.project.midi_input = self.midi.connected.clone().unwrap_or_default();
        }
        std::thread::sleep(Duration::from_millis(200));
        self.osc = None;
        lock(&self.core).rt.link = None;
        self.midi.disconnect_clock();
        self.midi.disconnect();
        self.engine.stop();
        self.save();
    }
}

/// 日本語表示用にWindows標準フォントを追加する
fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    // OSの標準の日本語フォント(Windows: メイリオ等 / Mac: ヒラギノ / Linux: Noto CJK)
    const JP_FONTS: &[&str] = &[
        "C:/Windows/Fonts/meiryo.ttc",
        "C:/Windows/Fonts/YuGothM.ttc",
        "C:/Windows/Fonts/msgothic.ttc",
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/Library/Fonts/Arial Unicode.ttf",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    ];
    for path in JP_FONTS {
        if let Ok(bytes) = std::fs::read(path) {
            fonts.font_data.insert("jp".into(), Arc::new(egui::FontData::from_owned(bytes)));
            // 通常の文字は日本語フォントを先頭にして字の太さを揃える。等幅(数値表示)は既定フォントを優先し、日本語だけ補う
            if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
                list.insert(0, "jp".into());
            }
            if let Some(list) = fonts.families.get_mut(&egui::FontFamily::Monospace) {
                list.push("jp".into());
            }
            break;
        }
    }
    ctx.set_fonts(fonts);
}

// ------------------------------------------------------------ 対象セレクタ用の共通部品

pub struct FxMeta {
    pub id: u32,
    pub name: String,
    pub profile: String,
    pub mode: usize,
}

/// UI描画中にCoreを借りたままでも使えるよう、選択肢の情報だけを複製したもの
pub struct SelMeta {
    pub fixtures: Vec<FxMeta>,
    pub groups: Vec<Group>,
    pub profiles: Vec<(String, String)>,
}

impl SelMeta {
    pub fn new(p: &Project, lib: &Library) -> Self {
        let fixtures: Vec<FxMeta> = p
            .fixtures
            .iter()
            .map(|f| FxMeta { id: f.id, name: f.name.clone(), profile: f.profile.clone(), mode: f.mode })
            .collect();
        let mut profiles: Vec<(String, String)> = Vec::new();
        for f in &fixtures {
            if !profiles.iter().any(|(id, _)| *id == f.profile) {
                let name = lib.get(&f.profile).map(|x| x.name.clone()).unwrap_or_else(|| f.profile.clone());
                profiles.push((f.profile.clone(), name));
            }
        }
        SelMeta { fixtures, groups: p.groups.clone(), profiles }
    }

    pub fn label(&self, sel: &Selector) -> String {
        match sel {
            Selector::All => "すべての機材".into(),
            Selector::Fixture(id) => self.fixtures.iter().find(|f| f.id == *id).map(|f| f.name.clone()).unwrap_or_else(|| "(削除された機材)".into()),
            Selector::Group(g) => format!("グループ: {g}"),
            Selector::Profile(p) => format!("同機種すべて: {}", self.profiles.iter().find(|(id, _)| id == p).map(|(_, n)| n.as_str()).unwrap_or(p)),
        }
    }

    fn members(&self, sel: &Selector) -> Vec<&FxMeta> {
        self.fixtures
            .iter()
            .filter(|f| match sel {
                Selector::All => true,
                Selector::Fixture(id) => f.id == *id,
                Selector::Group(g) => self.groups.iter().any(|x| x.name == *g && x.fixtures.contains(&f.id)),
                Selector::Profile(p) => f.profile == *p,
            })
            .collect()
    }

    /// 選択範囲の機材が持つ機能名(attr)の一覧
    pub fn attrs(&self, lib: &Library, sel: &Selector) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for f in self.members(sel) {
            if let Some(m) = lib.mode(&f.profile, f.mode) {
                for c in m.channels.iter().filter(|c| c.fine_of.is_none()) {
                    // LEDバーのピクセル(red.1, red.2 ...)は「red」(全ピクセル)にまとめる
                    let a = crate::fixture::split_pixel(&c.attr).0.to_string();
                    if !out.contains(&a) {
                        out.push(a);
                    }
                }
            }
        }
        out
    }

    /// 選択範囲の最初の機材が持つ、そのattrのレンジ一覧(パッドに色/ゴボを割り当てる時の候補)
    pub fn ranges(&self, lib: &Library, sel: &Selector, attr: &str) -> Vec<Range> {
        for f in self.members(sel) {
            if let Some(m) = lib.mode(&f.profile, f.mode) {
                if let Some(ch) = m.find_attr(attr) {
                    return m.channels[ch].ranges.clone();
                }
            }
        }
        Vec::new()
    }
}

pub fn selector_combo(ui: &mut egui::Ui, salt: &str, sel: &mut Selector, meta: &SelMeta) -> bool {
    let mut changed = false;
    egui::ComboBox::from_id_salt(salt).selected_text(meta.label(sel)).width(190.0).show_ui(ui, |ui| {
        let mut pick = |ui: &mut egui::Ui, s: Selector, label: String| {
            if ui.selectable_label(*sel == s, label).clicked() && *sel != s {
                *sel = s;
                changed = true;
            }
        };
        pick(ui, Selector::All, "すべての機材".into());
        for f in &meta.fixtures {
            pick(ui, Selector::Fixture(f.id), f.name.clone());
        }
        for g in &meta.groups {
            pick(ui, Selector::Group(g.name.clone()), format!("グループ: {}", g.name));
        }
        for (id, name) in &meta.profiles {
            pick(ui, Selector::Profile(id.clone()), format!("同機種すべて: {name}"));
        }
    });
    changed
}
