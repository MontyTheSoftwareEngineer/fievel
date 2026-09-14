use crate::config::parse_key;
use evdev::KeyCode as K;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RemapConfig {
    pub chord_timeout: u64,
    pub main: BTreeMap<String, String>,
    pub layers: BTreeMap<String, BTreeMap<String, String>>,
}

impl Default for RemapConfig {
    fn default() -> Self {
        Self {
            chord_timeout: 25,
            main: BTreeMap::new(),
            layers: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    Key(K),
    Layer(String),
    Mouse,
    Timeout(Box<Action>, Duration, Box<Action>),
}

#[derive(Clone, Debug)]
struct Mapping {
    keys: Vec<K>,
    action: Action,
}

#[derive(Default)]
struct Parsed {
    main: Vec<Mapping>,
    layers: BTreeMap<String, Vec<Mapping>>,
}

fn action_key(text: &str) -> Result<K, String> {
    match text.trim().to_ascii_lowercase().as_str() {
        "ctrl" | "control" => Ok(K::KEY_LEFTCTRL),
        "super" | "meta" => Ok(K::KEY_LEFTMETA),
        "alt" => Ok(K::KEY_LEFTALT),
        "shift" => Ok(K::KEY_LEFTSHIFT),
        _ => parse_key(text),
    }
}

fn parse_action(text: &str, nested: bool) -> Result<Action, String> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("free_mouse") {
        return Ok(Action::Mouse);
    }
    if let Some(body) = text
        .strip_prefix("layer(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let name = body.trim();
        if matches!(name, "control" | "ctrl" | "meta" | "super") {
            return action_key(name).map(Action::Key);
        }
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return Err(format!("invalid layer action {text:?}"));
        }
        return Ok(Action::Layer(name.to_owned()));
    }
    if let Some(body) = text
        .strip_prefix("timeout(")
        .and_then(|s| s.strip_suffix(')'))
    {
        if nested {
            return Err("nested timeout actions are not supported".to_owned());
        }
        let mut depth = 0_i32;
        let mut start = 0;
        let mut parts = Vec::new();
        for (index, ch) in body.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    parts.push(body[start..index].trim());
                    start = index + 1;
                }
                _ => {}
            }
            if depth < 0 {
                return Err(format!("invalid timeout action {text:?}"));
            }
        }
        parts.push(body[start..].trim());
        if depth != 0 || parts.len() != 3 {
            return Err(format!(
                "timeout requires (short_action, milliseconds, held_action): {text:?}"
            ));
        }
        let milliseconds = parse_milliseconds(parts[1])?;
        return Ok(Action::Timeout(
            Box::new(parse_action(parts[0], true)?),
            Duration::from_millis(milliseconds),
            Box::new(parse_action(parts[2], true)?),
        ));
    }
    action_key(text).map(Action::Key)
}

fn parse_milliseconds(text: &str) -> Result<u64, String> {
    if text.is_empty() || !text.bytes().all(|c| c.is_ascii_digit()) {
        return Err(format!("invalid timeout milliseconds {text:?}"));
    }
    let value = text
        .parse::<u64>()
        .map_err(|_| format!("invalid timeout milliseconds {text:?}"))?;
    if value == 0 {
        return Err("timeout milliseconds must be greater than zero".to_owned());
    }
    Ok(value)
}

fn parse_table(table: &BTreeMap<String, String>) -> Result<Vec<Mapping>, String> {
    let mut mappings: Vec<Mapping> = Vec::new();
    for (trigger, text) in table {
        let mut keys = trigger
            .split('+')
            .map(action_key)
            .collect::<Result<Vec<_>, _>>()?;
        keys.sort_unstable();
        if keys.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(format!("repeated key in trigger {trigger:?}"));
        }
        if mappings.iter().any(|mapping| mapping.keys == keys) {
            return Err(format!("duplicate normalized trigger {trigger:?}"));
        }
        mappings.push(Mapping {
            keys,
            action: parse_action(text, false)?,
        });
    }
    Ok(mappings)
}

impl RemapConfig {
    fn parsed(&self) -> Result<Parsed, String> {
        if self.chord_timeout == 0 {
            return Err("remap.chord_timeout must be greater than zero".to_owned());
        }
        let parsed = Parsed {
            main: parse_table(&self.main)?,
            layers: self
                .layers
                .iter()
                .map(|(name, table)| {
                    if name.is_empty()
                        || !name
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                        || matches!(name.as_str(), "control" | "ctrl" | "meta" | "super")
                    {
                        return Err(format!("invalid or reserved layer name {name:?}"));
                    }
                    Ok((name.clone(), parse_table(table)?))
                })
                .collect::<Result<_, _>>()?,
        };
        fn check_action(action: &Action, parsed: &Parsed) -> Result<(), String> {
            match action {
                Action::Layer(name) if !parsed.layers.contains_key(name) => {
                    Err(format!("undefined remap layer {name:?}"))
                }
                Action::Timeout(short, _, held) => {
                    check_action(short, parsed)?;
                    check_action(held, parsed)
                }
                _ => Ok(()),
            }
        }
        let all: Vec<_> = parsed
            .main
            .iter()
            .chain(parsed.layers.values().flatten())
            .collect();
        for mapping in &all {
            check_action(&mapping.action, &parsed)?;
            // Single-key fallbacks are intentional; strict chord subsets are ambiguous.
            if mapping.keys.len() > 1
                && all.iter().any(|other| {
                    mapping.keys.len() < other.keys.len()
                        && mapping.keys.iter().all(|key| other.keys.contains(key))
                })
            {
                return Err("chords cannot be strict subsets of other chords".to_owned());
            }
        }
        Ok(parsed)
    }

    pub fn validate(&self) -> Result<(), String> {
        self.parsed().map(|_| ())
    }

