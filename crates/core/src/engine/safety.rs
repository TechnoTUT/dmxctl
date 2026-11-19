//! 異常時・終了のシグナル・シャットダウンで、安全なフレームを送ってから止まる。
use super::*;

/// 異常時(画面や出力のスレッドがpanicした・終了のシグナルを受けた)に立てる。
/// 立つと出力スレッドは、以後は安全なフレームだけを送る(戻らない。アプリの再起動が必要)
pub static EMERGENCY: AtomicBool = AtomicBool::new(false);
/// 安全なフレームを送り終えたら、プロセスを終了する(終了のシグナルを受けたとき)
pub(super) static EXIT_AFTER_SAFE: AtomicBool = AtomicBool::new(false);
/// 出力スレッドが動いている間だけ立つ(終了時の安全停止を、出力スレッドが送れるときだけ待つため)
pub(super) static ENGINE_RUNNING: AtomicBool = AtomicBool::new(false);
/// 安全なフレームを送る回数(40Hzで約0.3秒)
pub(super) const SAFE_TICKS: u32 = 12;

/// 安全停止に入る。`exit`=安全なフレームを送り終えたら終了する
pub fn emergency_stop(exit: bool) {
    if exit {
        EXIT_AFTER_SAFE.store(true, Ordering::SeqCst);
    }
    EMERGENCY.store(true, Ordering::SeqCst);
}

/// 画面のスレッドのpanicで安全停止する(出力スレッドが安全なフレームを送る時間を待ってから終了へ進む)。
/// Ctrl+C・終了のシグナルでも、安全なフレームを送ってから終了する
/// install_safety_hooks を呼んだか(終了のシグナルを受けるのは、実際のアプリのプロセスだけ)
pub static SAFETY_HOOKS: AtomicBool = AtomicBool::new(false);

pub fn install_safety_hooks() {
    SAFETY_HOOKS.store(true, Ordering::SeqCst);
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        // 画面(main)が落ちるとアプリが終わる。出力スレッドの中のpanicは、出力スレッド自身が受け止める
        if std::thread::current().name() == Some("main") {
            emergency_stop(false);
            std::thread::sleep(TICK * (SAFE_TICKS + 2));
        }
    }));
    #[cfg(unix)]
    unsafe {
        extern "C" fn on_signal(_: libc::c_int) {
            // シグナルの中ではフラグを立てるだけ(終了は出力スレッドが安全なフレームを送ってから行う)。
            // 2回目(出力スレッドが動いていないときなど)は、すぐに終了する
            if EMERGENCY.load(Ordering::SeqCst) {
                unsafe { libc::_exit(130) };
            }
            emergency_stop(true);
        }
        let h = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGINT, h);
        libc::signal(libc::SIGTERM, h);
        libc::signal(libc::SIGHUP, h);
        // 画面を通らずにプロセスが終わるとき(Linuxでログアウト時に画面のサーバーが先に落ちて、
        // ウィンドウのライブラリが exit() するなど)も、出力スレッドが動いていれば安全なフレームを送ってから終わる
        extern "C" fn on_exit() {
            if ENGINE_RUNNING.load(Ordering::SeqCst) && !EMERGENCY.load(Ordering::SeqCst) {
                emergency_stop(false);
                std::thread::sleep(TICK * (SAFE_TICKS + 2));
            }
        }
        libc::atexit(on_exit);
    }
    // Windows: Ctrl+C / Ctrl+Break / コンソールを閉じた / ログオフ / シャットダウン。
    // コンソールがあるとき(開発用のデバッグビルド)だけ届く。リリース版はウィンドウだけのアプリ
    // (main.rs の windows_subsystem)でコンソールが無いので、シャットダウン・ログオフは下の watch_windows_shutdown で受ける
    #[cfg(windows)]
    unsafe {
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn SetConsoleCtrlHandler(handler: Option<unsafe extern "system" fn(u32) -> i32>, add: i32) -> i32;
            fn ExitProcess(code: u32) -> !;
        }
        unsafe extern "system" fn on_ctrl(kind: u32) -> i32 {
            // 2回目のCtrl+C / Ctrl+Break(出力スレッドが動いていないときなど)は、すぐに終了する。
            // シャットダウン等(2以上)は下の watch_windows_shutdown が先に安全停止を始めていることがあるので、待つほうにする
            if kind <= 1 && EMERGENCY.load(Ordering::SeqCst) {
                unsafe { ExitProcess(130) };
            }
            emergency_stop(true);
            // この関数から戻ると、コンソールを閉じたとき等はWindowsがプロセスを終わらせてしまうので、
            // 出力スレッドが安全なフレームを送り終えて終了するまで待つ
            std::thread::sleep(TICK * (SAFE_TICKS + 8));
            1
        }
        SetConsoleCtrlHandler(Some(on_ctrl), 1);
    }
    #[cfg(windows)]
    watch_windows_shutdown();
}

