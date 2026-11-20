//! OSC受信(TouchDesignerなどから照明を動かす)。
//!
//! 2通りの動かし方がある:
//! - 割り当て(MIDI/OSCタブ・マップ)にOSCアドレスを付けると、MIDIと同じ動作をOSCで実行できる(「OSCラーン」で登録)
//! - 決まったアドレス(`/dmx/...`)で、よく使う操作を直接呼べる(`ADDRESSES`)
//!
//! 値は 0.0〜1.0 で受け取る(フェーダー用)。ボタン系は 0.5以上で押した、未満で離した扱い。値なしのメッセージは1回押した扱い

use crate::color::{ColorTarget, NamedColor};
use crate::engine::{lock, Core, Shared};
use crate::project::*;
use std::time::{Duration, Instant};

pub use dmxctl_io::osc::{encode, parse, Arg, Message, OscListener};

/// OSCで「押している間だけ」のレーザー: 最後に1を受けてからこの時間届かなければ止める
pub const LASER_OSC_HOLD: std::time::Duration = std::time::Duration::from_secs(1);

/// 決まったアドレスの一覧(画面の説明用)。値の範囲は 0.0〜1.0
pub const ADDRESSES: &[(&str, &str)] = &[
    ("/dmx/master  f", "全体光量(0〜1)"),
    ("/dmx/blackout  f", "ブラックアウト(1=ON / 0=OFF)"),
    ("/dmx/fade  f [秒]", "暗転フェード(1=暗くする / 0=戻す)。秒数を付けられる。/dmx/fade/time f で標準の秒数"),
    ("/dmx/flash  f", "フラッシュ(1の間だけ白で全開 / 0で戻す)。/dmx/flash/color f は今の色のまま全開"),
    ("/dmx/laser  f", "ARM済みのレーザーを出力(1を送信し続けている間のみ。1秒途絶えると停止 / 0で停止)。/dmx/laser/latch f で常時出力のON/OFF"),
    ("/dmx/bpm  f", "BPM(数値そのまま。例: 128)"),
    ("/dmx/tap", "タップテンポ"),
    ("/dmx/resync", "頭出し(小節の頭へ)"),
    ("/dmx/scene  s", "シーンを名前で呼び出す"),
    ("/dmx/position  s", "位置プリセットを名前で呼び出す"),
    ("/dmx/color  s", "色(赤/red, 青/blue など)。全体/個別スイッチに従う"),
    ("/dmx/hue  f", "色相(0〜1で一周)。全体/個別スイッチに従う"),
    ("/dmx/main  s", "メイン色(色の名前)。/dmx/main/hue f で色相"),
    ("/dmx/sub  s", "サブ色(色の名前)。/dmx/sub/hue f で色相"),
    ("/dmx/main/rgb  f f f [秒]", "メイン色をRGBで(0〜1。0〜255でもよい)。受けた色へなめらかに近づける。4つ目の数値で速さ(秒)。/dmx/sub/rgb はサブ色"),
    ("/dmx/main/rgb/at  f f f 秒", "先読みした色: その秒数後にちょうどこの色になるよう、まっすぐ近づける(遅れなく出すため。/dmx/sub/rgb/at も)"),
    ("/dmx/rgb/smooth  f", "RGBで受けた色へ近づける速さ(秒。0ですぐ。標準 0.5)"),
    ("/dmx/rgb/off", "RGBで受けた色をやめて、パレットの色に戻す(手で色を選んでも戻る)"),
    ("/dmx/level  f [秒]", "明るさ(0〜1。波形の大きさなど)。全体光量に掛ける。なめらかに近づく。/dmx/level/at f 秒 で先読み"),
    ("/dmx/level/depth  f", "明るさの効き具合(0=効かない〜1=0まで暗くなる。標準 0.7)。/dmx/level/off でやめる"),
    ("/dmx/phrase  s", "曲のフレーズ(intro / verse / chorus / bridge / up / down / outro)。/dmx/phrase/<名前> に割り当てた動作を実行、無ければ同じ名前のシーンを呼び出す"),
    ("/dmx/swap  f", "メイン⇄サブ入れ替え(値なし/1で入れ替え)。/dmx/swap/auto f で自動入れ替えON/OFF"),
    ("/dmx/effect  s f", "エフェクトのON/OFF(名前, 1/0)"),
    ("/dmx/chase  s f", "チェイスのON/OFF(名前, 1/0)"),
    ("/dmx/page  i", "パッドのページ(1〜8)"),
    ("/dmx/fixture/<番号>/<機能>  f", "機材1台のチャンネル(0〜1)。番号はパッチタブのID。例: /dmx/fixture/1/dimmer"),
    ("/dmx/all/<機能>  f", "全機材のチャンネル(0〜1)。例: /dmx/all/pan"),
];

