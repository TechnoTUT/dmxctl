//! 保存されるデータ(パッチ・シーン・エフェクト・MIDI割り当て・出力設定)。

use crate::color::{ColorRole, ColorTarget, NamedColor, Palette, PaletteOp, ScopeOp, SelectOp};
use crate::fixture::Library;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const NUM_UNIVERSES: usize = 32;

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize)]
pub struct FixtureInstance {
    #[schemars(extend("readOnly"=true))]
    pub id: u32,
    pub name: String,
    pub profile: String,
    pub mode: usize,
    /// 0始まり(表示は+1)
    pub universe: usize,
    /// 1始まり
    pub address: u16,
    /// 現在のベース値(DMX値)
    pub values: Vec<u8>,
    /// 安全のため保存せず、起動時は必ずfalse
    #[serde(skip)]
    pub armed: bool,
    /// メイン色/サブ色のどちらを出すか
    #[serde(default)]
    pub color_role: ColorRole,
    /// 使う(false=出力しない。操作・エフェクトの対象からも外す)。LEDバーの本数の切り替えなどに使う
    #[serde(default = "yes")]
    pub enabled: bool,
    /// 動いてよい範囲(機能名 → [下限, 上限] のDMX値)。レーザーの位置など。出力の直前に必ずこの中に収める
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub limits: BTreeMap<String, [u8; 2]>,
}