/// Windowsのシャットダウン・ログオフで安全停止する。
/// ウィンドウだけのアプリには、シャットダウンは WM_QUERYENDSESSION → WM_ENDSESSION として届く(on_exit は呼ばれない)。
/// 画面(egui)のウィンドウとは別に、見えないウィンドウを専用のスレッドで作って受ける
/// (画面のスレッドが止まっていても受けられるように)
#[cfg(windows)]
pub(super) fn watch_windows_shutdown() {
    use std::ffi::c_void;
    use std::ptr::null_mut;
    type Hwnd = *mut c_void;
    #[repr(C)]
    struct WndClassW {
        style: u32,
        wnd_proc: unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize,
        cls_extra: i32,
        wnd_extra: i32,
        instance: *mut c_void,
        icon: *mut c_void,
        cursor: *mut c_void,
        background: *mut c_void,
        menu_name: *const u16,
        class_name: *const u16,
    }
    #[repr(C)]
    struct Msg {
        hwnd: Hwnd,
        message: u32,
        wparam: usize,
        lparam: isize,
        time: u32,
        pt: [i32; 2],
        private: u32,
    }
    #[link(name = "user32")]
    unsafe extern "system" {
        fn RegisterClassW(class: *const WndClassW) -> u16;
        fn CreateWindowExW(ex_style: u32, class: *const u16, name: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: Hwnd, menu: *mut c_void, instance: *mut c_void, param: *mut c_void) -> Hwnd;
        fn DefWindowProcW(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize;
        fn GetMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32) -> i32;
        fn TranslateMessage(msg: *const Msg) -> i32;
        fn DispatchMessageW(msg: *const Msg) -> isize;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
    }
    const WM_ENDSESSION: u32 = 0x0016;

    unsafe extern "system" fn wnd_proc(hwnd: Hwnd, msg: u32, wparam: usize, lparam: isize) -> isize {
        // wparam != 0 = 本当に終わる(キャンセルされたときは0)。
        // ここから戻るとWindowsはいつでもプロセスを終わらせるので、出力スレッドが安全なフレームを送り終えて終了するまで待つ
        if msg == WM_ENDSESSION && wparam != 0 {
            emergency_stop(true);
            std::thread::sleep(TICK * (SAFE_TICKS + 8));
            return 0;
        }
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    let spawned = std::thread::Builder::new().name("shutdown-watch".into()).spawn(|| {
        let class: Vec<u16> = "dmxctl-shutdown-watch\0".encode_utf16().collect();
        // SAFETY: Win32の公開APIを、宣言どおりの型で呼ぶ。ウィンドウはこのスレッドで作り、このスレッドでメッセージを処理する
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            let wc = WndClassW {
                style: 0,
                wnd_proc,
                cls_extra: 0,
                wnd_extra: 0,
                instance,
                icon: null_mut(),
                cursor: null_mut(),
                background: null_mut(),
                menu_name: std::ptr::null(),
                class_name: class.as_ptr(),
            };
            if RegisterClassW(&wc) == 0 {
                return;
            }
            // 親なしのウィンドウ(表示はしない)。メッセージ専用ウィンドウ(HWND_MESSAGE)にはシャットダウンが届かないので使わない
            let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, null_mut(), null_mut(), instance, null_mut());
            if hwnd.is_null() {
                return;
            }
            let mut msg: Msg = std::mem::zeroed();
            while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    });
    if let Err(e) = spawned {
        eprintln!("シャットダウンの監視を始められません: {e}");
    }
}
