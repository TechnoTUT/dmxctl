//! 画面。APC mini mk2 と同じ並びのボタン、フェーダー、4台のプレビュー、機材の設定。

use crate::apc::{self, Input};
use crate::core::{lock, Config, Shared};
use crate::dmx::{self, OpenDmx};
use crate::engine::*;
use crate::fixture::{Head, Mode, COLORS, PAR_CHANNELS};
use eframe::egui::{self, vec2, Align2, Color32, FontFamily, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const BG_PAD: Color32 = Color32::from_rgb(34, 36, 42);

pub struct App {
    shared: Arc<Shared>,
    dmx_port: Option<String>,
    dmx: Option<OpenDmx>,
    ports: Vec<(String, String)>,
    midi: Option<apc::Connection>,
    midi_error: String,
    last_scan: Option<Instant>,
    held: HashMap<Action, bool>,
    saved: Config,
    last_save: Instant,
    shot: Option<crate::shot::Shot>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        setup_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
        let cfg = Config::load();
        let mut state = State::new(cfg.setup.clone(), cfg.bpm);
        state.look = cfg.look;
        let shared = Shared::new(state);
        shared.spawn_engine();
        let dmx = cfg.dmx_port.clone().map(|p| OpenDmx::new(p, shared.frame.clone()));
        App {
            shared,
            dmx_port: cfg.dmx_port.clone(),
            dmx,
            ports: dmx::list_ports(),
            midi: None,
            midi_error: String::new(),
            last_scan: None,
            held: HashMap::new(),
            saved: cfg,
            last_save: Instant::now(),
            shot: crate::shot::Shot::from_env(),
        }
    }

    fn config(&self) -> Config {
        let s = lock(&self.shared.state);
        Config { setup: s.setup.clone(), look: s.look, bpm: s.clock.bpm(), dmx_port: self.dmx_port.clone() }
    }

    fn autosave(&mut self) {
        if self.last_save.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_save = Instant::now();
        let c = self.config();
        if c != self.saved {
            match c.save() {
                Ok(()) => self.saved = c,
                Err(e) => eprintln!("設定を保存できませんでした: {e}"),
            }
        }
    }

    /// APC mini mk2 を2秒ごとに探して、つながっていなければ接続する(抜けたら切る)
    fn scan_midi(&mut self, ctx: &egui::Context) {
        if self.last_scan.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
            return;
        }
        self.last_scan = Some(Instant::now());
        let ports = apc::input_ports();
        if let Some(c) = &self.midi {
            if !ports.contains(&c.port) {
                self.midi = None;
                apc::disconnect(&self.shared.leds);
            }
            return;
        }
        let Some(port) = apc::find_port(&ports) else {
            self.midi_error = "APC mini mk2 が見つかりません".into();
            return;
        };
        let shared = self.shared.clone();
        let ctx = ctx.clone();
        let on_input = move |i: Input| {
            {
                let mut s = lock(&shared.state);
                match i {
                    Input::Button(a, down) => s.press(a, down, shared.now()),
                    Input::Fader(n, v) => s.faders[n] = v,
                    Input::Faders(f) => s.faders = f,
                }
            }
            ctx.request_repaint();
        };
        match apc::connect(&port, &self.shared.leds, on_input) {
            Ok(c) => {
                self.midi = Some(c);
                self.midi_error.clear();
            }
            Err(e) => self.midi_error = format!("APC mini mk2 に接続できません: {e}"),
        }
    }

    fn set_dmx_port(&mut self, port: Option<String>) {
        if port == self.dmx_port {
            return;
        }
        self.dmx = None; // 先に今のポートを閉じる
        self.dmx = port.clone().map(|p| OpenDmx::new(p, self.shared.frame.clone()));
        self.dmx_port = port;
    }

    fn press(&self, a: Action, down: bool) {
        lock(&self.shared.state).press(a, down, self.shared.now());
    }

    fn keyboard(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (space, b, f, s) = ctx.input(|i| (i.key_pressed(egui::Key::Space), i.key_pressed(egui::Key::B), i.key_down(egui::Key::F), i.key_down(egui::Key::S)));
        if space {
            self.press(Action::Tap, true);
        }
        if b {
            self.press(Action::Blackout, true);
        }
        for (a, down) in [(Action::Flash, f), (Action::Strobe, s)] {
            let was = self.held.get(&a).copied().unwrap_or(false);
            if was != down {
                self.press(a, down);
                self.held.insert(a, down);
            }
        }
    }

    // ------------------------------------------------------------ 上の帯

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("SPOT4").font(FontId::new(20.0, bold())).color(Color32::WHITE));
            ui.add_space(16.0);

            let (bpm, beat) = {
                let s = lock(&self.shared.state);
                (s.clock.bpm(), s.beat)
            };
            let mut v = bpm;
            ui.label("BPM");
            if ui.add(egui::DragValue::new(&mut v).range(60.0..=200.0).speed(0.1).fixed_decimals(1)).changed() {
                let t = self.shared.now();
                lock(&self.shared.state).clock.set_bpm(v, t);
            }
            for a in [Action::Tap, Action::Half, Action::Double, Action::Downbeat] {
                if ui.button(apc::label(a)).clicked() {
                    self.press(a, true);
                }
            }
            // 拍の表示(小節の何拍目か)
            let (rect, _) = ui.allocate_exact_size(vec2(4.0 * 18.0, 18.0), Sense::hover());
            let n = beat.rem_euclid(4.0) as usize;
            for k in 0..4 {
                let c = rect.left_center() + vec2(9.0 + k as f32 * 18.0, 0.0);
                let col = if k == n { if k == 0 { Color32::from_rgb(255, 80, 80) } else { Color32::from_rgb(80, 230, 110) } } else { Color32::from_gray(60) };
                ui.painter().circle_filled(c, 6.0, col);
            }

            ui.add_space(20.0);
            ui.separator();
            ui.label("USB-DMX");
            let selected = self.dmx_port.clone();
            let mut choice = selected.clone();
            egui::ComboBox::from_id_salt("dmx_port").width(230.0).selected_text(selected.as_deref().unwrap_or("出力しない(画面だけ)")).show_ui(ui, |ui| {
                ui.selectable_value(&mut choice, None, "出力しない(画面だけ)");
                for (name, desc) in &self.ports {
                    let text = if desc.is_empty() { name.clone() } else { format!("{name}  ({desc})") };
                    ui.selectable_value(&mut choice, Some(name.clone()), text);
                }
            });
            if ui.button("⟳").on_hover_text("ポートを探し直す").clicked() {
                self.ports = dmx::list_ports();
            }
            self.set_dmx_port(choice);
            let status = self.dmx.as_ref().map(|d| d.status()).unwrap_or_else(|| "画面だけ".into());
            let ok = status == "出力中";
            ui.label(RichText::new(status).color(if ok { Color32::from_rgb(80, 230, 110) } else { Color32::from_rgb(240, 180, 60) }));

            ui.separator();
            match &self.midi {
                Some(_) => ui.label(RichText::new("APC mini mk2 接続中").color(Color32::from_rgb(80, 230, 110))),
                None => ui.label(RichText::new(&self.midi_error).color(Color32::from_rgb(240, 180, 60))),
            };
        });
    }

    // ------------------------------------------------------------ APCの図

    fn button(&mut self, ui: &mut egui::Ui, rect: Rect, a: Action, active: bool) {
        self.button_named(ui, rect, a, active, apc::label(a));
    }

    fn button_named(&mut self, ui: &mut egui::Ui, rect: Rect, a: Action, active: bool, label: String) {
        let id = ui.id().with(("btn", format!("{a:?}")));
        let resp = ui.interact(rect, id, Sense::click_and_drag());
        if a.momentary() {
            let down = resp.is_pointer_button_down_on();
            let was = self.held.get(&a).copied().unwrap_or(false);
            if down != was {
                self.press(a, down);
                self.held.insert(a, down);
            }
        } else if resp.clicked() {
            self.press(a, true);
        }
        let [r, g, b] = apc::palette_rgb(apc::palette(a));
        let base = Color32::from_rgb(r, g, b);
        let p = ui.painter();
        let fill = if active { base } else { mix(BG_PAD, base, 0.22) };
        p.rect_filled(rect, 5.0, if resp.hovered() && !active { mix(fill, Color32::WHITE, 0.08) } else { fill });
        let text = if active { Color32::BLACK } else { mix(Color32::from_gray(200), base, 0.3) };
        let galley = p.layout(label, FontId::proportional(12.5), text, rect.width() - 6.0);
        p.galley(rect.center() - galley.size() / 2.0, galley, text);
    }

    fn controller(&mut self, ui: &mut egui::Ui) {
        const W: f32 = 76.0;
        const H: f32 = 44.0;
        const GAP: f32 = 5.0;
        let (rect, _) = ui.allocate_exact_size(vec2(9.0 * (W + GAP) + 6.0, 9.0 * (H + GAP)), Sense::hover());
        let origin = rect.min;
        let (active, row): (HashMap<Action, bool>, [Unit; N]) = {
            let s = lock(&self.shared.state);
            let mut m = HashMap::new();
            for r in 0..8 {
                for c in 0..8 {
                    if let Some(a) = apc::pad(r, c) {
                        m.insert(a, s.is_active(a));
                    }
                }
            }
            for i in 0..8u8 {
                m.insert(Action::Scene(i), s.is_active(Action::Scene(i)) || s.setup.scenes[i as usize].is_some() && s.shift);
                m.insert(Action::Fixture(i), i < N as u8 && s.is_active(Action::Fixture(i)));
            }
            m.insert(Action::Shift, s.shift);
            (m, s.setup.row())
        };
        for r in 0..8 {
            for c in 0..8 {
                let Some(a) = apc::pad(r, c) else { continue };
                let rr = Rect::from_min_size(origin + vec2(c as f32 * (W + GAP), r as f32 * (H + GAP)), vec2(W, H));
                self.button(ui, rr, a, active[&a]);
            }
        }
        // 右: シーン(SHIFTを押しながらで保存)
        for i in 0..8u8 {
            let rr = Rect::from_min_size(origin + vec2(8.0 * (W + GAP) + 6.0, i as f32 * (H + GAP)), vec2(W, H));
            self.button(ui, rr, Action::Scene(i), active[&Action::Scene(i)]);
        }
        // 下: トラックボタン(並びの左から1〜4台のオン/オフ)とSHIFT
        let y = 8.0 * (H + GAP) + 4.0;
        for i in 0..N as u8 {
            let rr = Rect::from_min_size(origin + vec2(i as f32 * (W + GAP), y), vec2(W, H - 10.0));
            self.button_named(ui, rr, Action::Fixture(i), active[&Action::Fixture(i)], unit_name(row[i as usize]));
        }
        let rr = Rect::from_min_size(origin + vec2(8.0 * (W + GAP) + 6.0, y), vec2(W, H - 10.0));
        self.button(ui, rr, Action::Shift, active[&Action::Shift]);
    }

    fn faders(&mut self, ui: &mut egui::Ui) {
        let mut f = lock(&self.shared.state).faders;
        let before = f;
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = 120.0;
            for (i, v) in f.iter_mut().enumerate() {
                ui.allocate_ui(vec2(76.0, 170.0), |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add(egui::Slider::new(v, 0.0..=1.0).vertical().show_value(false));
                        ui.label(RichText::new(format!("{:.0}%", *v * 100.0)).monospace().size(11.0));
                        ui.label(RichText::new(FADER_NAMES[i]).size(11.5));
                    });
                });
            }
        });
        if f != before {
            lock(&self.shared.state).faders = f;
        }
    }

    // ------------------------------------------------------------ プレビュー

    fn preview(&mut self, ui: &mut egui::Ui) {
        let (heads, pars, angles, enabled, setup, t) = {
            let s = lock(&self.shared.state);
            (s.heads, s.pars, s.angles, s.enabled, s.setup.clone(), self.shared.now())
        };
        ui.label(RichText::new("プレビュー(客席から見た図。床に横一列。手前に向くビームほど太く見える)").size(12.0).color(Color32::GRAY));
        let (rect, _) = ui.allocate_exact_size(vec2(440.0, 270.0), Sense::hover());
        draw_stage(ui, rect, &setup.row(), &heads, &pars, &angles, &enabled, t);
        ui.add_space(4.0);
        let line = |ui: &mut egui::Ui, name: String, addr: u16, ch: &[u8]| {
            let text = ch.iter().map(|v| format!("{v:>3}")).collect::<Vec<_>>().join(" ");
            ui.label(RichText::new(format!("{name:<6} @{addr:<3} {text}")).monospace().size(10.5).color(Color32::GRAY));
        };
        for (i, h) in heads.iter().enumerate() {
            let mut ch = vec![0u8; setup.mode.channels()];
            crate::fixture::encode(h, setup.mode, &mut ch);
            line(ui, unit_name(Unit::Spot(i)), setup.spot_addresses[i], &ch);
        }
        for (i, p) in pars.iter().enumerate() {
            let mut ch = [0u8; PAR_CHANNELS];
            crate::fixture::encode_par(p, &mut ch);
            line(ui, unit_name(Unit::Par(i)), setup.par_addresses[i], &ch);
        }
    }

    fn settings(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("機材の設定").default_open(false).show(ui, |ui| {
            let mut setup = lock(&self.shared.state).setup.clone();
            let before = setup.clone();
            ui.horizontal(|ui| {
                ui.label("並び(客席から見て左から)");
                egui::ComboBox::from_id_salt("layout").selected_text(LAYOUTS[setup.layout as usize % LAYOUTS.len()].0).show_ui(ui, |ui| {
                    for (k, (name, _)) in LAYOUTS.iter().enumerate() {
                        ui.selectable_value(&mut setup.layout, k as u8, *name);
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label("ムービング(MINI SPOT 30)のモード(本体の CHnd と合わせる)");
                ui.radio_value(&mut setup.mode, Mode::Ch9, "9ch");
                ui.radio_value(&mut setup.mode, Mode::Ch11, "11ch(なめらか)");
            });
            egui::Grid::new("fixtures").num_columns(4).spacing([14.0, 6.0]).show(ui, |ui| {
                ui.label("");
                ui.label("DMXアドレス");
                ui.label("Pan反転");
                ui.label("Tilt反転");
                ui.end_row();
                for i in 0..N_SPOT {
                    ui.label(unit_name(Unit::Spot(i)));
                    ui.add(egui::DragValue::new(&mut setup.spot_addresses[i]).range(1..=512));
                    ui.checkbox(&mut setup.spot_invert_pan[i], "");
                    ui.checkbox(&mut setup.spot_invert_tilt[i], "");
                    ui.end_row();
                }
                for i in 0..N_PAR {
                    ui.label(format!("{}(RGBWA 5ch)", unit_name(Unit::Par(i))));
                    ui.add(egui::DragValue::new(&mut setup.par_addresses[i]).range(1..=512));
                    ui.end_row();
                }
            });
            ui.horizontal(|ui| {
                ui.label("LED(GLP)の最大の明るさ");
                ui.add(egui::Slider::new(&mut setup.par_max, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)))
                    .on_hover_text("フラッシュ・ストロボも含めて、LEDはこれより明るくならない。ムービングには効かない");
            });
            ui.add_space(6.0);
            ui.label(RichText::new("ムービングの向き合わせ").strong());
            ui.label(RichText::new("1.「真上」にして、ビームがまっすぐ上を向くまで「真上のTilt」を動かす\n2.「正面」にして、客席側へ45°倒れるまで「正面のPan」を動かす\n   後ろへ倒れる台は Tilt反転、左右が逆に動く台は Pan反転 にチェック").size(12.0).color(Color32::GRAY));
            let mut calib = lock(&self.shared.state).calib;
            ui.horizontal(|ui| {
                ui.radio_value(&mut calib, Calib::Off, "演出");
                ui.radio_value(&mut calib, Calib::Up, "真上(白・全灯)");
                ui.radio_value(&mut calib, Calib::Front, "正面(白・全灯)");
            });
            lock(&self.shared.state).calib = calib;
            ui.add(egui::Slider::new(&mut setup.tilt_up, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0}", v * 255.0)).text("真上のTilt"));
            ui.add(egui::Slider::new(&mut setup.pan_front, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0}", v * 255.0)).text("正面のPan"));
            if setup != before {
                lock(&self.shared.state).setup = setup;
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("フェーダーを初期値に").clicked() {
                    lock(&self.shared.state).faders = FADER_DEFAULTS;
                }
                if ui.button("シーンを全部消す").clicked() {
                    let mut s = lock(&self.shared.state);
                    s.setup.scenes = [None; 8];
                    s.active_scene = None;
                }
            });
        });
    }

    fn help(&self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("使い方").default_open(false).show(ui, |ui| {
            ui.label("・上から: 動き / 速さ・揃え方 / 色 / 色の変え方・向き / ゴボ / ゴボの変え方・客席・停止・オート / 明るさの効果 / キメ・テンポ");
            ui.label("・フラッシュ・ストロボは押している間だけ。ブラックアウト中でも光る");
            ui.label("・右の列(シーン): SHIFTを押しながらで今の見た目を保存、押すと呼び出し");
            ui.label("・下の段: 並びの左から1台ずつオン/オフ。右端のトラックボタンは拍で点滅");
            ui.label("・LED(RGBWA)は色・明るさの効果・ストロボに合わせて光る。動き・ゴボはムービングだけ");
            ui.label("・オート: 16小節ごとに見た目が自動で変わる");
            ui.label("・キーボード: Space=TAP / F=フラッシュ(押している間) / S=ストロボ(押している間) / B=ブラックアウト");
            ui.label("・ムービング本体は DMXモード、CHnd を 9CH(または11CH)、Addr を上の設定と同じにする");
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.scan_midi(&ctx);
        self.keyboard(&ctx);
        egui::Panel::top("top").show(ui, |ui| {
            ui.add_space(4.0);
            self.top_bar(ui);
            ui.add_space(2.0);
        });
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::both().auto_shrink([false; 2]).show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        self.controller(ui);
                        ui.add_space(6.0);
                        self.faders(ui);
                    });
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        self.preview(ui);
                        ui.add_space(8.0);
                        self.settings(ui);
                        self.help(ui);
                    });
                });
            });
        });
        self.autosave();
        if let Some(s) = self.shot.as_mut() {
            s.tick(&ctx);
        }
        ctx.request_repaint_after(Duration::from_millis(33));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Err(e) = self.config().save() {
            eprintln!("設定を保存できませんでした: {e}");
        }
        // 出力スレッドを止める前に消灯したフレームを送る(OpenDmx の Drop で全0を送る)
        self.dmx = None;
        apc::disconnect(&self.shared.leds);
    }
}

