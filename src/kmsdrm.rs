//! デスクトップ環境の無いLinuxで、画面に直接(KMS/DRM)出す。
//! eframe(winit)は X11 / Wayland が要るので、こちらは SDL2 の KMSDRM で全画面を作って入力を受け、
//! egui を OpenGL ES(egui_glow)で描く。アプリ本体(`App`)は eframe のときと同じものを使う

mod keys;
mod touch;

use crate::app::App;
use eframe::egui::{self, pos2, vec2, Event, MouseWheelUnit, OutputCommand, PointerButton, Pos2, Rect, TouchPhase, ViewportCommand, ViewportId};
use sdl2::event::{Event as SdlEvent, WindowEvent};
use sdl2::mouse::{MouseButton, MouseWheelDirection};
use sdl2::video::{GLProfile, SwapInterval};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// 別スレッド(MIDI・OSCなど)から再描画を頼まれたときに、入力待ちのループを起こすための合図
struct Wake;

/// KMSDRMで起動するか。`--kmsdrm` か `DMXCTL_KMSDRM=1` で必ず、`DMXCTL_KMSDRM=0` で使わない。
/// どちらでもなければ、デスクトップ環境が無いとき(DISPLAY も WAYLAND_DISPLAY も無い)に使う
pub fn wanted() -> bool {
    if std::env::args().any(|a| a == "--kmsdrm") {
        return true;
    }
    match std::env::var("DMXCTL_KMSDRM").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none(),
    }
}

/// 画面の拡大率。`DMXCTL_SCALE=1.5` などで指定、無ければ画面の大きさから決める
/// (大きい画面: ウィンドウ版の既定の大きさ 1280×800 が収まる倍率を0.25刻みで。
/// 小さい画面: 一番小さいウィンドウ 960×620 が収まるなら等倍、収まらなければ縮める)
fn scale(w: u32, h: u32) -> f32 {
    if let Some(s) = std::env::var("DMXCTL_SCALE").ok().and_then(|v| v.parse::<f32>().ok()).filter(|s| s.is_finite()) {
        return s.clamp(0.5, 4.0);
    }
    let (w, h) = (w as f32, h as f32);
    let fit = (w / 1280.0).min(h / 800.0);
    if fit >= 1.0 {
        return ((fit * 4.0).floor() / 4.0).min(4.0);
    }
    let fit_min = (w / 960.0).min(h / 620.0);
    ((fit_min * 20.0).floor() / 20.0).clamp(0.5, 1.0)
}

