//! 実行中の状態 `Runtime`。保存しない値と、出力ループが使う値をまとめる。
use super::*;

pub struct Runtime {
    pub audit_depth: usize,
    pub trace: Option<crate::trace::Trace>,
    pub trace_ticket: u64,
    pub revision: u64,
    pub revision_hash: Option<u64>,
    pub session: String,
    pub physical_output_enabled: bool,
    pub project_path: Option<std::path::PathBuf>,
    pub remote_holds: HashMap<u64, crate::hold::Hold>,
    pub frame_id: u64,
    /// Deadline for remote momentary controls; enforced by the output thread.
    pub flash_deadline: Option<Instant>,
    /// Automation can supply a monotonic clock without changing the GUI clock.
    pub virtual_now: Option<Instant>,
    pub recorder: Option<crate::recorder::Recorder>,
    pub safety_events: Vec<serde_json::Value>,
    pub blackout: bool,
    pub fades: Vec<Fade>,
    /// バインディングごとの「押されている」状態(エッジ検出用)
    pub pad_state: HashMap<usize, bool>,
    /// トグル動作のバインディングの現在状態
    pub toggle_state: HashMap<usize, bool>,
    /// 全体光量(0.0〜1.0)。ディマーにかかる。保存しない(起動時は全開)
    pub master: f32,
    /// 拍の位置(0から連続して増える。`BEATS_PER_BAR`拍で1小節)
    pub beat: f64,
    pub last_beat_t: Option<Instant>,
    /// タップテンポ用に、直近のタップの時刻
    pub taps: Vec<Instant>,
    /// 動作中のチェイスが今何ステップ目(通し番号)か。ステップ切り替えの検出と、画面の表示に使う
    pub chase_idx: HashMap<String, i64>,
    /// 最後に呼び出したシーン(パッドの点灯表示用)
    pub last_scene: Option<String>,
    pub led_status: String,
    /// 色の操作先: true=全体(全機材) / false=個別(選択中の機材)。保存しない
    pub color_global: bool,
    /// 個別モードで色を掛ける機材(id)
    pub selected: HashSet<u32>,
    /// 最後に選んだ色(パッドの点灯表示用)
    pub last_color: Option<NamedColor>,
    /// 色相フェーダー(HSVのH)の現在値(0〜360度)。画面のスライダーと表示用
    pub hue: f32,
    pub learn: Option<usize>,
    pub last_midi: String,
    pub midi_status: String,
    /// 今押されているノート(マップ表示用。チャンネルは区別しない)
    pub notes_down: [bool; 128],
    /// CCの最後に受け取った値(マップ表示用。まだ動かしていないフェーダーはNone)
    pub cc_value: [Option<u8>; 128],
    pub frames: Vec<[u8; 512]>,
    pub used_len: Vec<usize>,
    pub output_status: Vec<String>,
    /// 出力設定を作り直す合図
    pub output_gen: u64,
    pub t0: Instant,
    /// 今のパッドのページ(0始まり)
    pub page: u8,
    pub link: Option<crate::io::link::Link>,
    pub link_peers: u64,
    pub clock: ClockIn,
    /// 最後に呼び出した位置プリセット(パッドの点灯表示用)
    pub last_position: Option<String>,
    /// OSCラーン: 次に来たOSCアドレスを、この割り当てに登録する
    pub osc_learn: Option<usize>,
    pub last_osc: String,
    pub osc_status: String,
    pub audio_status: String,
    /// 音声検出の確からしさ(0〜1)と、最後に採用した時刻
    pub audio_conf: f32,
    pub audio_pending: f32,
    pub audio_seen: Option<Instant>,
    /// メイン色とサブ色を入れ替えているか(自動入れ替えとは別。保存しない)
    pub color_swapped: bool,
    /// 最後にメイン/サブの色を機材に出したときの「入れ替え」の状態。まだ一度も出していなければNone
    pub palette_shown: Option<bool>,
    /// フラッシュ中なら Some(白で全開か)
    pub flash: Option<bool>,
    /// 暗転フェードの明るさ(1=通常 / 0=暗転)。全体光量にかかる
    pub dim: f32,
    /// 暗転フェードの向かう先(0か1)と、かける秒数
    pub dim_target: f32,
    pub dim_secs: f32,
    /// 機材ごとの明るさ(0.0〜1.0。「明るさ」のフェーダーで変える)。無い機材は1.0。全体光量と同じところに掛かる。保存しない
    pub level: HashMap<u32, f32>,
    /// ミュート中の機材(明るさのチャンネルを0にする)。保存しない
    pub muted: HashSet<u32>,
    /// ピクセルエフェクト全体の速さの倍率と、倍率を変えても模様が飛ばないようにするための拍のずれ
    pub px_speed: f32,
    pub px_offset: f64,
    /// ピクセルエフェクトに足す斜めのずれ(周)
    pub px_skew: f32,
    /// NDIの受信結果(映像の平均色・音の色)
    pub ndi: crate::io::ndi::NdiState,
    /// NDIから作った、今のメイン色/サブ色(NDIを使っていなければNone)。保存しない
    pub live_main: Option<[f32; 3]>,
    pub live_sub: Option<[f32; 3]>,
    /// OSCのRGB(/dmx/main/rgb・/dmx/sub/rgb)で受けた色。[メイン, サブ]。保存しない
    pub osc_rgb: [Option<OscRgb>; 2],
    /// OSCで受けた明るさ(/dmx/level。波形の大きさなど)。全体光量に掛ける。保存しない
    pub osc_level: Option<OscGlide<1>>,
    /// オート: 最後に曲の情報(色・明るさ・フレーズ)が届いた時刻
    pub auto_last: Option<Instant>,
    /// オート: 前のフレームで、つながっていたか
    pub auto_was_online: bool,
    /// オート: 待機の見た目(薄い水色)の混ぜ具合(0=本番〜1=待機)
    pub idle_mix: f32,
    /// オート: 最後にMIDIコントローラー・キーボードで操作した時刻(手動優先)
    pub manual_last: Option<Instant>,
    /// オート: 今使っている組み込みの動き(フレーズの名前)
    pub auto_key: String,
    /// オート: 最後にフレーズが届いたときの拍(フレーズの情報が来ない曲かどうか)
    pub phrase_beat: Option<f64>,
    /// オート: 盛り上がり(OSCの明るさを数秒でならしたもの。0〜1)と、最後に見た16拍のかたまり
    pub energy: f32,
    pub energy_block: i64,
    /// オート: 最後に受けた明るさ(盛り上がりの元。手動優先の間も覚えておく)
    pub energy_input: Option<f32>,
    /// オート: サビに入った瞬間の全開を、この拍で戻す
    pub accent_until: Option<f64>,
    /// オート: プリセットの明るさ(全体光量に掛ける)
    pub auto_bright: Option<OscGlide<1>>,
    /// オート: プリセットで「波形の色を使わない」にしているフレーズの間
    pub wave_color_off: bool,
    /// オート: プリセットでONにしたエフェクト(次のフレーズでOFFに戻すため)
    pub auto_fx: Vec<String>,
    /// オート: 最後に Beat Link Trigger の気配(Linkの仲間がいる・曲の情報が届いた)があった時刻。起動時は起動した時刻
    pub blt_seen: Instant,
    /// 最後に受けたフレーズ(/dmx/phrase。Beat Link Triggerなどから)
    pub phrase: String,
    /// ムービングの動きの位置(1で1周)と、それを進めるための前回の拍
    pub motion_phase: f64,
    pub motion_beat: Option<f64>,
    /// ムービングごとの、前回のPanの角度(度)。次の向きの選び方に使う
    pub motion_pan: HashMap<u32, f64>,
    /// キーボードで押しているキー(crate::keys の番号)
    pub keys_down: Vec<u8>,
    /// キーボードの数字の段に出す、LEDバーの模様のセット
    pub key_page: u8,
    /// サーバーにつないだクライアント(`dmxctl connect`)では、操作をサーバーへ送る口。
    /// このときの Core はサーバーの写しで、出力ループも外部との連携も動かさない
    pub remote: Option<std::sync::Arc<dyn crate::live::LiveForward>>,
    /// クライアントでの表示用: サーバーで実行中のフェードの数・Ableton Linkが動いているか
    pub remote_fades: usize,
    pub remote_link: bool,
    /// クライアントでの表示用: サーバーが安全停止中か(このプロセスの EMERGENCY とは別)
    pub remote_emergency: bool,
}

