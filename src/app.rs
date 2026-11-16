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

/// デスクトップ環境の無い画面(KMSDRM)で動いているか。ファイルを選ぶ画面(OSのダイアログ)が出せず、
/// ウィンドウの閉じるボタンも無いので、ファイルメニューの中身を変える。
/// App::new で eguiのContextに覚えさせる(Appを持たない画面の部品からも読めるように)
fn no_desktop_id() -> egui::Id {
    egui::Id::new("dmxctl_no_desktop")
}

pub fn no_desktop(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(no_desktop_id())).unwrap_or(false)
}

/// 動いているもの(拍で動く点・色の追従・フェード・モニター)を描き直す間隔。約30fps。
/// (eguiは頼んだ時刻より1フレーム分(1/60秒)早めに描くので、33msに1/60秒を足して頼む)
pub const SMOOTH: Duration = Duration::from_millis(50);

/// このウィンドウが前面か。前面でない(映像ソフトなどを前に出している)ときは、描き直しを減らしてCPUを譲る。
/// デスクトップ環境の無い画面(KMSDRM)では他のアプリが前に出ることは無いので、いつも前面
pub fn in_front(ui: &egui::Ui) -> bool {
    no_desktop(ui.ctx()) || ui.input(|i| i.focused)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Control,
    Map,
    Patch,
    Profiles,
    Pairs,
    Laser,
    Keys,
    Auto,
    Scenes,
    Effects,
    Sequence,
    Midi,
    Output,
    Monitor,
}

/// タブ。先頭の`LIVE_TABS`個が本番で使う画面、残りが準備・設定
const TABS: [(Tab, &str); 14] = [
    (Tab::Map, "MIDI"),
    (Tab::Keys, "キーボード"),
    (Tab::Control, "コントロール"),
    (Tab::Effects, "エフェクト"),
    (Tab::Scenes, "シーン"),
    (Tab::Sequence, "BPM/チェイス"),
    (Tab::Auto, "オート"),
    (Tab::Pairs, "ペア"),
    (Tab::Laser, "レーザー"),
    (Tab::Patch, "パッチ"),
    (Tab::Profiles, "機材定義"),
    (Tab::Output, "出力"),
    (Tab::Midi, "MIDI設定"),
    (Tab::Monitor, "モニター"),
];
const LIVE_TABS: usize = 8;

pub const RED: Color32 = crate::theme::DANGER;
pub const WARN: Color32 = crate::theme::WARN;

pub struct App {
    pub(crate) launch: crate::control::Launch,
    control: Option<crate::control::Host>,
    shutdown_requested: bool,
    shutdown_done: bool,
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
    /// モニターで、使っていないチャンネルも512chまで並べる
    pub monitor_all: bool,
    pub midi_ports: Vec<String>,
    pub midi_choice: String,
    pub serial_ports: Vec<(String, String)>,
    pub d2xx: Result<Vec<String>, String>,
    pub new_group: String,
    pub map_sel: Option<crate::apcmini::Ctl>,
    /// レーザータブで、動いてよい範囲を編集中のレーザー
    pub laser_edit: Option<u32>,
    /// コントロール画面で隠しているカテゴリ
    pub cat_hidden: HashSet<String>,
    /// シーン保存の対象から外すカテゴリ
    pub scene_skip_cats: HashSet<String>,
    pub pe: crate::ui_profile::ProfileEd,
    pub osc: Option<crate::osc::OscServer>,
    /// ブラウザの操作パネル
    pub web: Option<crate::web::WebServer>,
    /// 操作パネルの状態(画面の表示用)
    pub web_status: String,
    /// NDIの受信(設定で有効にしている間だけ動かす)
    ndi: Option<crate::ndi::NdiService>,
    audio: Option<crate::audio::AudioService>,
    pub audio_ports: Vec<String>,
    /// OSCの受信ポートの編集中の値(「適用」で反映)
    pub osc_port_edit: u16,
    /// 位置プリセットの保存欄
    pub pos_name: String,
    pub pos_fade: f32,
    /// コントロールタブの色ボタンの操作先(全体/個別スイッチ・メイン色・サブ色)
    pub color_edit: crate::color::ColorTarget,
    /// キーボードのF/Gでフラッシュ中(離したら止める)
    /// キーボードタブの絵をマウスで押しているキー
    pub(crate) key_mouse: Option<u8>,
    /// キーボードタブで、色の段(Shift)を出したままにする
    pub(crate) key_color_view: bool,
    pub ctx: egui::Context,
    /// 診断表示(環境変数 DMXCTL_PERF=1): 描画1回あたりの時間
    perf: Option<Perf>,
    /// 確認用の画面保存(環境変数 DMXCTL_SHOT)
    shot: Option<crate::shot::Shot>,
}