fn yes() -> bool {
    true
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Group {
    pub name: String,
    pub fixtures: Vec<u32>,
}

/// 操作の対象(機材の選び方)
#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum Selector {
    All,
    Fixture(u32),
    Group(String),
    /// 同じ機種すべて(プロファイルid)
    Profile(String),
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum Wave {
    Sine,
    Triangle,
    Saw,
    Square,
    Random,
}

impl Wave {
    pub const ALL: [Wave; 5] = [Wave::Sine, Wave::Triangle, Wave::Saw, Wave::Square, Wave::Random];
    pub fn label(self) -> &'static str {
        match self {
            Wave::Sine => "サイン",
            Wave::Triangle => "三角",
            Wave::Saw => "ノコギリ",
            Wave::Square => "矩形",
            Wave::Random => "ランダム",
        }
    }
    /// phase: 周期の小数部(0..1) / cycle: 何周目か。戻り値は0..1
    pub fn eval(self, phase: f32, cycle: u64, seed: u32) -> f32 {
        match self {
            Wave::Sine => 0.5 - 0.5 * (phase * std::f32::consts::TAU).cos(),
            Wave::Triangle => 1.0 - (2.0 * phase - 1.0).abs(),
            Wave::Saw => phase,
            Wave::Square => {
                if phase < 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Wave::Random => {
                let mut x = cycle.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (seed as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9);
                x ^= x >> 30;
                x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
                x ^= x >> 27;
                (x & 0xFFFF) as f32 / 65535.0
            }
        }
    }
}

/// 対象チャンネルを min〜max の間で周期的に動かす
#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize)]
pub struct Effect {
    pub name: String,
    pub enabled: bool,
    pub sel: Selector,
    pub attr: String,
    pub wave: Wave,
    #[schemars(range(min=0))]
    pub rate_hz: f32,
    #[schemars(range(min=0,max=255))]
    pub min: f32,
    #[schemars(range(min=0,max=255))]
    pub max: f32,
    /// 位相オフセット(度)
    pub phase: f32,
    /// 選択内の機材ごとにずらす位相(度)
    pub spread: f32,
    /// 0=自由(rate_hzで動く) / 0より大きい=BPMに同期して、この拍数で1周期
    #[serde(default)]
    pub beats: f32,
}

impl Effect {
    pub fn new(name: &str, attr: &str) -> Self {
        Effect {
            name: name.into(),
            enabled: false,
            sel: Selector::All,
            attr: attr.into(),
            wave: Wave::Sine,
            rate_hz: 0.25,
            min: 64.0,
            max: 192.0,
            phase: 0.0,
            spread: 0.0,
            beats: 0.0,
        }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize)]
pub struct SceneValue {
    pub fixture: u32,
    pub attr: String,
    /// 上位バイト+下位バイト/256
    pub value: f32,
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub name: String,
    /// 呼び出し時のフェード秒数
    #[schemars(range(min=0,max=120))]
    pub fade: f32,
    pub values: Vec<SceneValue>,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum TriggerKind {
    Note,
    Cc,
    /// パソコンのキーボード(番号は crate::keys のキーの番号)
    Key,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct MidiTrigger {
    pub kind: TriggerKind,
    /// 0..15。Noneならどのチャンネルでも反応
    pub channel: Option<u8>,
    pub number: u8,
}

impl MidiTrigger {
    pub fn label(&self) -> String {
        let k = match self.kind {
            TriggerKind::Note => "Note",
            TriggerKind::Cc => "CC",
            TriggerKind::Key => return format!("キー {}", crate::keys::name(self.number)),
        };
        match self.channel {
            Some(c) => format!("{k} {} (ch{})", self.number, c + 1),
            None => format!("{k} {} (全ch)", self.number),
        }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum PadMode {
    /// フェーダー/ノブ: 入力値0..127を off〜on に対応させる
    Continuous,
    /// 押している間だけ on、離すと off
    Momentary,
    /// 押すたびに on/off を切り替え
    Toggle,
    /// 押したとき on にするだけ
    Trigger,
}

impl PadMode {
    pub const ALL: [PadMode; 4] = [PadMode::Continuous, PadMode::Momentary, PadMode::Toggle, PadMode::Trigger];
    pub fn label(self) -> &'static str {
        match self {
            PadMode::Continuous => "フェーダー(連続)",
            PadMode::Momentary => "押している間",
            PadMode::Toggle => "トグル",
            PadMode::Trigger => "ワンショット",
        }
    }
}

pub const BPM_MIN: f32 = 20.0;
pub const BPM_MAX: f32 = 300.0;
/// 1小節の拍数(拍の表示と「頭出し」の単位)
pub const BEATS_PER_BAR: i64 = 4;

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum ChaseOrder {
    Forward,
    PingPong,
    Random,
}

impl ChaseOrder {
    pub const ALL: [ChaseOrder; 3] = [ChaseOrder::Forward, ChaseOrder::PingPong, ChaseOrder::Random];
    pub fn label(self) -> &'static str {
        match self {
            ChaseOrder::Forward => "順番",
            ChaseOrder::PingPong => "往復",
            ChaseOrder::Random => "ランダム",
        }
    }
}

/// 通し番号`idx`(拍の位置から決まる)が、`n`個のステップのうちどれに当たるか
pub fn step_index(order: ChaseOrder, idx: i64, n: usize) -> usize {
    if n <= 1 {
        return 0;
    }
    let n_i = n as i64;
    match order {
        ChaseOrder::Forward => idx.rem_euclid(n_i) as usize,
        ChaseOrder::PingPong => {
            let period = 2 * n_i - 2;
            let p = idx.rem_euclid(period);
            (if p < n_i { p } else { period - p }) as usize
        }
        ChaseOrder::Random => {
            // 1周(nステップ)ごとに、全ステップを一度ずつ、順番をシャッフルして出す。
            // 偏らず、周の境目でも同じステップが続かない
            if n == 2 {
                return idx.rem_euclid(2) as usize;
            }
            let (round, pos) = (idx.div_euclid(n_i), idx.rem_euclid(n_i) as usize);
            let mut cur = shuffled(round, n);
            let prev_last = *shuffled(round - 1, n).last().unwrap();
            if cur[0] == prev_last {
                cur.swap(0, 1);
            }
            cur[pos]
        }
    }
}

/// `round`から決まる、0..n の並べ替え(Fisher-Yates + splitmix64)
fn shuffled(round: i64, n: usize) -> Vec<usize> {
    let mut x = (round as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    let mut next = move || {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let mut v: Vec<usize> = (0..n).collect();
    for i in (1..n).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        v.swap(i, j);
    }
    v
}

/// BPMに合わせてシーンを順番に呼び出す
#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Chase {
    pub name: String,
    pub enabled: bool,
    /// 呼び出すシーン名(順番)
    pub steps: Vec<String>,
    /// 1ステップの長さ(拍)
    pub step_beats: f32,
    /// シーン切り替えのフェード(拍)。0=カット
    pub fade_beats: f32,
    pub order: ChaseOrder,
}

impl Default for Chase {
    fn default() -> Self {
        Chase { name: "チェイス".into(), enabled: false, steps: Vec::new(), step_beats: 1.0, fade_beats: 0.0, order: ChaseOrder::Forward }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum TempoOp {
    /// タップテンポ(叩いた間隔からBPMを決め、拍の位置も叩いたタイミングに合わせる)
    Tap,
    /// 拍の位置を小節の頭に戻す
    Resync,
    /// フェーダー/ノブ: 入力値0..127を min〜max のBPMに対応させる
    Set { min: f32, max: f32 },
    /// 押すたびにBPMを delta だけ増減
    Nudge { delta: f32 },
    /// テンポの元を 内部 ⇄ Ableton Link で切り替える
    Link,
    /// テンポの元を 内部 ⇄ 音声からの自動検出 で切り替える
    Audio,
}

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum PairOp {
    /// 連動のON/OFF
    Link,
    /// 左右反転のON/OFF
    Mirror,
}

/// 2台の機材のペア設定。`follower`は`leader`(元)の値に追従し、指定した機能は反転する
#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Pair {
    pub name: String,
    /// 連動のON/OFF
    pub enabled: bool,
    pub leader: u32,
    pub follower: u32,
    /// 左右反転のON/OFF(OFFなら、反転せずそのまま追従)
    pub mirror: bool,
    /// 反転する機能(attr)
    pub invert: Vec<String>,
    /// 連動しないカテゴリ(既定は「制御」: リセットなどのマクロまで連動させない)
    pub skip_cats: Vec<String>,
}

impl Default for Pair {
    fn default() -> Self {
        Pair { name: "ペア".into(), enabled: true, leader: 0, follower: 0, mirror: true, invert: vec!["pan".into()], skip_cats: vec!["制御".into()] }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize)]
pub enum Action {
    SetValue { sel: Selector, attr: String, mode: PadMode, on: f32, off: f32 },
    Scene { name: String },
    Effect { name: String, mode: PadMode },
    Blackout { mode: PadMode },
    /// 全体光量(すべての機材のディマーにかかるマスター)。on/offは%(0〜100)
    Master { mode: PadMode, on: f32, off: f32 },
    /// BPM(テンポ)の操作
    Tempo { op: TempoOp },
    /// チェイスの開始/停止
    Chase { name: String, mode: PadMode },
    /// ペアの連動 / 左右反転のON/OFF
    Pair { name: String, op: PairOp, mode: PadMode },
    /// 色の全体制御。colorがNoneなら、フェーダー/ノブで色相を動かす
    Color { color: Option<NamedColor>, target: ColorTarget },
    /// 色の操作先を「全体」⇄「個別(選択中の機材)」に切り替える
    ColorScope { op: ScopeOp },
    /// 個別モードで色を掛ける機材を選ぶ
    Select { fixture: u32, op: SelectOp },
    /// 位置プリセット(パン/チルト)の呼び出し
    Position { name: String },
    /// パッドのページ切り替え
    Page { op: PageOp },
    /// メイン色/サブ色の入れ替え
    Palette { op: PaletteOp },
    /// フラッシュ: 全機材(レーザーを除く)を全開にする。white=白で全開(falseなら今の色のまま)。元の値は変えない
    Flash { white: bool, mode: PadMode },
    /// 暗転フェード: secs秒かけて暗くする/戻す
    Fade { secs: f32, mode: PadMode },
    /// 対象の機材だけの明るさ(フェーダー)。全体光量と同じように、ディマー(無ければ色の強さ)に掛かる
    Level { sel: Selector },
    /// ミュート: 対象の機材を消す(明るさのチャンネルを0にする)
    Mute { sel: Selector, mode: PadMode },
    /// LEDバー(ピクセルエフェクト)全体の速さ・斜めを、フェーダーで変える
    Pixel { param: crate::pixel::PixelParam },
    /// 機材がメイン色/サブ色のどちらで光るかを決める
    Role { sel: Selector, op: RoleOp },
    /// ARM(レーザーなど、許可するまで安全値を出し続ける機材の出力を許可する)
    Arm { sel: Selector, mode: PadMode },
    /// NDI(映像の平均色・音の色)をメイン色/サブ色に使う。同じものをもう一度押すと使わない
    LiveColor { role: crate::color::ColorRole, source: crate::ndi::LiveSource },
    /// メイン⇄サブの自動入れ替えの間隔(拍)。押すと自動入れ替えもONにする
    SwapBeats { beats: f32 },
    /// LEDバーを何本使うか(パッチの順に先頭から。残りは使わない)
    BarCount { count: u8 },
    /// ムービングの動き(形・1周の長さ・揃え方・向き・大きさ)
    Motion { op: crate::motion::MotionOp },
    /// キーボードの数字の段に出すLEDバーの模様のセットを切り替える
    KeyPage { op: PageOp },
}

/// 色の系統の決め方
#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RoleOp {
    /// 押すたびに メイン ⇄ サブ
    Toggle,
    Main,
    Sub,
    /// 1台おきに メイン/サブ(対象の順番で)
    Alternate,
}

impl RoleOp {
    pub const ALL: [RoleOp; 4] = [RoleOp::Toggle, RoleOp::Main, RoleOp::Sub, RoleOp::Alternate];
    pub fn label(self) -> &'static str {
        match self {
            RoleOp::Toggle => "押すたびに メイン⇄サブ",
            RoleOp::Main => "メインにする",
            RoleOp::Sub => "サブにする",
            RoleOp::Alternate => "1台おきに メイン/サブ",
        }
    }
}

/// パッドのページ数
pub const PAGES: u8 = 8;
/// 位置プリセットに記録する機能(パン/チルト。16bitの下位は自動で含まれる)
pub const POSITION_ATTRS: [&str; 2] = ["pan", "tilt"];

#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum PageOp {
    /// 指定のページへ(0始まり)
    Go(u8),
    Next,
    Prev,
}

impl PageOp {
    pub fn label(self) -> String {
        match self {
            PageOp::Go(p) => format!("ページ{}へ", p + 1),
            PageOp::Next => "次のページ".into(),
            PageOp::Prev => "前のページ".into(),
        }
    }
}

/// テンポと拍の位置を、何に合わせるか
#[derive(schemars::JsonSchema, Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Default)]
pub enum TempoSource {
    /// このアプリのBPM(TAP・手入力)
    #[default]
    Internal,
    /// Ableton Link(映像ソフトやDAWと共有)
    Link,
    /// 外部から来るMIDIクロック
    MidiClock,
    /// 音声入力からBPMと拍のタイミングを自動で検出
    Audio,
}

impl TempoSource {
    pub const ALL: [TempoSource; 4] = [TempoSource::Internal, TempoSource::Link, TempoSource::MidiClock, TempoSource::Audio];
    pub fn label(self) -> &'static str {
        match self {
            TempoSource::Internal => "内部(TAP・手入力)",
            TempoSource::Link => "Ableton Link",
            TempoSource::MidiClock => "MIDIクロック",
            TempoSource::Audio => "音声から自動検出",
        }
    }
}

