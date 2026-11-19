//! クライアント: サーバーにつなぎ、手元の写しをサーバーの状態に合わせ、手元の操作をサーバーへ送る。
use super::*;

pub(super) struct LinkInner {
    tx: Mutex<Option<mpsc::Sender<String>>>,
    seq: AtomicU64,
}

/// クライアントからサーバーへ送る口(`Runtime::remote` に入れる)
#[derive(Clone)]
pub struct Link {
    inner: Arc<LinkInner>,
}

impl Link {
    fn new() -> Self {
        Link { inner: Arc::new(LinkInner { tx: Mutex::new(None), seq: AtomicU64::new(0) }) }
    }

    fn send(&self, mut msg: Value) -> u64 {
        let seq = self.inner.seq.fetch_add(1, Ordering::SeqCst) + 1;
        msg["seq"] = json!(seq);
        if let Some(tx) = guard(&self.inner.tx).as_ref() {
            let _ = tx.send(msg.to_string());
        }
        seq
    }


    fn sent(&self) -> u64 {
        self.inner.seq.load(Ordering::SeqCst)
    }
}

impl LiveForward for Link {
    /// 実行中の状態を変える操作を送る
    fn live(&self, op: &LiveOp) {
        self.send(json!({"t":"live","op":op}));
    }
}

pub(super) struct ClientSync {
    /// サーバーが持っているはずのプロジェクト
    confirmed: Value,
    /// まだ処理済みと言われていない自分の編集(通し番号つき)
    pending: Vec<(u64, Vec<Change>)>,
    /// 手元の写しのプロジェクトを、confirmed + pending で作り直す(画面のフレームの境目で)
    rebuild: bool,
    /// サーバーから最後に届いたラーンの状態と、自分が最後にラーンを送った番号
    learn: (Option<usize>, Option<usize>),
    learn_seq: u64,
    /// 最後に届いた状態の処理済みの番号と、ARMの状態
    ack: u64,
    armed: Value,
    /// サーバーのPCの機器の一覧と、受け取った回数
    devices: Option<Devices>,
    devices_count: u64,
    /// サーバーのPCのプロジェクトのファイルの一覧
    files: Option<Files>,
    connected: bool,
    /// 切れているあいだの編集を知らせたか(つながるたびに戻す)
    offline_noticed: bool,
    status: String,
    errors: Vec<String>,
}

/// 初めの接続で受け取ったもの(手元の写しのCoreを作る)
pub struct Initial {
    pub project: Project,
    pub lib: Library,
}

/// サーバーへの接続(`dmxctl connect`)
pub struct Client {
    pub target: String,
    link: Link,
    sync: Arc<Mutex<ClientSync>>,
    stop: Arc<AtomicBool>,
    pending: Option<(TcpStream, BufReader<TcpStream>, Value)>,
    supervisor: Option<JoinHandle<()>>,
    ctx: dmxctl_core::wake::Wake,
    /// 画面の1フレームの前のプロジェクトとラーンの状態
    before: Option<(Value, (Option<usize>, Option<usize>))>,
}

pub(super) fn resolve(target: &str) -> anyhow::Result<SocketAddr> {
    let has_port = target.parse::<SocketAddr>().is_ok() || target.rsplit_once(':').is_some_and(|(h, p)| !h.contains(':') && p.parse::<u16>().is_ok());
    let with_port = if has_port { target.to_string() } else { format!("{target}:{DEFAULT_PORT}") };
    with_port.to_socket_addrs()?.next().with_context(|| format!("接続先が見つかりません: {target}"))
}