struct Perf {
    frames: u32,
    ui_time: Duration,
    /// 前のフレームのCPU時間(図形の変換・描画の準備を含む)の合計
    frame_cpu: f32,
    since: Instant,
    text: String,
}

impl App {
    /// 画面の土台(eframe / KMSDRM)によらず、eguiのContextだけで作る。`no_desktop`=デスクトップ環境の無い画面(KMSDRM)
    pub fn new(egui_ctx: &egui::Context, no_desktop: bool) -> Self {
        Self::with_options(egui_ctx, no_desktop, crate::control::Launch::default()).expect("default app startup")
    }

    pub fn with_options(egui_ctx: &egui::Context, no_desktop: bool, options: crate::control::Launch) -> anyhow::Result<Self> {
        egui_ctx.data_mut(|d| d.insert_temp(no_desktop_id(), no_desktop));
        crate::theme::setup_fonts(egui_ctx);
        crate::theme::apply(egui_ctx);

        let path = options.project.clone().unwrap_or_else(default_project_path);
        let fixture_dir = if options.project.is_some() {
            path.parent().unwrap_or(std::path::Path::new(".")).join("fixtures")
        } else { config_dir().join("fixtures") };
        let (lib, lib_errors) = Library::load(&fixture_dir);
        let lib = Arc::new(lib);
        let mut notice = lib_errors.join(" / ");
        // 起動のたびに、前回の保存ファイルをバックアップに残す(うっかり消した割り当てなどを戻せるように)
        if options.autosave {
            let backups = if options.project.is_some() { path.parent().unwrap_or(std::path::Path::new(".")).join("backups") } else { backup_dir() };
            backup_to(&path, &backups, BACKUP_KEEP);
        }
        let mut project = if options.project.is_some() {
            anyhow::ensure!(lib_errors.is_empty(), "fixture errors: {}", lib_errors.join("; "));
            let text = std::fs::read_to_string(&path)?;
            let project: Project = serde_json::from_str(text.trim_start_matches('\u{feff}'))?;
            crate::commands::validate(&project, &lib)?;
            project
        } else { match Project::load_lenient(&path) {
            Ok((p, skipped)) if skipped.is_empty() => p,
            // 新しい版で保存したファイル: 読めない項目だけ既定値にして読む(元のファイルは別名で残す)
            Ok((p, skipped)) => {
                let saved = Project::backup_broken(&path).map(|p| format!("元のファイルは {} に残しました。", p.display())).unwrap_or_default();
                notice = format!("この版では読めない項目を既定値にしました({})。新しい版で保存したファイルかもしれません。{saved}", skipped.join(", "));
                p
            }
            // 初めての起動: おすすめのMIDI配置で始める
            Err(_) if !path.exists() => {
                let mut p = Project::starter(&lib);
                crate::layout::apply(&mut p, &lib);
                p
            }
            Err(e) => {
                let saved = Project::backup_broken(&path).map(|p| format!(" 元のファイルは {} に退避しました。", p.display())).unwrap_or_default();
                notice = format!("プロジェクトを読めませんでした({e})。初期状態で起動します。{saved}");
                Project::starter(&lib)
            }
        }};
        // 全体光量を右端のフェーダーに初期割り当て(既存のプロジェクトにも、一度だけ)
        if project.seed_master_binding() == Some(false) {
            notice = format!("{notice} 右端のフェーダーは別の用途で使われているため、全体光量は未割り当てです(MIDIタブで割り当てられます)");
        }
        let midi_choice = project.midi_input.clone();
        let core: Shared = Arc::new(Mutex::new(Core::new(project, lib)));
        // 初回のoutput_gen(0)でエンジンが出力を組み立てる
        let control = options.listen.map(|address|crate::control::Host::start(address,egui_ctx.clone())).transpose()?;
        let engine = Engine::spawn_with_output(core.clone(), options.output);

        let mut app = App {
            launch: options.clone(),
            control,
            shutdown_requested: false,
            shutdown_done: false,
            core,
            engine,
            midi: Midi::default(),
            tab: Tab::Map,
            project_path: path,
            last_save: Instant::now(),
            notice,
            add_profile: 0,
            scene_skip: HashSet::new(),
            scene_name: String::new(),
            scene_fade: 1.0,
            monitor_universe: 0,
            monitor_all: false,
            midi_ports: if options.inputs { midi::list_ports() } else { Vec::new() },
            midi_choice,
            serial_ports: if options.output { crate::opendmx::list_serial_ports() } else { Vec::new() },
            d2xx: Ok(Vec::new()),
            new_group: String::new(),
            map_sel: None,
            laser_edit: None,
            cat_hidden: HashSet::new(),
            scene_skip_cats: HashSet::new(),
            pe: Default::default(),
            osc: None,
            web: None,
            web_status: String::new(),
            ndi: None,
            audio: None,
            audio_ports: if options.inputs { crate::audio::input_devices() } else { Vec::new() },
            osc_port_edit: 9000,
            pos_name: String::new(),
            pos_fade: 0.5,
            color_edit: crate::color::ColorTarget::Scope,
            key_mouse: None,
            // 開発・確認用: DMXCTL_KEY_COLOR=1 で、キーボードタブを色の段で開く
            key_color_view: std::env::var_os("DMXCTL_KEY_COLOR").is_some(),
            ctx: egui_ctx.clone(),
            perf: std::env::var_os("DMXCTL_PERF").map(|_| Perf { frames: 0, ui_time: Duration::ZERO, frame_cpu: 0.0, since: Instant::now(), text: String::new() }),
            shot: crate::shot::Shot::from_env(),
        };
        // 確認用: DMXCTL_TAB=タブ名 で開くタブ、DMXCTL_UNIVERSE=番号 でモニターのユニバースを選ぶ
        if let Ok(name) = std::env::var("DMXCTL_TAB")
            && let Some((t, _)) = TABS.iter().find(|(_, label)| *label == name) {
            app.tab = *t;
        }
        if let Some(u) = std::env::var("DMXCTL_UNIVERSE").ok().and_then(|v| v.parse::<usize>().ok()) {
            app.monitor_universe = u.clamp(1, NUM_UNIVERSES) - 1;
        }
        if let Some(p) = std::env::var("DMXCTL_PAGE").ok().and_then(|v| v.parse::<u8>().ok()) {
            lock(&app.core).rt.page = p.clamp(1, PAGES) - 1;
        }
        // 前回のMIDI機器、なければAPC mini が見つかれば自動接続。
        // 環境変数 DMXCTL_NO_MIDI=1 なら自動接続しない(本番のアプリが使っているコントローラーを、2つ目のアプリが奪わないように)
        let auto_midi = options.inputs && std::env::var_os("DMXCTL_NO_MIDI").is_none();
        if !auto_midi {
            app.midi_choice.clear();
        } else if (app.midi_choice.is_empty() || !app.midi_ports.contains(&app.midi_choice))
            && let Some(p) = crate::apcmini::find_port(&app.midi_ports) {
            app.midi_choice = p;
        }
        if !app.midi_choice.is_empty() && app.midi_ports.contains(&app.midi_choice) {
            let name = app.midi_choice.clone();
            match app.midi.connect(&app.core, egui_ctx.clone(), &name) {
                Ok(()) => lock(&app.core).project.midi_input = name,
                Err(e) => app.notice = e,
            }
        }
        app.apply_services();
        Ok(app)
    }