impl Action {
    pub fn kind_label(&self) -> &'static str {
        match self {
            Action::SetValue { .. } => "チャンネル値",
            Action::Scene { .. } => "シーン呼出",
            Action::Effect { .. } => "エフェクト",
            Action::Blackout { .. } => "ブラックアウト",
            Action::Master { .. } => "全体光量",
            Action::Tempo { .. } => "テンポ",
            Action::Chase { .. } => "チェイス",
            Action::Pair { .. } => "ペア",
            Action::Color { .. } => "色(全体制御)",
            Action::ColorScope { .. } => "色:全体/個別",
            Action::Select { .. } => "機材選択",
            Action::Position { .. } => "位置プリセット",
            Action::Page { .. } => "ページ切替",
            Action::Palette { .. } => "メイン/サブ色",
            Action::Flash { .. } => "フラッシュ",
            Action::Fade { .. } => "暗転フェード",
            Action::Level { .. } => "明るさ(機材ごと)",
            Action::Mute { .. } => "ミュート",
            Action::Pixel { .. } => "LEDバー調整",
            Action::Role { .. } => "色の系統",
            Action::Arm { .. } => "ARM",
            Action::LiveColor { .. } => "NDIの色",
            Action::SwapBeats { .. } => "入れ替えの間隔",
            Action::BarCount { .. } => "LEDバーの本数",
            Action::Motion { .. } => "ムービングの動き",
            Action::KeyPage { .. } => "キーボードの模様セット",
        }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize)]