pub fn run() -> Result<(), String> {
    // 終了のシグナル(Ctrl+C など)は engine::install_safety_hooks で受ける(SDLには取らせない)
    sdl2::hint::set("SDL_NO_SIGNAL_HANDLERS", "1");
    // 指の入力は touch.rs でマウスの操作に直すので、SDLにはマウスの操作を作らせない(タッチパネルで1回触ると2回押したことになる)
    sdl2::hint::set("SDL_TOUCH_MOUSE_EVENTS", "0");
    if std::env::var_os("SDL_VIDEODRIVER").is_none() {
        sdl2::hint::set("SDL_VIDEODRIVER", "kmsdrm");
    }
    let sdl = sdl2::init()?;
    let video = sdl.video()?;
    let events = sdl.event()?;
    events.register_custom_event::<Wake>()?;

    let gl_attr = video.gl_attr();
    gl_attr.set_context_profile(GLProfile::GLES);
    gl_attr.set_context_version(3, 0);
    gl_attr.set_double_buffer(true);
    let (w, h) = video.current_display_mode(0).map(|m| (m.w.max(1) as u32, m.h.max(1) as u32)).unwrap_or((1280, 800));
    let window = video.window("DMX Controller", w, h).opengl().fullscreen_desktop().build().map_err(|e| {
        format!("画面を開けませんでした({e})。画面の装置(/dev/dri)と入力の装置(/dev/input)を使える権限があるか(video・input グループ)、他のアプリが画面を使っていないかを確かめてください")
    })?;
    // OpenGL ES 3.0 が無い古いGPU(Raspberry Pi 3 など)では 2.0 で
    let gl_ctx = match window.gl_create_context() {
        Ok(c) => c,
        Err(_) => {
            gl_attr.set_context_version(2, 0);
            window.gl_create_context()?
        }
    };
    window.gl_make_current(&gl_ctx)?;
    let _ = video.gl_set_swap_interval(SwapInterval::VSync);
    let gl = unsafe { glow::Context::from_loader_function(|s| video.gl_get_proc_address(s) as *const _) };
    // 診断(DMXCTL_PERF=1): 使っているGPU(llvmpipe / softpipe ならソフトウェア描画で遅い)と、1秒ごとのフレームの内訳
    let perf = std::env::var_os("DMXCTL_PERF").is_some();
    if perf {
        use glow::HasContext;
        let (renderer, version) = unsafe { (gl.get_parameter_string(glow::RENDERER), gl.get_parameter_string(glow::VERSION)) };
        eprintln!("GPU: {renderer} / {version} / ドライバ: {}", video.current_video_driver());
    }
    let mut stats = FrameStats::default();
    let mut painter = egui_glow::Painter::new(Arc::new(gl), "", None, true).map_err(|e| format!("描画の準備ができませんでした: {e}"))?;

    let ctx = egui::Context::default();
    // 次に描く時刻。別スレッドからの依頼は、すぐ描くなら合図を送ってループを起こす
    let wake_at: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
    {
        let wake_at = wake_at.clone();
        let sender = events.event_sender();
        ctx.set_request_repaint_callback(move |info| {
            // 今の予定より早く描くことになったら、待っているループを起こして待ち時間を決め直させる
            let earlier = Instant::now().checked_add(info.delay).is_some_and(|at| request_at(&wake_at, at));
            if earlier || info.delay.is_zero() {
                let _ = sender.push_custom_event(Wake);
            }
        });
    }
    // ここから先で失敗して戻ると app.shutdown()(ブラックアウト・保存)を通らないので、失敗しうるものは先に用意する
    let mut pump = sdl.event_pump()?;
    let mut app = App::new(&ctx, true);
    if !can_read_input() {
        let msg = "キーボード・マウスを読めません(/dev/input の権限が無い)。`sudo usermod -aG input $USER` のあとログインし直してください。終了は Ctrl+C(起動した端末で)";
        eprintln!("{msg}");
        app.notice = msg.into();
    }

    video.text_input().start();
    let start = Instant::now();
    // KMSDRMでは画面の大きさは変わらないので、拡大率は最初に一度だけ決める
    let (pw, ph) = window.drawable_size();
    let mut input = Input { mods: egui::Modifiers::default(), focused: true, ppp: scale(pw, ph), quit: false, touch: touch::Touch::new(pw as f32, ph as f32), warp: None };
    let mouse = sdl.mouse();
    // 次のフレームに渡すもの(撮った画面など)
    let mut pending: Vec<Event> = Vec::new();
    let mut cpu: Option<f32> = None;
    request_at(&wake_at, Instant::now());
    while !input.quit {
        // 次に描く時刻まで待つ(入力があればすぐ起きる)。
        // 起こす合図(Wake)は描く時刻が早まったという知らせなので、時刻を見直して、まだなら待ち続ける
        let first = loop {
            let next = *wake_at.lock().unwrap_or_else(|e| e.into_inner());
            let ev = match next.map(|t| t.saturating_duration_since(Instant::now())) {
                Some(d) if d.is_zero() => None,
                Some(d) => pump.wait_event_timeout(d.as_millis().clamp(1, u32::MAX as u128) as u32),
                None => Some(pump.wait_event()),
            };
            match ev {
                Some(e) if e.is_user_event() => continue,
                other => break other,
            }
        };
        let (pw, ph) = window.drawable_size();
        let mut raw = egui::RawInput { events: std::mem::take(&mut pending), ..Default::default() };
        for ev in first.into_iter().chain(pump.poll_iter()) {
            input.translate(ev, &mut raw.events, &video);
        }
        // 指の入力で動かしたカーソルを、SDLのカーソルにも反映する(押し込むクリックがその位置になるように)
        if let Some((x, y)) = input.warp.take() {
            mouse.warp_mouse_in_window(&window, x, y);
        }
        if input.quit {
            break;
        }

        let size = vec2(pw as f32, ph as f32) / input.ppp;
        raw.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, size));
        let vp = raw.viewports.entry(ViewportId::ROOT).or_default();
        vp.native_pixels_per_point = Some(input.ppp);
        vp.inner_rect = Some(Rect::from_min_size(Pos2::ZERO, size));
        vp.monitor_size = Some(size);
        vp.focused = Some(input.focused);
        vp.fullscreen = Some(true);
        raw.focused = input.focused;
        raw.time = Some(start.elapsed().as_secs_f64());
        raw.max_texture_side = Some(painter.max_texture_side());

        *wake_at.lock().unwrap_or_else(|e| e.into_inner()) = None;
        let t = Instant::now();
        let out = ctx.run_ui(raw, |ui| app.frame(ui, cpu));
        let t_ui = t.elapsed();
        for cmd in out.platform_output.commands {
            if let OutputCommand::CopyText(s) = cmd {
                let _ = video.clipboard().set_clipboard_text(&s);
            }
        }
        let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
        let mut textures = out.textures_delta;
        painter.clear([pw, ph], ctx.global_style().visuals.panel_fill.to_normalized_gamma_f32());
        painter.paint_and_update_textures([pw, ph], out.pixels_per_point, &prims, &mut textures);
        cpu = Some(t.elapsed().as_secs_f32());
        let t_paint = t.elapsed();

        if let Some(vo) = out.viewport_output.get(&ViewportId::ROOT) {
            for c in &vo.commands {
                match c {
                    ViewportCommand::Close => input.quit = true,
                    ViewportCommand::Screenshot(user_data) => {
                        let image = Arc::new(painter.read_screen_rgba([pw, ph]));
                        pending.push(Event::Screenshot { viewport_id: ViewportId::ROOT, user_data: user_data.clone(), image });
                        request_at(&wake_at, Instant::now());
                    }
                    _ => {}
                }
            }
            if let Some(at) = Instant::now().checked_add(vo.repaint_delay) {
                request_at(&wake_at, at);
            }
        }
        window.gl_swap_window();
        if perf {
            stats.add(t_ui, t_paint - t_ui, t.elapsed() - t_paint);
        }
    }
    app.shutdown();
    painter.destroy();
    Ok(())
}

