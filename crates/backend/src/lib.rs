//! バックエンド層。`core` を画面なしで動かし、外から操作できるようにする部分。
//!
//! 起動と保存 (`host`)、`serve`/`connect` の通信 (`remote`)、ブラウザの操作パネル (`web`)、
//! 自動操作用の CLI (`cli`・`control`・`commands`・`editing`・`recording`・`diagnostics`)。
//!
//! この層から `ui` は参照しない。
pub mod cli;
mod commands;
pub mod control;
pub mod controls;
pub mod diagnostics;
pub mod editing;
pub mod entry;
pub mod host;
pub mod project_patch;
pub mod recording;
pub mod remote;
pub mod web;
#[cfg(test)]
mod tests;
