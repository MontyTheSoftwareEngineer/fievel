use evdev::KeyCode as K;
use serde::{Deserialize, Deserializer};
use std::{collections::BTreeMap, error::Error, fs, io, path::Path};

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub mode: Mode,
    pub notify: bool,
    pub speeds: Speeds,
    pub keys: Keys,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::default(),
            notify: true,
            speeds: Speeds::default(),
            keys: Keys::default(),
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
}

impl Default for Speeds {
    fn default() -> Self {
        Self {
            normal: 800.0,
            slow: 200.0,
            fast: 1600.0,
            scroll: 8.0,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keys {
    #[serde(deserialize_with = "deserialize_key")]
    pub free_mouse: K,
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
    #[serde(deserialize_with = "deserialize_key")]
    pub exit: K,
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            free_mouse: K::KEY_F3,
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
            exit: K::KEY_ESC,
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

    pub fn named(&self) -> [(&'static str, K); 14] {
        [
            ("free_mouse", self.free_mouse),
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
            ("exit", self.exit),
        ]
    }
}

fn deserialize_key<'de, D: Deserializer<'de>>(deserializer: D) -> Result<K, D::Error> {
    let value = String::deserialize(deserializer)?;
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
        .map_err(|_| serde::de::Error::custom(format!("unknown keyboard key {value:?}")))?;
    if key == K::KEY_RESERVED || key == K::KEY_UNKNOWN {
        return Err(serde::de::Error::custom("reserved/unknown keys cannot be bound"));
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
        for (name, value) in [
            ("normal", self.speeds.normal),
            ("slow", self.speeds.slow),
            ("fast", self.speeds.fast),
            ("scroll", self.speeds.scroll),
        ] {
            validate_speed(value).map_err(|error| format!("speeds.{name}: {error}"))?;
        }
        let mut assigned = BTreeMap::new();
        for (name, key) in self.keys.named() {
            if let Some(other) = assigned.insert(key, name) {
                return Err(format!("keys.{name} and keys.{other} both bind {key:?}"));
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
        assert_eq!(config.keys.named(), Keys::default().named());
        assert_eq!(config.speeds.normal, 800.0);
        assert_eq!(config.speeds.slow, 200.0);
        assert_eq!(config.speeds.fast, 1600.0);
        assert_eq!(config.speeds.scroll, 8.0);
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
    fn partial_configs_keep_unspecified_defaults() {
        let config = Config::parse(
            "[speeds]\nnormal = 900\n[keys]\nfree_mouse = 'KEY_F4'\nleft = 'q'",
        )
        .unwrap();
        assert_eq!(config.speeds.normal, 900.0);
        assert_eq!(config.speeds.fast, 1600.0);
        assert_eq!(config.keys.free_mouse, K::KEY_F4);
        assert_eq!(config.keys.left, K::KEY_Q);
        assert_eq!(config.keys.scroll_up, K::KEY_COMMA);
        Config::parse("").unwrap();
    }

    #[test]
    fn accepts_punctuation_and_case_insensitive_names() {
        let config = Config::parse(
            "[keys]\nscroll_up = ','\nscroll_right = '.'\nexit = 'Escape'\nfast = 'key_s'",
        )
        .unwrap();
        assert_eq!(config.keys.named(), Keys::default().named());
    }

    #[test]
    fn rejects_invalid_speeds_bindings_and_typos() {
        for text in [
            "[speeds]\nnormal = 0",
            "[speeds]\nslow = -1",
            "[speeds]\nfast = nan",
            "[speeds]\nscroll = inf",
            "[speeds]\nfast = 100001",
            "[keys]\nleft = 'not-a-key'",
            "[keys]\nleft = 'BTN_LEFT'",
            "[keys]\nleft = 'KEY_RESERVED'",
            "[keys]\nleft = 'j'",
            "[keys]\nslow = 'f3'",
            "[keys]\nexit = 's'",
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
        assert_eq!(Config::load(&path, false).unwrap().keys.free_mouse, K::KEY_F3);
        assert!(Config::load(&path, true).is_err());
        assert!(Config::load(Path::new(env!("CARGO_MANIFEST_DIR")), false).is_err());
    }
}
