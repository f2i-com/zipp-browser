//! Linux evdev key codes (`linux/input-event-codes.h`) to winit keys, with a
//! US layout for the characters they type.

use winit_core::keyboard::{Key, KeyCode, KeyLocation, NamedKey, NativeKey, NativeKeyCode, PhysicalKey};

pub(crate) struct Mapped {
    pub physical: PhysicalKey,
    pub location: KeyLocation,
    /// What the key types, unshifted and shifted, when it types anything.
    pub chars: Option<(char, char)>,
    pub named: Option<NamedKey>,
}

impl Mapped {
    /// The logical key with Shift (and Caps Lock, for letters) applied.
    pub fn logical(&self, shift: bool, caps: bool) -> Key {
        match (self.chars, self.named) {
            (Some(chars), _) => Key::Character(self.char(chars, shift, caps).to_string().into()),
            (None, Some(named)) => Key::Named(named),
            (None, None) => match self.physical {
                PhysicalKey::Unidentified(NativeKeyCode::Xkb(code)) => Key::Unidentified(NativeKey::Xkb(code)),
                _ => Key::Unidentified(NativeKey::Unidentified),
            },
        }
    }

    pub fn text(&self, shift: bool, caps: bool) -> Option<char> {
        self.chars.map(|chars| self.char(chars, shift, caps))
    }

    fn char(&self, (lower, upper): (char, char), shift: bool, caps: bool) -> char {
        let shifted = if lower.is_ascii_alphabetic() { shift ^ caps } else { shift };
        if shifted { upper } else { lower }
    }
}

