use std::time::Duration;

use evdev::KeyCode;

use crate::config;

pub struct Keycast {
    on: bool,
    keys: Vec<KeyCode>,
    idle: Duration,
    timeout: Duration,
    max_keys: usize,
}

impl Keycast {
    pub fn new(config: &config::Keycast) -> Self {
        Self {
            on: false,
            keys: Vec::new(),
            idle: Duration::ZERO,
            timeout: Duration::from_secs_f64(config.timeout),
            max_keys: config.max_keys,
        }
    }

    pub fn keys(&self) -> &[KeyCode] {
        &self.keys
    }

    pub fn toggle(&mut self) {
        self.on = !self.on;
        self.clear();
    }

    pub fn press(&mut self, key: KeyCode) {
        if self.on {
            if self.keys.len() == self.max_keys {
                self.keys.remove(0);
            }
            self.keys.push(key);
            self.idle = Duration::ZERO;
        }
    }

    pub fn advance(&mut self, elapsed: Duration) {
        if !self.keys.is_empty() {
            self.idle = self.idle.saturating_add(elapsed);
            if self.idle >= self.timeout {
                self.clear();
            }
        }
    }

    pub fn clear(&mut self) {
        self.keys.clear();
        self.idle = Duration::ZERO;
    }

    pub fn reset(&mut self) {
        self.on = false;
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evdev::KeyCode as K;

    #[test]
    fn history_rolls_over_expires_and_restarts_idle_timer() {
        let mut caster = Keycast::new(&config::Keycast {
            max_keys: 2,
            ..config::Keycast::default()
        });
        caster.press(K::KEY_H);
        assert!(caster.keys().is_empty());
        caster.toggle();
        caster.press(K::KEY_K);
        caster.advance(Duration::from_secs(2));
        caster.press(K::KEY_L);
        caster.advance(Duration::from_secs(2));
        assert_eq!(caster.keys(), [K::KEY_K, K::KEY_L]);
        caster.press(K::KEY_SPACE);
        assert_eq!(caster.keys(), [K::KEY_L, K::KEY_SPACE]);
        caster.advance(Duration::from_millis(2999));
        assert_eq!(caster.keys().len(), 2);
        caster.advance(Duration::from_millis(1));
        assert!(caster.keys().is_empty());
        caster.press(K::KEY_H);
        caster.toggle();
        assert!(caster.keys().is_empty());
        caster.press(K::KEY_L);
        assert!(caster.keys().is_empty());
    }

    #[test]
    fn single_key_limit_and_full_elapsed_time() {
        let mut caster = Keycast::new(&config::Keycast {
            max_keys: 1,
            ..config::Keycast::default()
        });
        caster.toggle();
        caster.press(K::KEY_K);
        caster.press(K::KEY_L);
        assert_eq!(caster.keys(), [K::KEY_L]);
        caster.advance(Duration::MAX);
        assert!(caster.keys().is_empty());
        caster.reset();
        caster.press(K::KEY_K);
        assert!(caster.keys().is_empty());
    }
}