/// 入力の装置(/dev/input/event*)を1つでも開けるか。開けないとSDLはキー・マウスを受け取れない
fn can_read_input() -> bool {
    let Ok(rd) = std::fs::read_dir("/dev/input") else { return false };
    rd.flatten()
        .filter(|e| e.file_name().to_str().is_some_and(|n| n.starts_with("event")))
        .any(|e| std::fs::File::open(e.path()).is_ok())
}

/// 指の入力の装置が、タッチパネルかトラックパッドか
fn touch_kind(touch_id: i64) -> touch::Kind {
    use sdl2::sys::SDL_TouchDeviceType as T;
    match unsafe { sdl2::sys::SDL_GetTouchDeviceType(touch_id) } {
        T::SDL_TOUCH_DEVICE_DIRECT => touch::Kind::Screen,
        _ => touch::Kind::Pad,
    }
}

/// 診断用: 1秒ごとに、描いた回数と1回あたりの時間(UI・描画・画面の切り替え待ち)を出す
#[derive(Default)]
struct FrameStats {
    since: Option<Instant>,
    frames: u32,
    ui: std::time::Duration,
    paint: std::time::Duration,
    swap: std::time::Duration,
    /// 一番長くかかったフレーム・一番長く空いたフレームの間隔(カクつきの手がかり)
    longest: std::time::Duration,
    gap: std::time::Duration,
    last: Option<Instant>,
}

