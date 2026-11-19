//! サーバー/クライアント構成(`--remote-listen` / `dmxctl connect`)。
//!
//! - **サーバー**: いつもどおり Core・出力ループ・OSC/Link/NDI/音声/操作パネルを動かし、
//!   LANのクライアントへプロジェクトと実行中の状態を配る。DMXの送出はサーバーだけが行う。
//! - **クライアント**: 画面(egui)とMIDIコントローラーだけを手元で動かす。手元の Core はサーバーの写しで、
//!   出力ループも外部との連携も動かさない。
//!
//! 手元の操作は2通りでサーバーへ届ける:
//! 1. 実行中の状態を変える操作(TAP・シーン・色・MIDIの入力など): `Core::live` が `LiveOp` をそのまま送る。
//!    手元の写しは変えず、サーバーから戻ってくる状態で更新される
//! 2. 画面がプロジェクトを直接書き換える編集(スライダー・名前の欄など): 1フレームの前後を比べた差分
//!    (`project_patch::Change`)を送る。手元の写しには、確認される前から編集が入っている
//!
//! 通信は TCP の上で1行1つのJSON。信頼できるネットワークだけで使う前提で、認証はしない。
//! サーバーは約30Hzで状態を、約10Hzでプロジェクトの差分を配る。クライアントは送ったものに通し番号(`seq`)を付け、
//! サーバーは処理済みの番号(`ack`)を状態に付けて返す。
//!
//! プロジェクトの食い違いを防ぐため、クライアントは「サーバーが持っているはずのプロジェクト」(`confirmed`)と
//! 「まだ処理済みと言われていない自分の編集」(`pending`)を分けて持つ。サーバーは接続ごとに、そのクライアントが
//! 持っているはずのプロジェクトを覚えて差分を作る。どちらも「処理済みの自分の編集 → 届いた差分」の順に当てるので、
//! 行き違いがあっても同じものになる。手元の写しは `confirmed` に `pending` を重ねたもの。

use crate::{editing::CoreEditing, project_patch::CoreProjectPatch};
use dmxctl_core::engine::{lock, Core, Shared, EMERGENCY};
use dmxctl_core::fixture::{Library, Profile};
use dmxctl_core::live::{LiveForward, LiveOp};
use dmxctl_core::project::{Project, PAGES};
use crate::project_patch::{self, Change};
use anyhow::{bail, ensure, Context as _};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const DEFAULT_PORT: u16 = 9300;
const PROTOCOL: u64 = 2;
/// 1行の上限(機材定義・プロジェクト全体を1行で送るので大きめ)
const MAX_LINE: u64 = 32 * 1024 * 1024;
/// 状態を配る間隔(約30Hz)と、プロジェクトの変化を調べる間隔(その3回に1回)
const BROADCAST: Duration = Duration::from_millis(33);
const PROJECT_EVERY: u32 = 3;
/// 同時につなげるクライアントの数
const MAX_CLIENTS: usize = 16;
/// 機材定義の保存・削除に失敗したときの説明(安全の設定を変えようとしたときなど)
const SAFETY_READ_ONLY: &str = "機材定義を保存できませんでした(レーザーなどの安全の設定は、サーバーのPCの画面か、サーバーを --remote-allow-safety-edit を付けて起動したときだけ変えられます)";

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn age(t: Option<Instant>) -> Option<u64> {
    t.map(|t| t.elapsed().as_millis() as u64)
}

fn from_age(v: &Value) -> Option<Instant> {
    v.as_u64().and_then(|ms| Instant::now().checked_sub(Duration::from_millis(ms)))
}

fn hex(data: &[u8]) -> String {
    const D: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        s.push(D[(b >> 4) as usize] as char);
        s.push(D[(b & 15) as usize] as char);
    }
    s
}

fn unhex(s: &str, out: &mut [u8; 512]) {
    let v = |c: u8| match c {
        b'0'..=b'9' => c - b'0',
        b'a'..=b'f' => c - b'a' + 10,
        b'A'..=b'F' => c - b'A' + 10,
        _ => 0,
    };
    for (i, pair) in s.as_bytes().chunks(2).take(512).enumerate() {
        if pair.len() == 2 {
            out[i] = (v(pair[0]) << 4) | v(pair[1]);
        }
    }
}

fn sorted_pairs<K: Ord + Copy, V: Copy>(m: &HashMap<K, V>) -> Vec<(K, V)> {
    let mut v: Vec<(K, V)> = m.iter().map(|(k, v)| (*k, *v)).collect();
    v.sort_by_key(|p| p.0);
    v
}

fn read_line(reader: &mut BufReader<TcpStream>, buf: &mut Vec<u8>) -> anyhow::Result<bool> {
    buf.clear();
    let n = reader.by_ref().take(MAX_LINE + 1).read_until(b'\n', buf)?;
    if n == 0 {
        return Ok(false);
    }
    ensure!(buf.len() as u64 <= MAX_LINE, "message is too large");
    Ok(true)
}


mod client;
mod server;
mod state;
#[cfg(test)]
mod tests;

pub use client::{Client, Initial, Link};
pub use server::{Files, OpenRequest, OpenSource, RemoteServer, ServerEvents};
pub use state::Devices;
// 子のモジュールが `use super::*` で、状態の写しの項目も使えるようにする
use state::*;

/// `--remote-listen` の値: `192.168.1.10` / `0.0.0.0:9300` など。
/// 認証をしないので、どこから受けるか(アドレス)は必ず書いてもらう。ポートを省くと 9300
pub fn parse_listen(value: &str) -> anyhow::Result<SocketAddr> {
    if let Ok(ip) = value.parse::<std::net::IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_PORT));
    }
    value.parse().with_context(|| format!("待ち受けのアドレスが不正です: {value}(例: 照明用のLANのアドレスなら 192.168.1.10、どこからでも受けるなら 0.0.0.0)"))
}
