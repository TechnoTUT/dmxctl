//! 指の入力(タッチパネル・トラックパッド)を、マウスの操作に直す。
//! SDL2 の KMSDRM(evdev)は、タッチパネルもトラックパッドも「指の入力」として送るだけで、
//! カーソルを動かしたりクリックにしたりはしないので、ここで行う。
//! - タッチパネル: 触った場所を左クリック(押している間はドラッグ)
//! - トラックパッド: 1本指でカーソルを動かす、軽く叩くと左クリック(2本指で叩くと右クリック)、2本指でスクロール。
//!   押し込むクリック(クリックパッドのボタン)は、SDLがマウスのボタンとして送ってくる

use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// タッチパネル(画面の上の位置)
    Screen,
    /// トラックパッド(画面とは別の板。動いた量だけ使う)
    Pad,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    /// カーソルをこの位置(画面のピクセル)へ
    MoveTo(f32, f32),
    /// 今のカーソルの位置でボタンを押す/離す
    Button { pressed: bool, secondary: bool },
    /// スクロール(画面のピクセル。指の動きに中身がついてくる向き)
    Scroll(f32, f32),
}

/// トラックパッドの端から端まで動かしたときに、画面の幅の何倍カーソルが動くか
const PAD_SPEED: f32 = 1.0;
/// これより短く、ほとんど動かさずに離したら「叩いた」(クリック)
const TAP_TIME: Duration = Duration::from_millis(250);
/// 叩いたとみなす動きの大きさ(パッドの大きさに対する割合)
const TAP_MOVE: f32 = 0.03;

#[derive(Default)]
struct Pad {
    /// 今触れている指の数
    fingers: u32,
    /// 今回触れている間の、指の数の最大(2本ならスクロール・右クリック)
    max: u32,
    started: Option<Instant>,
    /// 今回触れている間に動いた量
    travel: f32,
}

pub struct Touch {
    /// 画面の大きさ(ピクセル)
    w: f32,
    h: f32,
    cursor: (f32, f32),
    /// タッチパネルでクリックに使っている指(装置, 指)
    screen_finger: Option<(i64, i64)>,
    pad: Pad,
}

impl Touch {
    pub fn new(w: f32, h: f32) -> Self {
        Touch { w, h, cursor: (w / 2.0, h / 2.0), screen_finger: None, pad: Pad::default() }
    }

    /// マウスが動いたとき(SDLのカーソルの位置)
    pub fn set_cursor(&mut self, x: f32, y: f32) {
        self.cursor = (x, y);
    }

    pub fn cursor(&self) -> (f32, f32) {
        self.cursor
    }

    fn move_to(&mut self, x: f32, y: f32) -> Action {
        self.cursor = (x.clamp(0.0, (self.w - 1.0).max(0.0)), y.clamp(0.0, (self.h - 1.0).max(0.0)));
        Action::MoveTo(self.cursor.0, self.cursor.1)
    }

    /// 指が触れた(x, y は装置の上の位置 0〜1)
    pub fn down(&mut self, kind: Kind, id: (i64, i64), x: f32, y: f32, now: Instant) -> Vec<Action> {
        match kind {
            Kind::Screen => {
                if self.screen_finger.is_some() {
                    return Vec::new();
                }
                self.screen_finger = Some(id);
                vec![self.move_to(x * self.w, y * self.h), Action::Button { pressed: true, secondary: false }]
            }
            Kind::Pad => {
                let p = &mut self.pad;
                if p.fingers == 0 {
                    *p = Pad { started: Some(now), ..Pad::default() };
                }
                p.fingers += 1;
                p.max = p.max.max(p.fingers);
                Vec::new()
            }
        }
    }

    /// 指が動いた(dx, dy は装置の上で動いた量 0〜1)
    pub fn motion(&mut self, kind: Kind, id: (i64, i64), x: f32, y: f32, dx: f32, dy: f32) -> Vec<Action> {
        match kind {
            Kind::Screen => {
                if self.screen_finger != Some(id) {
                    return Vec::new();
                }
                vec![self.move_to(x * self.w, y * self.h)]
            }
            Kind::Pad => {
                let p = &mut self.pad;
                p.travel += dx.hypot(dy);
                if p.max >= 2 {
                    // 2本指: スクロール(指それぞれから届くので、本数で割る)。一度2本で触れたら、離すまでカーソルは動かさない
                    let n = p.fingers.max(1) as f32;
                    return vec![Action::Scroll(dx * self.w / n, dy * self.h / n)];
                }
                let (cx, cy) = self.cursor;
                vec![self.move_to(cx + dx * self.w * PAD_SPEED, cy + dy * self.h * PAD_SPEED)]
            }
        }
    }

