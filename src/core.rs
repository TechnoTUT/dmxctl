//! コア層。画面にもネットワークの操作口にも頼らない、照明の中身の部分。
//!
//! - 設定とデータの形 (`project`・`fixture`・`color`・`keys`・`motion`・`pixel`・`look`)
//! - 値の合成と出力ループ (`engine`)
//! - 機器・プロトコルとのやりとり (`io`: NDI・OSC・DMX・Art-Net・MIDI・音声・Link)
//!
//! この層から `backend` と `ui` は参照しない。向きは `ui` → `backend` → `core` だけ。
pub mod color;
pub mod engine;
pub mod fixture;
pub mod io;
pub mod keys;
pub mod layout;
pub mod led;
pub mod look;
pub mod motion;
pub mod pixel;
pub mod project;
#[cfg(test)]
mod tests;
