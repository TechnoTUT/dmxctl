//! 画面層。eframe と KMS/DRM の画面、タブごとの描画、見た目。
//!
//! 起動・保存・遠隔操作は `backend::host::Host` が受け持つ。画面は `Host` を1つ持ち、その `core` を読んで描画する。
//! 実行中の状態を変更するときは `core::live::LiveOp` にして `Core::live` へ渡す。
pub mod app;
#[cfg(target_os = "linux")]
pub mod kmsdrm;
pub mod shot;
pub mod theme;
pub mod auto;
pub mod binding;
pub mod control;
pub mod keys;
pub mod laser;
pub mod map;
pub mod midi;
pub mod output;
pub mod pair;
pub mod patch;
pub mod profile;
pub mod remote;
pub mod sequence;
pub mod show;
#[cfg(test)]
mod tests;
