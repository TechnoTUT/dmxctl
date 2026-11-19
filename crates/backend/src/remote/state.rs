//! 実行中の状態を、サーバーから送れる形のJSONにする・クライアントで写しに反映する。
use super::*;

/// 画面の表示に使う実行中の状態
pub(super) fn rt_state(c: &Core) -> Value {
    let rt = &c.rt;
    let mut v = json!({
        "blackout": rt.blackout,
        "master": rt.master,
        "page": rt.page,
        "key_page": rt.key_page,
        "color_global": rt.color_global,
        "selected": sorted_ids(&rt.selected),
        "muted": sorted_ids(&rt.muted),
        "level": sorted_pairs(&rt.level),
        "learn": rt.learn,
        "osc_learn": rt.osc_learn,
        "last_position": rt.last_position,
        "last_color": rt.last_color,
        "toggle_state": sorted_pairs(&rt.toggle_state),
        "pad_state": sorted_pairs(&rt.pad_state),
        // ARMは保存しない項目(プロジェクトのJSONに入らない)なので、ここで配る
        "armed": c.project.fixtures.iter().filter(|f| f.armed).map(|f| f.id).collect::<Vec<_>>(),
    });
    // json! は項目が多いと展開しきれないので、2つに分けて合わせる
    let more = json!({
        "beat": rt.beat,
        "beat_age": age(rt.last_beat_t),
        "link_peers": rt.link_peers,
        "link": rt.link.as_ref().is_some_and(|l| l.connected()),
        "osc_status": rt.osc_status,
        "audio_status": rt.audio_status,
        "audio_conf": rt.audio_conf,
        "audio_seen": age(rt.audio_seen),
        "ndi": {"sources": rt.ndi.sources, "status": rt.ndi.status, "audio": rt.ndi.audio, "video": rt.ndi.video},
        "phrase": rt.phrase,
        "auto_key": rt.auto_key,
        "energy": rt.energy,
        "idle_mix": rt.idle_mix,
        "notes_down": (0..128u8).filter(|&n| rt.notes_down[n as usize]).collect::<Vec<_>>(),
        "cc_value": rt.cc_value.to_vec(),
        "used_len": rt.used_len,
        "output_status": rt.output_status,
        "last_midi": rt.last_midi,
        "last_osc": rt.last_osc,
        "chase_idx": rt.chase_idx,
        "last_scene": rt.last_scene,
        "color_swapped": rt.color_swapped,
        "dim": rt.dim,
        "dim_target": rt.dim_target,
        "flash": rt.flash,
        "hue": rt.hue,
        "px_speed": rt.px_speed,
        "osc_rgb": rt.osc_rgb.map(|o| o.map(|o| o.current)),
        "osc_level": rt.osc_level.map(|o| o.current[0]),
        "live_main": rt.live_main,
        "live_sub": rt.live_sub,
        "keys_down": rt.keys_down,
        "fades": rt.fades.len(),
        "physical_output": rt.physical_output_enabled,
        "auto_age": age(rt.auto_last),
        "blt_age": age(Some(rt.blt_seen)),
        "manual_age": age(rt.manual_last),
        "clock_age": age(rt.clock.last),
        "emergency": EMERGENCY.load(Ordering::SeqCst),
        "dirty": c.dirty,
        "frame": rt.frame_id,
    });
    if let (Some(a), Value::Object(b)) = (v.as_object_mut(), more) {
        a.extend(b);
    }
    v["laser"] = json!({"latch": rt.laser_latch, "hold": rt.laser_hold, "auto": rt.auto_laser, "auto_online": rt.auto_was_online});
    v
}

pub(super) fn sorted_ids(set: &std::collections::HashSet<u32>) -> Vec<u32> {
    let mut v: Vec<u32> = set.iter().copied().collect();
    v.sort();
    v
}

pub(super) fn glide<const N: usize>(current: [f32; N]) -> dmxctl_core::engine::OscGlide<N> {
    dmxctl_core::engine::OscGlide { target: current, current, secs: 0.0, due: None, last: Instant::now() }
}

pub(super) fn val<T: DeserializeOwned>(v: &Value, k: &str, old: T) -> T {
    v.get(k).and_then(|x| serde_json::from_value(x.clone()).ok()).unwrap_or(old)
}

