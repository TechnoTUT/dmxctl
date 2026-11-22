//! 見た目(色・フォント・余白)。暗い会場でも目に優しい、黒基調のテーマ。
//! 画面の部品はここの色と`card`・`help`などを使って、全体の見た目をそろえる

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Margin, RichText, Stroke, TextStyle};
use std::sync::Arc;

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

/// 画面の地
pub const BG: Color32 = rgb(0x0F, 0x11, 0x16);
/// 上下のバー
pub const BAR: Color32 = rgb(0x14, 0x17, 0x1D);
/// カード(設定のまとまり)
pub const CARD: Color32 = rgb(0x18, 0x1C, 0x23);
pub const LINE: Color32 = rgb(0x27, 0x2C, 0x36);
/// ボタンなど
pub const WIDGET: Color32 = rgb(0x23, 0x28, 0x31);
pub const WIDGET_HOVER: Color32 = rgb(0x2D, 0x33, 0x3E);
pub const WIDGET_ACTIVE: Color32 = rgb(0x38, 0x3F, 0x4C);
pub const TEXT: Color32 = rgb(0xE7, 0xEA, 0xF0);
pub const TEXT_DIM: Color32 = rgb(0x8C, 0x94, 0xA3);
pub const TEXT_FAINT: Color32 = rgb(0x5A, 0x61, 0x6E);
/// ロゴの赤。塗りには、透明度を付けるか薄めて利用する
pub const BRAND: Color32 = rgb(0xC7, 0x00, 0x0A);
/// 選択中のタブなど。黒い地で文字が読めるよう、BRAND を4割ほど白に寄せた色
pub const ACCENT: Color32 = rgb(0xDD, 0x66, 0x6A);
pub const OK: Color32 = rgb(0x4A, 0xDE, 0x80);
pub const WARN: Color32 = rgb(0xF5, 0xA5, 0x24);
pub const DANGER: Color32 = rgb(0xEF, 0x44, 0x44);

/// 太字(見出し・パッドの文字)のフォント
pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("bold".into()))
}

/// 透明度を付けた色(背景のうっすらした色付けに)
pub fn alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0) as u8)
}

/// 2色を混ぜる(t=0でa、1でb)
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// 背景の明るさに合わせた文字の色(白か黒)
pub fn text_on(fill: Color32) -> Color32 {
    let lum = 0.3 * fill.r() as f32 + 0.59 * fill.g() as f32 + 0.11 * fill.b() as f32;
    if lum > 150.0 { rgb(0x10, 0x12, 0x16) } else { Color32::WHITE }
}

/// 設定のまとまりを囲むカード
pub fn card() -> egui::Frame {
    egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0, LINE)).corner_radius(10).inner_margin(Margin::symmetric(12, 10))
}

/// 見出し
pub fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).font(bold(16.0)).color(TEXT));
}

/// 小さな「?」。説明はマウスを乗せたときだけ出す(画面に長い説明文を並べない)
pub fn help(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
    let hovered = resp.hovered();
    let p = ui.painter();
    p.rect_stroke(egui::Rect::from_center_size(rect.center(), egui::vec2(15.0, 15.0)), 3.0, Stroke::new(1.2, if hovered { ACCENT } else { TEXT_FAINT }), egui::StrokeKind::Inside);
    p.text(rect.center(), egui::Align2::CENTER_CENTER, "?", bold(11.0), if hovered { ACCENT } else { TEXT_DIM });
    resp.on_hover_text(text)
}

/// 状態を表す小さな四角
pub fn dot(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 16.0), egui::Sense::hover());
    ui.painter().rect_filled(egui::Rect::from_center_size(rect.center(), egui::vec2(7.0, 7.0)), 1.5, color);
}

/// 日本語フォント(OS標準)を読み込む。太字は見出しとパッドの文字に使う
pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    // (通常, 太字)の候補。Windows: メイリオ / Mac: ヒラギノ角ゴ / Linux: Noto CJK
    const JP_FONTS: &[(&str, &str)] = &[
        ("C:/Windows/Fonts/meiryo.ttc", "C:/Windows/Fonts/meiryob.ttc"),
        ("C:/Windows/Fonts/YuGothM.ttc", "C:/Windows/Fonts/YuGothB.ttc"),
        ("C:/Windows/Fonts/msgothic.ttc", ""),
        ("/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", "/System/Library/Fonts/ヒラギノ角ゴシック W6.ttc"),
        ("/System/Library/Fonts/Hiragino Sans GB.ttc", ""),
        ("/Library/Fonts/Arial Unicode.ttf", ""),
        ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", "/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc"),
        ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", "/usr/share/fonts/noto-cjk/NotoSansCJK-Bold.ttc"),
        ("/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc", "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Bold.ttc"),
        ("/usr/share/fonts/google-noto-sans-cjk-fonts/NotoSansCJK-Regular.ttc", "/usr/share/fonts/google-noto-sans-cjk-fonts/NotoSansCJK-Bold.ttc"),
        ("/usr/share/fonts/opentype/ipafont-gothic/ipagp.ttf", ""),
        ("/usr/share/fonts/truetype/takao-gothic/TakaoPGothic.ttf", ""),
        ("/usr/share/fonts/truetype/vlgothic/VL-PGothic-Regular.ttf", ""),
    ];
    // Linuxで上のどれも無いときは、fontconfigに日本語フォントを聞く
    let found = std::iter::once_with(fontconfig_jp).flatten().map(|f| (f, String::new()));
    let candidates = JP_FONTS.iter().map(|&(r, b)| (r.to_string(), b.to_string())).chain(found);
    let mut bold_list: Vec<String> = Vec::new();
    for (regular, bold) in candidates {
        let Ok(bytes) = std::fs::read(&regular) else { continue };
        fonts.font_data.insert("jp".into(), Arc::new(egui::FontData::from_owned(bytes)));
        // 通常の文字は日本語フォントを先頭にして字の太さを揃える。等幅(数値表示)は既定フォントを優先し、日本語だけ補う
        if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
            list.insert(0, "jp".into());
        }
        if let Some(list) = fonts.families.get_mut(&FontFamily::Monospace) {
            list.push("jp".into());
        }
        if let Ok(b) = std::fs::read(&bold) {
            fonts.font_data.insert("jp_bold".into(), Arc::new(egui::FontData::from_owned(b)));
            bold_list.push("jp_bold".into());
        }
        bold_list.push("jp".into());
        break;
    }
    // 太字が無ければ通常の文字で代わりにする
    let mut fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    bold_list.append(&mut fallback);
    bold_list.dedup();
    fonts.families.insert(FontFamily::Name("bold".into()), bold_list);
    ctx.set_fonts(fonts);
}

