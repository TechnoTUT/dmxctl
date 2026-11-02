//! 色の全体制御(全体/個別の操作先)と、メイン色/サブ色(パレット)。

use super::*;

impl Core {
    // ---- 色の全体制御 ----

    /// 色を掛ける機材(id)。`Scope`は「全体/個別」スイッチに従う。`Main`/`Sub`は、その系統に割り当てた機材
    pub fn color_targets(&self, target: ColorTarget) -> Vec<u32> {
        if let Some(role) = target.slot() {
            return self.project.fixtures.iter().filter(|f| f.color_role == role).map(|f| f.id).collect();
        }
        let all = matches!(target, ColorTarget::All) || (matches!(target, ColorTarget::Scope) && self.rt.color_global);
        self.project.fixtures.iter().filter(|f| all || self.rt.selected.contains(&f.id)).map(|f| f.id).collect()
    }

    // ---- メイン色/サブ色 ----

    /// 今、メインとサブが入れ替わっているか(入れ替えボタン + 拍による自動入れ替え)
    pub fn palette_swapped(&self) -> bool {
        let p = &self.project.palette;
        let auto = p.auto && p.auto_beats > 0.0 && (self.rt.beat / p.auto_beats as f64).floor().rem_euclid(2.0) >= 1.0;
        self.rt.color_swapped ^ auto
    }

    /// この系統の機材に、今出す色
    pub fn role_rgb(&self, role: ColorRole) -> Option<[f32; 3]> {
        let p = &self.project.palette;
        let main_side = match role {
            ColorRole::Main => true,
            ColorRole::Sub => false,
            ColorRole::Off => return None,
        };
        // NDI(映像・音)を使っている系統は、その色
        let main = self.rt.live_main.unwrap_or(p.main);
        let sub = self.rt.live_sub.unwrap_or(p.sub);
        let (a, b) = if main_side != self.palette_swapped() { (main, sub) } else { (sub, main) };
        if p.fade {
            // 間隔の拍数でもう一方の色へ、同じ拍数で戻る(なめらかに)
            let t = (0.5 - 0.5 * (std::f64::consts::PI * self.rt.beat / p.auto_beats.max(0.25) as f64).cos()) as f32;
            return Some([0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t));
        }
        Some(a)
    }

    /// NDIの色をメイン色/サブ色に反映する(変わったときだけ機材に出し直す。保存のきっかけにはしない)
    pub(super) fn run_live_colors(&mut self) {
        use crate::ndi::LiveSource;
        let pick = |s: LiveSource, st: &crate::ndi::NdiState| match s {
            LiveSource::Off => None,
            LiveSource::Audio => st.audio,
            LiveSource::Video => st.video,
        };
        let cfg = &self.project.ndi;
        let (main, sub) = if cfg.enabled { (pick(cfg.main, &self.rt.ndi), pick(cfg.sub, &self.rt.ndi)) } else { (None, None) };
        let near = |a: Option<[f32; 3]>, b: Option<[f32; 3]>| match (a, b) {
            (Some(a), Some(b)) => (0..3).all(|k| (a[k] - b[k]).abs() < 1.0 / 255.0),
            (None, None) => true,
            _ => false,
        };
        if near(main, self.rt.live_main) && near(sub, self.rt.live_sub) {
            return;
        }
        self.rt.live_main = main;
        self.rt.live_sub = sub;
        let was = self.dirty;
        self.apply_palette();
        self.dirty = was;
    }

    /// メイン/サブに割り当てた機材に、今の色を出す
    pub fn apply_palette(&mut self) {
        let plan: Vec<(u32, [f32; 3])> = self.project.fixtures.iter().filter_map(|f| self.role_rgb(f.color_role).map(|c| (f.id, c))).collect();
        for (id, c) in plan {
            self.apply_rgb(&[id], c);
        }
        self.rt.palette_shown = Some(self.palette_swapped());
    }

    pub fn set_palette_color(&mut self, slot: ColorRole, rgb: [f32; 3]) {
        match slot {
            ColorRole::Main => self.project.palette.main = rgb,
            ColorRole::Sub => self.project.palette.sub = rgb,
            ColorRole::Off => return,
        }
        self.dirty = true;
        self.apply_palette();
    }

    pub fn set_color_role(&mut self, id: u32, role: ColorRole) {
        if let Some(i) = self.project.fixture_index(id) {
            self.project.fixtures[i].color_role = role;
            self.dirty = true;
            self.apply_palette();
        }
    }

    /// メインとサブを入れ替える
    pub fn swap_palette(&mut self) {
        self.rt.color_swapped = !self.rt.color_swapped;
        self.apply_palette();
    }

