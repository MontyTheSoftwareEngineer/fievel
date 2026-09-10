use evdev::{EventType, InputEvent, KeyCode, RelativeAxisCode};
use std::collections::BTreeSet;
use std::time::Duration;

use KeyCode as K;
use RelativeAxisCode as R;
use crate::config::Config;

#[derive(Default)]
pub struct Output {
    pub keyboard: Vec<InputEvent>,
    pub mouse: Vec<InputEvent>,
}

pub struct Engine {
    held: BTreeSet<K>,
    forwarded: BTreeSet<K>,
    suppressed: BTreeSet<K>,
    buttons: BTreeSet<K>,
    motion: [f64; 2],
    scroll: [f64; 2],
    config: Config,
}

impl Engine {
    pub fn new(config: Config) -> Self {
        Self {
            held: BTreeSet::new(),
            forwarded: BTreeSet::new(),
            suppressed: BTreeSet::new(),
            buttons: BTreeSet::new(),
            motion: [0.0; 2],
            scroll: [0.0; 2],
            config,
        }
    }

    pub fn active(&self) -> bool {
        self.held.contains(&self.config.keys.free_mouse)
    }

    pub fn emergency_exit(&self) -> bool {
        self.active() && self.held.contains(&self.config.keys.exit)
    }

    fn speed(&self) -> f64 {
        if self.active() && self.held.contains(&self.config.keys.slow) {
            self.config.speeds.slow
        } else if self.active() && self.held.contains(&self.config.keys.fast) {
            self.config.speeds.fast
        } else {
            self.config.speeds.normal
        }
    }

    fn direction(&self, negative: K, positive: K) -> i32 {
        if !self.active() {
            return 0;
        }
        i32::from(self.held.contains(&positive)) - i32::from(self.held.contains(&negative))
    }

    fn motion_direction(&self) -> [i32; 2] {
        [
            self.direction(self.config.keys.left, self.config.keys.right),
            self.direction(self.config.keys.up, self.config.keys.down),
        ]
    }

    fn scroll_direction(&self) -> [i32; 2] {
        [
            self.direction(self.config.keys.scroll_left, self.config.keys.scroll_right),
            self.direction(self.config.keys.scroll_down, self.config.keys.scroll_up),
        ]
    }

    pub fn key(&mut self, key: K, value: i32) -> Output {
        let mut out = Output::default();
        if !(0..=2).contains(&value) {
            return out; // Ignore values outside the Linux EV_KEY protocol.
        }
        let old_motion = self.motion_direction();
        let old_scroll = self.scroll_direction();
        let was_active = self.active();
        match value {
            0 => {
                self.held.remove(&key);
            }
            1 => {
                self.held.insert(key);
            }
            _ => {}
        }

        if self.active() && !was_active {
            // Transfer already-held controls from the keyboard to the mouse.
            for control in self.config.keys.controls() {
                if self.forwarded.remove(&control) {
                    out.keyboard.push(key_event(control, 0));
                }
                if self.held.contains(&control) {
                    self.suppressed.insert(control);
                }
            }
        }

        let consume = key == self.config.keys.free_mouse
            || self.suppressed.contains(&key)
            || (self.active()
                && (self.config.keys.controls().contains(&key) || key == self.config.keys.exit));
        if consume {
            if value == 0 {
                self.suppressed.remove(&key);
                if self.forwarded.remove(&key) {
                    out.keyboard.push(key_event(key, 0));
                }
            } else {
                self.suppressed.insert(key);
            }
        } else {
            match value {
                1 if self.forwarded.insert(key) => out.keyboard.push(key_event(key, 1)),
                0 if self.forwarded.remove(&key) => out.keyboard.push(key_event(key, 0)),
                2 if self.forwarded.contains(&key) => out.keyboard.push(key_event(key, 2)),
                _ => {}
            }
        }

        for (control, button) in [
            (self.config.keys.left_click, K::BTN_LEFT),
            (self.config.keys.right_click, K::BTN_RIGHT),
        ] {
            let down = self.active() && self.held.contains(&control);
            if down && self.buttons.insert(button) {
                out.mouse.push(key_event(button, 1));
            } else if !down && self.buttons.remove(&button) {
                out.mouse.push(key_event(button, 0));
            }
        }

        if self.motion_direction() != old_motion {
            self.motion = [0.0; 2];
        }
        let scroll = self.scroll_direction();
        for axis in 0..2 {
            if scroll[axis] != old_scroll[axis] {
                self.scroll[axis] = 0.0;
                if scroll[axis] != 0 {
                    out.mouse.push(relative_event(
                        [R::REL_HWHEEL, R::REL_WHEEL][axis],
                        scroll[axis],
                    ));
                }
            }
        }
        out
    }

