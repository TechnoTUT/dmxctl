//! ホスト: 画面によらずに動く部分(Core・DMXの出力ループ・MIDI/OSC/Link/NDI/音声/操作パネル・
//! 自動操作の窓口・保存)。画面(`app::App`)はこれを1つ持ち、画面なし(`dmxctl headless` / `serve`)は
//! これだけで動く。

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
    engine: Engine,
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
        let engine = Engine::spawn_with_output(core.clone(), options.output);

        let mut host = Host {
            launch: options.clone(),
            control,
            shutdown_requested: false,
            shutdown_done: false,
            core,
            engine,
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
        match self.reload_checked(Some(&path), false, false) {
            Ok(_) => self.notice = format!("読み込みました: {}", path.display()),
            Err(error) => self.notice = format!("読み込みに失敗しました: {error}"),
        }
    }

    pub(crate) fn autosave(&mut self) {
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

    /// 自動操作の窓口(control)に届いた要求を処理する
    pub(crate) fn control_tick(&mut self) {
        if let Some(mut server) = self.control.take() {
            self.shutdown_requested |= server.poll(self);
            self.control = Some(server);
        }
    }

    /// Runs the same Host and Engine as the GUI, without forcing BLT/auto settings.
    pub fn serve(options: crate::control::Launch) -> anyhow::Result<()> {
        let ctx = egui::Context::default();
        let mut host = Self::start(&ctx, options)?;
        let address = host.control.as_ref().map(|server|server.address);
        println!("{}",serde_json::json!({"ready":true,"listen":address,"project":host.project_path,
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
        if let Some(mut server) = self.control.take() {
            // A recording may not be active; keep the normal shutdown path quiet.
            let _ = server.finish_recording(self);
        }
        if self.launch.autosave { self.save(); }
    }
}
