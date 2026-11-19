//! MIDIの受信と、割り当て(バインディング)の実行。

use super::*;

impl Core {
    /// APC mini mk2 から読み取ったフェーダー1..9の位置を表示用に記録する。
    /// 動作(割り当て)は実行しない: 接続した瞬間に機材が動いたり、レーザーが出たりしないようにするため
    pub fn set_fader_positions(&mut self, positions: &[u8; 9]) {
        for (i, &v) in positions.iter().enumerate() {
            self.rt.cc_value[0x30 + i] = Some(v & 0x7F);
        }
        // 全体光量だけは、実機のフェーダー位置に合わせる(位置と出力がずれたままにならないように)
        let mut master = None;
        for b in &self.project.bindings {
            if let (Action::Master { mode: PadMode::Continuous, on, off }, Some(t)) = (&b.action, b.trigger)
                && t.kind == TriggerKind::Cc && (0x30..=0x38).contains(&t.number) && t.channel.is_none_or(|c| c == 0) {
                let v = positions[(t.number - 0x30) as usize] as f32;
                master = Some(((off + (on - off) * v / 127.0) / 100.0).clamp(0.0, 1.0));
            }
        }
        if let Some(m) = master {
            self.rt.master = m;
        }
        // 機材ごとの明るさとLEDバーの速さ・斜めも、全体光量と同じく実機のフェーダー位置に合わせる
        let sync: Vec<(usize, f32)> = self
            .project
            .bindings
            .iter()
            .enumerate()
            .filter(|(_, b)| matches!(b.action, Action::Level { .. } | Action::Pixel { .. }))
            .filter_map(|(i, b)| {
                let t = b.trigger?;
                (t.kind == TriggerKind::Cc && (0x30..=0x38).contains(&t.number) && t.channel.is_none_or(|c| c == 0)).then(|| (i, positions[(t.number - 0x30) as usize] as f32 / 127.0))
            })
            .collect();
        for (i, v) in sync {
            self.apply_binding(i, v, false);
        }
    }

    pub fn handle_midi(&mut self, m: MidiMsg) {
        if m.kind != TriggerKind::Key {
            self.record_input(serde_json::json!({"cmd":"input","source":"midi","kind":m.kind,
                "channel":m.channel,"number":m.number,"value":m.value}));
        }
        self.note_manual();
        let k = match m.kind {
            TriggerKind::Note => "Note",
            TriggerKind::Cc => "CC",
            TriggerKind::Key => return self.handle_key(m.number, m.value > 0),
        };
        self.rt.last_midi = format!("{k} {} = {} (ch{})", m.number, m.value, m.channel + 1);
        let n = (m.number & 0x7F) as usize;
        match m.kind {
            TriggerKind::Note => self.rt.notes_down[n] = m.value > 0,
            TriggerKind::Cc => self.rt.cc_value[n] = Some(m.value),
            TriggerKind::Key => {}
        }

        // MIDIラーン: 次に来たノートオン/CCをトリガとして登録
        if let Some(idx) = self.rt.learn {
            if m.kind == TriggerKind::Cc || m.value > 0 {
                if let Some(b) = self.project.bindings.get_mut(idx) {
                    b.trigger = Some(MidiTrigger { kind: m.kind, channel: Some(m.channel), number: m.number });
                }
                self.rt.learn = None;
                self.dirty = true;
            }
            return;
        }

        // 今のページの割り当てと全ページ共通の割り当てに反応する。
        // ページを切り替える前に押したパッドは、離したときに元の割り当てへ届ける(押している間の動作が戻らなくならないように)
        let page = self.rt.page;
        let hits: Vec<usize> = self
            .project
            .bindings
            .iter()
            .enumerate()
            .filter(|(i, b)| {
                b.trigger.is_some_and(|t| t.kind == m.kind && t.number == m.number && t.channel.is_none_or(|c| c == m.channel))
                    && (b.on_page(page) || (m.kind == TriggerKind::Note && self.rt.pad_state.get(i) == Some(&true)))
            })
            .map(|(i, _)| i)
            .collect();
        let pressed = match m.kind {
            TriggerKind::Note | TriggerKind::Key => m.value > 0,
            TriggerKind::Cc => m.value >= 64,
        };
        for i in hits {
            self.apply_binding(i, m.value as f32 / 127.0, pressed);
        }
    }