pub struct Binding {
    /// Persistent identity for CLI controls; zero is assigned on first use/load.
    #[serde(default)]
    #[schemars(extend("readOnly"=true))]
    pub id: u64,
    pub name: String,
    pub trigger: Option<MidiTrigger>,
    pub action: Action,
    /// MIDIで反応するページ(0始まり)。None=全ページ共通
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u8>,
    /// OSCで動かすときのアドレス(例: /tap)。ページに関係なく反応する
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub osc: Option<String>,
}

impl Binding {
    pub fn on_page(&self, page: u8) -> bool {
        self.page.is_none_or(|p| p == page)
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum OutputKind {
    ArtNet { ip: String, art_universe: u16 },
    /// FTDI系USB-DMXをCOMポートとして使う
    OpenDmxSerial { port: String },
    /// FTDI系USB-DMXをD2XXドライバ(ftd2xx.dll)で直接使う
    OpenDmxD2xx { index: u32 },
}

impl OutputKind {
    pub fn label(&self) -> &'static str {
        match self {
            OutputKind::ArtNet { .. } => "Art-Net",
            OutputKind::OpenDmxSerial { .. } => "USB-DMX (COMポート)",
            OutputKind::OpenDmxD2xx { .. } => "USB-DMX (D2XX)",
        }
    }
}

#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OutputCfg {
    pub enabled: bool,
    /// このアプリ内のユニバース番号(0始まり)
    pub universe: usize,
    pub kind: OutputKind,
}

/// オートの設定
#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AutoCfg {
    pub enabled: bool,
    /// 曲の情報が届いていないとき(待機)の色(0〜1のRGB)。既定は薄い水色
    pub idle_rgb: [f32; 3],
    /// 待機の明るさ(0〜1)
    pub idle_level: f32,
    /// 曲の情報がこの秒数届かなければ待機にする
    pub timeout_secs: f32,
    /// フレーズに合わせて、プリセットの見た目(ムービング・色・明るさ・シーン・エフェクト)にする
    pub builtin: bool,
    /// MIDIコントローラー・キーボードで操作したら、この秒数はオートを止める(手動優先。0なら止めない)
    pub manual_secs: f32,
    /// フレーズの情報が来ない曲では、波形の大きさ(盛り上がり)で見た目を切り替える
    pub energy: bool,
    /// フレーズに入った瞬間の1拍の全開(プリセットで「入った瞬間に全開」にしたフレーズ)を使う
    pub accent: bool,
    /// フレーズごとの見た目のプリセット(イントロ・サビ…でどう動くか)。`preset` 番目を使う
    pub presets: Vec<AutoPreset>,
    pub preset: usize,
    /// Beat Link Trigger の気配(Linkの仲間・曲の情報)がこの秒数無ければ、「BLTが見つかりません」と知らせる(0なら知らせない)
    pub alert_secs: f32,
}

/// オートで扱うフレーズ(BLTが送る名前, 画面に出す名前)
pub const PHRASES: [(&str, &str); 7] = [
    ("intro", "イントロ"),
    ("verse", "バース"),
    ("bridge", "ブリッジ"),
    ("up", "ビルドアップ"),
    ("chorus", "サビ"),
    ("down", "ダウン"),
    ("outro", "アウトロ"),
];

/// フレーズの見た目のプリセット(名前と、フレーズごとの見た目)
#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AutoPreset {
    pub name: String,
    /// フレーズ(PHRASESの英語名) → 見た目
    pub looks: BTreeMap<String, PhraseLook>,
}

impl Default for AutoPreset {
    fn default() -> Self {
        AutoPreset::standard()
    }
}

/// メイン色/サブ色の動き方
#[derive(schemars::JsonSchema, Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ColorMove {
    /// 変えない(今のまま)
    #[default]
    Keep,
    /// 拍で入れ替える
    Swap,
    /// グラデーションで行き来する
    Fade,
}

/// 1つのフレーズでの見た目
#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PhraseLook {
    /// ムービングの形・1周の拍数・揃え方・向き(None=今の向きのまま)
    pub shape: crate::motion::Shape,
    pub beats: f32,
    pub spread: crate::motion::Spread,
    pub position: Option<u8>,
    /// メイン色/サブ色の動き方と、その間隔(拍)
    pub color: ColorMove,
    pub color_beats: f32,
    /// 明るさ(0〜1。全体光量に掛ける)
    pub brightness: f32,
    /// 波形の色(/dmx/main/rgb)を使う。OFFならパレットの色のまま
    pub wave_color: bool,
    /// 入った瞬間に1拍だけ今の色で全開
    pub flash: bool,
    /// 呼び出すシーン(空なら呼ばない)
    pub scene: String,
    /// ONにするエフェクト・チェイス・ピクセルエフェクト(次のフレーズで、そこに無いものはOFFに戻す)
    pub effects: Vec<String>,
}

