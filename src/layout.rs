//! おすすめのMIDI配置(AKAI APC mini mk2)。マップの「配置」メニュー・初回起動・`--apply-layout` で使う。
//!
//! パッド(上から): 1〜4段=LEDバーの模様(チル/グルーヴ/ピーク/モノクロ) / 5段=ムービングなど他の機材 /
//! 6段=メイン色 / 7段=サブ色 / 8段=押している間だけ効くアクセント(フラッシュ・ストロボなど)
//! 右の列(シーンボタン): TAP・頭出し・メイン⇄サブ・自動入れ替え・暗転フェード・BLACKOUT
//! 下の列(トラックボタン): LEDバー1〜8のミュート
//! フェーダー: LEDバーの明るさ・速さ・斜め / 機種ごとの明るさ / メイン・サブの色相 / マスター=全体光量

use crate::apcmini::{pad_note, Ctl};
use crate::color::{ColorTarget, NamedColor, PaletteOp};
use crate::fixture::Library;
use crate::pixel::{PixelParam, FACE_SETS};
use crate::project::*;

/// メイン色・サブ色の段に並べる色(左から)
pub const COLORS: [NamedColor; 8] = [
    NamedColor::Red,
    NamedColor::Orange,
    NamedColor::Yellow,
    NamedColor::Green,
    NamedColor::Cyan,
    NamedColor::Blue,
    NamedColor::Violet,
    NamedColor::White,
];

#[derive(Debug, Default, PartialEq)]
pub struct Report {
    /// 置いた割り当て
    pub bindings: usize,
    /// 置き換えのために外した、今までの割り当て
    pub removed: usize,
    /// 足したLEDバーの模様
    pub presets: usize,
    /// 足したムービング用のエフェクト
    pub effects: usize,
}

impl Report {
    pub fn text(&self) -> String {
        format!(
            "おすすめ配置にしました: 割り当て {} 件(今までの {} 件は置き換え)/ LEDバーの模様 {} 件・エフェクト {} 件を追加",
            self.bindings, self.removed, self.presets, self.effects
        )
    }
}

/// 機種ごとの情報(パッチの順)
struct Kind {
    profile: String,
    name: String,
    /// その機種が持つ機能名(attr)
    attrs: Vec<String>,
}

impl Kind {
    fn attr(&self, a: &str) -> bool {
        self.attrs.iter().any(|x| x == a)
    }
}

