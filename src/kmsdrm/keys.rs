//! SDLのキーを eguiのキーに直す

use eframe::egui::{Key, Modifiers};
use sdl2::keyboard::{Keycode, Mod, Scancode};

/// キーの位置(USキーボードでの場所)。アプリのキー割り当ては位置で決めるので、こちらが本命
pub fn physical(sc: Scancode) -> Option<Key> {
    use Scancode as S;
    Some(match sc {
        S::A => Key::A,
        S::B => Key::B,
        S::C => Key::C,
        S::D => Key::D,
        S::E => Key::E,
        S::F => Key::F,
        S::G => Key::G,
        S::H => Key::H,
        S::I => Key::I,
        S::J => Key::J,
        S::K => Key::K,
        S::L => Key::L,
        S::M => Key::M,
        S::N => Key::N,
        S::O => Key::O,
        S::P => Key::P,
        S::Q => Key::Q,
        S::R => Key::R,
        S::S => Key::S,
        S::T => Key::T,
        S::U => Key::U,
        S::V => Key::V,
        S::W => Key::W,
        S::X => Key::X,
        S::Y => Key::Y,
        S::Z => Key::Z,
        S::Num0 | S::Kp0 => Key::Num0,
        S::Num1 | S::Kp1 => Key::Num1,
        S::Num2 | S::Kp2 => Key::Num2,
        S::Num3 | S::Kp3 => Key::Num3,
        S::Num4 | S::Kp4 => Key::Num4,
        S::Num5 | S::Kp5 => Key::Num5,
        S::Num6 | S::Kp6 => Key::Num6,
        S::Num7 | S::Kp7 => Key::Num7,
        S::Num8 | S::Kp8 => Key::Num8,
        S::Num9 | S::Kp9 => Key::Num9,
        S::F1 => Key::F1,
        S::F2 => Key::F2,
        S::F3 => Key::F3,
        S::F4 => Key::F4,
        S::F5 => Key::F5,
        S::F6 => Key::F6,
        S::F7 => Key::F7,
        S::F8 => Key::F8,
        S::F9 => Key::F9,
        S::F10 => Key::F10,
        S::F11 => Key::F11,
        S::F12 => Key::F12,
        S::F13 => Key::F13,
        S::F14 => Key::F14,
        S::F15 => Key::F15,
        S::F16 => Key::F16,
        S::F17 => Key::F17,
        S::F18 => Key::F18,
        S::F19 => Key::F19,
        S::F20 => Key::F20,
        S::F21 => Key::F21,
        S::F22 => Key::F22,
        S::F23 => Key::F23,
        S::F24 => Key::F24,
        S::Escape => Key::Escape,
        S::Tab => Key::Tab,
        S::Backspace => Key::Backspace,
        S::Return | S::KpEnter | S::Return2 => Key::Enter,
        S::Space => Key::Space,
        S::Insert => Key::Insert,
        S::Delete => Key::Delete,
        S::Home => Key::Home,
        S::End => Key::End,
        S::PageUp => Key::PageUp,
        S::PageDown => Key::PageDown,
        S::Left => Key::ArrowLeft,
        S::Right => Key::ArrowRight,
        S::Up => Key::ArrowUp,
        S::Down => Key::ArrowDown,
        S::Minus | S::KpMinus => Key::Minus,
        S::Equals | S::KpEquals => Key::Equals,
        S::KpPlus => Key::Plus,
        S::LeftBracket => Key::OpenBracket,
        S::RightBracket => Key::CloseBracket,
        S::Backslash | S::NonUsHash => Key::Backslash,
        S::Semicolon => Key::Semicolon,
        S::Apostrophe => Key::Quote,
        S::Grave => Key::Backtick,
        S::Comma | S::KpComma => Key::Comma,
        S::Period | S::KpPeriod => Key::Period,
        S::Slash | S::KpDivide => Key::Slash,
        S::Copy => Key::Copy,
        S::Cut => Key::Cut,
        S::Paste => Key::Paste,
        _ => return None,
    })
}

/// キーに書いてある文字(配列によって変わる)。文字でないキーは位置と同じ
pub fn logical(kc: Option<Keycode>, sc: Option<Scancode>) -> Option<Key> {
    // 文字のキーは、キーコードがその文字(Unicode)になっている
    let ch = kc.and_then(|k| u32::try_from(k.into_i32()).ok()).filter(|&c| c > 0x20 && c < 0x4000_0000).and_then(char::from_u32);
    if let Some(key) = ch.and_then(|c| Key::from_name(&c.to_string())) {
        return Some(key);
    }
    sc.and_then(physical)
}

pub fn modifiers(m: Mod) -> Modifiers {
    let ctrl = m.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD);
    Modifiers {
        alt: m.intersects(Mod::LALTMOD | Mod::RALTMOD),
        ctrl,
        shift: m.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
        mac_cmd: false,
        command: ctrl,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_tab_keys_are_mapped() {
        // キーボードタブで使うキー(数字・QWERTY・ASDF・ZXCVの段)は、全部位置で取れる
        for sc in [Scancode::Num1, Scancode::Q, Scancode::Semicolon, Scancode::Slash, Scancode::Comma, Scancode::Period] {
            assert!(physical(sc).is_some(), "{sc:?}");
        }
        assert_eq!(physical(Scancode::Num0), Some(Key::Num0));
        assert_eq!(physical(Scancode::Escape), Some(Key::Escape));
    }

    #[test]
    fn logical_follows_the_letter_on_the_key() {
        // AZERTY配列: Aの位置(Q)を押すと文字は a
        assert_eq!(logical(Some(Keycode::A), Some(Scancode::Q)), Some(Key::A));
        assert_eq!(logical(Some(Keycode::RETURN), Some(Scancode::Return)), Some(Key::Enter));
        assert_eq!(logical(None, Some(Scancode::Left)), Some(Key::ArrowLeft));
    }

    #[test]
    fn ctrl_is_command() {
        let m = modifiers(Mod::LCTRLMOD | Mod::RSHIFTMOD);
        assert!(m.ctrl && m.command && m.shift && !m.alt);
    }
}