// ------------------------------------------------------------ 動作

/// フレーズの名前を揃える(大文字小文字・先頭の「:」・番号を除く。verse2 → verse)
pub(crate) fn phrase_key(s: &str) -> String {
    s.trim().trim_start_matches(':').to_lowercase().trim_end_matches(|c: char| c.is_ascii_digit()).trim_end_matches(['-', '_', ' ']).to_string()
}

/// フレーズの日本語の呼び名(シーンの名前に使えるように)
fn phrase_ja(key: &str) -> Option<&'static str> {
    Some(match key {
        "intro" => "イントロ",
        "verse" => "バース",
        "bridge" => "ブリッジ",
        "chorus" => "サビ",
        "up" => "アップ",
        "down" => "ダウン",
        "outro" => "アウトロ",
        _ => return None,
    })
}

fn color_by_name(s: &str) -> Option<NamedColor> {
    NamedColor::from_name(s)
}

impl Core {
    /// 曲のフレーズが変わった(Beat Link Triggerのフレーズのトリガーなどから)。
    /// /dmx/phrase/<名前> に割り当てた動作を実行する。無ければ同じ名前のシーン(英語・日本語どちらでも)を呼び出す
    pub fn phrase(&mut self, name: &str) {
        self.note_live_input();
        self.rt.phrase = name.trim().to_string();
        self.rt.phrase_beat = Some(self.rt.beat);
        let key = phrase_key(name);
        // 手動優先の間は、覚えるだけで照明は変えない
        if self.manual_active() {
            return;
        }

        let mut names = vec![name.trim().to_string(), key.clone()];
        names.extend(phrase_ja(&key).map(str::to_string));
        names.extend(PHRASES.iter().find(|(k, _)| *k == key).map(|(_, ja)| ja.to_string()));
        names.dedup();
        let addrs: Vec<String> = names.iter().map(|n| format!("/dmx/phrase/{n}")).collect();
        let hits: Vec<usize> = self.project.bindings.iter().enumerate().filter(|(_, b)| b.osc.as_ref().is_some_and(|a| addrs.contains(a))).map(|(i, _)| i).collect();
        if !hits.is_empty() {
            for i in hits {
                self.apply_binding(i, 1.0, true);
                self.apply_binding(i, 1.0, false);
            }
            return;
        }
        // オートのプリセットの見た目。プリセットでシーンを選んでいなければ、フレーズと同じ名前のシーンも呼び出す
        let preset_scene = self.auto_phrase(&key);
        let scene = self.project.scenes.iter().find(|s| names.iter().any(|n| s.name.eq_ignore_ascii_case(n))).map(|s| s.name.clone());
        if let (false, Some(s)) = (preset_scene, scene) {
            self.recall_scene(&s);
        }
    }

    pub fn handle_osc(&mut self, m: &Message) {
        if self.rt.observe.is_recording() && self.rt.observe.is_root_input() {
            let args:Vec<serde_json::Value>=m.args.iter().map(|a|match a {
                Arg::Int(v)=>serde_json::json!(v),Arg::Float(v)=>serde_json::json!(v),
                Arg::Str(v)=>serde_json::json!(v),Arg::Bool(v)=>serde_json::json!(v),
            }).collect();
            self.record_input(serde_json::json!({"cmd":"input","source":"osc","address":m.addr,"args":args}));
        }
        let first_num = m.args.iter().find_map(|a| a.num());
        let args_text: Vec<String> = m
            .args
            .iter()
            .map(|a| match a {
                Arg::Int(i) => i.to_string(),
                Arg::Float(f) => format!("{f:.3}"),
                Arg::Str(s) => format!("\"{s}\""),
                Arg::Bool(b) => b.to_string(),
            })
            .collect();
        self.rt.last_osc = format!("{} {}", m.addr, args_text.join(" ")).trim_end().to_string();

        // OSCラーン: 次に来たアドレスを割り当てに登録
        if let Some(idx) = self.rt.osc_learn.take() {
            if let Some(b) = self.project.bindings.get_mut(idx).filter(|b| !(self.rt.learn_guarded && b.is_safety())) {
                b.osc = Some(m.addr.clone());
                self.dirty = true;
            }
            self.rt.learn_guarded &= self.rt.learn.is_some();
            return;
        }

        // フレーズはアドレスに名前を入れて送られることもある(/dmx/phrase/chorus)
        if let Some(name) = m.addr.strip_prefix("/dmx/phrase/") {
            self.rt.phrase = name.to_string();
        }
        let hits: Vec<usize> = self.project.bindings.iter().enumerate().filter(|(_, b)| b.osc.as_deref() == Some(m.addr.as_str())).map(|(i, _)| i).collect();
        if !hits.is_empty() {
            for i in hits {
                match first_num {
                    Some(v) => self.apply_binding(i, v, v >= 0.5),
                    // 値なしは「押して離した」
                    None => {
                        self.apply_binding(i, 1.0, true);
                        self.apply_binding(i, 1.0, false);
                    }
                }
            }
            return;
        }
        self.builtin_osc(m, first_num);
    }

