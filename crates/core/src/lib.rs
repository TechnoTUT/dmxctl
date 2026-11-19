//! コア層。画面にもネットワークの操作口にも頼らない、照明の中身の部分。
//!
//! - 設定とデータの形 (`project`・`fixture`・`color`・`keys`・`motion`・`pixel`・`look`)
//! - 値の合成と出力ループ (`engine`)、実行中の状態を変える操作 (`live`)
//! - 機器・プロトコルとのやりとり (`io`: NDI・OSC・DMX・Art-Net・MIDI・音声・Link)
//!
//! この層から `backend` と `ui` は参照しない。向きは `ui` → `backend` → `core` だけ。
// モデル層。パスを変えずに使えるよう、そのまま再公開する
pub use dmxctl_model::{color, fixture, keys, layout, motion, pixel, project};

pub mod engine;
pub mod hold;
pub mod io;
pub mod live;
pub mod led;
pub mod look;
pub mod observe;
pub mod trace;
pub mod wake;
pub mod recorder;
#[cfg(test)]
mod tests;