    /// キーボードのキー(crate::core::keys の番号)を押した/離した。
    /// キーボードの割り当ては APC のページとは関係なく、数字の段だけ「模様のセット」(key_page)で切り替わる
    pub fn handle_key(&mut self, number: u8, pressed: bool) {
        self.record_input(serde_json::json!({"cmd":"input","source":"key","number":number,"pressed":pressed}));
        self.note_manual();
        self.rt.keys_down.retain(|k| *k != number);
        if pressed {
            self.rt.keys_down.push(number);
        }
        let page = self.rt.key_page;
        let hits: Vec<usize> = self
            .project
            .bindings
            .iter()
            .enumerate()
            .filter(|(i, b)| {
                b.trigger.is_some_and(|t| t.kind == TriggerKind::Key && t.number == number)
                    && (b.page.is_none_or(|p| p == page) || self.rt.pad_state.get(i) == Some(&true))
            })
            .map(|(i, _)| i)
            .collect();
        for i in hits {
            self.apply_binding(i, if pressed { 1.0 } else { 0.0 }, pressed);
        }
    }

    /// キーボードの割り当てにある「模様のセット」の数
    pub fn key_pages(&self) -> u8 {
        self.project.bindings.iter().filter(|b| b.trigger.is_some_and(|t| t.kind == TriggerKind::Key)).filter_map(|b| b.page).max().map(|p| p + 1).unwrap_or(1)
    }

    /// LEDバー(ピクセルを持つ機材)を、パッチの順に先頭から`count`本だけ使う
    pub fn set_bar_count(&mut self, count: usize) {
        let lib = self.lib.clone();
        let mut n = 0;
        for f in self.project.fixtures.iter_mut() {
            if lib.info(&f.profile, f.mode).is_some_and(|i| !i.pixels.is_empty()) {
                f.enabled = n < count;
                n += 1;
            }
        }
        self.dirty = true;
        // 使い始めた機材にも、今のメイン/サブ色を出す
        self.apply_palette();
    }

    /// 使っているLEDバーの本数と、パッチしてある本数
    pub fn bar_count(&self) -> (usize, usize) {
        let bars: Vec<bool> = self.project.fixtures.iter().filter(|f| self.lib.info(&f.profile, f.mode).is_some_and(|i| !i.pixels.is_empty())).map(|f| f.enabled).collect();
        (bars.iter().filter(|e| **e).count(), bars.len())
    }

    /// この対象・機能を動かしているエフェクトを止める(ボタンで選んだ値が、エフェクトに上書きされないように)
    pub fn stop_effects_on(&mut self, sel: &Selector, attr: &str) {
        let targets: HashSet<usize> = self.resolve(sel).into_iter().collect();
        let names: Vec<String> = self
            .project
            .effects
            .iter()
            .filter(|e| e.enabled && e.attr == attr && self.resolve(&e.sel).into_iter().any(|i| targets.contains(&i)))
            .map(|e| e.name.clone())
            .collect();
        for e in self.project.effects.iter_mut().filter(|e| names.contains(&e.name)) {
            e.enabled = false;
        }
    }