/// カラーchの値 → 画面の色
fn head_color(raw: u8, t: f64) -> Color32 {
    if raw >= 140 {
        let speed = 0.2 + (raw - 140) as f64 / 115.0 * 2.0;
        return hsv((t * speed).rem_euclid(1.0) as f32);
    }
    let idx = (raw / 10) as usize;
    let rgb = |k: usize| {
        let [r, g, b] = COLORS[k].2;
        Color32::from_rgb(r, g, b)
    };
    if idx < 8 {
        return rgb(idx);
    }
    // 2色の半分ずつ: マゼンタ/オレンジ, オレンジ/水色, 水色/黄, 黄/青, 青/緑, 緑/赤
    const SPLIT: [(usize, usize); 6] = [(7, 6), (6, 5), (5, 4), (4, 3), (3, 2), (2, 1)];
    let (a, b) = SPLIT[(idx - 8).min(5)];
    mix(rgb(a), rgb(b), 0.5)
}

#[allow(clippy::too_many_arguments)]
fn draw_stage(ui: &egui::Ui, rect: Rect, row: &[Unit; N], heads: &[Head; N_SPOT], pars: &[[f32; 5]; N_PAR], angles: &[[f64; 2]; N_SPOT], enabled: &[bool; N], t: f64) {
    let p = ui.painter().with_clip_rect(rect.shrink(1.0));
    let bg = Color32::from_rgb(14, 15, 19);
    p.rect_filled(rect, 6.0, bg);
    p.rect_stroke(rect, 6.0, Stroke::new(1.0, Color32::from_gray(55)), StrokeKind::Inside);
    let floor = rect.bottom() - 28.0;
    p.line_segment([Pos2::new(rect.left() + 8.0, floor), Pos2::new(rect.right() - 8.0, floor)], Stroke::new(1.5, Color32::from_gray(70)));
    let len = (floor - rect.top() - 12.0) as f64;
    for (u, unit) in row.iter().enumerate() {
        let base = Pos2::new(rect.left() + rect.width() * (u as f32 + 0.5) / N as f32, floor - 10.0);
        let (label, c, level) = match *unit {
            Unit::Spot(i) => {
                let h = &heads[i];
                let d = direction(angles[i][0], angles[i][1]);
                // ストロボ中は点滅させて見せる
                let strobe_on = h.strobe == 0 || (t * (4.0 + h.strobe as f64 / 20.0)).fract() < 0.5;
                let level = if strobe_on { h.dimmer } else { 0.0 };
                let c = head_color(h.color, t);
                if level > 0.01 {
                    let end = base + vec2((d[0] * len) as f32, -(d[1] * len) as f32);
                    // 手前(客席側)へ向くほど太く・明るく、奥へ向くほど暗く
                    let z = d[2] as f32;
                    let width = 3.0 + 9.0 * z.max(0.0);
                    let k = level * (0.55 + 0.45 * z.max(-0.6));
                    p.line_segment([base, end], Stroke::new(width + 6.0, mix(bg, c, k * 0.25)));
                    p.line_segment([base, end], Stroke::new(width, mix(bg, c, k)));
                    if z > 0.3 {
                        p.circle_filled(end, width * 0.9, mix(bg, c, k));
                    }
                }
                p.text(base + vec2(0.0, -16.0), Align2::CENTER_BOTTOM, gobo_name(h.gobo), FontId::proportional(10.0), Color32::from_gray(120));
                (format!("ムービング{}", i + 1), c, level)
            }
            Unit::Par(i) => {
                let (c, level) = par_color(&pars[i]);
                if level > 0.01 {
                    // 上向きの広い光(ウォッシュ)
                    for (spread, k) in [(70.0, 0.18), (45.0, 0.3), (22.0, 0.45)] {
                        let h = len as f32 * 0.75;
                        let pts = vec![base, base + vec2(-spread, -h), base + vec2(spread, -h)];
                        p.add(egui::Shape::convex_polygon(pts, mix(bg, c, level * k), Stroke::NONE));
                    }
                }
                (format!("LED{}", i + 1), c, level)
            }
        };
        p.rect_filled(Rect::from_center_size(base + vec2(0.0, 4.0), vec2(26.0, 14.0)), 3.0, Color32::from_gray(if enabled[u] { 90 } else { 45 }));
        p.circle_filled(base, 6.0, mix(Color32::from_gray(50), c, level.max(0.15)));
        let (label, col) = if enabled[u] { (label, Color32::GRAY) } else { (format!("{label} OFF"), Color32::from_rgb(255, 80, 80)) };
        p.text(Pos2::new(base.x, rect.bottom() - 8.0), Align2::CENTER_CENTER, label, FontId::proportional(10.5), col);
    }
}

