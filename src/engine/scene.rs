//! シーン・位置プリセットの保存と呼び出し、値のフェード。

use super::*;

pub struct Fade {
    pub fixture: u32,
    pub ch: usize,
    pub from: f32,
    pub to: f32,
    pub start: Instant,
    pub dur: f32,
}

impl Core {
    /// 選んだ機材の現在値をシーンにする。`skip_cats`に含まれるカテゴリ(動き/色など)のチャンネルは保存しない
    pub fn capture_scene(&self, name: &str, fade: f32, fixtures: &[u32], skip_cats: &HashSet<String>) -> Scene {
        let mut values = Vec::new();
        for fx in self.project.fixtures.iter().filter(|f| fixtures.contains(&f.id)) {
            let Some(mode) = self.lib.mode(&fx.profile, fx.mode) else { continue };
            if fx.values.len() != mode.channels.len() {
                continue;
            }
            for (ci, ch) in mode.channels.iter().enumerate() {
                if ch.fine_of.is_some() || skip_cats.contains(ch.category_name()) {
                    continue;
                }
                values.push(SceneValue { fixture: fx.id, attr: ch.attr.clone(), value: mode.get_f(&fx.values, ci) });
            }
        }
        Scene { name: name.into(), fade, values }
    }

    /// 既存シーンの各項目を現在の値で更新する。動きだけ等の部分シーンは、部分のまま更新される
    pub fn refresh_scene(&mut self, idx: usize) {
        let Some(scene) = self.project.scenes.get(idx).cloned() else { return };
        let mut values = scene.values;
        for sv in &mut values {
            let Some(fx) = self.project.fixtures.iter().find(|f| f.id == sv.fixture) else { continue };
            let Some(mode) = self.lib.mode(&fx.profile, fx.mode) else { continue };
            let Some(ch) = mode.find_attr(&sv.attr) else { continue };
            if fx.values.len() == mode.channels.len() {
                sv.value = mode.get_f(&fx.values, ch);
            }
        }
        self.project.scenes[idx].values = values;
        self.dirty = true;
    }

    pub fn recall_scene(&mut self, name: &str) {
        self.recall_scene_with(name, None);
    }

    /// `fade_override`(秒)があれば、シーン自身のフェード時間の代わりに使う(チェイスがBPMから決める)
    pub fn recall_scene_with(&mut self, name: &str, fade_override: Option<f32>) {
        let Some(scene) = self.project.scenes.iter().find(|s| s.name == name).cloned() else { return };
        let fade = fade_override.unwrap_or(scene.fade);
        self.rt.last_scene = Some(scene.name.clone());
        self.apply_values(&scene.values, fade);
    }

    // ---- 位置プリセット ----

    /// 今のパン/チルトを位置プリセットにする(パン/チルトを持つ全機材)
    pub fn capture_position(&self, name: &str, fade: f32) -> Scene {
        let mut values = Vec::new();
        for fx in &self.project.fixtures {
            let Some(mode) = self.lib.mode(&fx.profile, fx.mode) else { continue };
            if fx.values.len() != mode.channels.len() {
                continue;
            }
            for attr in POSITION_ATTRS {
                if let Some(ch) = mode.find_attr(attr) {
                    values.push(SceneValue { fixture: fx.id, attr: attr.into(), value: mode.get_f(&fx.values, ch) });
                }
            }
        }
        Scene { name: name.into(), fade, values }
    }

    pub fn recall_position(&mut self, name: &str) {
        let Some(p) = self.project.positions.iter().find(|s| s.name == name).cloned() else { return };
        // 位置プリセットを中心にして動かす(「向き」は外す)
        self.yield_motion(true);
        self.apply_values(&p.values, p.fade);
        self.rt.last_position = Some(p.name);
    }

    /// シーン/位置プリセットの値を、フェード(秒)付きで適用する
    fn apply_values(&mut self, values: &[SceneValue], fade: f32) {
        let lib = self.lib.clone();
        let now = Instant::now();
        for sv in values {
            let Some(i) = self.project.fixture_index(sv.fixture) else { continue };
            let fx = &mut self.project.fixtures[i];
            let Some(mode) = lib.mode(&fx.profile, fx.mode) else { continue };
            let Some(ch) = mode.find_attr(&sv.attr) else { continue };
            if fx.values.len() != mode.channels.len() {
                continue;
            }
            let id = fx.id;
            self.rt.fades.retain(|f| !(f.fixture == id && f.ch == ch));
            let from = mode.get_f(&fx.values, ch);
            // カラー/ゴボ等の離散チャンネルはフェードせず即切り替える(途中の値で別の色が出てしまうため)
            if fade <= 0.0 || !mode.channels[ch].ranges.is_empty() {
                mode.set_f(&mut fx.values, ch, sv.value);
            } else {
                self.rt.fades.push(Fade { fixture: id, ch, from, to: sv.value, start: now, dur: fade });
            }
        }
        self.dirty = true;
    }

    pub(super) fn advance_fades(&mut self, now: Instant) {
        if self.rt.fades.is_empty() {
            return;
        }
        let lib = self.lib.clone();
        let fades = std::mem::take(&mut self.rt.fades);
        let mut keep = Vec::with_capacity(fades.len());
        for f in fades {
            let Some(i) = self.project.fixture_index(f.fixture) else { continue };
            let fx = &mut self.project.fixtures[i];
            let Some(mode) = lib.mode(&fx.profile, fx.mode) else { continue };
            if fx.values.len() != mode.channels.len() {
                continue;
            }
            let p = if f.dur <= 0.0 { 1.0 } else { ((now - f.start).as_secs_f32() / f.dur).min(1.0) };
            mode.set_f(&mut fx.values, f.ch, f.from + (f.to - f.from) * p);
            if p < 1.0 {
                keep.push(f);
            }
        }
        self.rt.fades = keep;
    }
}
