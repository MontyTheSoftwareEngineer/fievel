//! Layout-aware translation between physical key codes and canonical key codes.
//!
//! Fievel works internally on canonical Linux `KeyCode`s. Each platform boundary
//! translates a physical key code (a Quartz virtual key code, a Windows scan code,
//! or an evdev key code) into a canonical key and back. This module owns the
//! canonical character tables and the shared translation logic so every platform
//! behaves the same way.

use crate::input::KeyCode as K;
use std::{collections::BTreeMap, io};

/// The character a key produces without Shift under a US layout.
///
/// This is the canonical table used for output remaps and for the base-character
/// lookup on input. It deliberately excludes keys whose US character is only
/// reachable with Shift (digits' symbols, shifted punctuation) and non-letter
/// keys such as `KEY_102ND`.
pub fn key_symbol(key: K) -> Option<char> {
    Some(match key {
        K::KEY_A => 'a',
        K::KEY_B => 'b',
        K::KEY_C => 'c',
        K::KEY_D => 'd',
        K::KEY_E => 'e',
        K::KEY_F => 'f',
        K::KEY_G => 'g',
        K::KEY_H => 'h',
        K::KEY_I => 'i',
        K::KEY_J => 'j',
        K::KEY_K => 'k',
        K::KEY_L => 'l',
        K::KEY_M => 'm',
        K::KEY_N => 'n',
        K::KEY_O => 'o',
        K::KEY_P => 'p',
        K::KEY_Q => 'q',
        K::KEY_R => 'r',
        K::KEY_S => 's',
        K::KEY_T => 't',
        K::KEY_U => 'u',
        K::KEY_V => 'v',
        K::KEY_W => 'w',
        K::KEY_X => 'x',
        K::KEY_Y => 'y',
        K::KEY_Z => 'z',
        K::KEY_1 => '1',
        K::KEY_2 => '2',
        K::KEY_3 => '3',
        K::KEY_4 => '4',
        K::KEY_5 => '5',
        K::KEY_6 => '6',
        K::KEY_7 => '7',
        K::KEY_8 => '8',
        K::KEY_9 => '9',
        K::KEY_0 => '0',
        K::KEY_GRAVE => '`',
        K::KEY_MINUS => '-',
        K::KEY_EQUAL => '=',
        K::KEY_LEFTBRACE => '[',
        K::KEY_RIGHTBRACE => ']',
        K::KEY_BACKSLASH => '\\',
        K::KEY_SEMICOLON => ';',
        K::KEY_APOSTROPHE => '\'',
        K::KEY_COMMA => ',',
        K::KEY_DOT => '.',
        K::KEY_SLASH => '/',
        K::KEY_SPACE => ' ',
        _ => return None,
    })
}