impl Default for PhraseLook {
    fn default() -> Self {
        PhraseLook::new(crate::motion::Shape::Still, 8.0, crate::motion::Spread::Fan, ColorMove::Keep, 4.0, 1.0)
    }
}

impl PhraseLook {
    pub fn new(shape: crate::motion::Shape, beats: f32, spread: crate::motion::Spread, color: ColorMove, color_beats: f32, brightness: f32) -> Self {
        PhraseLook { shape, beats, spread, position: None, color, color_beats, brightness, wave_color: true, flash: false, scene: String::new(), effects: Vec::new() }
    }
}

impl AutoPreset {
    /// 標準: 盛り上がるところほど速く大きく、静かなところはゆっくり暗め
    pub fn standard() -> Self {
        use crate::motion::{Shape as S, Spread as P};
        use ColorMove::*;
        let mut looks = BTreeMap::new();
        let mut put = |k: &str, l: PhraseLook| {
            looks.insert(k.to_string(), l);
        };
        put("intro", PhraseLook::new(S::Circle, 16.0, P::Fan, Fade, 8.0, 0.6));
        put("verse", PhraseLook::new(S::Eight, 8.0, P::Fan, Swap, 4.0, 0.8));
        put("bridge", PhraseLook::new(S::PanSweep, 4.0, P::Mirror, Fade, 4.0, 0.8));
        put("up", PhraseLook::new(S::TiltSweep, 2.0, P::Sync, Swap, 2.0, 0.9));
        put("chorus", PhraseLook { flash: true, ..PhraseLook::new(S::Cross, 2.0, P::Mirror, Swap, 1.0, 1.0) });
        put("down", PhraseLook::new(S::Wave, 8.0, P::Fan, Fade, 8.0, 0.6));
        put("outro", PhraseLook::new(S::Circle, 16.0, P::Fan, Fade, 8.0, 0.6));
        AutoPreset { name: "標準".into(), looks }
    }

    /// 激しめ: 全体に速く、サビはランダムに1拍で動いて色も毎拍入れ替える
    pub fn intense() -> Self {
        use crate::motion::{Shape as S, Spread as P};
        use ColorMove::*;
        let mut p = AutoPreset::standard();
        p.name = "激しめ".into();
        let mut put = |k: &str, l: PhraseLook| {
            p.looks.insert(k.to_string(), l);
        };
        put("intro", PhraseLook::new(S::Circle, 8.0, P::Fan, Swap, 4.0, 0.7));
        put("verse", PhraseLook::new(S::Eight, 4.0, P::Mirror, Swap, 2.0, 0.9));
        put("up", PhraseLook { flash: false, ..PhraseLook::new(S::TiltSweep, 1.0, P::Sync, Swap, 1.0, 1.0) });
        put("chorus", PhraseLook { flash: true, ..PhraseLook::new(S::Random, 1.0, P::Fan, Swap, 1.0, 1.0) });
        p
    }
}

impl Default for AutoCfg {
    fn default() -> Self {
        AutoCfg { enabled: false, idle_rgb: [0.45, 0.8, 1.0], idle_level: 0.35, timeout_secs: 5.0, builtin: true, manual_secs: 120.0, energy: true, accent: true, presets: vec![AutoPreset::standard(), AutoPreset::intense()], preset: 0, alert_secs: 60.0 }
    }
}

/// 古い保存ファイルにも、後から増えた項目の既定値を補って読めるようにしている
#[derive(schemars::JsonSchema, Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    pub next_binding_id: u64,
    pub fixtures: Vec<FixtureInstance>,
    pub groups: Vec<Group>,
    pub effects: Vec<Effect>,
    pub scenes: Vec<Scene>,
    pub bindings: Vec<Binding>,
    pub outputs: Vec<OutputCfg>,
    /// 前回接続したMIDI入力ポート名
    pub midi_input: String,
    pub next_id: u32,
    /// 「全体光量」を右端のフェーダーに初期割り当てしたか(消しても復活させないための印)
    pub master_seeded: bool,
    /// 全体のテンポ
    #[schemars(range(min=20,max=300))]
    pub bpm: f32,
    pub chases: Vec<Chase>,
    pub pairs: Vec<Pair>,
    /// APC mini mk2 のパッドLEDを点灯するか
    pub led_enabled: bool,
    /// 待機中(割り当てがあって、効いていないとき)のLEDの明るさ: 0=10% / 1=25% / 2=50%
    pub led_idle: u8,
    /// 位置プリセット(パン/チルトだけのシーン)
    pub positions: Vec<Scene>,
    pub tempo_source: TempoSource,
    /// MIDIクロックを受け取るポート。空なら、メインのMIDI入力で受け取る
    pub clock_input: String,
    /// 音声からBPMを検出する入力機器(空なら既定の入力)
    pub audio_input: String,
    pub osc_enabled: bool,
    pub osc_port: u16,
    /// ブラウザ(スマホなど)の操作パネル(http://このPCのIP:web_port/)
    pub web_enabled: bool,
    pub web_port: u16,
    /// 操作パネルの合言葉(空なら無し。同じネットワークの誰でも操作できる)
    pub web_pin: String,
    /// OSCのRGBで受けた色へ近づける速さ(秒)
    pub osc_rgb_secs: f32,
    /// OSCで受けた明るさ(/dmx/level)の効き具合(0=効かない〜1=0まで暗くなる)
    pub osc_level_depth: f32,
    /// オート(Beat Link Triggerなどから曲の情報が届いている間は自動で動かし、届かなければ待機の色で光らせる)
    pub auto: AutoCfg,
    /// メイン色とサブ色
    pub palette: Palette,
    /// LEDバーのピクセルエフェクト(名前は通常のエフェクトと共通で、同じ名前ならまとめてON/OFF)
    pub pixel_fx: Vec<crate::pixel::PixelFx>,
    /// 暗転フェードの秒数(画面のボタン・キーボードのD)
    pub fade_secs: f32,
    /// 色を指定するパッドを、APC mini mk2 のフルカラー(SysEx)で実際の色に光らせる
    pub led_true_color: bool,
    /// NDI(映像の平均色・音の色)をメイン色/サブ色に使う設定
    pub ndi: crate::ndi::NdiCfg,
    /// ムービングの動き(形・1周の長さ・揃え方・向き)
    pub motion: crate::motion::Motion,
}

