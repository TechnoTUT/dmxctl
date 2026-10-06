//! おすすめのMIDI配置(AKAI APC mini mk2)。マップの「配置」メニュー・初回起動・`--apply-layout` で使う。
//!
//! パッド(上から): 1〜4段=LEDバーの模様(チル/グルーヴ/ピーク/モノクロ) / 5段=ムービングの動き /
//! 6段=ムービングの見た目(明るさ・ゴボ・カラー) / 7段=その他の機材(Twinkle Ball・レーザー) / 8段=押している間だけ効くアクセント
//! 右の列(シーンボタン): TAP・頭出し・メイン⇄サブ・自動入れ替え・暗転フェード・BLACKOUT
//! 下の列(トラックボタン): 機種ごとの消灯(LEDバー全部・ムービング・その他の機種・レーザー)
//! フェーダー: LEDバーの明るさ・速さ・斜め / 機種ごとの明るさ / メイン・サブの色相 / マスター=全体光量

use crate::apcmini::{pad_note, Ctl};
use crate::color::{ColorTarget, PaletteOp};
use crate::fixture::Library;
use crate::pixel::{PixelParam, FACE_SETS};
use crate::project::*;

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
    mode: crate::fixture::Mode,
}

impl Kind {
    fn attr(&self, a: &str) -> bool {
        self.mode.find_attr(a).is_some()
    }
    /// 選択式のチャンネルで、表示名に`keyword`を含む範囲の真ん中の値
    fn value(&self, attr: &str, keyword: &str) -> Option<f32> {
        let ch = &self.mode.channels[self.mode.find_attr(attr)?];
        ch.ranges.iter().find(|r| r.label.contains(keyword)).map(|r| r.mid() as f32)
    }
    /// チャンネルの初期値(トグルを切ったときに戻す値)
    fn default_of(&self, attr: &str) -> f32 {
        self.mode.find_attr(attr).map(|i| self.mode.channels[i].default as f32).unwrap_or(0.0)
    }
    fn sel(&self) -> Selector {
        Selector::Profile(self.profile.clone())
    }
}

