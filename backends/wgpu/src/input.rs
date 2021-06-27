use winit::event::{ModifiersState, VirtualKeyCode};
use zi::terminal::Key;

#[inline]
pub fn map_key(key: VirtualKeyCode, modifiers: &ModifiersState) -> Option<Key> {
    match key {
        VirtualKeyCode::Back => Some(Key::Backspace),
        VirtualKeyCode::Left => Some(Key::Left),
        VirtualKeyCode::Right => Some(Key::Right),
        VirtualKeyCode::Up => Some(Key::Up),
        VirtualKeyCode::Down => Some(Key::Down),
        VirtualKeyCode::Home => Some(Key::Home),
        VirtualKeyCode::End => Some(Key::End),
        VirtualKeyCode::PageUp => Some(Key::PageUp),
        VirtualKeyCode::PageDown => Some(Key::PageDown),
        VirtualKeyCode::Delete => Some(Key::Delete),
        VirtualKeyCode::Insert => Some(Key::Insert),
        VirtualKeyCode::Escape => Some(Key::Esc),
        maybe_char => map_char(maybe_char).map(|character| {
            if modifiers.ctrl() {
                Key::Ctrl(character)
            } else if modifiers.alt() {
                Key::Alt(character)
            } else {
                Key::Char(character)
            }
        }),
    }
}

#[inline]
pub fn map_char(key: VirtualKeyCode) -> Option<char> {
    Some(match key {
        VirtualKeyCode::A => 'a',
        VirtualKeyCode::B => 'b',
        VirtualKeyCode::C => 'c',
        VirtualKeyCode::D => 'd',
        VirtualKeyCode::E => 'e',
        VirtualKeyCode::F => 'f',
        VirtualKeyCode::G => 'g',
        VirtualKeyCode::H => 'h',
        VirtualKeyCode::I => 'i',
        VirtualKeyCode::J => 'j',
        VirtualKeyCode::K => 'k',
        VirtualKeyCode::L => 'l',
        VirtualKeyCode::M => 'm',
        VirtualKeyCode::N => 'n',
        VirtualKeyCode::O => 'o',
        VirtualKeyCode::P => 'p',
        VirtualKeyCode::Q => 'q',
        VirtualKeyCode::R => 'r',
        VirtualKeyCode::S => 's',
        VirtualKeyCode::T => 't',
        VirtualKeyCode::U => 'u',
        VirtualKeyCode::V => 'v',
        VirtualKeyCode::W => 'w',
        VirtualKeyCode::X => 'x',
        VirtualKeyCode::Y => 'y',
        VirtualKeyCode::Z => 'z',
        VirtualKeyCode::Apostrophe => '\'',
        // VirtualKeyCode::Backquote => '`',
        VirtualKeyCode::Backslash => '\\',
        VirtualKeyCode::Comma => ',',
        VirtualKeyCode::Equals => '=',
        VirtualKeyCode::LBracket => '(',
        VirtualKeyCode::Minus => '-',
        VirtualKeyCode::Period => '.',
        VirtualKeyCode::RBracket => ')',
        VirtualKeyCode::Semicolon => ';',
        VirtualKeyCode::Slash => '/',
        VirtualKeyCode::Return => '\n',
        VirtualKeyCode::Space => ' ',
        VirtualKeyCode::Tab => '\t',
        _ => {
            return None;
        }
    })
}
