//! 右のパネル(図のコントロールを選んだとき): そのコントロールの割り当ての編集。

use super::*;

/// 編集パネルでの操作
#[derive(Default)]
pub(super) struct EditorOut {
    pub(super) changed: bool,
    /// 削除する割り当て(Coreを借り終えてから消す)
    pub(super) remove: Option<usize>,
    /// 「×」で閉じた
    pub(super) close: bool,
}

/// 選んだコントロールの割り当ての編集。今のページで効く割り当て(共通 + このページ)だけを編集対象にする
pub(super) fn control_editor(ui: &mut egui::Ui, core: &mut Core, ctl: Ctl, page: u8, use_pages: bool, cx: &EditCtx) -> EditorOut {
    let mut out = EditorOut::default();
    ui.horizontal(|ui| {
        ui.label(RichText::new(ctl.label()).font(theme::bold(14.0)).color(theme::TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("×").on_hover_text("閉じる(ステージの表示に戻る)").clicked() {
                out.close = true;
            }
        });
    });
    let (kind, num) = ctl.trigger();
    let live = match kind {
        TriggerKind::Cc => match core.rt.cc_value[num as usize] {
            Some(v) => format!("現在の値: {v} / 127"),
            None => "現在の値: (まだ動かしていません)".to_string(),
        },
        TriggerKind::Note => if core.rt.notes_down[num as usize] { "押されています" } else { "離されています" }.to_string(),
        TriggerKind::Key => String::new(),
    };
    ui.label(RichText::new(live).small().color(theme::TEXT_DIM));
    ui.add_space(4.0);

    let mine = |b: &Binding| b.trigger.is_some_and(|t| ctl.matches(&t));
    let idxs: Vec<usize> = core.project.bindings.iter().enumerate().filter(|(_, b)| mine(b) && b.on_page(page)).map(|(i, _)| i).collect();
    let elsewhere = core.project.bindings.iter().filter(|b| mine(b) && !b.on_page(page)).count();
    if use_pages {
        ui.label(RichText::new(format!("ページ{} を表示中", page + 1)).small().color(theme::TEXT_DIM));
    }
    if idxs.is_empty() {
        ui.label(RichText::new("まだ割り当てがありません。").color(theme::TEXT_DIM));
    }
    for i in idxs {
        let learn = Learn { midi: &mut core.rt.learn, osc: &mut core.rt.osc_learn, guarded: &mut core.rt.learn_guarded };
        let r = binding_editor(ui, i, &mut core.project.bindings[i], learn, cx);
        out.changed |= r.changed;
        if r.remove {
            out.remove = Some(i);
        }
    }
    if elsewhere > 0 {
        ui.label(RichText::new(format!("ほかのページにも {elsewhere} 件の割り当てがあります(ページ番号で切り替え)")).small().color(theme::TEXT_DIM));
    }
    ui.add_space(4.0);
    if ui.button("＋ このコントロールに割り当てを追加").clicked() {
        // ページを使っているときは、パッドの新しい割り当ては表示中のページ専用にする
        let pg = (use_pages && matches!(ctl, Ctl::Pad(_))).then_some(page);
        core.project.bindings.push(Binding { id: 0,
            name: ctl.short_name(),
            trigger: Some(MidiTrigger { kind, channel: None, number: num }),
            action: default_action(matches!(ctl, Ctl::Fader(_))),
            page: pg,
            osc: None,
        });
        out.changed = true;
    }
    out
}