impl FrameStats {
    fn add(&mut self, ui: std::time::Duration, paint: std::time::Duration, swap: std::time::Duration) {
        let since = *self.since.get_or_insert_with(Instant::now);
        let now = Instant::now();
        if let Some(last) = self.last {
            self.gap = self.gap.max(now - last);
        }
        self.last = Some(now);
        self.longest = self.longest.max(ui + paint + swap);
        self.frames += 1;
        self.ui += ui;
        self.paint += paint;
        self.swap += swap;
        if since.elapsed().as_secs_f32() >= 1.0 {
            let n = self.frames as f32;
            let ms = |d: std::time::Duration| d.as_secs_f32() * 1000.0 / n;
            eprintln!(
                "{} fps  UI {:.1}ms  描画 {:.1}ms  切り替え {:.1}ms  最長 {:.0}ms  最大の間隔 {:.0}ms",
                self.frames,
                ms(self.ui),
                ms(self.paint),
                ms(self.swap),
                self.longest.as_secs_f32() * 1000.0,
                self.gap.as_secs_f32() * 1000.0
            );
            *self = FrameStats { last: self.last, ..FrameStats::default() };
        }
    }
}

/// 次に描く時刻を、より早いほうに。早めたらtrue
fn request_at(wake_at: &Mutex<Option<Instant>>, at: Instant) -> bool {
    let mut w = wake_at.lock().unwrap_or_else(|e| e.into_inner());
    let earlier = w.is_none_or(|t| at < t);
    if earlier {
        *w = Some(at);
    }
    earlier
}

/// SDLの入力を egui の入力に直すときの状態
struct Input {
    mods: egui::Modifiers,
    focused: bool,
    /// 画面の拡大率(SDLの座標はピクセル、eguiは論理座標)
    ppp: f32,
    quit: bool,
    /// 指の入力(タッチパネル・トラックパッド)をマウスの操作に直す
    touch: touch::Touch,
    /// SDLのカーソルを動かす先(ピクセル)
    warp: Option<(i32, i32)>,
}

impl Input {
    fn pos(&self, x: i32, y: i32) -> Pos2 {
        pos2(x as f32 / self.ppp, y as f32 / self.ppp)
    }

    fn apply_touch(&mut self, acts: Vec<touch::Action>, out: &mut Vec<Event>) {
        for a in acts {
            match a {
                touch::Action::MoveTo(x, y) => {
                    out.push(Event::PointerMoved(pos2(x / self.ppp, y / self.ppp)));
                    self.warp = Some((x as i32, y as i32));
                }
                touch::Action::Button { pressed, secondary } => {
                    let (x, y) = self.touch.cursor();
                    let button = if secondary { PointerButton::Secondary } else { PointerButton::Primary };
                    out.push(Event::PointerButton { pos: pos2(x / self.ppp, y / self.ppp), button, pressed, modifiers: self.mods });
                }
                touch::Action::Scroll(x, y) => {
                    out.push(Event::MouseWheel { unit: MouseWheelUnit::Point, delta: vec2(x, y) / self.ppp, phase: TouchPhase::Move, modifiers: self.mods });
                }
            }
        }
    }

