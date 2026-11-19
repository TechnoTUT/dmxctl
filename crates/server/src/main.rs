//! 画面を持たない dmxctl。`dmxctl` と同じ引数のうち、画面の要らない動かし方だけを受け付ける。
//! 画面のクレートを含まないので、デスクトップ環境の無いサーバーやコンテナでビルド・実行できる。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use dmxctl_backend::entry::{self, Mode};

const USAGE: &str = "dmxctl-server headless [--project FILE] [--listen IP:PORT] [--output none|live] [--inputs none|configured] [--no-autosave]
    画面を出さずに動かす(OSC受信・Ableton Link・オートを必ずONにする)
dmxctl-server apply-layout
    保存してあるプロジェクトを、おすすめのMIDI配置にする(先にバックアップを残す)
dmxctl-server carabiner install
    Ableton Link に参加する Carabiner を、配布元から取って、設定フォルダに入れる。Linkを使うときに1回だけ要る
dmxctl-server serve|ctl|inspect|simulate|run|watch|log|diff ...
    自動操作・検証用のCLI(JSONで返す)。詳しくは下

画面を出す動かし方(引数なし・kmsdrm・connect)は、画面付きの dmxctl で使う。
以前の書き方(--headless / --apply-layout / DMXCTL_HEADLESS=1)も使えます。
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (mode, rest) = Mode::of(&args);
    entry::attach_console();
    if entry::run_noninteractive(&mode, rest, USAGE) {
        return;
    }
    if mode.is_interactive() {
        eprintln!("この実行ファイルは画面を持ちません。使える動かし方は次のとおりです。\n{USAGE}");
        std::process::exit(2);
    }
    entry::run_headless(entry::parse_launch(&mode, rest))
}