pub(super) fn handshake(target: &str, seq: u64) -> anyhow::Result<(TcpStream, BufReader<TcpStream>, Value)> {
    let address = resolve(target)?;
    let stream = TcpStream::connect_timeout(&address, Duration::from_secs(3)).with_context(|| format!("サーバー({address})につながりません"))?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut writer = stream.try_clone()?;
    writer.set_write_timeout(Some(Duration::from_secs(5)))?;
    writeln!(writer, "{}", json!({"t":"hello","protocol":PROTOCOL,"seq":seq}))?;
    let mut reader = BufReader::new(stream);
    let mut buf = Vec::new();
    ensure!(read_line(&mut reader, &mut buf)?, "サーバーが接続を閉じました");
    let v: Value = serde_json::from_slice(&buf)?;
    if v["t"] == "error" {
        bail!("{}", v["msg"].as_str().unwrap_or("サーバーに断られました"));
    }
    ensure!(v["t"] == "welcome", "サーバーの応答が不正です");
    // 状態は約30Hzで届く。長く届かなければ切れたとみなす
    reader.get_ref().set_read_timeout(Some(Duration::from_secs(5)))?;
    Ok((writer, reader, v))
}

pub(super) fn library_from(v: &Value) -> anyhow::Result<Library> {
    let profiles: Vec<Profile> = serde_json::from_value(v["profiles"].clone())?;
    let builtin: Vec<String> = serde_json::from_value(v.get("builtin").cloned().unwrap_or(json!([]))).unwrap_or_default();
    Ok(Library::from_profiles(profiles, builtin))
}

/// サーバーのプロジェクトに、その機械だけの項目(MIDI機器)を戻したもの
pub(super) fn project_from(server: &Value, local: Option<&Project>) -> anyhow::Result<Project> {
    let mut project: Project = serde_json::from_value(server.clone())?;
    project.midi_input = local.map(|p| p.midi_input.clone()).unwrap_or_default();
    Ok(project)
}

impl Client {
    /// サーバーにつなぎ、プロジェクトと機材定義を受け取る。続きの同期は `attach` で始める
    pub fn connect(target: &str, ctx: dmxctl_core::wake::Wake) -> anyhow::Result<(Self, Initial)> {
        let (writer, reader, v) = handshake(target, 0)?;
        let confirmed = v["project"].clone();
        let mut project = project_from(&confirmed, None)?;
        let lib = library_from(&v)?;
        let armed = v["rt"]["armed"].clone();
        let ids: Vec<u32> = serde_json::from_value(armed.clone()).unwrap_or_default();
        for f in project.fixtures.iter_mut() {
            f.armed = ids.contains(&f.id);
        }
        let learn = (val(&v["rt"], "learn", None), val(&v["rt"], "osc_learn", None));
        let sync = ClientSync {
            confirmed,
            pending: Vec::new(),
            rebuild: false,
            learn,
            learn_seq: 0,
            ack: 0,
            armed,
            devices: None,
            devices_count: 0,
            files: None,
            connected: true,
            offline_noticed: false,
            status: format!("接続中: {target}"),
            errors: Vec::new(),
        };
        let client = Client {
            target: target.to_string(),
            link: Link::new(),
            sync: Arc::new(Mutex::new(sync)),
            stop: Arc::new(AtomicBool::new(false)),
            pending: Some((writer, reader, v)),
            supervisor: None,
            ctx,
            before: None,
        };
        Ok((client, Initial { project, lib }))
    }

