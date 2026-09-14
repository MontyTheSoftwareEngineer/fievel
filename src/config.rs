use evdev::KeyCode as K;
use serde::{Deserialize, Deserializer};
use std::{collections::BTreeMap, error::Error, fs, io, path::Path};

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub mode: Mode,
    pub notify: bool,
    pub home_end_enabled: bool,
    pub speeds: Speeds,
    pub easing: Easing,
    pub keys: Keys,
    pub keycast: Keycast,
    pub remap: crate::remap::RemapConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::default(),
            notify: true,
            home_end_enabled: true,
            speeds: Speeds::default(),
            easing: Easing::default(),
            keys: Keys::default(),
            keycast: Keycast::default(),
            remap: crate::remap::RemapConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keycast {
    pub enabled: bool,
    #[serde(deserialize_with = "deserialize_key")]
    pub hotkey: K,
    pub max_keys: usize,
    pub timeout: f64,
}

impl Default for Keycast {
    fn default() -> Self {
        Self {
            enabled: false,
            hotkey: K::KEY_ESC,
            max_keys: 8,
            timeout: 3.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Hold,
    Toggle,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Speeds {
    pub normal: f64,
    pub slow: f64,
    pub fast: f64,
    pub scroll: f64,
    pub scroll_slow: f64,
    pub scroll_fast: f64,
}

impl Default for Speeds {
    fn default() -> Self {
        Self {
            normal: 300.0,
            slow: 100.0,
            fast: 900.0,
            scroll: 6.0,
            scroll_slow: 1.5,
            scroll_fast: 24.0,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Easing {
    pub movement: f64,
    pub scroll: f64,
}

impl Default for Easing {
    fn default() -> Self {
        Self {
            movement: 0.2,
            scroll: 0.3,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keys {
    #[serde(deserialize_with = "deserialize_chord")]
    pub free_mouse: Vec<K>,
    #[serde(deserialize_with = "deserialize_key")]
    pub left: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub down: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub up: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub right: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub left_click: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub right_click: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub scroll_left: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub scroll_down: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub scroll_up: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub scroll_right: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub slow: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub fast: K,
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            free_mouse: vec![K::KEY_F3],
            left: K::KEY_H,
            down: K::KEY_J,
            up: K::KEY_K,
            right: K::KEY_L,
            left_click: K::KEY_SPACE,
            right_click: K::KEY_I,
            scroll_left: K::KEY_N,
            scroll_down: K::KEY_M,
            scroll_up: K::KEY_COMMA,
            scroll_right: K::KEY_DOT,
            slow: K::KEY_A,
            fast: K::KEY_S,
        }
    }
}

impl Keys {
    pub fn controls(&self) -> [K; 12] {
        [
            self.left,
            self.down,
            self.up,
            self.right,
            self.left_click,
            self.right_click,
            self.scroll_left,
            self.scroll_down,
            self.scroll_up,
            self.scroll_right,
            self.slow,
            self.fast,
        ]
    }

    pub fn named(&self) -> Vec<(&'static str, K)> {
        self.free_mouse
            .iter()
            .map(|key| ("free_mouse", *key))
            .chain(self.named_controls())
            .collect()
    }

    fn named_controls(&self) -> [(&'static str, K); 12] {
        [
            ("left", self.left),
            ("down", self.down),
            ("up", self.up),
            ("right", self.right),
            ("left_click", self.left_click),
            ("right_click", self.right_click),
            ("scroll_left", self.scroll_left),
            ("scroll_down", self.scroll_down),
            ("scroll_up", self.scroll_up),
            ("scroll_right", self.scroll_right),
            ("slow", self.slow),
            ("fast", self.fast),
        ]
    }
}

fn deserialize_key<'de, D: Deserializer<'de>>(deserializer: D) -> Result<K, D::Error> {
    let value = String::deserialize(deserializer)?;
    parse_key(&value).map_err(serde::de::Error::custom)
}

fn deserialize_chord<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<K>, D::Error> {
    let value = String::deserialize(deserializer)?;
    value
        .split('+')
        .map(parse_key)
        .collect::<Result<_, _>>()
        .map_err(serde::de::Error::custom)
}

pub(crate) fn parse_key(value: &str) -> Result<K, String> {
    let upper = value.trim().to_ascii_uppercase();
    let name = match upper.as_str() {
        "," => "COMMA",
        "." => "DOT",
        "ESCAPE" => "ESC",
        "RETURN" => "ENTER",
        other => other,
    };
    let name = if name.starts_with("KEY_") {
        name.to_owned()
    } else {
        format!("KEY_{name}")
    };
    let key = name
        .parse::<K>()
        .map_err(|_| format!("unknown keyboard key {value:?}"))?;
    if key == K::KEY_RESERVED || key == K::KEY_UNKNOWN {
        return Err("reserved/unknown keys cannot be bound".to_owned());
    }
    Ok(key)
}

pub fn validate_speed(speed: f64) -> Result<(), String> {
    if speed.is_finite() && speed > 0.0 && speed <= 100_000.0 {
        Ok(())
    } else {
        Err("speed must be finite, greater than zero, and at most 100000".to_owned())
    }
}

impl Config {
    pub fn mouse_controls(&self) -> Vec<K> {
        let mut controls = self.keys.controls().to_vec();
        if self.keycast.enabled {
            controls.push(self.keycast.hotkey);
        }
        controls
    }

    pub fn parse(text: &str) -> Result<Self, Box<dyn Error>> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    pub fn load(path: &Path, required: bool) -> Result<Self, Box<dyn Error>> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text)
                .map_err(|error| format!("Invalid config {}: {error}", path.display()).into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound && !required => {
                eprintln!("No config at {}; using defaults", path.display());
                Ok(Self::default())
            }
            Err(error) => Err(format!("Cannot read config {}: {error}", path.display()).into()),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        self.remap.validate()?;
        if !(1..=32).contains(&self.keycast.max_keys) {
            return Err("keycast.max_keys: must be between 1 and 32".to_owned());
        }
        if !self.keycast.timeout.is_finite()
            || self.keycast.timeout <= 0.0
            || self.keycast.timeout > 60.0
        {
            return Err("keycast.timeout: must be finite, greater than zero, and at most 60 seconds".to_owned());
        }
        if self.keycast.enabled
            && self.keys.named().iter().any(|(_, key)| *key == self.keycast.hotkey)
        {
            return Err("keycast.hotkey must differ from Free Mouse Mode activation and control keys".to_owned());
        }
        for (name, value) in [
            ("movement", self.easing.movement),
            ("scroll", self.easing.scroll),
        ] {
            if !value.is_finite() || !(0.0..=1.0).contains(&value) {
                return Err(format!("easing.{name}: must be finite and between 0 and 1"));
            }
        }
        for (name, value) in [
            ("normal", self.speeds.normal),
            ("slow", self.speeds.slow),
            ("fast", self.speeds.fast),
            ("scroll", self.speeds.scroll),
            ("scroll_slow", self.speeds.scroll_slow),
            ("scroll_fast", self.speeds.scroll_fast),
        ] {
            validate_speed(value).map_err(|error| format!("speeds.{name}: {error}"))?;
        }
        let mut assigned = BTreeMap::new();
        for (name, key) in self.keys.named() {
            // Chord members may also be controls, but a single activation key may not.
            if name == "free_mouse" && self.keys.free_mouse.len() > 1 {
                continue;
            }
            if let Some(other) = assigned.insert(key, name) {
                return Err(format!("keys.{name} and keys.{other} both bind {key:?}"));
            }
        }
        let mut chord = std::collections::BTreeSet::new();
        if self.keys.free_mouse.is_empty() {
            return Err("keys.free_mouse must contain at least one key".to_owned());
        }
        for key in &self.keys.free_mouse {
            if !chord.insert(key) {
                return Err(format!("keys.free_mouse repeats {key:?}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_matches_defaults() {
        let config = Config::parse(include_str!("../fievel.config")).unwrap();
        assert_eq!(config.mode, Mode::Hold);
        assert!(config.notify);
        assert!(config.home_end_enabled);
        assert!(!config.keycast.enabled);
        assert_eq!(config.keycast.hotkey, K::KEY_ESC);
        assert_eq!(config.keycast.max_keys, 8);
        assert_eq!(config.keycast.timeout, 3.0);
        assert_eq!(config.keys.named(), Keys::default().named());
        assert_eq!(config.easing.movement, 0.2);
        assert_eq!(config.easing.scroll, 0.3);
        for speeds in [config.speeds, Speeds::default(), Config::parse("").unwrap().speeds] {
            assert_eq!(speeds.normal, 300.0);
            assert_eq!(speeds.slow, 100.0);
            assert_eq!(speeds.fast, 900.0);
            assert_eq!(speeds.scroll, 6.0);
            assert_eq!(speeds.scroll_slow, 1.5);
            assert_eq!(speeds.scroll_fast, 24.0);
        }
    }

    #[test]
    fn keycast_configuration_and_validation() {
        let config = Config::parse(
            "[keycast]\nenabled = true\nhotkey = 'F8'\nmax_keys = 6\ntimeout = 2.5",
        ).unwrap();
        assert!(config.keycast.enabled);
        assert_eq!(config.keycast.hotkey, K::KEY_F8);
        assert_eq!(config.keycast.max_keys, 6);
        assert_eq!(config.keycast.timeout, 2.5);
        assert!(config.mouse_controls().contains(&K::KEY_F8));
        assert!(!Config::default().mouse_controls().contains(&K::KEY_ESC));
        for text in [
            "enabled = 'true'", "hotkey = 'not-a-key'", "hotkey = 'a+b'",
            "max_keys = 0", "max_keys = 33", "max_keys = -1",
            "timeout = 0", "timeout = -1", "timeout = 61",
            "timeout = nan", "timeout = inf", "timout = 3",
            "enabled = true\nhotkey = 'h'", "enabled = true\nhotkey = 'f3'",
        ] {
            assert!(Config::parse(&format!("[keycast]\n{text}")).is_err(), "{text}");
        }
        assert!(Config::parse(
            "[keys]\nfree_mouse = 'leftalt+space'\n[keycast]\nenabled = true\nhotkey = 'leftalt'"
        ).is_err());
    }

    #[test]
    fn easing_defaults_overrides_and_validation() {
        let default = Config::parse("").unwrap();
        assert_eq!(default.easing.movement, 0.2);
        assert_eq!(default.easing.scroll, 0.3);
        let config = Config::parse("[easing]\nmovement = 0").unwrap();
        assert_eq!(config.easing.movement, 0.0);
        assert_eq!(config.easing.scroll, 0.3);
        let config = Config::parse("[easing]\nmovement = 1\nscroll = 0.5").unwrap();
        assert_eq!(config.easing.movement, 1.0);
        assert_eq!(config.easing.scroll, 0.5);
        for field in ["movement", "scroll"] {
            for value in ["-0.1", "1.01", "nan", "inf", "-inf", "'0.2'", "true"] {
                assert!(Config::parse(&format!("[easing]\n{field} = {value}")).is_err());
            }
        }
        assert!(Config::parse("[easing]\nmovment = 0.2").is_err());
    }

    #[test]
    fn activation_mode_defaults_to_hold_and_accepts_only_hold_or_toggle() {
        assert_eq!(Config::parse("").unwrap().mode, Mode::Hold);
        assert_eq!(Config::parse("mode = 'hold'").unwrap().mode, Mode::Hold);
        assert_eq!(Config::parse("mode = 'toggle'").unwrap().mode, Mode::Toggle);
        for text in ["mode = 'tap'", "mode = true", "mode = 1"] {
            assert!(Config::parse(text).is_err());
        }
    }

    #[test]
    fn indicator_defaults_on_and_accepts_only_booleans() {
        assert!(Config::default().notify);
        assert!(Config::parse("").unwrap().notify);
        assert!(Config::parse("mode = 'toggle'").unwrap().notify);
        assert!(Config::parse("notify = true").unwrap().notify);
        assert!(!Config::parse("notify = false").unwrap().notify);
        for text in ["notify = 'true'", "notify = 1", "notify = []"] {
            assert!(Config::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn home_end_defaults_on_and_accepts_only_booleans() {
        assert!(Config::default().home_end_enabled);
        assert!(Config::parse("").unwrap().home_end_enabled);
        assert!(Config::parse("home_end_enabled = true").unwrap().home_end_enabled);
        assert!(!Config::parse("home_end_enabled = false").unwrap().home_end_enabled);
        for value in ["'true'", "1", "[]"] {
            assert!(Config::parse(&format!("home_end_enabled = {value}")).is_err());
        }
    }

    #[test]
    fn partial_configs_keep_unspecified_defaults() {
        let config = Config::parse(
            "[speeds]\nnormal = 900\n[keys]\nfree_mouse = 'KEY_F4'\nleft = 'q'",
        )
        .unwrap();
        assert_eq!(config.speeds.normal, 900.0);
        assert_eq!(config.speeds.fast, 900.0);
        assert_eq!(config.keys.free_mouse, vec![K::KEY_F4]);
        assert_eq!(config.keys.left, K::KEY_Q);
        assert_eq!(config.keys.scroll_up, K::KEY_COMMA);
        Config::parse("").unwrap();
    }

    #[test]
    fn accepts_punctuation_and_case_insensitive_names() {
        let config = Config::parse(
            "[keys]\nscroll_up = ','\nscroll_right = '.'\nfast = 'key_s'",
        )
        .unwrap();
        assert_eq!(config.keys.named(), Keys::default().named());
        assert_eq!(parse_key("Escape").unwrap(), K::KEY_ESC);
    }

    #[test]
    fn activation_chords_accept_spaces_case_and_overlapping_controls() {
        let config = Config::parse("[keys]\nfree_mouse = ' LeftAlt + KEY_SPACE '").unwrap();
        assert_eq!(config.keys.free_mouse, vec![K::KEY_LEFTALT, K::KEY_SPACE]);
        assert_eq!(config.keys.left_click, K::KEY_SPACE);
        let config = Config::parse("[keys]\nfree_mouse = 'leftctrl+leftalt+f4'").unwrap();
        assert_eq!(config.keys.free_mouse.len(), 3);
    }

    #[test]
    fn rejects_invalid_speeds_bindings_and_typos() {
        for text in [
            "[speeds]\nnormal = 0",
            "[speeds]\nslow = -1",
            "[speeds]\nfast = nan",
            "[speeds]\nscroll = inf",
            "[speeds]\nscroll_slow = 0",
            "[speeds]\nscroll_fast = nan",
            "[speeds]\nscroll_fast = 100001",
            "[speeds]\nfast = 100001",
            "[keys]\nleft = 'not-a-key'",
            "[keys]\nleft = 'BTN_LEFT'",
            "[keys]\nleft = 'KEY_RESERVED'",
            "[keys]\nleft = 'j'",
            "[keys]\nslow = 'f3'",
            "[keys]\nexit = 's'",
            "[keys]\nexit = 'esc'",
            "[keys]\nfree_mouse = ''",
            "[keys]\nfree_mouse = 'leftalt+'",
            "[keys]\nfree_mouse = '+space'",
            "[keys]\nfree_mouse = 'leftalt++space'",
            "[keys]\nfree_mouse = 'space+KEY_SPACE'",
            "[keys]\nfree_mouse = 'leftalt+not-a-key'",
            "[keys]\nleft = 'leftalt+h'",
            "[keys]\nfree_mosu = 'f4'",
            "[speeds]\nnromal = 900",
            "[unknown]\nnormal = 900",
            "[keys]\nleft = 35",
            "invalid toml",
        ] {
            assert!(Config::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn missing_optional_file_defaults_but_required_file_errors() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/nonexistent-config-directory/fievel.config");
        assert_eq!(Config::load(&path, false).unwrap().keys.free_mouse, vec![K::KEY_F3]);
        assert!(Config::load(&path, true).is_err());
        assert!(Config::load(Path::new(env!("CARGO_MANIFEST_DIR")), false).is_err());
    }
}
