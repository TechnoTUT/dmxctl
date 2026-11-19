//! 図全体: コントロールごとの情報を今のページの割り当てから集め、実機と同じ配置に並べて描く。

use super::*;
use std::collections::HashMap;

/// 図の1コントロール分の情報(今のページで効く割り当てから作る)
pub(super) fn control_info(core: &Core, meta: &SelMeta, look_rgb: &HashMap<u32, [f32; 3]>, page: u8, ctl: Ctl) -> Info {
    let hits: Vec<&Binding> = core.project.bindings.iter().filter(|b| b.on_page(page) && b.trigger.is_some_and(|t| ctl.matches(&t))).collect();
    let Some(first) = hits.first() else {
        return Info { tag: String::new(), title: String::new(), tip: format!("{}\n割り当てなし", ctl.label()), count: 0, color: None, active: false, rgb: None, readout: Readout::Raw, preview: Preview::None };
    };
    let (tag, title) = pad_text(&first.action, meta, core);
    // フェーダーとチャンネル値は、名前を付けてあればその名前のほうが分かりやすい(「LEDバー 明るさ」「ストロボ」など)
    let named = !first.name.trim().is_empty() && first.name != ctl.short_name();
    let title = match ctl {
        // フェーダーは「対象 / 何を」の2行に分ける
        Ctl::Fader(_) if named => match first.name.trim().rsplit_once(' ') {
            Some((a, b)) => format!("{a}\n{b}"),
            None => first.name.trim().to_string(),
        },
        _ if named && matches!(first.action, Action::SetValue { .. }) => first.name.clone(),
        _ => title,
    };
    let tip = hits.iter().map(|b| format!("{}: {}", b.name, summary(&b.action, meta).replace('\n', " "))).collect::<Vec<_>>().join("\n");
    Info {
        tag,
        title: if hits.len() > 1 { format!("{title} +{}", hits.len() - 1) } else { title },
        tip: format!("{}\n{tip}", ctl.label()),
        count: hits.len(),
        color: Some(action_color(&first.action, &core.project)),
        active: hits.iter().any(|b| dmxctl_core::led::is_active(core, &b.action)),
        rgb: if matches!(ctl, Ctl::Pad(_)) { dmxctl_core::led::action_rgb(core, &first.action, look_rgb) } else { None },
        readout: readout(&first.action),
        preview: match &first.action {
            Action::Effect { name, .. } if matches!(ctl, Ctl::Pad(_)) => match core.project.pixel_fx.iter().find(|f| f.name == *name) {
                Some(f) => Preview::Face(core.face_preview(f, 16, 30)),
                None => effect_path(core, name).unwrap_or(Preview::None),
            },
            _ => Preview::None,
        },
    }
}

/// APC mini mk2 の図を、残りの場所に合わせた大きさで描く。`sel`=選択中のコントロール。戻り値=クリックされたコントロール
pub(super) fn controller_map(ui: &mut egui::Ui, rt: &Runtime, sel: Option<Ctl>, info: &dyn Fn(Ctl) -> Info) -> Option<Ctl> {
    let mut clicked = None;
    // 図の大きさを、残りの場所に合わせて決める(パッドは横長、フェーダーはパッド2段ぶん)
    let area = ui.available_rect_before_wrap();
    let gap = 6.0;
    let side_k = 0.72;
    let (track_k, fader_k) = (0.42, 2.9);
    let pad_h = ((area.height() - 9.0 * gap - 8.0) / (8.0 + track_k + fader_k)).clamp(30.0, 86.0);
    let pad_w = ((area.width() - 8.0 * gap - 8.0) / (8.0 + side_k)).clamp(pad_h, pad_h * 1.8);
    let side_w = (pad_w * side_k).max(54.0);
    let track_h = (pad_h * track_k).max(22.0);
    let fader_h = pad_h * fader_k;
    let side_x = 8.0 * (pad_w + gap) + 2.0;
    let track_y = 8.0 * (pad_h + gap) + 2.0;
    let fader_y = track_y + track_h + gap + 2.0;
    let total = vec2(side_x + side_w, fader_y + fader_h);

    egui::ScrollArea::both().auto_shrink([false; 2]).show(ui, |ui| {
        let (area, _) = ui.allocate_exact_size(total, Sense::hover());
        let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(area.min + vec2(x, y), vec2(w, h));
        let down = |ctl: Ctl| rt.notes_down[ctl.trigger().1 as usize];

        // パッド(最上段が0x38..0x3F)
        for row in 0..8u8 {
            for col in 0..8u8 {
                let ctl = Ctl::Pad(pad_note(row, col));
                let r = at(col as f32 * (pad_w + gap), row as f32 * (pad_h + gap), pad_w, pad_h);
                if button_cell(ui, r, ctl, &info(ctl), down(ctl), sel == Some(ctl)) {
                    clicked = Some(ctl);
                }
            }
        }
        // シーンボタン(上から1..8)
        for i in 0..8u8 {
            let ctl = Ctl::Scene(i);
            let r = at(side_x, i as f32 * (pad_h + gap), side_w, pad_h);
            if button_cell(ui, r, ctl, &info(ctl), down(ctl), sel == Some(ctl)) {
                clicked = Some(ctl);
            }
        }
        // トラックボタン(左から1..8)とSHIFT
        for i in 0..8u8 {
            let ctl = Ctl::Track(i);
            let r = at(i as f32 * (pad_w + gap), track_y, pad_w, track_h);
            if button_cell(ui, r, ctl, &info(ctl), down(ctl), sel == Some(ctl)) {
                clicked = Some(ctl);
            }
        }
        let r = at(side_x, track_y, side_w, track_h);
        if button_cell(ui, r, Ctl::Shift, &info(Ctl::Shift), down(Ctl::Shift), sel == Some(Ctl::Shift)) {
            clicked = Some(Ctl::Shift);
        }
        // フェーダー(左から1..8、9がマスター)
        for i in 0..9u8 {
            let ctl = Ctl::Fader(i);
            let (x, w) = if i < 8 { (i as f32 * (pad_w + gap), pad_w) } else { (side_x, side_w) };
            let r = at(x, fader_y, w, fader_h);
            if fader_cell(ui, r, ctl, &info(ctl), rt.cc_value[ctl.trigger().1 as usize], sel == Some(ctl)) {
                clicked = Some(ctl);
            }
        }
    });
    clicked
}