/// The canonical key for a character produced without Shift.
///
/// Letters are matched case-insensitively so Caps Lock and shifted letters still
/// resolve to the same key.
pub fn unshifted_key(character: char) -> Option<K> {
    Some(match character {
        'a' | 'A' => K::KEY_A,
        'b' | 'B' => K::KEY_B,
        'c' | 'C' => K::KEY_C,
        'd' | 'D' => K::KEY_D,
        'e' | 'E' => K::KEY_E,
        'f' | 'F' => K::KEY_F,
        'g' | 'G' => K::KEY_G,
        'h' | 'H' => K::KEY_H,
        'i' | 'I' => K::KEY_I,
        'j' | 'J' => K::KEY_J,
        'k' | 'K' => K::KEY_K,
        'l' | 'L' => K::KEY_L,
        'm' | 'M' => K::KEY_M,
        'n' | 'N' => K::KEY_N,
        'o' | 'O' => K::KEY_O,
        'p' | 'P' => K::KEY_P,
        'q' | 'Q' => K::KEY_Q,
        'r' | 'R' => K::KEY_R,
        's' | 'S' => K::KEY_S,
        't' | 'T' => K::KEY_T,
        'u' | 'U' => K::KEY_U,
        'v' | 'V' => K::KEY_V,
        'w' | 'W' => K::KEY_W,
        'x' | 'X' => K::KEY_X,
        'y' | 'Y' => K::KEY_Y,
        'z' | 'Z' => K::KEY_Z,
        '1' => K::KEY_1,
        '2' => K::KEY_2,
        '3' => K::KEY_3,
        '4' => K::KEY_4,
        '5' => K::KEY_5,
        '6' => K::KEY_6,
        '7' => K::KEY_7,
        '8' => K::KEY_8,
        '9' => K::KEY_9,
        '0' => K::KEY_0,
        '`' => K::KEY_GRAVE,
        '-' => K::KEY_MINUS,
        '=' => K::KEY_EQUAL,
        '[' => K::KEY_LEFTBRACE,
        ']' => K::KEY_RIGHTBRACE,
        '\\' => K::KEY_BACKSLASH,
        ';' => K::KEY_SEMICOLON,
        '\'' => K::KEY_APOSTROPHE,
        ',' => K::KEY_COMMA,
        '.' => K::KEY_DOT,
        '/' => K::KEY_SLASH,
        ' ' => K::KEY_SPACE,
        _ => return None,
    })
}

/// The canonical key for a character produced with Shift under a US layout.
///
/// This is used when the base character of a physical key is not canonical: for
/// example an AZERTY digit row key produces `&` unshifted and `1` shifted, so the
/// shifted table maps `1` back to `KEY_1`.
pub fn shifted_key(character: char) -> Option<K> {
    Some(match character {
        '!' => K::KEY_1,
        '@' => K::KEY_2,
        '#' => K::KEY_3,
        '$' => K::KEY_4,
        '%' => K::KEY_5,
        '^' => K::KEY_6,
        '&' => K::KEY_7,
        '*' => K::KEY_8,
        '(' => K::KEY_9,
        ')' => K::KEY_0,
        '~' => K::KEY_GRAVE,
        '_' => K::KEY_MINUS,
        '+' => K::KEY_EQUAL,
        '{' => K::KEY_LEFTBRACE,
        '}' => K::KEY_RIGHTBRACE,
        '|' => K::KEY_BACKSLASH,
        ':' => K::KEY_SEMICOLON,
        '"' => K::KEY_APOSTROPHE,
        '<' => K::KEY_COMMA,
        '>' => K::KEY_DOT,
        '?' => K::KEY_SLASH,
        _ => return None,
    })
}

/// The canonical key for a character in either the unshifted or shifted table.
pub fn symbol_key(character: char) -> Option<K> {
    unshifted_key(character).or_else(|| shifted_key(character))
}

/// How a canonical key should be emitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Emission {
    /// Emit this physical key code, optionally pressing Shift first.
    Physical { keycode: u16, needs_shift: bool },
    /// Emit the canonical key through the platform's own physical table.
    Logical,
}

/// The destination of one emitted event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A physical key code from the active layout.
    Physical(u16),
    /// A canonical key for the platform's physical table to translate.
    Logical(K),
}

/// A platform's source of layout information.
///
/// Implementations translate physical key codes (Quartz virtual key codes on
/// macOS, scan codes on Windows, evdev key codes on Linux) into characters and
/// back. Every method must be cheap enough to call per key event; platforms that
/// need a keymap should cache it inside the implementation.
pub trait LayoutSource {
    /// The character the physical key produces without Shift.
    fn base_char(&self, keycode: u16) -> Option<char>;

    /// The character the physical key produces with Shift.
    fn shift_char(&self, keycode: u16) -> Option<char>;

    /// The physical key code and Shift requirement for a character.
    fn key_for_char(&self, character: char) -> Option<(u16, bool)>;

    /// The canonical key for a physical key code when no character matches.
    fn physical_key(&self, keycode: u16) -> Option<K>;
}

/// A layout source plus the canonical translation rules.
pub struct Layout<S> {
    source: S,
}