    /// 手元の写しのCoreを、サーバーの状態に合わせ続ける(切れたら2秒ごとにつなぎ直す)。
    /// このあと、Core::live の操作はサーバーへ送られる
    pub fn attach(&mut self, core: Shared) {
        let (writer, reader, welcome) = self.pending.take().expect("attach once");
        {
            let mut c = lock(&core);
            c.rt.remote = Some(std::sync::Arc::new(self.link.clone()));
            apply_rt_state(&mut c, &welcome["rt"]);
            let s = guard(&self.sync);
            c.rt.learn = s.learn.0;
            c.rt.osc_learn = s.learn.1;
        }
        // 送る口は先に開いておく(スレッドが動き出す前に送ったものも落とさない)
        let (tx, rx) = mpsc::channel::<String>();
        *guard(&self.link.inner.tx) = Some(tx);
        self.request_devices();
        let (link, sync, stop, target, ctx) = (self.link.clone(), self.sync.clone(), self.stop.clone(), self.target.clone(), self.ctx.clone());
        self.supervisor = Some(
            std::thread::Builder::new()
                .name("remote-client".into())
                .spawn(move || {
                    let mut conn = Some((writer, reader, rx));
                    while !stop.load(Ordering::Relaxed) {
                        let (writer, reader, rx) = match conn.take() {
                            Some(c) => c,
                            None => match handshake(&target, link.sent()) {
                                Ok((w, r, v)) => {
                                    let (tx, rx) = mpsc::channel::<String>();
                                    apply_welcome(&core, &sync, &v);
                                    *guard(&link.inner.tx) = Some(tx);
                                    link.send(json!({"t":"devices"}));
                                    (w, r, rx)
                                }
                                Err(e) => {
                                    guard(&sync).status = format!("切断(再接続中): {e}");
                                    ctx.request_repaint();
                                    std::thread::sleep(Duration::from_secs(2));
                                    continue;
                                }
                            },
                        };
                        {
                            let mut s = guard(&sync);
                            s.connected = true;
                            s.offline_noticed = false;
                            s.status = format!("接続中: {target}");
                        }
                        let error = run_connection(writer, reader, rx, &core, &sync, &link, &stop, &ctx);
                        *guard(&link.inner.tx) = None;
                        {
                            let mut s = guard(&sync);
                            s.connected = false;
                            s.status = format!("切断(再接続中){}", error.map(|e| format!(": {e}")).unwrap_or_default());
                        }
                        ctx.request_repaint();
                        if !stop.load(Ordering::Relaxed) {
                            std::thread::sleep(Duration::from_secs(1));
                        }
                    }
                })
                .expect("remote client thread"),
        );
    }

    /// 画面の1フレームの前: サーバーから届いたプロジェクト・ラーンの状態を写しに入れ、前の状態を覚える
    pub fn before_frame(&mut self, core: &Shared) {
        let mut c = lock(core);
        let mut s = guard(&self.sync);
        if s.rebuild {
            s.rebuild = false;
            let mut value = s.confirmed.clone();
            let ClientSync { pending, errors, .. } = &mut *s;
            pending.retain(|(_, changes)| {
                let ok = project_patch::apply(&mut value, changes).is_ok();
                if !ok {
                    errors.push("サーバーで変わった項目と重なった編集を取り消しました".into());
                }
                ok
            });
            match project_from(&value, Some(&c.project)) {
                Ok(project) => {
                    c.project = project;
                    apply_armed(&mut c, &s.armed);
                }
                Err(e) => s.errors.push(format!("サーバーのプロジェクトを読めません: {e}")),
            }
        }
        // ラーンは、自分が送ったものが処理された後だけ、サーバーの状態に合わせる
        if s.ack >= s.learn_seq {
            c.rt.learn = s.learn.0;
            c.rt.osc_learn = s.learn.1;
        }
        self.before = serde_json::to_value(&c.project).ok().map(|p| (p, (c.rt.learn, c.rt.osc_learn)));
    }

    /// 画面の1フレームの後: 手元で書き換えたプロジェクト・ラーンの状態をサーバーへ送る
    pub fn after_frame(&mut self, core: &Shared) {
        let Some((before, learn)) = self.before.take() else { return };
        let c = lock(core);
        let mut s = guard(&self.sync);
        if !s.connected {
            // 切れているあいだの編集は送れない(つなぎ直すとサーバーの内容で上書きされる)。黙って捨てずに知らせる
            if !s.offline_noticed && serde_json::to_value(&c.project).is_ok_and(|now| now != before) {
                s.offline_noticed = true;
                s.errors.push("サーバーにつながっていないので、この編集はサーバーに届きません(つなぎ直すとサーバーの内容に戻ります)".into());
            }
            return;
        }
        if let Ok(now) = serde_json::to_value(&c.project) {
            let changes = project_patch::diff_project(&before, &now);
            if !changes.is_empty() {
                let seq = self.link.send(json!({"t":"patch","changes":changes}));
                s.pending.push((seq, changes));
            }
        }
        let now_learn = (c.rt.learn, c.rt.osc_learn);
        if now_learn != learn {
            self.link.live(&LiveOp::SetLearn { midi: now_learn.0, osc: now_learn.1 });
            s.learn_seq = self.link.sent();
        }
    }