impl Default for Project {
    fn default() -> Self {
        Project {
            next_binding_id: 1,
            fixtures: Vec::new(),
            groups: Vec::new(),
            effects: Vec::new(),
            scenes: Vec::new(),
            bindings: Vec::new(),
            outputs: Vec::new(),
            midi_input: String::new(),
            next_id: 1,
            master_seeded: false,
            bpm: 120.0,
            chases: Vec::new(),
            pairs: Vec::new(),
            led_enabled: true,
            led_idle: 1,
            positions: Vec::new(),
            tempo_source: TempoSource::Internal,
            clock_input: String::new(),
            audio_input: String::new(),
            osc_enabled: false,
            osc_port: 9000,
            web_enabled: false,
            web_port: 8080,
            web_pin: String::new(),
            osc_rgb_secs: 0.5,
            osc_level_depth: 0.7,
            auto: AutoCfg::default(),
            palette: Palette::default(),
            pixel_fx: Vec::new(),
            fade_secs: 3.0,
            led_true_color: true,
            ndi: crate::ndi::NdiCfg::default(),
            motion: crate::motion::Motion::default(),
        }
    }
}

impl Project {
    pub fn ensure_binding_ids(&mut self) {
        use std::collections::HashSet;
        let mut used = HashSet::new();
        let highest = self.bindings.iter().map(|b| b.id).filter(|&id| id < u64::MAX).max().unwrap_or(0);
        self.next_binding_id = self.next_binding_id.max(highest + 1).max(1);
        for binding in &mut self.bindings {
            if binding.id == 0 || binding.id == u64::MAX || !used.insert(binding.id) {
                while used.contains(&self.next_binding_id) || self.next_binding_id == u64::MAX {
                    self.next_binding_id = self.next_binding_id.wrapping_add(1).max(1);
                }
                binding.id = self.next_binding_id;
                used.insert(binding.id);
                self.next_binding_id = self.next_binding_id.wrapping_add(1).max(1);
            }
        }
    }

    pub fn load(path: &Path) -> anyhow::Result<Project> {
        Ok(Project::load_lenient(path)?.0)
    }

    /// 読む。新しい版で保存したファイル(この版が知らない動きの形などがある)でも、
    /// 読めない項目だけを既定値にして(配列なら、その要素を飛ばして)読む。飛ばした項目の場所も返す
    pub fn load_lenient(path: &Path) -> anyhow::Result<(Project, Vec<String>)> {
        let s = std::fs::read_to_string(path)?;
        // メモ帳などで保存されたBOM付きUTF-8も読めるようにする
        Project::from_json_lenient(s.trim_start_matches('\u{feff}'))
    }

    pub fn from_json_lenient(s: &str) -> anyhow::Result<(Project, Vec<String>)> {
        // JSONとして壊れているものは読まない(どこを飛ばせばよいか分からないので)
        let mut v: serde_json::Value = serde_json::from_str(s)?;
        let mut skipped = Vec::new();
        for _ in 0..200 {
            match serde_path_to_error::deserialize::<_, Project>(&v) {
                Ok(p) => return Ok((p, skipped)),
                Err(e) => {
                    let path = e.path().clone();
                    if !remove_at(&mut v, &path) {
                        return Err(e.into_inner().into());
                    }
                    // 中身を飛ばしても読めず、それを含む項目ごと飛ばしたときは、外側だけを記録する
                    let p = path.to_string();
                    skipped.retain(|s: &String| !(s.starts_with(&p) && matches!(s.as_bytes().get(p.len()), Some(b'.' | b'['))));
                    skipped.push(p);
                }
            }
        }
        anyhow::bail!("読めない項目が多すぎます")
    }

    /// 読めなかった保存ファイルを、自動保存で上書きされる前に別名で退避する
    pub fn backup_broken(path: &Path) -> Option<PathBuf> {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let dest = path.with_extension(format!("broken-{stamp}.json"));
        std::fs::copy(path, &dest).ok().map(|_| dest)
    }

    /// 書き込み途中で壊れないよう、一時ファイル経由で置き換える
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn fixture_index(&self, id: u32) -> Option<usize> {
        self.fixtures.iter().position(|f| f.id == id)
    }

    /// 初回起動用: 手持ちの2機材を並べた状態
    pub fn starter(lib: &Library) -> Project {
        let mut p = Project::default();
        p.add_fixture(lib, "stage-evolution/minispot30");
        p.add_fixture(lib, "stage-evolution/laserscan300");
        p.seed_master_binding();
        p
    }

