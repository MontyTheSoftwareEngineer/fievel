use crate::font::Color;
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
    pub hints: Hints,
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
            hints: Hints::default(),
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
            .map(|key| ("keys.free_mouse", *key))
            .chain(self.named_controls())
            .collect()
    }

    fn named_controls(&self) -> [(&'static str, K); 12] {
        [
            ("keys.left", self.left),
            ("keys.down", self.down),
            ("keys.up", self.up),
            ("keys.right", self.right),
            ("keys.left_click", self.left_click),
            ("keys.right_click", self.right_click),
            ("keys.scroll_left", self.scroll_left),
            ("keys.scroll_down", self.scroll_down),
            ("keys.scroll_up", self.scroll_up),
            ("keys.scroll_right", self.scroll_right),
            ("keys.slow", self.slow),
            ("keys.fast", self.fast),
        ]
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Hints {
    pub keys: HintKeys,
    pub label_symbols: String,
    #[serde(deserialize_with = "deserialize_color")]
    pub border_color: Color,
    #[serde(deserialize_with = "deserialize_color")]
    pub fill_color: Color,
    #[serde(deserialize_with = "deserialize_color")]
    pub readability_color: Color,
    #[serde(deserialize_with = "deserialize_color")]
    pub label_color: Color,
    #[serde(deserialize_with = "deserialize_color")]
    pub label_highlight_color: Color,
    pub min_width: u32,
    pub max_width: u32,
    pub min_height: u32,
    pub max_height: u32,
}

impl Default for Hints {
    fn default() -> Self {
        Self {
            keys: HintKeys::default(),
            label_symbols: "abcdefghijklmnopqrstuvwxyz".to_owned(),
            border_color: Color::rgba(0x00, 0xff, 0x00, 0xe0),
            fill_color: Color::rgba(0x00, 0xff, 0x00, 0x18),
            readability_color: Color::rgba(0x40, 0x40, 0x40, 0xe6),
            label_color: Color::rgba(0xff, 0xff, 0xff, 0xff),
            label_highlight_color: Color::rgba(0xff, 0xc1, 0x07, 0xff),
            min_width: 8,
            max_width: 499,
            min_height: 4,
            max_height: 49,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct HintKeys {
    #[serde(deserialize_with = "deserialize_chord")]
    pub left: Vec<K>,
    #[serde(deserialize_with = "deserialize_chord")]
    pub right: Vec<K>,
    #[serde(deserialize_with = "deserialize_key")]
    pub cancel: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub toggle_background: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub debug: K,
}

impl Default for HintKeys {
    fn default() -> Self {
        Self {
            left: vec![K::KEY_LEFTMETA, K::KEY_SPACE],
            right: vec![K::KEY_LEFTMETA, K::KEY_I],
            cancel: K::KEY_ENTER,
            toggle_background: K::KEY_LEFTCTRL,
            debug: K::KEY_F8,
        }
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

fn deserialize_color<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Color, D::Error> {
    let value = String::deserialize(deserializer)?;
    Color::parse(&value).map_err(serde::de::Error::custom)
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
        validate_chord("keys.free_mouse", &self.keys.free_mouse)?;
        validate_chord("hints.keys.left", &self.hints.keys.left)?;
        validate_chord("hints.keys.right", &self.hints.keys.right)?;
        validate_hint_symbols(&self.hints.label_symbols)?;
        let toggle = self.hints.keys.toggle_background;
        if toggle == self.hints.keys.cancel
            || toggle == K::KEY_ESC
            || toggle == K::KEY_BACKSPACE
            || self
                .hints
                .label_symbols
                .chars()
                .any(|symbol| parse_key(&symbol.to_string()) == Ok(toggle))
        {
            return Err(
                "hints.keys.toggle_background conflicts with hint labels or cancellation/editing keys"
                    .to_owned(),
            );
        }
        validate_limits(self.hints.min_width, self.hints.max_width, "width")?;
        let debug = self.hints.keys.debug;
        if [K::KEY_RESERVED, K::KEY_UNKNOWN, K::KEY_ESC, K::KEY_BACKSPACE,
            self.hints.keys.cancel, toggle].contains(&debug)
            || self.hints.keys.left.contains(&debug)
            || self.hints.keys.right.contains(&debug)
            || self.keys.free_mouse.contains(&debug)
            || self.hints.label_symbols.chars()
                .any(|symbol| parse_key(&symbol.to_string()) == Ok(debug))
        {
            return Err("hints.keys.debug conflicts with reserved, label, cancel, peek, or activation keys".to_owned());
        }
        validate_limits(self.hints.min_height, self.hints.max_height, "height")?;
        let mut assigned = BTreeMap::new();
        for (name, key) in self.keys.named() {
            if name == "keys.free_mouse" && self.keys.free_mouse.len() > 1 {
                continue;
            }
            if let Some(other) = assigned.insert(key, name) {
                return Err(format!("{name} and {other} both bind {key:?}"));
            }
        }
        for (name, chord) in [
            ("hints.keys.left", &self.hints.keys.left),
            ("hints.keys.right", &self.hints.keys.right),
        ] {
            if chord.len() == 1 {
                let key = chord[0];
                if let Some(other) = assigned.insert(key, name) {
                    return Err(format!("{name} and {other} both bind {key:?}"));
                }
            }
        }
        if let Some(other) = assigned.insert(self.hints.keys.cancel, "hints.keys.cancel") {
            return Err(format!(
                "hints.keys.cancel and {other} both bind {:?}",
                self.hints.keys.cancel
            ));
        }
        validate_unique_chords(
            "keys.free_mouse",
            &self.keys.free_mouse,
            "hints.keys.left",
            &self.hints.keys.left,
        )?;
        validate_unique_chords(
            "keys.free_mouse",
            &self.keys.free_mouse,
            "hints.keys.right",
            &self.hints.keys.right,
        )?;
        validate_unique_chords(
            "hints.keys.left",
            &self.hints.keys.left,
            "hints.keys.right",
            &self.hints.keys.right,
        )?;
        Ok(())
    }
}

fn validate_chord(name: &str, chord: &[K]) -> Result<(), String> {
    let mut keys = std::collections::BTreeSet::new();
    if chord.is_empty() {
        return Err(format!("{name} must contain at least one key"));
    }
    for key in chord {
        if !keys.insert(key) {
            return Err(format!("{name} repeats {key:?}"));
        }
    }
    Ok(())
}

fn validate_unique_chords(
    first_name: &str,
    first: &[K],
    second_name: &str,
    second: &[K],
) -> Result<(), String> {
    if first == second {
        return Err(format!(
            "{first_name} and {second_name} both bind the same chord"
        ));
    }
    Ok(())
}

fn validate_hint_symbols(symbols: &str) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    let chars: Vec<char> = symbols.chars().collect();
    if chars.len() < 2 || chars.len() > 26 {
        return Err("hints.label_symbols: must contain 2-26 unique ASCII letters".to_owned());
    }
    for symbol in chars {
        if !symbol.is_ascii_lowercase() || !seen.insert(symbol) {
            return Err("hints.label_symbols: must contain 2-26 unique ASCII letters".to_owned());
        }
    }
    Ok(())
}

fn validate_limits(min: u32, max: u32, axis: &str) -> Result<(), String> {
    if min == 0 {
        return Err(format!("hints.min_{axis}: must be greater than zero"));
    }
    if max < min {
        return Err(format!(
            "hints.max_{axis}: must be at least hints.min_{axis}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_key_defaults_customization_and_conflicts() {
        assert_eq!(Config::parse("").unwrap().hints.keys.debug, K::KEY_F8);
        assert_eq!(Config::parse("[hints.keys]\ndebug = 'f9'").unwrap().hints.keys.debug, K::KEY_F9);
        for key in ["reserved", "unknown", "not-a-key", "esc", "enter", "backspace",
            "leftctrl", "leftmeta", "space", "i", "z", "f3", "f8+f9"] {
            assert!(Config::parse(&format!("[hints.keys]\ndebug = '{key}'")).is_err(), "{key}");
        }
        for text in [
            "[hints.keys]\ndebug = 'f9'\ncancel = 'f9'",
            "[hints.keys]\ndebug = 'f9'\ntoggle_background = 'f9'",
            "[hints.keys]\ndebug = 'f9'\nleft = 'f9'",
            "[hints.keys]\ndebug = 'f9'\nright = 'leftmeta+f9'",
            "[hints.keys]\ndebugg = 'f9'",
        ] {
            assert!(Config::parse(text).is_err(), "{text}");
        }
        assert_eq!(Config::parse(include_str!("../fievel.config")).unwrap().hints.keys.debug, K::KEY_F8);
    }

    #[test]
    fn example_matches_defaults() {
        let config = Config::parse(include_str!("../fievel.config")).unwrap();
        let default = Config::default();
        assert_eq!(config.mode, Mode::Hold);
        assert!(config.notify);
        assert!(config.home_end_enabled);
        assert!(!config.keycast.enabled);
        assert_eq!(config.keycast.hotkey, K::KEY_ESC);
        assert_eq!(config.keycast.max_keys, 8);
        assert_eq!(config.keycast.timeout, 3.0);
        assert_eq!(config.keys.named(), default.keys.named());
        assert_eq!(config.hints, default.hints);
        assert_eq!(config.easing.movement, 0.2);
        assert_eq!(config.easing.scroll, 0.3);
        for speeds in [
            config.speeds,
            default.speeds,
            Config::parse("").unwrap().speeds,
        ] {
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
        assert!(
            Config::parse("home_end_enabled = true")
                .unwrap()
                .home_end_enabled
        );
        assert!(
            !Config::parse("home_end_enabled = false")
                .unwrap()
                .home_end_enabled
        );
        for value in ["'true'", "1", "[]"] {
            assert!(Config::parse(&format!("home_end_enabled = {value}")).is_err());
        }
    }

    #[test]
    fn partial_configs_keep_unspecified_defaults() {
        let config = Config::parse(
            "[speeds]\nnormal = 900\n[keys]\nfree_mouse = 'KEY_F4'\nleft = 'q'\n[hints.keys]\nleft = 'leftmeta+j'",
        )
        .unwrap();
        assert_eq!(config.speeds.normal, 900.0);
        assert_eq!(config.speeds.fast, 900.0);
        assert_eq!(config.keys.free_mouse, vec![K::KEY_F4]);
        assert_eq!(config.keys.left, K::KEY_Q);
        assert_eq!(config.keys.scroll_up, K::KEY_COMMA);
        assert_eq!(config.hints.keys.left, vec![K::KEY_LEFTMETA, K::KEY_J]);
        Config::parse("").unwrap();
    }

    #[test]
    fn accepts_punctuation_case_and_colors() {
        let config = Config::parse(
            "[keys]\nscroll_up = ','\nscroll_right = '.'\nfast = 'key_s'\n[hints]\nborder_color = '#00ff00e0'",
        )
        .unwrap();
        assert_eq!(config.keys.named(), Keys::default().named());
        assert_eq!(parse_key("Escape").unwrap(), K::KEY_ESC);
        assert_eq!(config.hints.border_color, Hints::default().border_color);
    }

    #[test]
    fn activation_chords_accept_spaces_case_and_overlapping_controls() {
        let config = Config::parse(
            "[keys]\nfree_mouse = ' LeftAlt + KEY_SPACE '\n[hints.keys]\nleft = ' LeftMeta + space '\nright = 'leftmeta+KEY_I'",
        )
        .unwrap();
        assert_eq!(config.keys.free_mouse, vec![K::KEY_LEFTALT, K::KEY_SPACE]);
        assert_eq!(config.keys.left_click, K::KEY_SPACE);
        assert_eq!(config.hints.keys.left, vec![K::KEY_LEFTMETA, K::KEY_SPACE]);
        let config = Config::parse("[keys]\nfree_mouse = 'leftctrl+leftalt+f4'").unwrap();
        assert_eq!(config.keys.free_mouse.len(), 3);
    }

    #[test]
    fn hint_defaults_and_validation_cover_symbols_sizes_and_conflicts() {
        let config = Config::parse("").unwrap();
        assert_eq!(config.hints.keys.left, vec![K::KEY_LEFTMETA, K::KEY_SPACE]);
        assert_eq!(config.hints.keys.right, vec![K::KEY_LEFTMETA, K::KEY_I]);
        assert_eq!(config.hints.keys.cancel, K::KEY_ENTER);
        assert_eq!(config.hints.keys.toggle_background, K::KEY_LEFTCTRL);
        assert_eq!(config.hints.keys.debug, K::KEY_F8);
        assert_eq!(config.hints.readability_color, Color::rgba(64, 64, 64, 230));
        assert_eq!(config.hints.label_symbols, "abcdefghijklmnopqrstuvwxyz");
        for text in [
            "[hints]\nlabel_symbols = 'a'",
            "[hints]\nlabel_symbols = 'abcA'",
            "[hints]\nlabel_symbols = 'abca'",
            "[hints]\nmin_width = 0",
            "[hints]\nmax_width = 1\nmin_width = 2",
            "[hints]\nmax_height = 3\nmin_height = 4",
            "[hints.keys]\nleft = ''",
            "[hints.keys]\nleft = 'leftmeta+leftmeta'",
            "[hints.keys]\nright = 'leftmeta+not-a-key'",
            "[hints.keys]\ncancel = 'space'",
            "[hints.keys]\ntoggle_background = 'enter'",
            "[hints.keys]\ntoggle_background = 'escape'",
            "[hints.keys]\ntoggle_background = 'backspace'",
            "[hints.keys]\ntoggle_background = 'z'",
            "[hints.keys]\ntoggle_background = 'leftctrl+space'",
            "[hints.keys]\ntoggle_background = 'not-a-key'",
            "[hints]\nreadability_color = 'grey'",
            "[hints.keys]\nleft = 'j'",
            "[hints.keys]\nleft = 'leftmeta+space'\nright = 'leftmeta+space'",
            "[hints]\ncolor = '#00ff00'",
        ] {
            assert!(Config::parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn hint_background_configuration_and_example_defaults() {
        let example = Config::parse(include_str!("../fievel.config")).unwrap();
        let defaults = Config::default();
        assert_eq!(
            example.hints.keys.toggle_background,
            defaults.hints.keys.toggle_background,
        );
        assert_eq!(
            example.hints.readability_color,
            defaults.hints.readability_color,
        );
        let custom = Config::parse(
            "[hints]\nreadability_color = '#606060cc'\n[hints.keys]\ntoggle_background = 'rightctrl'",
        )
        .unwrap();
        assert_eq!(custom.hints.keys.toggle_background, K::KEY_RIGHTCTRL);
        assert_eq!(custom.hints.readability_color, Color::rgba(96, 96, 96, 204));
        assert!(Config::parse(
            "[hints.keys]\ncancel = 'rightctrl'\ntoggle_background = 'rightctrl'"
        )
        .is_err());
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
        assert_eq!(
            Config::load(&path, false).unwrap().keys.free_mouse,
            vec![K::KEY_F3]
        );
        assert!(Config::load(&path, true).is_err());
        assert!(Config::load(Path::new(env!("CARGO_MANIFEST_DIR")), false).is_err());
    }
}
