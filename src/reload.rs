use crate::input::KeyCode;

pub(crate) fn is_modifier(key: KeyCode) -> bool {
    [
        KeyCode::KEY_LEFTCTRL,
        KeyCode::KEY_RIGHTCTRL,
        KeyCode::KEY_LEFTSHIFT,
        KeyCode::KEY_RIGHTSHIFT,
        KeyCode::KEY_LEFTALT,
        KeyCode::KEY_RIGHTALT,
        KeyCode::KEY_LEFTMETA,
        KeyCode::KEY_RIGHTMETA,
    ]
    .contains(&key)
}

pub fn matches_press(
    chord: &[KeyCode],
    key: KeyCode,
    value: i32,
    is_held: impl Fn(KeyCode) -> bool,
) -> bool {
    value == 1
        && !is_modifier(key)
        && chord.contains(&key)
        && chord.iter().all(|key| is_held(*key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn reload_chord_matches_only_when_all_keys_are_pressed() {
        let chord = [
            KeyCode::KEY_LEFTMETA,
            KeyCode::KEY_LEFTSHIFT,
            KeyCode::KEY_R,
        ];
        let mut held = BTreeSet::new();
        held.extend(chord);

        assert!(matches_press(&chord, KeyCode::KEY_R, 1, |key| held.contains(&key)));
        assert!(!matches_press(&chord, KeyCode::KEY_LEFTSHIFT, 1, |key| held.contains(&key)));
        assert!(!matches_press(&chord, KeyCode::KEY_R, 2, |key| held.contains(&key)));
        held.remove(&KeyCode::KEY_LEFTSHIFT);
        assert!(!matches_press(&chord, KeyCode::KEY_R, 1, |key| held.contains(&key)));
    }
}
