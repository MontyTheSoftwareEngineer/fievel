use std::time::Duration;

use evdev::KeyCode;

use crate::{
    config::Config,
    engine::{Engine, Output},
    remap::{Event, Remapper},
};

pub struct InputEngine {
    remapper: Remapper,
    mouse: Engine,
}

impl InputEngine {
    pub fn new(config: Config) -> Result<Self, String> {
        let mut remapper = Remapper::new(config.remap.clone())?;
        remapper.set_mouse_controls(config.keys.controls().to_vec());
        Ok(Self {
            remapper,
            mouse: Engine::new(config),
        })
    }

    pub fn active(&self) -> bool {
        self.mouse.active()
    }

    pub fn key(&mut self, key: KeyCode, value: i32) -> Output {
        let mut output = Output::default();
        let mouse = &mut self.mouse;
        self.remapper
            .key_with(key, value, &mut |event| dispatch(mouse, &mut output, event));
        output
    }

    pub fn advance(&mut self, elapsed: Duration) -> Output {
        let mut output = self.mouse.advance(elapsed);
        let mouse = &mut self.mouse;
        self.remapper
            .advance_with(elapsed, &mut |event| dispatch(mouse, &mut output, event));
        output
    }

    pub fn release_all(&mut self) -> Output {
        self.remapper.clear();
        self.mouse.release_all()
    }
}

fn dispatch(mouse: &mut Engine, output: &mut Output, event: Event) -> bool {
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
        let mut config =
            Config::parse(include_str!("../homerow.config")).unwrap();
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
