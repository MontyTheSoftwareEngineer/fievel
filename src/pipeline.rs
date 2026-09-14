use std::{collections::BTreeSet, time::Duration};

use evdev::KeyCode;

use crate::{
    config::{Config, HintKeys},
    engine::{Engine, Output},
    hints::ClickKind,
    remap::{Event, Remapper},
};

pub struct InputEngine {
    remapper: Remapper,
    mouse: Engine,
    hints: HintActivation,
}

struct HintActivation {
    keys: HintKeys,
    held: BTreeSet<KeyCode>,
    pending: Option<ClickKind>,
}

impl HintActivation {
    fn key(&mut self, key: KeyCode, value: i32) -> bool {
        let left_before = self.chord_held(&self.keys.left);
        let right_before = self.chord_held(&self.keys.right);
        match value {
            1 => {
                self.held.insert(key);
            }
            0 => {
                self.held.remove(&key);
            }
            _ => return false,
        }
        if value == 1 {
            if !left_before && self.chord_held(&self.keys.left) {
                self.pending = Some(ClickKind::Left);
            } else if !right_before && self.chord_held(&self.keys.right) {
                self.pending = Some(ClickKind::Right);
            }
        }
        self.pending.is_some()
    }

    fn chord_held(&self, chord: &[KeyCode]) -> bool {
        chord.iter().all(|key| self.held.contains(key))
    }
}

impl InputEngine {
    pub fn new(config: Config) -> Result<Self, String> {
        let mut remapper = Remapper::new(config.remap.clone())?;
        remapper.set_mouse_controls(config.mouse_controls());
        Ok(Self {
            remapper,
            hints: HintActivation {
                keys: config.hints.keys.clone(),
                held: BTreeSet::new(),
                pending: None,
            },
            mouse: Engine::new(config),
        })
    }

    pub fn active(&self) -> bool {
        self.mouse.active()
    }

    pub fn keycast_keys(&self) -> &[KeyCode] {
        self.mouse.keycast_keys()
    }

    pub fn speed_mode(&self) -> crate::engine::SpeedMode {
        self.mouse.speed_mode()
    }

    pub fn key(&mut self, key: KeyCode, value: i32) -> Output {
        let mut output = Output::default();
        let mouse = &mut self.mouse;
        let hints = &mut self.hints;
        self.remapper.key_with(key, value, &mut |event| {
            dispatch(mouse, hints, &mut output, event)
        });
        self.clear_hint_inputs();
        output
    }

    pub fn advance(&mut self, elapsed: Duration) -> Output {
        let mut output = self.mouse.advance(elapsed);
        let mouse = &mut self.mouse;
        let hints = &mut self.hints;
        self.remapper.advance_with(elapsed, &mut |event| {
            dispatch(mouse, hints, &mut output, event)
        });
        self.clear_hint_inputs();
        output
    }

    fn clear_hint_inputs(&mut self) {
        if self.hints.pending.is_some() {
            self.remapper.clear();
            self.hints.held.clear();
        }
    }

    pub fn take_hint_request(&mut self) -> Option<ClickKind> {
        self.hints.pending.take()
    }

    pub fn release_all(&mut self) -> Output {
        self.remapper.clear();
        self.hints.held.clear();
        self.hints.pending = None;
        self.mouse.release_all()
    }
}

fn dispatch(
    mouse: &mut Engine,
    hints: &mut HintActivation,
    output: &mut Output,
    event: Event,
) -> bool {
    if hints.pending.is_some() {
        return false;
    }
    if let Event::Key(key, value) = event {
        if hints.key(key, value) {
            append(output, mouse.release_all());
            return false;
        }
    }
    append(
        output,
        match event {
            Event::Key(key, value) => mouse.key(key, value),
            Event::Mouse(down) => mouse.mouse(down),
        },
    );
    mouse.active()
}