/// fontconfig(fc-match)が選ぶ日本語フォントのファイル。Linux以外・見つからないときはNone
fn fontconfig_jp() -> Option<String> {
    if cfg!(any(windows, target_os = "macos")) {
        return None;
    }
    let out = std::process::Command::new("fc-match").args(["-f", "%{file}", "sans-serif:lang=ja"]).output().ok()?;
    let path = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!path.is_empty()).then_some(path)
}

/// 色・余白・角の丸みを設定する(OSの明るい/暗い設定によらず、常に暗いテーマ)
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Heading, bold(18.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();

        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(8.0, 7.0);
        s.button_padding = egui::vec2(10.0, 4.0);
        s.interact_size = egui::vec2(36.0, 26.0);
        s.window_margin = Margin::same(12);
        s.menu_margin = Margin::same(8);
        s.slider_width = 160.0;
        s.combo_width = 120.0;
        s.icon_width = 16.0;
        s.icon_width_inner = 9.0;
        s.indent = 18.0;
        s.scroll = egui::style::ScrollStyle::floating();

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.override_text_color = None;
        v.panel_fill = BG;
        v.window_fill = BAR;
        v.window_stroke = Stroke::new(1.0, LINE);
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(8);
        v.extreme_bg_color = rgb(0x0B, 0x0D, 0x11);
        v.faint_bg_color = rgb(0x16, 0x19, 0x20);
        v.code_bg_color = WIDGET;
        v.hyperlink_color = ACCENT;
        v.warn_fg_color = WARN;
        v.error_fg_color = DANGER;
        v.slider_trailing_fill = true;
        v.indent_has_left_vline = false;
        v.collapsing_header_frame = false;
        v.selection.bg_fill = alpha(ACCENT, 0.22);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.popup_shadow = egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(110) };
        v.window_shadow = v.popup_shadow;

        // 小さいチェックボックスが丸く見えないよう、角の丸みは控えめに
        let corner = CornerRadius::same(4);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = CARD;
        w.noninteractive.weak_bg_fill = CARD;
        w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.noninteractive.corner_radius = corner;
        w.inactive.bg_fill = WIDGET;
        w.inactive.weak_bg_fill = WIDGET;
        w.inactive.bg_stroke = Stroke::new(1.0, rgb(0x2C, 0x32, 0x3C));
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.corner_radius = corner;
        w.hovered.bg_fill = WIDGET_HOVER;
        w.hovered.weak_bg_fill = WIDGET_HOVER;
        w.hovered.bg_stroke = Stroke::new(1.0, rgb(0x3E, 0x46, 0x54));
        w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        w.hovered.corner_radius = corner;
        w.hovered.expansion = 0.0;
        w.active.bg_fill = WIDGET_ACTIVE;
        w.active.weak_bg_fill = WIDGET_ACTIVE;
        w.active.bg_stroke = Stroke::new(1.0, ACCENT);
        w.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        w.active.corner_radius = corner;
        w.active.expansion = 0.0;
        w.open.bg_fill = WIDGET_HOVER;
        w.open.weak_bg_fill = WIDGET_HOVER;
        w.open.bg_stroke = Stroke::new(1.0, rgb(0x3E, 0x46, 0x54));
        w.open.fg_stroke = Stroke::new(1.0, TEXT);
        w.open.corner_radius = corner;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_stays_readable_on_bright_and_dark_fills() {
        assert_eq!(text_on(Color32::WHITE), rgb(0x10, 0x12, 0x16));
        assert_eq!(text_on(Color32::from_rgb(255, 230, 0)), rgb(0x10, 0x12, 0x16));
        assert_eq!(text_on(BG), Color32::WHITE);
        assert_eq!(text_on(Color32::from_rgb(0, 0, 255)), Color32::WHITE);
        assert_eq!(mix(Color32::BLACK, Color32::WHITE, 0.5), Color32::from_gray(128));
    }
}
