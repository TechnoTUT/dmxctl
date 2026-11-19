//! 40Hzの出力スレッド。フレームを合成して各出力へ送る。
use super::*;

/// 出力スレッド。40Hzでフレームを合成して各出力へ送る
pub struct Engine {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Engine {
    pub fn spawn(core: Shared) -> Engine {
        Self::spawn_with_output(core, true)
    }

    /// Keep rendering at 40 Hz even when physical output is disabled.
    pub fn spawn_with_output(core: Shared, output_enabled: bool) -> Engine {
        lock(&core).rt.physical_output_enabled=output_enabled;
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        let handle = std::thread::Builder::new()
            .name("dmx-engine".into())
            .spawn(move || run(core, s2, output_enabled))
            .expect("engine thread");
        Engine { stop, handle: Some(handle) }
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) { self.stop(); }
}

pub(super) fn run(core: Shared, stop: Arc<AtomicBool>, output_enabled: bool) {
    struct Running;
    impl Drop for Running {
        fn drop(&mut self) {
            ENGINE_RUNNING.store(false, Ordering::SeqCst);
        }
    }
    ENGINE_RUNNING.store(true, Ordering::SeqCst);
    let _running = Running;
    let mut cfgs: Vec<OutputCfg> = Vec::new();
    let mut outs: Vec<Box<dyn Output>> = Vec::new();
    let mut gen_seen = u64::MAX;
    let mut next = Instant::now();
    // 異常時に送るフレーム(毎回、最新のものを作っておく)
    let mut safe: Option<(Vec<[u8; 512]>, Vec<usize>)> = None;
    let mut safe_sent = 0u32;

    while !stop.load(Ordering::SeqCst) {
        if EMERGENCY.load(Ordering::SeqCst) {
            // 安全停止中: Coreを待たずに(画面のスレッドが止まっていても)安全なフレームを送り続ける
            if let Some((frames, lens)) = &safe {
                send_frames(&cfgs, &mut outs, frames, lens);
                // Safety output uses a separate path from render: record it too.
                if let Ok(mut c) = core.try_lock() {
                    c.rt.frames = frames.clone();
                    c.rt.frame_id += 1;
                    c.rt.safety_events.clear();
                    if let Some(recorder) = c.rt.recorder.clone() {
                        recorder.frame(&c, true);
                    }
                }
            }
            safe_sent += 1;
            if safe_sent >= SAFE_TICKS && EXIT_AFTER_SAFE.load(Ordering::SeqCst) {
                std::process::exit(130);
            }
            wait_tick(&mut next);
            continue;
        }
        // 1回分の処理でpanicしても、出力を止めずに安全停止へ切り替える(最後のフレームが出っぱなしにならないように)
        let tick = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let Some(mut c) = lock_unless_emergency(&core) else { return };
            c.render();
            let (frames, lens, cfg_now, gen_now) = (c.rt.frames.clone(), c.rt.used_len.clone(), c.project.outputs.clone(), c.rt.output_gen);
            safe = Some((c.safe_frames(), lens.clone()));
            drop(c);
            if output_enabled && gen_now != gen_seen {
                // 先に古い出力を閉じてからポートを開き直す
                outs.clear();
                outs = cfg_now.iter().map(output::create).collect();
                cfgs = cfg_now;
                gen_seen = gen_now;
            }
            send_frames(&cfgs, &mut outs, &frames, &lens);
            let statuses: Vec<String> = outs.iter().map(|o| o.status()).collect();
            lock(&core).rt.output_status = statuses;
        }));
        if tick.is_err() {
            emergency_stop(false);
            continue;
        }
        wait_tick(&mut next);
    }
}

/// Coreを借りる。待っている間に安全停止になったら諦める(panicした画面のスレッドがロックを持ったままでも止まらないように)
pub(super) fn lock_unless_emergency(core: &Shared) -> Option<MutexGuard<'_, Core>> {
    loop {
        match core.try_lock() {
            Ok(g) => return Some(g),
            Err(std::sync::TryLockError::Poisoned(e)) => return Some(e.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => {
                if EMERGENCY.load(Ordering::SeqCst) {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}

pub(super) fn send_frames(cfgs: &[OutputCfg], outs: &mut [Box<dyn Output>], frames: &[[u8; 512]], lens: &[usize]) {
    for (cfg, o) in cfgs.iter().zip(outs.iter_mut()) {
        if !cfg.enabled {
            continue;
        }
        if let (Some(f), Some(&len)) = (frames.get(cfg.universe), lens.get(cfg.universe)) {
            o.send(f, len);
        }
    }
}

pub(super) fn wait_tick(next: &mut Instant) {
    *next += TICK;
    let now = Instant::now();
    if *next > now {
        std::thread::sleep(*next - now);
    } else {
        *next = now;
    }
}