    /// 全体光量の初期割り当て: 右端のフェーダー(APC mini mk2 のマスターフェーダー = CC 0x38)
    pub fn default_master_binding() -> Binding {
        Binding { id: 0,
            name: "全体光量".into(),
            trigger: Some(MidiTrigger { kind: TriggerKind::Cc, channel: None, number: 0x38 }),
            action: Action::Master { mode: PadMode::Continuous, on: 100.0, off: 0.0 },
            page: None,
            osc: None,
        }
    }

    /// 全体光量の割り当てを、まだ作っていなければ右端のフェーダーに作る(一度作ったら、消しても復活しない)。
    /// None=処理済み / Some(true)=作った / Some(false)=右端のフェーダーが別の用途で使われているので作らなかった
    pub fn seed_master_binding(&mut self) -> Option<bool> {
        if self.master_seeded {
            return None;
        }
        self.master_seeded = true;
        if self.bindings.iter().any(|b| matches!(b.action, Action::Master { .. })) {
            return None;
        }
        if self.bindings.iter().any(|b| b.trigger.is_some_and(|t| t.kind == TriggerKind::Cc && t.number == 0x38)) {
            return Some(false);
        }
        self.bindings.push(Self::default_master_binding());
        Some(true)
    }

    /// ユニバース1の空いているアドレスに機材を追加する
    pub fn add_fixture(&mut self, lib: &Library, profile: &str) -> Option<u32> {
        let prof = lib.get(profile)?;
        let mode = prof.modes.first()?;
        let n = mode.channels.len();
        let mut used = [false; 512];
        for f in self.fixtures.iter().filter(|f| f.universe == 0) {
            if let Some(m) = lib.mode(&f.profile, f.mode) {
                for c in 0..m.channels.len() {
                    if let Some(u) = used.get_mut(f.address as usize - 1 + c) {
                        *u = true;
                    }
                }
            }
        }
        let address = (0..=512usize.saturating_sub(n)).find(|&a| !used[a..a + n].iter().any(|&u| u)).unwrap_or(0) + 1;
        let id = self.next_id;
        self.next_id += 1;
        let same = self.fixtures.iter().filter(|f| f.profile == profile).count();
        self.fixtures.push(FixtureInstance {
            id,
            name: format!("{} {}", prof.name, same + 1),
            profile: profile.into(),
            mode: 0,
            universe: 0,
            address: address as u16,
            values: mode.default_values(),
            armed: false,
            color_role: ColorRole::Main,
            enabled: true,
            limits: BTreeMap::new(),
        });
        Some(id)
    }
}

/// `path` の場所の項目を消す(構造体の項目なら既定値に、配列なら要素ごと)。
/// 最後が「enumの種類」などで消せなければ、1つ上を消す。消せたら true
fn remove_at(v: &mut serde_json::Value, path: &serde_path_to_error::Path) -> bool {
    use serde_path_to_error::Segment;
    let segs: Vec<&Segment> = path.iter().collect();
    // 消せる場所(オブジェクトの項目・配列の要素)まで、末尾から戻る
    let Some(last) = segs.iter().rposition(|s| matches!(s, Segment::Map { .. } | Segment::Seq { .. })) else { return false };
    let mut cur = v;
    for s in &segs[..last] {
        cur = match s {
            Segment::Map { key } => match cur.get_mut(key.as_str()) {
                Some(x) => x,
                None => return false,
            },
            Segment::Seq { index } => match cur.get_mut(*index) {
                Some(x) => x,
                None => return false,
            },
            // enumの中身(外側のキーが種類名のオブジェクト)は、そのまま下りる
            Segment::Enum { variant } => {
                if cur.get(variant.as_str()).is_some() {
                    cur.get_mut(variant.as_str()).expect("直前に確かめた")
                } else {
                    cur
                }
            }
            Segment::Unknown => return false,
        };
    }
    match segs[last] {
        Segment::Map { key } => cur.as_object_mut().is_some_and(|o| o.remove(key.as_str()).is_some()),
        Segment::Seq { index } => cur.as_array_mut().is_some_and(|a| {
            let ok = *index < a.len();
            if ok {
                a.remove(*index);
            }
            ok
        }),
        _ => false,
    }
}

/// バックアップを残す数
pub const BACKUP_KEEP: usize = 20;

pub fn backup_dir() -> PathBuf {
    config_dir().join("backups")
}

/// 今の日時(このPCの時刻)。ファイル名用「20260928-143005」
fn stamp() -> String {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct SystemTime {
            year: u16,
            month: u16,
            dow: u16,
            day: u16,
            hour: u16,
            minute: u16,
            second: u16,
            ms: u16,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetLocalTime(t: *mut SystemTime);
        }
        let mut t = SystemTime::default();
        unsafe { GetLocalTime(&mut t) };
        format!("{:04}{:02}{:02}-{:02}{:02}{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second)
    }
    #[cfg(unix)]
    {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as libc::time_t;
        // SAFETY: localtime_r は結果を渡したバッファに書くだけ(スレッドセーフ版)
        let mut t: libc::tm = unsafe { std::mem::zeroed() };
        unsafe { libc::localtime_r(&now, &mut t) };
        format!("{:04}{:02}{:02}-{:02}{:02}{:02}", t.tm_year + 1900, t.tm_mon + 1, t.tm_mday, t.tm_hour, t.tm_min, t.tm_sec)
    }
    #[cfg(not(any(windows, unix)))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string()
    }
}