    /// 設定に合わせて、Ableton Link・OSC受信・MIDIクロック用の入力を開始/停止する
    pub fn apply_services(&mut self) {
        if !self.launch.inputs {
            self.osc = None;
            self.web = None;
            self.ndi = None;
            self.audio = None;
            self.midi.disconnect_clock();
            self.midi.disconnect();
            lock(&self.core).rt.link = None;
            return;
        }
        let (osc_on, port, src, clock_port, ndi_on, audio_port, web_want) = {
            let mut c = lock(&self.core);
            c.ensure_link();
            (
                c.project.osc_enabled,
                c.project.osc_port,
                c.project.tempo_source,
                c.project.clock_input.clone(),
                c.project.ndi.enabled,
                c.project.audio_input.clone(),
                c.project.web_enabled.then_some(c.project.web_port),
            )
        };
        // ブラウザの操作パネル
        if self.web.as_ref().map(|w| w.port) != web_want {
            self.web = None;
            self.web_status = match web_want {
                Some(port) => match crate::web::WebServer::start(self.core.clone(), Some(self.ctx.clone()), port) {
                    Ok(w) => {
                        self.web = Some(w);
                        crate::web::url(port)
                    }
                    Err(e) => {
                        self.notice = format!("操作パネル: {e}");
                        e
                    }
                },
                None => String::new(),
            };
        }
        if ndi_on != self.ndi.is_some() {
            self.ndi = ndi_on.then(|| crate::ndi::NdiService::start(self.core.clone(), Some(self.ctx.clone())));
        }
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
        // 音声からのBPM検出(テンポの元が「音声」のときだけ入力を開く)
        let want_audio = (src == TempoSource::Audio).then(|| audio_port.clone());
        if self.audio.as_ref().map(|a| &a.device) != want_audio.as_ref() {
            self.audio = None;
            match want_audio {
                Some(dev) => self.audio = Some(crate::audio::AudioService::start(self.core.clone(), &dev)),
                None => lock(&self.core).rt.audio_status = "停止中".into(),
            }
        }
        // MIDIクロックを、メインの入力とは別のポートから受け取る
        let want_clock = (src == TempoSource::MidiClock && !clock_port.is_empty() && self.midi.connected.as_deref() != Some(clock_port.as_str())).then_some(clock_port);
        if self.midi.clock_connected != want_clock {
            self.midi.disconnect_clock();
            if let Some(name) = want_clock
                && let Err(e) = self.midi.connect_clock(&self.core, &name) {
                self.notice = format!("MIDIクロック: {e}");
            }
        }
    }