    fn builtin_osc(&mut self, m: &Message, num: Option<f32>) {
        let Some(path) = m.addr.strip_prefix("/dmx/") else { return };
        let text = m.args.iter().find_map(|a| a.text()).map(str::to_string);
        // ボタン系: 値なし、または0.5以上のときだけ動く(TouchDesignerのボタンは押すと1、離すと0を送る)
        let press = num.is_none_or(|v| v >= 0.5);
        let parts: Vec<&str> = path.split('/').collect();
        match parts.as_slice() {
            ["master"] => {
                if let Some(v) = num {
                    self.rt.master = v.clamp(0.0, 1.0);
                }
            }
            ["blackout"] => self.rt.blackout = press,
            ["bpm"] => {
                if let Some(v) = num {
                    self.set_bpm(v);
                }
            }
            ["tap"] if press => self.tap(),
            ["resync"] if press => self.resync(),
            ["scene"] if press => {
                if let Some(n) = text {
                    self.recall_scene(&n);
                }
            }
            ["position"] if press => {
                if let Some(n) = text {
                    self.recall_position(&n);
                }
            }
            ["color"] if press => {
                if let Some(c) = text.as_deref().and_then(color_by_name) {
                    self.apply_named_color(c, ColorTarget::Scope);
                }
            }
            ["hue"] => {
                if let Some(v) = num {
                    self.apply_hue(v.clamp(0.0, 1.0) * 360.0, ColorTarget::Scope);
                }
            }
            ["effect"] | ["chase"] => {
                let Some(name) = text else { return };
                let on = num.is_none_or(|v| v >= 0.5);
                if parts[0] == "effect" {
                    self.set_effect(&name, on);
                    if self.project.pixel_fx.iter().any(|e| e.name == name) {
                        self.set_pixel_fx(&name, on);
                    }
                } else {
                    for c in self.project.chases.iter_mut().filter(|c| c.name == name) {
                        c.enabled = on;
                    }
                }
                self.dirty = true;
            }
            [slot @ ("main" | "sub")] if press => {
                if let Some(c) = text.as_deref().and_then(color_by_name) {
                    self.apply_named_color(c, if *slot == "main" { ColorTarget::Main } else { ColorTarget::Sub });
                }
            }
            [slot @ ("main" | "sub"), "rgb", rest @ ..] if rest.is_empty() || rest == ["at"] => {
                let nums: Vec<f32> = m.args.iter().filter_map(|a| a.num()).collect();
                if let [r, g, b, more @ ..] = nums.as_slice() {
                    // 0〜255で送ってくるものもある(どれかが1より大きければ0〜255とみなす)
                    let scale = if [r, g, b].iter().any(|v| **v > 1.0) { 255.0 } else { 1.0 };
                    let rgb = [r, g, b].map(|v| (v / scale).clamp(0.0, 1.0));
                    let slot = if *slot == "main" { 0 } else { 1 };
                    if rest.is_empty() {
                        self.osc_rgb(slot, rgb, more.first().copied(), None);
                    } else {
                        // /rgb/at r g b 秒: 先読みした色。その秒数後にちょうどその色になる(秒が無ければ0.3秒)
                        self.osc_rgb(slot, rgb, None, Some(more.first().copied().unwrap_or(0.3)));
                    }
                }
            }
            ["level", rest @ ..] if rest.is_empty() || rest == ["at"] => {
                let nums: Vec<f32> = m.args.iter().filter_map(|a| a.num()).collect();
                if let [v, more @ ..] = nums.as_slice() {
                    if rest.is_empty() {
                        self.osc_level(*v, more.first().copied(), None);
                    } else {
                        self.osc_level(*v, None, Some(more.first().copied().unwrap_or(0.3)));
                    }
                }
            }
            ["level", "depth"] => {
                if let Some(v) = num {
                    self.project.osc_level_depth = v.clamp(0.0, 1.0);
                    self.dirty = true;
                }
            }
            ["level", "off"] if press => self.rt.osc_level = None,
            ["rgb", "smooth"] => {
                if let Some(v) = num {
                    self.project.osc_rgb_secs = v.clamp(0.0, 30.0);
                    self.dirty = true;
                }
            }
            ["rgb", "off"] if press => {
                self.rt.osc_rgb = [None, None];
            }
            // フレーズの終わりに0を送ってくるものもあるので、押した扱いのときだけ
            ["phrase"] if press => {
                if let Some(n) = text {
                    self.phrase(&n);
                }
            }
            ["phrase", name] if press => self.phrase(name),
            [slot @ ("main" | "sub"), "hue"] => {
                if let Some(v) = num {
                    self.apply_hue(v.clamp(0.0, 1.0) * 360.0, if *slot == "main" { ColorTarget::Main } else { ColorTarget::Sub });
                }
            }
            ["swap"] if press => self.swap_palette(),
            ["flash"] => self.set_flash(press.then_some(true)),
            // 押している間だけ: 送る側が落ちても止まるよう、送り続けないと切れる
            ["laser"] => {
                self.rt.laser_hold = press;
                self.rt.laser_hold_until = press.then(|| self.now() + LASER_OSC_HOLD);
            }
            ["laser", "latch"] => self.rt.laser_latch = press,
            // 2つ目の数値があれば秒数(無ければ設定の秒数)
            ["fade"] => {
                let secs = m.args.iter().filter_map(|a| a.num()).nth(1).unwrap_or(self.project.fade_secs);
                self.fade_to_black(press, secs);
            }
            ["fade", "time"] => {
                if let Some(v) = num {
                    self.project.fade_secs = v.clamp(0.0, 60.0);
                    self.dirty = true;
                }
            }
            ["flash", "color"] => self.set_flash(press.then_some(false)),
            ["swap", "auto"] => self.set_palette_auto(press),
            ["page"] => {
                if let Some(v) = num {
                    self.rt.page = (v.round() as i64 - 1).clamp(0, PAGES as i64 - 1) as u8;
                }
            }
            ["fixture", id, attr] => {
                if let (Ok(id), Some(v)) = (id.parse::<u32>(), num) {
                    self.set_attr(&Selector::Fixture(id), attr, v.clamp(0.0, 1.0) * 255.0);
                }
            }
            ["all", attr] => {
                if let Some(v) = num {
                    self.set_attr(&Selector::All, attr, v.clamp(0.0, 1.0) * 255.0);
                }
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------ 受信スレッド

/// 指定のUDPポートでOSCを待ち受け、受けたメッセージで `Core` を動かす。Dropで止まる
pub struct OscServer {
    _listener: OscListener,
    pub port: u16,
}

impl OscServer {
    /// `ctx`は受信したときに画面を更新させるためのもの(映像ソフトから連続で来ても、画面の更新は0.25秒に1回まで)
    pub fn start(core: Shared, ctx: Option<crate::wake::Wake>, port: u16) -> Result<OscServer, String> {
        lock(&core).rt.osc_status = format!("受信待ち (UDP {port})");
        let mut last_status = Instant::now();
        let listener = OscListener::start(port, move |msgs, count| {
            {
                let mut c = lock(&core);
                for m in msgs {
                    c.handle_osc(m);
                }
                if last_status.elapsed() > Duration::from_millis(250) {
                    c.rt.osc_status = format!("受信中 (UDP {port}) {count}件");
                    last_status = Instant::now();
                }
            }
            if let Some(ctx) = &ctx {
                ctx.request_repaint_after(Duration::from_millis(250));
            }
        })?;
        Ok(OscServer { _listener: listener, port })
    }
}