    pub fn advance(&mut self, elapsed: Duration) -> Output {
        let mut out = Output::default();
        // Do not jump across the screen after a suspended or stalled process.
        let seconds = elapsed.min(Duration::from_millis(50)).as_secs_f64();
        let direction = self.motion_direction();
        let length = f64::from(direction[0] * direction[0] + direction[1] * direction[1]).sqrt();
        if length > 0.0 {
            for (axis, direction) in direction.into_iter().enumerate() {
                self.motion[axis] += f64::from(direction) / length * self.speed() * seconds;
                let amount = self.motion[axis].trunc() as i32;
                self.motion[axis] -= f64::from(amount);
                if amount != 0 {
                    out.mouse
                        .push(relative_event([R::REL_X, R::REL_Y][axis], amount));
                }
            }
        }
        for (axis, direction) in self.scroll_direction().into_iter().enumerate() {
            self.scroll[axis] += f64::from(direction) * self.config.speeds.scroll * seconds;
            let amount = self.scroll[axis].trunc() as i32;
            self.scroll[axis] -= f64::from(amount);
            if amount != 0 {
                out.mouse
                    .push(relative_event([R::REL_HWHEEL, R::REL_WHEEL][axis], amount));
            }
        }
        out
    }

    pub fn release_all(&mut self) -> Output {
        let mut out = Output::default();
        for key in &self.forwarded {
            out.keyboard.push(key_event(*key, 0));
        }
        for button in &self.buttons {
            out.mouse.push(key_event(*button, 0));
        }
        self.forwarded.clear();
        self.buttons.clear();
        self.held.clear();
        self.suppressed.clear();
        self.motion = [0.0; 2];
        self.scroll = [0.0; 2];
        out
    }
}

fn key_event(key: K, value: i32) -> InputEvent {
    InputEvent::new(EventType::KEY.0, key.0, value)
}