    pub fn save(&mut self) {
        if let Err(error) = self.save_checked() {
            self.notice = format!("保存に失敗しました: {error}");
        }
    }

    pub(crate) fn save_checked(&mut self) -> anyhow::Result<()> {
        let project = {
            let mut c = lock(&self.core);
            c.project.ensure_binding_ids();
            c.dirty = false;
            c.project.clone()
        };
        if let Err(e) = project.save(&self.project_path) {
            lock(&self.core).dirty = true;
            return Err(e);
        }
        self.last_save = Instant::now();
        Ok(())
    }

    fn load_from(&mut self, path: PathBuf) {
        // 読み込むと今の内容は置き換わる(自動保存で上書きされる)ので、先に保存してバックアップに残す
        self.save();
        backup_to(&self.project_path, &backup_dir(), BACKUP_KEEP);
        match Project::load(&path) {
            Ok(p) => {
                let mut c = lock(&self.core);
                c.release_remote_controls();
                c.project = p;
                c.project.ensure_binding_ids();
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
        if !self.launch.autosave { return; }
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
        use crate::theme;
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            brand(ui);
            ui.add_space(14.0);
            ui.spacing_mut().item_spacing.x = 2.0;
            for (k, (t, label)) in TABS.iter().enumerate() {
                if k == LIVE_TABS {
                    ui.add_space(8.0);
                    ui.separator();
                    ui.add_space(8.0);
                }
                if tab_button(ui, self.tab == *t, label).clicked() {
                    self.tab = *t;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.menu_button("ファイル ▼", |ui| {
                    if ui.button("保存").on_hover_text(if cfg!(target_os = "macos") { "⌘S" } else { "Ctrl+S" }).clicked() {
                        self.save();
                        self.notice = "保存しました".into();
                        ui.close();
                    }
                    if no_desktop(ui.ctx()) {
                        self.file_menu_no_desktop(ui);
                        return;
                    }
                    if ui.button("別名で保存…").clicked() {
                        ui.close();
                        if let Some(p) = rfd::FileDialog::new().add_filter("プロジェクト", &["json"]).set_file_name("show.json").save_file() {
                            self.project_path = p;
                            self.save();
                        }
                    }
                    if ui.button("開く…").clicked() {
                        ui.close();
                        if let Some(p) = rfd::FileDialog::new().add_filter("プロジェクト", &["json"]).pick_file() {
                            self.load_from(p);
                        }
                    }
                    ui.separator();
                    if ui.button("バックアップから戻す…").on_hover_text("起動のたび・別のファイルを開く前に、自動でバックアップを残しています(最新20個)。選んだものに戻します(戻す前の状態もバックアップされます)").clicked() {
                        ui.close();
                        if let Some(p) = rfd::FileDialog::new().add_filter("プロジェクト", &["json"]).set_directory(backup_dir()).pick_file() {
                            self.load_from(p);
                        }
                    }
                });
                if lock(&self.core).dirty {
                    ui.label(RichText::new("● 未保存").small().color(theme::WARN)).on_hover_text("変更は5秒ごとに自動で保存されます");
                }
            });
        });
        ui.add_space(6.0);
        self.transport_row(ui);
        ui.add_space(6.0);
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        use crate::theme;
        let (midi, last, outs, blt) = {
            let c = lock(&self.core);
            (c.rt.midi_status.clone(), c.rt.last_midi.clone(), c.rt.output_status.clone(), c.project.auto.enabled.then(|| c.blt_state()))
        };
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            if crate::engine::EMERGENCY.load(std::sync::atomic::Ordering::SeqCst) {
                theme::dot(ui, theme::DANGER);
                ui.label(RichText::new("安全停止中(アプリを再起動してください)").small().color(theme::DANGER));
                ui.separator();
            }
            let connected = self.midi.connected.is_some();
            theme::dot(ui, if connected { theme::OK } else { theme::TEXT_FAINT });
            ui.label(RichText::new(format!("MIDI {midi}")).small().color(theme::TEXT_DIM));
            if !last.is_empty() {
                ui.label(RichText::new(format!("最後の入力: {last}")).small().color(theme::TEXT_FAINT));
            }
            ui.separator();
            // 出力はまとめて1行に(詳しくはマウスを乗せると出る)
            let on: Vec<(usize, &String)> = outs.iter().enumerate().filter(|(_, s)| s.as_str() != "無効").collect();
            let bad = on.iter().filter(|(_, s)| output_has_error(s)).count();
            let color = if outs.is_empty() || bad > 0 { theme::WARN } else if on.is_empty() { theme::TEXT_FAINT } else { theme::OK };
            theme::dot(ui, color);
            let text = if !self.launch.output {
                "出力: シミュレーション(実機送信なし)".to_string()
            } else if outs.is_empty() {
                "出力: 未設定(「出力」タブで追加)".to_string()
            } else if bad > 0 {
                format!("出力 {}/{} ・ エラー {bad}件", on.len(), outs.len())
            } else {
                format!("出力 {}/{}", on.len(), outs.len())
            };
            let detail = outs.iter().enumerate().map(|(i, s)| format!("出力{}: {s}", i + 1)).collect::<Vec<_>>().join("\n");
            ui.label(RichText::new(text).small().color(if bad > 0 { theme::WARN } else { theme::TEXT_DIM })).on_hover_text(detail);
            // オート中は、Beat Link Trigger の様子(見つからなければ目立たせる。押すとオートタブへ)
            if let Some(state) = blt {
                use crate::engine::BltState;
                ui.separator();
                let (color, text) = match state {
                    BltState::Receiving => (theme::OK, "BLT 受信中".to_string()),
                    BltState::Waiting => (theme::TEXT_DIM, "BLT 再生待ち".to_string()),
                    BltState::Missing(_) => (theme::WARN, "⚠ BLTが見つかりません".to_string()),
                    BltState::Unknown => (theme::TEXT_FAINT, "BLT 待機中".to_string()),
                };
                theme::dot(ui, color);
                let resp = ui.add(egui::Label::new(RichText::new(text).small().color(if matches!(state, BltState::Missing(_)) { theme::WARN } else { theme::TEXT_DIM })).sense(egui::Sense::click()));
                if resp.on_hover_text(state.message()).clicked() {
                    self.tab = Tab::Auto;
                }
            }
            if let Some(p) = &self.perf {
                ui.separator();
                ui.label(RichText::new(&p.text).small().color(theme::TEXT_FAINT));
            }
            if !self.notice.is_empty() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("消す").clicked() {
                        self.notice.clear();
                    }
                    ui.add(egui::Label::new(RichText::new(&self.notice).small().color(theme::WARN)).truncate()).on_hover_text(&self.notice);
                });
            }
        });
        ui.add_space(1.0);
    }
}