/// 保存ファイルを`dir`にコピーして残す(起動時・別のファイルを開く前)。古いものは`keep`個まで残して消す
pub fn backup_to(path: &Path, dir: &Path, keep: usize) -> Option<PathBuf> {
    if !path.exists() {
        return None;
    }
    std::fs::create_dir_all(dir).ok()?;
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("project");
    let mut dest = dir.join(format!("{stem}-{}.json", stamp()));
    let mut n = 2;
    while dest.exists() {
        dest = dir.join(format!("{stem}-{}-{n}.json", stamp()));
        n += 1;
    }
    std::fs::copy(path, &dest).ok()?;
    // 古いものから消す(このアプリが作ったバックアップだけ)
    for p in list_backups(dir, stem).into_iter().skip(keep) {
        std::fs::remove_file(p).ok();
    }
    Some(dest)
}

/// `dir`にある、保存ファイル`stem`のバックアップ(このアプリが作ったもの「stem-日付-時刻.json」)。新しい順
pub fn list_backups(dir: &Path, stem: &str) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let prefix = format!("{stem}-");
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json") && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix)))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    files.sort_by(|a, b| b.cmp(a));
    files.into_iter().map(|(_, p)| p).collect()
}

/// 設定の保存場所。Windows: %APPDATA%\dmxctl / Mac: ~/Library/Application Support/dmxctl / Linux: ~/.config/dmxctl
pub fn config_dir() -> PathBuf {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    // 開発・確認用: 本番の設定に触れずに別の設定で起動する
    if let Some(dir) = env("DMXCTL_CONFIG_DIR") {
        return dir;
    }
    let base = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))
    };
    base.unwrap_or_else(|| PathBuf::from(".")).join("dmxctl")
}

pub fn default_project_path() -> PathBuf {
    config_dir().join("project.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_files_load_by_skipping_what_this_version_does_not_know() {
        // 新しい版で保存したファイル: 知らない動きの形と、知らない種類の割り当てがある
        let mut p = Project { bpm: 133.0, ..Project::default() };
        p.bindings.push(Binding { id: 0, name: "ok".into(), trigger: None, action: Action::Blackout { mode: PadMode::Toggle }, page: None, osc: None });
        p.bindings.push(Binding { id: 0, name: "new".into(), trigger: None, action: Action::Blackout { mode: PadMode::Toggle }, page: None, osc: None });
        let mut v = serde_json::to_value(&p).unwrap();
        v["motion"]["shape"] = "Hyperspace".into();
        v["bindings"][1]["action"] = serde_json::json!({"Teleport": {"to": 3}});
        let (q, skipped) = Project::from_json_lenient(&v.to_string()).unwrap();
        assert_eq!(q.bpm, 133.0, "ほかの項目はそのまま");
        assert_eq!(q.motion.shape, crate::motion::Shape::Still, "知らない形は既定値");
        assert_eq!(q.bindings.len(), 1, "知らない割り当ては飛ばす");
        assert_eq!(q.bindings[0].name, "ok");
        assert_eq!(skipped.len(), 2, "{skipped:?}");
        // 今の版のファイルは何も飛ばさない
        let (_, none) = Project::from_json_lenient(&serde_json::to_string(&p).unwrap()).unwrap();
        assert!(none.is_empty());
        // JSONとして壊れているものは読まない
        assert!(Project::from_json_lenient("{ broken").is_err());
    }

    #[test]
    fn project_roundtrip() {
        let mut p = Project::default();
        p.effects.push(Effect::new("test", "pan"));
        p.bindings.push(Binding { id: 0,
            name: "pad".into(),
            trigger: Some(MidiTrigger { kind: TriggerKind::Note, channel: Some(9), number: 36 }),
            action: Action::SetValue { sel: Selector::Profile("x".into()), attr: "dimmer".into(), mode: PadMode::Toggle, on: 255.0, off: 0.0 },
            page: None,
            osc: None,
        });
        p.outputs.push(OutputCfg { enabled: true, universe: 0, kind: OutputKind::ArtNet { ip: "127.0.0.1".into(), art_universe: 0 } });
        let s = serde_json::to_string(&p).unwrap();
        let q: Project = serde_json::from_str(&s).unwrap();
        assert_eq!(q.bindings.len(), 1);
        assert_eq!(q.outputs[0].kind, p.outputs[0].kind);
    }

    #[test]
    fn load_accepts_bom_and_backup_preserves_broken_file() {
        let dir = std::env::temp_dir().join(format!("dmxctl-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = dir.join("bom.json");
        std::fs::write(&ok, format!("\u{feff}{}", serde_json::to_string(&Project::default()).unwrap())).unwrap();
        assert!(Project::load(&ok).is_ok());

        let bad = dir.join("bad.json");
        std::fs::write(&bad, "{ not json").unwrap();
        assert!(Project::load(&bad).is_err());
        let backup = Project::backup_broken(&bad).expect("backup");
        assert_eq!(std::fs::read_to_string(backup).unwrap(), "{ not json");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn waves_stay_in_unit_range() {
        for w in Wave::ALL {
            for i in 0..=100 {
                let v = w.eval(i as f32 / 100.0, i as u64, 3);
                assert!((0.0..=1.0).contains(&v), "{w:?} -> {v}");
            }
        }
    }
}
