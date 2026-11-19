//! 音声入力を `Core` につなぐ。入力とテンポの検出は `dmxctl-io` の `audio`。
//! 状態の文言は `rt.audio_status` に、テンポの推定は `Core::audio_report` に渡す。

use crate::engine::{lock, Shared};
use dmxctl_io::audio::{AudioCapture, AudioHost};
use std::sync::Arc;
use std::time::Instant;

pub use dmxctl_io::audio::{input_devices, Analyzer, Estimate, DETECT_MAX, DETECT_MIN};

struct CoreHost(Shared);

impl AudioHost for CoreHost {
    fn set_status(&self, status: String) {
        lock(&self.0).rt.audio_status = status;
    }

    fn report(&self, estimate: Estimate, now: Instant) {
        lock(&self.0).audio_report(estimate, now);
    }
}

/// 音声入力。Dropで止まる
pub struct AudioService {
    _capture: AudioCapture,
    pub device: String,
}

impl AudioService {
    /// `device`が空なら既定の入力
    pub fn start(core: Shared, device: &str) -> AudioService {
        AudioService { _capture: AudioCapture::start(Arc::new(CoreHost(core)), device), device: device.to_string() }
    }
}