/// 出力の状態の文字から、エラーかどうか
fn output_has_error(status: &str) -> bool {
    ["エラー", "失敗", "見つかりません", "開けません"].iter().any(|k| status.contains(k))
}

/// 左上のロゴ
fn brand(ui: &mut egui::Ui) {
    use crate::theme;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
    let p = ui.painter();
    let c = rect.center();
    // 3色の光が重なるマーク
    for (dx, dy, color) in [(-3.5, -2.5, Color32::from_rgb(255, 70, 110)), (3.5, -2.5, Color32::from_rgb(70, 140, 255)), (0.0, 3.5, theme::ACCENT)] {
        p.rect_filled(egui::Rect::from_center_size(c + egui::vec2(dx, dy), egui::vec2(9.5, 9.5)), 2.0, theme::alpha(color, 0.85));
    }
    ui.label(RichText::new("DMX").font(theme::bold(15.0)).color(theme::TEXT));
}

/// タブのボタン(選ばれているタブはうっすら色付きの角丸)
fn tab_button(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
    use crate::theme;
    let font = egui::FontId::proportional(13.5);
    let galley = ui.painter().layout_no_wrap(label.to_string(), font, theme::TEXT);
    let size = galley.size() + egui::vec2(18.0, 12.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let hovered = resp.hovered();
    let (fill, color) = if selected {
        (theme::alpha(theme::ACCENT, 0.15), theme::ACCENT)
    } else if hovered {
        (theme::WIDGET, theme::TEXT)
    } else {
        (Color32::TRANSPARENT, theme::TEXT_DIM)
    };
    ui.painter().rect_filled(rect, 7.0, fill);
    ui.painter().galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, color);
    resp
}

