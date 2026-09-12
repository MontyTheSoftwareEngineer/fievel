use evdev::{EventType, InputEvent, KeyCode, RelativeAxisCode};
use std::collections::BTreeSet;
use std::time::Duration;

use KeyCode as K;
use RelativeAxisCode as R;
use crate::config::{Config, Mode};

const WHEEL_UNITS_PER_NOTCH: i32 = 120;

#[derive(Default)]
pub struct Output {
    pub keyboard: Vec<InputEvent>,
    pub mouse: Vec<InputEvent>,
}

pub struct Engine {
    toggled: bool,
    direct_mouse_held: bool,
    held: BTreeSet<K>,
    forwarded: BTreeSet<K>,
    suppressed: BTreeSet<K>,
    activation_keys: BTreeSet<K>,
    scroll_reserved: BTreeSet<K>,
    buttons: BTreeSet<K>,
    motion: [f64; 2],
    scroll: [f64; 2],
    velocity: [f64; 2],
    scroll_velocity: [f64; 2],
    legacy_scroll: [i32; 2],
    config: Config,
}

impl Engine {
    pub fn new(config: Config) -> Self {
        Self {
            toggled: false,
            direct_mouse_held: false,
            held: BTreeSet::new(),
            forwarded: BTreeSet::new(),
            suppressed: BTreeSet::new(),
            activation_keys: BTreeSet::new(),
            scroll_reserved: BTreeSet::new(),
            buttons: BTreeSet::new(),
            motion: [0.0; 2],
            scroll: [0.0; 2],
            velocity: [0.0; 2],
            scroll_velocity: [0.0; 2],
            legacy_scroll: [0; 2],
            config,
        }
    }

    pub fn active(&self) -> bool {
        match self.config.mode {
            Mode::Hold => self.chord_held() || self.direct_mouse_held,
            Mode::Toggle => self.toggled,
        }
    }

    fn chord_held(&self) -> bool {
        self.config.keys.free_mouse.iter().all(|key| self.held.contains(key))
    }

    fn control_held(&self, key: K) -> bool {
        self.held.contains(&key) && !self.activation_keys.contains(&key)
    }

    fn speed(&self) -> f64 {
        self.mode_speed(
            self.config.speeds.normal,
            self.config.speeds.slow,
            self.config.speeds.fast,
        )
    }

    fn scroll_speed(&self) -> f64 {
        self.mode_speed(
            self.config.speeds.scroll,
            self.config.speeds.scroll_slow,
            self.config.speeds.scroll_fast,
        )
    }

    fn mode_speed(&self, normal: f64, slow: f64, fast: f64) -> f64 {
        if self.active() && self.control_held(self.config.keys.slow) {
            slow
        } else if self.active() && self.control_held(self.config.keys.fast) {
            fast
        } else {
            normal
        }
    }

    fn direction(&self, negative: K, positive: K) -> i32 {
        if !self.active() {
            return 0;
        }
        i32::from(self.control_held(positive)) - i32::from(self.control_held(negative))
    }

    fn motion_direction(&self) -> [i32; 2] {
        [
            self.direction(self.config.keys.left, self.config.keys.right),
            self.direction(self.config.keys.up, self.config.keys.down),
        ]
    }

    fn scroll_direction(&self) -> [i32; 2] {
        [
            (self.config.keys.scroll_left, self.config.keys.scroll_right),
            (self.config.keys.scroll_down, self.config.keys.scroll_up),
        ]
        .map(|(negative, positive)| {
            let scrolling = |key| {
                self.active() && self.control_held(key) && !self.scroll_reserved.contains(&key)
            };
            i32::from(scrolling(positive)) - i32::from(scrolling(negative))
        })
    }

    fn home_end_chords(&self) -> [bool; 2] {
        [
            (self.config.keys.scroll_up, self.config.keys.scroll_down),
            (self.config.keys.scroll_left, self.config.keys.scroll_right),
        ]
        .map(|(first, second)| {
            self.config.home_end_enabled
                && self.active()
                && self.control_held(first)
                && self.control_held(second)
        })
    }

    pub fn key(&mut self, key: K, value: i32) -> Output {
        self.input(Input::Key(key, value))
    }

    pub fn mouse(&mut self, down: bool) -> Output {
        self.input(Input::Mouse(down))
    }

