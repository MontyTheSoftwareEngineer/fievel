use crate::{
    engine::Output,
    input::{EventType, InputEvent, KeyCode},
};
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_ACTIONS: usize = 20_000;
const MAX_DURATION: Duration = Duration::from_secs(5 * 60);
const MAX_STORE_BYTES: u64 = 128 * 1024 * 1024;
const STORE_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HintClick {
    pub output_index: Option<usize>,
    pub x: f64,
    pub y: f64,
    pub right: bool,
}

#[derive(Clone, Default)]
pub struct MacroController {
    mode: Mode,
    registers: BTreeMap<char, Vec<Step>>,
    clock: Duration,
    consumed: BTreeSet<KeyCode>,
    notice: Option<&'static str>,
    store_path: Option<PathBuf>,
    store_dirty: bool,
}

#[derive(Clone, Default)]
enum Mode {
    #[default]
    Off,
    Menu,
    ChooseRecord,
    ChoosePlayback,
    Recording {
        register: char,
        steps: Vec<Step>,
        action_count: usize,
    },
    Playing {
        register: char,
        steps: Vec<Step>,
        next: usize,
        elapsed: Duration,
        held_keys: BTreeSet<KeyCode>,
        held_buttons: BTreeSet<KeyCode>,
    },
}

#[derive(Clone)]
struct Step {
    at: Duration,
    keyboard: Vec<InputEvent>,
    mouse: Vec<InputEvent>,
    hint_clicks: Vec<HintClick>,
}