impl App {
    fn control_tick(&mut self) {
        if let Some(mut host) = self.control.take() {
            self.shutdown_requested |= host.poll(self);
            self.control = Some(host);
        }
    }

    /// Runs the same App and Engine as the GUI, without forcing BLT/auto settings.
    pub fn serve(options: crate::control::Launch) -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        let mut app = Self::with_options(&ctx, true, options)?;
        let address = app.control.as_ref().map(|host|host.address);
        println!("{}",serde_json::json!({"ready":true,"listen":address,"project":app.project_path,
            "output_enabled":app.launch.output,"inputs_enabled":app.launch.inputs,"autosave":app.launch.autosave}));
        while !app.shutdown_requested {
            app.control_tick();
            app.autosave();
            if !app.shutdown_requested { std::thread::sleep(crate::engine::TICK); }
        }
        app.shutdown();
        Ok(())
    }

    fn ui_timed(&mut self, ui: &mut egui::Ui) {
        let t_frame = Instant::now();
        self.ui_body(ui);
        if let Some(p) = self.perf.as_mut() {
            p.frames += 1;
            p.ui_time += t_frame.elapsed();
            let secs = p.since.elapsed().as_secs_f64();
            if secs >= 2.0 {
                let n = p.frames.max(1) as f64;
                p.text = format!("描画 {:.1}回/秒, ui() {:.2}ms/回, フレーム全体 {:.2}ms/回", p.frames as f64 / secs, p.ui_time.as_secs_f64() * 1000.0 / n, p.frame_cpu as f64 * 1000.0 / n);
                eprintln!("{}", p.text);
                p.frames = 0;
                p.ui_time = Duration::ZERO;
                p.frame_cpu = 0.0;
                p.since = Instant::now();
            }
        }
    }
}