    fn translate(&mut self, ev: SdlEvent, out: &mut Vec<Event>, video: &sdl2::VideoSubsystem) {
        match ev {
            SdlEvent::Quit { .. } => self.quit = true,
            SdlEvent::Window { win_event: WindowEvent::FocusGained, .. } => self.focused = true,
            SdlEvent::Window { win_event: WindowEvent::FocusLost, .. } => self.focused = false,
            SdlEvent::MouseMotion { x, y, .. } => {
                self.touch.set_cursor(x as f32, y as f32);
                out.push(Event::PointerMoved(self.pos(x, y)));
            }
            SdlEvent::FingerDown { touch_id, finger_id, x, y, .. } => {
                let acts = self.touch.down(touch_kind(touch_id), (touch_id, finger_id), x, y, Instant::now());
                self.apply_touch(acts, out);
            }
            SdlEvent::FingerMotion { touch_id, finger_id, x, y, dx, dy, .. } => {
                let acts = self.touch.motion(touch_kind(touch_id), (touch_id, finger_id), x, y, dx, dy);
                self.apply_touch(acts, out);
            }
            SdlEvent::FingerUp { touch_id, finger_id, x, y, .. } => {
                let acts = self.touch.up(touch_kind(touch_id), (touch_id, finger_id), x, y, Instant::now());
                self.apply_touch(acts, out);
            }
            SdlEvent::MouseButtonDown { mouse_btn, x, y, .. } | SdlEvent::MouseButtonUp { mouse_btn, x, y, .. } => {
                let pressed = matches!(ev, SdlEvent::MouseButtonDown { .. });
                let button = match mouse_btn {
                    MouseButton::Left => PointerButton::Primary,
                    MouseButton::Right => PointerButton::Secondary,
                    MouseButton::Middle => PointerButton::Middle,
                    MouseButton::X1 => PointerButton::Extra1,
                    MouseButton::X2 => PointerButton::Extra2,
                    MouseButton::Unknown => return,
                };
                out.push(Event::PointerButton { pos: self.pos(x, y), button, pressed, modifiers: self.mods });
            }
            SdlEvent::MouseWheel { precise_x, precise_y, direction, .. } => {
                // SDLは「右・上へ回した」が正、eguiは中身が動く向き(左・下へ動く)が正
                let flip = if direction == MouseWheelDirection::Flipped { -1.0 } else { 1.0 };
                let delta = vec2(-precise_x, precise_y) * flip;
                out.push(Event::MouseWheel { unit: MouseWheelUnit::Line, delta, phase: TouchPhase::Move, modifiers: self.mods });
            }
            SdlEvent::KeyDown { keycode, scancode, keymod, repeat, .. } | SdlEvent::KeyUp { keycode, scancode, keymod, repeat, .. } => {
                let pressed = matches!(ev, SdlEvent::KeyDown { .. });
                let mods = keys::modifiers(keymod);
                // eguiの ui.input(|i| i.modifiers) は ModifiersChanged でしか変わらない(Shiftを押している間の表示などに使う)
                if mods != self.mods {
                    out.push(Event::ModifiersChanged(mods));
                }
                self.mods = mods;
                let Some(key) = keys::logical(keycode, scancode) else { return };
                if pressed && self.mods.command {
                    match key {
                        egui::Key::C => out.push(Event::Copy),
                        egui::Key::X => out.push(Event::Cut),
                        egui::Key::V => {
                            if let Ok(text) = video.clipboard().clipboard_text() {
                                out.push(Event::Paste(text));
                            }
                        }
                        _ => {}
                    }
                }
                out.push(Event::Key { key, physical_key: scancode.and_then(keys::physical), pressed, repeat, modifiers: self.mods });
            }
            // Ctrl を押しながらの文字は、ショートカットなので文字として入れない
            SdlEvent::TextInput { text, .. } if !self.mods.ctrl && !text.chars().all(char::is_control) => out.push(Event::Text(text)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_fits_the_default_window_size() {
        // DMXCTL_SCALE を指定していないとき
        if std::env::var_os("DMXCTL_SCALE").is_some() {
            return;
        }
        assert_eq!(scale(1280, 800), 1.0);
        assert_eq!(scale(1920, 1080), 1.25);
        assert_eq!(scale(3840, 2160), 2.5);
        assert_eq!(scale(1366, 768), 1.0);
        assert_eq!(scale(1024, 600), 0.95);
        assert_eq!(scale(800, 480), 0.75);
    }
}