impl Step {
    fn new(at: Duration) -> Self {
        Self {
            at,
            keyboard: Vec::new(),
            mouse: Vec::new(),
            hint_clicks: Vec::new(),
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MacroStore {
    version: u32,
    registers: Vec<StoredRegister>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredRegister {
    key: char,
    steps: Vec<StoredStep>,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum StoredStep {
    CompactText((u64, String, String, Vec<StoredHintClick>)),
    CompactArrays(
        (
            u64,
            Vec<StoredEvent>,
            Vec<StoredEvent>,
            Vec<StoredHintClick>,
        ),
    ),
    Legacy(StoredStepFields),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredStepFields {
    at_micros: u64,
    keyboard: Vec<StoredEvent>,
    mouse: Vec<StoredEvent>,
    hint_clicks: Vec<StoredHintClick>,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum StoredEvent {
    Compact(StoredEventTuple),
    Legacy(StoredEventFields),
}

#[derive(Deserialize, Serialize)]
struct StoredEventTuple(u16, u16, i32);

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredEventFields {
    event_type: u16,
    code: u16,
    value: i32,
}

#[derive(Deserialize, Serialize)]
#[serde(untagged)]
enum StoredHintClick {
    Compact(StoredHintClickTuple),
    CompactIndex(StoredHintClickIndexTuple),
    Legacy(HintClick),
}

#[derive(Deserialize, Serialize)]
struct StoredHintClickTuple(String, f64, f64, bool);

#[derive(Deserialize, Serialize)]
struct StoredHintClickIndexTuple(i64, f64, f64, bool);

impl MacroStore {
    fn from_registers(registers: &BTreeMap<char, Vec<Step>>) -> Self {
        Self {
            version: STORE_VERSION,
            registers: registers
                .iter()
                .map(|(key, steps)| StoredRegister {
                    key: *key,
                    steps: steps
                        .iter()
                        .map(|step| {
                            StoredStep::CompactText((
                                step.at.as_micros() as u64,
                                encode_events(&step.keyboard),
                                encode_events(&step.mouse),
                                step.hint_clicks
                                    .iter()
                                    .copied()
                                    .map(StoredHintClick::from)
                                    .collect(),
                            ))
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    fn into_registers(self) -> Result<(BTreeMap<char, Vec<Step>>, bool), String> {
        if self.version != STORE_VERSION {
            return Err(format!("unsupported version {}", self.version));
        }
        if self.registers.len() > 26 {
            return Err("contains more than 26 registers".to_owned());
        }

        let mut registers = BTreeMap::new();
        let mut compact = true;
        for register in self.registers {
            if !register.key.is_ascii_lowercase() {
                return Err(format!("invalid register {:?}", register.key));
            }
            if registers.contains_key(&register.key) {
                return Err(format!("duplicate register {:?}", register.key));
            }
            let mut steps = Vec::with_capacity(register.steps.len());
            let mut previous_at = None;
            let mut action_count = 0usize;
            for stored_step in register.steps {
                let (at_micros, keyboard, mouse, stored_clicks) = match stored_step {
                    StoredStep::CompactText((at, keyboard, mouse, clicks)) => (
                        at,
                        decode_events(&keyboard, false)?,
                        decode_events(&mouse, true)?,
                        clicks,
                    ),
                    StoredStep::CompactArrays((at, keyboard, mouse, clicks)) => {
                        compact = false;
                        (
                            at,
                            keyboard
                                .into_iter()
                                .map(|event| event.into_input_event(false))
                                .collect::<Result<_, _>>()?,
                            mouse
                                .into_iter()
                                .map(|event| event.into_input_event(true))
                                .collect::<Result<_, _>>()?,
                            clicks,
                        )
                    }
                    StoredStep::Legacy(step) => {
                        compact = false;
                        (
                            step.at_micros,
                            step.keyboard
                                .into_iter()
                                .map(|event| event.into_input_event(false))
                                .collect::<Result<_, _>>()?,
                            step.mouse
                                .into_iter()
                                .map(|event| event.into_input_event(true))
                                .collect::<Result<_, _>>()?,
                            step.hint_clicks,
                        )
                    }
                };
                if at_micros > MAX_DURATION.as_micros() as u64 {
                    return Err(format!(
                        "register {:?} exceeds the recording time limit",
                        register.key
                    ));
                }
                let at = Duration::from_micros(at_micros);
                if previous_at.is_some_and(|previous| at < previous) {
                    return Err(format!(
                        "register {:?} has out-of-order steps",
                        register.key
                    ));
                }
                previous_at = Some(at);
                let step_actions = keyboard
                    .len()
                    .checked_add(mouse.len())
                    .and_then(|count| count.checked_add(stored_clicks.len()))
                    .ok_or_else(|| "macro action count overflow".to_owned())?;
                action_count = action_count
                    .checked_add(step_actions)
                    .ok_or_else(|| "macro action count overflow".to_owned())?;
                if action_count > MAX_ACTIONS {
                    return Err(format!(
                        "register {:?} exceeds the action limit",
                        register.key
                    ));
                }
                let hint_clicks = stored_clicks
                    .into_iter()
                    .map(StoredHintClick::into_click)
                    .collect::<Result<Vec<_>, _>>()?;
                steps.push(Step {
                    at,
                    keyboard,
                    mouse,
                    hint_clicks,
                });
            }
            registers.insert(register.key, steps);
        }
        Ok((registers, compact))
    }
}

impl StoredEvent {
    fn into_input_event(self, allow_relative: bool) -> Result<InputEvent, String> {
        let (event_type, code, value) = match self {
            Self::Compact(StoredEventTuple(event_type, code, value)) => (event_type, code, value),
            Self::Legacy(event) => (event.event_type, event.code, event.value),
        };
        let valid_key = event_type == EventType::KEY.0 && (0..=2).contains(&value);
        let valid_relative = allow_relative && event_type == EventType::RELATIVE.0;
        if !valid_key && !valid_relative {
            return Err(format!(
                "unsupported event type {} with value {}",
                event_type, value
            ));
        }
        Ok(InputEvent::new(event_type, code, value))
    }
}

impl From<HintClick> for StoredHintClick {
    fn from(click: HintClick) -> Self {
        Self::Compact(StoredHintClickTuple(
            click
                .output_index
                .map_or_else(String::new, |index| index.to_string()),
            click.x,
            click.y,
            click.right,
        ))
    }
}

impl StoredHintClick {
    fn into_click(self) -> Result<HintClick, String> {
        let click = match self {
            Self::Compact(StoredHintClickTuple(index, x, y, right)) => HintClick {
                output_index: if index.is_empty() {
                    None
                } else {
                    Some(
                        index
                            .parse()
                            .map_err(|_| "invalid hint click output index".to_owned())?,
                    )
                },
                x,
                y,
                right,
            },
            Self::CompactIndex(StoredHintClickIndexTuple(index, x, y, right)) => HintClick {
                output_index: match index {
                    -1 => None,
                    index if index >= 0 => Some(
                        usize::try_from(index)
                            .map_err(|_| "hint click output index is too large".to_owned())?,
                    ),
                    _ => return Err("invalid hint click output index".to_owned()),
                },
                x,
                y,
                right,
            },
            Self::Legacy(click) => click,
        };
        if !click.x.is_finite() || !click.y.is_finite() {
            return Err("invalid hint click coordinates".to_owned());
        }
        Ok(click)
    }
}

fn encode_events(events: &[InputEvent]) -> String {
    events
        .iter()
        .map(|event| {
            format!(
                "{},{},{}",
                event.event_type().0,
                event.code(),
                event.value()
            )
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn decode_events(encoded: &str, allow_relative: bool) -> Result<Vec<InputEvent>, String> {
    if encoded.is_empty() {
        return Ok(Vec::new());
    }
    encoded
        .split(';')
        .map(|event| {
            let mut fields = event.split(',');
            let event_type = fields
                .next()
                .ok_or_else(|| "incomplete stored event".to_owned())?
                .trim()
                .parse()
                .map_err(|_| "invalid event type in macro store".to_owned())?;
            let code = fields
                .next()
                .ok_or_else(|| "incomplete stored event".to_owned())?
                .trim()
                .parse()
                .map_err(|_| "invalid event code in macro store".to_owned())?;
            let value = fields
                .next()
                .ok_or_else(|| "incomplete stored event".to_owned())?
                .trim()
                .parse()
                .map_err(|_| "invalid event value in macro store".to_owned())?;
            if fields.next().is_some() {
                return Err("unexpected event data in macro store".to_owned());
            }
            StoredEvent::Legacy(StoredEventFields {
                event_type,
                code,
                value,
            })
            .into_input_event(allow_relative)
        })
        .collect()
}

fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("macros");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = parent.join(format!(".{file_name}.{}.{nonce}.tmp", std::process::id()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&temporary)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[derive(Debug)]
pub enum HotkeyResult {
    Buffered,
    Suppressed,
    Forward(Vec<(KeyCode, i32)>),
    Activated,
}

#[derive(Clone)]
pub struct MacroHotkey {
    chord: BTreeSet<KeyCode>,
    allow_forwarded: bool,
    activates: bool,
    held: BTreeSet<KeyCode>,
    forwarded: BTreeSet<KeyCode>,
    pending: Vec<(KeyCode, i32)>,
    swallowed: BTreeSet<KeyCode>,
}

impl MacroHotkey {
    pub fn new(chord: &[KeyCode]) -> Self {
        Self::with_forwarded(chord, false, true)
    }

    pub fn new_logical(chord: &[KeyCode]) -> Self {
        Self::with_forwarded(chord, true, true)
    }

    pub fn new_passthrough(chord: &[KeyCode]) -> Self {
        Self::with_forwarded(chord, false, false)
    }

    pub fn new_logical_passthrough(chord: &[KeyCode]) -> Self {
        Self::with_forwarded(chord, true, false)
    }

    fn with_forwarded(chord: &[KeyCode], allow_forwarded: bool, activates: bool) -> Self {
        Self {
            chord: chord.iter().copied().collect(),
            allow_forwarded,
            activates,
            held: BTreeSet::new(),
            forwarded: BTreeSet::new(),
            pending: Vec::new(),
            swallowed: BTreeSet::new(),
        }
    }

    pub fn swallows(&self, key: KeyCode) -> bool {
        self.swallowed.contains(&key)
    }

    pub fn forward_for_hint(&mut self, key: KeyCode, value: i32) -> Vec<(KeyCode, i32)> {
        let mut events = std::mem::take(&mut self.pending);
        events.push((key, value));
        if self.chord.contains(&key) {
            match value {
                1 => {
                    self.held.insert(key);
                }
                0 => {
                    self.held.remove(&key);
                }
                _ => {}
            }
        }
        self.forward_pending(&events);
        events
    }

    pub fn clear(&mut self) {
        self.held.clear();
        self.forwarded.clear();
        self.pending.clear();
        self.swallowed.clear();
    }

    pub fn forward_buffered(&mut self) -> Vec<(KeyCode, i32)> {
        let pending = std::mem::take(&mut self.pending);
        self.forward_pending(&pending);
        pending
    }

    pub fn key(&mut self, key: KeyCode, value: i32) -> Option<HotkeyResult> {
        if self.swallowed.contains(&key) {
            if value == 0 {
                self.swallowed.remove(&key);
                self.held.remove(&key);
            }
            return Some(HotkeyResult::Suppressed);
        }

        if self.chord.contains(&key) {
            match value {
                1 => {
                    self.held.insert(key);
                    if !self.pending.is_empty() {
                        self.pending.push((key, value));
                        return Some(self.activate_if_complete());
                    }
                    if self.allow_forwarded || self.forwarded.is_disjoint(&self.chord) {
                        self.pending.push((key, value));
                        return Some(self.activate_if_complete());
                    }
                }
                0 => {
                    self.held.remove(&key);
                    self.forwarded.remove(&key);
                    if !self.pending.is_empty() {
                        self.pending.push((key, value));
                        let pending = std::mem::take(&mut self.pending);
                        self.forward_pending(&pending);
                        return Some(HotkeyResult::Forward(pending));
                    }
                }
                2 if !self.pending.is_empty() => {
                    self.pending.push((key, value));
                    return Some(HotkeyResult::Buffered);
                }
                _ => {}
            }
        } else if !self.pending.is_empty() {
            let mut pending = std::mem::take(&mut self.pending);
            self.forward_pending(&pending);
            pending.push((key, value));
            return Some(HotkeyResult::Forward(pending));
        }

        if self.chord.contains(&key) {
            match value {
                1 => {
                    self.forwarded.insert(key);
                }
                0 => {
                    self.forwarded.remove(&key);
                }
                _ => {}
            }
        }
        None
    }

    fn activate_if_complete(&mut self) -> HotkeyResult {
        if self.chord.iter().all(|member| self.held.contains(member))
            && (self.allow_forwarded || self.forwarded.is_disjoint(&self.chord))
        {
            if !self.activates {
                let pending = std::mem::take(&mut self.pending);
                self.forward_pending(&pending);
                return HotkeyResult::Forward(pending);
            }
            self.swallowed.extend(
                self.chord
                    .iter()
                    .filter_map(|member| self.held.contains(member).then_some(*member)),
            );
            self.pending.clear();
            HotkeyResult::Activated
        } else {
            HotkeyResult::Buffered
        }
    }

    fn forward_pending(&mut self, events: &[(KeyCode, i32)]) {
        for (key, value) in events {
            if !self.chord.contains(key) {
                continue;
            }
            match value {
                1 => {
                    self.forwarded.insert(*key);
                }
                0 => {
                    self.forwarded.remove(key);
                }
                _ => {}
            }
        }
    }
}

impl MacroController {
    pub fn load_store(&mut self, path: impl Into<PathBuf>) -> Result<(), String> {
        let path = path.into();
        self.store_path = Some(path.clone());
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.registers.clear();
                self.store_dirty = false;
                return Ok(());
            }
            Err(error) => {
                return Err(format!(
                    "Cannot read macro store {}: {error}",
                    path.display()
                ));
            }
        };
        if metadata.len() > MAX_STORE_BYTES {
            return Err(format!(
                "Macro store {} exceeds the {} MiB limit",
                path.display(),
                MAX_STORE_BYTES / (1024 * 1024)
            ));
        }
        let contents = fs::read_to_string(&path)
            .map_err(|error| format!("Cannot read macro store {}: {error}", path.display()))?;
        let store: MacroStore = toml::from_str(&contents)
            .map_err(|error| format!("Invalid macro store {}: {error}", path.display()))?;
        let (registers, compact) = store
            .into_registers()
            .map_err(|error| format!("Invalid macro store {}: {error}", path.display()))?;
        self.registers = registers;
        self.store_dirty = !compact;
        self.persist_store();
        Ok(())
    }

    pub fn shortcut(&mut self) {
        self.notice = None;
        if self.recording() {
            self.finish_recording();
        } else if !self.active() {
            self.mode = Mode::Menu;
        }
    }

    pub fn active(&self) -> bool {
        !matches!(&self.mode, Mode::Off)
    }

    pub fn recording(&self) -> bool {
        matches!(&self.mode, Mode::Recording { .. })
    }

    pub fn consume_until_release(&mut self, keys: impl IntoIterator<Item = KeyCode>) {
        self.consumed.extend(keys);
    }

    pub fn key(&mut self, key: KeyCode, value: i32) -> Option<Output> {
        if value == 0 && self.consumed.remove(&key) {
            return Some(Output::default());
        }

        let mode = std::mem::replace(&mut self.mode, Mode::Off);
        match mode {
            Mode::Off => None,
            Mode::Menu => {
                self.mode = Mode::Menu;
                if value == 1 {
                    self.consumed.insert(key);
                    self.notice = None;
                    match key {
                        KeyCode::KEY_ESC => self.mode = Mode::Off,
                        KeyCode::KEY_R => self.mode = Mode::ChooseRecord,
                        KeyCode::KEY_P if !self.registers.is_empty() => {
                            self.mode = Mode::ChoosePlayback
                        }
                        _ => {}
                    }
                }
                Some(Output::default())
            }
            Mode::ChooseRecord => {
                self.mode = Mode::ChooseRecord;
                if value == 1 {
                    self.consumed.insert(key);
                    if key == KeyCode::KEY_ESC {
                        self.mode = Mode::Menu;
                    } else if let Some(register) = register_key(key) {
                        self.clock = Duration::ZERO;
                        self.registers.remove(&register);
                        self.mode = Mode::Recording {
                            register,
                            steps: Vec::new(),
                            action_count: 0,
                        };
                    }
                }
                Some(Output::default())
            }
            Mode::ChoosePlayback => {
                self.mode = Mode::ChoosePlayback;
                if value == 1 {
                    self.consumed.insert(key);
                    if key == KeyCode::KEY_ESC {
                        self.mode = Mode::Menu;
                    } else if let Some(register) = register_key(key) {
                        if let Some(steps) = self.registers.get(&register).cloned() {
                            self.mode = Mode::Playing {
                                register,
                                steps,
                                next: 0,
                                elapsed: Duration::ZERO,
                                held_keys: BTreeSet::new(),
                                held_buttons: BTreeSet::new(),
                            };
                            return Some(self.advance_playback(Duration::ZERO));
                        }
                    }
                }
                Some(Output::default())
            }
            Mode::Recording {
                register,
                steps,
                action_count,
            } => {
                if key == KeyCode::KEY_ESC && value == 1 {
                    self.consumed.insert(key);
                    self.registers.insert(register, steps);
                    self.mode = Mode::Menu;
                    self.store_dirty = true;
                    self.persist_store();
                    return Some(Output::default());
                }
                self.mode = Mode::Recording {
                    register,
                    steps,
                    action_count,
                };
                None
            }
            Mode::Playing {
                register,
                steps,
                next,
                elapsed,
                held_keys,
                held_buttons,
            } => {
                if value == 1 {
                    self.consumed.insert(key);
                    if key == KeyCode::KEY_ESC {
                        self.mode = Mode::Playing {
                            register,
                            steps,
                            next,
                            elapsed,
                            held_keys,
                            held_buttons,
                        };
                        let output = self.stop_playback();
                        self.mode = Mode::Menu;
                        return Some(output);
                    }
                }
                self.mode = Mode::Playing {
                    register,
                    steps,
                    next,
                    elapsed,
                    held_keys,
                    held_buttons,
                };
                Some(Output::default())
            }
        }
    }

    pub fn advance(&mut self, elapsed: Duration) -> Output {
        match &self.mode {
            Mode::Recording { .. } => {
                self.clock = self.clock.saturating_add(elapsed);
                if self.clock > MAX_DURATION {
                    self.finish_recording();
                    self.notice = Some("RECORDING TIME LIMIT REACHED");
                }
                Output::default()
            }
            Mode::Playing { .. } => self.advance_playback(elapsed),
            _ => Output::default(),
        }
    }

    pub fn record_output(&mut self, output: &Output) {
        if output.keyboard.is_empty() && output.mouse.is_empty() && output.macro_clicks.is_empty() {
            return;
        }
        let added = output.keyboard.len() + output.mouse.len() + output.macro_clicks.len();
        let exceeds_limit = matches!(
            &self.mode,
            Mode::Recording { action_count, .. }
                if action_count.saturating_add(added) > MAX_ACTIONS
        );
        if exceeds_limit {
            self.finish_recording();
            self.notice = Some("RECORDING ACTION LIMIT REACHED");
            return;
        }
        let Mode::Recording {
            steps,
            action_count,
            ..
        } = &mut self.mode
        else {
            return;
        };
        let step = current_step(steps, self.clock);
        step.keyboard.extend(output.keyboard.iter().cloned());
        step.mouse.extend(output.mouse.iter().cloned());
        step.hint_clicks.extend(output.macro_clicks.iter().copied());
        *action_count += added;
    }

    pub fn record_hint_click(&mut self, click: HintClick) {
        if !matches!(&self.mode, Mode::Recording { action_count, .. } if *action_count < MAX_ACTIONS)
        {
            if self.recording() {
                self.finish_recording();
                self.notice = Some("RECORDING ACTION LIMIT REACHED");
            }
            return;
        }
        let Mode::Recording {
            steps,
            action_count,
            ..
        } = &mut self.mode
        else {
            return;
        };
        current_step(steps, self.clock).hint_clicks.push(click);
        *action_count += 1;
    }

    pub fn status(&self) -> Option<String> {
        if let Some(notice) = self.notice {
            return Some(format!("{notice}  ESC CLOSE"));
        }
        let registers = self
            .registers
            .keys()
            .map(|register| register.to_ascii_uppercase())
            .collect::<String>();
        match &self.mode {
            Mode::Off => None,
            Mode::Menu => Some(format!(
                "MACROS  R RECORD  P PLAY  [{}]  ESC CLOSE",
                if registers.is_empty() {
                    "NONE"
                } else {
                    &registers
                }
            )),
            Mode::ChooseRecord => Some(format!(
                "RECORD INTO A-Z  OVERWRITE [{}]  ESC BACK",
                if registers.is_empty() {
                    "NONE"
                } else {
                    &registers
                }
            )),
            Mode::ChoosePlayback => Some(format!("PLAY REGISTER [{}]  ESC BACK", registers)),
            Mode::Recording {
                register,
                action_count,
                ..
            } => Some(format!(
                "REC {}  {action_count} ACTIONS  HOTKEY / ESC STOP",
                register.to_ascii_uppercase()
            )),
            Mode::Playing { register, .. } => {
                Some(format!("PLAY {}  ESC STOP", register.to_ascii_uppercase()))
            }
        }
    }

    pub fn stop(&mut self) -> Output {
        if self.recording() {
            self.finish_recording();
        }
        let output = self.stop_playback();
        self.mode = Mode::Off;
        self.consumed.clear();
        self.notice = None;
        self.persist_store();
        output
    }

    fn finish_recording(&mut self) {
        if let Mode::Recording {
            register, steps, ..
        } = std::mem::replace(&mut self.mode, Mode::Menu)
        {
            self.registers.insert(register, steps);
            self.store_dirty = true;
            self.persist_store();
        }
    }

    fn persist_store(&mut self) {
        if !self.store_dirty {
            return;
        }
        let Some(path) = &self.store_path else {
            return;
        };
        let path = path.clone();
        let result = toml::to_string_pretty(&MacroStore::from_registers(&self.registers))
            .map_err(io::Error::other)
            .and_then(|contents| {
                if contents.len() as u64 > MAX_STORE_BYTES {
                    return Err(io::Error::other("macro store exceeds the size limit"));
                }
                write_atomically(&path, contents.as_bytes())
            });
        match result {
            Ok(()) => self.store_dirty = false,
            Err(error) => {
                eprintln!(
                    "fievel: cannot save macro store {}: {error}",
                    path.display()
                );
            }
        }
    }

    fn stop_playback(&mut self) -> Output {
        let Mode::Playing {
            held_keys,
            held_buttons,
            ..
        } = std::mem::replace(&mut self.mode, Mode::Off)
        else {
            return Output::default();
        };
        let mut output = Output::default();
        output
            .keyboard
            .extend(held_keys.into_iter().map(|key| key_event(key, 0)));
        output
            .mouse
            .extend(held_buttons.into_iter().map(|key| key_event(key, 0)));
        output
    }

    fn advance_playback(&mut self, elapsed: Duration) -> Output {
        let mut output = Output::default();
        let mut finished = false;
        if let Mode::Playing {
            steps,
            next,
            elapsed: playback_elapsed,
            held_keys,
            held_buttons,
            ..
        } = &mut self.mode
        {
            *playback_elapsed = playback_elapsed.saturating_add(elapsed);
            while let Some(step) = steps.get(*next).filter(|step| step.at <= *playback_elapsed) {
                output.keyboard.extend(step.keyboard.iter().cloned());
                output.mouse.extend(step.mouse.iter().cloned());
                output.macro_clicks.extend(step.hint_clicks.iter().copied());
                update_held(held_keys, &step.keyboard);
                update_held(held_buttons, &step.mouse);
                *next += 1;
            }
            if *next == steps.len() {
                let held_keys = std::mem::take(held_keys);
                let held_buttons = std::mem::take(held_buttons);
                output
                    .keyboard
                    .extend(held_keys.into_iter().map(|key| key_event(key, 0)));
                output
                    .mouse
                    .extend(held_buttons.into_iter().map(|key| key_event(key, 0)));
                finished = true;
            }
        }
        if finished {
            self.mode = Mode::Menu;
        }
        output
    }
}

fn current_step(steps: &mut Vec<Step>, at: Duration) -> &mut Step {
    if steps.last().map_or(true, |step| step.at != at) {
        steps.push(Step::new(at));
    }
    steps.last_mut().unwrap()
}

fn register_key(key: KeyCode) -> Option<char> {
    [
        (KeyCode::KEY_A, 'a'),
        (KeyCode::KEY_B, 'b'),
        (KeyCode::KEY_C, 'c'),
        (KeyCode::KEY_D, 'd'),
        (KeyCode::KEY_E, 'e'),
        (KeyCode::KEY_F, 'f'),
        (KeyCode::KEY_G, 'g'),
        (KeyCode::KEY_H, 'h'),
        (KeyCode::KEY_I, 'i'),
        (KeyCode::KEY_J, 'j'),
        (KeyCode::KEY_K, 'k'),
        (KeyCode::KEY_L, 'l'),
        (KeyCode::KEY_M, 'm'),
        (KeyCode::KEY_N, 'n'),
        (KeyCode::KEY_O, 'o'),
        (KeyCode::KEY_P, 'p'),
        (KeyCode::KEY_Q, 'q'),
        (KeyCode::KEY_R, 'r'),
        (KeyCode::KEY_S, 's'),
        (KeyCode::KEY_T, 't'),
        (KeyCode::KEY_U, 'u'),
        (KeyCode::KEY_V, 'v'),
        (KeyCode::KEY_W, 'w'),
        (KeyCode::KEY_X, 'x'),
        (KeyCode::KEY_Y, 'y'),
        (KeyCode::KEY_Z, 'z'),
    ]
    .into_iter()
    .find_map(|(candidate, register)| (candidate == key).then_some(register))
}

fn key_event(key: KeyCode, value: i32) -> InputEvent {
    InputEvent::new(EventType::KEY.0, key.0, value)
}

fn update_held(held: &mut BTreeSet<KeyCode>, events: &[InputEvent]) {
    for event in events {
        if event.event_type() != EventType::KEY {
            continue;
        }
        let key = KeyCode(event.code());
        match event.value() {
            1 => {
                held.insert(key);
            }
            0 => {
                held.remove(&key);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    fn events(events: &[InputEvent]) -> Vec<(KeyCode, i32)> {
        events
            .iter()
            .map(|event| (KeyCode(event.code()), event.value()))
            .collect()
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("fievel-macros-{}-{nonce}", std::process::id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn macro_hotkey_tracks_forwarded_keys_after_an_incomplete_chord() {
        let mut hotkey = MacroHotkey::new(&[
            KeyCode::KEY_LEFTMETA,
            KeyCode::KEY_M,
            KeyCode::KEY_LEFTSHIFT,
        ]);
        assert!(matches!(
            hotkey.key(KeyCode::KEY_LEFTMETA, 1),
            Some(HotkeyResult::Buffered)
        ));
        assert!(matches!(
            hotkey.key(KeyCode::KEY_M, 1),
            Some(HotkeyResult::Buffered)
        ));
        assert!(matches!(
            hotkey.key(KeyCode::KEY_LEFTMETA, 0),
            Some(HotkeyResult::Forward(_))
        ));
        assert!(hotkey.forwarded.contains(&KeyCode::KEY_M));
        assert!(matches!(hotkey.key(KeyCode::KEY_LEFTSHIFT, 1), None));
        assert!(matches!(hotkey.key(KeyCode::KEY_LEFTMETA, 1), None));
        assert!(!hotkey.forwarded.is_disjoint(&hotkey.chord));
    }

    #[test]
    fn recording_register_replays_events_with_timing_and_releases_keys() {
        let mut macros = MacroController::default();
        macros.shortcut();
        assert!(macros.status().unwrap().contains("R RECORD"));
        macros.key(KeyCode::KEY_R, 1);
        macros.key(KeyCode::KEY_R, 0);
        macros.key(KeyCode::KEY_A, 1);
        macros.key(KeyCode::KEY_A, 0);

        macros.advance(Duration::from_millis(100));
        let mut output = Output::default();
        output.keyboard.push(key_event(KeyCode::KEY_B, 1));
        macros.record_output(&output);
        macros.shortcut();
        assert!(macros.status().unwrap().contains("[A]"));

        macros.key(KeyCode::KEY_P, 1);
        macros.key(KeyCode::KEY_P, 0);
        let initial = macros.key(KeyCode::KEY_A, 1).unwrap();
        assert!(initial.keyboard.is_empty());
        assert!(macros
            .advance(Duration::from_millis(99))
            .keyboard
            .is_empty());
        let played = macros.advance(Duration::from_millis(1));
        assert_eq!(
            events(&played.keyboard),
            [(KeyCode::KEY_B, 1), (KeyCode::KEY_B, 0),]
        );
        assert!(macros.status().unwrap().contains("R RECORD"));
    }

    #[test]
    fn stopping_playback_releases_held_mouse_buttons() {
        let mut macros = MacroController::default();
        macros.shortcut();
        macros.key(KeyCode::KEY_R, 1);
        macros.key(KeyCode::KEY_A, 1);
        macros.key(KeyCode::KEY_A, 0);
        let mut output = Output::default();
        output.mouse.push(key_event(KeyCode::BTN_LEFT, 1));
        macros.record_output(&output);
        macros.advance(Duration::from_millis(100));
        output.mouse[0] = key_event(KeyCode::BTN_LEFT, 0);
        macros.record_output(&output);
        macros.shortcut();

        macros.key(KeyCode::KEY_P, 1);
        let played = macros.key(KeyCode::KEY_A, 1).unwrap();
        assert_eq!(events(&played.mouse), [(KeyCode::BTN_LEFT, 1)]);
        let stopped = macros.key(KeyCode::KEY_ESC, 1).unwrap();
        assert_eq!(events(&stopped.mouse), [(KeyCode::BTN_LEFT, 0)]);
    }

    #[test]
    fn recorded_hint_click_keeps_its_target_and_button() {
        let mut macros = MacroController::default();
        macros.shortcut();
        macros.key(KeyCode::KEY_R, 1);
        macros.key(KeyCode::KEY_A, 1);
        macros.key(KeyCode::KEY_A, 0);
        let click = HintClick {
            output_index: Some(2),
            x: 123.5,
            y: 456.25,
            right: true,
        };
        macros.record_hint_click(click);
        macros.shortcut();

        macros.key(KeyCode::KEY_P, 1);
        let played = macros.key(KeyCode::KEY_A, 1).unwrap();
        assert_eq!(played.macro_clicks, [click]);
    }

    #[test]
    fn completed_registers_persist_and_reload_for_playback() {
        let directory = TestDirectory::new();
        let path = directory.0.join("config/macros.toml");
        let mut macros = MacroController::default();
        macros.load_store(&path).unwrap();
        macros.shortcut();
        macros.key(KeyCode::KEY_R, 1);
        macros.key(KeyCode::KEY_R, 0);
        macros.key(KeyCode::KEY_A, 1);
        macros.key(KeyCode::KEY_A, 0);

        let mut output = Output::default();
        output.keyboard.push(key_event(KeyCode::KEY_B, 1));
        macros.record_output(&output);
        let click = HintClick {
            output_index: Some(2),
            x: 123.5,
            y: 456.25,
            right: true,
        };
        macros.record_hint_click(click);
        macros.key(KeyCode::KEY_ESC, 1);
        assert!(path.exists());

        macros.key(KeyCode::KEY_R, 1);
        macros.key(KeyCode::KEY_R, 0);
        macros.key(KeyCode::KEY_C, 1);
        macros.key(KeyCode::KEY_C, 0);
        output.keyboard[0] = key_event(KeyCode::KEY_D, 1);
        macros.record_output(&output);
        macros.shortcut();

        let mut loaded = MacroController::default();
        loaded.load_store(&path).unwrap();
        loaded.shortcut();
        assert!(loaded.status().unwrap().contains("[AC]"));
        loaded.key(KeyCode::KEY_P, 1);
        loaded.key(KeyCode::KEY_P, 0);
        let played = loaded.key(KeyCode::KEY_A, 1).unwrap();

        assert_eq!(
            events(&played.keyboard),
            [(KeyCode::KEY_B, 1), (KeyCode::KEY_B, 0)]
        );
        assert_eq!(played.macro_clicks, [click]);
    }

    #[test]
    fn shutdown_saves_an_in_progress_recording() {
        let directory = TestDirectory::new();
        let path = directory.0.join("macros.toml");
        let mut macros = MacroController::default();
        macros.load_store(&path).unwrap();
        macros.shortcut();
        macros.key(KeyCode::KEY_R, 1);
        macros.key(KeyCode::KEY_A, 1);
        macros.key(KeyCode::KEY_A, 0);

        let mut output = Output::default();
        output.keyboard.push(key_event(KeyCode::KEY_B, 1));
        macros.record_output(&output);
        macros.stop();

        let mut loaded = MacroController::default();
        loaded.load_store(&path).unwrap();
        loaded.shortcut();
        assert!(loaded.status().unwrap().contains("[A]"));
    }

    #[test]
    fn macro_store_uses_compact_events() {
        let directory = TestDirectory::new();
        let path = directory.0.join("macros.toml");
        let mut macros = MacroController::default();
        macros.load_store(&path).unwrap();
        macros.shortcut();
        macros.key(KeyCode::KEY_R, 1);
        macros.key(KeyCode::KEY_A, 1);
        macros.key(KeyCode::KEY_A, 0);

        let mut output = Output::default();
        output
            .mouse
            .extend((0..1_000).map(|_| InputEvent::new(EventType::RELATIVE.0, 0, 1)));
        macros.record_output(&output);
        macros.shortcut();

        let size = fs::metadata(path).unwrap().len();
        assert!(size < 20_000, "serialized file was {size} bytes");
    }

    #[test]
    fn macro_store_loads_the_previous_verbose_format() {
        let directory = TestDirectory::new();
        let path = directory.0.join("macros.toml");
        fs::write(
            &path,
            r#"
version = 1

[[registers]]
key = "a"

[[registers.steps]]
at_micros = 0
keyboard = [{ event_type = 1, code = 48, value = 1 }]
mouse = []
hint_clicks = []
"#,
        )
        .unwrap();

        let mut macros = MacroController::default();
        macros.load_store(&path).unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("event_type ="));
        macros.shortcut();
        macros.key(KeyCode::KEY_P, 1);
        macros.key(KeyCode::KEY_P, 0);
        let played = macros.key(KeyCode::KEY_A, 1).unwrap();

        assert_eq!(
            events(&played.keyboard),
            [(KeyCode::KEY_B, 1), (KeyCode::KEY_B, 0)]
        );
    }

    #[test]
    fn macro_store_loads_and_compacts_the_previous_array_format() {
        let directory = TestDirectory::new();
        let path = directory.0.join("macros.toml");
        fs::write(
            &path,
            r#"
version = 1

[[registers]]
key = "a"

[[registers.steps]]
at_micros = 0
keyboard = []
mouse = []
hint_clicks = [[2, 1.5, 2.5, true]]
"#,
        )
        .unwrap();

        let mut macros = MacroController::default();
        macros.load_store(&path).unwrap();
        assert!(!fs::read_to_string(&path)
            .unwrap()
            .contains("[[2, 1.5, 2.5, true]]"));
        macros.shortcut();
        macros.key(KeyCode::KEY_P, 1);
        macros.key(KeyCode::KEY_P, 0);
        let played = macros.key(KeyCode::KEY_A, 1).unwrap();

        assert_eq!(
            played.macro_clicks,
            [HintClick {
                output_index: Some(2),
                x: 1.5,
                y: 2.5,
                right: true,
            }]
        );
    }
}
