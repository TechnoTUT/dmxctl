//! dmxctl の本体。層は3つのクレートで、参照してよい向きは `ui` → `backend` → `core` だけ。
//!
//! - [`core`]: 設定の形・値の合成・機器とプロトコルの入出力。クレート `dmxctl-core`
//! - [`backend`]: 画面なしの起動・保存・遠隔操作・CLI。クレート `dmxctl-backend`
//! - [`ui`]: 画面。クレート `dmxctl-ui`
//!
//! このクレートは、起動の `main.rs` と、結合テストが使う再公開だけを持つ。
pub use dmxctl_backend as backend;
pub use dmxctl_core as core;
pub use dmxctl_ui as ui;