    /// エフェクトをON/OFFする。ONにしたときは、同じ機材の同じチャンネルを動かしている他のエフェクトをOFFにする
    /// (ムービングの動きは1つずつ。後のものが上書きして混ざらないように)
    pub fn set_effect(&mut self, name: &str, on: bool) {
        if on {
            let mine: HashSet<(usize, String)> = self
                .project
                .effects
                .iter()
                .filter(|e| e.name == name)
                .flat_map(|e| self.resolve(&e.sel).into_iter().map(move |i| (i, e.attr.clone())))
                .collect();
            let others: Vec<String> = self
                .project
                .effects
                .iter()
                .filter(|e| e.enabled && e.name != name && self.resolve(&e.sel).into_iter().any(|i| mine.contains(&(i, e.attr.clone()))))
                .map(|e| e.name.clone())
                .collect();
            for e in self.project.effects.iter_mut().filter(|e| others.contains(&e.name)) {
                e.enabled = false;
            }
            // ゴボ・カラーホイールのような選択式のチャンネルは、ボタンで選んでいた値を初期値に戻す
            // (エフェクトを止めたときに前の選択が残らず、ボタンの点灯も消える)
            let lib = self.lib.clone();
            let mut reset: Vec<(u32, String, f32)> = Vec::new();
            for (i, attr) in &mine {
                let f = &self.project.fixtures[*i];
                if let Some(m) = lib.mode(&f.profile, f.mode)
                    && let Some(ch) = m.find_attr(attr).map(|c| &m.channels[c])
                    && !ch.ranges.is_empty() {
                    reset.push((f.id, attr.clone(), ch.default as f32));
                }
            }
            for (id, attr, v) in reset {
                self.set_attr(&Selector::Fixture(id), &attr, v);
            }
        }
        // ムービングのパン/チルトを動かすエフェクトを点けたら、動き(形・向き)は止める(上書きして効かなくなるので)
        if on {
            let heads: HashSet<usize> = self.motion_heads().into_iter().collect();
            let moves = self.project.effects.iter().any(|e| e.name == name && (e.attr == "pan" || e.attr == "tilt") && self.resolve(&e.sel).iter().any(|i| heads.contains(i)));
            if moves {
                self.yield_motion(false);
            }
        }
        for e in self.project.effects.iter_mut().filter(|e| e.name == name) {
            e.enabled = on;
        }
        self.dirty = true;
    }

    /// 割り当てを実行する。`v`=入力値(0.0〜1.0。フェーダー/ノブ用)、`pressed`=押されているか(ボタン用)
    pub fn apply_binding(&mut self, idx: usize, v: f32, pressed: bool) {
        let Some(action) = self.project.bindings.get(idx).map(|b| b.action.clone()) else { return };
        self.apply_action(idx, action, v, pressed);
    }