    pub fn input_keys(&self) -> Result<Vec<K>, String> {
        let parsed = self.parsed()?;
        Ok(parsed
            .main
            .iter()
            .chain(parsed.layers.values().flatten())
            .flat_map(|mapping| mapping.keys.iter().copied())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }

    pub fn output_keys(&self) -> Result<Vec<K>, String> {
        fn collect(action: &Action, keys: &mut BTreeSet<K>) {
            match action {
                Action::Key(key) => {
                    keys.insert(*key);
                }
                Action::Timeout(short, _, held) => {
                    collect(short, keys);
                    collect(held, keys);
                }
                _ => {}
            }
        }
        let parsed = self.parsed()?;
        let mut keys = BTreeSet::new();
        for mapping in parsed.main.iter().chain(parsed.layers.values().flatten()) {
            collect(&mapping.action, &mut keys);
        }
        Ok(keys.into_iter().collect())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    Key(K, i32),
    Mouse(bool),
}

struct Pending {
    key: K,
    at: Duration,
}

enum BindingAction {
    Active(Action),
    Waiting {
        short: Action,
        held: Action,
        deadline: Duration,
    },
    Released,
}

struct Binding {
    keys: BTreeSet<K>,
    action: BindingAction,
}

pub struct Remapper {
    config: Parsed,
    chord_timeout: Duration,
    now: Duration,
    pressed: BTreeSet<K>,
    pending: Vec<Pending>,
    bindings: BTreeMap<u64, Binding>,
    next_binding: u64,
    emitted: BTreeMap<K, usize>,
    layers: Vec<(String, usize)>,
    mouse_count: usize,
    mouse_active: bool,
    mouse_controls: BTreeSet<K>,
}

impl Remapper {
    pub fn new(config: RemapConfig) -> Result<Self, String> {
        Ok(Self {
            config: config.parsed()?,
            chord_timeout: Duration::from_millis(config.chord_timeout),
            now: Duration::ZERO,
            pressed: BTreeSet::new(),
            pending: Vec::new(),
            bindings: BTreeMap::new(),
            next_binding: 0,
            emitted: BTreeMap::new(),
            layers: Vec::new(),
            mouse_count: 0,
            mouse_active: false,
            mouse_controls: BTreeSet::new(),
        })
    }

    pub fn for_shortcuts(config: RemapConfig, keys: &[K]) -> Result<Self, String> {
        fn relevant(action: &Action, keys: &[K], layers: &BTreeSet<String>) -> bool {
            match action {
                Action::Key(key) => keys.contains(key),
                Action::Layer(name) => layers.contains(name),
                Action::Timeout(short, _, held) => {
                    relevant(short, keys, layers) || relevant(held, keys, layers)
                }
                Action::Mouse => false,
            }
        }

        let mut remapper = Self::new(config)?;
        let mut layers = BTreeSet::new();
        loop {
            let previous = layers.len();
            for (name, mappings) in &remapper.config.layers {
                if mappings
                    .iter()
                    .any(|mapping| relevant(&mapping.action, keys, &layers))
                {
                    layers.insert(name.clone());
                }
            }
            if layers.len() == previous {
                break;
            }
        }
        remapper
            .config
            .main
            .retain(|mapping| relevant(&mapping.action, keys, &layers));
        for mappings in remapper.config.layers.values_mut() {
            mappings.retain(|mapping| relevant(&mapping.action, keys, &layers));
        }
        Ok(remapper)
    }

    pub fn set_mouse_controls(&mut self, keys: Vec<K>) {
        self.mouse_controls = keys.into_iter().collect();
    }

    pub fn set_mouse_active(&mut self, active: bool) {
        self.mouse_active = active;
    }

    pub fn clear(&mut self) {
        self.now = Duration::ZERO;
        self.pressed.clear();
        self.pending.clear();
        self.bindings.clear();
        self.emitted.clear();
        self.layers.clear();
        self.mouse_count = 0;
        self.mouse_active = false;
        self.next_binding = 0;
    }

    fn mappings(&self) -> Vec<&Mapping> {
        let mut seen = BTreeSet::new();
        let mut result = Vec::new();
        for mapping in self
            .layers
            .iter()
            .rev()
            .filter_map(|(name, _)| self.config.layers.get(name))
            .flatten()
            .chain(self.config.main.iter())
        {
            if self.allowed(mapping) && seen.insert(&mapping.keys) {
                result.push(mapping);
            }
        }
        result
    }

    fn allowed(&self, mapping: &Mapping) -> bool {
        !self.mouse_active
            || !mapping
                .keys
                .iter()
                .any(|key| self.mouse_controls.contains(key))
            || mapping.action == Action::Mouse
    }

    fn single_action(&self, key: K) -> Action {
        self.mappings()
            .into_iter()
            .find(|mapping| mapping.keys == [key] && self.allowed(mapping))
            .map(|mapping| mapping.action.clone())
            .unwrap_or(Action::Key(key))
    }

    fn emit(&mut self, event: Event, emit: &mut dyn FnMut(Event) -> bool) {
        let active = emit(event);
        self.set_mouse_active(active);
    }

    fn activate(&mut self, action: &Action, emit: &mut dyn FnMut(Event) -> bool) {
        match action {
            Action::Key(key) => {
                let count = self.emitted.entry(*key).or_default();
                let first = *count == 0;
                *count += 1;
                if first {
                    self.emit(Event::Key(*key, 1), emit);
                }
            }
            Action::Layer(name) => {
                if let Some((_, count)) = self.layers.iter_mut().find(|(layer, _)| layer == name) {
                    *count += 1;
                } else {
                    self.layers.push((name.clone(), 1));
                }
            }
            Action::Mouse => {
                let first = self.mouse_count == 0;
                self.mouse_count += 1;
                if first {
                    self.emit(Event::Mouse(true), emit);
                }
            }
            Action::Timeout(..) => {}
        }
    }

    fn deactivate(&mut self, action: &Action, emit: &mut dyn FnMut(Event) -> bool) {
        match action {
            Action::Key(key) => {
                if let Some(count) = self.emitted.get_mut(key) {
                    *count -= 1;
                    if *count == 0 {
                        self.emitted.remove(key);
                        self.emit(Event::Key(*key, 0), emit);
                    }
                }
            }
            Action::Layer(name) => {
                if let Some(index) = self.layers.iter().position(|(layer, _)| layer == name) {
                    self.layers[index].1 -= 1;
                    if self.layers[index].1 == 0 {
                        self.layers.remove(index);
                    }
                }
            }
            Action::Mouse => {
                self.mouse_count = self.mouse_count.saturating_sub(1);
                if self.mouse_count == 0 {
                    self.emit(Event::Mouse(false), emit);
                }
            }
            Action::Timeout(..) => {}
        }
    }

    fn bind(
        &mut self,
        keys: Vec<K>,
        action: Action,
        at: Duration,
        emit: &mut dyn FnMut(Event) -> bool,
    ) {
        let action = match action {
            Action::Timeout(short, duration, held) => {
                let deadline = at.saturating_add(duration);
                if self.now >= deadline {
                    self.activate(&held, emit);
                    BindingAction::Active(*held)
                } else {
                    BindingAction::Waiting {
                        short: *short,
                        held: *held,
                        deadline,
                    }
                }
            }
            action => {
                self.activate(&action, emit);
                BindingAction::Active(action)
            }
        };
        let id = self.next_binding;
        self.next_binding = self.next_binding.wrapping_add(1);
        self.bindings.insert(
            id,
            Binding {
                keys: keys.into_iter().collect(),
                action,
            },
        );
    }

    fn resolve_waiting(&mut self, interrupted: bool, emit: &mut dyn FnMut(Event) -> bool) {
        let ready: Vec<_> = self
            .bindings
            .iter()
            .filter_map(|(id, binding)| match &binding.action {
                BindingAction::Waiting {
                    short,
                    held,
                    deadline,
                } => {
                    if self.now >= *deadline {
                        Some((*id, held.clone()))
                    } else if interrupted {
                        Some((*id, short.clone()))
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .collect();
        for (id, action) in ready {
            self.activate(&action, emit);
            if let Some(binding) = self.bindings.get_mut(&id) {
                binding.action = BindingAction::Active(action);
            }
        }
    }

    fn flush_pending(&mut self, emit: &mut dyn FnMut(Event) -> bool) {
        let pending = std::mem::take(&mut self.pending);
        let now = self.now;
        for pending in pending {
            // Replay at the original press times: a delayed flush must not turn
            // a quick timeout-key shortcut into its held action.
            self.now = pending.at;
            self.resolve_waiting(true, emit);
            let action = self.single_action(pending.key);
            self.bind(vec![pending.key], action, pending.at, emit);
        }
        self.now = now;
        self.resolve_waiting(false, emit);
    }

    fn expire(&mut self, emit: &mut dyn FnMut(Event) -> bool) {
        self.resolve_waiting(false, emit);
        if self
            .pending
            .first()
            .is_some_and(|pending| self.now >= pending.at.saturating_add(self.chord_timeout))
        {
            self.flush_pending(emit);
        }
    }

    pub fn advance_with(&mut self, elapsed: Duration, emit: &mut dyn FnMut(Event) -> bool) {
        self.now = self.now.saturating_add(elapsed);
        self.expire(emit);
    }

    pub fn key_with(&mut self, key: K, value: i32, emit: &mut dyn FnMut(Event) -> bool) {
        self.expire(emit);
        match value {
            1 if !self.pressed.contains(&key) => {
                self.resolve_waiting(true, emit);
                // Only one still-possible chord is buffered. An unrelated press commits
                // its predecessors first, so replayed modifiers affect that press.
                loop {
                    let mut keys: Vec<_> = self.pending.iter().map(|pending| pending.key).collect();
                    keys.push(key);
                    let candidate = self.mappings().into_iter().find(|mapping| {
                        mapping.keys.len() > 1
                            && self.allowed(mapping)
                            && keys.iter().all(|key| mapping.keys.contains(key))
                            && mapping.keys.iter().all(|member| {
                                !self.pressed.contains(member)
                                    || self.pending.iter().any(|pending| pending.key == *member)
                            })
                    });
                    if let Some(mapping) = candidate {
                        let complete = mapping.keys.len() == keys.len();
                        let action = mapping.action.clone();
                        self.pressed.insert(key);
                        self.pending.push(Pending { key, at: self.now });
                        if complete {
                            let at = self.now;
                            self.pending.clear();
                            self.bind(keys, action, at, emit);
                        }
                        break;
                    }
                    if self.pending.is_empty() {
                        self.resolve_waiting(true, emit);
                        let action = self.single_action(key);
                        self.pressed.insert(key);
                        self.bind(vec![key], action, self.now, emit);
                        break;
                    }
                    self.flush_pending(emit);
                    self.resolve_waiting(true, emit);
                }
            }
            0 if self.pressed.contains(&key) => {
                // Earlier presses must reach the output before this release,
                // especially when it releases a modifier used by those presses.
                self.flush_pending(emit);
                self.pressed.remove(&key);
                let id = self
                    .bindings
                    .iter()
                    .find_map(|(id, binding)| binding.keys.contains(&key).then_some(*id));
                if let Some(id) = id {
                    if let Some(mut binding) = self.bindings.remove(&id) {
                        match &binding.action {
                            BindingAction::Active(action) => self.deactivate(action, emit),
                            BindingAction::Waiting { short, .. } => {
                                self.activate(short, emit);
                                self.deactivate(short, emit);
                            }
                            BindingAction::Released => {}
                        }
                        binding.keys.remove(&key);
                        if !binding.keys.is_empty() {
                            binding.action = BindingAction::Released;
                            self.bindings.insert(id, binding);
                        }
                    }
                }
            }
            2 => {
                if let Some(output) = self.bindings.values().find_map(|binding| {
                    if binding.keys.contains(&key) {
                        if let BindingAction::Active(Action::Key(output)) = binding.action {
                            return Some(output);
                        }
                    }
                    None
                }) {
                    self.emit(Event::Key(output, 2), emit);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Remapper {
        fn collect_events(
            &mut self,
            process: impl FnOnce(&mut Self, &mut dyn FnMut(Event) -> bool),
        ) -> Vec<Event> {
            let mut events = Vec::new();
            let mut active = self.mouse_active;
            process(self, &mut |event| {
                events.push(event);
                // Simple collector for state-machine tests; feedback regressions
                // below supply an actual Engine instead.
                if event == Event::Mouse(true) {
                    active = true;
                }
                active
            });
            events
        }

        fn key(&mut self, key: K, value: i32) -> Vec<Event> {
            self.collect_events(|remapper, emit| remapper.key_with(key, value, emit))
        }

        fn advance(&mut self, elapsed: Duration) -> Vec<Event> {
            self.collect_events(|remapper, emit| remapper.advance_with(elapsed, emit))
        }
    }

    const EXAMPLE: &str = r#"
        chord_timeout = 25
        [main]
        'a+s' = 'super'
        's+d' = 'ctrl'
        'i+o' = 'ctrl'
        'u+i' = 'super'
        'a+f' = 'escape'
        'f+s' = 'free_mouse'
        'w+e' = 'layer(nav)'
        capslock = 'timeout(ctrl, 175, layer(nav))'
        [layers.nav]
        h = 'left'
        j = 'down'
        k = 'up'
        l = 'right'
    "#;

    fn config(text: &str) -> RemapConfig {
        let config: RemapConfig = toml::from_str(text).unwrap();
        config.validate().unwrap();
        config
    }

    fn remap(text: &str) -> Remapper {
        Remapper::new(config(text)).unwrap()
    }

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn default_is_immediate_passthrough_with_repeat() {
        let mut r = Remapper::new(RemapConfig::default()).unwrap();
        assert_eq!(r.key(K::KEY_A, 1), [Event::Key(K::KEY_A, 1)]);
        assert_eq!(r.key(K::KEY_A, 2), [Event::Key(K::KEY_A, 2)]);
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert_eq!(r.key(K::KEY_A, 0), [Event::Key(K::KEY_A, 0)]);
        assert!(r.key(K::KEY_A, 0).is_empty());
        assert!(r.key(K::KEY_A, 2).is_empty());
    }

    #[test]
    fn exact_example_all_chords_work_in_both_orders() {
        for (a, b, output) in [
            (K::KEY_A, K::KEY_S, Event::Key(K::KEY_LEFTMETA, 1)),
            (K::KEY_S, K::KEY_D, Event::Key(K::KEY_LEFTCTRL, 1)),
            (K::KEY_I, K::KEY_O, Event::Key(K::KEY_LEFTCTRL, 1)),
            (K::KEY_U, K::KEY_I, Event::Key(K::KEY_LEFTMETA, 1)),
            (K::KEY_A, K::KEY_F, Event::Key(K::KEY_ESC, 1)),
            (K::KEY_F, K::KEY_S, Event::Mouse(true)),
        ] {
            for (first, second) in [(a, b), (b, a)] {
                let mut r = remap(EXAMPLE);
                assert!(r.key(first, 1).is_empty());
                assert!(r.advance(ms(24)).is_empty());
                assert_eq!(r.key(second, 1), [output]);
                let release = match output {
                    Event::Key(key, _) => Event::Key(key, 0),
                    Event::Mouse(_) => Event::Mouse(false),
                };
                assert_eq!(r.key(first, 0), [release]);
                assert!(r.key(second, 0).is_empty());
            }
        }
    }

    #[test]
    fn keyd_alias_example_includes_timeout_and_legacy_f3_chord() {
        let text = EXAMPLE
            .replace("'ctrl'", "'layer(control)'")
            .replace("'super'", "'layer(meta)'")
            .replace(
                "timeout(ctrl, 175, layer(nav))",
                "timeout(layer(control),175,layer(nav))",
            );
        let mut cfg = config(&text);
        cfg.main.insert("d+f".to_owned(), "f3".to_owned());
        cfg.validate().unwrap();
        assert!(cfg.output_keys().unwrap().contains(&K::KEY_F3));
        for (a, b, output) in [
            (K::KEY_A, K::KEY_S, K::KEY_LEFTMETA),
            (K::KEY_S, K::KEY_D, K::KEY_LEFTCTRL),
            (K::KEY_I, K::KEY_O, K::KEY_LEFTCTRL),
            (K::KEY_U, K::KEY_I, K::KEY_LEFTMETA),
            (K::KEY_A, K::KEY_F, K::KEY_ESC),
            (K::KEY_D, K::KEY_F, K::KEY_F3),
        ] {
            for (first, second) in [(a, b), (b, a)] {
                let mut r = Remapper::new(cfg.clone()).unwrap();
                assert!(r.key(first, 1).is_empty());
                assert_eq!(r.key(second, 1), [Event::Key(output, 1)]);
                assert_eq!(r.key(second, 0), [Event::Key(output, 0)]);
                assert!(r.key(first, 0).is_empty());
            }
        }
        let mut r = Remapper::new(cfg).unwrap();
        assert!(r.key(K::KEY_CAPSLOCK, 1).is_empty());
        assert_eq!(
            r.key(K::KEY_C, 1),
            [Event::Key(K::KEY_LEFTCTRL, 1), Event::Key(K::KEY_C, 1)]
        );
        assert_eq!(r.key(K::KEY_CAPSLOCK, 0), [Event::Key(K::KEY_LEFTCTRL, 0)]);
        assert!(r.key(K::KEY_CAPSLOCK, 1).is_empty());
        assert!(r.advance(ms(175)).is_empty());
        assert_eq!(r.key(K::KEY_H, 1), [Event::Key(K::KEY_LEFT, 1)]);
    }

    #[test]
    fn chord_deadline_requires_fresh_presses() {
        let mut r = remap(EXAMPLE);
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert_eq!(r.advance(ms(25)), [Event::Key(K::KEY_A, 1)]);
        // A is expired, but S can still begin its other fresh chords.
        assert!(r.key(K::KEY_S, 1).is_empty());
        assert_eq!(r.advance(ms(25)), [Event::Key(K::KEY_S, 1)]);
        assert_eq!(r.key(K::KEY_A, 0), [Event::Key(K::KEY_A, 0)]);
        assert_eq!(r.key(K::KEY_S, 0), [Event::Key(K::KEY_S, 0)]);
        assert!(r.key(K::KEY_S, 1).is_empty());
        assert_eq!(r.key(K::KEY_A, 1), [Event::Key(K::KEY_LEFTMETA, 1)]);
    }

    #[test]
    fn failed_chord_replays_tap_and_modifier_before_following_key() {
        let mut r = remap("[main]\na='ctrl'\n'a+s'='escape'");
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert_eq!(
            r.key(K::KEY_A, 0),
            [
                Event::Key(K::KEY_LEFTCTRL, 1),
                Event::Key(K::KEY_LEFTCTRL, 0),
            ]
        );
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert_eq!(
            r.key(K::KEY_C, 1),
            [Event::Key(K::KEY_LEFTCTRL, 1), Event::Key(K::KEY_C, 1),]
        );
        assert_eq!(r.key(K::KEY_A, 0), [Event::Key(K::KEY_LEFTCTRL, 0)]);
    }

    #[test]
    fn modifier_release_flushes_earlier_pending_press_first() {
        let mut r = remap("[main]\n'a+f'='escape'");
        assert_eq!(r.key(K::KEY_LEFTCTRL, 1), [Event::Key(K::KEY_LEFTCTRL, 1)]);
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert_eq!(
            r.key(K::KEY_LEFTCTRL, 0),
            [Event::Key(K::KEY_A, 1), Event::Key(K::KEY_LEFTCTRL, 0)]
        );
        assert_eq!(r.key(K::KEY_A, 0), [Event::Key(K::KEY_A, 0)]);
        assert!(r.advance(ms(25)).is_empty());
    }

    #[test]
    fn chord_modifier_release_flushes_pending_press_with_shared_counts() {
        for shared in [false, true] {
            let mut r = remap(EXAMPLE);
            assert!(r.key(K::KEY_S, 1).is_empty());
            assert_eq!(r.key(K::KEY_D, 1), [Event::Key(K::KEY_LEFTCTRL, 1)]);
            if shared {
                assert!(r.key(K::KEY_I, 1).is_empty());
                assert!(r.key(K::KEY_O, 1).is_empty());
            }
            assert!(r.key(K::KEY_A, 1).is_empty());
            let mut expected = vec![Event::Key(K::KEY_A, 1)];
            if !shared {
                expected.push(Event::Key(K::KEY_LEFTCTRL, 0));
            }
            assert_eq!(r.key(K::KEY_D, 0), expected);
            if shared {
                assert_eq!(r.key(K::KEY_I, 0), [Event::Key(K::KEY_LEFTCTRL, 0)]);
                assert!(r.key(K::KEY_O, 0).is_empty());
            }
            assert!(r.key(K::KEY_S, 0).is_empty());
            assert_eq!(r.key(K::KEY_A, 0), [Event::Key(K::KEY_A, 0)]);
        }
    }

    #[test]
    fn overlapping_chords_choose_first_completion_and_reserve_members() {
        let mut r = remap(EXAMPLE);
        assert!(r.key(K::KEY_S, 1).is_empty());
        assert_eq!(r.key(K::KEY_A, 1), [Event::Key(K::KEY_LEFTMETA, 1)]);
        assert_eq!(r.key(K::KEY_D, 1), [Event::Key(K::KEY_D, 1)]);
        assert_eq!(r.key(K::KEY_A, 0), [Event::Key(K::KEY_LEFTMETA, 0)]);
        assert!(r.key(K::KEY_S, 2).is_empty());
        assert!(r.key(K::KEY_A, 1).is_empty()); // Can begin A+F, but not A+held S.
        assert_eq!(r.advance(ms(25)), [Event::Key(K::KEY_A, 1)]);
        assert!(r.key(K::KEY_S, 0).is_empty());
    }

    #[test]
    fn three_key_chord_failure_replays_in_press_order() {
        let mut r = remap("[main]\n'a+s+d'='escape'\na='ctrl'\ns='shift'");
        assert!(r.key(K::KEY_S, 1).is_empty());
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert_eq!(
            r.key(K::KEY_C, 1),
            [
                Event::Key(K::KEY_LEFTSHIFT, 1),
                Event::Key(K::KEY_LEFTCTRL, 1),
                Event::Key(K::KEY_C, 1),
            ]
        );
        let mut r = remap("[main]\n'a+s+d'='escape'");
        assert!(r.key(K::KEY_D, 1).is_empty());
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert_eq!(r.key(K::KEY_S, 1), [Event::Key(K::KEY_ESC, 1)]);
        assert_eq!(r.key(K::KEY_A, 0), [Event::Key(K::KEY_ESC, 0)]);
        assert!(r.key(K::KEY_D, 0).is_empty());
        assert!(r.key(K::KEY_S, 0).is_empty());
    }

    #[test]
    fn timeout_tap_interrupt_and_hold_boundary() {
        let mut r = remap(EXAMPLE);
        assert!(r.key(K::KEY_CAPSLOCK, 1).is_empty());
        assert!(r.advance(ms(174)).is_empty());
        assert_eq!(
            r.key(K::KEY_CAPSLOCK, 0),
            [
                Event::Key(K::KEY_LEFTCTRL, 1),
                Event::Key(K::KEY_LEFTCTRL, 0),
            ]
        );
        assert!(r.key(K::KEY_CAPSLOCK, 1).is_empty());
        assert_eq!(
            r.key(K::KEY_C, 1),
            [Event::Key(K::KEY_LEFTCTRL, 1), Event::Key(K::KEY_C, 1),]
        );
        assert!(r.advance(ms(500)).is_empty());
        assert_eq!(r.key(K::KEY_H, 1), [Event::Key(K::KEY_H, 1)]);
        assert_eq!(r.key(K::KEY_CAPSLOCK, 0), [Event::Key(K::KEY_LEFTCTRL, 0)]);

        let mut r = remap(EXAMPLE);
        assert!(r.key(K::KEY_CAPSLOCK, 1).is_empty());
        assert!(r.advance(ms(175)).is_empty());
        assert_eq!(r.key(K::KEY_H, 1), [Event::Key(K::KEY_LEFT, 1)]);
        assert!(r.key(K::KEY_CAPSLOCK, 0).is_empty());
        assert_eq!(r.key(K::KEY_H, 0), [Event::Key(K::KEY_LEFT, 0)]);
        assert_eq!(r.key(K::KEY_H, 1), [Event::Key(K::KEY_H, 1)]);
    }

    #[test]
    fn repeats_do_not_interrupt_timeouts_or_repeat_layer_mouse_actions() {
        let mut r = remap("[main]\na='timeout(ctrl, 50, super)'\ns='free_mouse'");
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert!(r.key(K::KEY_A, 2).is_empty());
        assert_eq!(r.advance(ms(50)), [Event::Key(K::KEY_LEFTMETA, 1)]);
        assert_eq!(r.key(K::KEY_A, 2), [Event::Key(K::KEY_LEFTMETA, 2)]);
        assert_eq!(r.key(K::KEY_A, 0), [Event::Key(K::KEY_LEFTMETA, 0)]);
        assert_eq!(r.key(K::KEY_S, 1), [Event::Mouse(true)]);
        assert!(r.key(K::KEY_S, 2).is_empty());
        assert_eq!(r.key(K::KEY_S, 0), [Event::Mouse(false)]);
    }

    #[test]
    fn timeout_interruption_precedes_buffered_chord() {
        let mut r = remap(EXAMPLE);
        assert!(r.key(K::KEY_CAPSLOCK, 1).is_empty());
        assert_eq!(r.key(K::KEY_A, 1), [Event::Key(K::KEY_LEFTCTRL, 1)]);
        assert_eq!(r.key(K::KEY_S, 1), [Event::Key(K::KEY_LEFTMETA, 1)]);
        assert!(r.advance(ms(200)).is_empty());
        assert_eq!(r.key(K::KEY_CAPSLOCK, 0), [Event::Key(K::KEY_LEFTCTRL, 0)]);
    }

    #[test]
    fn delayed_replay_preserves_timeout_interruption_timestamps() {
        let mut r = remap("[main]\n'a+s+d'='escape'\na='timeout(ctrl, 15, super)'");
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert!(r.advance(ms(10)).is_empty());
        assert!(r.key(K::KEY_S, 1).is_empty());
        assert_eq!(
            r.advance(ms(100)),
            [Event::Key(K::KEY_LEFTCTRL, 1), Event::Key(K::KEY_S, 1),]
        );
        assert_eq!(r.key(K::KEY_A, 0), [Event::Key(K::KEY_LEFTCTRL, 0)]);

        let mut r = remap("[main]\n'a+s'='escape'\na='timeout(ctrl, 15, super)'");
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert_eq!(r.advance(ms(25)), [Event::Key(K::KEY_LEFTMETA, 1)]);
        assert_eq!(r.key(K::KEY_A, 0), [Event::Key(K::KEY_LEFTMETA, 0)]);
    }

    #[test]
    fn layer_chord_falls_through_and_held_bindings_survive_layer_release() {
        for (a, b) in [(K::KEY_W, K::KEY_E), (K::KEY_E, K::KEY_W)] {
            let mut r = remap(EXAMPLE);
            assert!(r.key(a, 1).is_empty());
            assert!(r.key(b, 1).is_empty());
            assert!(r.key(a, 2).is_empty());
            assert_eq!(r.key(K::KEY_H, 1), [Event::Key(K::KEY_LEFT, 1)]);
            assert_eq!(r.key(K::KEY_C, 1), [Event::Key(K::KEY_C, 1)]);
            assert!(r.key(K::KEY_A, 1).is_empty());
            assert_eq!(r.key(K::KEY_S, 1), [Event::Key(K::KEY_LEFTMETA, 1)]);
            assert!(r.key(a, 0).is_empty());
            assert_eq!(r.key(K::KEY_H, 2), [Event::Key(K::KEY_LEFT, 2)]);
            assert_eq!(r.key(K::KEY_H, 0), [Event::Key(K::KEY_LEFT, 0)]);
        }
    }

    #[test]
    fn keys_layers_and_mouse_use_reference_counts() {
        let mut r = remap("[main]\na='ctrl'\ns='ctrl'\nd='free_mouse'\nf='free_mouse'");
        assert_eq!(r.key(K::KEY_LEFTCTRL, 1), [Event::Key(K::KEY_LEFTCTRL, 1)]);
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert!(r.key(K::KEY_S, 1).is_empty());
        assert!(r.key(K::KEY_LEFTCTRL, 0).is_empty());
        assert!(r.key(K::KEY_A, 0).is_empty());
        assert_eq!(r.key(K::KEY_S, 0), [Event::Key(K::KEY_LEFTCTRL, 0)]);
        assert_eq!(r.key(K::KEY_D, 1), [Event::Mouse(true)]);
        assert!(r.key(K::KEY_F, 1).is_empty());
        assert!(r.key(K::KEY_D, 0).is_empty());
        assert_eq!(r.key(K::KEY_F, 0), [Event::Mouse(false)]);

        let mut r = remap("[main]\na='layer(nav)'\ns='layer(nav)'\n[layers.nav]\nh='left'");
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert!(r.key(K::KEY_S, 1).is_empty());
        assert!(r.key(K::KEY_A, 0).is_empty());
        assert_eq!(r.key(K::KEY_H, 1), [Event::Key(K::KEY_LEFT, 1)]);
        assert!(r.key(K::KEY_S, 0).is_empty());
        assert_eq!(r.key(K::KEY_H, 0), [Event::Key(K::KEY_LEFT, 0)]);
    }

    #[test]
    fn overlapping_chord_modifier_producers_do_not_release_early() {
        let mut r = remap(EXAMPLE);
        assert!(r.key(K::KEY_S, 1).is_empty());
        assert_eq!(r.key(K::KEY_D, 1), [Event::Key(K::KEY_LEFTCTRL, 1)]);
        assert!(r.key(K::KEY_I, 1).is_empty());
        assert!(r.key(K::KEY_O, 1).is_empty());
        assert!(r.key(K::KEY_D, 0).is_empty());
        assert_eq!(r.key(K::KEY_I, 0), [Event::Key(K::KEY_LEFTCTRL, 0)]);
        assert!(r.key(K::KEY_S, 0).is_empty());
        assert!(r.key(K::KEY_O, 0).is_empty());
    }

    #[test]
    fn mouse_controls_bypass_chords_and_layers_except_direct_exit_chord() {
        let mut r = remap(EXAMPLE);
        r.set_mouse_controls(vec![K::KEY_I, K::KEY_H, K::KEY_S]);
        r.set_mouse_active(true);
        assert_eq!(r.key(K::KEY_I, 1), [Event::Key(K::KEY_I, 1)]);
        assert_eq!(r.key(K::KEY_O, 1), [Event::Key(K::KEY_O, 1)]);
        assert!(r.key(K::KEY_S, 1).is_empty()); // S+F still exits mouse mode.
        assert_eq!(r.key(K::KEY_F, 1), [Event::Mouse(true)]);
        r.set_mouse_active(false);
        assert_eq!(r.key(K::KEY_I, 0), [Event::Key(K::KEY_I, 0)]);
        assert_eq!(r.key(K::KEY_F, 0), [Event::Mouse(false)]);
        assert!(r.key(K::KEY_S, 0).is_empty());

        let mut r = remap(EXAMPLE);
        r.set_mouse_controls(vec![K::KEY_H]);
        assert!(r.key(K::KEY_CAPSLOCK, 1).is_empty());
        assert!(r.advance(ms(175)).is_empty());
        r.set_mouse_active(true);
        assert_eq!(r.key(K::KEY_H, 1), [Event::Key(K::KEY_H, 1)]);
        assert_eq!(r.key(K::KEY_J, 1), [Event::Key(K::KEY_DOWN, 1)]);
        r.set_mouse_active(false);
        assert_eq!(r.key(K::KEY_H, 0), [Event::Key(K::KEY_H, 0)]);
    }

    #[test]
    fn held_remapping_releases_original_output_after_mouse_activation() {
        let mut r = remap(EXAMPLE);
        r.set_mouse_controls(vec![K::KEY_I]);
        assert!(r.key(K::KEY_I, 1).is_empty());
        assert_eq!(r.key(K::KEY_O, 1), [Event::Key(K::KEY_LEFTCTRL, 1)]);
        r.set_mouse_active(true);
        assert_eq!(r.key(K::KEY_I, 0), [Event::Key(K::KEY_LEFTCTRL, 0)]);
        assert!(r.key(K::KEY_O, 0).is_empty());
        assert_eq!(r.key(K::KEY_I, 1), [Event::Key(K::KEY_I, 1)]);
    }

    #[test]
    fn mouse_speed_control_without_exit_chord_is_never_buffered() {
        let mut r = remap("[main]\n's+d'='ctrl'\ni='escape'\n'i+o'='super'");
        r.set_mouse_controls(vec![K::KEY_S, K::KEY_I]);
        r.set_mouse_active(true);
        assert_eq!(r.key(K::KEY_S, 1), [Event::Key(K::KEY_S, 1)]);
        assert_eq!(r.key(K::KEY_S, 2), [Event::Key(K::KEY_S, 2)]);
        assert_eq!(r.key(K::KEY_D, 1), [Event::Key(K::KEY_D, 1)]);
        assert_eq!(r.key(K::KEY_I, 1), [Event::Key(K::KEY_I, 1)]);
        assert!(r.advance(ms(100)).is_empty());
    }

    #[test]
    fn mouse_exit_chord_remains_accessible_under_layer_override() {
        let mut r =
            remap("[main]\na='layer(nav)'\n'f+s'='free_mouse'\n[layers.nav]\n'f+s'='escape'");
        r.set_mouse_controls(vec![K::KEY_S]);
        assert!(r.key(K::KEY_A, 1).is_empty());
        r.set_mouse_active(true);
        assert!(r.key(K::KEY_S, 1).is_empty());
        assert_eq!(r.key(K::KEY_F, 1), [Event::Mouse(true)]);
    }

    #[test]
    fn replayed_mouse_activation_bypasses_later_controls_in_same_batch() {
        let mut r = remap("[main]\n'a+s+d'='ctrl'\na='free_mouse'\ns='escape'");
        r.set_mouse_controls(vec![K::KEY_S]);
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert!(r.key(K::KEY_S, 1).is_empty());
        assert_eq!(
            r.advance(ms(25)),
            [Event::Mouse(true), Event::Key(K::KEY_S, 1)]
        );
        r.set_mouse_active(true);
        assert_eq!(r.key(K::KEY_A, 0), [Event::Mouse(false)]);
        r.set_mouse_active(false);
        assert_eq!(r.key(K::KEY_S, 0), [Event::Key(K::KEY_S, 0)]);
    }

    #[test]
    fn interrupted_timeout_mouse_activation_bypasses_interrupting_control() {
        let mut r = remap("[main]\na='timeout(free_mouse, 175, ctrl)'\ns='escape'\n's+d'='super'");
        r.set_mouse_controls(vec![K::KEY_S]);
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert_eq!(
            r.key(K::KEY_S, 1),
            [Event::Mouse(true), Event::Key(K::KEY_S, 1)]
        );
    }

    #[test]
    fn failed_chord_mouse_activation_bypasses_interrupting_layer_mapping() {
        let mut r = remap(
            "[main]\ncapslock='layer(nav)'\na='free_mouse'\n'a+d'='ctrl'\n\
             [layers.nav]\ns='escape'\n's+f'='super'",
        );
        r.set_mouse_controls(vec![K::KEY_S]);
        assert!(r.key(K::KEY_CAPSLOCK, 1).is_empty());
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert_eq!(
            r.key(K::KEY_S, 1),
            [Event::Mouse(true), Event::Key(K::KEY_S, 1)]
        );
        r.set_mouse_active(true);
        assert_eq!(r.key(K::KEY_S, 2), [Event::Key(K::KEY_S, 2)]);
        assert_eq!(r.key(K::KEY_S, 0), [Event::Key(K::KEY_S, 0)]);
    }

    fn dispatch(mouse: &mut crate::engine::Engine, events: &mut Vec<Event>, event: Event) -> bool {
        events.push(event);
        match event {
            Event::Key(key, value) => {
                mouse.key(key, value);
            }
            Event::Mouse(down) => {
                mouse.mouse(down);
            }
        }
        mouse.active()
    }

    #[test]
    fn synchronous_toggle_off_restores_layer_mapping_in_same_call() {
        let mut r = remap(
            "[main]\ncapslock='layer(nav)'\na='free_mouse'\n'a+d'='free_mouse'\n\
             [layers.nav]\nh='left'",
        );
        let mut mouse = crate::engine::Engine::new(crate::config::Config {
            mode: crate::config::Mode::Toggle,
            ..Default::default()
        });
        r.set_mouse_controls(vec![K::KEY_H]);
        assert!(r.key(K::KEY_CAPSLOCK, 1).is_empty());
        mouse.mouse(true);
        mouse.mouse(false);
        assert!(mouse.active());
        r.set_mouse_active(mouse.active());
        let mut events = Vec::new();
        r.key_with(K::KEY_A, 1, &mut |event| {
            dispatch(&mut mouse, &mut events, event)
        });
        assert!(events.is_empty());
        r.key_with(K::KEY_H, 1, &mut |event| {
            dispatch(&mut mouse, &mut events, event)
        });
        assert_eq!(events, [Event::Mouse(true), Event::Key(K::KEY_LEFT, 1)]);
        assert!(!mouse.active());
    }

    #[test]
    fn synchronous_flushed_legacy_f3_activation_bypasses_layer_mapping() {
        let mut r = remap(
            "[main]\ncapslock='layer(nav)'\na='f3'\n'a+d'='ctrl'\n\
             [layers.nav]\nh='left'\n'h+j'='super'",
        );
        let mut mouse = crate::engine::Engine::new(crate::config::Config::default());
        r.set_mouse_controls(vec![K::KEY_H]);
        assert!(r.key(K::KEY_CAPSLOCK, 1).is_empty());
        let mut events = Vec::new();
        r.key_with(K::KEY_A, 1, &mut |event| {
            dispatch(&mut mouse, &mut events, event)
        });
        r.key_with(K::KEY_H, 1, &mut |event| {
            dispatch(&mut mouse, &mut events, event)
        });
        assert_eq!(events, [Event::Key(K::KEY_F3, 1), Event::Key(K::KEY_H, 1)]);
        assert!(mouse.active());
    }

    #[test]
    fn synchronous_timer_replay_observes_legacy_activation_before_next_key() {
        let mut r = remap("[main]\na='f3'\n'a+h+d'='ctrl'\nh='left'");
        let mut mouse = crate::engine::Engine::new(crate::config::Config::default());
        r.set_mouse_controls(vec![K::KEY_H]);
        let mut events = Vec::new();
        for key in [K::KEY_A, K::KEY_H] {
            r.key_with(key, 1, &mut |event| {
                dispatch(&mut mouse, &mut events, event)
            });
        }
        assert!(events.is_empty());
        r.advance_with(ms(25), &mut |event| {
            dispatch(&mut mouse, &mut events, event)
        });
        assert_eq!(events, [Event::Key(K::KEY_F3, 1), Event::Key(K::KEY_H, 1)]);
        assert!(mouse.active());
    }

    #[test]
    fn fresh_released_member_can_begin_another_chord() {
        let mut r = remap(EXAMPLE);
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert_eq!(r.key(K::KEY_S, 1), [Event::Key(K::KEY_LEFTMETA, 1)]);
        assert_eq!(r.key(K::KEY_S, 0), [Event::Key(K::KEY_LEFTMETA, 0)]);
        assert!(r.key(K::KEY_S, 1).is_empty());
        assert_eq!(r.key(K::KEY_D, 1), [Event::Key(K::KEY_LEFTCTRL, 1)]);
        assert!(r.key(K::KEY_A, 0).is_empty());
        assert_eq!(r.key(K::KEY_S, 0), [Event::Key(K::KEY_LEFTCTRL, 0)]);
        assert!(r.key(K::KEY_D, 0).is_empty());
    }

    #[test]
    fn layer_priority_overrides_exact_chords_and_restores_previous_layer() {
        let mut r = remap(
            "[main]\na='layer(one)'\ns='layer(two)'\n'h+j'='escape'\n\
             [layers.one]\nh='left'\n'h+j'='ctrl'\n\
             [layers.two]\nh='right'\n'h+j'='super'",
        );
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert!(r.key(K::KEY_S, 1).is_empty());
        assert!(r.key(K::KEY_H, 1).is_empty());
        assert_eq!(r.key(K::KEY_J, 1), [Event::Key(K::KEY_LEFTMETA, 1)]);
        assert!(r.key(K::KEY_S, 0).is_empty());
        assert_eq!(r.key(K::KEY_H, 0), [Event::Key(K::KEY_LEFTMETA, 0)]);
        assert!(r.key(K::KEY_J, 0).is_empty());
        assert!(r.key(K::KEY_H, 1).is_empty());
        assert_eq!(r.advance(ms(25)), [Event::Key(K::KEY_LEFT, 1)]);
        assert!(r.key(K::KEY_A, 0).is_empty());
        assert_eq!(r.key(K::KEY_H, 0), [Event::Key(K::KEY_LEFT, 0)]);
    }

    #[test]
    fn timeout_deadline_wins_over_press_and_release_at_exact_boundary() {
        let mut r = remap("[main]\na='timeout(ctrl, 175, super)'");
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert_eq!(r.advance(ms(175)), [Event::Key(K::KEY_LEFTMETA, 1)]);
        assert_eq!(r.key(K::KEY_C, 1), [Event::Key(K::KEY_C, 1)]);
        assert_eq!(r.key(K::KEY_A, 0), [Event::Key(K::KEY_LEFTMETA, 0)]);

        let mut r = remap("[main]\na='timeout(ctrl, 175, super)'");
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert!(r.advance(ms(174)).is_empty());
        assert_eq!(
            r.key(K::KEY_C, 1),
            [Event::Key(K::KEY_LEFTCTRL, 1), Event::Key(K::KEY_C, 1)]
        );
        assert!(r.advance(ms(1)).is_empty());
        assert_eq!(r.key(K::KEY_A, 0), [Event::Key(K::KEY_LEFTCTRL, 0)]);
    }

    #[test]
    fn clear_forgets_pending_bound_and_mouse_state_but_keeps_controls() {
        let mut r = remap(EXAMPLE);
        r.set_mouse_controls(vec![K::KEY_I]);
        assert!(r.key(K::KEY_A, 1).is_empty());
        assert_eq!(r.key(K::KEY_S, 1), [Event::Key(K::KEY_LEFTMETA, 1)]);
        r.clear();
        assert!(r.key(K::KEY_A, 0).is_empty());
        assert!(r.advance(ms(1000)).is_empty());
        r.set_mouse_active(true);
        assert_eq!(r.key(K::KEY_I, 1), [Event::Key(K::KEY_I, 1)]);
    }

    #[test]
    fn capabilities_include_layers_both_timeout_branches_and_aliases() {
        let cfg = config(
            "[main]\n'a+s'='layer(control)'\nd='timeout(escape, 50, layer(meta))'\nf='layer(nav)'\n[layers.nav]\nh='left'",
        );
        assert_eq!(
            cfg.input_keys()
                .unwrap()
                .into_iter()
                .collect::<BTreeSet<_>>(),
            [K::KEY_A, K::KEY_S, K::KEY_D, K::KEY_F, K::KEY_H]
                .into_iter()
                .collect()
        );
        assert_eq!(
            cfg.output_keys()
                .unwrap()
                .into_iter()
                .collect::<BTreeSet<_>>(),
            [K::KEY_LEFTCTRL, K::KEY_LEFTMETA, K::KEY_ESC, K::KEY_LEFT]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn invalid_configs_are_rejected_without_runtime_panics() {
        for text in [
            "chord_timeout=0",
            "[main]\na='layer(missing)'",
            "[main]\na='timeout(ctrl, 0, super)'",
            "[main]\na='timeout(ctrl, -1, super)'",
            "[main]\na='timeout(ctrl, 1.5, super)'",
            "[main]\na='timeout(ctrl, 18446744073709551616, super)'",
            "[main]\na='timeout(ctrl, 25)'",
            "[main]\na='timeout(ctrl, 25, timeout(a, 5, b))'",
            "[main]\na='timeout(ctrl, 25, layer(nav)'",
            "[main]\na='not_a_key'",
            "[main]\nnot_a_key='ctrl'",
            "[main]\n'a+'='ctrl'",
            "[main]\n'a+A'='ctrl'",
            "[main]\n'a+s'='ctrl'\n'S+A'='super'",
            "[main]\na='ctrl'\nA='super'",
            "[main]\n'a+s'='ctrl'\n'a+s+d'='super'",
            "[main]\na='layer()'",
            "[layers.control]\na='ctrl'",
        ] {
            let cfg: RemapConfig = toml::from_str(text).unwrap();
            assert!(cfg.validate().is_err(), "{text}");
            assert!(cfg.input_keys().is_err(), "{text}");
            assert!(cfg.output_keys().is_err(), "{text}");
            assert!(Remapper::new(cfg).is_err(), "{text}");
        }
        assert!(toml::from_str::<RemapConfig>("unknown=true").is_err());
        assert!(toml::from_str::<RemapConfig>("[main]\na=4").is_err());
    }
}
