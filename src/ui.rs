//! 画面層。eframe と KMS/DRM の画面、タブごとの描画、見た目。
//!
//! 実行中の状態は `backend::host::Host` を通して扱い、`core` を直接動かさない。
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
pub mod sequence;
pub mod show;
#[cfg(test)]
mod tests;