fn gobo_name(g: u8) -> String {
    match g {
        0..=7 | 64..=71 => "オープン".into(),
        8..=63 => format!("ゴボ{}", [8u8, 16, 25, 32, 40, 48, 56].iter().rposition(|&v| g >= v).unwrap_or(0) + 1),
        72..=127 => "シェイク".into(),
        _ => "回転".into(),
    }
}

/// RGBWA → 画面の色と明るさ(白は白、アンバーは橙として足す)
fn par_color(v: &[f32; 5]) -> (Color32, f32) {
    let [r, g, b, w, a] = *v;
    let rgb = [r + w + a, g + w + a * 0.55, b + w];
    let level = rgb.iter().cloned().fold(0.0f32, f32::max);
    if level <= 0.0 {
        return (Color32::from_gray(60), 0.0);
    }
    let c = rgb.map(|x| (x / level * 255.0).round() as u8);
    (Color32::from_rgb(c[0], c[1], c[2]), level.min(1.0))
}

fn mix(a: Color32, b: Color32, k: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

fn hsv(h: f32) -> Color32 {
    let x = |o: f32| ((((h + o) * 6.0).rem_euclid(6.0) - 3.0).abs() - 1.0).clamp(0.0, 1.0);
    Color32::from_rgb((x(0.0) * 255.0) as u8, (x(2.0 / 3.0) * 255.0) as u8, (x(1.0 / 3.0) * 255.0) as u8)
}

fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    // (通常, 太字)の候補。Mac: ヒラギノ角ゴ / Windows: メイリオ
    const JP_FONTS: &[(&str, &str)] = &[
        ("/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", "/System/Library/Fonts/ヒラギノ角ゴシック W6.ttc"),
        ("/System/Library/Fonts/Hiragino Sans GB.ttc", ""),
        ("C:/Windows/Fonts/meiryo.ttc", "C:/Windows/Fonts/meiryob.ttc"),
        ("C:/Windows/Fonts/YuGothM.ttc", "C:/Windows/Fonts/YuGothB.ttc"),
    ];
    let mut bold_list: Vec<String> = Vec::new();
    for (regular, b) in JP_FONTS {
        let Ok(bytes) = std::fs::read(regular) else { continue };
        fonts.font_data.insert("jp".into(), Arc::new(egui::FontData::from_owned(bytes)));
        if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
            list.insert(0, "jp".into());
        }
        if let Some(list) = fonts.families.get_mut(&FontFamily::Monospace) {
            list.push("jp".into());
        }
        if let Ok(bytes) = std::fs::read(b) {
            fonts.font_data.insert("jp_bold".into(), Arc::new(egui::FontData::from_owned(bytes)));
            bold_list.push("jp_bold".into());
        }
        bold_list.push("jp".into());
        break;
    }
    let mut fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    bold_list.append(&mut fallback);
    bold_list.dedup();
    fonts.families.insert(FontFamily::Name("bold".into()), bold_list);
    ctx.set_fonts(fonts);
}