/// APC mini mk2 の割り当てを、おすすめの配置に置き換える(APC以外のMIDI機器やOSCだけの割り当ては残す)。
/// LEDバーの模様・ムービング用のエフェクトは、無ければ足す
pub fn apply(p: &mut Project, lib: &Library) -> Report {
    let mut r = Report { presets: crate::pixel::add_face_presets(p), ..Report::default() };

    // ピクセルを持つ機材(LEDバー)と、それ以外の機種
    let mut bars: Vec<(u32, String)> = Vec::new();
    let mut kinds: Vec<Kind> = Vec::new();
    // ARMが必要な機種(レーザー)は、明るさ・ミュートには使わず、動きのプリセットだけに使う
    let mut armed: Vec<Kind> = Vec::new();
    for f in &p.fixtures {
        let (Some(prof), Some(mode)) = (lib.get(&f.profile), lib.mode(&f.profile, f.mode)) else { continue };
        if !mode.pixels().is_empty() {
            bars.push((f.id, f.name.clone()));
            continue;
        }
        let list = if prof.needs_arm { &mut armed } else { &mut kinds };
        if !list.iter().any(|k| k.profile == f.profile) {
            list.push(Kind { profile: f.profile.clone(), name: prof.name.clone(), mode: mode.clone() });
        }
    }
    let bar_sel = if bars.is_empty() { None } else { Some(bar_group(p, &bars)) };

    // 5〜7段: 動きのプリセット(ムービングの動き / ムービングの見た目 / その他の機材)
    let mut rows: [Vec<(String, Action)>; 3] = Default::default();
    let add_fx = |p: &mut Project, r: &mut Report, list: Vec<Effect>| -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for e in list {
            if !names.contains(&e.name) {
                names.push(e.name.clone());
            }
            // 同じ名前・同じ機能のエフェクトが既にあれば、そちらを使う
            if !p.effects.iter().any(|x| x.name == e.name && x.attr == e.attr) {
                p.effects.push(e);
                r.effects += 1;
            }
        }
        names
    };
    let fx = |sel: &Selector, name: &str, attr: &str, wave: Wave, min: f32, max: f32, beats: f32, phase: f32, spread: f32| Effect {
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
    let toggle = |k: &Kind, name: &str, attr: &str, keyword: &str| -> Option<(String, Action)> {
        let on = k.value(attr, keyword)?;
        Some((name.to_string(), Action::SetValue { sel: k.sel(), attr: attr.into(), mode: PadMode::Toggle, on, off: k.default_of(attr) }))
    };
    let effect = |n: String| (n.clone(), Action::Effect { name: n, mode: PadMode::Toggle });

    if let Some(m) = kinds.iter().find(|k| k.attr("pan") && k.attr("tilt")) {
        let sel = m.sel();
        // 動き(Pan/Tilt)
        let moves = vec![
            fx(&sel, "円運動", "pan", Wave::Sine, 96.0, 160.0, 8.0, 0.0, 0.0),
            fx(&sel, "円運動", "tilt", Wave::Sine, 96.0, 160.0, 8.0, 90.0, 0.0),
            fx(&sel, "8の字", "pan", Wave::Sine, 96.0, 160.0, 8.0, 0.0, 0.0),
            fx(&sel, "8の字", "tilt", Wave::Sine, 100.0, 156.0, 4.0, 90.0, 0.0),
            fx(&sel, "パン揺れ", "pan", Wave::Sine, 80.0, 176.0, 4.0, 0.0, 90.0),
            fx(&sel, "チルト揺れ", "tilt", Wave::Sine, 90.0, 150.0, 4.0, 0.0, 90.0),
            fx(&sel, "ワイパー", "pan", Wave::Triangle, 60.0, 196.0, 4.0, 0.0, 0.0),
            fx(&sel, "クロス", "pan", Wave::Sine, 80.0, 176.0, 4.0, 0.0, 180.0),
            fx(&sel, "バラバラ", "pan", Wave::Random, 70.0, 186.0, 2.0, 0.0, 0.0),
            fx(&sel, "バラバラ", "tilt", Wave::Random, 90.0, 160.0, 2.0, 0.0, 0.0),
            fx(&sel, "高速円", "pan", Wave::Sine, 108.0, 148.0, 1.0, 0.0, 0.0),
            fx(&sel, "高速円", "tilt", Wave::Sine, 108.0, 148.0, 1.0, 90.0, 0.0),
        ];
        rows[0] = add_fx(p, &mut r, moves).into_iter().map(effect).collect();
        // 見た目(明るさ・ゴボ・カラーホイール)
        let mut looks = Vec::new();
        if m.attr("dimmer") {
            looks.push(fx(&sel, "点灯チェイス", "dimmer", Wave::Square, 0.0, 255.0, 4.0, 0.0, 90.0));
            looks.push(fx(&sel, "ランダム点灯", "dimmer", Wave::Random, 0.0, 255.0, 1.0, 0.0, 0.0));
            looks.push(fx(&sel, "ブレス", "dimmer", Wave::Sine, 30.0, 255.0, 8.0, 0.0, 90.0));
        }
        if let Some(c) = m.mode.find_attr("gobo") {
            // ゴボ1〜7を順番に(選択式の範囲を、ゆっくりなめていく)
            let gobos: Vec<&crate::fixture::Range> = m.mode.channels[c].ranges.iter().filter(|r| r.label.starts_with("ゴボ") && !r.sweep).collect();
            if let (Some(a), Some(b)) = (gobos.first(), gobos.last()) {
                looks.push(fx(&sel, "ゴボ切替", "gobo", Wave::Saw, a.from as f32, b.to as f32, 8.0, 0.0, 0.0));
            }
        }
        rows[1] = add_fx(p, &mut r, looks).into_iter().map(effect).collect();
        rows[1].extend(
            [
                toggle(m, "ゴボ回転", "gobo", "ゴボホイール回転"),
                toggle(m, "ゴボシェイク", "gobo", "シェイク"),
                toggle(m, "カラー回転", "color", "レインボー"),
                m.attr("dimmer").then(|| ("点灯".to_string(), Action::SetValue { sel: sel.clone(), attr: "dimmer".into(), mode: PadMode::Toggle, on: 255.0, off: 0.0 })),
            ]
            .into_iter()
            .flatten(),
        );
    }
    // その他の機材: モーターで回る灯具(Twinkle Ballなど)とレーザー
    for k in kinds.iter().filter(|k| k.attr("rotation")) {
        rows[2].extend(toggle(k, "回転", "rotation", "回転"));
        let sway = fx(&k.sel(), &format!("{} 揺れ", k.name), "rotation", Wave::Sine, 0.0, 129.0, 8.0, 0.0, 0.0);
        rows[2].extend(add_fx(p, &mut r, vec![sway]).into_iter().map(effect));
        rows[2].extend(toggle(k, "オート", "program", "オートプログラム1"));
        rows[2].extend(toggle(k, "サウンド", "program", "サウンドプログラム1"));
    }
    for k in &armed {
        rows[2].extend(
            [
                toggle(k, "オートショー", "mode", "オートショー(速)"),
                toggle(k, "回転", "z_roll", "時計まわり"),
                toggle(k, "ズーム", "size", "ズームイン/アウト"),
                toggle(k, "描画", "draw", "パターン描画"),
                toggle(k, "7色", "color", "7色チェンジ"),
            ]
            .into_iter()
            .flatten(),
        );
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

    for (k, row) in rows.into_iter().enumerate() {
        for (col, (name, action)) in row.into_iter().take(8).enumerate() {
            put(p, Ctl::Pad(pad_note(4 + k as u8, col as u8)), name, action);
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

    // 下の列(トラックボタン): 機種ごとの消灯(LEDバーはまとめて1つ)
    let mut mutes: Vec<(String, Selector)> = bar_sel.iter().map(|sel| ("ミュート:LEDバー".to_string(), sel.clone())).collect();
    mutes.extend(kinds.iter().chain(armed.iter()).map(|k| (format!("ミュート:{}", k.name), k.sel())));
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
