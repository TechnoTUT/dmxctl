//! 入出力層。機器とプロトコルの、設定やエンジンの状態を知らない部分。
//! USB-DMX・Art-Net の送出、Ableton Link の窓口 Carabiner への接続など。
//!
//! 保存する設定の型は `dmxctl-model` から借りる。エンジンの `Core` には触れない。
//! `Core` を動かすための橋渡しは `dmxctl-core` にある。
pub mod carabiner;
pub mod link;
pub mod osc;
pub mod opendmx;
pub mod output;
