use crate::font::Color;
use crate::grid::GridQuadrant;
use crate::input::KeyCode as K;
use serde::{de::DeserializeOwned, Deserialize, Deserializer};
use std::{
    collections::{BTreeMap, HashSet},
    error::Error,
    fs, io,
    io::Write,
    path::Path,
};

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

fn is_letter_key(key: K) -> bool {
    [
        K::KEY_A,
        K::KEY_B,
        K::KEY_C,
        K::KEY_D,
        K::KEY_E,
        K::KEY_F,
        K::KEY_G,
        K::KEY_H,
        K::KEY_I,
        K::KEY_J,
        K::KEY_K,
        K::KEY_L,
        K::KEY_M,
        K::KEY_N,
        K::KEY_O,
        K::KEY_P,
        K::KEY_Q,
        K::KEY_R,
        K::KEY_S,
        K::KEY_T,
        K::KEY_U,
        K::KEY_V,
        K::KEY_W,
        K::KEY_X,
        K::KEY_Y,
        K::KEY_Z,
    ]
    .contains(&key)
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
    /// Easing while the pointer slows down or stops; unset uses `movement`.
    pub movement_stop: Option<f64>,
    pub scroll: f64,
}

impl Default for Easing {
    fn default() -> Self {
        Self {
            movement: 0.2,
            movement_stop: None,
            scroll: 0.3,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Keys {
    #[serde(deserialize_with = "deserialize_chord")]
    pub free_mouse: Vec<K>,
    #[serde(deserialize_with = "deserialize_chord")]
    pub reload_config: Vec<K>,
    #[serde(deserialize_with = "deserialize_chord")]
    pub config_screen: Vec<K>,
    #[serde(deserialize_with = "deserialize_key")]
    pub toggle_hold: K,
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
    pub locate: K,
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            free_mouse: vec![K::KEY_F3],
            reload_config: vec![K::KEY_LEFTMETA, K::KEY_LEFTSHIFT, K::KEY_R],
            config_screen: vec![K::KEY_LEFTMETA, K::KEY_LEFTSHIFT, K::KEY_SLASH],
            toggle_hold: K::KEY_Z,
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
            locate: K::KEY_Q,
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
            .chain([("keys.toggle_hold", self.toggle_hold)])
            .chain(self.named_controls())
            .collect()
    }

    fn named_controls(&self) -> [(&'static str, K); 13] {
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
            ("keys.locate", self.locate),
        ]
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Hints {
    pub recursive_grid_enabled: bool,
    pub keys: HintKeys,
    pub label_symbols: String,
    #[serde(deserialize_with = "deserialize_color")]
    pub border_color: Color,
    #[serde(deserialize_with = "deserialize_color")]
    pub fill_color: Color,
    pub grid_colors: GridColors,
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
            recursive_grid_enabled: false,
            keys: HintKeys::default(),
            label_symbols: "abcdefghijklmnopqrstuvwxyz".to_owned(),
            border_color: Color::rgba(0x00, 0xff, 0x00, 0xe0),
            fill_color: Color::rgba(0x00, 0xff, 0x00, 0x18),
            grid_colors: GridColors::default(),
            readability_color: Color::rgba(0x40, 0x40, 0x40, 0xe6),
            label_color: Color::rgba(0xff, 0xff, 0xff, 0xff),
            label_highlight_color: Color::rgba(0xff, 0xc1, 0x07, 0xff),
            min_width: 8,
            max_width: 499,
            min_height: 4,
            max_height: 96,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct GridColors {
    #[serde(deserialize_with = "deserialize_optional_color")]
    pub upper_left: Option<Color>,
    #[serde(deserialize_with = "deserialize_optional_color")]
    pub upper_right: Option<Color>,
    #[serde(deserialize_with = "deserialize_optional_color")]
    pub lower_left: Option<Color>,
    #[serde(deserialize_with = "deserialize_optional_color")]
    pub lower_right: Option<Color>,
}

impl GridColors {
    fn for_quadrant(&self, quadrant: GridQuadrant) -> Option<Color> {
        match quadrant {
            GridQuadrant::UpperLeft => self.upper_left,
            GridQuadrant::UpperRight => self.upper_right,
            GridQuadrant::LowerLeft => self.lower_left,
            GridQuadrant::LowerRight => self.lower_right,
        }
    }
}

impl Hints {
    pub fn grid_border_color(&self, quadrant: GridQuadrant) -> Color {
        self.grid_colors
            .for_quadrant(quadrant)
            .unwrap_or(self.border_color)
    }

    pub fn grid_fill_color(&self, quadrant: GridQuadrant) -> Color {
        match self.grid_colors.for_quadrant(quadrant) {
            Some(color) => Color::rgba(color.r, color.g, color.b, self.fill_color.a),
            None => self.fill_color,
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
    #[serde(deserialize_with = "deserialize_chord")]
    pub grid: Vec<K>,
    #[serde(deserialize_with = "deserialize_key")]
    pub grid_upper_left: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub grid_upper_right: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub grid_lower_left: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub grid_lower_right: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub grid_left_click: K,
    #[serde(deserialize_with = "deserialize_key")]
    pub grid_right_click: K,
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
            grid: vec![K::KEY_LEFTMETA, K::KEY_G],
            grid_upper_left: K::KEY_U,
            grid_upper_right: K::KEY_I,
            grid_lower_left: K::KEY_J,
            grid_lower_right: K::KEY_K,
            grid_left_click: K::KEY_SPACE,
            grid_right_click: K::KEY_R,
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

fn deserialize_optional_color<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Color>, D::Error> {
    Option::<String>::deserialize(deserializer)?
        .map(|value| Color::parse(&value).map_err(serde::de::Error::custom))
        .transpose()
}

type TomlTable = toml::map::Map<String, toml::Value>;

fn toml_table(entries: impl IntoIterator<Item = (&'static str, toml::Value)>) -> toml::Value {
    toml::Value::Table(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn toml_key(key: K) -> toml::Value {
    toml::Value::String(format!("{key:?}"))
}

fn toml_chord(chord: &[K]) -> toml::Value {
    toml::Value::String(
        chord
            .iter()
            .map(|key| format!("{key:?}"))
            .collect::<Vec<_>>()
            .join(" + "),
    )
}

fn toml_color(color: Color) -> toml::Value {
    toml::Value::String(format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        color.r, color.g, color.b, color.a
    ))
}

fn merge_toml_values(base: &mut toml::Value, overlay: toml::Value) {
    match (base, overlay) {
        (toml::Value::Table(base), toml::Value::Table(overlay)) => {
            for (key, value) in overlay {
                if let Some(existing) = base.get_mut(&key) {
                    merge_toml_values(existing, value);
                } else {
                    base.insert(key, value);
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

fn parse_toml_value<T: DeserializeOwned>(value: &toml::Value) -> Result<T, String> {
    value
        .clone()
        .try_into()
        .map_err(|error: toml::de::Error| error.to_string())
}

fn warn_unknown_fields(table: &TomlTable, known: &[&str], prefix: &str) {
    for key in table.keys() {
        if !known.contains(&key.as_str()) {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            eprintln!("Warning: unknown config key {path}; skipping it");
        }
    }
}

fn warn_invalid_config_value(path: &str, error: &str) {
    let prefix = format!("{path}: ");
    let reason = error.strip_prefix(&prefix).unwrap_or(error);
    eprintln!("Warning: invalid config value {path}: {reason}; skipping it");
}

fn config_section<'a>(table: &'a TomlTable, key: &str, path: &str) -> Option<&'a TomlTable> {
    match table.get(key) {
        Some(value) => match value.as_table() {
            Some(table) => Some(table),
            None => {
                eprintln!("Warning: invalid config value {path}: expected a table; skipping it");
                None
            }
        },
        None => None,
    }
}

fn apply_value<T: DeserializeOwned>(
    table: &TomlTable,
    key: &str,
    path: &str,
    target: &mut T,
    changed: &mut HashSet<String>,
) {
    apply_parsed(table, key, path, target, changed, parse_toml_value);
}

fn apply_parsed<T>(
    table: &TomlTable,
    key: &str,
    path: &str,
    target: &mut T,
    changed: &mut HashSet<String>,
    parse: impl FnOnce(&toml::Value) -> Result<T, String>,
) {
    let Some(value) = table.get(key) else {
        return;
    };
    match parse(value) {
        Ok(parsed) => {
            *target = parsed;
            changed.insert(path.to_owned());
        }
        Err(error) => {
            warn_invalid_config_value(path, &error);
        }
    }
}

fn apply_key(
    table: &TomlTable,
    key: &str,
    path: &str,
    target: &mut K,
    changed: &mut HashSet<String>,
) {
    apply_parsed(table, key, path, target, changed, |value| {
        parse_toml_value::<String>(value).and_then(|value| parse_key(&value))
    });
}

fn apply_chord(
    table: &TomlTable,
    key: &str,
    path: &str,
    target: &mut Vec<K>,
    changed: &mut HashSet<String>,
) {
    apply_parsed(table, key, path, target, changed, |value| {
        let value = parse_toml_value::<String>(value)?;
        let chord = value
            .split('+')
            .map(parse_key)
            .collect::<Result<Vec<_>, _>>()?;
        validate_chord(path, &chord)?;
        Ok(chord)
    });
}

fn apply_color(
    table: &TomlTable,
    key: &str,
    path: &str,
    target: &mut Color,
    changed: &mut HashSet<String>,
) {
    apply_parsed(table, key, path, target, changed, |value| {
        parse_toml_value::<String>(value).and_then(|value| Color::parse(&value))
    });
}

fn apply_optional_color(
    table: &TomlTable,
    key: &str,
    path: &str,
    target: &mut Option<Color>,
    changed: &mut HashSet<String>,
) {
    apply_parsed(table, key, path, target, changed, |value| {
        parse_toml_value::<String>(value)
            .and_then(|value| Color::parse(&value))
            .map(Some)
    });
}

fn remap_with_entry(
    remap: &crate::remap::RemapConfig,
    layer: Option<&str>,
    trigger: &str,
    action: &str,
) -> crate::remap::RemapConfig {
    let mut candidate = remap.clone();
    if let Some(layer) = layer {
        if let Some(entries) = candidate.layers.get_mut(layer) {
            entries.insert(trigger.to_owned(), action.to_owned());
        }
    } else {
        candidate.main.insert(trigger.to_owned(), action.to_owned());
    }
    candidate
}

fn reset_config_value(config: &mut Config, path: &str) -> bool {
    let defaults = Config::default();
    match path {
        "mode" => config.mode = defaults.mode,
        "notify" => config.notify = defaults.notify,
        "home_end_enabled" => config.home_end_enabled = defaults.home_end_enabled,
        "keycast.enabled" => config.keycast.enabled = defaults.keycast.enabled,
        "keycast.hotkey" => config.keycast.hotkey = defaults.keycast.hotkey,
        "keycast.max_keys" => config.keycast.max_keys = defaults.keycast.max_keys,
        "keycast.timeout" => config.keycast.timeout = defaults.keycast.timeout,
        "speeds.normal" => config.speeds.normal = defaults.speeds.normal,
        "speeds.slow" => config.speeds.slow = defaults.speeds.slow,
        "speeds.fast" => config.speeds.fast = defaults.speeds.fast,
        "speeds.scroll" => config.speeds.scroll = defaults.speeds.scroll,
        "speeds.scroll_slow" => config.speeds.scroll_slow = defaults.speeds.scroll_slow,
        "speeds.scroll_fast" => config.speeds.scroll_fast = defaults.speeds.scroll_fast,
        "easing.movement" => config.easing.movement = defaults.easing.movement,
        "easing.movement_stop" => config.easing.movement_stop = defaults.easing.movement_stop,
        "easing.scroll" => config.easing.scroll = defaults.easing.scroll,
        "keys.free_mouse" => config.keys.free_mouse = defaults.keys.free_mouse,
        "keys.reload_config" => config.keys.reload_config = defaults.keys.reload_config,
        "keys.config_screen" => config.keys.config_screen = defaults.keys.config_screen,
        "keys.toggle_hold" => config.keys.toggle_hold = defaults.keys.toggle_hold,
        "keys.left" => config.keys.left = defaults.keys.left,
        "keys.down" => config.keys.down = defaults.keys.down,
        "keys.up" => config.keys.up = defaults.keys.up,
        "keys.right" => config.keys.right = defaults.keys.right,
        "keys.left_click" => config.keys.left_click = defaults.keys.left_click,
        "keys.right_click" => config.keys.right_click = defaults.keys.right_click,
        "keys.scroll_left" => config.keys.scroll_left = defaults.keys.scroll_left,
        "keys.scroll_down" => config.keys.scroll_down = defaults.keys.scroll_down,
        "keys.scroll_up" => config.keys.scroll_up = defaults.keys.scroll_up,
        "keys.scroll_right" => config.keys.scroll_right = defaults.keys.scroll_right,
        "keys.slow" => config.keys.slow = defaults.keys.slow,
        "keys.fast" => config.keys.fast = defaults.keys.fast,
        "keys.locate" => config.keys.locate = defaults.keys.locate,
        "hints.keys.left" => config.hints.keys.left = defaults.hints.keys.left,
        "hints.keys.right" => config.hints.keys.right = defaults.hints.keys.right,
        "hints.keys.grid" => config.hints.keys.grid = defaults.hints.keys.grid,
        "hints.keys.grid_upper_left" => {
            config.hints.keys.grid_upper_left = defaults.hints.keys.grid_upper_left
        }
        "hints.keys.grid_upper_right" => {
            config.hints.keys.grid_upper_right = defaults.hints.keys.grid_upper_right
        }
        "hints.keys.grid_lower_left" => {
            config.hints.keys.grid_lower_left = defaults.hints.keys.grid_lower_left
        }
        "hints.keys.grid_lower_right" => {
            config.hints.keys.grid_lower_right = defaults.hints.keys.grid_lower_right
        }
        "hints.keys.grid_left_click" => {
            config.hints.keys.grid_left_click = defaults.hints.keys.grid_left_click
        }
        "hints.keys.grid_right_click" => {
            config.hints.keys.grid_right_click = defaults.hints.keys.grid_right_click
        }
        "hints.keys.cancel" => config.hints.keys.cancel = defaults.hints.keys.cancel,
        "hints.keys.toggle_background" => {
            config.hints.keys.toggle_background = defaults.hints.keys.toggle_background
        }
        "hints.keys.debug" => config.hints.keys.debug = defaults.hints.keys.debug,
        "hints.recursive_grid_enabled" => {
            config.hints.recursive_grid_enabled = defaults.hints.recursive_grid_enabled
        }
        "hints.label_symbols" => config.hints.label_symbols = defaults.hints.label_symbols,
        "hints.border_color" => config.hints.border_color = defaults.hints.border_color,
        "hints.fill_color" => config.hints.fill_color = defaults.hints.fill_color,
        "hints.grid_colors.upper_left" => {
            config.hints.grid_colors.upper_left = defaults.hints.grid_colors.upper_left
        }
        "hints.grid_colors.upper_right" => {
            config.hints.grid_colors.upper_right = defaults.hints.grid_colors.upper_right
        }
        "hints.grid_colors.lower_left" => {
            config.hints.grid_colors.lower_left = defaults.hints.grid_colors.lower_left
        }
        "hints.grid_colors.lower_right" => {
            config.hints.grid_colors.lower_right = defaults.hints.grid_colors.lower_right
        }
        "hints.readability_color" => {
            config.hints.readability_color = defaults.hints.readability_color
        }
        "hints.label_color" => config.hints.label_color = defaults.hints.label_color,
        "hints.label_highlight_color" => {
            config.hints.label_highlight_color = defaults.hints.label_highlight_color
        }
        "hints.min_width" => config.hints.min_width = defaults.hints.min_width,
        "hints.max_width" => config.hints.max_width = defaults.hints.max_width,
        "hints.min_height" => config.hints.min_height = defaults.hints.min_height,
        "hints.max_height" => config.hints.max_height = defaults.hints.max_height,
        "remap" | "remap.chord_timeout" | "remap.main" | "remap.layers" => {
            config.remap = defaults.remap
        }
        _ => return false,
    }
    true
}

fn conflicting_debug_field(config: &Config, path: &str) -> bool {
    let debug = config.hints.keys.debug;
    match path {
        "keys.free_mouse" => config.keys.free_mouse.contains(&debug),
        "hints.keys.left" => config.hints.keys.left.contains(&debug),
        "hints.keys.right" => config.hints.keys.right.contains(&debug),
        "hints.keys.grid" => config.hints.keys.grid.contains(&debug),
        "hints.keys.cancel" => config.hints.keys.cancel == debug,
        "hints.keys.toggle_background" => config.hints.keys.toggle_background == debug,
        "hints.label_symbols" => config
            .hints
            .label_symbols
            .chars()
            .any(|symbol| parse_key(&symbol.to_string()) == Ok(debug)),
        _ => false,
    }
}

fn validation_path(config: &Config, error: &str, changed: &HashSet<String>) -> Option<String> {
    if error.contains("keycast.hotkey") {
        if let Some((path, _)) = config
            .keys
            .named()
            .into_iter()
            .find(|(_, key)| *key == config.keycast.hotkey)
        {
            if changed.contains(path) {
                return Some(path.to_owned());
            }
        }
    }

    if error.contains("hints.keys.debug conflicts") && !changed.contains("hints.keys.debug") {
        for path in [
            "keys.free_mouse",
            "hints.keys.left",
            "hints.keys.right",
            "hints.keys.grid",
            "hints.keys.cancel",
            "hints.keys.toggle_background",
            "hints.label_symbols",
        ] {
            if changed.contains(path) && conflicting_debug_field(config, path) {
                return Some(path.to_owned());
            }
        }
    }

    if error.contains("hints.keys.toggle_background conflicts")
        && !changed.contains("hints.keys.toggle_background")
    {
        if changed.contains("hints.keys.cancel")
            && config.hints.keys.cancel == config.hints.keys.toggle_background
        {
            return Some("hints.keys.cancel".to_owned());
        }
        if changed.contains("hints.label_symbols")
            && config.hints.label_symbols.chars().any(|symbol| {
                parse_key(&symbol.to_string()) == Ok(config.hints.keys.toggle_background)
            })
        {
            return Some("hints.label_symbols".to_owned());
        }
    }

    changed
        .iter()
        .filter(|path| error.contains(path.as_str()))
        .max_by_key(|path| (error.rfind(path.as_str()).unwrap_or(0), path.len()))
        .cloned()
}

pub(crate) fn parse_key(value: &str) -> Result<K, String> {
    let upper = value.trim().to_ascii_uppercase();
    let name = match upper.as_str() {
        "CTRL" | "CONTROL" => "KEY_LEFTCTRL",
        "SUPER" | "META" | "WIN" | "WINDOWS" | "CMD" | "COMMAND" => "KEY_LEFTMETA",
        "ALT" | "OPTION" => "KEY_LEFTALT",
        "SHIFT" => "KEY_LEFTSHIFT",
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
        controls.push(self.keys.locate);
        controls.extend([K::KEY_LEFTSHIFT, K::KEY_RIGHTSHIFT]);
        if self.keycast.enabled {
            controls.push(self.keycast.hotkey);
        }
        controls
    }

    pub fn parse(text: &str) -> Result<Self, Box<dyn Error>> {
        let value: toml::Value = toml::from_str(text)?;
        let root = value
            .as_table()
            .expect("a TOML document deserializes to a table");
        let mut config = Self::default();
        let mut changed = HashSet::new();

        warn_unknown_fields(
            root,
            &[
                "mode",
                "notify",
                "home_end_enabled",
                "speeds",
                "easing",
                "keys",
                "keycast",
                "hints",
                "remap",
            ],
            "",
        );
        apply_value(root, "mode", "mode", &mut config.mode, &mut changed);
        apply_value(root, "notify", "notify", &mut config.notify, &mut changed);
        apply_value(
            root,
            "home_end_enabled",
            "home_end_enabled",
            &mut config.home_end_enabled,
            &mut changed,
        );

        if let Some(table) = config_section(root, "keycast", "keycast") {
            warn_unknown_fields(
                table,
                &["enabled", "hotkey", "max_keys", "timeout"],
                "keycast",
            );
            apply_value(
                table,
                "enabled",
                "keycast.enabled",
                &mut config.keycast.enabled,
                &mut changed,
            );
            apply_key(
                table,
                "hotkey",
                "keycast.hotkey",
                &mut config.keycast.hotkey,
                &mut changed,
            );
            apply_value(
                table,
                "max_keys",
                "keycast.max_keys",
                &mut config.keycast.max_keys,
                &mut changed,
            );
            apply_value(
                table,
                "timeout",
                "keycast.timeout",
                &mut config.keycast.timeout,
                &mut changed,
            );
        }

        if let Some(table) = config_section(root, "speeds", "speeds") {
            warn_unknown_fields(
                table,
                &[
                    "normal",
                    "slow",
                    "fast",
                    "scroll",
                    "scroll_slow",
                    "scroll_fast",
                ],
                "speeds",
            );
            apply_value(
                table,
                "normal",
                "speeds.normal",
                &mut config.speeds.normal,
                &mut changed,
            );
            apply_value(
                table,
                "slow",
                "speeds.slow",
                &mut config.speeds.slow,
                &mut changed,
            );
            apply_value(
                table,
                "fast",
                "speeds.fast",
                &mut config.speeds.fast,
                &mut changed,
            );
            apply_value(
                table,
                "scroll",
                "speeds.scroll",
                &mut config.speeds.scroll,
                &mut changed,
            );
            apply_value(
                table,
                "scroll_slow",
                "speeds.scroll_slow",
                &mut config.speeds.scroll_slow,
                &mut changed,
            );
            apply_value(
                table,
                "scroll_fast",
                "speeds.scroll_fast",
                &mut config.speeds.scroll_fast,
                &mut changed,
            );
        }

        if let Some(table) = config_section(root, "easing", "easing") {
            warn_unknown_fields(table, &["movement", "movement_stop", "scroll"], "easing");
            apply_value(
                table,
                "movement",
                "easing.movement",
                &mut config.easing.movement,
                &mut changed,
            );
            apply_value(
                table,
                "movement_stop",
                "easing.movement_stop",
                &mut config.easing.movement_stop,
                &mut changed,
            );
            apply_value(
                table,
                "scroll",
                "easing.scroll",
                &mut config.easing.scroll,
                &mut changed,
            );
        }

        if let Some(table) = config_section(root, "keys", "keys") {
            warn_unknown_fields(
                table,
                &[
                    "free_mouse",
                    "reload_config",
                    "config_screen",
                    "toggle_hold",
                    "left",
                    "down",
                    "up",
                    "right",
                    "left_click",
                    "right_click",
                    "scroll_left",
                    "scroll_down",
                    "scroll_up",
                    "scroll_right",
                    "slow",
                    "fast",
                    "locate",
                ],
                "keys",
            );
            apply_chord(
                table,
                "free_mouse",
                "keys.free_mouse",
                &mut config.keys.free_mouse,
                &mut changed,
            );
            apply_chord(
                table,
                "reload_config",
                "keys.reload_config",
                &mut config.keys.reload_config,
                &mut changed,
            );
            apply_chord(
                table,
                "config_screen",
                "keys.config_screen",
                &mut config.keys.config_screen,
                &mut changed,
            );
            apply_key(
                table,
                "toggle_hold",
                "keys.toggle_hold",
                &mut config.keys.toggle_hold,
                &mut changed,
            );
            apply_key(
                table,
                "left",
                "keys.left",
                &mut config.keys.left,
                &mut changed,
            );
            apply_key(
                table,
                "down",
                "keys.down",
                &mut config.keys.down,
                &mut changed,
            );
            apply_key(table, "up", "keys.up", &mut config.keys.up, &mut changed);
            apply_key(
                table,
                "right",
                "keys.right",
                &mut config.keys.right,
                &mut changed,
            );
            apply_key(
                table,
                "left_click",
                "keys.left_click",
                &mut config.keys.left_click,
                &mut changed,
            );
            apply_key(
                table,
                "right_click",
                "keys.right_click",
                &mut config.keys.right_click,
                &mut changed,
            );
            apply_key(
                table,
                "scroll_left",
                "keys.scroll_left",
                &mut config.keys.scroll_left,
                &mut changed,
            );
            apply_key(
                table,
                "scroll_down",
                "keys.scroll_down",
                &mut config.keys.scroll_down,
                &mut changed,
            );
            apply_key(
                table,
                "scroll_up",
                "keys.scroll_up",
                &mut config.keys.scroll_up,
                &mut changed,
            );
            apply_key(
                table,
                "scroll_right",
                "keys.scroll_right",
                &mut config.keys.scroll_right,
                &mut changed,
            );
            apply_key(
                table,
                "slow",
                "keys.slow",
                &mut config.keys.slow,
                &mut changed,
            );
            apply_key(
                table,
                "fast",
                "keys.fast",
                &mut config.keys.fast,
                &mut changed,
            );
            apply_key(
                table,
                "locate",
                "keys.locate",
                &mut config.keys.locate,
                &mut changed,
            );
        }

        if let Some(table) = config_section(root, "hints", "hints") {
            warn_unknown_fields(
                table,
                &[
                    "keys",
                    "recursive_grid_enabled",
                    "label_symbols",
                    "border_color",
                    "fill_color",
                    "grid_colors",
                    "readability_color",
                    "label_color",
                    "label_highlight_color",
                    "min_width",
                    "max_width",
                    "min_height",
                    "max_height",
                ],
                "hints",
            );
            apply_value(
                table,
                "recursive_grid_enabled",
                "hints.recursive_grid_enabled",
                &mut config.hints.recursive_grid_enabled,
                &mut changed,
            );
            apply_value(
                table,
                "label_symbols",
                "hints.label_symbols",
                &mut config.hints.label_symbols,
                &mut changed,
            );
            apply_color(
                table,
                "border_color",
                "hints.border_color",
                &mut config.hints.border_color,
                &mut changed,
            );
            apply_color(
                table,
                "fill_color",
                "hints.fill_color",
                &mut config.hints.fill_color,
                &mut changed,
            );
            if let Some(grid_colors) = config_section(table, "grid_colors", "hints.grid_colors") {
                warn_unknown_fields(
                    grid_colors,
                    &["upper_left", "upper_right", "lower_left", "lower_right"],
                    "hints.grid_colors",
                );
                apply_optional_color(
                    grid_colors,
                    "upper_left",
                    "hints.grid_colors.upper_left",
                    &mut config.hints.grid_colors.upper_left,
                    &mut changed,
                );
                apply_optional_color(
                    grid_colors,
                    "upper_right",
                    "hints.grid_colors.upper_right",
                    &mut config.hints.grid_colors.upper_right,
                    &mut changed,
                );
                apply_optional_color(
                    grid_colors,
                    "lower_left",
                    "hints.grid_colors.lower_left",
                    &mut config.hints.grid_colors.lower_left,
                    &mut changed,
                );
                apply_optional_color(
                    grid_colors,
                    "lower_right",
                    "hints.grid_colors.lower_right",
                    &mut config.hints.grid_colors.lower_right,
                    &mut changed,
                );
            }
            apply_color(
                table,
                "readability_color",
                "hints.readability_color",
                &mut config.hints.readability_color,
                &mut changed,
            );
            apply_color(
                table,
                "label_color",
                "hints.label_color",
                &mut config.hints.label_color,
                &mut changed,
            );
            apply_color(
                table,
                "label_highlight_color",
                "hints.label_highlight_color",
                &mut config.hints.label_highlight_color,
                &mut changed,
            );
            apply_value(
                table,
                "min_width",
                "hints.min_width",
                &mut config.hints.min_width,
                &mut changed,
            );
            apply_value(
                table,
                "max_width",
                "hints.max_width",
                &mut config.hints.max_width,
                &mut changed,
            );
            apply_value(
                table,
                "min_height",
                "hints.min_height",
                &mut config.hints.min_height,
                &mut changed,
            );
            apply_value(
                table,
                "max_height",
                "hints.max_height",
                &mut config.hints.max_height,
                &mut changed,
            );

            if let Some(keys) = config_section(table, "keys", "hints.keys") {
                warn_unknown_fields(
                    keys,
                    &[
                        "left",
                        "right",
                        "grid",
                        "grid_upper_left",
                        "grid_upper_right",
                        "grid_lower_left",
                        "grid_lower_right",
                        "grid_left_click",
                        "grid_right_click",
                        "cancel",
                        "toggle_background",
                        "debug",
                    ],
                    "hints.keys",
                );
                apply_chord(
                    keys,
                    "left",
                    "hints.keys.left",
                    &mut config.hints.keys.left,
                    &mut changed,
                );
                apply_chord(
                    keys,
                    "right",
                    "hints.keys.right",
                    &mut config.hints.keys.right,
                    &mut changed,
                );
                apply_chord(
                    keys,
                    "grid",
                    "hints.keys.grid",
                    &mut config.hints.keys.grid,
                    &mut changed,
                );
                apply_key(
                    keys,
                    "grid_upper_left",
                    "hints.keys.grid_upper_left",
                    &mut config.hints.keys.grid_upper_left,
                    &mut changed,
                );
                apply_key(
                    keys,
                    "grid_upper_right",
                    "hints.keys.grid_upper_right",
                    &mut config.hints.keys.grid_upper_right,
                    &mut changed,
                );
                apply_key(
                    keys,
                    "grid_lower_left",
                    "hints.keys.grid_lower_left",
                    &mut config.hints.keys.grid_lower_left,
                    &mut changed,
                );
                apply_key(
                    keys,
                    "grid_lower_right",
                    "hints.keys.grid_lower_right",
                    &mut config.hints.keys.grid_lower_right,
                    &mut changed,
                );
                apply_key(
                    keys,
                    "grid_left_click",
                    "hints.keys.grid_left_click",
                    &mut config.hints.keys.grid_left_click,
                    &mut changed,
                );
                apply_key(
                    keys,
                    "grid_right_click",
                    "hints.keys.grid_right_click",
                    &mut config.hints.keys.grid_right_click,
                    &mut changed,
                );
                apply_key(
                    keys,
                    "cancel",
                    "hints.keys.cancel",
                    &mut config.hints.keys.cancel,
                    &mut changed,
                );
                apply_key(
                    keys,
                    "toggle_background",
                    "hints.keys.toggle_background",
                    &mut config.hints.keys.toggle_background,
                    &mut changed,
                );
                apply_key(
                    keys,
                    "debug",
                    "hints.keys.debug",
                    &mut config.hints.keys.debug,
                    &mut changed,
                );
            }
        }

        if let Some(table) = config_section(root, "remap", "remap") {
            warn_unknown_fields(table, &["chord_timeout", "main", "layers"], "remap");
            let mut remap = crate::remap::RemapConfig::default();
            apply_parsed(
                table,
                "chord_timeout",
                "remap.chord_timeout",
                &mut remap.chord_timeout,
                &mut changed,
                |value| {
                    let timeout = parse_toml_value::<u64>(value)?;
                    if timeout == 0 {
                        Err("must be greater than zero".to_owned())
                    } else {
                        Ok(timeout)
                    }
                },
            );

            let mut entries: Vec<(Option<String>, String, String)> = Vec::new();
            if let Some(main) = config_section(table, "main", "remap.main") {
                for (trigger, value) in main {
                    match parse_toml_value::<String>(value) {
                        Ok(action) => entries.push((None, trigger.clone(), action)),
                        Err(error) => eprintln!(
                            "Warning: invalid config value remap.main.{trigger}: {error}; skipping it"
                        ),
                    }
                }
            }

            let mut layer_entries = Vec::new();
            if let Some(layers) = config_section(table, "layers", "remap.layers") {
                for (name, value) in layers {
                    if name.is_empty()
                        || !name.chars().all(|character| {
                            character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
                        })
                        || matches!(name.as_str(), "control" | "ctrl" | "meta" | "super")
                    {
                        eprintln!(
                            "Warning: invalid config value remap.layers.{name}: invalid or reserved layer name; skipping it"
                        );
                        continue;
                    }
                    let Some(layer) = value.as_table() else {
                        eprintln!(
                            "Warning: invalid config value remap.layers.{name}: expected a table; skipping it"
                        );
                        continue;
                    };
                    let mut values = Vec::new();
                    for (trigger, action) in layer {
                        match parse_toml_value::<String>(action) {
                            Ok(action) => values.push((trigger.clone(), action)),
                            Err(error) => eprintln!(
                                "Warning: invalid config value remap.layers.{name}.{trigger}: {error}; skipping it"
                            ),
                        }
                    }
                    layer_entries.push((name.clone(), values));
                }
            }

            remap.layers = layer_entries
                .iter()
                .map(|(name, _)| (name.clone(), BTreeMap::new()))
                .collect();
            for (layer, entries) in layer_entries {
                for (trigger, action) in entries {
                    let candidate = remap_with_entry(&remap, Some(&layer), &trigger, &action);
                    match candidate.validate() {
                        Ok(()) => remap = candidate,
                        Err(error) => eprintln!(
                            "Warning: invalid config value remap.layers.{layer}.{trigger}: {error}; skipping it"
                        ),
                    }
                }
            }
            for (layer, trigger, action) in entries {
                let candidate = remap_with_entry(&remap, layer.as_deref(), &trigger, &action);
                match candidate.validate() {
                    Ok(()) => remap = candidate,
                    Err(error) => {
                        let path = layer.map_or_else(
                            || format!("remap.main.{trigger}"),
                            |layer| format!("remap.layers.{layer}.{trigger}"),
                        );
                        eprintln!("Warning: invalid config value {path}: {error}; skipping it");
                    }
                }
            }
            config.remap = remap;
            changed.insert("remap".to_owned());
        }

        if let Err(error) = config.remap.validate() {
            eprintln!("Warning: invalid config value remap: {error}; skipping remapping settings");
            config.remap = crate::remap::RemapConfig::default();
            changed.remove("remap");
        }

        while let Err(error) = config.validate() {
            let path = validation_path(&config, &error, &changed)
                .or_else(|| changed.iter().max().cloned());
            let Some(path) = path else {
                eprintln!("Warning: invalid config value: {error}; using remaining defaults");
                config = Self::default();
                break;
            };
            if !reset_config_value(&mut config, &path) {
                warn_invalid_config_value(&path, &error);
                config = Self::default();
                break;
            }
            warn_invalid_config_value(&path, &error);
            changed.remove(&path);
        }
        Ok(config)
    }

    pub(crate) fn parse_strict(text: &str) -> Result<Self, Box<dyn Error>> {
        let values: toml::Value = toml::from_str(text)?;
        let mut defaults: toml::Value = toml::from_str(&Self::default().to_toml()?)?;
        merge_toml_values(&mut defaults, values);
        let config: Self = defaults.try_into()?;
        config.validate().map_err(io::Error::other)?;
        Ok(config)
    }

    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        use toml::Value;

        let mut root = TomlTable::new();
        root.insert(
            "mode".to_owned(),
            Value::String(
                match self.mode {
                    Mode::Hold => "hold",
                    Mode::Toggle => "toggle",
                }
                .to_owned(),
            ),
        );
        root.insert("notify".to_owned(), Value::Boolean(self.notify));
        root.insert(
            "home_end_enabled".to_owned(),
            Value::Boolean(self.home_end_enabled),
        );
        root.insert(
            "speeds".to_owned(),
            toml_table([
                ("normal", Value::Float(self.speeds.normal)),
                ("slow", Value::Float(self.speeds.slow)),
                ("fast", Value::Float(self.speeds.fast)),
                ("scroll", Value::Float(self.speeds.scroll)),
                ("scroll_slow", Value::Float(self.speeds.scroll_slow)),
                ("scroll_fast", Value::Float(self.speeds.scroll_fast)),
            ]),
        );
        let mut easing = TomlTable::new();
        easing.insert("movement".to_owned(), Value::Float(self.easing.movement));
        if let Some(movement_stop) = self.easing.movement_stop {
            easing.insert("movement_stop".to_owned(), Value::Float(movement_stop));
        }
        easing.insert("scroll".to_owned(), Value::Float(self.easing.scroll));
        root.insert("easing".to_owned(), Value::Table(easing));
        root.insert(
            "keys".to_owned(),
            toml_table([
                ("free_mouse", toml_chord(&self.keys.free_mouse)),
                ("reload_config", toml_chord(&self.keys.reload_config)),
                ("config_screen", toml_chord(&self.keys.config_screen)),
                ("toggle_hold", toml_key(self.keys.toggle_hold)),
                ("left", toml_key(self.keys.left)),
                ("down", toml_key(self.keys.down)),
                ("up", toml_key(self.keys.up)),
                ("right", toml_key(self.keys.right)),
                ("left_click", toml_key(self.keys.left_click)),
                ("right_click", toml_key(self.keys.right_click)),
                ("scroll_left", toml_key(self.keys.scroll_left)),
                ("scroll_down", toml_key(self.keys.scroll_down)),
                ("scroll_up", toml_key(self.keys.scroll_up)),
                ("scroll_right", toml_key(self.keys.scroll_right)),
                ("slow", toml_key(self.keys.slow)),
                ("fast", toml_key(self.keys.fast)),
                ("locate", toml_key(self.keys.locate)),
            ]),
        );
        root.insert(
            "keycast".to_owned(),
            toml_table([
                ("enabled", Value::Boolean(self.keycast.enabled)),
                ("hotkey", toml_key(self.keycast.hotkey)),
                ("max_keys", Value::Integer(self.keycast.max_keys as i64)),
                ("timeout", Value::Float(self.keycast.timeout)),
            ]),
        );

        let hint_keys = toml_table([
            ("left", toml_chord(&self.hints.keys.left)),
            ("right", toml_chord(&self.hints.keys.right)),
            ("grid", toml_chord(&self.hints.keys.grid)),
            ("grid_upper_left", toml_key(self.hints.keys.grid_upper_left)),
            (
                "grid_upper_right",
                toml_key(self.hints.keys.grid_upper_right),
            ),
            ("grid_lower_left", toml_key(self.hints.keys.grid_lower_left)),
            (
                "grid_lower_right",
                toml_key(self.hints.keys.grid_lower_right),
            ),
            ("grid_left_click", toml_key(self.hints.keys.grid_left_click)),
            (
                "grid_right_click",
                toml_key(self.hints.keys.grid_right_click),
            ),
            ("cancel", toml_key(self.hints.keys.cancel)),
            (
                "toggle_background",
                toml_key(self.hints.keys.toggle_background),
            ),
            ("debug", toml_key(self.hints.keys.debug)),
        ]);
        let mut grid_colors = TomlTable::new();
        for (name, color) in [
            ("upper_left", self.hints.grid_colors.upper_left),
            ("upper_right", self.hints.grid_colors.upper_right),
            ("lower_left", self.hints.grid_colors.lower_left),
            ("lower_right", self.hints.grid_colors.lower_right),
        ] {
            if let Some(color) = color {
                grid_colors.insert(name.to_owned(), toml_color(color));
            }
        }
        root.insert(
            "hints".to_owned(),
            toml_table([
                (
                    "recursive_grid_enabled",
                    Value::Boolean(self.hints.recursive_grid_enabled),
                ),
                ("keys", hint_keys),
                (
                    "label_symbols",
                    Value::String(self.hints.label_symbols.clone()),
                ),
                ("border_color", toml_color(self.hints.border_color)),
                ("fill_color", toml_color(self.hints.fill_color)),
                ("grid_colors", Value::Table(grid_colors)),
                (
                    "readability_color",
                    toml_color(self.hints.readability_color),
                ),
                ("label_color", toml_color(self.hints.label_color)),
                (
                    "label_highlight_color",
                    toml_color(self.hints.label_highlight_color),
                ),
                ("min_width", Value::Integer(self.hints.min_width.into())),
                ("max_width", Value::Integer(self.hints.max_width.into())),
                ("min_height", Value::Integer(self.hints.min_height.into())),
                ("max_height", Value::Integer(self.hints.max_height.into())),
            ]),
        );

        let main = self
            .remap
            .main
            .iter()
            .map(|(trigger, action)| (trigger.clone(), Value::String(action.clone())))
            .collect();
        let layers = self
            .remap
            .layers
            .iter()
            .map(|(name, mappings)| {
                (
                    name.clone(),
                    Value::Table(
                        mappings
                            .iter()
                            .map(|(trigger, action)| {
                                (trigger.clone(), Value::String(action.clone()))
                            })
                            .collect(),
                    ),
                )
            })
            .collect();
        root.insert(
            "remap".to_owned(),
            toml_table([
                ("chord_timeout", Value::try_from(self.remap.chord_timeout)?),
                ("main", Value::Table(main)),
                ("layers", Value::Table(layers)),
            ]),
        );

        toml::to_string_pretty(&Value::Table(root))
    }

    pub fn load(path: &Path, required: bool) -> Result<Self, Box<dyn Error>> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Cannot create config directory {}: {error}",
                    parent.display()
                )
            })?;
        }
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

    pub fn load_or_create_default(path: &Path) -> Result<Self, Box<dyn Error>> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "Cannot create config directory {}: {error}",
                    parent.display()
                )
            })?;
        }

        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => file
                .write_all(include_str!("../fievel.config").as_bytes())
                .map_err(|error| {
                    format!("Cannot write default config {}: {error}", path.display())
                })?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(
                    format!("Cannot create default config {}: {error}", path.display()).into(),
                )
            }
        }

        Self::load(path, true)
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
            return Err(
                "keycast.timeout: must be finite, greater than zero, and at most 60 seconds"
                    .to_owned(),
            );
        }
        if self.keycast.enabled
            && (self
                .keys
                .named()
                .iter()
                .any(|(_, key)| *key == self.keycast.hotkey)
                || self.keys.config_screen.as_slice() == [self.keycast.hotkey])
        {
            return Err(
                "keycast.hotkey must differ from Free Mouse Mode activation, control keys, and keys.config_screen"
                    .to_owned(),
            );
        }
        for (name, value) in [
            ("movement", self.easing.movement),
            (
                "movement_stop",
                self.easing.movement_stop.unwrap_or(self.easing.movement),
            ),
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
        validate_chord("keys.reload_config", &self.keys.reload_config)?;
        validate_chord("keys.config_screen", &self.keys.config_screen)?;
        if !self
            .keys
            .config_screen
            .iter()
            .any(|key| !crate::reload::is_modifier(*key))
        {
            return Err("keys.config_screen must include a non-modifier key".to_owned());
        }
        if !self
            .keys
            .reload_config
            .iter()
            .any(|key| !crate::reload::is_modifier(*key))
        {
            return Err("keys.reload_config must include a non-modifier key".to_owned());
        }
        validate_unique_chords(
            "keys.reload_config",
            &self.keys.reload_config,
            "keys.config_screen",
            &self.keys.config_screen,
        )?;
        for (name, chord) in [
            ("keys.free_mouse", &self.keys.free_mouse),
            ("hints.keys.left", &self.hints.keys.left),
            ("hints.keys.right", &self.hints.keys.right),
            ("hints.keys.grid", &self.hints.keys.grid),
        ] {
            validate_unique_chords("keys.reload_config", &self.keys.reload_config, name, chord)?;
            validate_unique_chords("keys.config_screen", &self.keys.config_screen, name, chord)?;
        }
        validate_chord("hints.keys.left", &self.hints.keys.left)?;
        validate_chord("hints.keys.right", &self.hints.keys.right)?;
        validate_chord("hints.keys.grid", &self.hints.keys.grid)?;
        let grid_keys = [
            (
                "hints.keys.grid_upper_left",
                self.hints.keys.grid_upper_left,
            ),
            (
                "hints.keys.grid_upper_right",
                self.hints.keys.grid_upper_right,
            ),
            (
                "hints.keys.grid_lower_left",
                self.hints.keys.grid_lower_left,
            ),
            (
                "hints.keys.grid_lower_right",
                self.hints.keys.grid_lower_right,
            ),
            (
                "hints.keys.grid_left_click",
                self.hints.keys.grid_left_click,
            ),
            (
                "hints.keys.grid_right_click",
                self.hints.keys.grid_right_click,
            ),
        ];
        for (name, key) in &grid_keys[..4] {
            if !is_letter_key(*key) {
                return Err(format!("{name} must be a letter from A to Z"));
            }
        }
        let mut assigned_grid_keys = BTreeMap::new();
        for (name, key) in grid_keys {
            if let Some(other) = assigned_grid_keys.insert(key, name) {
                return Err(format!("{name} and {other} both bind {key:?}"));
            }
            if [
                K::KEY_RESERVED,
                K::KEY_UNKNOWN,
                K::KEY_ESC,
                K::KEY_BACKSPACE,
                K::KEY_LEFTCTRL,
                K::KEY_RIGHTCTRL,
                K::KEY_LEFTALT,
                K::KEY_RIGHTALT,
                K::KEY_LEFTMETA,
                K::KEY_RIGHTMETA,
                K::KEY_LEFTSHIFT,
                K::KEY_RIGHTSHIFT,
                self.hints.keys.cancel,
                self.hints.keys.toggle_background,
                self.hints.keys.debug,
            ]
            .contains(&key)
            {
                return Err(format!(
                    "{name} conflicts with a modifier, reserved grid, cancellation, peek, or debug key"
                ));
            }
            if self.keys.free_mouse.len() == 1 && self.keys.free_mouse[0] == key {
                return Err(format!("{name} conflicts with keys.free_mouse"));
            }
        }
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
        if [
            K::KEY_RESERVED,
            K::KEY_UNKNOWN,
            K::KEY_ESC,
            K::KEY_BACKSPACE,
            self.hints.keys.cancel,
            toggle,
        ]
        .contains(&debug)
            || self.hints.keys.left.contains(&debug)
            || self.hints.keys.right.contains(&debug)
            || self.hints.keys.grid.contains(&debug)
            || self.keys.free_mouse.contains(&debug)
            || self
                .hints
                .label_symbols
                .chars()
                .any(|symbol| parse_key(&symbol.to_string()) == Ok(debug))
        {
            return Err(
                "hints.keys.debug conflicts with reserved, label, cancel, peek, or activation keys"
                    .to_owned(),
            );
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
            ("hints.keys.grid", &self.hints.keys.grid),
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
        validate_unique_chords(
            "keys.free_mouse",
            &self.keys.free_mouse,
            "hints.keys.grid",
            &self.hints.keys.grid,
        )?;
        validate_unique_chords(
            "hints.keys.left",
            &self.hints.keys.left,
            "hints.keys.grid",
            &self.hints.keys.grid,
        )?;
        validate_unique_chords(
            "hints.keys.right",
            &self.hints.keys.right,
            "hints.keys.grid",
            &self.hints.keys.grid,
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
    if first.len() == second.len() && first.iter().all(|key| second.contains(key)) {
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

    fn configured_super() -> K {
        K::KEY_LEFTMETA
    }

    fn configured_right_super() -> K {
        K::KEY_RIGHTMETA
    }

    #[test]
    fn link_detection_is_not_configurable() {
        assert!(Config::parse("").is_ok());
        assert!(Config::parse(include_str!("../fievel.config")).is_ok());
        assert!(Config::parse("[hints]\ncolor_links = false").is_ok());
        assert!(Config::parse("[hints]\nunderline_links = false").is_ok());
    }

    #[test]
    fn debug_key_defaults_customization_and_conflicts() {
        assert_eq!(Config::parse("").unwrap().hints.keys.debug, K::KEY_F8);
        assert_eq!(
            Config::parse("[hints.keys]\ndebug = 'f9'")
                .unwrap()
                .hints
                .keys
                .debug,
            K::KEY_F9
        );
        for key in [
            "reserved",
            "unknown",
            "not-a-key",
            "esc",
            "enter",
            "backspace",
            "leftctrl",
            "leftmeta",
            "space",
            "i",
            "z",
            "f3",
            "f8+f9",
        ] {
            assert!(
                Config::parse(&format!("[hints.keys]\ndebug = '{key}'")).is_ok(),
                "{key}"
            );
        }
        for text in [
            "[hints.keys]\ndebug = 'f9'\ncancel = 'f9'",
            "[hints.keys]\ndebug = 'f9'\ntoggle_background = 'f9'",
            "[hints.keys]\ndebug = 'f9'\nleft = 'f9'",
            "[hints.keys]\ndebug = 'f9'\nright = 'leftmeta+f9'",
            "[hints.keys]\ndebugg = 'f9'",
        ] {
            assert!(Config::parse(text).is_ok(), "{text}");
        }
        assert_eq!(
            Config::parse(include_str!("../fievel.config"))
                .unwrap()
                .hints
                .keys
                .debug,
            K::KEY_F8
        );
    }

    #[test]
    fn example_matches_defaults_with_home_row_remappings() {
        let config = Config::parse(include_str!("../fievel.config")).unwrap();
        let default = Config::default();
        assert_eq!(config.mode, Mode::Hold);
        assert!(config.notify);
        assert!(config.home_end_enabled);
        assert!(!config.keycast.enabled);
        assert_eq!(config.keycast.hotkey, K::KEY_ESC);
        assert_eq!(config.keycast.max_keys, 8);
        assert_eq!(config.keycast.timeout, 3.0);
        assert_eq!(config.keys.locate, K::KEY_Q);
        assert_eq!(config.keys.named(), default.keys.named());
        assert_eq!(config.hints, default.hints);
        assert!(default.remap.main.is_empty());
        assert!(default.remap.layers.is_empty());
        assert_eq!(config.remap.main["d+f"], "free_mouse");
        assert_eq!(config.remap.layers["nav"]["h"], "left");
        assert_eq!(config.easing.movement, 0.4);
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
        let config =
            Config::parse("[keycast]\nenabled = true\nhotkey = 'F8'\nmax_keys = 6\ntimeout = 2.5")
                .unwrap();
        assert!(config.keycast.enabled);
        assert_eq!(config.keycast.hotkey, K::KEY_F8);
        assert_eq!(config.keycast.max_keys, 6);
        assert_eq!(config.keycast.timeout, 2.5);
        assert!(config.mouse_controls().contains(&K::KEY_F8));
        assert!(Config::default().mouse_controls().contains(&K::KEY_Q));
        assert!(!Config::default().mouse_controls().contains(&K::KEY_ESC));
        for text in [
            "enabled = 'true'",
            "hotkey = 'not-a-key'",
            "hotkey = 'a+b'",
            "max_keys = 0",
            "max_keys = 33",
            "max_keys = -1",
            "timeout = 0",
            "timeout = -1",
            "timeout = 61",
            "timeout = nan",
            "timeout = inf",
            "timout = 3",
            "enabled = true\nhotkey = 'h'",
            "enabled = true\nhotkey = 'f3'",
        ] {
            assert!(
                Config::parse(&format!("[keycast]\n{text}")).is_ok(),
                "{text}"
            );
        }
        assert!(Config::parse(
            "[keys]\nfree_mouse = 'leftalt+space'\n[keycast]\nenabled = true\nhotkey = 'leftalt'"
        )
        .is_ok());
    }

    #[test]
    fn both_shift_keys_are_mouse_controls() {
        let controls = Config::default().mouse_controls();
        assert!(controls.contains(&K::KEY_LEFTSHIFT));
        assert!(controls.contains(&K::KEY_RIGHTSHIFT));
    }

    #[test]
    fn reload_config_chord_defaults_and_can_be_rebound() {
        let default = vec![configured_super(), K::KEY_LEFTSHIFT, K::KEY_R];
        assert_eq!(Config::default().keys.reload_config, default);
        assert_eq!(Config::parse("").unwrap().keys.reload_config, default);
        assert_eq!(
            Config::parse("[keys]\nreload_config = 'rightmeta + f8'")
                .unwrap()
                .keys
                .reload_config,
            [configured_right_super(), K::KEY_F8]
        );
        assert_eq!(
            Config::parse("[keys]\nreload_config = 'f8 + f8'")
                .unwrap()
                .keys
                .reload_config,
            default
        );
        assert_eq!(
            Config::parse("[keys]\nreload_config = 'space + leftmeta'")
                .unwrap()
                .keys
                .reload_config,
            default
        );
        assert_eq!(
            Config::parse("[keys]\nreload_config = 'leftmeta + leftshift'")
                .unwrap()
                .keys
                .reload_config,
            default
        );
    }

    #[test]
    fn config_screen_chord_defaults_and_can_be_rebound() {
        let default = vec![configured_super(), K::KEY_LEFTSHIFT, K::KEY_SLASH];
        assert_eq!(Config::default().keys.config_screen, default);
        assert_eq!(Config::parse("").unwrap().keys.config_screen, default);
        assert_eq!(
            Config::parse("[keys]\nconfig_screen = 'rightmeta + shift + f9'")
                .unwrap()
                .keys
                .config_screen,
            [configured_right_super(), K::KEY_LEFTSHIFT, K::KEY_F9]
        );
        assert_eq!(
            Config::parse("[keys]\nconfig_screen = 'leftmeta + leftshift'")
                .unwrap()
                .keys
                .config_screen,
            default
        );
    }

    #[test]
    fn effective_config_serializes_all_settings_and_round_trips() {
        let config = Config::parse(include_str!("../fievel.config")).unwrap();
        let serialized = config.to_toml().unwrap();
        let reparsed = Config::parse(&serialized).unwrap();

        assert_eq!(reparsed.mode, config.mode);
        assert_eq!(reparsed.notify, config.notify);
        assert_eq!(reparsed.home_end_enabled, config.home_end_enabled);
        assert_eq!(
            (
                reparsed.speeds.normal,
                reparsed.speeds.slow,
                reparsed.speeds.fast,
                reparsed.speeds.scroll,
                reparsed.speeds.scroll_slow,
                reparsed.speeds.scroll_fast,
            ),
            (
                config.speeds.normal,
                config.speeds.slow,
                config.speeds.fast,
                config.speeds.scroll,
                config.speeds.scroll_slow,
                config.speeds.scroll_fast,
            )
        );
        assert_eq!(
            (
                reparsed.easing.movement,
                reparsed.easing.movement_stop,
                reparsed.easing.scroll,
            ),
            (
                config.easing.movement,
                config.easing.movement_stop,
                config.easing.scroll,
            )
        );
        assert_eq!(reparsed.keys.named(), config.keys.named());
        assert_eq!(reparsed.keys.config_screen, config.keys.config_screen);
        assert_eq!(reparsed.keys.reload_config, config.keys.reload_config);
        assert_eq!(reparsed.keys.free_mouse, config.keys.free_mouse);
        assert_eq!(reparsed.keycast.enabled, config.keycast.enabled);
        assert_eq!(reparsed.keycast.hotkey, config.keycast.hotkey);
        assert_eq!(reparsed.keycast.max_keys, config.keycast.max_keys);
        assert_eq!(reparsed.keycast.timeout, config.keycast.timeout);
        assert_eq!(reparsed.hints, config.hints);
        assert_eq!(reparsed.remap.chord_timeout, config.remap.chord_timeout);
        assert_eq!(reparsed.remap.main, config.remap.main);
        assert_eq!(reparsed.remap.layers, config.remap.layers);
        assert!(serialized.contains("[hints.keys]"));
        assert!(serialized.contains("config_screen ="));
    }

    #[test]
    fn strict_config_parser_fills_platform_defaults_and_rejects_invalid_values() {
        let config = Config::parse_strict("notify = false").unwrap();
        assert!(!config.notify);
        assert_eq!(
            config.keys.config_screen,
            vec![configured_super(), K::KEY_LEFTSHIFT, K::KEY_SLASH]
        );
        assert!(Config::parse_strict("[speeds]\nnormal = -1").is_err());
        assert!(Config::parse_strict("unknown = true").is_err());
        assert!(Config::parse_strict(
            "[keycast]\nenabled = true\nhotkey = 'f9'\n[keys]\nconfig_screen = 'f9'"
        )
        .is_err());
    }

    #[test]
    fn easing_defaults_overrides_and_validation() {
        let default = Config::parse("").unwrap();
        assert_eq!(default.easing.movement, 0.2);
        assert_eq!(default.easing.movement_stop, None);
        assert_eq!(default.easing.scroll, 0.3);
        let config = Config::parse("[easing]\nmovement = 0").unwrap();
        assert_eq!(config.easing.movement, 0.0);
        assert_eq!(config.easing.scroll, 0.3);
        let config = Config::parse("[easing]\nmovement = 1\nscroll = 0.5").unwrap();
        assert_eq!(config.easing.movement, 1.0);
        assert_eq!(config.easing.scroll, 0.5);
        for field in ["movement", "movement_stop", "scroll"] {
            for value in ["-0.1", "1.01", "nan", "inf", "-inf", "'0.2'", "true"] {
                assert!(Config::parse(&format!("[easing]\n{field} = {value}")).is_ok());
            }
        }
        assert!(Config::parse("[easing]\nmovment = 0.2").is_ok());
    }

    #[test]
    fn movement_stop_easing_is_optional_and_validated() {
        let config = Config::parse("[easing]\nmovement = 0.1\nmovement_stop = 1").unwrap();
        assert_eq!(
            (config.easing.movement, config.easing.movement_stop),
            (0.1, Some(1.0))
        );
        let config = Config::parse("[easing]\nmovement_stop = 1.5").unwrap();
        assert_eq!(config.easing.movement_stop, None);
    }

    #[test]
    fn activation_mode_defaults_to_hold_and_accepts_only_hold_or_toggle() {
        assert_eq!(Config::parse("").unwrap().mode, Mode::Hold);
        assert_eq!(Config::parse("mode = 'hold'").unwrap().mode, Mode::Hold);
        assert_eq!(Config::parse("mode = 'toggle'").unwrap().mode, Mode::Toggle);
        for text in ["mode = 'tap'", "mode = true", "mode = 1"] {
            assert!(Config::parse(text).is_ok());
        }
    }

    #[test]
    fn cursor_locator_key_defaults_to_q_and_must_not_conflict_with_controls() {
        assert_eq!(Config::parse("").unwrap().keys.locate, K::KEY_Q);
        assert_eq!(
            Config::parse("[keys]\nlocate = 'f9'").unwrap().keys.locate,
            K::KEY_F9
        );
        for binding in ["h", "f3", "z", "space"] {
            assert!(
                Config::parse(&format!("[keys]\nlocate = '{binding}'")).is_ok(),
                "{binding}"
            );
        }
        assert!(Config::parse("[keys]\nlocater = 'f9'").is_ok());
        assert!(Config::parse("[keys]\nleft = 'q'").is_ok());
    }

    #[test]
    fn indicator_defaults_on_and_accepts_only_booleans() {
        assert!(Config::default().notify);
        assert!(Config::parse("").unwrap().notify);
        assert!(Config::parse("mode = 'toggle'").unwrap().notify);
        assert!(Config::parse("notify = true").unwrap().notify);
        assert!(!Config::parse("notify = false").unwrap().notify);
        for text in ["notify = 'true'", "notify = 1", "notify = []"] {
            assert!(Config::parse(text).is_ok(), "{text}");
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
            assert!(Config::parse(&format!("home_end_enabled = {value}")).is_ok());
        }
    }

    #[test]
    fn partial_configs_keep_unspecified_defaults() {
        let config = Config::parse(
            "[speeds]\nnormal = 900\n[keys]\nfree_mouse = 'KEY_F4'\nleft = 'w'\n[hints.keys]\nleft = 'leftmeta+j'",
        )
        .unwrap();
        assert_eq!(config.speeds.normal, 900.0);
        assert_eq!(config.speeds.fast, 900.0);
        assert_eq!(config.keys.free_mouse, vec![K::KEY_F4]);
        assert_eq!(config.keys.left, K::KEY_W);
        assert_eq!(config.keys.locate, K::KEY_Q);
        assert_eq!(config.keys.scroll_up, K::KEY_COMMA);
        assert_eq!(config.hints.keys.left, vec![configured_super(), K::KEY_J]);
        Config::parse("").unwrap();
    }

    #[test]
    fn invalid_values_and_unknown_keys_do_not_discard_valid_settings() {
        let config = Config::parse(
            "notify = false\nmdoe = 'toggle'\n[speeds]\nnormal = 450\nfast = 0\n[keys]\nleft = 'not-a-key'\nright = 'd'\n[hints]\nlabel_color = 'red'\nmin_width = 12",
        )
        .unwrap();

        assert!(!config.notify);
        assert_eq!(config.mode, Mode::Hold);
        assert_eq!(config.speeds.normal, 450.0);
        assert_eq!(config.speeds.fast, Speeds::default().fast);
        assert_eq!(config.keys.left, Keys::default().left);
        assert_eq!(config.keys.right, K::KEY_D);
        assert_eq!(config.hints.label_color, Hints::default().label_color);
        assert_eq!(config.hints.min_width, 12);

        let config = Config::parse("notify = false\n[hints.keys]\nleft = 'f8'").unwrap();
        assert!(!config.notify);
        assert_eq!(config.hints.keys.left, [configured_super(), K::KEY_SPACE]);
    }

    #[test]
    fn invalid_remap_entries_do_not_discard_valid_entries() {
        let config = Config::parse(
            "[remap]\nchord_timeout = 10\n[remap.main]\n'a+s' = 'ctrl'\n'b+d' = 'not-a-key'",
        )
        .unwrap();

        assert_eq!(config.remap.chord_timeout, 10);
        assert_eq!(config.remap.main.len(), 1);
        assert_eq!(config.remap.main["a+s"], "ctrl");
    }

    #[test]
    fn cursor_locator_key_is_configurable_and_cannot_conflict_with_controls() {
        assert_eq!(Config::parse("").unwrap().keys.locate, K::KEY_Q);
        assert_eq!(
            Config::parse("[keys]\nfree_mouse = 'f3'")
                .unwrap()
                .keys
                .locate,
            K::KEY_Q
        );
        assert_eq!(
            Config::parse("[keys]\nlocate = 'f9'").unwrap().keys.locate,
            K::KEY_F9
        );
        for binding in ["h", "f3", "z", "space"] {
            assert!(
                Config::parse(&format!("[keys]\nlocate = '{binding}'")).is_ok(),
                "{binding}"
            );
        }
        assert!(Config::parse("[keys]\nfree_mouse = 'q'").is_ok());
        assert!(Config::parse("[keys]\nlocater = 'f9'").is_ok());
        assert!(Config::parse("[keys]\nleft = 'q'").is_ok());
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
    fn modifier_aliases_work_in_all_config_bindings() {
        let config = Config::parse(
            "[keys]\nreload_config = 'ctrl + shift + r'\n\
             [hints.keys]\nleft = 'command + space'\ntoggle_background = 'option'",
        )
        .unwrap();
        assert_eq!(
            config.keys.reload_config,
            [K::KEY_LEFTCTRL, K::KEY_LEFTSHIFT, K::KEY_R]
        );
        assert_eq!(config.hints.keys.left, [configured_super(), K::KEY_SPACE]);
        assert_eq!(config.hints.keys.toggle_background, K::KEY_LEFTALT);

        for (alias, key) in [
            ("ctrl", K::KEY_LEFTCTRL),
            ("control", K::KEY_LEFTCTRL),
            ("super", K::KEY_LEFTMETA),
            ("meta", K::KEY_LEFTMETA),
            ("win", K::KEY_LEFTMETA),
            ("windows", K::KEY_LEFTMETA),
            ("cmd", K::KEY_LEFTMETA),
            ("command", K::KEY_LEFTMETA),
            ("alt", K::KEY_LEFTALT),
            ("option", K::KEY_LEFTALT),
            ("shift", K::KEY_LEFTSHIFT),
        ] {
            assert_eq!(parse_key(alias), Ok(key), "{alias}");
        }
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
        assert!(!config.hints.recursive_grid_enabled);
        assert_eq!(
            config.hints.keys.left,
            vec![configured_super(), K::KEY_SPACE]
        );
        assert_eq!(config.hints.keys.right, vec![configured_super(), K::KEY_I]);
        assert_eq!(config.hints.keys.grid, vec![configured_super(), K::KEY_G]);
        assert_eq!(config.hints.keys.grid_upper_left, K::KEY_U);
        assert_eq!(config.hints.keys.grid_upper_right, K::KEY_I);
        assert_eq!(config.hints.keys.grid_lower_left, K::KEY_J);
        assert_eq!(config.hints.keys.grid_lower_right, K::KEY_K);
        assert_eq!(config.hints.keys.grid_left_click, K::KEY_SPACE);
        assert_eq!(config.hints.keys.grid_right_click, K::KEY_R);
        assert_eq!(config.hints.keys.cancel, K::KEY_ENTER);
        assert_eq!(config.hints.keys.toggle_background, K::KEY_LEFTCTRL);
        assert_eq!(config.hints.keys.debug, K::KEY_F8);
        assert_eq!(config.hints.readability_color, Color::rgba(64, 64, 64, 230));
        assert_eq!(
            config.hints.grid_border_color(GridQuadrant::UpperLeft),
            config.hints.border_color
        );
        assert_eq!(
            config.hints.grid_fill_color(GridQuadrant::LowerRight),
            config.hints.fill_color
        );
        assert_eq!(config.hints.label_symbols, "abcdefghijklmnopqrstuvwxyz");
        assert_eq!(config.hints.max_height, 96);
        let config =
            Config::parse("[hints.keys]\ngrid_left_click = 'f1'\ngrid_right_click = 'f2'").unwrap();
        assert_eq!(config.hints.keys.grid_left_click, K::KEY_F1);
        assert_eq!(config.hints.keys.grid_right_click, K::KEY_F2);
        let config = Config::parse(
            "[hints.keys]\ngrid_upper_left = 'a'\ngrid_upper_right = 's'\n\
             grid_lower_left = 'd'\ngrid_lower_right = 'f'",
        )
        .unwrap();
        assert_eq!(config.hints.keys.grid_upper_left, K::KEY_A);
        assert_eq!(config.hints.keys.grid_upper_right, K::KEY_S);
        assert_eq!(config.hints.keys.grid_lower_left, K::KEY_D);
        assert_eq!(config.hints.keys.grid_lower_right, K::KEY_F);
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
            "[hints.keys]\ngrid = ''",
            "[hints.keys]\ngrid = 'leftmeta+leftmeta'",
            "[hints.keys]\ngrid_left_click = 'u'",
            "[hints.keys]\ngrid_right_click = 'space'\ngrid_left_click = 'space'",
            "[hints.keys]\ngrid_left_click = 'enter'",
            "[hints.keys]\ngrid_upper_left = 'u'\ngrid_upper_right = 'u'",
            "[hints.keys]\ngrid_lower_left = 'space'",
            "[hints.keys]\ngrid_upper_left = 'f1'",
            "[hints.keys]\ngrid_right_click = 'f3'\n[keys]\nfree_mouse = 'f3'",
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
            "[hints.keys]\nleft = 'leftmeta+space'\ngrid = 'leftmeta+space'",
            "[hints]\ncolor = '#00ff00'",
        ] {
            assert!(Config::parse(text).is_ok(), "{text}");
        }
    }

    #[test]
    fn removed_super_is_alt_setting_is_rejected_by_strict_config() {
        assert!(Config::parse_strict("super_is_alt = true").is_err());
        assert!(Config::parse_strict("win_super_is_alt = true").is_err());
    }

    #[test]
    fn recursive_grid_is_opt_in() {
        let defaults = Config::parse("").unwrap();
        assert!(!defaults.hints.recursive_grid_enabled);

        let enabled = Config::parse("[hints]\nrecursive_grid_enabled = true").unwrap();
        assert!(enabled.hints.recursive_grid_enabled);
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
        let grid = Config::parse(
            "[hints]\nborder_color = '#0000ffe0'\nfill_color = '#0000ff44'\n\
             [hints.grid_colors]\nupper_left = '#ff0000c0'\nlower_right = '#ffff0080'",
        )
        .unwrap();
        assert_eq!(
            grid.hints.grid_border_color(GridQuadrant::UpperLeft),
            Color::rgba(255, 0, 0, 192)
        );
        assert_eq!(
            grid.hints.grid_fill_color(GridQuadrant::UpperLeft),
            Color::rgba(255, 0, 0, 68)
        );
        assert_eq!(
            grid.hints.grid_border_color(GridQuadrant::UpperRight),
            Color::rgba(0, 0, 255, 224)
        );
        assert_eq!(
            grid.hints.grid_fill_color(GridQuadrant::UpperRight),
            Color::rgba(0, 0, 255, 68)
        );
        assert!(Config::parse(
            "[hints.keys]\ncancel = 'rightctrl'\ntoggle_background = 'rightctrl'"
        )
        .is_ok());
    }

    #[test]
    fn skips_invalid_speeds_bindings_and_typos() {
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
        ] {
            assert!(Config::parse(text).is_ok(), "{text}");
        }
        assert!(Config::parse("invalid toml").is_err());
    }

    #[test]
    fn missing_optional_file_defaults_but_required_file_errors() {
        let directory = std::env::temp_dir().join(format!(
            "fievel-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("nested/fievel.config");
        assert_eq!(
            Config::load(&path, false).unwrap().keys.free_mouse,
            vec![K::KEY_F3]
        );
        assert!(path.parent().unwrap().is_dir());
        assert!(Config::load(&path, true).is_err());
        assert!(Config::load(Path::new(env!("CARGO_MANIFEST_DIR")), false).is_err());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn creates_missing_default_config_and_preserves_existing_config() {
        let directory = std::env::temp_dir().join(format!(
            "fievel-default-config-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("nested/fievel.config");

        let config = Config::load_or_create_default(&path).unwrap();
        assert_eq!(config.keys.free_mouse, vec![K::KEY_F3]);
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            include_str!("../fievel.config")
        );

        fs::write(&path, "notify = false\n").unwrap();
        assert!(!Config::load_or_create_default(&path).unwrap().notify);

        fs::remove_dir_all(directory).unwrap();
    }
}