    pub(crate) fn apply_action(&mut self, idx: usize, action: Action, v: f32, pressed: bool) {
        use crate::core::pixel::PixelParam;
        let v = v.clamp(0.0, 1.0);
        let was = self.rt.pad_state.insert(idx, pressed).unwrap_or(false);
        let edge = Edge { pressed, was };

        match action {
            // ---- フェーダーの値や、離したときにも働くもの ----
            Action::SetValue { sel, attr, mode: PadMode::Continuous, on, off } => self.set_attr(&sel, &attr, off + (on - off) * v),
            Action::SetValue { sel, attr, mode: PadMode::Momentary, on, off } => {
                if edge.changed() {
                    self.set_attr(&sel, &attr, if pressed { on } else { off });
                }
            }
            Action::Master { mode: PadMode::Continuous, on, off } => self.set_master((off + (on - off) * v) / 100.0),
            Action::Master { mode: PadMode::Momentary, on, off } => {
                if edge.changed() {
                    self.set_master((if pressed { on } else { off }) / 100.0);
                }
            }
            // フェーダーの全行程で色相環を一周する(下端=赤 → 上端=赤に戻る)
            Action::Color { color: None, target } => self.apply_hue(v * 360.0, target),
            Action::Tempo { op: TempoOp::Set { min, max } } => self.set_bpm(min + (max - min) * v),
            Action::Level { sel } => {
                for id in self.resolve_ids(&sel) {
                    self.rt.level.insert(id, v);
                }
            }
            Action::Pixel { param: PixelParam::Speed } => self.set_pixel_speed(crate::core::pixel::speed_step(v)),
            Action::Pixel { param: PixelParam::Skew } => self.rt.px_skew = v * crate::core::pixel::SKEW_FADER_MAX,
            Action::Motion { op: crate::core::motion::MotionOp::Size } => self.motion_op(crate::core::motion::MotionOp::Size, v),
            Action::Motion { op: crate::core::motion::MotionOp::Glide } => self.motion_op(crate::core::motion::MotionOp::Glide, v),
            // 押している間だけ個別
            Action::ColorScope { op: ScopeOp::HoldIndividual } => {
                if edge.changed() {
                    self.rt.color_global = !pressed;
                }
            }
            // 押したときに入れ替え、離したときに戻す
            Action::Palette { op: PaletteOp::HoldSwap } => {
                if edge.changed() {
                    self.swap_palette();
                }
            }
            // 押している間だけ暗転
            Action::Fade { secs, mode: PadMode::Continuous | PadMode::Momentary } => {
                if edge.changed() {
                    self.fade_to_black(pressed, secs);
                }
            }

            // ---- ON/OFFの切り替え(トグル/ワンショット/押している間は Edge::switch が決める) ----
            Action::Effect { name, mode } => {
                // 通常のエフェクトとピクセルエフェクトは、同じ名前ならまとめてON/OFF
                let cur = self.project.effects.iter().find(|e| e.name == name).map(|e| e.enabled).or_else(|| self.project.pixel_fx.iter().find(|e| e.name == name).map(|e| e.enabled));
                if let Some(en) = cur.and_then(|c| edge.switch(mode, c)) {
                    self.set_effect(&name, en);
                    if self.project.pixel_fx.iter().any(|e| e.name == name) {
                        self.set_pixel_fx(&name, en);
                    }
                    self.dirty = true;
                }
            }
            Action::Pair { name, op, mode } => {
                let cur = self.project.pairs.iter().find(|p| p.name == name).map(|p| if op == PairOp::Link { p.enabled } else { p.mirror });
                if let Some(v) = cur.and_then(|c| edge.switch(mode, c)) {
                    for p in self.project.pairs.iter_mut().filter(|p| p.name == name) {
                        if op == PairOp::Link {
                            p.enabled = v;
                        } else {
                            p.mirror = v;
                        }
                    }
                    self.dirty = true;
                }
            }
            Action::Chase { name, mode } => {
                let cur = self.project.chases.iter().find(|c| c.name == name).map(|c| c.enabled);
                if let Some(en) = cur.and_then(|c| edge.switch(mode, c)) {
                    for c in self.project.chases.iter_mut().filter(|c| c.name == name) {
                        c.enabled = en;
                    }
                    self.dirty = true;
                }
            }
            Action::Flash { white, mode } => {
                if let Some(on) = edge.switch(mode, self.rt.flash.is_some()) {
                    self.set_flash(on.then_some(white));
                }
            }
            Action::Mute { sel, mode } => {
                let ids = self.resolve_ids(&sel);
                let all = !ids.is_empty() && ids.iter().all(|id| self.rt.muted.contains(id));
                if let Some(m) = edge.switch(mode, all) {
                    for id in ids {
                        if m {
                            self.rt.muted.insert(id);
                        } else {
                            self.rt.muted.remove(&id);
                        }
                    }
                }
            }
            Action::Arm { sel, mode } => {
                let idx: Vec<usize> = self.resolve(&sel);
                let all = !idx.is_empty() && idx.iter().all(|&i| self.project.fixtures[i].armed);
                if let Some(a) = edge.switch(mode, all) {
                    for i in idx {
                        self.project.fixtures[i].armed = a;
                    }
                }
            }
            Action::Blackout { mode } => {
                if let Some(b) = edge.switch(mode, self.rt.blackout) {
                    self.rt.blackout = b;
                }
            }

            // ---- ここから下は、押した瞬間にだけ働く ----
            _ if !edge.rising() => {}
            Action::SetValue { sel, attr, mode: PadMode::Toggle, on, off } => self.toggle_value(idx, &sel, &attr, on, off),
            Action::SetValue { sel, attr, mode: PadMode::Trigger, on, .. } => self.set_attr(&sel, &attr, on),
            Action::Master { mode: PadMode::Toggle, on, off } => {
                // 今ON値ならOFF値へ、そうでなければON値へ
                let to = if (self.rt.master - on / 100.0).abs() < 0.005 { off } else { on };
                self.set_master(to / 100.0);
            }
            Action::Master { mode: PadMode::Trigger, on, .. } => self.set_master(on / 100.0),
            Action::Color { color: Some(c), target } => self.apply_named_color(c, target),
            Action::ColorScope { op: ScopeOp::Toggle } => self.rt.color_global = !self.rt.color_global,
            Action::ColorScope { op: ScopeOp::Global } => self.rt.color_global = true,
            Action::ColorScope { op: ScopeOp::Individual } => self.rt.color_global = false,
            Action::Select { fixture, op: SelectOp::Only } => {
                self.rt.selected.clear();
                self.rt.selected.insert(fixture);
            }
            Action::Select { fixture, op: SelectOp::Toggle } => self.toggle_selected(fixture),
            Action::Role { sel, op } => self.set_roles(&sel, op),
            Action::LiveColor { role, source } => self.toggle_live_color(role, source),
            Action::Palette { op: PaletteOp::Swap } => self.swap_palette(),
            Action::Palette { op: PaletteOp::Auto } => self.set_palette_auto(!self.project.palette.auto),
            Action::Palette { op: PaletteOp::Fade } => self.set_palette_fade(!self.project.palette.fade),
            Action::SwapBeats { beats } => {
                self.project.palette.auto_beats = beats.max(0.25);
                self.set_palette_auto(true);
            }
            Action::Tempo { op: TempoOp::Tap } => self.tap(),
            Action::Tempo { op: TempoOp::Resync } => self.resync(),
            Action::Tempo { op: TempoOp::Nudge { delta } } => self.set_bpm(self.project.bpm + delta),
            Action::Tempo { op: TempoOp::Audio } => self.toggle_tempo_source(TempoSource::Audio),
            Action::Tempo { op: TempoOp::Link } => self.toggle_tempo_source(TempoSource::Link),
            Action::Fade { secs, mode: PadMode::Toggle } => self.toggle_fade(secs),
            Action::Fade { secs, mode: PadMode::Trigger } => self.fade_to_black(true, secs),
            Action::Scene { name } => self.recall_scene(&name),
            Action::Position { name } => self.recall_position(&name),
            Action::Page { op } => {
                self.rt.page = match op {
                    PageOp::Go(p) => p.min(PAGES - 1),
                    PageOp::Next => (self.rt.page + 1) % PAGES,
                    PageOp::Prev => (self.rt.page + PAGES - 1) % PAGES,
                };
            }
            Action::BarCount { count } => self.set_bar_count(count as usize),
            Action::KeyPage { op } => {
                let n = self.key_pages();
                self.rt.key_page = match op {
                    PageOp::Go(p) => p.min(n - 1),
                    PageOp::Next => (self.rt.key_page + 1) % n,
                    PageOp::Prev => (self.rt.key_page + n - 1) % n,
                };
            }
            Action::Motion { op } => self.motion_op(op, v),
        }
    }