impl Runtime {
    pub(super) fn new() -> Self {
        Runtime {
            audit_depth: 0,
            blackout: false,
            fades: Vec::new(),
            pad_state: HashMap::new(),
            toggle_state: HashMap::new(),
            master: 1.0,
            beat: 0.0,
            last_beat_t: None,
            taps: Vec::new(),
            chase_idx: HashMap::new(),
            last_scene: None,
            led_status: String::new(),
            color_global: true,
            selected: HashSet::new(),
            last_color: None,
            hue: 0.0,
            learn: None,
            last_midi: String::new(),
            midi_status: "未接続".into(),
            notes_down: [false; 128],
            cc_value: [None; 128],
            frames: vec![[0; 512]; NUM_UNIVERSES],
            used_len: vec![MIN_SLOTS; NUM_UNIVERSES],
            output_status: Vec::new(),
            output_gen: 0,
            frame_id: 0,
            trace: None,
            trace_ticket: 0,
            revision: 0,
            revision_hash: None,
            session: format!("{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos()),
            physical_output_enabled: false,
            project_path: None,
            remote_holds: HashMap::new(),
            remote: None,
            remote_fades: 0,
            remote_link: false,
            remote_emergency: false,
            flash_deadline: None,
            virtual_now: None,
            recorder: None,
            safety_events: Vec::new(),
            t0: Instant::now(),
            page: 0,
            link: None,
            link_peers: 0,
            clock: ClockIn::default(),
            last_position: None,
            osc_learn: None,
            last_osc: String::new(),
            osc_status: "停止中".into(),
            audio_status: "停止中".into(),
            audio_conf: 0.0,
            audio_pending: 0.0,
            audio_seen: None,
            color_swapped: false,
            palette_shown: None,
            flash: None,
            dim: 1.0,
            dim_target: 1.0,
            dim_secs: 3.0,
            level: HashMap::new(),
            muted: HashSet::new(),
            px_speed: 1.0,
            px_offset: 0.0,
            px_skew: 0.0,
            ndi: crate::io::ndi::NdiState::default(),
            live_main: None,
            live_sub: None,
            osc_rgb: [None, None],
            osc_level: None,
            auto_last: None,
            auto_was_online: false,
            idle_mix: 0.0,
            manual_last: None,
            auto_key: String::new(),
            phrase_beat: None,
            energy: 0.0,
            energy_block: i64::MIN,
            energy_input: None,
            accent_until: None,
            auto_bright: None,
            wave_color_off: false,
            auto_fx: Vec::new(),
            blt_seen: Instant::now(),
            phrase: String::new(),
            motion_phase: 0.0,
            motion_beat: None,
            motion_pan: HashMap::new(),
            keys_down: Vec::new(),
            key_page: 0,
        }
    }
}
