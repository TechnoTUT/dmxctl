//! ホスト: 画面によらずに動く部分(Core・DMXの出力ループ・MIDI/OSC/Link/NDI/音声/操作パネル・
//! 自動操作の窓口・保存)。画面(`app::App`)はこれを1つ持ち、画面なし(`dmxctl headless` / `serve`)は
//! これだけで動く。
//!
//! サーバーにつなぐクライアント(`dmxctl connect HOST`)のホストは、Core がサーバーの写しになり、
//! 出力ループ・外部との連携・自動操作の窓口を動かさない(MIDIコントローラーだけは手元のものを使う)。
//! 保存はサーバーに頼む。

use crate::engine::{lock, Core, Engine, Shared};
use crate::fixture::Library;
use crate::midi::{self, Midi};
use crate::project::*;
use eframe::egui;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct Host {
    pub(crate) launch: crate::control::Launch,
    control: Option<crate::control::Server>,
    pub(crate) shutdown_requested: bool,
    shutdown_done: bool,
    pub core: Shared,
    /// 出力ループ(クライアントでは動かさない)
    engine: Option<Engine>,
    /// LANのクライアントの待ち受け(`--remote-listen`)
    pub remote: Option<crate::remote::RemoteServer>,
    /// サーバーへの接続(クライアントのとき)
    client: Option<crate::remote::Client>,
    pub midi: Midi,
    pub project_path: PathBuf,
    last_save: Instant,
    /// 画面の下に出す知らせ(画面なしでは標準エラーに出す)
    pub notice: String,
    pub osc: Option<crate::osc::OscServer>,
    /// ブラウザの操作パネル
    pub web: Option<crate::web::WebServer>,
    /// 操作パネルの状態(画面の表示用)
    pub web_status: String,
    /// NDIの受信(設定で有効にしている間だけ動かす)
    ndi: Option<crate::ndi::NdiService>,
    audio: Option<crate::audio::AudioService>,
    /// 描き直しを頼む先(画面なしでは使われないContext)
    pub ctx: egui::Context,
    /// プロジェクトを読み直した回数(画面は、変わったら編集中の選択などを捨てる)
    pub(crate) reloads: u64,
    /// サービスの設定を反映した回数(画面は、変わったらOSCポートの編集欄などを今の値に戻す)
    pub(crate) services_applied: u64,
    /// MIDI機器をプロジェクトの設定に合わせた回数(画面は、変わったら機器の選択欄を合わせる)
    pub(crate) midi_synced: u64,
    /// 起動時に自動接続しようとしたMIDI機器(画面の機器の選択欄の初期値。自動接続しないときは空)
    pub(crate) initial_midi: String,
}

impl Host {
    pub fn start(ctx: &egui::Context, options: crate::control::Launch) -> anyhow::Result<Self> {
        if let Some(target) = options.connect.clone() {
            return Self::connect(ctx, options, &target);
        }
        let path = options.project.clone().unwrap_or_else(default_project_path);
        let path = if path.is_absolute() {path}else{std::env::current_dir()?.join(path)};
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
        let mut initial_core=Core::new(project,lib);
        initial_core.rt.project_path=Some(path.clone());
        let core: Shared = Arc::new(Mutex::new(initial_core));
        // 初回のoutput_gen(0)でエンジンが出力を組み立てる
        let control = options.listen.map(|address|crate::control::Server::start(address,ctx.clone())).transpose()?;
        let engine = Some(Engine::spawn_with_output(core.clone(), options.output));
        let remote = options.remote_listen.map(|address| crate::remote::RemoteServer::start(core.clone(), ctx.clone(), address, options.remote_allow_safety_edit)).transpose()?;

        let mut host = Host {
            launch: options.clone(),
            control,
            shutdown_requested: false,
            shutdown_done: false,
            core,
            engine,
            remote,
            client: None,
            midi: Midi::default(),
            project_path: path,
            last_save: Instant::now(),
            notice,
            osc: None,
            web: None,
            web_status: String::new(),
            ndi: None,
            audio: None,
            ctx: ctx.clone(),
            reloads: 0,
            services_applied: 0,
            midi_synced: 0,
            initial_midi: String::new(),
        };
        if let Some(p) = std::env::var("DMXCTL_PAGE").ok().and_then(|v| v.parse::<u8>().ok()) {
            lock(&host.core).rt.page = p.clamp(1, PAGES) - 1;
        }
        host.auto_connect_midi();
        host.apply_services();
        Ok(host)
    }

