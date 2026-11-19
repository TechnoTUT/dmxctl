//! 画面や待機中のスレッドを起こす口。`core` と `backend` は画面の部品を知らずに、再描画や終了を頼める。
//! 画面を持たないときは `Wake::none()` で、何もしない。
use std::sync::Arc;
use std::time::Duration;

type Repaint = dyn Fn(Option<Duration>) + Send + Sync;

#[derive(Clone)]
pub struct Wake {
    repaint: Arc<Repaint>,
    close: Arc<dyn Fn() + Send + Sync>,
}

impl Wake {
    /// `repaint` は、すぐに描き直すとき `None`、少し後に描き直すとき `Some(待つ時間)` で呼ばれる。`close` は画面を閉じる
    pub fn new(repaint: impl Fn(Option<Duration>) + Send + Sync + 'static, close: impl Fn() + Send + Sync + 'static) -> Self {
        Wake { repaint: Arc::new(repaint), close: Arc::new(close) }
    }

    /// 画面が無いときの、何もしない口
    pub fn none() -> Self {
        Wake::new(|_| {}, || {})
    }

    pub fn request_repaint(&self) {
        (self.repaint)(None);
    }

    pub fn request_repaint_after(&self, after: Duration) {
        (self.repaint)(Some(after));
    }

    pub fn request_close(&self) {
        (self.close)();
    }
}

impl Default for Wake {
    fn default() -> Self {
        Wake::none()
    }
}