    /// グラデーション(メイン⇄サブを少しずつ行き来)のON/OFF。自動入れ替えとはどちらか一方
    pub fn set_palette_fade(&mut self, on: bool) {
        self.project.palette.fade = on;
        if on {
            self.project.palette.auto = false;
        }
        self.dirty = true;
        self.apply_palette();
    }

    pub fn set_palette_auto(&mut self, on: bool) {
        if on {
            self.project.palette.fade = false;
        }
        self.project.palette.auto = on;
        self.dirty = true;
        self.apply_palette();
    }

    /// 自動入れ替え中は、拍の区切りで色を出し直す
    pub(super) fn run_palette_auto(&mut self) {
        // グラデーション中は、毎回の出力で色を出し直す(保存のきっかけにはしない)
        if self.project.palette.fade {
            let was = self.dirty;
            self.apply_palette();
            self.dirty = was;
            return;
        }
        if !self.project.palette.auto || self.rt.palette_shown == Some(self.palette_swapped()) {
            return;
        }
        // 拍による切り替えは一時的なものなので、自動保存のきっかけにしない
        let was = self.dirty;
        self.apply_palette();
        self.dirty = was;
    }

    /// 各機材の仕組み(RGB / カラーホイール / 色レンジ)に合わせて、その色に設定する
    pub fn apply_rgb(&mut self, ids: &[u32], rgb: [f32; 3]) {
        let lib = self.lib.clone();
        for &id in ids {
            let Some(i) = self.project.fixture_index(id) else { continue };
            let fx = &self.project.fixtures[i];
            let Some(mode) = lib.mode(&fx.profile, fx.mode) else { continue };
            for (attr, v) in color::plan_for_mode(mode, rgb) {
                self.set_attr(&Selector::Fixture(id), &attr, v);
            }
        }
    }

    pub fn apply_named_color(&mut self, c: NamedColor, target: ColorTarget) {
        if let Some(slot) = target.slot() {
            self.set_palette_color(slot, c.rgb());
            return;
        }
        let ids = self.color_targets(target);
        self.apply_rgb(&ids, c.rgb());
        self.rt.last_color = Some(c);
    }

    /// HSVの色相H(度。0〜360で一周。0=赤 / 60=黄 / 120=緑 / 180=水色 / 240=青 / 300=マゼンタ / 360=赤に戻る)を、
    /// 彩度・明度を最大にした色として、対象の機材に掛ける
    pub fn apply_hue(&mut self, degrees: f32, target: ColorTarget) {
        let h = degrees.clamp(0.0, 360.0);
        if let Some(slot) = target.slot() {
            self.set_palette_color(slot, color::hsv(h, 1.0, 1.0));
            return;
        }
        let ids = self.color_targets(target);
        self.apply_rgb(&ids, color::hsv(h, 1.0, 1.0));
        self.rt.hue = h;
        self.rt.last_color = None;
    }

    pub fn toggle_selected(&mut self, id: u32) {
        if !self.rt.selected.remove(&id) {
            self.rt.selected.insert(id);
        }
    }

    /// 対象の機材の色の系統(メイン/サブ)をまとめて変える
    pub fn set_roles(&mut self, sel: &Selector, op: RoleOp) {
        let ids = self.resolve_ids(sel);
        let all_main = ids.iter().all(|id| self.project.fixture_index(*id).is_some_and(|i| self.project.fixtures[i].color_role == ColorRole::Main));
        for (n, id) in ids.into_iter().enumerate() {
            let role = match op {
                RoleOp::Main => ColorRole::Main,
                RoleOp::Sub => ColorRole::Sub,
                // まとめて切り替え: 全部メインならサブへ、そうでなければメインへ
                RoleOp::Toggle => if all_main { ColorRole::Sub } else { ColorRole::Main },
                RoleOp::Alternate => if n % 2 == 0 { ColorRole::Main } else { ColorRole::Sub },
            };
            self.set_color_role(id, role);
        }
    }

    /// メイン色/サブ色にNDIの色(映像/音)を使う。同じものを選び直すと使わない(パレットの色に戻る)
    pub fn toggle_live_color(&mut self, role: ColorRole, source: crate::ndi::LiveSource) {
        let cfg = &mut self.project.ndi;
        let slot = if role == ColorRole::Sub { &mut cfg.sub } else { &mut cfg.main };
        *slot = if *slot == source { crate::ndi::LiveSource::Off } else { source };
        if source != crate::ndi::LiveSource::Off {
            cfg.enabled = true;
        }
        self.dirty = true;
    }
}