fn append(output: &mut Output, next: Output) {
    output.keyboard.extend(next.keyboard);
    output.mouse.extend(next.mouse);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Mode;
    use evdev::{InputEvent, KeyCode as K, RelativeAxisCode as R};

    fn config() -> Config {
        let mut config = Config::parse(include_str!("../homerow.config")).unwrap();
        config.easing.movement = 0.0;
        config.easing.scroll = 0.0;
        config
    }

    fn keys(events: &[InputEvent]) -> Vec<(K, i32)> {
        events
            .iter()
            .map(|event| (K(event.code()), event.value()))
            .collect()
    }

    #[test]
    fn keycast_custom_hotkey_bypasses_remappings_only_in_free_mouse_mode() {
        let config = Config::parse(
            "[keycast]\nenabled = true\nhotkey = 'f8'\n\
             [keys]\nup = 'u'\n\
             [remap.main]\n'd+f' = 'free_mouse'\nf8 = 'escape'\nu = 'left'",
        ).unwrap();
        let mut engine = InputEngine::new(config).unwrap();
        assert_eq!(keys(&engine.key(K::KEY_F8, 1).keyboard), [(K::KEY_ESC, 1)]);
        assert_eq!(keys(&engine.key(K::KEY_F8, 0).keyboard), [(K::KEY_ESC, 0)]);
        engine.key(K::KEY_D, 1);
        engine.key(K::KEY_F, 1);
        assert!(engine.active());
        assert!(engine.key(K::KEY_F8, 1).keyboard.is_empty());
        assert!(engine.key(K::KEY_F8, 0).keyboard.is_empty());
        assert!(engine.key(K::KEY_U, 1).keyboard.is_empty());
        engine.key(K::KEY_L, 1);
        assert_eq!(engine.keycast_keys(), [K::KEY_U, K::KEY_L]);
        engine.key(K::KEY_U, 0);
        engine.advance(Duration::from_secs(3));
        assert!(engine.keycast_keys().is_empty());
        engine.key(K::KEY_U, 1);
        assert_eq!(engine.keycast_keys(), [K::KEY_U]);
        engine.key(K::KEY_D, 0);
        assert!(!engine.active());
        assert!(engine.keycast_keys().is_empty());
        engine.release_all();
    }

    #[test]
    fn entering_hints_clears_keycast_and_mouse_mode() {
        for mode in ["hold", "toggle"] {
            let config = Config::parse(&format!(
                "mode = '{mode}'\n[keycast]\nenabled = true",
            ))
            .unwrap();
            let mut engine = InputEngine::new(config).unwrap();
            engine.key(K::KEY_F3, 1);
            engine.key(K::KEY_ESC, 1);
            engine.key(K::KEY_ESC, 0);
            engine.key(K::KEY_L, 1);
            engine.key(K::KEY_S, 1);
            assert_eq!(engine.keycast_keys(), [K::KEY_L, K::KEY_S]);
            assert_eq!(engine.speed_mode(), crate::engine::SpeedMode::Fast);

            engine.key(K::KEY_LEFTMETA, 1);
            let output = engine.key(K::KEY_SPACE, 1);
            assert_eq!(keys(&output.keyboard), [(K::KEY_LEFTMETA, 0)]);
            assert!(output.mouse.is_empty());
            assert_eq!(engine.take_hint_request(), Some(ClickKind::Left));
            assert!(!engine.active());
            assert!(engine.keycast_keys().is_empty());
            assert_eq!(engine.speed_mode(), crate::engine::SpeedMode::Normal);
            assert!(engine.advance(Duration::from_millis(20)).mouse.is_empty());
        }
    }

    #[test]
    fn physical_hint_shortcuts_release_modifiers_and_consume_completing_key() {
        for (key, click) in [
            (K::KEY_SPACE, ClickKind::Left),
            (K::KEY_I, ClickKind::Right),
        ] {
            let mut engine = InputEngine::new(Config::default()).unwrap();
            assert_eq!(
                keys(&engine.key(K::KEY_LEFTMETA, 1).keyboard),
                [(K::KEY_LEFTMETA, 1)]
            );
            let output = engine.key(key, 1);
            assert_eq!(keys(&output.keyboard), [(K::KEY_LEFTMETA, 0)]);
            assert!(output.mouse.is_empty());
            assert_eq!(engine.take_hint_request(), Some(click));
            assert_eq!(engine.take_hint_request(), None);
            assert!(!engine.active());
            for (key, value) in [(key, 2), (key, 0), (K::KEY_LEFTMETA, 0)] {
                engine.key(key, value);
                assert_eq!(engine.take_hint_request(), None);
            }
        }
    }

    #[test]
    fn remapped_super_enters_native_hints_instead_of_forwarding_desktop_shortcut() {
        for (first, second) in [
            (K::KEY_A, K::KEY_S),
            (K::KEY_S, K::KEY_A),
            (K::KEY_U, K::KEY_I),
            (K::KEY_I, K::KEY_U),
        ] {
            let mut engine = InputEngine::new(config()).unwrap();
            assert!(engine.key(first, 1).keyboard.is_empty());
            assert_eq!(
                keys(&engine.key(second, 1).keyboard),
                [(K::KEY_LEFTMETA, 1)]
            );
            let output = engine.key(K::KEY_SPACE, 1);
            assert_eq!(keys(&output.keyboard), [(K::KEY_LEFTMETA, 0)]);
            assert!(output.mouse.is_empty());
            assert_eq!(engine.take_hint_request(), Some(ClickKind::Left));
            for key in [first, second, K::KEY_SPACE] {
                assert!(engine.key(key, 0).keyboard.is_empty());
            }
            assert_eq!(keys(&engine.key(K::KEY_Z, 1).keyboard), [(K::KEY_Z, 1)]);
        }
    }

    #[test]
    fn buffered_remapped_right_hint_shortcut_activates_on_timer() {
        let mut engine = InputEngine::new(config()).unwrap();
        engine.key(K::KEY_A, 1);
        engine.key(K::KEY_S, 1);
        assert!(engine.key(K::KEY_I, 1).keyboard.is_empty());
        assert_eq!(engine.take_hint_request(), None);
        let output = engine.advance(Duration::from_millis(30));
        assert_eq!(keys(&output.keyboard), [(K::KEY_LEFTMETA, 0)]);
        assert_eq!(engine.take_hint_request(), Some(ClickKind::Right));
        assert!(engine.release_all().keyboard.is_empty());
    }

    #[test]
    fn hint_shortcuts_accept_reverse_order_and_custom_remapped_modifiers() {
        let mut engine = InputEngine::new(Config::default()).unwrap();
        engine.key(K::KEY_SPACE, 1);
        let output = engine.key(K::KEY_LEFTMETA, 1);
        assert_eq!(keys(&output.keyboard), [(K::KEY_SPACE, 0)]);
        assert_eq!(engine.take_hint_request(), Some(ClickKind::Left));

        let config =
            Config::parse("[hints.keys]\nleft = 'leftctrl+space'\n[remap.main]\n'a+s' = 'ctrl'")
                .unwrap();
        let mut engine = InputEngine::new(config).unwrap();
        engine.key(K::KEY_A, 1);
        engine.key(K::KEY_S, 1);
        let output = engine.key(K::KEY_SPACE, 1);
        assert_eq!(keys(&output.keyboard), [(K::KEY_LEFTCTRL, 0)]);
        engine.release_all();
        assert_eq!(engine.take_hint_request(), None);
    }

    #[test]
    fn direct_mouse_chord_consumes_letters_in_both_orders() {
        for (first, second) in [(K::KEY_F, K::KEY_D), (K::KEY_D, K::KEY_F)] {
            let mut engine = InputEngine::new(config()).unwrap();
            assert!(engine.key(first, 1).keyboard.is_empty());
            assert!(engine
                .advance(Duration::from_millis(10))
                .keyboard
                .is_empty());
            let activation = engine.key(second, 1);
            assert!(activation.keyboard.is_empty());
            assert!(activation.mouse.is_empty());
            assert!(engine.active());
            assert!(engine.key(K::KEY_H, 1).keyboard.is_empty());
            let movement = engine.advance(Duration::from_millis(20));
            assert!(movement
                .mouse
                .iter()
                .any(|event| { event.code() == R::REL_X.0 && event.value() < 0 }));
            assert!(engine.key(first, 0).keyboard.is_empty());
            assert!(!engine.active());
            assert!(engine.key(second, 0).keyboard.is_empty());
            assert!(engine.key(K::KEY_H, 0).keyboard.is_empty());
            assert!(engine.release_all().keyboard.is_empty());
        }
    }

    #[test]
    fn s_accelerates_mouse_without_triggering_shared_modifier_chords() {
        for (first, second) in [(K::KEY_F, K::KEY_D), (K::KEY_D, K::KEY_F)] {
            let mut engine = InputEngine::new(config()).unwrap();
            engine.key(first, 1);
            engine.key(second, 1);
            engine.key(K::KEY_H, 1);
            let normal = engine.advance(Duration::from_millis(20));
            assert_eq!(normal.mouse[0].code(), R::REL_X.0);
            assert_eq!(normal.mouse[0].value(), -6);
            assert!(engine.key(K::KEY_S, 1).keyboard.is_empty());
            let fast = engine.advance(Duration::from_millis(20));
            assert!(fast.keyboard.is_empty());
            assert_eq!(fast.mouse[0].code(), R::REL_X.0);
            assert_eq!(fast.mouse[0].value(), -18);
            assert!(engine.key(K::KEY_S, 0).keyboard.is_empty());
            let normal = engine.advance(Duration::from_millis(20));
            assert_eq!(normal.mouse[0].value(), -6);
            assert!(engine.active());
        }
    }

    #[test]
    fn caps_shortcut_and_held_navigation_are_distinct() {
        let mut engine = InputEngine::new(config()).unwrap();
        assert!(engine.key(K::KEY_CAPSLOCK, 1).keyboard.is_empty());
        assert!(engine
            .advance(Duration::from_millis(100))
            .keyboard
            .is_empty());
        assert_eq!(
            keys(&engine.key(K::KEY_C, 1).keyboard),
            [(K::KEY_LEFTCTRL, 1), (K::KEY_C, 1)]
        );
        engine.advance(Duration::from_millis(200));
        assert_eq!(keys(&engine.key(K::KEY_C, 0).keyboard), [(K::KEY_C, 0)]);
        assert_eq!(
            keys(&engine.key(K::KEY_CAPSLOCK, 0).keyboard),
            [(K::KEY_LEFTCTRL, 0)]
        );

        assert!(engine.key(K::KEY_CAPSLOCK, 1).keyboard.is_empty());
        assert!(engine
            .advance(Duration::from_millis(175))
            .keyboard
            .is_empty());
        for (source, target) in [
            (K::KEY_H, K::KEY_LEFT),
            (K::KEY_J, K::KEY_DOWN),
            (K::KEY_K, K::KEY_UP),
            (K::KEY_L, K::KEY_RIGHT),
        ] {
            assert_eq!(keys(&engine.key(source, 1).keyboard), [(target, 1)]);
            assert_eq!(keys(&engine.key(source, 2).keyboard), [(target, 2)]);
            assert_eq!(keys(&engine.key(source, 0).keyboard), [(target, 0)]);
        }
        assert!(engine.key(K::KEY_CAPSLOCK, 0).keyboard.is_empty());
        assert_eq!(keys(&engine.key(K::KEY_H, 1).keyboard), [(K::KEY_H, 1)]);
    }

    #[test]
    fn example_modifier_and_escape_chords_hold_their_outputs() {
        for (first, second, target) in [
            (K::KEY_A, K::KEY_S, K::KEY_LEFTMETA),
            (K::KEY_U, K::KEY_I, K::KEY_LEFTMETA),
            (K::KEY_S, K::KEY_D, K::KEY_LEFTCTRL),
            (K::KEY_I, K::KEY_O, K::KEY_LEFTCTRL),
            (K::KEY_A, K::KEY_F, K::KEY_ESC),
        ] {
            for (first, second) in [(first, second), (second, first)] {
                let mut engine = InputEngine::new(config()).unwrap();
                assert!(engine.key(first, 1).keyboard.is_empty());
                assert_eq!(keys(&engine.key(second, 1).keyboard), [(target, 1)]);
                assert_eq!(keys(&engine.key(K::KEY_C, 1).keyboard), [(K::KEY_C, 1)]);
                assert_eq!(keys(&engine.key(K::KEY_C, 0).keyboard), [(K::KEY_C, 0)]);
                assert_eq!(keys(&engine.key(first, 0).keyboard), [(target, 0)]);
                assert!(engine.key(second, 0).keyboard.is_empty());
                assert!(engine.release_all().keyboard.is_empty());
            }
        }
    }

    #[test]
    fn navigation_chord_holds_layer_until_first_release() {
        for (first, second) in [(K::KEY_W, K::KEY_E), (K::KEY_E, K::KEY_W)] {
            let mut engine = InputEngine::new(config()).unwrap();
            assert!(engine.key(first, 1).keyboard.is_empty());
            assert!(engine.key(second, 1).keyboard.is_empty());
            assert_eq!(keys(&engine.key(K::KEY_J, 1).keyboard), [(K::KEY_DOWN, 1)]);
            assert!(engine.key(first, 0).keyboard.is_empty());
            assert_eq!(keys(&engine.key(K::KEY_J, 0).keyboard), [(K::KEY_DOWN, 0)]);
            assert!(engine.key(second, 0).keyboard.is_empty());
            assert_eq!(keys(&engine.key(K::KEY_J, 1).keyboard), [(K::KEY_J, 1)]);
        }
    }

    #[test]
    fn mouse_controls_override_navigation_and_modifier_chords() {
        let mut engine = InputEngine::new(config()).unwrap();
        engine.key(K::KEY_CAPSLOCK, 1);
        engine.advance(Duration::from_millis(175));
        engine.key(K::KEY_F, 1);
        assert!(engine.key(K::KEY_D, 1).keyboard.is_empty());
        assert!(engine.active());
        assert!(engine.key(K::KEY_H, 1).keyboard.is_empty());
        let click = engine.key(K::KEY_I, 1);
        assert!(click.keyboard.is_empty());
        assert_eq!(keys(&click.mouse), [(K::BTN_RIGHT, 1)]);
        assert_eq!(keys(&engine.key(K::KEY_I, 0).mouse), [(K::BTN_RIGHT, 0)]);
        let movement = engine.advance(Duration::from_millis(20));
        assert!(movement.keyboard.is_empty());
        assert!(movement
            .mouse
            .iter()
            .any(|event| event.code() == R::REL_X.0));
        engine.key(K::KEY_H, 0);
        engine.key(K::KEY_D, 0);
        engine.key(K::KEY_F, 0);
        assert!(!engine.active());
        assert_eq!(keys(&engine.key(K::KEY_H, 1).keyboard), [(K::KEY_LEFT, 1)]);
        engine.key(K::KEY_CAPSLOCK, 0);
        assert_eq!(keys(&engine.key(K::KEY_H, 0).keyboard), [(K::KEY_LEFT, 0)]);
    }

    #[test]
    fn direct_toggle_chord_can_exit_and_releases_mouse_buttons() {
        let mut config = config();
        config.mode = Mode::Toggle;
        let mut engine = InputEngine::new(config).unwrap();
        for key in [K::KEY_F, K::KEY_D] {
            assert!(engine.key(key, 1).keyboard.is_empty());
        }
        assert!(engine.active());
        for key in [K::KEY_D, K::KEY_F] {
            assert!(engine.key(key, 0).keyboard.is_empty());
        }
        assert!(engine.active());
        assert_eq!(keys(&engine.key(K::KEY_SPACE, 1).mouse), [(K::BTN_LEFT, 1)]);
        assert!(engine.key(K::KEY_F, 1).keyboard.is_empty());
        let exit = engine.key(K::KEY_D, 1);
        assert!(exit.keyboard.is_empty());
        assert_eq!(keys(&exit.mouse), [(K::BTN_LEFT, 0)]);
        assert!(!engine.active());
        for key in [K::KEY_SPACE, K::KEY_F, K::KEY_D] {
            assert!(engine.key(key, 0).keyboard.is_empty());
        }
    }

    #[test]
    fn legacy_key_mapping_can_still_activate_mouse_mode() {
        let mut config = config();
        config.remap.main.insert("d+f".to_owned(), "f3".to_owned());
        let mut engine = InputEngine::new(config).unwrap();
        assert!(engine.key(K::KEY_D, 1).keyboard.is_empty());
        assert!(engine.key(K::KEY_F, 1).keyboard.is_empty());
        assert!(engine.active());
        assert!(engine.key(K::KEY_D, 0).keyboard.is_empty());
        assert!(!engine.active());
        assert!(engine.key(K::KEY_F, 0).keyboard.is_empty());
    }

    #[test]
    fn timed_mouse_action_activates_without_another_key_event() {
        let config =
            Config::parse("[remap.main]\ncapslock = 'timeout(ctrl, 175, free_mouse)'").unwrap();
        let mut engine = InputEngine::new(config).unwrap();
        assert!(engine.key(K::KEY_CAPSLOCK, 1).keyboard.is_empty());
        engine.advance(Duration::from_millis(174));
        assert!(!engine.active());
        assert!(engine.advance(Duration::from_millis(1)).keyboard.is_empty());
        assert!(engine.active());
        assert_eq!(keys(&engine.key(K::KEY_SPACE, 1).mouse), [(K::BTN_LEFT, 1)]);
        assert_eq!(
            keys(&engine.key(K::KEY_CAPSLOCK, 0).mouse),
            [(K::BTN_LEFT, 0)]
        );
        assert!(!engine.active());
        assert!(engine.key(K::KEY_SPACE, 0).keyboard.is_empty());
    }

    #[test]
    fn replayed_toggle_exit_routes_following_key_through_navigation() {
        let config = Config::parse(
            "mode = 'toggle'\n[remap.main]\ncapslock = 'layer(nav)'\n\
             f = 'free_mouse'\n'f+q' = 'escape'\n[remap.layers.nav]\nh = 'left'",
        )
        .unwrap();
        let mut engine = InputEngine::new(config).unwrap();
        engine.key(K::KEY_F3, 1);
        engine.key(K::KEY_F3, 0);
        engine.key(K::KEY_CAPSLOCK, 1);
        assert!(engine.active());
        assert!(engine.key(K::KEY_F, 1).keyboard.is_empty());
        assert_eq!(keys(&engine.key(K::KEY_H, 1).keyboard), [(K::KEY_LEFT, 1)]);
        assert!(!engine.active());
        assert!(engine.key(K::KEY_F, 0).keyboard.is_empty());
        assert_eq!(keys(&engine.key(K::KEY_H, 0).keyboard), [(K::KEY_LEFT, 0)]);
    }

    #[test]
    fn replayed_legacy_activation_routes_following_key_to_mouse() {
        let config = Config::parse(
            "[remap.main]\ncapslock = 'layer(nav)'\nf = 'f3'\n\
             'f+q' = 'escape'\n[remap.layers.nav]\nh = 'left'",
        )
        .unwrap();
        let mut engine = InputEngine::new(config).unwrap();
        engine.key(K::KEY_CAPSLOCK, 1);
        assert!(engine.key(K::KEY_F, 1).keyboard.is_empty());
        assert!(engine.key(K::KEY_H, 1).keyboard.is_empty());
        assert!(engine.active());
        assert!(engine.key(K::KEY_H, 0).keyboard.is_empty());
        assert!(engine.key(K::KEY_F, 0).keyboard.is_empty());
        assert!(!engine.active());
    }

    #[test]
    fn releasing_modifier_chord_does_not_reorder_pending_shortcut() {
        let mut engine = InputEngine::new(config()).unwrap();
        engine.key(K::KEY_S, 1);
        assert_eq!(
            keys(&engine.key(K::KEY_D, 1).keyboard),
            [(K::KEY_LEFTCTRL, 1)]
        );
        assert!(engine.key(K::KEY_A, 1).keyboard.is_empty());
        assert_eq!(
            keys(&engine.key(K::KEY_D, 0).keyboard),
            [(K::KEY_A, 1), (K::KEY_LEFTCTRL, 0)]
        );
        assert_eq!(keys(&engine.key(K::KEY_A, 0).keyboard), [(K::KEY_A, 0)]);
        assert!(engine.key(K::KEY_S, 0).keyboard.is_empty());
    }

    #[test]
    fn cleanup_releases_mapped_modifiers_and_discards_pending_keys() {
        let mut engine = InputEngine::new(config()).unwrap();
        engine.key(K::KEY_A, 1);
        assert_eq!(
            keys(&engine.key(K::KEY_S, 1).keyboard),
            [(K::KEY_LEFTMETA, 1)]
        );
        engine.key(K::KEY_CAPSLOCK, 1);
        assert_eq!(keys(&engine.release_all().keyboard), [(K::KEY_LEFTMETA, 0)]);
        assert!(!engine.active());
        assert!(engine.advance(Duration::from_secs(1)).keyboard.is_empty());
        assert!(engine.release_all().keyboard.is_empty());
    }

    #[test]
    fn empty_remapping_preserves_existing_mouse_and_keyboard_behavior() {
        let mut engine = InputEngine::new(Config::default()).unwrap();
        assert_eq!(keys(&engine.key(K::KEY_A, 1).keyboard), [(K::KEY_A, 1)]);
        assert_eq!(keys(&engine.key(K::KEY_A, 2).keyboard), [(K::KEY_A, 2)]);
        assert_eq!(keys(&engine.key(K::KEY_A, 0).keyboard), [(K::KEY_A, 0)]);
        assert!(engine.key(K::KEY_F3, 1).keyboard.is_empty());
        assert!(engine.active());
        assert_eq!(keys(&engine.key(K::KEY_SPACE, 1).mouse), [(K::BTN_LEFT, 1)]);
        let exit = engine.key(K::KEY_F3, 0);
        assert_eq!(keys(&exit.mouse), [(K::BTN_LEFT, 0)]);
        assert!(!engine.active());
        assert!(engine.key(K::KEY_SPACE, 0).keyboard.is_empty());
    }
}
