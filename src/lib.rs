//! Shared application engine and automation interface.
pub mod apcmini;
pub mod audio;
pub mod app;
#[cfg(test)]
pub mod binding_tests;
pub mod color;
pub mod engine;
#[cfg(test)]
pub mod engine_tests;
pub mod fixture;
pub mod host;
pub mod keys;
#[cfg(target_os = "linux")]
pub mod kmsdrm;
pub mod layout;
#[cfg(test)]
pub mod layout_tests;
pub mod led;
pub mod link;
pub mod live;
pub mod look;
#[cfg(test)]
pub mod bpm_tests;
#[cfg(test)]
pub mod color_tests;
#[cfg(test)]
pub mod led_tests;
#[cfg(test)]
pub mod master_tests;
#[cfg(test)]
pub mod perf_tests;
#[cfg(test)]
pub mod pair_tests;
#[cfg(test)]
pub mod live_tests;
#[cfg(test)]
pub mod pixel_tests;
#[cfg(test)]
pub mod palette_tests;
#[cfg(test)]
pub mod safety_tests;
#[cfg(test)]
pub mod sync_tests;
pub mod midi;
pub mod motion;
pub mod ndi;
pub mod opendmx;
pub mod osc;
pub mod output;
pub mod pixel;
pub mod project;
pub mod project_patch;
pub mod recording;
pub mod remote;
pub mod shot;
pub mod theme;
pub mod web;
pub mod ui_auto;
pub mod ui_binding;
pub mod ui_control;
pub mod ui_keys;
pub mod ui_laser;
pub mod ui_map;
pub mod ui_midi;
pub mod ui_output;
pub mod ui_pair;
pub mod ui_patch;
pub mod ui_profile;
pub mod ui_sequence;
pub mod ui_show;

pub mod cli;
mod commands;
pub mod control;
pub mod controls;
pub mod editing;
pub mod diagnostics;
