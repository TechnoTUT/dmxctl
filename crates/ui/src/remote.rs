//! 起動後に、サーバー・クライアントの動作へ切り替える画面。
//! 引数付きで起動しなくても、`--remote-listen` と `dmxctl connect` の相当を利用できる。
use crate::app::{wake_of, App};
use dmxctl_backend::host::Host;
use eframe::egui::{self, RichText};
use std::sync::mpsc::{channel, Receiver};

/// 画面に残す、サーバー・クライアントの編集中の値
pub struct RemoteUi {
    /// サーバーとして待ち受けるIPアドレスとポートの編集中の値
    pub listen_ip: String,
    pub listen_port: u16,
    pub listen_error: String,
    /// 「サーバーに接続」の入力欄。開いている間だけある
    pub connect: Option<ConnectDialog>,
    /// 終了後に起動し直す引数。終了のあとで実行する
    pub relaunch: Option<Vec<String>>,
}

pub struct ConnectDialog {
    pub target: String,
    pub error: String,
    /// 接続を確認している間の結果の受け口
    pub probing: Option<Receiver<Result<(), String>>>,
}

impl Default for RemoteUi {
    fn default() -> Self {
        Self {
            listen_ip: dmxctl_backend::web::local_ip().map(|ip| ip.to_string()).unwrap_or_else(|| "0.0.0.0".into()),
            listen_port: dmxctl_backend::remote::DEFAULT_PORT,
            listen_error: String::new(),
            connect: None,
            relaunch: None,
        }
    }
}

impl App {
    /// 別のPCの画面から操作させるための待ち受け。保存せず、起動のたびに停止から始める
    pub(crate) fn remote_row(&mut self, ui: &mut egui::Ui) {
        if self.host.is_client() {
            return;
        }
        let mut on = self.host.remote.is_some();
        let before = on;
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(!on, |ui| {
                ui.label("別のPCから操作する");
            });
            ui.add_enabled_ui(!on, |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.remote.listen_ip).desired_width(120.0))
                    .on_hover_text("待ち受けるIPアドレス。照明用のLANのアドレスを指定すると、そのLANからだけ接続できます。どこからでも受けるなら 0.0.0.0");
                ui.label("ポート");
                ui.add(egui::DragValue::new(&mut self.remote.listen_port).range(1024..=65535));
            });
            ui.checkbox(&mut on, "待ち受ける").on_hover_text("ONにすると、別のPCの dmxctl から、この PC の画面と同じ操作ができます。認証が無いため、照明用のLANの中だけで利用してください。この設定は保存せず、起動のたびにOFFになります");
            if let Some(r) = &self.host.remote {
                ui.label(RichText::new(format!("接続先: {}", r.address)).monospace());
            } else if !self.remote.listen_error.is_empty() {
                ui.label(RichText::new(&self.remote.listen_error).color(crate::theme::WARN));
            }
        });
        if on != before {
            self.remote.listen_error.clear();
            if on {
                let text = format!("{}:{}", self.remote.listen_ip.trim(), self.remote.listen_port);
                match text.parse::<std::net::SocketAddr>() {
                    Ok(address) => match self.host.remote_start(address) {
                        Ok(()) => self.host.notice = format!("クライアントの待ち受けを始めました: {address}。認証が無いため、信頼できるLANだけで利用してください"),
                        Err(e) => self.remote.listen_error = e.to_string(),
                    },
                    Err(_) => self.remote.listen_error = format!("アドレスが不正です: {text}"),
                }
            } else {
                self.host.remote_stop();
                self.host.notice = "クライアントの待ち受けを止めました".into();
            }
        }
    }

    /// ファイルメニューの項目。サーバーに接続する画面を開く
    pub(crate) fn connect_menu_item(&mut self, ui: &mut egui::Ui) {
        if ui.button("サーバーに接続…").on_hover_text("別のPCで動いている dmxctl に接続して、この PC は画面と MIDI コントローラーだけにします。この PC の照明の出力は止まります").clicked() {
            ui.close();
            self.remote.connect.get_or_insert(ConnectDialog { target: String::new(), error: String::new(), probing: None });
        }
    }

    /// クライアントのファイルメニューの項目。通常の起動へ戻す
    pub(crate) fn disconnect_menu_item(&mut self, ui: &mut egui::Ui) {
        if ui.button("サーバーから切断して通常起動に戻す").on_hover_text("このウィンドウを閉じて、サーバーに接続しない通常の起動をやり直します").clicked() {
            ui.close();
            self.remote.relaunch = Some(Vec::new());
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    /// 「サーバーに接続」の入力欄。接続できると確かめてから、起動し直す
    pub(crate) fn connect_dialog(&mut self, ctx: &egui::Context) {
        let Some(dialog) = self.remote.connect.as_mut() else { return };
        if let Some(rx) = &dialog.probing {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    let args = vec!["connect".to_string(), dialog.target.trim().to_string()];
                    self.remote.connect = None;
                    self.remote.relaunch = Some(args);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                Ok(Err(e)) => {
                    dialog.error = e;
                    dialog.probing = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => dialog.probing = None,
            }
        }
        let mut open = true;
        let mut start = false;
        let mut cancel = false;
        egui::Window::new("サーバーに接続").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).open(&mut open).show(ctx, |ui| {
            ui.label("サーバーのIPアドレスを入力してください。ポートの既定は 9300 です。");
            ui.label(RichText::new("サーバー側は、MIDI設定タブで「待ち受ける」をONにするか、--remote-listen を付けて起動しておきます").small().weak());
            let busy = dialog.probing.is_some();
            let edit = ui.add_enabled(!busy, egui::TextEdit::singleline(&mut dialog.target).hint_text("192.168.1.10").desired_width(220.0));
            if edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                start = true;
            }
            ui.label(RichText::new("接続すると、この PC の照明の出力・OSC・Link は止まります").small().color(crate::theme::WARN));
            if !dialog.error.is_empty() {
                ui.label(RichText::new(&dialog.error).color(crate::theme::WARN));
            }
            ui.horizontal(|ui| {
                if ui.add_enabled(!busy && !dialog.target.trim().is_empty(), egui::Button::new(if busy { "確認中…" } else { "接続" })).clicked() {
                    start = true;
                }
                if ui.button("キャンセル").clicked() {
                    cancel = true;
                }
            });
        });
        if start && dialog.probing.is_none() && !dialog.target.trim().is_empty() {
            dialog.error.clear();
            let (tx, rx) = channel();
            let (target, wake) = (dialog.target.trim().to_string(), wake_of(ctx));
            let repaint = ctx.clone();
            let spawned = std::thread::Builder::new().name("connect-probe".into()).spawn(move || {
                let _ = tx.send(Host::probe_server(&target, wake).map_err(|e| e.to_string()));
                repaint.request_repaint();
            });
            match spawned {
                Ok(_) => dialog.probing = Some(rx),
                Err(e) => dialog.error = e.to_string(),
            }
        }
        if !open || cancel {
            self.remote.connect = None;
        } else if self.remote.connect.as_ref().is_some_and(|d| d.probing.is_some()) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}