    /// 全体光量を設定する(0.0〜1.0)
    fn set_master(&mut self, m: f32) {
        self.rt.master = m.clamp(0.0, 1.0);
    }

    /// トグル動作のチャンネル値: 今ON値ならOFF値へ、そうでなければON値へ
    fn toggle_value(&mut self, idx: usize, sel: &Selector, attr: &str, on: f32, off: f32) {
        // 今の値で判断する(同じチャンネルの別のボタンで変わっていても、押したほうが選ばれる)
        let now_on = !crate::core::led::set_value_active(self, sel, attr, on);
        self.rt.toggle_state.insert(idx, now_on);
        if now_on {
            self.stop_effects_on(sel, attr);
        }
        self.set_attr(sel, attr, if now_on { on } else { off });
    }

    /// テンポの元を`s`にする。すでに`s`なら内部のテンポに戻す
    fn toggle_tempo_source(&mut self, s: TempoSource) {
        let to = if self.project.tempo_source == s { TempoSource::Internal } else { s };
        self.set_tempo_source(to);
    }
}

/// パッドの押下状態の変化(前回→今回)
#[derive(Clone, Copy)]
struct Edge {
    pressed: bool,
    was: bool,
}

impl Edge {
    /// 押した瞬間
    fn rising(self) -> bool {
        self.pressed && !self.was
    }
    /// 押した/離した瞬間
    fn changed(self) -> bool {
        self.pressed != self.was
    }
    /// ON/OFFを切り替える動作の新しい状態(変えないときはNone)。
    /// トグル=押すたびに`cur`を反転、ワンショット=押したらON、それ以外=押している間だけON
    fn switch(self, mode: PadMode, cur: bool) -> Option<bool> {
        match mode {
            PadMode::Toggle => self.rising().then_some(!cur),
            PadMode::Trigger => self.rising().then_some(true),
            PadMode::Continuous | PadMode::Momentary => self.changed().then_some(self.pressed),
        }
    }
}