    fn input(&mut self, input: Input) -> Output {
        let mut out = Output::default();
        if matches!(input, Input::Key(_, value) if !(0..=2).contains(&value)) {
            return out; // Ignore values outside the Linux EV_KEY protocol.
        }
        let old_motion = self.motion_direction();
        let old_scroll = self.scroll_direction();
        let old_home_end = self.home_end_chords();
        let was_active = self.active();
        let was_chord_held = self.chord_held();
        let was_direct_held = self.direct_mouse_held;
        match input {
            Input::Key(key, 0) => {
                self.held.remove(&key);
                self.activation_keys.remove(&key);
                self.scroll_reserved.remove(&key);
            }
            Input::Key(key, 1) => {
                self.held.insert(key);
            }
            Input::Mouse(down) => self.direct_mouse_held = down,
            _ => {}
        }

        let chord_pressed = !was_chord_held && self.chord_held();
        if (chord_pressed || (!was_direct_held && self.direct_mouse_held))
            && self.config.mode == Mode::Toggle
        {
            self.toggled = !self.toggled;
        }
        if chord_pressed {
            // Release forwarded chord members (especially modifiers) and reserve
            // them until key-up, so activation cannot also click, move, or scroll.
            for member in &self.config.keys.free_mouse {
                if self.forwarded.remove(member) {
                    out.keyboard.push(key_event(*member, 0));
                }
                self.activation_keys.insert(*member);
                self.suppressed.insert(*member);
            }
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

        if let Input::Key(key, value) = input {
            let consume = self.config.keys.free_mouse.as_slice() == [key]
                || self.suppressed.contains(&key)
                || (self.active() && self.config.keys.controls().contains(&key));
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
        }

        for (index, down) in self.home_end_chords().into_iter().enumerate() {
            if input.is_press() && down && !old_home_end[index] {
                // A page jump must not be followed by coasting or by the remaining
                // scroll key when the chord is released one member at a time.
                self.reset_scroll();
                for control in [
                    self.config.keys.scroll_left,
                    self.config.keys.scroll_right,
                    self.config.keys.scroll_down,
                    self.config.keys.scroll_up,
                ] {
                    if self.control_held(control) {
                        self.scroll_reserved.insert(control);
                    }
                }
                let target = [K::KEY_HOME, K::KEY_END][index];
                if self.forwarded.contains(&target) {
                    // Do not release a Home/End key the user is physically holding.
                    out.keyboard.push(key_event(target, 2));
                } else {
                    out.keyboard.push(key_event(target, 1));
                    out.keyboard.push(key_event(target, 0));
                }
            }
        }

        for (control, button) in [
            (self.config.keys.left_click, K::BTN_LEFT),
            (self.config.keys.right_click, K::BTN_RIGHT),
        ] {
            let down = self.active() && self.control_held(control);
            if down && self.buttons.insert(button) {
                out.mouse.push(key_event(button, 1));
            } else if !down && self.buttons.remove(&button) {
                out.mouse.push(key_event(button, 0));
            }
        }

        if !self.active() {
            self.reset_motion();
        }
        if instant(self.config.easing.movement) && self.motion_direction() != old_motion {
            self.motion = [0.0; 2];
        }
        let scroll = self.scroll_direction();
        for axis in 0..2 {
            if scroll[axis] != old_scroll[axis] {
                if instant(self.config.easing.scroll) {
                    self.scroll[axis] = 0.0;
                }
                if instant(self.config.easing.scroll) && scroll[axis] != 0 {
                    self.emit_scroll(axis, scroll[axis] * WHEEL_UNITS_PER_NOTCH, &mut out);
                }
            }
        }
        out
    }

    pub fn advance(&mut self, elapsed: Duration) -> Output {
        let mut out = Output::default();
        // Do not jump across the screen after a suspended or stalled process.
        let seconds = elapsed.min(Duration::from_millis(50)).as_secs_f64();
        if !self.active() || seconds == 0.0 {
            return out;
        }
        let direction = self.motion_direction();
        let length = f64::from(direction[0] * direction[0] + direction[1] * direction[1]).sqrt();
        for (axis, direction) in direction.into_iter().enumerate() {
            let target = f64::from(direction) / length.max(1.0) * self.speed();
            let amount = advance_axis(
                &mut self.motion[axis],
                &mut self.velocity[axis],
                target,
                self.config.easing.movement,
                seconds,
                1.0,
            );
            if amount != 0 {
                out.mouse
                    .push(relative_event([R::REL_X, R::REL_Y][axis], amount));
            }
        }
        for (axis, direction) in self.scroll_direction().into_iter().enumerate() {
            let target = f64::from(direction) * self.scroll_speed();
            let stepped = instant(self.config.easing.scroll);
            let amount = advance_axis(
                &mut self.scroll[axis],
                &mut self.scroll_velocity[axis],
                target,
                self.config.easing.scroll,
                seconds,
                if stepped { 1.0 } else { 1.0 / f64::from(WHEEL_UNITS_PER_NOTCH) },
            );
            if amount != 0 {
                self.emit_scroll(
                    axis,
                    if stepped { amount * WHEEL_UNITS_PER_NOTCH } else { amount },
                    &mut out,
                );
            }
        }
        out
    }

    fn emit_scroll(&mut self, axis: usize, amount: i32, out: &mut Output) {
        // Linux high-resolution wheels use 120 units per notch. Emit legacy
        // notches from the same stream so older consumers keep the same distance.
        self.legacy_scroll[axis] += amount;
        let notches = self.legacy_scroll[axis] / WHEEL_UNITS_PER_NOTCH;
        self.legacy_scroll[axis] %= WHEEL_UNITS_PER_NOTCH;
        if notches != 0 {
            out.mouse.push(relative_event([R::REL_HWHEEL, R::REL_WHEEL][axis], notches));
        }
        out.mouse.push(relative_event(
            [R::REL_HWHEEL_HI_RES, R::REL_WHEEL_HI_RES][axis],
            amount,
        ));
    }

    fn reset_scroll(&mut self) {
        self.scroll = [0.0; 2];
        self.scroll_velocity = [0.0; 2];
        self.legacy_scroll = [0; 2];
    }

    fn reset_motion(&mut self) {
        self.motion = [0.0; 2];
        self.velocity = [0.0; 2];
        self.reset_scroll();
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
        self.activation_keys.clear();
        self.scroll_reserved.clear();
        self.toggled = false;
        self.direct_mouse_held = false;
        self.reset_motion();
        out
    }
}

#[derive(Clone, Copy)]
enum Input {
    Key(K, i32),
    Mouse(bool),
}

impl Input {
    fn is_press(self) -> bool {
        matches!(self, Self::Key(_, 1) | Self::Mouse(true))
    }
}

fn instant(easing: f64) -> bool {
    easing == 0.0 || easing == 1.0
}

fn advance_axis(
    remainder: &mut f64,
    velocity: &mut f64,
    target: f64,
    easing: f64,
    seconds: f64,
    units_per_step: f64,
) -> i32 {
    if instant(easing) {
        *velocity = target;
        *remainder += target * seconds;
    } else {
        // Easing is the fraction of the velocity gap closed per 1/60 second.
        // Integrate the exponential exactly so distance does not depend on tick size.
        let rate = -60.0 * (-easing).ln_1p();
        let exponent = rate * seconds;
        let alpha = -(-exponent).exp_m1();
        // Avoid cancellation (or underflow) for very small positive easing factors.
        let accelerated_time = if exponent < 1e-4 {
            seconds * exponent * (0.5 - exponent / 6.0 + exponent * exponent / 24.0)
        } else {
            seconds - alpha / rate
        };
        let gap = target - *velocity;
        *remainder += *velocity * seconds + gap * accelerated_time;
        *velocity += gap * alpha;
        // Stop once the remaining coast would be less than 0.001 input units.
        if target == 0.0 && velocity.abs() < rate * 0.001 {
            *velocity = 0.0;
        }
    }
    let amount = (*remainder / units_per_step).trunc() as i32;
    *remainder -= f64::from(amount) * units_per_step;
    if target == 0.0 && *velocity == 0.0 {
        *remainder = 0.0;
    }
    amount
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

    fn wheel_events(axis: R, notches: i32) -> Vec<(u16, u16, i32)> {
        vec![
            (EventType::RELATIVE.0, axis.0, notches),
            (
                EventType::RELATIVE.0,
                if axis == R::REL_WHEEL { R::REL_WHEEL_HI_RES.0 } else { R::REL_HWHEEL_HI_RES.0 },
                notches * WHEEL_UNITS_PER_NOTCH,
            ),
        ]
    }

    fn engine() -> Engine {
        let mut config = instant_config();
        config.speeds.normal = 1000.0;
        config.speeds.scroll = 10.0;
        Engine::new(config)
    }

    fn toggle_engine() -> Engine {
        let mut config = instant_config();
        config.mode = Mode::Toggle;
        Engine::new(config)
    }

    fn instant_config() -> Config {
        let mut config = Config::default();
        config.easing.movement = 0.0;
        config.easing.scroll = 0.0;
        config
    }

    #[test]
    fn home_end_chords_tap_once_in_either_order_and_activation_mode() {
        for mode in [Mode::Hold, Mode::Toggle] {
            for (first, second, target) in [
                (K::KEY_M, K::KEY_COMMA, K::KEY_HOME),
                (K::KEY_COMMA, K::KEY_M, K::KEY_HOME),
                (K::KEY_N, K::KEY_DOT, K::KEY_END),
                (K::KEY_DOT, K::KEY_N, K::KEY_END),
            ] {
                let mut config = instant_config();
                config.mode = mode;
                let mut e = Engine::new(config);
                e.key(K::KEY_F3, 1);
                if mode == Mode::Toggle {
                    e.key(K::KEY_F3, 0);
                }
                assert!(e.key(first, 1).keyboard.is_empty());
                let tap = events(&[key_event(target, 1), key_event(target, 0)]);
                assert_eq!(events(&e.key(second, 1).keyboard), tap);
                for key in [first, second] {
                    assert!(e.key(key, 2).keyboard.is_empty());
                    assert!(e.key(key, 1).keyboard.is_empty());
                }
                let out = e.advance(Duration::from_millis(50));
                assert!(out.keyboard.is_empty());
                assert!(out.mouse.is_empty());
                assert!(e.key(K::KEY_A, 1).keyboard.is_empty());
                assert!(e.key(first, 0).keyboard.is_empty());
                assert_eq!(events(&e.key(first, 1).keyboard), tap);
                assert!(e.key(second, 0).keyboard.is_empty());
                assert!(e.key(first, 0).keyboard.is_empty());
                assert!(e.release_all().keyboard.is_empty());
            }
        }
    }

    #[test]
    fn home_end_clears_scroll_momentum_and_reserves_held_scroll_keys_until_release() {
        for easing in [0.0, 0.3, 1.0] {
            for (first, second, other, target) in [
                (K::KEY_M, K::KEY_COMMA, K::KEY_N, K::KEY_HOME),
                (K::KEY_COMMA, K::KEY_M, K::KEY_DOT, K::KEY_HOME),
                (K::KEY_N, K::KEY_DOT, K::KEY_M, K::KEY_END),
                (K::KEY_DOT, K::KEY_N, K::KEY_COMMA, K::KEY_END),
            ] {
                for release_first in [first, second] {
                    let mut config = Config::default();
                    config.easing.scroll = easing;
                    let mut e = Engine::new(config);
                    e.key(K::KEY_F3, 1);
                    e.key(first, 1);
                    e.key(other, 1);
                    for _ in 0..10 {
                        e.advance(Duration::from_millis(50));
                    }
                    assert!(e.scroll_velocity.iter().all(|speed| *speed != 0.0));

                    let out = e.key(second, 1);
                    assert_eq!(
                        events(&out.keyboard),
                        events(&[key_event(target, 1), key_event(target, 0)]),
                    );
                    assert!(out.mouse.is_empty());
                    assert_eq!(e.scroll_velocity, [0.0; 2]);
                    assert_eq!(e.scroll, [0.0; 2]);
                    assert_eq!(e.legacy_scroll, [0; 2]);
                    assert!(e.advance(Duration::from_millis(50)).mouse.is_empty());

                    let release_second = if release_first == first { second } else { first };
                    for key in [release_first, release_second, other] {
                        assert!(e.key(key, 2).mouse.is_empty());
                        assert!(e.key(key, 0).mouse.is_empty());
                        for _ in 0..10 {
                            assert!(e.advance(Duration::from_millis(50)).mouse.is_empty());
                        }
                    }
                    assert!(e.scroll_reserved.is_empty());
                    e.key(first, 1);
                    let mut resumed = false;
                    for _ in 0..10 {
                        resumed |= !e.advance(Duration::from_millis(50)).mouse.is_empty();
                    }
                    assert!(resumed);
                    e.key(second, 1);
                    assert!(!e.scroll_reserved.is_empty());
                    e.release_all();
                    assert!(e.scroll_reserved.is_empty());
                }
            }
        }
    }

    #[test]
    fn home_end_disabled_preserves_opposite_scroll_coasting_and_release_behavior() {
        for easing in [0.0, 0.3] {
            let mut config = Config::default();
            config.home_end_enabled = false;
            config.easing.scroll = easing;
            let mut e = Engine::new(config);
            e.key(K::KEY_F3, 1);
            e.key(K::KEY_M, 1);
            e.advance(Duration::from_millis(50));
            assert!(e.key(K::KEY_COMMA, 1).keyboard.is_empty());
            assert!(e.scroll_reserved.is_empty());
            let coasting = e.advance(Duration::from_millis(50));
            assert_eq!(coasting.mouse.is_empty(), easing == 0.0);
            e.key(K::KEY_M, 0);
            let mut resumed = false;
            for _ in 0..10 {
                resumed |= !e.advance(Duration::from_millis(50)).mouse.is_empty();
            }
            assert!(resumed);
        }
    }

    #[test]
    fn home_end_chords_are_inactive_outside_mouse_mode_and_when_disabled() {
        for enabled in [false, true] {
            let mut config = instant_config();
            config.home_end_enabled = enabled;
            let mut e = Engine::new(config);
            for key in [K::KEY_M, K::KEY_COMMA, K::KEY_N, K::KEY_DOT] {
                assert_eq!(events(&e.key(key, 1).keyboard), events(&[key_event(key, 1)]));
            }
            e.release_all();
        }
        let mut config = instant_config();
        config.home_end_enabled = false;
        let mut e = Engine::new(config);
        e.key(K::KEY_F3, 1);
        for key in [K::KEY_M, K::KEY_COMMA, K::KEY_N, K::KEY_DOT] {
            assert!(e.key(key, 1).keyboard.is_empty());
            assert!(e.key(key, 2).keyboard.is_empty());
        }
        assert!(e.advance(Duration::from_millis(50)).mouse.is_empty());
    }

    #[test]
    fn home_end_chords_follow_remapped_scroll_not_movement_keys() {
        let config = Config::parse(
            "[keys]\nscroll_up = 'up'\nscroll_down = 'down'\nscroll_left = 'left'\nscroll_right = 'right'",
        ).unwrap();
        let mut e = Engine::new(config);
        e.key(K::KEY_F3, 1);
        for (first, second, target) in [
            (K::KEY_UP, K::KEY_DOWN, K::KEY_HOME),
            (K::KEY_LEFT, K::KEY_RIGHT, K::KEY_END),
        ] {
            assert!(e.key(first, 1).keyboard.is_empty());
            assert_eq!(
                events(&e.key(second, 1).keyboard),
                events(&[key_event(target, 1), key_event(target, 0)]),
            );
        }
        for key in [K::KEY_N, K::KEY_M, K::KEY_COMMA, K::KEY_DOT] {
            assert_eq!(events(&e.key(key, 1).keyboard), events(&[key_event(key, 1)]));
        }
        for key in [K::KEY_H, K::KEY_J, K::KEY_K, K::KEY_L] {
            assert!(e.key(key, 1).keyboard.is_empty());
        }
    }

    #[test]
    fn home_end_chords_respect_activation_reservations_and_mode_exit() {
        for mode in ["hold", "toggle"] {
            let config = Config::parse(&format!(
                "mode = '{mode}'\n[keys]\nfree_mouse = 'm+comma'",
            )).unwrap();
            let mut e = Engine::new(config);
            e.key(K::KEY_M, 1);
            assert_eq!(events(&e.key(K::KEY_COMMA, 1).keyboard), events(&[key_event(K::KEY_M, 0)]));
            assert!(e.key(K::KEY_M, 2).keyboard.is_empty());
            assert!(e.key(K::KEY_COMMA, 0).keyboard.is_empty());
            assert!(e.key(K::KEY_COMMA, 1).keyboard.is_empty());
        }
        let mut e = toggle_engine();
        e.key(K::KEY_F3, 1);
        e.key(K::KEY_F3, 0);
        e.key(K::KEY_M, 1);
        e.key(K::KEY_COMMA, 1);
        assert!(e.key(K::KEY_F3, 1).keyboard.is_empty());
        assert!(e.key(K::KEY_COMMA, 2).keyboard.is_empty());
        assert!(e.key(K::KEY_M, 0).keyboard.is_empty());
        assert!(e.key(K::KEY_COMMA, 0).keyboard.is_empty());
        e.release_all();
        e.key(K::KEY_F3, 1);
        e.key(K::KEY_M, 1);
        assert_eq!(
            events(&e.key(K::KEY_COMMA, 1).keyboard),
            events(&[key_event(K::KEY_HOME, 1), key_event(K::KEY_HOME, 0)]),
        );
    }

    #[test]
    fn home_end_chords_transfer_preheld_controls_and_preserve_forwarded_keys() {
        let mut e = engine();
        e.key(K::KEY_M, 1);
        e.key(K::KEY_COMMA, 1);
        assert_eq!(
            events(&e.key(K::KEY_F3, 1).keyboard),
            events(&[
                key_event(K::KEY_M, 0),
                key_event(K::KEY_COMMA, 0),
                key_event(K::KEY_HOME, 1),
                key_event(K::KEY_HOME, 0),
            ]),
        );
        e.release_all();
        for (first, second, target) in [
            (K::KEY_M, K::KEY_COMMA, K::KEY_HOME),
            (K::KEY_N, K::KEY_DOT, K::KEY_END),
        ] {
            e.key(target, 1);
            e.key(K::KEY_F3, 1);
            e.key(first, 1);
            assert_eq!(events(&e.key(second, 1).keyboard), events(&[key_event(target, 2)]));
            assert_eq!(events(&e.key(target, 0).keyboard), events(&[key_event(target, 0)]));
            assert!(e.release_all().keyboard.is_empty());
        }
    }

    fn advance_ticks(e: &mut Engine, ticks: usize, millis: u64) -> [i32; 4] {
        let mut total = [0; 4];
        for _ in 0..ticks {
            for event in e.advance(Duration::from_millis(millis)).mouse {
                let axis = match R(event.code()) {
                    R::REL_X => 0,
                    R::REL_Y => 1,
                    R::REL_HWHEEL => 2,
                    R::REL_WHEEL => 3,
                    R::REL_HWHEEL_HI_RES | R::REL_WHEEL_HI_RES => continue,
                    _ => panic!("unexpected axis"),
                };
                total[axis] += event.value();
            }
        }
        total
    }

    #[test]
    fn high_resolution_scroll_emits_many_small_steps_during_the_initial_ramp() {
        for (key, axis, sign) in [
            (K::KEY_N, R::REL_HWHEEL_HI_RES, -1),
            (K::KEY_DOT, R::REL_HWHEEL_HI_RES, 1),
            (K::KEY_M, R::REL_WHEEL_HI_RES, -1),
            (K::KEY_COMMA, R::REL_WHEEL_HI_RES, 1),
        ] {
            let mut e = Engine::new(Config::default());
            e.key(K::KEY_F3, 1);
            assert!(e.key(key, 1).mouse.is_empty());
            let mut steps = Vec::new();
            for _ in 0..25 {
                for event in e.advance(Duration::from_millis(4)).mouse {
                    // The first 100 ms is less than a full notch: only hi-res output.
                    assert_eq!(R(event.code()), axis);
                    let amount = event.value() * sign;
                    assert!((1..120).contains(&amount));
                    steps.push(amount);
                }
            }
            assert!(steps.len() >= 15, "too few ramp updates: {steps:?}");
            assert!(steps.windows(2).all(|pair| pair[1] >= pair[0] - 1));
            assert!(steps.last().unwrap() > steps.first().unwrap());
            let rate = -60.0 * (-0.3_f64).ln_1p();
            let expected = 120.0 * 6.0 * (0.1 - (1.0 - (-rate * 0.1).exp()) / rate);
            assert!((f64::from(steps.iter().sum::<i32>()) - expected).abs() < 1.0);
        }
    }

    #[test]
    fn high_resolution_distance_matches_speed_modes_and_legacy_stream_on_reversal() {
        for (modifier, speed) in [(None, 6.0), (Some(K::KEY_A), 1.5), (Some(K::KEY_S), 24.0)] {
            let mut e = Engine::new(Config::default());
            e.key(K::KEY_F3, 1);
            if let Some(key) = modifier {
                e.key(key, 1);
            }
            for key in [K::KEY_DOT, K::KEY_COMMA] {
                e.key(key, 1);
            }
            let mut high = [0; 2];
            let mut legacy = [0; 2];
            for phase in 0..3 {
                if phase == 1 {
                    e.key(K::KEY_DOT, 0);
                    e.key(K::KEY_COMMA, 0);
                    e.key(K::KEY_N, 1);
                    e.key(K::KEY_M, 1);
                } else if phase == 2 {
                    e.key(K::KEY_N, 0);
                    e.key(K::KEY_M, 0);
                }
                for _ in 0..500 {
                    for event in e.advance(Duration::from_millis(4)).mouse {
                        match R(event.code()) {
                            R::REL_HWHEEL => legacy[0] += event.value(),
                            R::REL_WHEEL => legacy[1] += event.value(),
                            R::REL_HWHEEL_HI_RES => high[0] += event.value(),
                            R::REL_WHEEL_HI_RES => high[1] += event.value(),
                            _ => panic!("unexpected event"),
                        }
                    }
                    for axis in 0..2 {
                        assert_eq!(high[axis] - legacy[axis] * 120, e.legacy_scroll[axis]);
                        assert!(e.legacy_scroll[axis].abs() < 120);
                    }
                }
                if phase == 0 {
                    let rate = -60.0 * (-0.3_f64).ln_1p();
                    let expected = 120.0 * speed * (2.0 - (1.0 - (-rate * 2.0).exp()) / rate);
                    for total in high {
                        assert!((f64::from(total) - expected).abs() < 1.0);
                    }
                }
            }
            // Equal holds in opposite directions, followed by the full coast,
            // cancel to within one high-resolution unit.
            assert!(high.iter().all(|total| total.abs() <= 1));
            assert_eq!(e.scroll_velocity, [0.0; 2]);
        }
    }

    #[test]
    fn eased_short_scroll_tap_is_fractional_and_pending_scroll_is_cleared_on_exit() {
        let mut e = Engine::new(Config::default());
        e.key(K::KEY_F3, 1);
        assert!(e.key(K::KEY_COMMA, 1).mouse.is_empty());
        let output = e.advance(Duration::from_millis(20));
        assert!(!output.mouse.is_empty());
        assert!(output.mouse.iter().all(|event| {
            R(event.code()) == R::REL_WHEEL_HI_RES && (1..120).contains(&event.value())
        }));
        assert_ne!(e.legacy_scroll[1], 0);
        e.key(K::KEY_COMMA, 0);
        assert!(!e.advance(Duration::from_millis(20)).mouse.is_empty());
        e.key(K::KEY_F3, 0);
        assert_eq!(e.legacy_scroll, [0; 2]);
        assert_eq!(e.scroll, [0.0; 2]);
        assert!(e.advance(Duration::from_millis(50)).mouse.is_empty());
        e.key(K::KEY_F3, 1);
        assert!(e.advance(Duration::from_millis(50)).mouse.is_empty());
        e.key(K::KEY_COMMA, 1);
        e.advance(Duration::from_millis(20));
        e.release_all();
        assert_eq!(e.legacy_scroll, [0; 2]);
    }

    #[test]
    fn eased_motion_accelerates_and_coasts_to_a_complete_stop_in_all_directions() {
        for (key, axis, sign) in [
            (K::KEY_H, 0, -1.0),
            (K::KEY_L, 0, 1.0),
            (K::KEY_K, 1, -1.0),
            (K::KEY_J, 1, 1.0),
        ] {
            let mut e = Engine::new(Config::default());
            e.key(K::KEY_F3, 1);
            e.key(key, 1);
            assert_eq!(e.velocity, [0.0; 2]);
            advance_ticks(&mut e, 1, 4);
            assert!(e.velocity[axis] * sign > 0.0);
            assert!(e.velocity[axis] * sign < 300.0);
            advance_ticks(&mut e, 125, 4);
            assert!((e.velocity[axis] - sign * 300.0).abs() < 1.0);
            let prior = e.velocity;
            e.key(key, 0);
            assert_eq!(e.velocity, prior);
            let coast = advance_ticks(&mut e, 1, 50);
            assert!(f64::from(coast[axis]) * sign > 0.0);
            assert!(e.velocity[axis].abs() < prior[axis].abs());
            advance_ticks(&mut e, 500, 4);
            assert_eq!(e.velocity, [0.0; 2]);
            assert_eq!(e.motion, [0.0; 2]);
            assert_eq!(advance_ticks(&mut e, 100, 4), [0; 4]);
        }
    }

    #[test]
    fn direction_changes_curve_and_opposites_decelerate_without_diagonal_speed_boost() {
        let mut e = Engine::new(Config::default());
        e.key(K::KEY_F3, 1);
        e.key(K::KEY_L, 1);
        advance_ticks(&mut e, 125, 4);
        e.key(K::KEY_L, 0);
        e.key(K::KEY_J, 1);
        let turn = advance_ticks(&mut e, 1, 50);
        assert!(turn[0] > 0 && turn[1] > 0);
        assert!(e.velocity[0] > 0.0 && e.velocity[1] > 0.0);
        e.key(K::KEY_L, 1);
        for _ in 0..125 {
            advance_ticks(&mut e, 1, 4);
            assert!(e.velocity[0].hypot(e.velocity[1]) <= 300.0 + 1e-9);
        }
        assert!((e.velocity[0] - 300.0 / 2.0_f64.sqrt()).abs() < 1.0);
        e.key(K::KEY_H, 1);
        e.key(K::KEY_K, 1);
        assert_eq!(e.motion_direction(), [0; 2]);
        assert!(advance_ticks(&mut e, 1, 50)[0] > 0);
        advance_ticks(&mut e, 500, 4);
        assert_eq!(e.velocity, [0.0; 2]);
        e.key(K::KEY_L, 0);
        e.key(K::KEY_J, 0);
        let reverse = advance_ticks(&mut e, 10, 4);
        assert!(reverse[0] < 0 && reverse[1] < 0);
    }

    #[test]
    fn easing_applies_to_speed_modes_with_slow_priority() {
        let mut e = Engine::new(Config::default());
        e.key(K::KEY_F3, 1);
        e.key(K::KEY_L, 1);
        e.key(K::KEY_COMMA, 1);
        advance_ticks(&mut e, 125, 4);
        for (key, value, target, scroll_target) in [
            (K::KEY_S, 1, 900.0, 24.0),
            (K::KEY_A, 1, 100.0, 1.5),
            (K::KEY_A, 0, 900.0, 24.0),
            (K::KEY_S, 0, 300.0, 6.0),
        ] {
            let before = [e.velocity[0], e.scroll_velocity[1]];
            e.key(key, value);
            assert_eq!([e.velocity[0], e.scroll_velocity[1]], before);
            assert_eq!(e.speed(), target);
            assert_eq!(e.scroll_speed(), scroll_target);
            advance_ticks(&mut e, 1, 4);
            for (old, current, target) in [
                (before[0], e.velocity[0], target),
                (before[1], e.scroll_velocity[1], scroll_target),
            ] {
                assert!(current > old.min(target) && current < old.max(target));
            }
            advance_ticks(&mut e, 250, 4);
            assert!((e.velocity[0] - target).abs() < 0.01);
            assert!((e.scroll_velocity[1] - scroll_target).abs() < 0.01);
        }
    }

    #[test]
    fn scroll_eases_on_both_axes_without_an_initial_jump_and_preserves_coasting() {
        for (horizontal, vertical, sign) in [
            (K::KEY_N, K::KEY_M, -1),
            (K::KEY_DOT, K::KEY_COMMA, 1),
        ] {
            let mut e = Engine::new(Config::default());
            e.key(K::KEY_F3, 1);
            e.key(K::KEY_S, 1);
            for key in [horizontal, vertical] {
                assert!(e.key(key, 1).mouse.is_empty());
                assert!(e.key(key, 2).mouse.is_empty());
            }
            assert_eq!(e.scroll_velocity, [0.0; 2]);
            advance_ticks(&mut e, 1, 4);
            assert!(e.scroll_velocity[0].abs() > 0.0);
            assert!(e.scroll_velocity[0].abs() < 24.0);
            advance_ticks(&mut e, 125, 4);
            let fraction = e.scroll;
            for key in [horizontal, vertical] {
                assert!(e.key(key, 0).mouse.is_empty());
            }
            assert_eq!(e.scroll, fraction);
            let tail = advance_ticks(&mut e, 500, 4);
            assert!(tail[2] * sign > 0 && tail[3] * sign > 0);
            assert_eq!(e.scroll_velocity, [0.0; 2]);
            assert_eq!(e.scroll, [0.0; 2]);
            assert_eq!(advance_ticks(&mut e, 100, 4), [0; 4]);
        }
    }

    #[test]
    fn mode_exit_and_cleanup_discard_all_eased_velocity_and_remainders() {
        for text in ["", "mode = 'toggle'", "[keys]\nfree_mouse = 'leftalt+f3'"] {
            for cleanup in [false, true] {
                let config = Config::parse(text).unwrap();
                let mut e = Engine::new(config);
                e.key(K::KEY_LEFTALT, 1);
                e.key(K::KEY_F3, 1);
                if e.config.mode == Mode::Toggle {
                    e.key(K::KEY_F3, 0);
                }
                for key in [K::KEY_L, K::KEY_M, K::KEY_SPACE] {
                    e.key(key, 1);
                }
                advance_ticks(&mut e, 40, 4);
                assert!(e.velocity[0] > 0.0 && e.scroll_velocity[1] < 0.0);
                assert_eq!(
                    events(&e.key(K::KEY_SPACE, 0).mouse),
                    vec![(EventType::KEY.0, K::BTN_LEFT.0, 0)]
                );
                if cleanup {
                    e.release_all();
                } else if e.config.mode == Mode::Toggle {
                    e.key(K::KEY_F3, 1);
                } else if e.config.keys.free_mouse.len() > 1 {
                    e.key(K::KEY_LEFTALT, 0);
                } else {
                    e.key(K::KEY_F3, 0);
                }
                assert!(!e.active());
                assert_eq!(e.velocity, [0.0; 2]);
                assert_eq!(e.scroll_velocity, [0.0; 2]);
                assert_eq!(e.motion, [0.0; 2]);
                assert_eq!(e.scroll, [0.0; 2]);
                assert_eq!(e.legacy_scroll, [0; 2]);
                assert_eq!(advance_ticks(&mut e, 50, 4), [0; 4]);
                for key in [K::KEY_L, K::KEY_M, K::KEY_F3, K::KEY_LEFTALT] {
                    e.key(key, 0);
                }
                e.key(K::KEY_LEFTALT, 1);
                e.key(K::KEY_F3, 1);
                assert!(e.active());
                assert_eq!(advance_ticks(&mut e, 50, 4), [0; 4]);
            }
        }
    }

    #[test]
    fn easing_is_time_based_and_stalls_are_capped() {
        fn moving_engine() -> Engine {
            let mut e = Engine::new(Config::default());
            e.key(K::KEY_F3, 1);
            e.key(K::KEY_L, 1);
            e.key(K::KEY_J, 1);
            e.key(K::KEY_COMMA, 1);
            e
        }
        let mut fine = moving_engine();
        let mut coarse = moving_engine();
        let fine_total = advance_ticks(&mut fine, 250, 4);
        let coarse_total = advance_ticks(&mut coarse, 20, 50);
        for axis in 0..4 {
            assert!((fine_total[axis] - coarse_total[axis]).abs() <= 1);
        }
        for axis in 0..2 {
            assert!((fine.velocity[axis] - coarse.velocity[axis]).abs() < 1e-9);
            assert!((fine.scroll_velocity[axis] - coarse.scroll_velocity[axis]).abs() < 1e-9);
        }
        let mut stalled = moving_engine();
        let mut capped = moving_engine();
        assert_eq!(advance_ticks(&mut stalled, 1, 5000), advance_ticks(&mut capped, 1, 50));
        assert_eq!(stalled.velocity, capped.velocity);
        assert_eq!(stalled.scroll_velocity, capped.scroll_velocity);
        let before = stalled.velocity;
        assert!(stalled.advance(Duration::ZERO).mouse.is_empty());
        assert_eq!(stalled.velocity, before);
    }

    #[test]
    fn easing_can_be_disabled_independently_and_one_is_instantaneous() {
        for (movement, scroll) in [(0.0, 0.3), (0.2, 0.0), (1.0, 1.0)] {
            let mut config = Config::default();
            config.easing.movement = movement;
            config.easing.scroll = scroll;
            let mut e = Engine::new(config);
            e.key(K::KEY_F3, 1);
            e.key(K::KEY_L, 1);
            e.key(K::KEY_COMMA, 1);
            advance_ticks(&mut e, 1, 50);
            assert_eq!(e.velocity[0] == 300.0, instant(movement));
            assert_eq!(e.scroll_velocity[1] == 6.0, instant(scroll));
            e.key(K::KEY_L, 0);
            e.key(K::KEY_COMMA, 0);
            advance_ticks(&mut e, 1, 50);
            assert_eq!(e.velocity[0] == 0.0, instant(movement));
            assert_eq!(e.scroll_velocity[1] == 0.0, instant(scroll));
        }
    }

    #[test]
    fn easing_factors_match_sixty_hertz_response_and_handle_tiny_values() {
        for easing in [0.2, 0.3] {
            let mut velocity = 0.0;
            let mut remainder = 0.0;
            advance_axis(&mut remainder, &mut velocity, 300.0, easing, 1.0 / 60.0, 1.0);
            assert!((velocity - 300.0 * easing).abs() < 1e-9);
        }
        for easing in [f64::from_bits(1), 1e-300, 1e-16, 1e-8] {
            let mut velocity = 0.0;
            let mut remainder = 0.0;
            for _ in 0..250 {
                assert_eq!(advance_axis(&mut remainder, &mut velocity, 300.0, easing, 0.004, 1.0), 0);
                assert!(velocity.is_finite() && velocity >= 0.0);
                assert!(remainder.is_finite() && remainder >= 0.0);
            }
            assert!(velocity < 0.001);
            assert!(remainder < 0.001);
        }
    }

    #[test]
    fn toggle_changes_only_on_new_presses_not_releases_or_repeats() {
        let mut e = toggle_engine();
        assert!(!e.active());
        for value in [1, 1, 2, 0, 0, 2] {
            assert!(e.key(K::KEY_F3, value).keyboard.is_empty());
            assert!(e.active());
        }
        for value in [1, 1, 2, 0] {
            assert!(e.key(K::KEY_F3, value).keyboard.is_empty());
            assert!(!e.active());
        }
        e.key(K::KEY_F3, 1);
        assert!(e.active());
    }

    #[test]
    fn toggle_off_releases_buttons_stops_motion_and_suppresses_held_controls() {
        let mut e = toggle_engine();
        e.key(K::KEY_F3, 1);
        e.key(K::KEY_F3, 0);
        for key in [K::KEY_L, K::KEY_SPACE, K::KEY_I, K::KEY_M, K::KEY_S] {
            assert!(e.key(key, 1).keyboard.is_empty());
        }
        assert!(!e.advance(Duration::from_millis(10)).mouse.is_empty());
        let output = e.key(K::KEY_F3, 1);
        assert_eq!(
            events(&output.mouse),
            vec![
                (EventType::KEY.0, K::BTN_LEFT.0, 0),
                (EventType::KEY.0, K::BTN_RIGHT.0, 0),
            ]
        );
        assert!(!e.active());
        assert!(e.advance(Duration::from_millis(50)).mouse.is_empty());
        for key in [K::KEY_L, K::KEY_SPACE, K::KEY_I, K::KEY_M, K::KEY_S] {
            assert!(e.key(key, 2).keyboard.is_empty());
            assert!(e.key(key, 0).keyboard.is_empty());
            assert_eq!(e.key(key, 1).keyboard.len(), 1);
        }
    }

    #[test]
    fn toggle_mode_supports_speed_modifiers_and_cleanup() {
        let mut e = toggle_engine();
        e.key(K::KEY_F3, 1);
        e.key(K::KEY_F3, 0);
        e.key(K::KEY_L, 1);
        for (key, value, distance) in [
            (K::KEY_L, 2, 3),
            (K::KEY_S, 1, 9),
            (K::KEY_A, 1, 1),
            (K::KEY_A, 0, 9),
            (K::KEY_S, 0, 3),
        ] {
            assert!(e.key(key, value).keyboard.is_empty());
            assert_eq!(e.advance(Duration::from_millis(10)).mouse[0].value(), distance);
        }
        assert_eq!(e.key(K::KEY_ESC, 1).keyboard.len(), 1);
        assert!(e.active());
        e.release_all();
        assert!(!e.active());
        assert!(e.advance(Duration::from_millis(10)).mouse.is_empty());
    }

    #[test]
    fn configured_toggle_key_transfers_preheld_controls() {
        let config = Config::parse("mode = 'toggle'\n[keys]\nfree_mouse = 'f4'").unwrap();
        let mut e = Engine::new(config);
        assert_eq!(e.key(K::KEY_F3, 1).keyboard.len(), 1);
        assert!(!e.active());
        e.key(K::KEY_SPACE, 1);
        let output = e.key(K::KEY_F4, 1);
        assert_eq!(events(&output.keyboard), vec![(EventType::KEY.0, K::KEY_SPACE.0, 0)]);
        assert_eq!(events(&output.mouse), vec![(EventType::KEY.0, K::BTN_LEFT.0, 1)]);
        e.key(K::KEY_F4, 0);
        assert!(e.active());
        assert_eq!(events(&e.key(K::KEY_SPACE, 0).mouse), vec![(EventType::KEY.0, K::BTN_LEFT.0, 0)]);
        e.key(K::KEY_F4, 1);
        assert!(!e.active());
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
        let mut config = instant_config();
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
            let expected = wheel_events(axis, amount);
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
    fn escape_passes_through_without_leaving_mouse_mode() {
        for mode in [Mode::Hold, Mode::Toggle] {
            let mut config = Config::default();
            config.mode = mode;
            let mut e = Engine::new(config);
            for active in [false, true] {
                if active {
                    e.key(K::KEY_F3, 1);
                }
                for value in [1, 2, 0] {
                    assert_eq!(
                        events(&e.key(K::KEY_ESC, value).keyboard),
                        vec![(EventType::KEY.0, K::KEY_ESC.0, value)]
                    );
                    assert_eq!(e.active(), active);
                }
            }
        }
    }

    #[test]
    fn hold_chord_activates_in_either_order_without_clicking_or_leaking_modifiers() {
        for (first, second) in [
            (K::KEY_LEFTALT, K::KEY_SPACE),
            (K::KEY_SPACE, K::KEY_LEFTALT),
        ] {
            for release_first in [first, second] {
                let config = Config::parse("[keys]\nfree_mouse = 'leftalt + space'").unwrap();
                let mut e = Engine::new(config);
                assert_eq!(e.key(first, 1).keyboard.len(), 1);
                assert!(!e.active());
                let output = e.key(second, 1);
                assert_eq!(events(&output.keyboard), vec![(EventType::KEY.0, first.0, 0)]);
                assert!(output.mouse.is_empty());
                assert!(e.active());
                for key in [first, second] {
                    for value in [1, 2] {
                        let output = e.key(key, value);
                        assert!(output.keyboard.is_empty());
                        assert!(output.mouse.is_empty());
                    }
                }
                e.key(K::KEY_I, 1);
                e.key(K::KEY_L, 1);
                e.key(K::KEY_M, 1);
                let output = e.key(release_first, 0);
                assert!(output.keyboard.is_empty());
                assert_eq!(events(&output.mouse), vec![(EventType::KEY.0, K::BTN_RIGHT.0, 0)]);
                assert!(!e.active());
                assert!(e.advance(Duration::from_millis(50)).mouse.is_empty());
                for key in [first, second, K::KEY_I, K::KEY_L, K::KEY_M] {
                    assert!(e.key(key, 2).keyboard.is_empty());
                    assert!(e.key(key, 0).keyboard.is_empty());
                }
                assert!(e.release_all().keyboard.is_empty());
                for value in [1, 2, 0] {
                    assert_eq!(e.key(K::KEY_SPACE, value).keyboard.len(), 1);
                }
            }
        }
    }

    #[test]
    fn toggle_chord_reserves_members_until_release_and_can_be_repressed() {
        let config = Config::parse("mode = 'toggle'\n[keys]\nfree_mouse = 'leftalt+space'").unwrap();
        let mut e = Engine::new(config);
        e.key(K::KEY_LEFTALT, 1);
        assert!(e.key(K::KEY_SPACE, 1).mouse.is_empty());
        assert!(e.active());
        for key in [K::KEY_LEFTALT, K::KEY_SPACE] {
            for value in [1, 2] {
                assert!(e.key(key, value).mouse.is_empty());
                assert!(e.active());
            }
        }
        e.key(K::KEY_SPACE, 0);
        assert!(e.active());
        assert!(e.key(K::KEY_SPACE, 1).mouse.is_empty());
        assert!(!e.active());
        e.key(K::KEY_SPACE, 0);
        e.key(K::KEY_SPACE, 1);
        assert!(e.active());
        e.key(K::KEY_LEFTALT, 0);
        assert!(e.key(K::KEY_SPACE, 2).mouse.is_empty());
        e.key(K::KEY_SPACE, 0);
        assert_eq!(
            events(&e.key(K::KEY_SPACE, 1).mouse),
            vec![(EventType::KEY.0, K::BTN_LEFT.0, 1)]
        );
        let output = e.key(K::KEY_LEFTALT, 1);
        assert!(output.keyboard.is_empty());
        assert_eq!(events(&output.mouse), vec![(EventType::KEY.0, K::BTN_LEFT.0, 0)]);
        assert!(!e.active());
        assert!(e.release_all().keyboard.is_empty());
        assert!(!e.active());
        e.key(K::KEY_SPACE, 1);
        assert!(!e.active());
    }

    #[test]
    fn chord_members_do_not_also_move_scroll_or_change_speed() {
        let config = Config::parse(
            "[keys]\nfree_mouse = 'leftalt+l+m+a+s'",
        ).unwrap();
        let mut e = Engine::new(config);
        for key in [K::KEY_L, K::KEY_M, K::KEY_A, K::KEY_S] {
            e.key(key, 1);
            assert!(!e.active());
        }
        let output = e.key(K::KEY_LEFTALT, 1);
        assert_eq!(output.keyboard.len(), 4);
        assert!(output.mouse.is_empty());
        assert!(e.active());
        assert_eq!(e.speed(), 300.0);
        assert_eq!(e.scroll_speed(), 6.0);
        assert!(e.advance(Duration::from_millis(50)).mouse.is_empty());
        e.release_all();
        assert!(e.activation_keys.is_empty());
        assert!(!e.active());
    }

    #[test]
    fn held_speed_keys_switch_immediately_and_slow_wins() {
        let mut e = Engine::new(instant_config());
        e.key(K::KEY_F3, 1);
        e.key(K::KEY_L, 1);
        let mut step = |key, value, distance| {
            assert!(e.key(key, value).keyboard.is_empty());
            assert_eq!(
                events(&e.advance(Duration::from_millis(10)).mouse),
                vec![(EventType::RELATIVE.0, R::REL_X.0, distance)]
            );
        };
        step(K::KEY_F3, 2, 3);
        step(K::KEY_S, 1, 9);
        step(K::KEY_S, 2, 9);
        step(K::KEY_S, 0, 3);
        step(K::KEY_A, 1, 1);
        step(K::KEY_A, 2, 1);
        step(K::KEY_S, 1, 1);
        step(K::KEY_S, 0, 1);
        step(K::KEY_S, 1, 1);
        step(K::KEY_A, 0, 9);
        step(K::KEY_S, 0, 3);
    }

    #[test]
    fn speed_keys_transfer_on_entry_and_stay_suppressed_until_released() {
        for key in [K::KEY_A, K::KEY_S] {
            let mut e = Engine::new(instant_config());
            assert_eq!(e.key(key, 1).keyboard.len(), 1);
            assert_eq!(
                events(&e.key(K::KEY_F3, 1).keyboard),
                vec![(EventType::KEY.0, key.0, 0)]
            );
            e.key(K::KEY_L, 1);
            assert_eq!(
                e.advance(Duration::from_millis(10)).mouse[0].value(),
                if key == K::KEY_A { 1 } else { 9 }
            );
            e.key(K::KEY_F3, 0);
            assert_eq!(e.speed(), 300.0);
            assert!(e.advance(Duration::from_millis(10)).mouse.is_empty());
            assert!(e.key(key, 2).keyboard.is_empty());
            assert!(e.key(key, 0).keyboard.is_empty());
            assert_eq!(e.key(key, 1).keyboard.len(), 1);
            e.key(key, 0);
            e.key(K::KEY_F3, 1);
            assert_eq!(e.speed(), 300.0);
        }
    }

    #[test]
    fn speed_modes_preserve_diagonal_normalization_and_modify_scroll_rate() {
        for (key, expected, scroll) in [(K::KEY_S, 636, 24.0), (K::KEY_A, 70, 1.5)] {
            let mut e = Engine::new(instant_config());
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
                        R::REL_WHEEL_HI_RES => continue,
                        _ => panic!("unexpected axis"),
                    };
                    totals[axis] += event.value();
                }
            }
            assert_eq!(totals[..2], [expected, expected]);
            assert!((scroll - 1.0..=scroll).contains(&f64::from(totals[2])));
        }
    }

    #[test]
    fn default_scroll_modes_match_mouseless_steady_state_rates() {
        for (modifier, rate) in [(None, 6.0), (Some(K::KEY_A), 1.5), (Some(K::KEY_S), 24.0)] {
            let mut e = Engine::new(instant_config());
            e.key(K::KEY_F3, 1);
            if let Some(key) = modifier {
                e.key(key, 1);
            }
            for key in [K::KEY_DOT, K::KEY_COMMA] {
                assert_eq!(e.key(key, 1).mouse[0].value(), 1);
            }
            assert_eq!(e.scroll_speed(), rate);
            let mut totals = [0; 2];
            for _ in 0..1000 {
                for event in e.advance(Duration::from_millis(4)).mouse {
                    let axis = match R(event.code()) {
                        R::REL_HWHEEL => 0,
                        R::REL_WHEEL => 1,
                        R::REL_HWHEEL_HI_RES | R::REL_WHEEL_HI_RES => continue,
                        _ => panic!("unexpected axis"),
                    };
                    totals[axis] += event.value();
                }
            }
            for total in totals {
                assert!((f64::from(total) - rate * 4.0).abs() <= 1.0);
            }
        }
    }

    #[test]
    fn configured_scroll_speeds_switch_immediately_with_slow_priority_on_both_axes() {
        for mode in ["hold", "toggle"] {
            let config = Config::parse(&format!(
                "mode = '{mode}'\n[easing]\nscroll = 0\n[speeds]\nscroll = 40\nscroll_slow = 20\nscroll_fast = 80"
            )).unwrap();
            let mut e = Engine::new(config);
            e.key(K::KEY_F3, 1);
            if mode == "toggle" {
                e.key(K::KEY_F3, 0);
            }
            for key in [K::KEY_N, K::KEY_M] {
                assert_eq!(e.key(key, 1).mouse[0].value(), -1);
            }
            for (key, value, amount) in [
                (K::KEY_M, 2, -2),
                (K::KEY_S, 1, -4),
                (K::KEY_S, 2, -4),
                (K::KEY_A, 1, -1),
                (K::KEY_S, 0, -1),
                (K::KEY_S, 1, -1),
                (K::KEY_A, 0, -4),
                (K::KEY_S, 0, -2),
            ] {
                let output = e.key(key, value);
                assert!(output.keyboard.is_empty());
                assert!(output.mouse.is_empty());
                assert_eq!(
                    events(&e.advance(Duration::from_millis(50)).mouse),
                    [wheel_events(R::REL_HWHEEL, amount), wheel_events(R::REL_WHEEL, amount)].concat()
                );
            }
        }
    }

    #[test]
    fn scroll_speed_changes_preserve_fractional_progress_without_extra_notches() {
        let config = Config::parse("[easing]\nscroll = 0\n[speeds]\nscroll = 10\nscroll_slow = 5\nscroll_fast = 20").unwrap();
        let mut e = Engine::new(config);
        e.key(K::KEY_F3, 1);
        e.key(K::KEY_COMMA, 1);
        assert!(e.advance(Duration::from_millis(50)).mouse.is_empty());
        assert!(e.key(K::KEY_S, 1).mouse.is_empty());
        assert_eq!(e.advance(Duration::from_millis(25)).mouse[0].value(), 1);
        assert!(e.key(K::KEY_A, 1).mouse.is_empty());
        for _ in 0..3 {
            assert!(e.advance(Duration::from_millis(50)).mouse.is_empty());
        }
        assert_eq!(e.advance(Duration::from_millis(50)).mouse[0].value(), 1);
    }

    #[test]
    fn all_configured_bindings_replace_defaults() {
        let config = Config::parse(
            r#"
            [easing]
            movement = 0
            scroll = 0
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
                wheel_events(axis, value)
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
        assert_eq!(e.key(K::KEY_F12, 1).keyboard.len(), 1);
        assert_eq!(e.key(K::KEY_ESC, 1).keyboard.len(), 1);
        assert!(e.active());
    }
}