/// APC mini mk2 の割り当てを、おすすめの配置に置き換える(APC以外のMIDI機器やOSCだけの割り当ては残す)。
/// LEDバーの模様・ムービング用のエフェクトは、無ければ足す
pub fn apply(p: &mut Project, lib: &Library) -> Report {
    let mut r = Report { presets: crate::pixel::add_face_presets(p), ..Report::default() };

    // ピクセルを持つ機材(LEDバー)と、それ以外の機種
    let mut bars: Vec<(u32, String)> = Vec::new();
    let mut kinds: Vec<Kind> = Vec::new();
    for f in &p.fixtures {
        let (Some(prof), Some(mode)) = (lib.get(&f.profile), lib.mode(&f.profile, f.mode)) else { continue };
        if !mode.pixels().is_empty() {
            bars.push((f.id, f.name.clone()));
        } else if !prof.needs_arm && !kinds.iter().any(|k| k.profile == f.profile) {
            let attrs = mode.channels.iter().filter(|c| c.fine_of.is_none()).map(|c| c.attr.clone()).collect();
            kinds.push(Kind { profile: f.profile.clone(), name: prof.name.clone(), attrs });
        }
    }
    let bar_sel = if bars.is_empty() { None } else { Some(bar_group(p, &bars)) };

    // ムービング(Pan/Tiltを持つ機種)のエフェクト
    let mover = kinds.iter().find(|k| k.attr("pan") && k.attr("tilt"));
    let mut mover_fx: Vec<String> = Vec::new();
    if let Some(m) = mover {
        let sel = Selector::Profile(m.profile.clone());
        let fx = |name: &str, attr: &str, wave: Wave, min: f32, max: f32, beats: f32, phase: f32, spread: f32| Effect {
            name: name.into(),
            enabled: false,
            sel: sel.clone(),
            attr: attr.into(),
            wave,
            rate_hz: 0.25,
            min,
            max,
            phase,
            spread,
            beats,
        };
        let mut list = vec![
            fx("円運動", "pan", Wave::Sine, 96.0, 160.0, 8.0, 0.0, 0.0),
            fx("円運動", "tilt", Wave::Sine, 96.0, 160.0, 8.0, 90.0, 0.0),
            fx("パン揺れ", "pan", Wave::Sine, 80.0, 176.0, 4.0, 0.0, 90.0),
            fx("チルト揺れ", "tilt", Wave::Sine, 90.0, 150.0, 4.0, 0.0, 90.0),
        ];
        if m.attr("dimmer") {
            list.push(fx("点灯チェイス", "dimmer", Wave::Square, 0.0, 255.0, 4.0, 0.0, 90.0));
            list.push(fx("ランダム点灯", "dimmer", Wave::Random, 0.0, 255.0, 1.0, 0.0, 0.0));
        }
        for e in list {
            if !mover_fx.contains(&e.name) {
                mover_fx.push(e.name.clone());
            }
            // 同じ名前のエフェクトが既にあれば、そちらを使う
            if !p.effects.iter().any(|x| x.name == e.name && x.attr == e.attr) {
                p.effects.push(e);
                r.effects += 1;
            }
        }
    }

    // 今までのAPC mini の割り当てを外す
    let before = p.bindings.len();
    p.bindings.retain(|b| !b.trigger.is_some_and(|t| Ctl::from_trigger(t.kind, t.number).is_some_and(|c| c.matches(&t))));
    r.removed = before - p.bindings.len();

    let mut put = |p: &mut Project, ctl: Ctl, name: String, action: Action| {
        let (kind, number) = ctl.trigger();
        p.bindings.push(Binding { name, trigger: Some(MidiTrigger { kind, channel: None, number }), action, page: None, osc: None });
        r.bindings += 1;
    };

    // 1〜4段: LEDバーの模様
    if bar_sel.is_some() {
        for (row, set) in FACE_SETS.iter().enumerate() {
            for (col, fx) in set.presets.iter().enumerate() {
                put(p, Ctl::Pad(pad_note(row as u8, col as u8)), fx.name.to_string(), Action::Effect { name: fx.name.into(), mode: PadMode::Toggle });
            }
        }
    }

    // 5段: ムービング・その他の機材
    let mut extras: Vec<(String, Action)> = mover_fx.into_iter().map(|n| (n.clone(), Action::Effect { name: n, mode: PadMode::Toggle })).collect();
    for k in &kinds {
        let sel = Selector::Profile(k.profile.clone());
        if k.attr("rotation") {
            extras.push(("回転".to_string(), Action::SetValue { sel: sel.clone(), attr: "rotation".into(), mode: PadMode::Toggle, on: 200.0, off: 0.0 }));
        }
        if k.attr("program") {
            extras.push(("オート".to_string(), Action::SetValue { sel, attr: "program".into(), mode: PadMode::Toggle, on: 25.0, off: 0.0 }));
        }
    }
    for (col, (name, action)) in extras.into_iter().take(8).enumerate() {
        put(p, Ctl::Pad(pad_note(4, col as u8)), name, action);
    }

    // 6・7段: メイン色・サブ色
    for (row, target, who) in [(5u8, ColorTarget::Main, "メイン"), (6u8, ColorTarget::Sub, "サブ")] {
        for (col, c) in COLORS.iter().enumerate() {
            put(p, Ctl::Pad(pad_note(row, col as u8)), format!("{who}:{}", c.label()), Action::Color { color: Some(*c), target });
        }
    }

    // 8段: 押している間だけ効くアクセント
    let strobe = p.fixtures.iter().any(|f| lib.mode(&f.profile, f.mode).is_some_and(|m| m.find_attr("strobe").is_some()));
    let mut accents: Vec<(&str, Action)> = vec![
        ("フラッシュ 白", Action::Flash { white: true, mode: PadMode::Momentary }),
        ("フラッシュ 色", Action::Flash { white: false, mode: PadMode::Momentary }),
    ];
    if strobe {
        accents.push(("ストロボ", Action::SetValue { sel: Selector::All, attr: "strobe".into(), mode: PadMode::Momentary, on: 230.0, off: 0.0 }));
    }
    accents.push(("入れ替え(押中)", Action::Palette { op: PaletteOp::HoldSwap }));
    accents.push(("消灯(押中)", Action::Blackout { mode: PadMode::Momentary }));
    for (col, (name, action)) in accents.into_iter().enumerate() {
        put(p, Ctl::Pad(pad_note(7, col as u8)), name.into(), action);
    }

    // 右の列(シーンボタン)
    let fade = p.fade_secs;
    for (i, name, action) in [
        (0u8, "TAP", Action::Tempo { op: TempoOp::Tap }),
        (1, "頭出し", Action::Tempo { op: TempoOp::Resync }),
        (2, "メイン⇄サブ", Action::Palette { op: PaletteOp::Swap }),
        (3, "自動入れ替え", Action::Palette { op: PaletteOp::Auto }),
        (6, "暗転フェード", Action::Fade { secs: fade, mode: PadMode::Toggle }),
        (7, "BLACKOUT", Action::Blackout { mode: PadMode::Toggle }),
    ] {
        put(p, Ctl::Scene(i), name.into(), action);
    }

    // 下の列(トラックボタン): LEDバー1本ずつのミュート。8本に満たなければ、他の機種ごとのミュート
    let mut mutes: Vec<(String, Selector)> = bars.iter().map(|(id, name)| (format!("ミュート:{name}"), Selector::Fixture(*id))).collect();
    mutes.extend(kinds.iter().map(|k| (format!("ミュート:{}", k.name), Selector::Profile(k.profile.clone()))));
    for (i, (name, sel)) in mutes.into_iter().take(8).enumerate() {
        put(p, Ctl::Track(i as u8), name, Action::Mute { sel, mode: PadMode::Toggle });
    }

    // フェーダー
    let mut faders: Vec<(String, Action)> = Vec::new();
    if let Some(sel) = &bar_sel {
        faders.push(("LEDバー 明るさ".into(), Action::Level { sel: sel.clone() }));
        faders.push(("LEDバー 速さ".into(), Action::Pixel { param: PixelParam::Speed }));
        faders.push(("LEDバー 斜め".into(), Action::Pixel { param: PixelParam::Skew }));
    }
    let room = 6 - faders.len();
    for k in kinds.iter().take(room) {
        let sel = Selector::Profile(k.profile.clone());
        let action = if k.attr("dimmer") { Action::SetValue { sel, attr: "dimmer".into(), mode: PadMode::Continuous, on: 255.0, off: 0.0 } } else { Action::Level { sel } };
        faders.push((format!("{} 明るさ", k.name), action));
    }
    for (i, (name, action)) in faders.into_iter().enumerate() {
        put(p, Ctl::Fader(i as u8), name, action);
    }
    put(p, Ctl::Fader(6), "メイン 色相".into(), Action::Color { color: None, target: ColorTarget::Main });
    put(p, Ctl::Fader(7), "サブ 色相".into(), Action::Color { color: None, target: ColorTarget::Sub });
    put(p, Ctl::Fader(8), "全体光量".into(), Action::Master { mode: PadMode::Continuous, on: 100.0, off: 0.0 });
    p.master_seeded = true;

    // 以前の「LEDバーの模様を配置」で入れた模様のうち、どこからも使われなくなったものは片付ける
    let used = |p: &Project, name: &str| p.bindings.iter().any(|b| matches!(&b.action, Action::Effect { name: n, .. } if n == name));
    let old: Vec<String> = p
        .pixel_fx
        .iter()
        .filter(|f| crate::pixel::PRESETS.iter().any(|(n, ..)| *n == f.name) && !FACE_SETS.iter().any(|s| s.presets.iter().any(|x| x.name == f.name)))
        .map(|f| f.name.clone())
        .collect();
    for name in old {
        if !used(p, &name) {
            p.pixel_fx.retain(|f| f.name != name);
        }
    }
    r
}