    /// サーバーにつなぐクライアントのホスト。Core はサーバーの写しで、出力ループ・外部との連携・保存は動かさない
    fn connect(ctx: &egui::Context, mut options: crate::control::Launch, target: &str) -> anyhow::Result<Self> {
        let (mut client, initial) = crate::remote::Client::connect(target, ctx.clone())?;
        let core: Shared = Arc::new(Mutex::new(Core::new(initial.project, Arc::new(initial.lib))));
        client.attach(core.clone());
        options.output = false;
        options.autosave = false;
        options.listen = None;
        options.remote_listen = None;
        options.remote_allow_safety_edit = false;
        let mut host = Host {
            launch: options,
            control: None,
            shutdown_requested: false,
            shutdown_done: false,
            core,
            engine: None,
            remote: None,
            client: Some(client),
            midi: Midi::default(),
            project_path: PathBuf::from(format!("{target} (サーバー)")),
            last_save: Instant::now(),
            notice: String::new(),
            osc: None,
            web: None,
            web_status: String::new(),
            ndi: None,
            audio: None,
            ctx: ctx.clone(),
            reloads: 0,
            services_applied: 0,
            midi_synced: 0,
            initial_midi: String::new(),
        };
        host.auto_connect_midi();
        // 終了のシグナル(Ctrl+C など)は、ふだんは出力ループが安全な値を送ってから終わらせる。
        // クライアントには出力ループが無い(照明も出していない)ので、画面を閉じてすぐ終わる
        if crate::engine::SAFETY_HOOKS.load(std::sync::atomic::Ordering::SeqCst) {
            let ctx = ctx.clone();
            std::thread::Builder::new().name("client-exit".into()).spawn(move || {
                while !crate::engine::EMERGENCY.load(std::sync::atomic::Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(50));
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                ctx.request_repaint();
                std::thread::sleep(Duration::from_secs(1));
                std::process::exit(0);
            })?;
        }
        Ok(host)
    }

    /// サーバーのPCの機器の一覧を頼み直す(クライアントのとき)
    pub fn request_devices(&self) {
        if let Some(c) = &self.client {
            c.request_devices();
        }
    }

    /// サーバーのPCの機器の一覧と、受け取った回数(クライアントのとき)
    pub fn server_devices(&self) -> Option<(u64, crate::remote::Devices)> {
        self.client.as_ref().and_then(|c| c.devices())
    }

    /// サーバーのPCのプロジェクトのファイルの一覧を頼み直す / 届いた一覧(クライアントのとき)
    pub fn request_server_files(&self) {
        if let Some(c) = &self.client {
            c.request_files();
        }
    }

    pub fn server_files(&self) -> Option<crate::remote::Files> {
        self.client.as_ref().and_then(|c| c.files())
    }

    /// サーバーのPCのファイルを、サーバーで読み込んでもらう(クライアントのとき)
    pub fn open_server_file(&mut self, path: &str) {
        if let Some(c) = &self.client {
            c.open_path(path);
            self.notice = format!("読み込みをサーバーに頼みました: {path}");
        }
    }

    /// このPCのファイルの中身をサーバーへ送って、読み込んでもらう(クライアントのとき)
    pub fn open_local_file(&mut self, path: &std::path::Path) {
        let Some(c) = &self.client else { return };
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("project.json").to_string();
        match std::fs::read_to_string(path) {
            Ok(text) => {
                c.open_text(&name, text.trim_start_matches('\u{feff}'));
                self.notice = format!("読み込みをサーバーに頼みました: {name}");
            }
            Err(e) => self.notice = format!("ファイルを読めません: {e}"),
        }
    }

    /// 機材定義の保存・削除をサーバーに頼む(クライアントのとき。頼んだらtrue)
    pub fn remote_save_profile(&self, profile: &crate::fixture::Profile) -> bool {
        self.client.as_ref().map(|c| c.save_profile(profile)).is_some()
    }

    pub fn remote_delete_profile(&self, id: &str) -> bool {
        self.client.as_ref().map(|c| c.delete_profile(id)).is_some()
    }

    /// 実機に出力しているか(クライアントでは、サーバーが出しているか)
    pub fn output_enabled(&self) -> bool {
        if self.client.is_some() { lock(&self.core).rt.physical_output_enabled } else { self.launch.output }
    }

    /// サーバーにつないだクライアントか
    pub fn is_client(&self) -> bool {
        self.client.is_some()
    }

    /// クライアントの接続の状態(クライアントでなければNone)
    pub fn client_status(&self) -> Option<(bool, String)> {
        self.client.as_ref().map(|c| (c.connected(), c.status()))
    }

    /// 画面の1フレームの前後(クライアントでは、サーバーから届いた編集を入れ、手元の編集を送る)
    pub(crate) fn before_frame(&mut self) {
        if let Some(c) = self.client.as_mut() {
            c.before_frame(&self.core);
        }
    }