    /// サーバーで保存する
    pub fn save(&self) {
        self.link.send(json!({"t":"save"}));
    }

    /// サーバーのPCの機器の一覧を頼む(届いたら `devices` で読める)
    pub fn request_devices(&self) {
        self.link.send(json!({"t":"devices"}));
    }

    /// サーバーのPCの機器の一覧と、受け取った回数(変わったかどうかを見る)
    pub fn devices(&self) -> Option<(u64, Devices)> {
        let s = guard(&self.sync);
        s.devices.clone().map(|d| (s.devices_count, d))
    }

    /// サーバーのPCのプロジェクトのファイルの一覧を頼む(届いたら `files` で読める)
    pub fn request_files(&self) {
        self.link.send(json!({"t":"files"}));
    }

    pub fn files(&self) -> Option<Files> {
        guard(&self.sync).files.clone()
    }

    /// サーバーのPCのファイルを読み込んでもらう
    pub fn open_path(&self, path: &str) {
        self.link.send(json!({"t":"open","path":path}));
    }

    /// このPCのファイルの中身を送って、サーバーで読み込んでもらう
    pub fn open_text(&self, name: &str, text: &str) {
        self.link.send(json!({"t":"open_content","name":name,"text":text}));
    }

    /// 機材定義をサーバーのフォルダに保存する / 消す
    pub fn save_profile(&self, profile: &Profile) {
        self.link.send(json!({"t":"profile_save","profile":profile}));
    }

    pub fn delete_profile(&self, id: &str) {
        self.link.send(json!({"t":"profile_delete","id":id}));
    }

    pub fn connected(&self) -> bool {
        guard(&self.sync).connected
    }

    pub fn status(&self) -> String {
        guard(&self.sync).status.clone()
    }

    /// サーバーが返したエラーなど(画面に出す)
    pub fn take_errors(&self) -> Vec<String> {
        std::mem::take(&mut guard(&self.sync).errors)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        *guard(&self.link.inner.tx) = None;
        if let Some(t) = self.supervisor.take() {
            let _ = t.join();
        }
    }
}

/// つなぎ直したとき: サーバーの今のプロジェクト・状態にそろえる(切れている間の手元の編集は捨てる)
pub(super) fn apply_welcome(core: &Shared, sync: &Mutex<ClientSync>, v: &Value) {
    let lib = library_from(v);
    let mut c = lock(core);
    let mut s = guard(sync);
    match lib {
        Ok(lib) => c.lib = Arc::new(lib),
        Err(e) => s.errors.push(format!("機材定義を読めません: {e}")),
    }
    s.confirmed = v["project"].clone();
    s.pending.clear();
    s.rebuild = true;
    s.armed = v["rt"]["armed"].clone();
    s.learn = (val(&v["rt"], "learn", None), val(&v["rt"], "osc_learn", None));
    s.learn_seq = 0;
    apply_rt_state(&mut c, &v["rt"]);
}

