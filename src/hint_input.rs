use std::{collections::BTreeSet, time::Duration};

use evdev::KeyCode as K;

use crate::{
    config::{Config, HintKeys},
    hints::ClickKind,
    remap::{Event, Remapper},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintInputEvent {
    Key(K, i32),
    Shortcut(ClickKind),
}

pub struct HintInput {
    remapper: Remapper,
    shortcuts: Shortcuts,
    now: Duration,
}

struct Shortcuts {
    keys: HintKeys,
    held: BTreeSet<K>,
    pending: Vec<(K, i32)>,
    deadline: Option<Duration>,
    timeout: Duration,
}

impl HintInput {
    pub fn new(config: &Config) -> Result<Self, String> {
        let keys: Vec<K> = config
            .hints
            .keys
            .left
            .iter()
            .chain(&config.hints.keys.right)
            .copied()
            .chain([config.hints.keys.toggle_background, config.hints.keys.debug])
            .collect();
        Ok(Self {
            remapper: Remapper::for_shortcuts(config.remap.clone(), &keys)?,
            shortcuts: Shortcuts {
                keys: config.hints.keys.clone(),
                held: BTreeSet::new(),
                pending: Vec::new(),
                deadline: None,
                timeout: Duration::from_millis(config.remap.chord_timeout),
            },
            now: Duration::ZERO,
        })
    }

    pub fn begin(&mut self, held: impl IntoIterator<Item = K>) {
        self.remapper.clear();
        self.shortcuts.held.clear();
        self.shortcuts.pending.clear();
        self.shortcuts.deadline = None;
        self.now = Duration::ZERO;
        let mut events = Vec::new();
        for key in held {
            self.remapper.key_with(key, 1, &mut |event| {
                events.push(event);
                false
            });
        }
        // Prime held shortcut and readability keys without replaying hint letters.
        self.remapper
            .advance_with(self.shortcuts.timeout, &mut |event| {
                events.push(event);
                false
            });
        for event in events {
            if let Event::Key(key, value) = event {
                match value {
                    1 => {
                        self.shortcuts.held.insert(key);
                    }
                    0 => {
                        self.shortcuts.held.remove(&key);
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn readability_held(&self) -> bool {
        self.shortcuts
            .held
            .contains(&self.shortcuts.keys.toggle_background)
    }

    pub fn key(&mut self, key: K, value: i32) -> Vec<HintInputEvent> {
        if value == 1 && (key == K::KEY_ESC || key == self.shortcuts.keys.cancel) {
            return vec![HintInputEvent::Key(key, value)];
        }
        let mut output = self.shortcuts.advance(self.now);
        let shortcuts = &mut self.shortcuts;
        let now = self.now;
        self.remapper.key_with(key, value, &mut |event| {
            if let Event::Key(key, value) = event {
                output.extend(shortcuts.key(key, value, now));
            }
            false
        });
        self.guard_debug_transition(output)
    }

    pub fn advance(&mut self, elapsed: Duration) -> Vec<HintInputEvent> {
        self.now = self.now.saturating_add(elapsed);
        let mut output = self.shortcuts.advance(self.now);
        let shortcuts = &mut self.shortcuts;
        let now = self.now;
        self.remapper.advance_with(elapsed, &mut |event| {
            if let Event::Key(key, value) = event {
                output.extend(shortcuts.key(key, value, now));
            }
            false
        });
        self.guard_debug_transition(output)
    }

    fn guard_debug_transition(&mut self, mut output: Vec<HintInputEvent>) -> Vec<HintInputEvent> {
        let keys = &self.shortcuts.keys;
        if output.contains(&HintInputEvent::Key(keys.debug, 1)) {
            // A chord timeout can release a buffered label in the same batch
            // as debug. Do not let that label click before the view changes.
            output.retain(|event| match event {
                HintInputEvent::Key(key, _) => {
                    [keys.debug, keys.cancel, K::KEY_ESC, keys.toggle_background].contains(key)
                }
                HintInputEvent::Shortcut(_) => true,
            });
            self.shortcuts.pending.clear();
            self.shortcuts.deadline = None;
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use HintInputEvent::{Key, Shortcut};

    fn plain() -> HintInput {
        HintInput::new(&Config::default()).unwrap()
    }

    #[test]
    fn debug_keys_and_remappings_do_not_replay_pending_clicks() {
        let config = Config::parse("[hints.keys]\ndebug = 'f9'\n[remap.main]\n'x+y' = 'f9'").unwrap();
        let mut input = HintInput::new(&config).unwrap();
        input.begin([]);
        assert!(input.key(K::KEY_I, 1).is_empty());
        assert_eq!(input.key(K::KEY_F9, 1), [Key(K::KEY_F9, 1)]);
        assert!(input.key(K::KEY_F9, 2).is_empty());
        assert!(input.advance(Duration::from_millis(50)).is_empty());
        input.key(K::KEY_F9, 0);
        assert_eq!(press(&mut input, &[K::KEY_X, K::KEY_Y]), [Key(K::KEY_F9, 1)]);
        assert_eq!(input.key(K::KEY_Y, 0), [Key(K::KEY_F9, 0)]);
        input.begin([]);
        assert_eq!(input.key(K::KEY_F8, 1), [Key(K::KEY_F8, 1)]);
        let mut defaults = plain();
        defaults.begin([]);
        assert_eq!(defaults.key(K::KEY_F8, 1), [Key(K::KEY_F8, 1)]);
    }

    fn home_row() -> HintInput {
        HintInput::new(&Config::parse(include_str!("../fievel.config")).unwrap()).unwrap()
    }

    fn press(input: &mut HintInput, keys: &[K]) -> Vec<HintInputEvent> {
        keys.iter().flat_map(|key| input.key(*key, 1)).collect()
    }

    #[test]
    fn activation_can_be_repressed_without_releasing_super() {
        for (terminal, click) in [
            (K::KEY_SPACE, ClickKind::Left),
            (K::KEY_I, ClickKind::Right),
        ] {
            let mut input = plain();
            input.begin([K::KEY_LEFTMETA, terminal]);
            assert!(input.key(terminal, 2).is_empty());
            assert_eq!(input.key(terminal, 0), [Key(terminal, 0)]);
            assert_eq!(input.key(terminal, 1), [Shortcut(click)]);
            assert!(input.key(terminal, 2).is_empty());
        }
    }

    #[test]
    fn physical_shortcuts_work_in_either_press_order_without_typing_i() {
        for keys in [[K::KEY_LEFTMETA, K::KEY_I], [K::KEY_I, K::KEY_LEFTMETA]] {
            let mut input = plain();
            input.begin([]);
            assert_eq!(press(&mut input, &keys), [Shortcut(ClickKind::Right)]);
            assert!(input.advance(Duration::from_millis(50)).is_empty());
        }
    }

    #[test]
    fn other_binding_requests_mode_switch_without_selecting_a_label() {
        let mut input = plain();
        input.begin([K::KEY_LEFTMETA, K::KEY_SPACE]);
        assert_eq!(input.key(K::KEY_I, 1), [Shortcut(ClickKind::Right)]);
        input.key(K::KEY_SPACE, 0);
        assert_eq!(input.key(K::KEY_SPACE, 1), [Shortcut(ClickKind::Left)]);
    }

    #[test]
    fn home_row_super_repress_is_consumed_in_both_orders() {
        for keys in [
            [K::KEY_A, K::KEY_S],
            [K::KEY_S, K::KEY_A],
            [K::KEY_U, K::KEY_I],
            [K::KEY_I, K::KEY_U],
        ] {
            let mut input = home_row();
            input.begin([]);
            let mut output = press(&mut input, &keys);
            output.extend(input.key(K::KEY_SPACE, 1));
            assert_eq!(output, [Shortcut(ClickKind::Left)]);
        }
    }

    #[test]
    fn held_home_row_super_survives_hint_entry() {
        let mut input = home_row();
        input.begin([K::KEY_A, K::KEY_S, K::KEY_SPACE]);
        input.key(K::KEY_SPACE, 0);
        assert_eq!(input.key(K::KEY_SPACE, 1), [Shortcut(ClickKind::Left)]);
    }

    #[test]
    fn incomplete_activation_letters_are_replayed_on_release_or_timeout() {
        let mut input = plain();
        input.begin([]);
        assert!(input.key(K::KEY_I, 1).is_empty());
        assert_eq!(input.key(K::KEY_I, 0), [Key(K::KEY_I, 1), Key(K::KEY_I, 0)]);
        assert!(input.key(K::KEY_I, 1).is_empty());
        assert_eq!(input.advance(Duration::from_millis(25)), [Key(K::KEY_I, 1)]);
        assert!(input.advance(Duration::from_millis(25)).is_empty());

        let mut input = home_row();
        input.begin([]);
        assert!(input.key(K::KEY_A, 1).is_empty());
        assert_eq!(input.key(K::KEY_A, 0), [Key(K::KEY_A, 1), Key(K::KEY_A, 0)]);
        assert!(input.key(K::KEY_S, 1).is_empty());
        assert_eq!(input.key(K::KEY_S, 0), [Key(K::KEY_S, 1), Key(K::KEY_S, 0)]);
    }

    #[test]
    fn readability_hold_gets_press_and_release_and_ignores_repeat() {
        let mut input = home_row();
        input.begin([]);
        assert_eq!(input.key(K::KEY_LEFTCTRL, 1), [Key(K::KEY_LEFTCTRL, 1)]);
        assert!(input.key(K::KEY_LEFTCTRL, 2).is_empty());
        assert_eq!(input.key(K::KEY_LEFTCTRL, 0), [Key(K::KEY_LEFTCTRL, 0)]);
    }

    #[test]
    fn remapped_ctrl_shows_readability_and_either_member_release_hides_it() {
        for chord in [
            [K::KEY_S, K::KEY_D],
            [K::KEY_D, K::KEY_S],
            [K::KEY_I, K::KEY_O],
            [K::KEY_O, K::KEY_I],
        ] {
            for released in [0, 1] {
                let mut input = home_row();
                input.begin([]);
                assert_eq!(press(&mut input, &chord), [Key(K::KEY_LEFTCTRL, 1)]);
                assert!(input.readability_held());
                for key in chord {
                    assert!(input.key(key, 2).is_empty());
                }
                assert_eq!(input.key(chord[released], 0), [Key(K::KEY_LEFTCTRL, 0)],);
                assert!(!input.readability_held());
                assert!(input.key(chord[1 - released], 0).is_empty());
            }
        }
    }

    #[test]
    fn readability_stays_held_until_all_ctrl_producers_release() {
        let mut input = home_row();
        input.begin([]);
        assert_eq!(
            press(&mut input, &[K::KEY_S, K::KEY_D]),
            [Key(K::KEY_LEFTCTRL, 1)]
        );
        assert!(press(&mut input, &[K::KEY_I, K::KEY_O]).is_empty());
        assert!(input.key(K::KEY_LEFTCTRL, 1).is_empty());
        assert!(input.key(K::KEY_S, 0).is_empty());
        assert!(input.key(K::KEY_I, 0).is_empty());
        assert!(input.readability_held());
        assert_eq!(input.key(K::KEY_LEFTCTRL, 0), [Key(K::KEY_LEFTCTRL, 0)]);
        assert!(!input.readability_held());
    }

    #[test]
    fn readability_chord_already_held_on_hint_entry_is_recognized() {
        let mut input = home_row();
        input.begin([K::KEY_S, K::KEY_D, K::KEY_LEFTMETA, K::KEY_SPACE]);
        assert!(input.readability_held());
        assert_eq!(input.key(K::KEY_D, 0), [Key(K::KEY_LEFTCTRL, 0)]);
        assert!(!input.readability_held());
        input.begin([K::KEY_LEFTCTRL]);
        assert!(input.readability_held());
        input.begin([]);
        assert!(!input.readability_held());
    }

    #[test]
    fn custom_readability_key_preserves_mappings_that_produce_it() {
        let config = Config::parse(
            "[hints.keys]\ntoggle_background = 'rightctrl'\n[remap.main]\n'x+y' = 'rightctrl'",
        )
        .unwrap();
        let mut input = HintInput::new(&config).unwrap();
        input.begin([]);
        assert_eq!(
            press(&mut input, &[K::KEY_X, K::KEY_Y]),
            [Key(K::KEY_RIGHTCTRL, 1)]
        );
        assert!(input.readability_held());
        assert_eq!(input.key(K::KEY_X, 0), [Key(K::KEY_RIGHTCTRL, 0)]);
        assert!(!input.readability_held());
    }

    #[test]
    fn separate_s_and_d_taps_still_type_hint_letters() {
        let mut input = home_row();
        input.begin([]);
        for key in [K::KEY_S, K::KEY_D] {
            assert!(input.key(key, 1).is_empty());
            assert_eq!(input.key(key, 0), [Key(key, 1), Key(key, 0)]);
            assert!(!input.readability_held());
        }
    }

    #[test]
    fn unrelated_remappings_do_not_rewrite_hint_letters_or_start_mouse_mode() {
        let config = Config::parse(
            "[remap.main]\nh = 'left'\nj = 'down'\n'd+f' = 'free_mouse'\n'w+e' = 'layer(nav)'\n\
                 [remap.layers.nav]\nk = 'up'",
        )
        .unwrap();
        let mut input = HintInput::new(&config).unwrap();
        input.begin([]);
        for key in [
            K::KEY_H,
            K::KEY_J,
            K::KEY_D,
            K::KEY_F,
            K::KEY_W,
            K::KEY_E,
            K::KEY_K,
        ] {
            assert_eq!(input.key(key, 1), [Key(key, 1)]);
        }
    }

    #[test]
    fn custom_single_key_and_modifier_activation_work() {
        let config = Config::parse("[hints.keys]\nleft = 'f5'\nright = 'leftalt+r'").unwrap();
        let mut input = HintInput::new(&config).unwrap();
        input.begin([]);
        assert_eq!(input.key(K::KEY_F5, 1), [Shortcut(ClickKind::Left)]);
        assert!(input.key(K::KEY_F5, 2).is_empty());
        assert_eq!(
            press(&mut input, &[K::KEY_R, K::KEY_LEFTALT]),
            [Shortcut(ClickKind::Right)]
        );
    }

    #[test]
    fn shortcut_layer_mappings_remain_available() {
        let config = Config::parse(
            "[remap.main]\ncapslock = 'layer(shortcuts)'\n[remap.layers.shortcuts]\nx = 'super'",
        )
        .unwrap();
        let mut input = HintInput::new(&config).unwrap();
        input.begin([]);
        assert_eq!(
            press(&mut input, &[K::KEY_CAPSLOCK, K::KEY_X, K::KEY_SPACE]),
            [Shortcut(ClickKind::Left)]
        );
    }

    #[test]
    fn backspace_and_background_flush_pending_labels_in_order() {
        for key in [K::KEY_BACKSPACE, K::KEY_LEFTCTRL] {
            let mut input = plain();
            input.begin([]);
            input.key(K::KEY_I, 1);
            assert_eq!(input.key(key, 1), [Key(K::KEY_I, 1), Key(key, 1)]);
        }
    }

    #[test]
    fn cancel_does_not_replay_a_buffered_label_that_could_click() {
        for key in [K::KEY_ESC, K::KEY_ENTER] {
            let mut input = plain();
            input.begin([]);
            input.key(K::KEY_I, 1);
            assert_eq!(input.key(key, 1), [Key(key, 1)]);
        }
    }
}

impl Shortcuts {
    fn drain(&mut self) -> Vec<HintInputEvent> {
        self.deadline = None;
        self.pending
            .drain(..)
            .map(|(key, value)| HintInputEvent::Key(key, value))
            .collect()
    }

    fn advance(&mut self, now: Duration) -> Vec<HintInputEvent> {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.drain()
        } else {
            Vec::new()
        }
    }

    fn key(&mut self, key: K, value: i32, now: Duration) -> Vec<HintInputEvent> {
        if value == 0 {
            self.held.remove(&key);
            let mut output = self.drain();
            output.push(HintInputEvent::Key(key, value));
            return output;
        }
        if value != 1 || !self.held.insert(key) {
            return Vec::new();
        }
        if key == self.keys.debug {
            self.pending.clear();
            self.deadline = None;
            return vec![HintInputEvent::Key(key, value)];
        }
        for (chord, click) in [
            (&self.keys.left, ClickKind::Left),
            (&self.keys.right, ClickKind::Right),
        ] {
            if chord.contains(&key) && chord.iter().all(|member| self.held.contains(member)) {
                self.pending.clear();
                self.deadline = None;
                return vec![HintInputEvent::Shortcut(click)];
            }
        }
        if key != self.keys.toggle_background
            && (self.keys.left.contains(&key) || self.keys.right.contains(&key))
        {
            self.pending.push((key, value));
            self.deadline
                .get_or_insert(now.saturating_add(self.timeout));
            Vec::new()
        } else {
            let mut output = self.drain();
            output.push(HintInputEvent::Key(key, value));
            output
        }
    }
}