/// 届いた状態を手元の写しに入れる。ラーンの状態だけは別(画面で書き換えるので、フレームの境目で入れる)
pub(super) fn apply_rt_state(c: &mut Core, d: &Value) {
    let rt = &mut c.rt;
    rt.blackout = val(d, "blackout", rt.blackout);
    rt.master = val::<f32>(d, "master", rt.master).clamp(0.0, 1.0);
    rt.page = val::<u8>(d, "page", rt.page).min(PAGES - 1);
    rt.key_page = val(d, "key_page", rt.key_page);
    rt.color_global = val(d, "color_global", rt.color_global);
    rt.selected = val::<Vec<u32>>(d, "selected", Vec::new()).into_iter().collect();
    rt.muted = val::<Vec<u32>>(d, "muted", Vec::new()).into_iter().collect();
    rt.level = val::<Vec<(u32, f32)>>(d, "level", Vec::new()).into_iter().collect();
    rt.last_position = val(d, "last_position", None);
    rt.last_color = val(d, "last_color", None);
    rt.toggle_state = val::<Vec<(usize, bool)>>(d, "toggle_state", Vec::new()).into_iter().collect();
    rt.pad_state = val::<Vec<(usize, bool)>>(d, "pad_state", Vec::new()).into_iter().collect();
    rt.beat = val(d, "beat", rt.beat);
    rt.last_beat_t = from_age(&d["beat_age"]);
    rt.link_peers = val(d, "link_peers", 0);
    rt.remote_link = val(d, "link", false);
    rt.osc_status = val(d, "osc_status", String::new());
    rt.audio_status = val(d, "audio_status", String::new());
    rt.audio_conf = val(d, "audio_conf", 0.0);
    rt.audio_seen = from_age(&d["audio_seen"]);
    rt.ndi.sources = val(&d["ndi"], "sources", Vec::new());
    rt.ndi.status = val(&d["ndi"], "status", String::new());
    rt.ndi.audio = val(&d["ndi"], "audio", None);
    rt.ndi.video = val(&d["ndi"], "video", None);
    rt.phrase = val(d, "phrase", String::new());
    rt.auto_key = val(d, "auto_key", String::new());
    rt.energy = val(d, "energy", 0.0);
    rt.idle_mix = val(d, "idle_mix", 0.0);
    rt.notes_down = [false; 128];
    for n in val::<Vec<u8>>(d, "notes_down", Vec::new()) {
        rt.notes_down[(n & 0x7F) as usize] = true;
    }
    for (i, v) in val::<Vec<Option<u8>>>(d, "cc_value", Vec::new()).into_iter().take(128).enumerate() {
        rt.cc_value[i] = v;
    }
    for (i, v) in val::<Vec<usize>>(d, "used_len", Vec::new()).into_iter().enumerate() {
        if let Some(x) = rt.used_len.get_mut(i) {
            *x = v.min(512);
        }
    }
    rt.output_status = val(d, "output_status", Vec::new());
    rt.last_midi = val(d, "last_midi", String::new());
    rt.last_osc = val(d, "last_osc", String::new());
    rt.chase_idx = val(d, "chase_idx", HashMap::new());
    rt.last_scene = val(d, "last_scene", None);
    rt.color_swapped = val(d, "color_swapped", false);
    rt.dim = val(d, "dim", 1.0);
    rt.laser_latch = val(&d["laser"], "latch", false);
    rt.laser_hold = val(&d["laser"], "hold", false);
    rt.auto_laser = val(&d["laser"], "auto", false);
    rt.auto_was_online = val(&d["laser"], "auto_online", rt.auto_was_online);
    rt.dim_target = val(d, "dim_target", 1.0);
    rt.flash = val(d, "flash", None);
    rt.hue = val(d, "hue", rt.hue);
    rt.px_speed = val(d, "px_speed", rt.px_speed);
    rt.osc_rgb = val::<[Option<[f32; 3]>; 2]>(d, "osc_rgb", [None, None]).map(|o| o.map(glide));
    rt.osc_level = val::<Option<f32>>(d, "osc_level", None).map(|v| glide([v]));
    rt.live_main = val(d, "live_main", None);
    rt.live_sub = val(d, "live_sub", None);
    rt.keys_down = val(d, "keys_down", Vec::new());
    rt.remote_fades = val(d, "fades", 0);
    // 実機に出しているか(サーバーの起動のしかた)。画面の「シミュレーション中」の表示に使う
    rt.physical_output_enabled = val(d, "physical_output", false);
    rt.auto_last = from_age(&d["auto_age"]);
    rt.blt_seen = from_age(&d["blt_age"]).unwrap_or_else(Instant::now);
    rt.manual_last = from_age(&d["manual_age"]);
    rt.clock.last = from_age(&d["clock_age"]);
    rt.frame_id = val(d, "frame", rt.frame_id);
    // このプロセスの EMERGENCY(終了のシグナルなど)は上書きしない
    rt.remote_emergency = val(d, "emergency", false);
    c.dirty = val(d, "dirty", false);
    apply_armed(c, d);
}

pub(super) fn apply_armed(c: &mut Core, d: &Value) {
    let ids: Vec<u32> = val(d, "armed", Vec::new());
    for f in c.project.fixtures.iter_mut() {
        f.armed = ids.contains(&f.id);
    }
}

pub(super) fn frames_json(frames: &[[u8; 512]], universes: impl Iterator<Item = usize>) -> Value {
    Value::Object(universes.map(|u| (u.to_string(), Value::String(hex(&frames[u])))).collect())
}

pub(super) fn library_json(lib: &Library) -> (Value, Value) {
    (json!(lib.profiles.iter().map(|p| p.as_ref()).collect::<Vec<_>>()), json!(lib.builtin_ids()))
}

/// サーバーのPCの機器の一覧(出力のシリアルポート・D2XX・音声入力・MIDIクロックの入力を選ぶのに使う。
/// これらはサーバーで開くので、クライアントのPCの一覧ではなく、こちらを出す)
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Devices {
    pub serial: Vec<(String, String)>,
    pub d2xx: Result<Vec<String>, String>,
    pub audio: Vec<String>,
    pub midi: Vec<String>,
}

impl Devices {
    pub(super) fn list() -> Self {
        Devices { serial: dmxctl_core::io::opendmx::list_serial_ports(), d2xx: dmxctl_core::io::opendmx::list_d2xx(), audio: dmxctl_core::io::audio::input_devices(), midi: dmxctl_core::io::midi::list_ports() }
    }
}