impl<S: LayoutSource> Layout<S> {
    pub fn new(source: S) -> Self {
        Self { source }
    }

    #[allow(dead_code)]
    pub fn source(&self) -> &S {
        &self.source
    }

    /// Translate a physical key code into a canonical key.
    ///
    /// The base character is preferred; if it is not canonical, the shifted
    /// character is tried; otherwise the platform's physical table is used.
    pub fn key_from_physical(&self, keycode: u16) -> Option<K> {
        if let Some(character) = self.source.base_char(keycode) {
            if let Some(key) = unshifted_key(character) {
                return Some(key);
            }
        }
        if let Some(character) = self.source.shift_char(keycode) {
            if let Some(key) = symbol_key(character) {
                return Some(key);
            }
        }
        self.source.physical_key(keycode)
    }

    /// How a canonical key should be emitted under the active layout.
    pub fn emission_for(&self, key: K) -> Emission {
        if let Some(character) = key_symbol(key) {
            if let Some((keycode, needs_shift)) = self.source.key_for_char(character) {
                return Emission::Physical {
                    keycode,
                    needs_shift,
                };
            }
        }
        Emission::Logical
    }
}

/// Tracks held keys and Shift so remap output follows the active layout.
///
/// The emitter translates canonical keys into physical key codes, synthesizes
/// Shift when a character requires it, and keeps the physical key code stable
/// for the whole press so releases and autorepeat use the same key.
pub struct LayoutEmitter<S> {
    layout: Layout<S>,
    held: BTreeMap<K, Emission>,
    synthetic_shift: bool,
    real_shift: bool,
}

impl<S: LayoutSource> LayoutEmitter<S> {
    pub fn new(layout: Layout<S>) -> Self {
        Self {
            layout,
            held: BTreeMap::new(),
            synthetic_shift: false,
            real_shift: false,
        }
    }

    #[allow(dead_code)]
    pub fn layout(&self) -> &Layout<S> {
        &self.layout
    }

    #[allow(dead_code)]
    /// Translate a physical key code into a canonical key.
    pub fn key_from_physical(&self, keycode: u16) -> Option<K> {
        self.layout.key_from_physical(keycode)
    }

    /// Emit one canonical key event, translating it through the active layout.
    pub fn emit<F>(&mut self, key: K, value: i32, sink: &mut F) -> io::Result<()>
    where
        F: FnMut(Target, i32) -> io::Result<()>,
    {
        if is_shift_key(key) {
            self.real_shift = value != 0;
            if value == 0 {
                self.synthetic_shift = false;
            }
            return sink(Target::Logical(key), value);
        }

        if value == 0 {
            let emission = self.held.remove(&key).unwrap_or(Emission::Logical);
            self.emit_one(emission, key, value, sink)?;
            return self.release_synthetic_shift(sink);
        }

        // Resolve the emission once per press so autorepeat and release reuse the
        // same physical key code even if the layout changes mid-press.
        let emission = match self.held.get(&key) {
            Some(emission) => *emission,
            None => {
                let emission = self.layout.emission_for(key);
                self.held.insert(key, emission);
                emission
            }
        };
        self.emit_one(emission, key, value, sink)
    }

    fn emit_one<F>(
        &mut self,
        emission: Emission,
        key: K,
        value: i32,
        sink: &mut F,
    ) -> io::Result<()>
    where
        F: FnMut(Target, i32) -> io::Result<()>,
    {
        match emission {
            Emission::Physical {
                keycode,
                needs_shift,
            } => {
                if needs_shift && value != 0 && !self.real_shift {
                    self.synthetic_shift = true;
                    sink(Target::Logical(K::KEY_LEFTSHIFT), 1)?;
                }
                sink(Target::Physical(keycode), value)
            }
            Emission::Logical => sink(Target::Logical(key), value),
        }
    }

