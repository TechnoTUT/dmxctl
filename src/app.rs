//! 画面(ウィンドウ枠・タブ)。各タブの中身は ui_*.rs。
//! Core・出力・外部との連携・保存はホスト(`host::Host`)が持ち、画面はそれを1つ持つ。

use crate::engine::lock;
use crate::fixture::{Library, Range};
use crate::host::Host;
use crate::midi;
use crate::project::*;
use eframe::egui::{self, Align, Color32, Layout, RichText};
use std::collections::HashSet;
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
    pub host: Host,
    pub tab: Tab,
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
    /// MIDIクロックの入力の候補(クロックはホストのPCで受けるので、クライアントではサーバーのPCの一覧)
    pub clock_ports: Vec<String>,
    pub serial_ports: Vec<(String, String)>,
    pub d2xx: Result<Vec<String>, String>,
    pub new_group: String,
    /// パッチタブで名前を書き換え中のグループの番号と、入力中の名前。確定するまでプロジェクトには反映しない
    pub group_rename: Option<(usize, String)>,
    pub map_sel: Option<crate::apcmini::Ctl>,
    /// レーザータブで、動いてよい範囲を編集中のレーザー
    pub laser_edit: Option<u32>,
    /// コントロール画面で隠しているカテゴリ
    pub cat_hidden: HashSet<String>,
    /// シーン保存の対象から外すカテゴリ
    pub scene_skip_cats: HashSet<String>,
    pub pe: crate::ui_profile::ProfileEd,
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
    /// ホストの読み直し・サービス反映の回数(変わったら、画面の編集中の値を捨てる)
    seen_reloads: u64,
    seen_services: u64,
    seen_midi: u64,
    seen_devices: u64,
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
        let host = Host::start(egui_ctx, options.clone())?;
        let osc_port_edit = lock(&host.core).project.osc_port;
        let mut app = App {
            midi_choice: host.initial_midi.clone(),
            osc_port_edit,
            seen_reloads: host.reloads,
            seen_services: host.services_applied,
            seen_midi: host.midi_synced,
            seen_devices: 0,
            clock_ports: Vec::new(),
            host,
            tab: Tab::Map,
            add_profile: 0,
            scene_skip: HashSet::new(),
            scene_name: String::new(),
            scene_fade: 1.0,
            monitor_universe: 0,
            monitor_all: false,
            midi_ports: if options.inputs { midi::list_ports() } else { Vec::new() },
            serial_ports: if options.output { crate::opendmx::list_serial_ports() } else { Vec::new() },
            d2xx: Ok(Vec::new()),
            new_group: String::new(),
            group_rename: None,
            map_sel: None,
            laser_edit: None,
            cat_hidden: HashSet::new(),
            scene_skip_cats: HashSet::new(),
            pe: Default::default(),
            audio_ports: if options.inputs { crate::audio::input_devices() } else { Vec::new() },
            pos_name: String::new(),
            pos_fade: 0.5,
            color_edit: crate::color::ColorTarget::Scope,
            key_mouse: None,
            // 開発・確認用: DMXCTL_KEY_COLOR=1 で、キーボードタブを色の段で開く
            key_color_view: std::env::var_os("DMXCTL_KEY_COLOR").is_some(),
            perf: std::env::var_os("DMXCTL_PERF").map(|_| Perf { frames: 0, ui_time: Duration::ZERO, frame_cpu: 0.0, since: Instant::now(), text: String::new() }),
            shot: crate::shot::Shot::from_env(),
        };
        // 確認用: DMXCTL_TAB=タブ名 で開くタブ、DMXCTL_UNIVERSE=番号 でモニターのユニバースを選ぶ
        if let Ok(name) = std::env::var("DMXCTL_TAB")
            && let Some((t, _)) = TABS.iter().find(|(_, label)| *label == name) {
            app.tab = *t;
        }
        app.clock_ports = app.midi_ports.clone();
        if let Some(u) = std::env::var("DMXCTL_UNIVERSE").ok().and_then(|v| v.parse::<usize>().ok()) {
            app.monitor_universe = u.clamp(1, NUM_UNIVERSES) - 1;
        }
        Ok(app)
    }

    /// ホストで起きたこと(自動操作からの読み直し・設定の反映)に、画面の編集中の値を合わせる
    fn follow_host(&mut self) {
        if self.seen_reloads != self.host.reloads {
            self.seen_reloads = self.host.reloads;
            self.map_sel = None;
            self.pe = Default::default();
        }
        if self.seen_midi != self.host.midi_synced {
            self.seen_midi = self.host.midi_synced;
            self.midi_choice = lock(&self.host.core).project.midi_input.clone();
        }
        // クライアント: 出力・音声・MIDIクロックの機器は、サーバーのPCのものから選ぶ
        if let Some((count, d)) = self.host.server_devices()
            && count != self.seen_devices {
            self.seen_devices = count;
            self.serial_ports = d.serial;
            self.d2xx = d.d2xx;
            self.audio_ports = d.audio;
            self.clock_ports = d.midi;
        }
        if self.seen_services != self.host.services_applied {
            self.seen_services = self.host.services_applied;
            self.osc_port_edit = lock(&self.host.core).project.osc_port;
        }
    }

    /// 設定を反映する(ホストへ)
    pub fn apply_services(&mut self) {
        self.host.apply_services();
        self.follow_host();
    }

    pub fn save(&mut self) {
        self.host.save();
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
                let menu = ui.menu_button("ファイル ▼", |ui| {
                    if ui.button("保存").on_hover_text(if cfg!(target_os = "macos") { "⌘S" } else { "Ctrl+S" }).clicked() {
                        self.save();
                        self.host.notice = if self.host.is_client() { "保存をサーバーに頼みました".into() } else { "保存しました".into() };
                        ui.close();
                    }
                    if self.host.is_client() {
                        self.file_menu_client(ui);
                        return;
                    }
                    if no_desktop(ui.ctx()) {
                        self.file_menu_no_desktop(ui);
                        return;
                    }
                    if ui.button("別名で保存…").clicked() {
                        ui.close();
                        if let Some(p) = rfd::FileDialog::new().add_filter("プロジェクト", &["json"]).set_file_name("show.json").save_file() {
                            self.host.project_path = p;
                            self.save();
                        }
                    }
                    if ui.button("開く…").clicked() {
                        ui.close();
                        if let Some(p) = rfd::FileDialog::new().add_filter("プロジェクト", &["json"]).pick_file() {
                            self.host.load_from(p);
                        }
                    }
                    ui.separator();
                    if ui.button("バックアップから戻す…").on_hover_text("起動のたび・別のファイルを開く前に、自動でバックアップを残しています(最新20個)。選んだものに戻します(戻す前の状態もバックアップされます)").clicked() {
                        ui.close();
                        if let Some(p) = rfd::FileDialog::new().add_filter("プロジェクト", &["json"]).set_directory(backup_dir()).pick_file() {
                            self.host.load_from(p);
                        }
                    }
                });
                // クライアント: メニューを開いたら、サーバーのファイルの一覧を取り直す
                if menu.response.clicked() {
                    self.host.request_server_files();
                }
                if lock(&self.host.core).dirty {
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
        let (midi, last, outs, blt, remote_emergency) = {
            let c = lock(&self.host.core);
            (c.rt.midi_status.clone(), c.rt.last_midi.clone(), c.rt.output_status.clone(), c.project.auto.enabled.then(|| c.blt_state()), c.rt.remote_emergency)
        };
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            if crate::engine::EMERGENCY.load(std::sync::atomic::Ordering::SeqCst) || remote_emergency {
                theme::dot(ui, theme::DANGER);
                ui.label(RichText::new("安全停止中(アプリを再起動してください)").small().color(theme::DANGER));
                ui.separator();
            }
            let connected = self.host.midi.connected.is_some();
            theme::dot(ui, if connected { theme::OK } else { theme::TEXT_FAINT });
            ui.label(RichText::new(format!("MIDI {midi}")).small().color(theme::TEXT_DIM));
            if !last.is_empty() {
                ui.label(RichText::new(format!("最後の入力: {last}")).small().color(theme::TEXT_FAINT));
            }
            ui.separator();
            // サーバーにつないでいるとき: 接続の状態(出力の状態はサーバーのもの)
            if let Some((connected, status)) = self.host.client_status() {
                theme::dot(ui, if connected { theme::OK } else { theme::WARN });
                ui.label(RichText::new(format!("サーバー: {status}")).small().color(if connected { theme::TEXT_DIM } else { theme::WARN }));
                ui.separator();
            }
            // 出力はまとめて1行に(詳しくはマウスを乗せると出る)
            let on: Vec<(usize, &String)> = outs.iter().enumerate().filter(|(_, s)| s.as_str() != "無効").collect();
            let bad = on.iter().filter(|(_, s)| output_has_error(s)).count();
            let color = if outs.is_empty() || bad > 0 { theme::WARN } else if on.is_empty() { theme::TEXT_FAINT } else { theme::OK };
            theme::dot(ui, color);
            let text = if !self.host.output_enabled() {
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
            if !self.host.notice.is_empty() {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.small_button("×").on_hover_text("消す").clicked() {
                        self.host.notice.clear();
                    }
                    ui.add(egui::Label::new(RichText::new(&self.host.notice).small().color(theme::WARN)).truncate()).on_hover_text(&self.host.notice);
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
            let mut c = lock(&self.host.core);
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
                                c.live(crate::live::LiveOp::Key { number: k, pressed: false });
                            }
                        }
                    }
                    continue;
                }
                if repeat {
                    continue;
                }
                match key {
                    egui::Key::Escape => c.live(crate::live::LiveOp::ToggleBlackout),
                    egui::Key::S if modifiers.command => save = true,
                    // デスクトップ環境の無い画面には閉じるボタンが無いので、Ctrl+Qで終了
                    egui::Key::Q if modifiers.command && no_desktop(ui.ctx()) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
                    _ if typing || modifiers.command || modifiers.alt || modifiers.ctrl => {}
                    egui::Key::Space => c.live(crate::live::LiveOp::Tap),
                    egui::Key::Enter => c.live(crate::live::LiveOp::Resync),
                    egui::Key::ArrowLeft => c.live(crate::live::LiveOp::StepPage { delta: -1 }),
                    egui::Key::ArrowRight => c.live(crate::live::LiveOp::StepPage { delta: 1 }),
                    k => {
                        if let Some(code) = crate::keys::from_key(k) {
                            c.live(crate::live::LiveOp::Key { number: if modifiers.shift { code + crate::keys::SHIFT } else { code }, pressed: true });
                        }
                    }
                }
            }
            // キーを押したまま他のウィンドウへ移ると離したことが届かないので、押しているキーを全部離す
            if !focused {
                for code in c.rt.keys_down.clone() {
                    c.live(crate::live::LiveOp::Key { number: code, pressed: false });
                }
            }
        }
        if save {
            self.save();
            self.host.notice = if self.host.is_client() { "保存をサーバーに頼みました".into() } else { "保存しました".into() };
        }
    }

    /// サーバーにつないでいるときのファイルメニュー(保存の後ろ)。ファイルはサーバーが読み込む
    /// (ローカルの「開く」と同じく、中身を読み込むだけで保存先は変えない。読み込む前の状態はサーバーのバックアップに残る)
    fn file_menu_client(&mut self, ui: &mut egui::Ui) {
        let files = self.host.server_files();
        let mut open: Option<String> = None;
        ui.separator();
        let list = |ui: &mut egui::Ui, items: &[String], current: &str, empty: &str, open: &mut Option<String>| {
            if items.is_empty() {
                ui.label(empty);
            }
            egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                for path in items {
                    let name = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path);
                    let label = if path == current { format!("{name}(今の保存先)") } else { name.to_string() };
                    if ui.button(label).on_hover_text(path).clicked() {
                        *open = Some(path.clone());
                        ui.close();
                    }
                }
            });
        };
        let current = files.as_ref().map(|f| f.current.clone()).unwrap_or_default();
        ui.menu_button("サーバーのファイルを開く", |ui| match &files {
            Some(f) => list(ui, &f.projects, &current, "プロジェクトのフォルダに .json がありません", &mut open),
            None => { ui.label("一覧を取得中…"); }
        })
        .response
        .on_hover_text("サーバーのプロジェクトのフォルダにあるファイルを、サーバーで読み込みます(保存先は変わりません)");
        ui.menu_button("サーバーのバックアップから戻す", |ui| match &files {
            Some(f) => list(ui, &f.backups, &current, "バックアップはまだありません", &mut open),
            None => { ui.label("一覧を取得中…"); }
        })
        .response
        .on_hover_text("サーバーが起動のたび・別のファイルを開く前に残しているバックアップに戻します(戻す前の状態もバックアップされます)");
        if !no_desktop(ui.ctx())
            && ui.button("このPCのファイルを開く…").on_hover_text("このPCのファイルの中身をサーバーへ送って、サーバーで読み込みます(保存先は変わりません)").clicked() {
            ui.close();
            if let Some(p) = rfd::FileDialog::new().add_filter("プロジェクト", &["json"]).pick_file() {
                self.host.open_local_file(&p);
            }
        }
        if ui.button("一覧を更新").clicked() {
            self.host.request_server_files();
        }
        if let Some(path) = open {
            self.host.open_server_file(&path);
        }
        if no_desktop(ui.ctx()) {
            ui.separator();
            if ui.button("終了").on_hover_text("Ctrl+Q").clicked() {
                ui.close();
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    /// デスクトップ環境の無い画面でのファイルメニュー(保存の後ろ)。OSのファイル選択は出せないので、
    /// バックアップは一覧から選ぶ。閉じるボタンの代わりに「終了」を置く
    fn file_menu_no_desktop(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        ui.menu_button("バックアップから戻す", |ui| {
            let stem = self.host.project_path.file_stem().and_then(|s| s.to_str()).unwrap_or("project").to_string();
            let backups = list_backups(&backup_dir(), &stem);
            if backups.is_empty() {
                ui.label("バックアップはまだありません");
            }
            egui::ScrollArea::vertical().max_height(400.0).show(ui, |ui| {
                for p in backups {
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
                    if ui.button(name).clicked() {
                        ui.close();
                        self.host.load_from(p);
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
            let c = lock(&self.host.core);
            let p = &c.project;
            // 色や値がひとりでに変わっているか(マップの「今の色」を追従させるため)
            let animated = p.effects.iter().any(|e| e.enabled) || p.pixel_fx.iter().any(|e| e.enabled) || p.chases.iter().any(|e| e.enabled) || p.palette.auto;
            (self.tab == Tab::Monitor || !c.rt.fades.is_empty() || c.rt.remote_fades > 0 || c.fading(), (1.0 - c.beat_now().rem_euclid(1.0)) * 60.0 / p.bpm.max(1.0) as f64, animated)
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
        self.host.autosave();
        self.host.sync_ndi(true);

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
        self.host.control_tick();
        self.follow_host();
        if self.host.shutdown_requested {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if let (Some(p), Some(cpu)) = (self.perf.as_mut(), cpu_usage) {
            p.frame_cpu += cpu;
        }
        self.host.before_frame();
        let before=lock(&self.host.core).history_snapshot();
        self.ui_timed(ui);
        self.host.after_frame();
        if before.is_some() {
            let after=lock(&self.host.core).history_snapshot();
            crate::recording::HistorySnapshot::observe(before,after,"gui_frame");
        }
        if let Some(s) = self.shot.as_mut() {
            s.tick(ui.ctx());
        }
    }

    /// 終了(ホストが安全な値を送ってから止める)
    pub fn shutdown(&mut self) {
        self.host.shutdown();
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        self.frame(ui, frame.info().cpu_usage);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.host.shutdown();
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