    pub(crate) fn after_frame(&mut self) {
        if let Some(c) = self.client.as_mut() {
            c.after_frame(&self.core);
            let errors = c.take_errors();
            if !errors.is_empty() {
                self.notice = format!("サーバー: {}", errors.join(" / "));
            }
        }
    }

    /// 前回のMIDI機器、なければAPC mini が見つかれば自動接続。
    /// 環境変数 DMXCTL_NO_MIDI=1 なら自動接続しない(本番のアプリが使っているコントローラーを、2つ目のアプリが奪わないように)
    fn auto_connect_midi(&mut self) {
        if !self.launch.inputs || std::env::var_os("DMXCTL_NO_MIDI").is_some() {
            return;
        }
        let ports = midi::list_ports();
        let mut name = lock(&self.core).project.midi_input.clone();
        if (name.is_empty() || !ports.contains(&name))
            && let Some(p) = crate::apcmini::find_port(&ports) {
            name = p;
        }
        self.initial_midi = name.clone();
        if !name.is_empty() && ports.contains(&name) {
            match self.midi.connect(&self.core, self.ctx.clone(), &name) {
                Ok(()) => lock(&self.core).project.midi_input = name,
                Err(e) => self.notice = e,
            }
        }
    }

    /// 設定に合わせて、Ableton Link・OSC受信・MIDIクロック用の入力を開始/停止する
    pub fn apply_services(&mut self) {
        // クライアントでは、OSC・Link・NDI・音声・操作パネルはサーバーが動かす
        if self.client.is_some() {
            self.services_applied += 1;
            return;
        }
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
        self.services_applied += 1;
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

    /// NDIの受信は、設定(パッド・画面)でONにしている間だけ動かす
    pub(crate) fn sync_ndi(&mut self, repaint: bool) {
        if self.client.is_some() {
            return;
        }
        let ndi_on = lock(&self.core).project.ndi.enabled;
        if self.launch.inputs && ndi_on != self.ndi.is_some() {
            self.ndi = ndi_on.then(|| crate::ndi::NdiService::start(self.core.clone(), repaint.then(|| self.ctx.clone())));
        }
    }

    pub fn save(&mut self) {
        if let Err(error) = self.save_checked() {
            self.notice = format!("保存に失敗しました: {error}");
        }
    }

    /// GUIの保存・自動保存・終了時の保存。厳しい検証(editing::validate)はかけない:
    /// GUIの編集で検証に通らない状態(機材を消したあとの参照など)になっても、作業内容を失わないように。
    /// リモートの "save" は、呼ぶ前に検証する
    pub(crate) fn save_checked(&mut self) -> anyhow::Result<()> {
        if let Some(c) = &self.client {
            c.save();
            return Ok(());
        }
        let project = {
            let mut c = lock(&self.core);
            c.project.ensure_binding_ids();
            c.dirty = false;
            c.project.clone()
        };
        // 失敗しても次の自動保存まで待つ(毎フレーム保存し直さない)
        self.last_save = Instant::now();
        if let Err(e) = project.save(&self.project_path) {
            lock(&self.core).dirty = true;
            return Err(e);
        }
        Ok(())
    }

    pub(crate) fn load_from(&mut self, path: PathBuf) {
        // クライアント: このPCで選んだファイルの中身を、サーバーで読み込んでもらう
        if self.client.is_some() {
            self.open_local_file(&path);
            return;
        }
        match self.reload_checked(Some(&path), false, false) {
            Ok(_) => self.notice = format!("読み込みました: {}", path.display()),
            Err(error) => self.notice = format!("読み込みに失敗しました: {error}"),
        }
    }

    pub(crate) fn autosave(&mut self) {
        // クライアントは自動保存しない(保存するかはサーバーの起動オプションで決まる。connect で autosave を切っている)
        if !self.launch.autosave || self.client.is_some() { return; }
        if self.last_save.elapsed() > Duration::from_secs(5) {
            let dirty = lock(&self.core).dirty;
            if dirty {
                self.save();
            } else {
                self.last_save = Instant::now();
            }
        }
    }

    /// 自動操作の窓口(control)に届いた要求を処理する
    pub fn control_tick(&mut self) {
        if let Some(mut server) = self.control.take() {
            self.shutdown_requested |= server.poll(self);
            self.control = Some(server);
        }
        // LANのクライアントから頼まれた保存・設定の反映
        let (save, services) = match &self.remote {
            Some(r) => (r.events.save.swap(false, std::sync::atomic::Ordering::SeqCst), r.events.services.swap(false, std::sync::atomic::Ordering::SeqCst)),
            None => (false, false),
        };
        if save {
            self.save();
        }
        if services {
            self.apply_services();
        }
        // クライアントから頼まれた、プロジェクトの読み込み(ローカルの「開く」と同じく、保存先は変えない)
        let requests = self.remote.as_ref().map(|r| std::mem::take(&mut *r.events.open.lock().unwrap_or_else(|e| e.into_inner()))).unwrap_or_default();
        for request in requests {
            let (name, result) = match &request.source {
                crate::remote::OpenSource::Path(path) => (path.display().to_string(), self.reload_checked(Some(path), false, false)),
                crate::remote::OpenSource::Text { name, text } => (name.clone(), self.reload_text(text, name, false, false)),
            };
            let msg = match result {
                Ok(_) => format!("読み込みました: {name}"),
                Err(e) => format!("読み込みに失敗しました({name}): {e}"),
            };
            self.notice = msg.clone();
            request.reply(&msg);
        }
    }

    /// Runs the same Host and Engine as the GUI, without forcing BLT/auto settings.
    pub fn serve(options: crate::control::Launch) -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        let mut host = Self::start(&ctx, options)?;
        let address = host.control.as_ref().map(|server|server.address);
        println!("{}",serde_json::json!({"ready":true,"listen":address,"remote_listen":host.remote.as_ref().map(|r|r.address),"project":host.project_path,
            "output_enabled":host.launch.output,"inputs_enabled":host.launch.inputs,"autosave":host.launch.autosave}));
        while !host.shutdown_requested {
            host.control_tick();
            host.autosave();
            if !host.shutdown_requested { std::thread::sleep(crate::engine::TICK); }
        }
        host.shutdown();
        Ok(())
    }