    /// Release a synthetic Shift if no held key still needs it.
    pub fn release_synthetic_shift<F>(&mut self, sink: &mut F) -> io::Result<()>
    where
        F: FnMut(Target, i32) -> io::Result<()>,
    {
        if !self.synthetic_shift {
            return Ok(());
        }
        let still_needed = self.held.values().any(|emission| {
            matches!(
                emission,
                Emission::Physical {
                    needs_shift: true,
                    ..
                }
            )
        });
        if still_needed {
            return Ok(());
        }
        self.synthetic_shift = false;
        sink(Target::Logical(K::KEY_LEFTSHIFT), 0)
    }

    /// Resolve the physical key code for a canonical key without emitting it.
    #[allow(dead_code)]
    pub fn emission_for(&self, key: K) -> Emission {
        self.layout.emission_for(key)
    }
}

/// Whether a canonical key is a Shift key.
pub fn is_shift_key(key: K) -> bool {
    key == K::KEY_LEFTSHIFT || key == K::KEY_RIGHTSHIFT
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeLayout {
        base: BTreeMap<u16, char>,
        shifted: BTreeMap<u16, char>,
        physical: BTreeMap<u16, K>,
    }

    impl FakeLayout {
        fn new() -> Self {
            Self {
                base: BTreeMap::new(),
                shifted: BTreeMap::new(),
                physical: BTreeMap::new(),
            }
        }

        fn with(mut self, keycode: u16, base: char, shifted: char, physical: K) -> Self {
            self.base.insert(keycode, base);
            self.shifted.insert(keycode, shifted);
            self.physical.insert(keycode, physical);
            self
        }
    }

    impl LayoutSource for FakeLayout {
        fn base_char(&self, keycode: u16) -> Option<char> {
            self.base.get(&keycode).copied()
        }

        fn shift_char(&self, keycode: u16) -> Option<char> {
            self.shifted.get(&keycode).copied()
        }

        fn key_for_char(&self, character: char) -> Option<(u16, bool)> {
            for (keycode, base) in &self.base {
                if *base == character {
                    return Some((*keycode, false));
                }
            }
            for (keycode, shifted) in &self.shifted {
                if *shifted == character {
                    return Some((*keycode, true));
                }
            }
            None
        }

        fn physical_key(&self, keycode: u16) -> Option<K> {
            self.physical.get(&keycode).copied()
        }
    }

    #[test]
    fn canonical_tables_round_trip() {
        for key in [
            K::KEY_A,
            K::KEY_Z,
            K::KEY_1,
            K::KEY_0,
            K::KEY_GRAVE,
            K::KEY_MINUS,
            K::KEY_EQUAL,
            K::KEY_LEFTBRACE,
            K::KEY_RIGHTBRACE,
            K::KEY_BACKSLASH,
            K::KEY_SEMICOLON,
            K::KEY_APOSTROPHE,
            K::KEY_COMMA,
            K::KEY_DOT,
            K::KEY_SLASH,
            K::KEY_SPACE,
        ] {
            let character = key_symbol(key).unwrap();
            assert_eq!(unshifted_key(character), Some(key));
        }
    }

    #[test]
    fn shifted_table_maps_us_symbols() {
        for (character, key) in [
            ('!', K::KEY_1),
            ('@', K::KEY_2),
            ('#', K::KEY_3),
            ('$', K::KEY_4),
            ('%', K::KEY_5),
            ('^', K::KEY_6),
            ('&', K::KEY_7),
            ('*', K::KEY_8),
            ('(', K::KEY_9),
            (')', K::KEY_0),
            ('~', K::KEY_GRAVE),
            ('_', K::KEY_MINUS),
            ('+', K::KEY_EQUAL),
            ('{', K::KEY_LEFTBRACE),
            ('}', K::KEY_RIGHTBRACE),
            ('|', K::KEY_BACKSLASH),
            (':', K::KEY_SEMICOLON),
            ('"', K::KEY_APOSTROPHE),
            ('<', K::KEY_COMMA),
            ('>', K::KEY_DOT),
            ('?', K::KEY_SLASH),
        ] {
            assert_eq!(shifted_key(character), Some(key));
            assert_eq!(symbol_key(character), Some(key));
        }
    }

    #[test]
    fn dvorak_layout_maps_letters_by_character() {
        // A Dvorak layout: physical key 38 (QWERTY J) types "h".
        let layout = Layout::new(FakeLayout::new().with(38, 'h', 'H', K::KEY_J).with(
            4,
            'd',
            'D',
            K::KEY_H,
        ));
        assert_eq!(layout.key_from_physical(38), Some(K::KEY_H));
        assert_eq!(layout.key_from_physical(4), Some(K::KEY_D));
    }

    #[test]
    fn azerty_digits_use_shifted_character() {
        // AZERTY: physical key 18 (QWERTY 1) types "&" and "1".
        let layout = Layout::new(FakeLayout::new().with(18, '&', '1', K::KEY_1).with(
            19,
            'é',
            '2',
            K::KEY_2,
        ));
        assert_eq!(layout.key_from_physical(18), Some(K::KEY_1));
        assert_eq!(layout.key_from_physical(19), Some(K::KEY_2));
    }

    #[test]
    fn non_canonical_characters_fall_back_to_physical() {
        let layout = Layout::new(FakeLayout::new().with(14, 'é', 'É', K::KEY_E));
        assert_eq!(layout.key_from_physical(14), Some(K::KEY_E));
    }

    #[test]
    fn emission_uses_layout_characters_and_shift() {
        let layout = Layout::new(FakeLayout::new().with(18, '&', '1', K::KEY_1));
        assert_eq!(
            layout.emission_for(K::KEY_1),
            Emission::Physical {
                keycode: 18,
                needs_shift: true
            }
        );
        assert_eq!(layout.emission_for(K::KEY_F3), Emission::Logical);
    }

    #[test]
    fn emitter_tracks_held_keys_and_synthetic_shift() {
        let layout = Layout::new(FakeLayout::new().with(18, '&', '1', K::KEY_1));
        let mut emitter = LayoutEmitter::new(layout);
        let mut events = Vec::new();
        let mut sink = |target: Target, value: i32| {
            events.push((target, value));
            Ok(())
        };

        emitter.emit(K::KEY_1, 1, &mut sink).unwrap();
        emitter.emit(K::KEY_1, 0, &mut sink).unwrap();

        assert_eq!(
            events,
            [
                (Target::Logical(K::KEY_LEFTSHIFT), 1),
                (Target::Physical(18), 1),
                (Target::Physical(18), 0),
                (Target::Logical(K::KEY_LEFTSHIFT), 0),
            ]
        );
    }

    #[test]
    fn emitter_keeps_shift_for_other_held_keys() {
        let layout = Layout::new(FakeLayout::new().with(18, '&', '1', K::KEY_1).with(
            19,
            'é',
            '2',
            K::KEY_2,
        ));
        let mut emitter = LayoutEmitter::new(layout);
        let mut events = Vec::new();
        let mut sink = |target: Target, value: i32| {
            events.push((target, value));
            Ok(())
        };

        emitter.emit(K::KEY_1, 1, &mut sink).unwrap();
        emitter.emit(K::KEY_2, 1, &mut sink).unwrap();
        emitter.emit(K::KEY_1, 0, &mut sink).unwrap();
        emitter.emit(K::KEY_2, 0, &mut sink).unwrap();

        assert_eq!(
            events,
            [
                (Target::Logical(K::KEY_LEFTSHIFT), 1),
                (Target::Physical(18), 1),
                (Target::Logical(K::KEY_LEFTSHIFT), 1),
                (Target::Physical(19), 1),
                (Target::Physical(18), 0),
                (Target::Physical(19), 0),
                (Target::Logical(K::KEY_LEFTSHIFT), 0),
            ]
        );
    }
}
