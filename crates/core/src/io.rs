//! 機器・プロトコルの入出力。NDI受信・OSC受信・USB-DMX送出・Art-Net送出・MIDI・音声・Ableton Link。
pub mod audio;
pub mod midi;
pub mod ndi;
pub mod osc;
pub use dmxctl_model::apcmini;
pub use dmxctl_io::{carabiner, link, opendmx, output};