    /// 画面を出さずに動かす(`dmxctl headless`)。モニターの無いPCで、Beat Link Triggerと一緒に置いてオートで動かす用。
    /// OSC受信・テンポの元(Ableton Link)・オートを必ずONにする。終了は SIGTERM / Ctrl+C(安全な値を送ってから終わる)
    pub fn run_headless(options: crate::control::Launch) -> ! {
        let ctx = egui::Context::default();
        let mut host = Self::start(&ctx, options).unwrap_or_else(|error| {
            eprintln!("{error}");
            std::process::exit(1);
        });
        let port = {
            let mut c = lock(&host.core);
            c.project.osc_enabled = true;
            c.project.web_enabled = true;
            c.project.tempo_source = TempoSource::Link;
            c.project.auto.enabled = true;
            c.dirty = true;
            c.project.osc_port
        };
        host.apply_services();
        if !host.notice.is_empty() {
            eprintln!("{}", host.notice);
        }
        println!("dmxctl: 画面なしで起動しました(設定: {} / OSC受信ポート {port} / テンポ: Ableton Link / オート: ON)", host.project_path.display());
        if let Some(r) = &host.remote {
            println!("クライアントの待ち受け: {}  (別のPCで dmxctl connect このPCのアドレス)", r.address);
        }
        if host.web.is_some() {
            println!("操作パネル: {}  (同じネットワークのスマホ・PCのブラウザで開く)", host.web_status);
        }
        let mut last = String::new();
        let mut beat_report = Instant::now();
        let mut blt_report: Option<Instant> = None;
        loop {
            std::thread::sleep(Duration::from_millis(250));
            host.control_tick();
            if host.shutdown_requested { host.shutdown(); std::process::exit(0); }
            host.autosave();
            let (state, bpm, peers, osc, blt) = {
                let c = lock(&host.core);
                let state = if c.manual_active() {
                    "手動優先(MIDI・キーボードで操作中)".to_string()
                } else if c.auto_online() {
                    format!("曲の情報を受信中(フレーズ: {} / 見た目: {})", if c.rt.phrase.is_empty() { "-" } else { &c.rt.phrase }, if c.rt.auto_key.is_empty() { "-" } else { &c.rt.auto_key })
                } else {
                    "待機(薄い水色)".to_string()
                };
                (state, c.project.bpm, c.rt.link_peers, c.rt.osc_status.clone(), c.blt_state())
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
            host.sync_ndi(false);
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
        // クライアント: 照明はサーバーが出しているので、つなぎを切るだけ(ブラックアウトも保存もしない)
        if self.client.is_some() {
            self.midi.disconnect();
            self.client = None;
            return;
        }
        self.remote = None;
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
        if let Some(engine) = self.engine.as_mut() {
            engine.stop();
        }
        if let Some(mut server) = self.control.take() {
            // A recording may not be active; keep the normal shutdown path quiet.
            let _ = server.finish_recording(self);
        }
        if self.launch.autosave { self.save(); }
    }
}