/// LEDバー全部を指す対象。ちょうどそれだけを含むグループがあれば使い、無ければ「LEDバー」グループを作る
fn bar_group(p: &mut Project, bars: &[(u32, String)]) -> Selector {
    let ids: Vec<u32> = bars.iter().map(|(id, _)| *id).collect();
    let same = |g: &Group| g.fixtures.len() == ids.len() && ids.iter().all(|id| g.fixtures.contains(id));
    if let Some(g) = p.groups.iter().find(|g| same(g)) {
        return Selector::Group(g.name.clone());
    }
    let mut name = "LEDバー".to_string();
    let mut n = 2;
    while p.groups.iter().any(|g| g.name == name) {
        name = format!("LEDバー{n}");
        n += 1;
    }
    p.groups.push(Group { name: name.clone(), fixtures: ids });
    Selector::Group(name)
}

/// `dmxctl --apply-layout`: 保存してあるプロジェクトをおすすめ配置にする(画面は開かない)。先にバックアップを残す
pub fn cli() -> Result<(), String> {
    let (lib, _) = Library::load(&config_dir().join("fixtures"));
    let path = default_project_path();
    let mut p = Project::load(&path).map_err(|e| format!("プロジェクトを読めませんでした: {e}"))?;
    let backup = backup_to(&path, &backup_dir(), BACKUP_KEEP);
    let r = apply(&mut p, &lib);
    p.save(&path).map_err(|e| format!("保存できませんでした: {e}"))?;
    println!("{}", r.text());
    if let Some(b) = backup {
        println!("元のプロジェクト: {}", b.display());
    }
    Ok(())
}
