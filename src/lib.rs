//! dmxctl の本体。層は3つで、参照してよい向きは `ui` → `backend` → `core` だけ。
//!
//! - [`core`]: 設定の形・値の合成・機器とプロトコルの入出力。別のクレート `dmxctl-core`
//! - [`backend`]: 画面なしの起動・保存・遠隔操作・CLI
//! - [`ui`]: 画面
pub mod backend;
pub mod ui;

pub use dmxctl_core as core;