/// キーボードショートカットの一覧(画面の説明用)
pub const SHORTCUTS: &[(&str, &str)] = &[
    ("Space", "TAP(タップテンポ)"),
    ("Enter", "頭出し(小節の頭へ)"),
    ("← / →", "前 / 次のページ(APC)"),
    ("Esc", "ブラックアウト ON/OFF"),
    ("数字・英字の4段", "キーボードの割り当て(「キーボード」タブに絵があります)"),
    ("Shift+数字・英字", "色(メイン色・サブ色・入れ替え・系統)"),
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
                    // 押したキーは、文字入力中でも必ず離す(押している間だけのフラッシュなどが残らないように)
                    // Shiftを先に離しても、押したときの割り当て(色の段など)を離す
                    if let Some(code) = crate::keys::from_key(key) {
                        for k in [code, code + crate::keys::SHIFT] {
                            if c.rt.keys_down.contains(&k) {
                                c.handle_key(k, false);
                            }
                        }
                    }
                    continue;
                }
                if repeat {
                    continue;
                }
                match key {
                    egui::Key::Escape => c.rt.blackout = !c.rt.blackout,
                    egui::Key::S if modifiers.command => save = true,
                    // デスクトップ環境の無い画面には閉じるボタンが無いので、Ctrl+Qで終了
                    egui::Key::Q if modifiers.command && no_desktop(ui.ctx()) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                    _ if typing || modifiers.command || modifiers.alt || modifiers.ctrl => {}
                    egui::Key::Space => c.tap(),
                    egui::Key::Enter => c.resync(),
                    egui::Key::ArrowLeft => c.rt.page = (c.rt.page + PAGES - 1) % PAGES,
                    egui::Key::ArrowRight => c.rt.page = (c.rt.page + 1) % PAGES,
                    k => {
                        if let Some(code) = crate::keys::from_key(k) {
                            c.handle_key(if modifiers.shift { code + crate::keys::SHIFT } else { code }, true);
                        }
                    }
                }
            }
            // キーを押したまま他のウィンドウへ移ると離したことが届かないので、押しているキーを全部離す
            if !focused {
                for code in c.rt.keys_down.clone() {
                    c.handle_key(code, false);
                }
            }
        }
        if save {
            self.save();
            self.notice = "保存しました".into();
        }
    }

    /// デスクトップ環境の無い画面でのファイルメニュー(保存の後ろ)。OSのファイル選択は出せないので、
    /// バックアップは一覧から選ぶ。閉じるボタンの代わりに「終了」を置く
    fn file_menu_no_desktop(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.menu_button("バックアップから戻す", |ui| {
            let stem = self.project_path.file_stem().and_then(|s| s.to_str()).unwrap_or("project").to_string();
            let backups = list_backups(&backup_dir(), &stem);
            if backups.is_empty() {
                ui.label("バックアップはまだありません");
            }
            egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                for p in backups {
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
                    if ui.button(name).clicked() {
                        ui.close();
                        self.load_from(p);
                    }
                }
            });
        })
        .response
        .on_hover_text("起動のたび・別のファイルを開く前に、自動でバックアップを残しています(最新20個)。選んだものに戻します(戻す前の状態もバックアップされます)");
        ui.separator();
        if ui.button("終了").on_hover_text("Ctrl+Q").clicked() {
            ui.close();
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn ui_body(&mut self, ui: &mut egui::Ui) {
        // DMXの出力は別スレッドなので、画面の更新頻度は出力に影響しない。
        // 前面のときは、動いているもの(フェード・モニター・色の追従・パッドの軌跡の点)を約30fpsでなめらかに描く。
        // 前面でないとき(映像ソフトを前に出しているとき)は、グラフィックPCと同居するためCPUを節約して4fpsに減らす。
        // 何も動いていなければ、操作時とMIDI入力時、それと拍の表示が変わる次の拍の頭だけ描く
        let (busy, to_beat, animated) = {
            let c = lock(&self.core);
            let p = &c.project;
            // 色や値がひとりでに変わっているか(マップの「今の色」を追従させるため)
            let animated = p.effects.iter().any(|e| e.enabled) || p.pixel_fx.iter().any(|e| e.enabled) || p.chases.iter().any(|e| e.enabled) || p.palette.auto;
            (self.tab == Tab::Monitor || !c.rt.fades.is_empty() || c.fading(), (1.0 - c.beat_now().rem_euclid(1.0)) * 60.0 / p.bpm.max(1.0) as f64, animated)
        };
        let focused = in_front(ui);
        // 拍の表示は、前面でなくても次の拍の頭で更新する(拍を飛ばさない)
        let mut idle = Duration::from_secs_f64(to_beat.max(0.0) + 0.01).min(Duration::from_millis(if focused { 500 } else { 1000 }));
        // MIDI画面は、エフェクトなどで色が動いている間は「今の色」とパッドのプレビューを追従する
        // (プレビューはメッシュでまとめて描くので、1回の描画は他のタブと大きく変わらない)
        if focused && animated && self.tab == Tab::Map {
            idle = idle.min(SMOOTH);
        }
        // (DMXの出力は画面と別スレッドで40Hz固定なので、画面の更新頻度は出力に影響しない)
        ui.ctx().request_repaint_after(if busy { if focused { SMOOTH } else { Duration::from_millis(250) } } else { idle });
        // 押しているキーを離すのは、本当にフォーカスを失ったとき(KMSDRMで別の端末へ切り替えたときなど。in_front はKMSDRMではいつも前面)
        self.shortcuts(ui, ui.input(|i| i.focused));
        self.autosave();
        // NDIの受信は、設定(パッド・画面)でONにしている間だけ動かす
        let ndi_on = lock(&self.core).project.ndi.enabled;
        if self.launch.inputs && ndi_on != self.ndi.is_some() {
            self.ndi = ndi_on.then(|| crate::ndi::NdiService::start(self.core.clone(), Some(self.ctx.clone())));
        }

        egui::Panel::top("top").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default().show(ui, |ui| match self.tab {
            Tab::Control => self.tab_control(ui),
            Tab::Map => self.tab_map(ui),
            Tab::Patch => self.tab_patch(ui),
            Tab::Profiles => self.tab_profiles(ui),
            Tab::Pairs => self.tab_pairs(ui),
            Tab::Laser => self.tab_laser(ui),
            Tab::Keys => self.tab_keys(ui),
            Tab::Auto => self.tab_auto(ui),
            Tab::Scenes => self.tab_scenes(ui),
            Tab::Effects => self.tab_effects(ui),
            Tab::Sequence => self.tab_sequence(ui),
            Tab::Midi => self.tab_midi(ui),
            Tab::Output => self.tab_output(ui),
            Tab::Monitor => self.tab_monitor(ui),
        });
    }
}