pub(super) fn apply_state(core: &Shared, sync: &Mutex<ClientSync>, link: &Link, v: &Value) {
    let mut c = lock(core);
    let mut s = guard(sync);
    if v.get("profiles").is_some() {
        match library_from(v) {
            Ok(lib) => c.lib = Arc::new(lib),
            Err(e) => s.errors.push(format!("機材定義を読めません: {e}")),
        }
    }
    let ack = v["ack"].as_u64().unwrap_or(0);
    s.ack = ack;
    // サーバーと同じ順に当てる: 処理済みの自分の編集 → 届いた差分
    let mut changed = false;
    while s.pending.first().is_some_and(|(seq, _)| *seq <= ack) {
        let (_, changes) = s.pending.remove(0);
        let ClientSync { confirmed, .. } = &mut *s;
        // 通らなかった編集はサーバーも当てていない(サーバーは全部を送り直してくる)
        let _ = project_patch::apply(confirmed, &changes);
        changed = true;
    }
    if let Some(changes) = v.get("project") {
        let changes: Vec<Change> = serde_json::from_value(changes.clone()).unwrap_or_default();
        let ClientSync { confirmed, .. } = &mut *s;
        if project_patch::apply(confirmed, &changes).is_err() {
            // 食い違った: 全部を送り直してもらう
            link.send(json!({"t":"resync"}));
        }
        changed = true;
    }
    if changed {
        s.rebuild = true;
    }
    if let Some(frames) = v["frames"].as_object() {
        for (u, h) in frames {
            if let (Ok(u), Some(h)) = (u.parse::<usize>(), h.as_str())
                && let Some(f) = c.rt.frames.get_mut(u)
            {
                unhex(h, f);
            }
        }
    }
    let learn = (c.rt.learn, c.rt.osc_learn);
    apply_rt_state(&mut c, &v["rt"]);
    // ラーンは画面で書き換えるので、ここでは戻さない(フレームの境目で入れる)
    (c.rt.learn, c.rt.osc_learn) = learn;
    s.learn = (val(&v["rt"], "learn", None), val(&v["rt"], "osc_learn", None));
    s.armed = v["rt"]["armed"].clone();
}

#[allow(clippy::too_many_arguments)]
pub(super) fn run_connection(
    writer: TcpStream,
    mut reader: BufReader<TcpStream>,
    rx: mpsc::Receiver<String>,
    core: &Shared,
    sync: &Mutex<ClientSync>,
    link: &Link,
    stop: &AtomicBool,
    ctx: &dmxctl_core::wake::Wake,
) -> Option<String> {
    let alive = Arc::new(AtomicBool::new(true));
    let sender = {
        let alive = alive.clone();
        let mut writer = writer;
        std::thread::Builder::new()
            .name("remote-client-send".into())
            .spawn(move || {
                while alive.load(Ordering::Relaxed) {
                    match rx.recv_timeout(Duration::from_millis(500)) {
                        Ok(line) => {
                            if writeln!(writer, "{line}").is_err() {
                                break;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(_) => break,
                    }
                }
                alive.store(false, Ordering::Relaxed);
                let _ = writer.shutdown(std::net::Shutdown::Both);
            })
            .expect("remote client sender")
    };
    let mut buf = Vec::new();
    let error = loop {
        if stop.load(Ordering::Relaxed) || !alive.load(Ordering::Relaxed) {
            break None;
        }
        match read_line(&mut reader, &mut buf) {
            Ok(true) => {}
            Ok(false) => break Some("サーバーが接続を閉じました".to_string()),
            Err(e) => break Some(e.to_string()),
        }
        let Ok(v) = serde_json::from_slice::<Value>(&buf) else { continue };
        match v["t"].as_str() {
            Some("state") => {
                apply_state(core, sync, link, &v);
                ctx.request_repaint();
            }
            Some("files") => {
                if let Ok(f) = serde_json::from_value::<Files>(v["files"].clone()) {
                    guard(sync).files = Some(f);
                }
                ctx.request_repaint();
            }
            // サーバーからの知らせ(読み込みの結果など)。エラーと同じく画面の下に出す
            Some("notice") => {
                guard(sync).errors.push(v["msg"].as_str().unwrap_or("").to_string());
                ctx.request_repaint();
            }
            Some("devices") => {
                if let Ok(d) = serde_json::from_value::<Devices>(v["devices"].clone()) {
                    let mut s = guard(sync);
                    s.devices = Some(d);
                    s.devices_count += 1;
                }
                ctx.request_repaint();
            }
            Some("error") => {
                guard(sync).errors.push(v["msg"].as_str().unwrap_or("").to_string());
                ctx.request_repaint();
                if v["fatal"] == true {
                    break v["msg"].as_str().map(str::to_string);
                }
            }
            _ => {}
        }
    };
    alive.store(false, Ordering::Relaxed);
    let _ = reader.get_ref().shutdown(std::net::Shutdown::Both);
    *guard(&link.inner.tx) = None;
    let _ = sender.join();
    error
}
