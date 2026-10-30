//! 状態の共有と、出力を計算する専用スレッド(画面が隠れていても40回/秒で動く)。

use crate::apc::LedOut;
use crate::dmx::Frame;
use crate::engine::{Look, Setup, State};
use crate::fixture;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub struct Shared {
    pub state: Mutex<State>,
    pub frame: Frame,
    pub leds: Mutex<LedOut>,
    start: Instant,
}

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Shared {
    pub fn new(state: State) -> Arc<Self> {
        Arc::new(Shared { state: Mutex::new(state), frame: Arc::new(Mutex::new([0; 512])), leds: Mutex::new(LedOut::default()), start: Instant::now() })
    }

    /// 起動からの秒数
    pub fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    pub fn spawn_engine(self: &Arc<Self>) {
        let me = self.clone();
        let _ = std::thread::Builder::new().name("engine".into()).spawn(move || {
            const PERIOD: Duration = Duration::from_millis(25);
            loop {
                let t0 = Instant::now();
                let mut universe = [0u8; 512];
                {
                    let mut s = lock(&me.state);
                    s.tick(me.now());
                    fixture::write_universe(&s.heads, s.setup.mode, &s.setup.addresses, &mut universe);
                    lock(&me.leds).sync(&s);
                }
                *lock(&me.frame) = universe;
                if let Some(rest) = PERIOD.checked_sub(t0.elapsed()) {
                    std::thread::sleep(rest);
                }
            }
        });
    }
}

/// 保存する設定
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub setup: Setup,
    pub look: Look,
    pub bpm: f64,
    /// USB-DMXのポート(None = 出力しない)
    pub dmx_port: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config { setup: Setup::default(), look: Look::default(), bpm: 128.0, dmx_port: None }
    }
}

fn config_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("SPOT4_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("spot4")
    } else {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join("Library/Application Support/spot4")
    }
}

impl Config {
    pub fn load() -> Config {
        std::fs::read_to_string(config_dir().join("settings.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        // 書きかけで落ちても壊れないよう、一時ファイルに書いてから置き換える
        let tmp = dir.join("settings.json.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dir.join("settings.json")).map_err(|e| e.to_string())
    }
}