fn relative_event(axis: R, value: i32) -> InputEvent {
    InputEvent::new(EventType::RELATIVE.0, axis.0, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn events(events: &[InputEvent]) -> Vec<(u16, u16, i32)> {
        events
            .iter()
            .map(|event| (event.event_type().0, event.code(), event.value()))
            .collect()
    }

    fn engine() -> Engine {
        let mut config = Config::default();
        config.speeds.normal = 1000.0;
        config.speeds.scroll = 10.0;
        Engine::new(config)
    }

    #[test]
    fn ordinary_keys_and_repeats_pass_through() {
        let mut e = engine();
        for key in crate::config::Keys::default().controls().into_iter().chain([K::KEY_LEFTCTRL]) {
            for value in [1, 2, 0] {
                assert_eq!(
                    events(&e.key(key, value).keyboard),
                    vec![(EventType::KEY.0, key.0, value)]
                );
            }
        }
        assert!(e.advance(Duration::from_millis(20)).mouse.is_empty());
    }

    #[test]
    fn f3_and_mouse_controls_never_type_in_mode() {
        let mut e = engine();
        assert!(e.key(K::KEY_F3, 1).keyboard.is_empty());
        assert!(e.key(K::KEY_F3, 2).keyboard.is_empty());
        for key in crate::config::Keys::default().controls() {
            for value in [1, 2, 0] {
                assert!(e.key(key, value).keyboard.is_empty());
            }
        }
        assert_eq!(e.key(K::KEY_B, 1).keyboard.len(), 1);
        assert!(e.key(K::KEY_F3, 0).keyboard.is_empty());
        assert!(!e.active());
    }

    #[test]
    fn all_four_motion_directions_are_constant() {
        for (key, axis, amount) in [
            (K::KEY_H, R::REL_X, -10),
            (K::KEY_J, R::REL_Y, 10),
            (K::KEY_K, R::REL_Y, -10),
            (K::KEY_L, R::REL_X, 10),
        ] {
            let mut e = engine();
            e.key(K::KEY_F3, 1);
            e.key(key, 1);
            for _ in 0..100 {
                e.key(key, 2);
                assert_eq!(
                    events(&e.advance(Duration::from_millis(10)).mouse),
                    vec![(EventType::RELATIVE.0, axis.0, amount)]
                );
            }
            e.key(key, 0);
            assert!(e.advance(Duration::from_millis(10)).mouse.is_empty());
        }
    }

    #[test]
    fn diagonal_speed_is_normalized_and_opposites_cancel() {
        let mut e = engine();
        e.key(K::KEY_F3, 1);
        e.key(K::KEY_H, 1);
        e.key(K::KEY_L, 1);
        assert!(e.advance(Duration::from_millis(10)).mouse.is_empty());
        e.key(K::KEY_H, 0);
        e.key(K::KEY_J, 1);
        let mut distance = [0; 2];
        for _ in 0..100 {
            for event in e.advance(Duration::from_millis(10)).mouse {
                distance[usize::from(event.code())] += event.value();
            }
        }
        assert_eq!(distance, [707, 707]);
    }

    #[test]
    fn fractional_motion_is_preserved_between_ticks() {
        let mut config = Config::default();
        config.speeds.normal = 10.0;
        let mut e = Engine::new(config);
        e.key(K::KEY_F3, 1);
        e.key(K::KEY_L, 1);
        let total: i32 = (0..1000)
            .flat_map(|_| e.advance(Duration::from_millis(4)).mouse)
            .map(|event| event.value())
            .sum();
        assert!((39..=40).contains(&total));
    }

    #[test]
    fn clicks_follow_press_and_release_without_autorepeat() {
        let mut e = engine();
        e.key(K::KEY_F3, 1);
        for (key, button) in [(K::KEY_SPACE, K::BTN_LEFT), (K::KEY_I, K::BTN_RIGHT)] {
            assert_eq!(
                events(&e.key(key, 1).mouse),
                vec![(EventType::KEY.0, button.0, 1)]
            );
            assert!(e.key(key, 2).mouse.is_empty());
            assert_eq!(
                events(&e.key(key, 0).mouse),
                vec![(EventType::KEY.0, button.0, 0)]
            );
        }
    }

    #[test]
    fn f3_release_stops_motion_scroll_and_both_buttons() {
        let mut e = engine();
        e.key(K::KEY_F3, 1);
        for key in [K::KEY_SPACE, K::KEY_I, K::KEY_H, K::KEY_M] {
            e.key(key, 1);
        }
        let output = e.key(K::KEY_F3, 0);
        assert_eq!(output.mouse.len(), 2);
        assert!(output.mouse.iter().all(|event| event.value() == 0));
        assert!(e.advance(Duration::from_millis(50)).mouse.is_empty());
        for key in [K::KEY_SPACE, K::KEY_I, K::KEY_H, K::KEY_M] {
            assert!(e.key(key, 2).keyboard.is_empty());
            assert!(e.key(key, 0).keyboard.is_empty());
            assert_eq!(e.key(key, 1).keyboard.len(), 1);
        }
    }

    #[test]
    fn mode_entry_releases_previously_forwarded_controls() {
        let mut e = engine();
        e.key(K::KEY_H, 1);
        e.key(K::KEY_SPACE, 1);
        let output = e.key(K::KEY_F3, 1);
        assert_eq!(output.keyboard.len(), 2);
        assert!(output.keyboard.iter().all(|event| event.value() == 0));
        assert_eq!(
            events(&output.mouse),
            vec![(EventType::KEY.0, K::BTN_LEFT.0, 1)]
        );
        assert!(!e.advance(Duration::from_millis(10)).mouse.is_empty());
    }

    #[test]
    fn scroll_taps_and_holds_have_correct_directions() {
        for (key, axis, amount) in [
            (K::KEY_N, R::REL_HWHEEL, -1),
            (K::KEY_M, R::REL_WHEEL, -1),
            (K::KEY_COMMA, R::REL_WHEEL, 1),
            (K::KEY_DOT, R::REL_HWHEEL, 1),
        ] {
            let mut e = engine();
            e.key(K::KEY_F3, 1);
            let expected = vec![(EventType::RELATIVE.0, axis.0, amount)];
            assert_eq!(events(&e.key(key, 1).mouse), expected);
            assert!(e.key(key, 2).mouse.is_empty());
            assert!(e.advance(Duration::from_millis(50)).mouse.is_empty());
            assert_eq!(
                events(&e.advance(Duration::from_millis(50)).mouse),
                expected
            );
            e.key(key, 0);
            assert!(e.advance(Duration::from_millis(50)).mouse.is_empty());
        }
    }

    #[test]
    fn cleanup_releases_keyboard_and_mouse_and_resets_mode() {
        let mut e = engine();
        e.key(K::KEY_LEFTCTRL, 1);
        e.key(K::KEY_F3, 1);
        e.key(K::KEY_SPACE, 1);
        let output = e.release_all();
        assert_eq!(
            events(&output.keyboard),
            vec![(EventType::KEY.0, K::KEY_LEFTCTRL.0, 0)]
        );
        assert_eq!(
            events(&output.mouse),
            vec![(EventType::KEY.0, K::BTN_LEFT.0, 0)]
        );
        assert!(!e.active());
        assert!(e.advance(Duration::from_millis(10)).mouse.is_empty());
    }

    #[test]
    fn emergency_exit_requires_f3_and_escape() {
        let mut e = engine();
        e.key(K::KEY_ESC, 1);
        assert!(!e.emergency_exit());
        e.key(K::KEY_ESC, 0);
        e.key(K::KEY_F3, 1);
        assert!(e.key(K::KEY_ESC, 1).keyboard.is_empty());
        assert!(e.emergency_exit());
    }

    #[test]
    fn held_speed_keys_switch_immediately_and_slow_wins() {
        let mut e = Engine::new(Config::default());
        e.key(K::KEY_F3, 1);
        e.key(K::KEY_L, 1);
        let mut step = |key, value, distance| {
            assert!(e.key(key, value).keyboard.is_empty());
            assert_eq!(
                events(&e.advance(Duration::from_millis(10)).mouse),
                vec![(EventType::RELATIVE.0, R::REL_X.0, distance)]
            );
        };
        step(K::KEY_F3, 2, 8);
        step(K::KEY_S, 1, 16);
        step(K::KEY_S, 2, 16);
        step(K::KEY_S, 0, 8);
        step(K::KEY_A, 1, 2);
        step(K::KEY_A, 2, 2);
        step(K::KEY_S, 1, 2);
        step(K::KEY_S, 0, 2);
        step(K::KEY_S, 1, 2);
        step(K::KEY_A, 0, 16);
        step(K::KEY_S, 0, 8);
    }

    #[test]
    fn speed_keys_transfer_on_entry_and_stay_suppressed_until_released() {
        for key in [K::KEY_A, K::KEY_S] {
            let mut e = Engine::new(Config::default());
            assert_eq!(e.key(key, 1).keyboard.len(), 1);
            assert_eq!(
                events(&e.key(K::KEY_F3, 1).keyboard),
                vec![(EventType::KEY.0, key.0, 0)]
            );
            e.key(K::KEY_L, 1);
            assert_eq!(
                e.advance(Duration::from_millis(10)).mouse[0].value(),
                if key == K::KEY_A { 2 } else { 16 }
            );
            e.key(K::KEY_F3, 0);
            assert_eq!(e.speed(), 800.0);
            assert!(e.advance(Duration::from_millis(10)).mouse.is_empty());
            assert!(e.key(key, 2).keyboard.is_empty());
            assert!(e.key(key, 0).keyboard.is_empty());
            assert_eq!(e.key(key, 1).keyboard.len(), 1);
            e.key(key, 0);
            e.key(K::KEY_F3, 1);
            assert_eq!(e.speed(), 800.0);
        }
    }

    #[test]
    fn speed_modes_preserve_diagonal_normalization_and_scroll_rate() {
        for (key, expected) in [(K::KEY_S, 1131), (K::KEY_A, 141)] {
            let mut config = Config::default();
            config.speeds.scroll = 10.0;
            let mut e = Engine::new(config);
            e.key(K::KEY_F3, 1);
            e.key(key, 1);
            e.key(K::KEY_L, 1);
            e.key(K::KEY_J, 1);
            e.key(K::KEY_COMMA, 1);
            let mut totals = [0; 3];
            for _ in 0..100 {
                for event in e.advance(Duration::from_millis(10)).mouse {
                    let axis = match R(event.code()) {
                        R::REL_X => 0,
                        R::REL_Y => 1,
                        R::REL_WHEEL => 2,
                        _ => panic!("unexpected axis"),
                    };
                    totals[axis] += event.value();
                }
            }
            assert_eq!(totals[..2], [expected, expected]);
            assert!((9..=10).contains(&totals[2]));
        }
    }

    #[test]
    fn all_configured_bindings_replace_defaults() {
        let config = Config::parse(
            r#"
            [speeds]
            normal = 1000
            slow = 100
            fast = 2000
            [keys]
            free_mouse = "f4"
            left = "q"
            down = "e"
            up = "r"
            right = "t"
            left_click = "z"
            right_click = "x"
            scroll_left = "c"
            scroll_down = "v"
            scroll_up = "b"
            scroll_right = "u"
            slow = "leftshift"
            fast = "leftctrl"
            exit = "f12"
            "#,
        )
        .unwrap();
        let mut e = Engine::new(config);
        assert_eq!(e.key(K::KEY_F3, 1).keyboard.len(), 1);
        assert!(!e.active());
        e.key(K::KEY_F3, 0);
        assert!(e.key(K::KEY_F4, 1).keyboard.is_empty());
        assert!(e.active());
        for key in crate::config::Keys::default().controls() {
            assert_eq!(e.key(key, 1).keyboard.len(), 1);
            assert_eq!(e.key(key, 0).keyboard.len(), 1);
        }
        for (key, axis, value) in [
            (K::KEY_Q, R::REL_X, -10),
            (K::KEY_E, R::REL_Y, 10),
            (K::KEY_R, R::REL_Y, -10),
            (K::KEY_T, R::REL_X, 10),
        ] {
            assert!(e.key(key, 1).keyboard.is_empty());
            assert_eq!(
                events(&e.advance(Duration::from_millis(10)).mouse),
                vec![(EventType::RELATIVE.0, axis.0, value)]
            );
            e.key(key, 0);
        }
        for (key, button) in [(K::KEY_Z, K::BTN_LEFT), (K::KEY_X, K::BTN_RIGHT)] {
            for value in [1, 0] {
                assert_eq!(
                    events(&e.key(key, value).mouse),
                    vec![(EventType::KEY.0, button.0, value)]
                );
            }
        }
        for (key, axis, value) in [
            (K::KEY_C, R::REL_HWHEEL, -1),
            (K::KEY_V, R::REL_WHEEL, -1),
            (K::KEY_B, R::REL_WHEEL, 1),
            (K::KEY_U, R::REL_HWHEEL, 1),
        ] {
            assert_eq!(
                events(&e.key(key, 1).mouse),
                vec![(EventType::RELATIVE.0, axis.0, value)]
            );
            e.key(key, 0);
        }
        e.key(K::KEY_T, 1);
        for (key, distance) in [(K::KEY_LEFTCTRL, 20), (K::KEY_LEFTSHIFT, 1)] {
            assert!(e.key(key, 1).keyboard.is_empty());
            assert_eq!(
                e.advance(Duration::from_millis(10)).mouse[0].value(),
                distance
            );
            e.key(key, 0);
        }
        e.key(K::KEY_F4, 0);
        assert!(e.advance(Duration::from_millis(10)).mouse.is_empty());
        e.key(K::KEY_F4, 1);
        assert!(e.key(K::KEY_F12, 1).keyboard.is_empty());
        assert!(e.emergency_exit());
    }
}
