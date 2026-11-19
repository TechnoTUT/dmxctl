//! ベース値・エフェクト・ブラックアウト・ARMゲートを合成して、ユニバースごとのフレームを作る。
use super::*;

impl Core {
    /// ベース値 + エフェクト + ブラックアウト/ARMゲートを合成して、ユニバースごとのフレームを作る
    pub fn render(&mut self) {
        let now = self.now();
        self.rt.safety_events.clear();
        self.expire_remote_controls(now);
        if self.rt.flash_deadline.is_some_and(|deadline|now >= deadline) {
            self.rt.flash = None;
            self.rt.flash_deadline = None;
            if let Some(recorder) = self.rt.observe.recorder() {
                recorder.emit(serde_json::json!({"type":"command","source":"lease_expired",
                    "command":{"cmd":"flash","on":false},"applies_from_frame":recorder.next_frame(),
                    "applies_from_engine_frame":self.rt.frame_id+1,
                    "t_ms":now.saturating_duration_since(self.rt.t0).as_millis() as u64}));
            }
        }
        self.trace_begin();
        // 拍を進める(ウィンドウ移動などで止まった直後に、拍が大きく飛ばないよう上限を付ける)
        let dt = self.rt.last_beat_t.map(|t| now.duration_since(t).as_secs_f64().min(0.5)).unwrap_or(0.0);
        self.rt.last_beat_t = Some(now);
        self.sync_beat(now, dt);
        self.run_auto(dt);
        self.run_palette_auto();
        self.run_live_colors();
        self.step_dim(dt);
        self.run_chases();
        self.advance_fades(now);
        self.trace_stage("automation_and_fades",None,serde_json::Value::Null);
        let t = (now - self.rt.t0).as_secs_f64();
        let lib = self.lib.clone();

        let mut vals: Vec<Vec<u8>> = self.project.fixtures.iter().map(|f| f.values.clone()).collect();

        for e in self.project.effects.iter().filter(|e| e.enabled) {
            for (n, i) in self.resolve(&e.sel).into_iter().enumerate() {
                let fx = &self.project.fixtures[i];
                let Some(mode) = lib.mode(&fx.profile, fx.mode) else { continue };
                let Some(info) = lib.info(&fx.profile, fx.mode) else { continue };
                let chans = info.channels(&e.attr);
                if chans.is_empty() || vals[i].len() != mode.channels.len() {
                    continue;
                }
                // 拍に同期するエフェクトは、拍の位置から位相を決める(テンポを変えても位相が飛ばない)
                let base = if e.beats > 0.0 { self.rt.beat / e.beats as f64 } else { t * e.rate_hz as f64 };
                let p = base + (e.phase as f64 + e.spread as f64 * n as f64) / 360.0;
                let cycle = p.floor();
                let frac = (p - cycle) as f32;
                let w = e.wave.eval(frac, cycle as i64 as u64, fx.id.wrapping_mul(31).wrapping_add(n as u32));
                for &ch in chans {
                    info.set_f(&mut vals[i], ch, e.min + (e.max - e.min) * w);
                }
            }
        }

        if self.rt.observe.trace.as_ref().is_some_and(|trace|trace.result.is_none()) {
            let id=self.rt.observe.trace.as_ref().unwrap().fixture;
            let effects:Vec<_>=self.project.effects.iter().filter(|e|e.enabled && self.resolve_ids(&e.sel).contains(&id)).collect();
            let details=serde_json::json!({"effects":effects});
            self.trace_stage("effects",Some(&vals),details);
        }
        // ムービングの動き(形・1周の長さ・揃え方・向き)
        self.render_motion(&mut vals);
        self.trace_stage("motion",Some(&vals),serde_json::Value::Null);

        // ピクセルエフェクト(LEDバー)
        self.render_pixel_fx(&mut vals);
        self.trace_stage("pixel",Some(&vals),serde_json::Value::Null);

        // ペア: フォロワーの値を、(エフェクト適用後の)リーダーの値から作る。反転もここで行う
        self.render_pairs(&mut vals);

        self.trace_stage("pairs",Some(&vals),serde_json::Value::Null);
        // 全体光量: 各機材のディマー(無ければ色の強さのチャンネル)に掛ける
        // 暗転フェードと機材ごとの明るさ(フェーダー)も、全体光量と同じところにかける
        // OSCで受けた明るさ(波形の大きさなど)も同じところにかける。効き具合(depth)が0なら効かない
        let level = self.rt.osc_level.as_mut().map(|o| o.step(now)[0]).map(|l| 1.0 - self.project.osc_level_depth.clamp(0.0, 1.0) * (1.0 - l.clamp(0.0, 1.0))).unwrap_or(1.0);
        // オートのプリセットの明るさ(曲の情報が届いている間)
        let bright = if self.project.auto.enabled && self.rt.auto_was_online { self.rt.auto_bright.as_mut().map(|o| o.step(now)[0]).unwrap_or(1.0) } else { 1.0 };
        let master = self.rt.master * self.rt.dim * level * bright;
        if master < 0.9999 || !self.rt.level.is_empty() {
            for (i, fx) in self.project.fixtures.iter().enumerate() {
                let master = master * self.rt.level.get(&fx.id).copied().unwrap_or(1.0);
                if master >= 0.9999 {
                    continue;
                }
                let Some(mode) = lib.mode(&fx.profile, fx.mode) else { continue };
                if vals[i].len() != mode.channels.len() {
                    continue;
                }
                let Some(info) = lib.info(&fx.profile, fx.mode) else { continue };
                for &ch in &info.master {
                    let v = info.get_f(&vals[i], ch);
                    info.set_f(&mut vals[i], ch, v * master);
                }
            }
        }

        if self.rt.observe.trace.as_ref().is_some_and(|t|t.result.is_none()) {
            let details=serde_json::json!({"master":self.rt.master,"dim":self.rt.dim,"osc_level":level,"auto_brightness":bright,"fixture_levels":self.rt.level});
            self.trace_stage("master",Some(&vals),details);
        }
        // フラッシュ: 全開にする(全体光量より優先。ブラックアウトとARMは下で優先される)。レーザーなどARMが必要な機材には掛けない
        if let Some(white) = self.rt.flash {
            for (i, fx) in self.project.fixtures.iter().enumerate() {
                let Some(profile) = lib.get(&fx.profile) else { continue };
                let (Some(mode), Some(info)) = (profile.modes.get(fx.mode), lib.info(&fx.profile, fx.mode)) else { continue };
                if profile.needs_arm || vals[i].len() != mode.channels.len() {
                    continue;
                }
                flash_values(mode, &info, &mut vals[i], white);
            }
        }

        self.trace_stage("flash",Some(&vals),serde_json::Value::Null);
        // オートで曲の情報が届いていないときは、薄い水色で待つ(ブラックアウトとARMは下で優先される)
        self.apply_idle(&mut vals);
        self.trace_stage("idle",Some(&vals),serde_json::Value::Null);

        for f in self.rt.frames.iter_mut() {
            *f = [0; 512];
        }
        let mut used = vec![0usize; NUM_UNIVERSES];

        for (i, fx) in self.project.fixtures.iter().enumerate() {
            // 使わない機材は出力しない(0のまま)
            if !fx.enabled {
                continue;
            }
            let Some(profile) = lib.get(&fx.profile) else { continue };
            let Some(mode) = profile.modes.get(fx.mode) else { continue };
            let v = &mut vals[i];
            let requested = (self.rt.observe.is_recording() || self.rt.observe.trace.as_ref().is_some_and(|t|t.fixture==fx.id && t.result.is_none())).then(||v.clone());
            if v.len() != mode.channels.len() || fx.universe >= NUM_UNIVERSES || fx.address == 0 {
                continue;
            }
            // 暗転しきったら、ブラックアウトと同じ扱い(レーザーなど明るさのチャンネルが無い機材も止める)
            let dark = self.rt.blackout || self.rt.dim <= 0.001;
            // ミュート中も安全値を出す(明るさのチャンネルが無いレーザーも消えるように)
            let muted = self.rt.muted.contains(&fx.id);
            if dark || muted {
                if let Some(info) = lib.info(&fx.profile, fx.mode) {
                    safe_values(mode, &info, v);
                }
            } else if profile.needs_arm && !fx.armed {
                for (c, ch) in mode.channels.iter().enumerate() {
                    if let Some(s) = ch.safe {
                        v[c] = s;
                    }
                }
            }
            if let Some(trace)=self.rt.observe.trace.as_mut().filter(|t|t.fixture==fx.id && t.result.is_none()) {
                trace.capture_single(fx.id,mode,v,"gates",serde_json::json!({"blackout":dark,"muted":muted,"armed":fx.armed,"needs_arm":profile.needs_arm}));
            }
            // 禁止の値(レーザーのオートショーなど)と、動いてよい範囲は、何をしていても最後に必ず直す
            let gated = requested.as_ref().map(|_| v.clone());
            guard_values(mode, &fx.limits, v);
            if let Some(trace)=self.rt.observe.trace.as_mut().filter(|t|t.fixture==fx.id && t.result.is_none()) {
                trace.capture_single(fx.id,mode,v,"safety",serde_json::json!({"limits":fx.limits}));
            }
            if let Some(before) = requested {
                let gated = gated.unwrap();
                for (ch, (&requested, &applied)) in before.iter().zip(v.iter()).enumerate() {
                    if requested != applied {
                        let mut reasons = Vec::new();
                        if requested != gated[ch] {
                            reasons.push(if dark { "blackout" } else if muted { "mute" } else { "not_armed" });
                        }
                        let [a, b] = fx.limits.get(&mode.channels[ch].attr).copied().unwrap_or([0, 255]);
                        let clamped = gated[ch].clamp(a.min(b), a.max(b));
                        if gated[ch] != clamped { reasons.push("limits"); }
                        if mode.channels[ch].is_forbidden(clamped) { reasons.push("forbid"); }
                        self.rt.safety_events.push(serde_json::json!({
                            "fixture": fx.id, "attr": mode.channels[ch].attr,
                            "channel": ch + 1, "requested": requested, "applied": applied,
                            "reason": reasons.join("+"), "reasons": reasons
                        }));
                    }
                }
            }
            let base = fx.address as usize - 1;
            for (c, &b) in v.iter().enumerate() {
                let a = base + c;
                if a < 512 {
                    self.rt.frames[fx.universe][a] = b;
                    used[fx.universe] = used[fx.universe].max(a + 1);
                }
            }
        }
        for (u, n) in used.into_iter().enumerate() {
            self.rt.used_len[u] = n.clamp(MIN_SLOTS, 512);
        }
        self.rt.frame_id += 1;
        self.trace_finish();
        if let Some(recorder) = self.rt.observe.recorder().cloned() {
            recorder.frame(self, false);
        }
    }
}