pub(crate) fn map(code: u16) -> Mapped {
    use KeyCode as C;
    use KeyLocation::{Left, Numpad, Right, Standard};
    use NamedKey as N;

    let (key, location, chars, named): (Option<KeyCode>, KeyLocation, Option<(char, char)>, Option<NamedKey>) =
        match code {
            1 => (Some(C::Escape), Standard, None, Some(N::Escape)),
            2..=11 => {
                const DIGITS: [(KeyCode, char, char); 10] = [
                    (KeyCode::Digit1, '1', '!'),
                    (KeyCode::Digit2, '2', '@'),
                    (KeyCode::Digit3, '3', '#'),
                    (KeyCode::Digit4, '4', '$'),
                    (KeyCode::Digit5, '5', '%'),
                    (KeyCode::Digit6, '6', '^'),
                    (KeyCode::Digit7, '7', '&'),
                    (KeyCode::Digit8, '8', '*'),
                    (KeyCode::Digit9, '9', '('),
                    (KeyCode::Digit0, '0', ')'),
                ];
                let (k, a, b) = DIGITS[(code - 2) as usize];
                (Some(k), Standard, Some((a, b)), None)
            }
            12 => (Some(C::Minus), Standard, Some(('-', '_')), None),
            13 => (Some(C::Equal), Standard, Some(('=', '+')), None),
            14 => (Some(C::Backspace), Standard, None, Some(N::Backspace)),
            15 => (Some(C::Tab), Standard, None, Some(N::Tab)),
            16..=25 | 30..=38 | 44..=50 => {
                let letter = match code {
                    16 => 'q', 17 => 'w', 18 => 'e', 19 => 'r', 20 => 't',
                    21 => 'y', 22 => 'u', 23 => 'i', 24 => 'o', 25 => 'p',
                    30 => 'a', 31 => 's', 32 => 'd', 33 => 'f', 34 => 'g',
                    35 => 'h', 36 => 'j', 37 => 'k', 38 => 'l',
                    44 => 'z', 45 => 'x', 46 => 'c', 47 => 'v', 48 => 'b',
                    49 => 'n', _ => 'm',
                };
                (Some(letter_code(letter)), Standard, Some((letter, letter.to_ascii_uppercase())), None)
            }
            26 => (Some(C::BracketLeft), Standard, Some(('[', '{')), None),
            27 => (Some(C::BracketRight), Standard, Some((']', '}')), None),
            28 => (Some(C::Enter), Standard, None, Some(N::Enter)),
            29 => (Some(C::ControlLeft), Left, None, Some(N::Control)),
            39 => (Some(C::Semicolon), Standard, Some((';', ':')), None),
            40 => (Some(C::Quote), Standard, Some(('\'', '"')), None),
            41 => (Some(C::Backquote), Standard, Some(('`', '~')), None),
            42 => (Some(C::ShiftLeft), Left, None, Some(N::Shift)),
            43 => (Some(C::Backslash), Standard, Some(('\\', '|')), None),
            51 => (Some(C::Comma), Standard, Some((',', '<')), None),
            52 => (Some(C::Period), Standard, Some(('.', '>')), None),
            53 => (Some(C::Slash), Standard, Some(('/', '?')), None),
            54 => (Some(C::ShiftRight), Right, None, Some(N::Shift)),
            55 => (Some(C::NumpadMultiply), Numpad, Some(('*', '*')), None),
            56 => (Some(C::AltLeft), Left, None, Some(N::Alt)),
            57 => (Some(C::Space), Standard, Some((' ', ' ')), None),
            58 => (Some(C::CapsLock), Standard, None, Some(N::CapsLock)),
            59..=68 => {
                const F: [(KeyCode, NamedKey); 10] = [
                    (KeyCode::F1, NamedKey::F1),
                    (KeyCode::F2, NamedKey::F2),
                    (KeyCode::F3, NamedKey::F3),
                    (KeyCode::F4, NamedKey::F4),
                    (KeyCode::F5, NamedKey::F5),
                    (KeyCode::F6, NamedKey::F6),
                    (KeyCode::F7, NamedKey::F7),
                    (KeyCode::F8, NamedKey::F8),
                    (KeyCode::F9, NamedKey::F9),
                    (KeyCode::F10, NamedKey::F10),
                ];
                let (k, n) = F[(code - 59) as usize];
                (Some(k), Standard, None, Some(n))
            }
            69 => (Some(C::NumLock), Numpad, None, Some(N::NumLock)),
            70 => (Some(C::ScrollLock), Standard, None, Some(N::ScrollLock)),
            71 => (Some(C::Numpad7), Numpad, Some(('7', '7')), None),
            72 => (Some(C::Numpad8), Numpad, Some(('8', '8')), None),
            73 => (Some(C::Numpad9), Numpad, Some(('9', '9')), None),
            74 => (Some(C::NumpadSubtract), Numpad, Some(('-', '-')), None),
            75 => (Some(C::Numpad4), Numpad, Some(('4', '4')), None),
            76 => (Some(C::Numpad5), Numpad, Some(('5', '5')), None),
            77 => (Some(C::Numpad6), Numpad, Some(('6', '6')), None),
            78 => (Some(C::NumpadAdd), Numpad, Some(('+', '+')), None),
            79 => (Some(C::Numpad1), Numpad, Some(('1', '1')), None),
            80 => (Some(C::Numpad2), Numpad, Some(('2', '2')), None),
            81 => (Some(C::Numpad3), Numpad, Some(('3', '3')), None),
            82 => (Some(C::Numpad0), Numpad, Some(('0', '0')), None),
            83 => (Some(C::NumpadDecimal), Numpad, Some(('.', '.')), None),
            86 => (Some(C::IntlBackslash), Standard, Some(('\\', '|')), None),
            87 => (Some(C::F11), Standard, None, Some(N::F11)),
            88 => (Some(C::F12), Standard, None, Some(N::F12)),
            96 => (Some(C::NumpadEnter), Numpad, None, Some(N::Enter)),
            97 => (Some(C::ControlRight), Right, None, Some(N::Control)),
            98 => (Some(C::NumpadDivide), Numpad, Some(('/', '/')), None),
            99 => (Some(C::PrintScreen), Standard, None, Some(N::PrintScreen)),
            100 => (Some(C::AltRight), Right, None, Some(N::Alt)),
            102 => (Some(C::Home), Standard, None, Some(N::Home)),
            103 => (Some(C::ArrowUp), Standard, None, Some(N::ArrowUp)),
            104 => (Some(C::PageUp), Standard, None, Some(N::PageUp)),
            105 => (Some(C::ArrowLeft), Standard, None, Some(N::ArrowLeft)),
            106 => (Some(C::ArrowRight), Standard, None, Some(N::ArrowRight)),
            107 => (Some(C::End), Standard, None, Some(N::End)),
            108 => (Some(C::ArrowDown), Standard, None, Some(N::ArrowDown)),
            109 => (Some(C::PageDown), Standard, None, Some(N::PageDown)),
            110 => (Some(C::Insert), Standard, None, Some(N::Insert)),
            111 => (Some(C::Delete), Standard, None, Some(N::Delete)),
            113 => (Some(C::AudioVolumeMute), Standard, None, Some(N::AudioVolumeMute)),
            114 => (Some(C::AudioVolumeDown), Standard, None, Some(N::AudioVolumeDown)),
            115 => (Some(C::AudioVolumeUp), Standard, None, Some(N::AudioVolumeUp)),
            116 => (Some(C::Power), Standard, None, Some(N::Power)),
            119 => (Some(C::Pause), Standard, None, Some(N::Pause)),
            125 => (Some(C::MetaLeft), Left, None, Some(N::Meta)),
            126 => (Some(C::MetaRight), Right, None, Some(N::Meta)),
            127 => (Some(C::ContextMenu), Standard, None, Some(N::ContextMenu)),
            158 => (Some(C::BrowserBack), Standard, None, Some(N::BrowserBack)),
            159 => (Some(C::BrowserForward), Standard, None, Some(N::BrowserForward)),
            173 => (Some(C::BrowserRefresh), Standard, None, Some(N::BrowserRefresh)),
            _ => (None, Standard, None, None),
        };
    Mapped {
        // XKB numbers keys as evdev does, plus 8.
        physical: key.map(PhysicalKey::Code).unwrap_or(PhysicalKey::Unidentified(NativeKeyCode::Xkb(code as u32 + 8))),
        location,
        chars,
        named,
    }
}

fn letter_code(letter: char) -> KeyCode {
    use KeyCode as C;
    match letter {
        'a' => C::KeyA, 'b' => C::KeyB, 'c' => C::KeyC, 'd' => C::KeyD, 'e' => C::KeyE,
        'f' => C::KeyF, 'g' => C::KeyG, 'h' => C::KeyH, 'i' => C::KeyI, 'j' => C::KeyJ,
        'k' => C::KeyK, 'l' => C::KeyL, 'm' => C::KeyM, 'n' => C::KeyN, 'o' => C::KeyO,
        'p' => C::KeyP, 'q' => C::KeyQ, 'r' => C::KeyR, 's' => C::KeyS, 't' => C::KeyT,
        'u' => C::KeyU, 'v' => C::KeyV, 'w' => C::KeyW, 'x' => C::KeyX, 'y' => C::KeyY,
        _ => C::KeyZ,
    }
}
