//! NDI受信を `Core` につなぐ。受信そのものは `dmxctl-io` の `ndi`。
//! 設定は `Core` のプロジェクトから読み、結果は `rt.ndi` に書く。照明への反映はエンジンが行う。

use crate::engine::{lock, Shared};
use crate::wake::Wake;
use dmxctl_io::ndi::{NdiHost, NdiService as Receiver};
use std::sync::Arc;
use std::time::Duration;

pub use dmxctl_io::ndi::{LiveSource, NdiCfg, NdiState};

struct CoreHost {
    core: Shared,
    ctx: Option<Wake>,
}

impl NdiHost for CoreHost {
    fn config(&self) -> NdiCfg {
        lock(&self.core).project.ndi.clone()
    }

    fn update(&self, f: &mut dyn FnMut(&mut NdiState)) {
        f(&mut lock(&self.core).rt.ndi);
    }

    fn notify(&self) {
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
    }
}

/// NDIの受信。Dropで止まる
pub struct NdiService {
    _receiver: Receiver,
}

impl NdiService {
    pub fn start(core: Shared, ctx: Option<Wake>) -> NdiService {
        NdiService { _receiver: Receiver::start(Arc::new(CoreHost { core, ctx })) }
    }
}