impl App {
    /// 1フレームぶんの画面。cpu_usage は前のフレームのCPU時間(秒。分からなければNone)
    pub fn frame(&mut self, ui: &mut egui::Ui, cpu_usage: Option<f32>) {
        self.control_tick();
        if self.shutdown_requested {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if let (Some(p), Some(cpu)) = (self.perf.as_mut(), cpu_usage) {
            p.frame_cpu += cpu;
        }
        self.ui_timed(ui);
        if let Some(s) = self.shot.as_mut() {
            s.tick(ui.ctx());
        }
    }

    /// 画面を出さずに動かす(`dmxctl --headless`)。モニターの無いPCで、Beat Link Triggerと一緒に置いてオートで動かす用。
    /// OSC受信・テンポの元(Ableton Link)・オートを必ずONにする。終了は SIGTERM / Ctrl+C(安全な値を送ってから終わる)
    pub fn run_headless() -> ! {
        Self::run_headless_with_options(crate::control::Launch::default())
    }

    pub fn run_headless_with_options(options: crate::control::Launch) -> ! {
        let ctx = egui::Context::default();
        let mut app = App::with_options(&ctx, true, options).unwrap_or_else(|error| {
            eprintln!("{error}");
            std::process::exit(1);
        });
        let port = {
            let mut c = lock(&app.core);
            c.project.osc_enabled = true;
            c.project.web_enabled = true;
            c.project.tempo_source = TempoSource::Link;
            c.project.auto.enabled = true;
            c.dirty = true;
            c.project.osc_port
        };
        app.apply_services();
        if !app.notice.is_empty() {
            eprintln!("{}", app.notice);
        }
        println!("dmxctl: 画面なしで起動しました(設定: {} / OSC受信ポート {port} / テンポ: Ableton Link / オート: ON)", app.project_path.display());
        if app.web.is_some() {
            println!("操作パネル: {}  (同じネットワークのスマホ・PCのブラウザで開く)", app.web_status);
        }
        let mut last = String::new();
        let mut beat_report = Instant::now();
        let mut blt_report: Option<Instant> = None;
        loop {
            std::thread::sleep(Duration::from_millis(250));
            app.control_tick();
            if app.shutdown_requested { app.shutdown(); std::process::exit(0); }
            app.autosave();
            let (ndi_on, state, bpm, peers, osc, blt) = {
                let c = lock(&app.core);
                let state = if c.manual_active() {
                    "手動優先(MIDI・キーボードで操作中)".to_string()
                } else if c.auto_online() {
                    format!("曲の情報を受信中(フレーズ: {} / 見た目: {})", if c.rt.phrase.is_empty() { "-" } else { &c.rt.phrase }, if c.rt.auto_key.is_empty() { "-" } else { &c.rt.auto_key })
                } else {
                    "待機(薄い水色)".to_string()
                };
                (c.project.ndi.enabled, state, c.project.bpm, c.rt.link_peers, c.rt.osc_status.clone(), c.blt_state())
            };
            // BLTが見つからないときは、見つからなくなったときと、その後10分ごとに知らせる
            let missing = matches!(blt, crate::engine::BltState::Missing(_));
            if missing && blt_report.is_none_or(|t: Instant| t.elapsed() > Duration::from_secs(600)) {
                eprintln!("{}", blt.message());
                blt_report = Some(Instant::now());
            } else if !missing && blt_report.is_some() {
                println!("Beat Link Trigger が見つかりました");
                blt_report = None;
            }
            if app.launch.inputs && ndi_on != app.ndi.is_some() {
                app.ndi = ndi_on.then(|| crate::ndi::NdiService::start(app.core.clone(), None));
            }
            // 状態が変わったときと、1分ごとに知らせる(journalctl で見られる)
            if state != last || beat_report.elapsed() > Duration::from_secs(60) {
                println!("{state} / BPM {bpm:.1} / Link {peers}台 / OSC {osc}");
                last = state;
                beat_report = Instant::now();
            }
        }
    }

    /// 終了時は先にブラックアウトを数フレーム送ってから止める(レーザーを出しっぱなしにしない)
    pub fn shutdown(&mut self) {
        if self.shutdown_done { return; }
        self.shutdown_done = true;
        {
            let mut c = lock(&self.core);
            c.rt.blackout = true;
            if self.launch.inputs { c.project.midi_input = self.midi.connected.clone().unwrap_or_default(); }
        }
        std::thread::sleep(Duration::from_millis(200));
        self.osc = None;
        self.web = None;
        self.ndi = None;
        lock(&self.core).rt.link = None;
        self.midi.disconnect_clock();
        self.midi.disconnect();
        self.engine.stop();
        if let Some(mut host) = self.control.take() {
            // A recording may not be active; keep the normal shutdown path quiet.
            let _ = host.finish_recording(self);
        }
        if self.launch.autosave { self.save(); }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.frame(ui, frame.info().cpu_usage);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shutdown();
    }
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
            if let Some(m) = lib.mode(&f.profile, f.mode)
                && let Some(ch) = m.find_attr(attr) {
                return m.channels[ch].ranges.clone();
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
