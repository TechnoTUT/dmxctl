//! ペア(リーダーの値をフォロワーへ写す連動)。

use super::*;

impl Core {
    /// ペアの連動計画。同じ機能名(attr)のチャンネルどうしを対応させる。
    /// 選択式(レンジ付き)のチャンネルは、機種が違うと値の意味が違うので、同じ機種のときだけ連動する
    pub fn pair_plan(&self, p: &Pair) -> Option<PairPlan> {
        let li = self.project.fixture_index(p.leader)?;
        let fi = self.project.fixture_index(p.follower)?;
        if li == fi {
            return None;
        }
        let (lf, ff) = (&self.project.fixtures[li], &self.project.fixtures[fi]);
        let lm = self.lib.mode(&lf.profile, lf.mode)?;
        let fm = self.lib.mode(&ff.profile, ff.mode)?;
        let same = lf.profile == ff.profile;
        let mut chans = Vec::new();
        for (fch, fdef) in fm.channels.iter().enumerate() {
            if fdef.fine_of.is_some() || p.skip_cats.iter().any(|c| c == fdef.category_name()) {
                continue;
            }
            let Some(lch) = lm.find_attr(&fdef.attr) else { continue };
            if lm.channels[lch].fine_of.is_some() {
                continue;
            }
            if !same && (!fdef.ranges.is_empty() || !lm.channels[lch].ranges.is_empty()) {
                continue;
            }
            chans.push((fch, lch, p.mirror && p.invert.contains(&fdef.attr)));
        }
        Some(PairPlan { li, fi, chans })
    }

    /// コントロール画面用: この機材がペアで連動している場合、連動後の値などを返す
    pub fn paired_view(&self, fixture_id: u32) -> Option<PairView> {
        let fi = self.project.fixture_index(fixture_id)?;
        let fx = &self.project.fixtures[fi];
        let fm = self.lib.mode(&fx.profile, fx.mode)?;
        if fx.values.len() != fm.channels.len() {
            return None;
        }
        let mut mask = vec![false; fm.channels.len()];
        let mut values = fx.values.clone();
        let mut inverted = Vec::new();
        let mut leader = None;
        for p in self.project.pairs.iter().filter(|p| p.enabled && p.follower == fixture_id) {
            let Some(plan) = self.pair_plan(p) else { continue };
            let lf = &self.project.fixtures[plan.li];
            let Some(lm) = self.lib.mode(&lf.profile, lf.mode) else { continue };
            apply_pair(&plan, lm, fm, &lf.values, &mut values);
            for &(fch, _, inv) in &plan.chans {
                mask[fch] = true;
                if inv && !inverted.contains(&fm.channels[fch].attr) {
                    inverted.push(fm.channels[fch].attr.clone());
                }
            }
            leader = Some(lf.name.clone());
        }
        leader.map(|leader| PairView { leader, mask, values, inverted })
    }
}

/// ペアの連動計画: (フォロワーのチャンネル, リーダーのチャンネル, 反転するか)
pub struct PairPlan {
    pub li: usize,
    pub fi: usize,
    pub chans: Vec<(usize, usize, bool)>,
}

/// 連動を適用する: リーダーの値(`lvals`)を、計画に従ってフォロワーの値(`fvals`)へ写す
pub fn apply_pair(plan: &PairPlan, lm: &Mode, fm: &Mode, lvals: &[u8], fvals: &mut [u8]) {
    if lvals.len() != lm.channels.len() || fvals.len() != fm.channels.len() {
        return;
    }
    for &(fch, lch, inv) in &plan.chans {
        let mut v = lm.get_f(lvals, lch);
        if inv {
            v = lm.invert_f(lch, v);
        }
        fm.set_f(fvals, fch, v);
    }
}

/// 画面表示用: ペアに連動している機材の、連動後の値と、連動しているチャンネル
pub struct PairView {
    pub leader: String,
    pub mask: Vec<bool>,
    pub values: Vec<u8>,
    pub inverted: Vec<String>,
}