    /// 指が離れた
    pub fn up(&mut self, kind: Kind, id: (i64, i64), x: f32, y: f32, now: Instant) -> Vec<Action> {
        match kind {
            Kind::Screen => {
                if self.screen_finger != Some(id) {
                    return Vec::new();
                }
                self.screen_finger = None;
                vec![self.move_to(x * self.w, y * self.h), Action::Button { pressed: false, secondary: false }]
            }
            Kind::Pad => {
                let p = &mut self.pad;
                p.fingers = p.fingers.saturating_sub(1);
                if p.fingers > 0 {
                    return Vec::new();
                }
                let tapped = p.started.is_some_and(|t| now.duration_since(t) < TAP_TIME) && p.travel < TAP_MOVE;
                let secondary = p.max >= 2;
                *p = Pad::default();
                if tapped {
                    vec![Action::Button { pressed: true, secondary }, Action::Button { pressed: false, secondary }]
                } else {
                    Vec::new()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const F1: (i64, i64) = (1, 0);
    const F2: (i64, i64) = (1, 1);

    #[test]
    fn touchscreen_clicks_where_touched() {
        let mut t = Touch::new(1000.0, 500.0);
        let now = Instant::now();
        assert_eq!(t.down(Kind::Screen, F1, 0.5, 0.5, now), vec![Action::MoveTo(500.0, 250.0), Action::Button { pressed: true, secondary: false }]);
        // 2本目の指は無視する
        assert!(t.down(Kind::Screen, F2, 0.1, 0.1, now).is_empty());
        assert_eq!(t.motion(Kind::Screen, F1, 0.6, 0.5, 0.1, 0.0), vec![Action::MoveTo(600.0, 250.0)]);
        assert_eq!(t.up(Kind::Screen, F1, 0.6, 0.5, now), vec![Action::MoveTo(600.0, 250.0), Action::Button { pressed: false, secondary: false }]);
    }

    #[test]
    fn trackpad_moves_the_cursor_by_the_amount_moved() {
        let mut t = Touch::new(1000.0, 500.0);
        t.set_cursor(100.0, 100.0);
        let now = Instant::now();
        assert!(t.down(Kind::Pad, F1, 0.9, 0.9, now).is_empty());
        // 触れた場所ではなく、動いた量だけ動く
        assert_eq!(t.motion(Kind::Pad, F1, 0.95, 0.9, 0.05, 0.0), vec![Action::MoveTo(150.0, 100.0)]);
        // 画面の外には出ない
        assert_eq!(t.motion(Kind::Pad, F1, 0.95, 0.0, 0.0, -0.9), vec![Action::MoveTo(150.0, 0.0)]);
        // 長く動かしたあとに離しても、クリックにはならない
        assert!(t.up(Kind::Pad, F1, 0.95, 0.0, now + Duration::from_secs(1)).is_empty());
    }

    #[test]
    fn trackpad_tap_clicks() {
        let mut t = Touch::new(1000.0, 500.0);
        let now = Instant::now();
        t.down(Kind::Pad, F1, 0.5, 0.5, now);
        t.motion(Kind::Pad, F1, 0.501, 0.5, 0.001, 0.0);
        let click = t.up(Kind::Pad, F1, 0.501, 0.5, now + Duration::from_millis(100));
        assert_eq!(click, vec![Action::Button { pressed: true, secondary: false }, Action::Button { pressed: false, secondary: false }]);
        // ゆっくり離したら(押したまま考えていた)クリックにしない
        t.down(Kind::Pad, F1, 0.5, 0.5, now);
        assert!(t.up(Kind::Pad, F1, 0.5, 0.5, now + Duration::from_millis(600)).is_empty());
    }

    #[test]
    fn trackpad_two_fingers_scroll_and_right_click() {
        let mut t = Touch::new(1000.0, 500.0);
        t.set_cursor(100.0, 100.0);
        let now = Instant::now();
        t.down(Kind::Pad, F1, 0.5, 0.5, now);
        t.down(Kind::Pad, F2, 0.6, 0.5, now);
        assert_eq!(t.motion(Kind::Pad, F1, 0.5, 0.6, 0.0, 0.1), vec![Action::Scroll(0.0, 25.0)]);
        // 1本離しても、残りの指でカーソルは動かない
        assert!(t.up(Kind::Pad, F2, 0.6, 0.6, now).is_empty());
        assert_eq!(t.motion(Kind::Pad, F1, 0.5, 0.7, 0.0, 0.1), vec![Action::Scroll(0.0, 50.0)]);
        assert_eq!(t.cursor(), (100.0, 100.0));
        t.up(Kind::Pad, F1, 0.5, 0.7, now + Duration::from_secs(1));
        // 2本指で軽く叩くと右クリック
        t.down(Kind::Pad, F1, 0.5, 0.5, now);
        t.down(Kind::Pad, F2, 0.6, 0.5, now);
        assert!(t.up(Kind::Pad, F1, 0.5, 0.5, now).is_empty());
        let click = t.up(Kind::Pad, F2, 0.6, 0.5, now + Duration::from_millis(100));
        assert_eq!(click, vec![Action::Button { pressed: true, secondary: true }, Action::Button { pressed: false, secondary: true }]);
    }
}
