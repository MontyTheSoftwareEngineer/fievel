mod config;
mod detect;
mod engine;
mod font;
mod hint_input;
mod hints;
mod indicator;
mod label;
mod odometer;
mod pipeline;
mod remap;

use clap::Parser;
use config::Config;
use engine::Output;
use evdev::{
    uinput::VirtualDevice, AttributeSet, BusType, Device, EventType, InputId, KeyCode,
    RelativeAxisCode,
};
use hint_input::{HintInput, HintInputEvent};
use hints::{ActiveHints, HintResult};
use indicator::Indicator;
use pipeline::InputEngine as Engine;
use std::{
    error::Error,
    fs, io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const KEYBOARD_NAME: &str = "fievel keyboard";
const MOUSE_NAME: &str = "fievel pointer";
const KEYD_KEYBOARD_NAME: &str = "keyd virtual keyboard";
const TICK: Duration = Duration::from_millis(4);

#[derive(Parser)]
#[command(
    version,
    about = "Keyboard remapping and mouse control (default: hold F3)"
)]
struct Args {
    /// Read only this keyboard event node (default: all suitable keyboards)
    #[arg(short, long)]
    device: Option<PathBuf>,

    /// List accessible keyboards without grabbing them
    #[arg(long)]
    list: bool,

    /// Report saved mouse-movement totals without opening input devices
    #[arg(long, conflicts_with_all = ["list", "check_config"])]
    odometer: bool,

    /// Reset both odometer totals, including in a running instance
    #[arg(long, conflicts_with_all = ["odometer", "list", "check_config"])]
    reset_odometer: bool,

    /// Reference scale for estimated miles (raw input units per inch)
    #[arg(long, default_value = "96", requires = "odometer", value_parser = positive_speed)]
    odometer_units_per_inch: f64,

    /// Config file (default: ~/.config/fievel/fievel.config)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Print the effective config and exit without opening input devices
    #[arg(long, conflicts_with = "list")]
    check_config: bool,

    /// Override normal pointer speed from config (input units per second)
    #[arg(long, value_parser = positive_speed)]
    speed: Option<f64>,

    /// Override normal scroll speed from config (notches per second)
    #[arg(long, value_parser = positive_speed)]
    scroll_speed: Option<f64>,
}

fn positive_speed(value: &str) -> Result<f64, String> {
    let speed: f64 = value.parse().map_err(|_| "expected a number".to_owned())?;
    config::validate_speed(speed)?;
    Ok(speed)
}

fn load_config(args: &Args) -> Result<(PathBuf, Config), Box<dyn Error>> {
    let path = match &args.config {
        Some(path) => path.clone(),
        None => {
            let home = std::env::var_os("HOME")
                .filter(|home| !home.is_empty())
                .ok_or("HOME is unset; specify a config file with --config PATH")?;
            PathBuf::from(home).join(".config/fievel/fievel.config")
        }
    };
    let mut config = Config::load(&path, args.config.is_some())?;
    if let Some(speed) = args.speed {
        config.speeds.normal = speed;
    }
    if let Some(speed) = args.scroll_speed {
        config.speeds.scroll = speed;
    }
    config.validate()?;
    Ok((path, config))
}

fn is_keyboard(device: &Device) -> bool {
    device.supported_keys().is_some_and(|keys| {
        [KeyCode::KEY_A, KeyCode::KEY_SPACE]
            .iter()
            .all(|key| keys.contains(*key))
    })
}

fn is_ours(device: &Device) -> bool {
    matches!(device.name(), Some(KEYBOARD_NAME | MOUSE_NAME))
}

fn input_sys_path(path: &Path) -> io::Result<PathBuf> {
    let node = path
        .file_name()
        .ok_or_else(|| io::Error::other("input device has no file name"))?;
    fs::canonicalize(Path::new("/sys/class/input").join(node))
}

fn is_virtual(path: &Path) -> io::Result<bool> {
    Ok(is_virtual_input(&input_sys_path(path)?))
}

fn is_virtual_input(sys_path: &Path) -> bool {
    // Bluetooth keyboards can live under virtual/misc/uhid, unlike uinput outputs.
    sys_path.starts_with("/sys/devices/virtual/input")
}

fn keyboards() -> io::Result<Vec<(PathBuf, Device)>> {
    keyboards_excluding(&[])
}

fn keyboards_excluding(excluded: &[PathBuf]) -> io::Result<Vec<(PathBuf, Device)>> {
    let mut devices = Vec::new();
    for entry in fs::read_dir("/dev/input")? {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().starts_with("event") {
            continue;
        }
        let path = entry.path();
        if excluded.contains(&path) {
            continue;
        }
        match Device::open(&path) {
            Ok(device) if is_keyboard(&device) && !is_ours(&device) => {
                devices.push((path, device));
            }
            Ok(_) => {}
            Err(error) => eprintln!("Cannot open {}: {error}", path.display()),
        }
    }
    devices.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(devices)
}

type KeyboardScanResult = io::Result<Vec<(PathBuf, Device)>>;

#[derive(Default)]
struct KeyboardScanner {
    worker: Option<thread::JoinHandle<KeyboardScanResult>>,
}

impl KeyboardScanner {
    fn start(&mut self, excluded: Vec<PathBuf>) -> io::Result<()> {
        if self.worker.is_none() {
            self.worker = Some(
                thread::Builder::new()
                    .name("fievel-keyboards".into())
                    .spawn(move || {
                        let mut devices = keyboards_excluding(&excluded)?;
                        devices.retain(|(path, device)| is_auto_candidate(path, device, true));
                        Ok(devices)
                    })?,
            );
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<KeyboardScanResult> {
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            self.finish()
        } else {
            None
        }
    }

    fn finish(&mut self) -> Option<KeyboardScanResult> {
        self.worker.take().map(|worker| {
            worker
                .join()
                .unwrap_or_else(|_| Err(io::Error::other("Keyboard discovery thread panicked")))
        })
    }
}

impl Drop for KeyboardScanner {
    fn drop(&mut self) {
        if let Some(Err(error)) = self.finish() {
            eprintln!("Cannot finish keyboard discovery: {error}");
        }
    }
}

fn automatic_input(name: Option<&str>, virtual_device: bool, pointer: bool) -> bool {
    !matches!(name, Some(KEYBOARD_NAME | MOUSE_NAME))
        && !pointer
        && (!virtual_device || name == Some(KEYD_KEYBOARD_NAME))
}

// Returns true if `device` is a keyboard fievel should automatically read, printing a
// diagnostic and returning false for anything skipped (virtual, combined pointer, etc).
fn is_auto_candidate(path: &Path, device: &Device, quiet: bool) -> bool {
    let virtual_device = match is_virtual(path) {
        Ok(value) => value,
        Err(error) => {
            if !quiet {
                eprintln!(
                    "Skipping {}: cannot classify input: {error}",
                    path.display()
                );
            }
            return false;
        }
    };
    let pointer = device.supported_events().contains(EventType::RELATIVE)
        || device.supported_events().contains(EventType::ABSOLUTE);
    if automatic_input(device.name(), virtual_device, pointer) {
        true
    } else {
        if !quiet {
            eprintln!(
                "Skipping {} ({}): virtual or combined keyboard/pointer input; \
                 use --device PATH to select it explicitly",
                path.display(),
                device.name().unwrap_or("unnamed"),
            );
        }
        false
    }
}

fn select_devices(path: Option<PathBuf>) -> Result<Vec<(PathBuf, Device)>, Box<dyn Error>> {
    if let Some(path) = path {
        let device = Device::open(&path)
            .map_err(|error| format!("Cannot open {}: {error}", path.display()))?;
        if is_ours(&device) {
            return Err("Refusing to read our own virtual output device".into());
        }
        if !is_keyboard(&device) {
            return Err(format!("{} is not a keyboard", path.display()).into());
        }
        return Ok(vec![(path, device)]);
    }
    let mut devices = Vec::new();
    for (path, device) in keyboards()? {
        if is_auto_candidate(&path, &device, false) {
            devices.push((path, device));
        }
    }
    if devices.is_empty() {
        return Err(
            "No suitable keyboards found; use --list, then --device PATH to select one".into(),
        );
    }
    Ok(devices)
}

fn prepare_reconnected_keyboard(device: &mut Device) -> io::Result<AttributeSet<KeyCode>> {
    device.set_nonblocking(true)?;
    device.grab()?;
    // Discovery leaves the device ungrabbed. Do not replay typing already sent
    // to the desktop while the scan was running.
    for _ in 0..32 {
        let settled = match device.fetch_events() {
            Ok(events) => {
                events.for_each(drop);
                false
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => true,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => false,
            Err(error) => return Err(error),
        };
        if settled {
            return device.get_key_state();
        }
    }
    Err(io::Error::other(
        "Keyboard event queue did not settle during reconnect",
    ))
}

fn attach_new_keyboards(
    candidates: KeyboardScanResult,
    inputs: &mut Vec<(PathBuf, Device)>,
    held: &mut HeldKeys,
    outputs: &mut Outputs,
    engine: &mut Engine,
    indicator: &mut Indicator,
) -> io::Result<()> {
    let candidates = match candidates {
        Ok(candidates) => candidates,
        Err(error) => {
            eprintln!("Cannot scan for new keyboards: {error}");
            return Ok(());
        }
    };
    for (path, mut device) in candidates {
        if inputs.iter().any(|(existing, _)| existing == &path) {
            continue;
        }
        let keys = match prepare_reconnected_keyboard(&mut device) {
            Ok(keys) => keys,
            Err(error) => {
                eprintln!(
                    "Skipping newly connected {}: cannot initialize: {error}",
                    path.display()
                );
                continue;
            }
        };
        eprintln!(
            "Reading newly connected {} ({})",
            path.display(),
            device.name().unwrap_or("unnamed"),
        );
        held.push();
        let index = inputs.len();
        // Seed held keys so modifiers already down when the keyboard reconnects still work.
        for key in keys.iter() {
            if held.key(index, key, 1) {
                outputs.emit(engine.key(key, 1))?;
                indicator.set_active(engine.active());
            }
        }
        inputs.push((path, device));
    }
    Ok(())
}

struct HeldKeys {
    sources: Vec<AttributeSet<KeyCode>>,
}

impl HeldKeys {
    fn new(count: usize) -> Self {
        Self {
            sources: (0..count).map(|_| AttributeSet::new()).collect(),
        }
    }

    fn contains(&self, key: KeyCode) -> bool {
        self.sources.iter().any(|keys| keys.contains(key))
    }

    fn keys(&self) -> AttributeSet<KeyCode> {
        self.sources.iter().flat_map(|keys| keys.iter()).collect()
    }

    fn key(&mut self, source: usize, key: KeyCode, value: i32) -> bool {
        let held = self.contains(key);
        let keys = &mut self.sources[source];
        match value {
            1 => {
                keys.insert(key);
                !held
            }
            0 => {
                let was_held = keys.contains(key);
                keys.remove(key);
                was_held && !self.contains(key)
            }
            2 => keys.contains(key),
            _ => false,
        }
    }

    fn remove(&mut self, source: usize) -> Vec<KeyCode> {
        self.sources
            .remove(source)
            .iter()
            .filter(|key| !self.contains(*key))
            .collect()
    }

    fn push(&mut self) {
        self.sources.push(AttributeSet::new());
    }
}

fn merge_keys<'a>(
    sources: impl IntoIterator<Item = &'a evdev::AttributeSetRef<KeyCode>>,
) -> AttributeSet<KeyCode> {
    sources.into_iter().flat_map(|keys| keys.iter()).collect()
}

fn validate_input_keys(
    supported: &evdev::AttributeSetRef<KeyCode>,
    config: &Config,
) -> Result<(), String> {
    let remapped_outputs = config.remap.output_keys()?;
    for (action, key) in config.keys.named() {
        if !supported.contains(key) && !remapped_outputs.contains(&key) {
            return Err(format!(
                "Selected keyboards do not support {action} = {key:?}; choose another input or binding"
            ));
        }
    }
    for (action, chord) in [
        ("hints.keys.left", &config.hints.keys.left),
        ("hints.keys.right", &config.hints.keys.right),
    ] {
        for key in chord {
            if !supported.contains(*key) && !remapped_outputs.contains(key) {
                return Err(format!(
                    "Selected keyboards do not support {action} = {key:?}; choose another input or binding"
                ));
            }
        }
    }
    for (action, key) in [
        ("hints.keys.cancel", config.hints.keys.cancel),
        (
            "hints.keys.toggle_background",
            config.hints.keys.toggle_background,
        ),
    ] {
        if !supported.contains(key) && !remapped_outputs.contains(&key) {
            return Err(format!(
                "Selected keyboards do not support {action} = {key:?}; choose another input or binding"
            ));
        }
    }
    for key in config.remap.input_keys()? {
        if !supported.contains(key) {
            return Err(format!(
                "Selected keyboards do not support remap input {key:?}; choose another input or binding"
            ));
        }
    }
    Ok(())
}

struct Outputs {
    keyboard: VirtualDevice,
    mouse: VirtualDevice,
    odometer: odometer::Counter,
}

impl Outputs {
    fn new(input_keys: &evdev::AttributeSetRef<KeyCode>, config: &Config) -> io::Result<Self> {
        let keys = keyboard_keys(input_keys, config).map_err(io::Error::other)?;
        let keyboard = VirtualDevice::builder()?
            .name(KEYBOARD_NAME)
            .input_id(InputId::new(BusType::BUS_USB, 0x1209, 0xf301, 1))
            .with_keys(&keys)?
            .build()?;
        let buttons: AttributeSet<KeyCode> = [KeyCode::BTN_LEFT, KeyCode::BTN_RIGHT]
            .into_iter()
            .collect();
        let axes: AttributeSet<RelativeAxisCode> = [
            RelativeAxisCode::REL_X,
            RelativeAxisCode::REL_Y,
            RelativeAxisCode::REL_WHEEL,
            RelativeAxisCode::REL_HWHEEL,
            RelativeAxisCode::REL_WHEEL_HI_RES,
            RelativeAxisCode::REL_HWHEEL_HI_RES,
        ]
        .into_iter()
        .collect();
        let mouse = VirtualDevice::builder()?
            .name(MOUSE_NAME)
            .input_id(InputId::new(BusType::BUS_USB, 0x1209, 0xf302, 1))
            .with_keys(&buttons)?
            .with_relative_axes(&axes)?
            .build()?;
        Ok(Self {
            keyboard,
            mouse,
            odometer: odometer::Counter::default(),
        })
    }

    fn emit(&mut self, output: Output) -> io::Result<()> {
        // Attempt both writes even if one device fails, especially during cleanup.
        let keyboard = if output.keyboard.is_empty() {
            Ok(())
        } else {
            self.keyboard.emit(&output.keyboard)
        };
        let mouse = if output.mouse.is_empty() {
            Ok(())
        } else {
            self.mouse.emit(&output.mouse)
        };
        if mouse.is_ok() {
            self.odometer.record(&output.mouse);
        }
        keyboard.and(mouse)
    }
}

fn keyboard_keys(
    input_keys: &evdev::AttributeSetRef<KeyCode>,
    config: &Config,
) -> Result<AttributeSet<KeyCode>, String> {
    Ok(input_keys
        .iter()
        .chain([KeyCode::KEY_HOME, KeyCode::KEY_END])
        .chain(config.remap.output_keys()?)
        .collect())
}

const RESCAN_INTERVAL: Duration = Duration::from_secs(2);

fn event_loop(
    inputs: &mut Vec<(PathBuf, Device)>,
    held: &mut HeldKeys,
    outputs: &mut Outputs,
    engine: &mut Engine,
    config: &Config,
    stop: &AtomicBool,
    indicator: &mut Indicator,
    mut scanner: Option<&mut KeyboardScanner>,
) -> io::Result<()> {
    let mut last = Instant::now();
    let mut next_scan = last + RESCAN_INTERVAL;
    let mut hint_mode: Option<ActiveHints> = None;
    let mut hint_input = HintInput::new(config).map_err(io::Error::other)?;
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        if hint_mode.is_none() {
            outputs.emit(engine.advance(now.duration_since(last)))?;
            indicator.set_active(engine.active());
            activate_pending_hints(engine, &mut hint_mode, &mut hint_input, held, config);
        } else {
            let actions = hint_input.advance(now.duration_since(last));
            handle_hint_input(actions, &mut hint_mode, config);
        }
        last = now;

        let mut index = 0;
        while index < inputs.len() {
            let (path, input) = &mut inputs[index];
            let disconnected = match input.fetch_events() {
                Ok(events) => {
                    for event in events {
                        if event.event_type() != EventType::KEY {
                            continue;
                        }
                        let key = KeyCode(event.code());
                        let value = event.value();
                        let changed = held.key(index, key, value);
                        if hint_mode.is_some() {
                            if !changed {
                                continue;
                            }
                            let actions = hint_input.key(key, value);
                            handle_hint_input(actions, &mut hint_mode, config);
                            continue;
                        }
                        if changed {
                            outputs.emit(engine.key(key, value))?;
                            indicator.set_active(engine.active());
                            activate_pending_hints(
                                engine,
                                &mut hint_mode,
                                &mut hint_input,
                                held,
                                config,
                            );
                        }
                    }
                    false
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    false
                }
                Err(error)
                    if error.raw_os_error() == Some(rustix::io::Errno::NODEV.raw_os_error()) =>
                {
                    eprintln!("Keyboard disconnected: {}", path.display());
                    true
                }
                Err(error) => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!("Cannot read {}: {error}", path.display()),
                    ));
                }
            };
            if disconnected {
                inputs.remove(index);
                for key in held.remove(index) {
                    if hint_mode.is_some() {
                        let actions = hint_input.key(key, 0);
                        handle_hint_input(actions, &mut hint_mode, config);
                    } else {
                        outputs.emit(engine.key(key, 0))?;
                    }
                }
                indicator.set_active(engine.active());
                if inputs.is_empty() && scanner.is_none() {
                    return Err(io::Error::other(
                        "All keyboards disconnected; restart fievel after reconnecting",
                    ));
                }
            } else {
                index += 1;
            }
        }
        // Retire disconnected handles before attaching replacements: Linux can
        // reuse an event-node path for the newly connected keyboard.
        if let Some(scanner) = scanner.as_deref_mut() {
            if let Some(candidates) = scanner.poll() {
                attach_new_keyboards(candidates, inputs, held, outputs, engine, indicator)?;
                next_scan = Instant::now() + RESCAN_INTERVAL;
            }
            if now >= next_scan {
                scanner.start(inputs.iter().map(|(path, _)| path.clone()).collect())?;
            }
        }
        thread::sleep(TICK.saturating_sub(now.elapsed()));
    }
    Ok(())
}

fn activate_pending_hints(
    engine: &mut Engine,
    hint_mode: &mut Option<ActiveHints>,
    hint_input: &mut HintInput,
    held: &HeldKeys,
    config: &Config,
) {
    if let Some(click) = engine.take_hint_request() {
        match ActiveHints::activate(&config.hints, click) {
            Ok(hints) => {
                hint_input.begin(held.keys().iter());
                *hint_mode = Some(hints);
                if hint_input.readability_held() {
                    handle_hint_input(
                        vec![HintInputEvent::Key(config.hints.keys.toggle_background, 1)],
                        hint_mode,
                        config,
                    );
                }
            }
            Err(error) => ActiveHints::warning(error.as_ref()),
        }
    }
}

fn handle_hint_input(
    actions: Vec<HintInputEvent>,
    hint_mode: &mut Option<ActiveHints>,
    config: &Config,
) {
    for action in actions {
        let Some(hints) = hint_mode.as_mut() else {
            break;
        };
        match action {
            HintInputEvent::Shortcut(click) => {
                if hints.click_kind() == click {
                    hints.cancel();
                    *hint_mode = None;
                } else {
                    hints.set_click_kind(click);
                }
            }
            HintInputEvent::Key(key, value) => match hints.handle_key(key, value, &config.hints) {
                Ok(HintResult::Continue) => {}
                Ok(HintResult::Cancelled | HintResult::Clicked) => *hint_mode = None,
                Err(error) => {
                    ActiveHints::warning(error.as_ref());
                    *hint_mode = None;
                }
            },
        }
    }
}

fn run(args: Args) -> Result<(), Box<dyn Error>> {
    if args.odometer {
        odometer::report(args.odometer_units_per_inch)?;
        return Ok(());
    }
    if args.reset_odometer {
        odometer::reset()?;
        return Ok(());
    }
    if args.list {
        for (path, device) in keyboards()? {
            println!(
                "{}\t{}\t{}",
                path.display(),
                if is_virtual(&path)? {
                    "virtual"
                } else {
                    "physical"
                },
                device.name().unwrap_or("(unnamed)")
            );
        }
        return Ok(());
    }

    let (config_path, config) = load_config(&args)?;
    if args.check_config {
        println!("Config: {}\n{config:#?}", config_path.display());
        return Ok(());
    }
    let explicit = args.device.is_some();
    let inputs = select_devices(args.device)?;
    let supported = merge_keys(
        inputs
            .iter()
            .filter_map(|(_, input)| input.supported_keys()),
    );
    validate_input_keys(&supported, &config)?;
    let stop = Arc::new(AtomicBool::new(false));
    for signal in [
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
        signal_hook::consts::SIGQUIT,
        signal_hook::consts::SIGTSTP,
    ] {
        signal_hook::flag::register(signal, Arc::clone(&stop))?;
    }
    let mut odometer = odometer::Odometer::start(Arc::clone(&stop))?;
    let mut outputs = Outputs::new(&supported, &config).map_err(|error| {
        format!("Cannot create uinput devices: {error}. Check /dev/uinput access")
    })?;
    outputs.odometer = odometer.counter.clone();
    let mut indicator = Indicator::new(config.notify);
    // Keep discovery alive outside the loop so shutdown releases input before
    // waiting for an in-flight scan to finish.
    let mut scanner = KeyboardScanner::default();
    // Give udev/compositors a chance to discover the outputs before grabbing input.
    thread::sleep(Duration::from_millis(500));
    if stop.load(Ordering::Relaxed) {
        odometer.finish()?;
        return Ok(());
    }
    let mut grabbed = Vec::new();
    for (path, mut input) in inputs {
        let result = input.set_nonblocking(true).and_then(|()| input.grab());
        if let Err(error) = result {
            let message = format!(
                "Cannot grab {}: {error}. Another remapper (such as keyd) may own it",
                path.display()
            );
            if explicit {
                return Err(message.into());
            }
            eprintln!("Skipping input: {message}");
            continue;
        }
        eprintln!(
            "Reading {} ({})",
            path.display(),
            input.name().unwrap_or("unnamed")
        );
        grabbed.push((path, input));
    }
    if grabbed.is_empty() {
        return Err(
            "Cannot grab any keyboards; check input permissions and other remappers".into(),
        );
    }
    let available = merge_keys(
        grabbed
            .iter()
            .filter_map(|(_, input)| input.supported_keys()),
    );
    validate_input_keys(&available, &config)?;
    let mut held = HeldKeys::new(grabbed.len());
    eprintln!(
        "{} {:?} for Free Mouse Mode; Ctrl+C exits.",
        match config.mode {
            config::Mode::Hold => "Hold",
            config::Mode::Toggle => "Press to toggle",
        },
        config.keys.free_mouse,
    );
    let mut engine = Engine::new(config.clone())?;
    // Seeding held keys preserves modifiers if the process starts mid-keypress.
    let result = (|| -> io::Result<()> {
        for (index, (_, input)) in grabbed.iter().enumerate() {
            for key in input.get_key_state()?.iter() {
                if held.key(index, key, 1) {
                    outputs.emit(engine.key(key, 1))?;
                    indicator.set_active(engine.active());
                }
            }
        }
        event_loop(
            &mut grabbed,
            &mut held,
            &mut outputs,
            &mut engine,
            &config,
            &stop,
            &mut indicator,
            if explicit { None } else { Some(&mut scanner) },
        )
    })();
    indicator.set_active(false);
    let release = outputs.emit(engine.release_all());
    let mut ungrab = Ok(());
    for (path, input) in &mut grabbed {
        if let Err(error) = input.ungrab() {
            eprintln!("Failed to ungrab {}: {error}", path.display());
            ungrab = Err(error);
        }
    }
    let saved = odometer.finish();
    if let Err(error) = &saved {
        eprintln!("Cannot save odometer: {error}");
    }
    if let Err(error) = release {
        eprintln!("Failed to release virtual keys/buttons: {error}");
        result?;
        saved?;
        return Err(error.into());
    }
    if let Err(error) = ungrab {
        eprintln!("Failed to ungrab input: {error}");
        result?;
        return Err(error.into());
    }
    result?;
    Ok(())
}

fn main() {
    if let Err(error) = run(Args::parse()) {
        eprintln!("fievel: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slow_keyboard_discovery_does_not_block_or_queue_more_scans() {
        let (release, resume) = std::sync::mpsc::channel();
        let mut scanner = KeyboardScanner {
            worker: Some(thread::spawn(move || {
                resume.recv_timeout(Duration::from_secs(2)).unwrap();
                Ok(Vec::new())
            })),
        };
        let worker_id = scanner.worker.as_ref().unwrap().thread().id();
        assert!(scanner.poll().is_none());
        scanner.start(Vec::new()).unwrap();
        assert_eq!(scanner.worker.as_ref().unwrap().thread().id(), worker_id);
        release.send(()).unwrap();
        assert!(scanner.finish().unwrap().unwrap().is_empty());
        assert!(scanner.poll().is_none());
    }

    #[test]
    fn completed_keyboard_scan_is_delivered_once() {
        let mut scanner = KeyboardScanner {
            worker: Some(thread::spawn(|| Ok(Vec::new()))),
        };
        let deadline = Instant::now() + Duration::from_secs(2);
        while !scanner.worker.as_ref().unwrap().is_finished() {
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        assert!(scanner.poll().unwrap().unwrap().is_empty());
        assert!(scanner.poll().is_none());
    }

    #[test]
    fn keyboard_discovery_errors_are_reported_and_clear_pending_scan() {
        for worker in [
            thread::spawn(|| Err(io::Error::other("scan failed"))),
            thread::spawn(|| panic!("scan panicked")),
        ] {
            let mut scanner = KeyboardScanner {
                worker: Some(worker),
            };
            assert!(scanner.finish().unwrap().is_err());
            assert!(scanner.worker.is_none());
        }
    }

    #[test]
    fn odometer_is_a_standalone_command() {
        assert!(
            Args::try_parse_from(["fievel", "--odometer"])
                .unwrap()
                .odometer
        );
        assert!(Args::try_parse_from(["fievel", "--odometer", "--list"]).is_err());
        assert!(Args::try_parse_from(["fievel", "--odometer", "--check-config"]).is_err());
        assert!(
            Args::try_parse_from(["fievel", "--reset-odometer"])
                .unwrap()
                .reset_odometer
        );
        for conflict in ["--odometer", "--list", "--check-config"] {
            assert!(Args::try_parse_from(["fievel", "--reset-odometer", conflict]).is_err());
        }
        assert_eq!(
            Args::try_parse_from(["fievel", "--odometer", "--odometer-units-per-inch", "800"])
                .unwrap()
                .odometer_units_per_inch,
            800.0
        );
        for invalid in ["0", "-1", "NaN", "inf"] {
            assert!(Args::try_parse_from([
                "fievel",
                "--odometer",
                "--odometer-units-per-inch",
                invalid
            ])
            .is_err());
        }
    }

    #[test]
    fn keyboard_supports_home_end_even_when_source_does_not() {
        let input_keys: AttributeSet<KeyCode> =
            [KeyCode::KEY_A, KeyCode::KEY_F3].into_iter().collect();
        let keys = keyboard_keys(&input_keys, &Config::default()).unwrap();
        for key in [
            KeyCode::KEY_A,
            KeyCode::KEY_F3,
            KeyCode::KEY_HOME,
            KeyCode::KEY_END,
        ] {
            assert!(keys.contains(key));
        }
    }

    #[test]
    fn keyboard_supports_all_remapping_targets_without_source_support() {
        let config = Config::parse(
            "[remap.main]\na = 'super'\ns = 'timeout(ctrl, 175, layer(nav))'\n\
             [remap.layers.nav]\nh = 'left'",
        )
        .unwrap();
        let input_keys: AttributeSet<KeyCode> = [KeyCode::KEY_A].into_iter().collect();
        let keys = keyboard_keys(&input_keys, &config).unwrap();
        for key in [
            KeyCode::KEY_A,
            KeyCode::KEY_LEFTCTRL,
            KeyCode::KEY_LEFTMETA,
            KeyCode::KEY_LEFT,
        ] {
            assert!(keys.contains(key));
        }
    }

    #[test]
    fn defaults_to_all_physical_keyboards_and_keyd_output() {
        for name in [
            Some("Laptop keyboard"),
            Some("USB keyboard"),
            Some("Bluetooth keyboard"),
            None,
        ] {
            assert!(automatic_input(name, false, false));
        }
        assert!(automatic_input(Some(KEYD_KEYBOARD_NAME), true, false));
    }

    #[test]
    fn automatic_selection_excludes_feedback_and_combined_pointer_inputs() {
        for name in [
            Some(KEYBOARD_NAME),
            Some(MOUSE_NAME),
            Some("other remapper"),
            None,
        ] {
            assert!(!automatic_input(name, true, false));
        }
        assert!(!automatic_input(Some(KEYBOARD_NAME), false, false));
        assert!(!automatic_input(
            Some("Keyboard with touchpad"),
            false,
            true
        ));
        assert!(!automatic_input(Some(KEYD_KEYBOARD_NAME), true, true));
    }

    #[test]
    fn virtual_classification_includes_bluetooth_hardware() {
        assert!(is_virtual_input(Path::new(
            "/sys/devices/virtual/input/input42/event42"
        )));
        for path in [
            "/sys/devices/pci0000:00/usb1/input/input1/event1",
            "/sys/devices/virtual/misc/uhid/0005:1234/input/input1/event1",
        ] {
            assert!(!is_virtual_input(Path::new(path)));
        }
    }

    #[test]
    fn capabilities_and_bindings_use_the_union_of_input_keys() {
        let config = Config::parse("[remap.main]\nz = 'f24'").unwrap();
        let first: AttributeSet<KeyCode> = config
            .keys
            .named()
            .into_iter()
            .map(|(_, key)| key)
            .chain(config.hints.keys.left.iter().copied())
            .chain(config.hints.keys.right.iter().copied())
            .chain([
                config.hints.keys.cancel,
                config.hints.keys.toggle_background,
            ])
            .collect();
        let second: AttributeSet<KeyCode> = [KeyCode::KEY_Z].into_iter().collect();
        let third: AttributeSet<KeyCode> = [KeyCode::KEY_VOLUMEUP].into_iter().collect();
        assert!(validate_input_keys(&first, &config).is_err());
        assert!(validate_input_keys(&second, &config).is_err());
        let supported = merge_keys([&*first, &*second, &*third]);
        validate_input_keys(&supported, &config).unwrap();
        let output = keyboard_keys(&supported, &config).unwrap();
        for key in [KeyCode::KEY_Z, KeyCode::KEY_VOLUMEUP, KeyCode::KEY_F24] {
            assert!(output.contains(key));
        }
    }

    #[test]
    fn multiple_keyboards_share_presses_without_premature_releases() {
        let mut held = HeldKeys::new(3);
        let key = KeyCode::KEY_LEFTCTRL;
        assert!(held.key(0, key, 1));
        assert!(!held.key(0, key, 1));
        assert!(!held.key(1, key, 1));
        assert!(!held.key(2, key, 1));
        assert!(!held.key(0, key, 0));
        assert!(!held.key(0, key, 2));
        assert!(held.key(1, key, 2));
        assert!(!held.key(1, key, 0));
        assert!(held.key(2, key, 0));
        assert!(!held.key(2, key, 0));
        assert!(!held.key(2, key, 2));
    }

    #[test]
    fn disconnect_releases_only_keys_not_held_elsewhere() {
        let mut held = HeldKeys::new(3);
        held.key(0, KeyCode::KEY_LEFTCTRL, 1);
        held.key(1, KeyCode::KEY_LEFTCTRL, 1);
        held.key(1, KeyCode::KEY_A, 1);
        held.key(2, KeyCode::KEY_B, 1);
        assert_eq!(held.remove(1), [KeyCode::KEY_A]);
        assert!(held.key(1, KeyCode::KEY_B, 0));
        assert_eq!(held.remove(0), [KeyCode::KEY_LEFTCTRL]);
        assert!(held.remove(0).is_empty());
    }

    #[test]
    fn reconnect_starts_with_fresh_held_keys_and_preserves_other_sources() {
        let mut held = HeldKeys::new(2);
        assert!(held.key(0, KeyCode::KEY_A, 1));
        assert!(held.key(1, KeyCode::KEY_LEFTCTRL, 1));
        assert_eq!(held.remove(0), [KeyCode::KEY_A]);
        held.push();
        assert!(!held.key(1, KeyCode::KEY_A, 2));
        assert!(held.key(1, KeyCode::KEY_A, 1));
        assert!(!held.key(1, KeyCode::KEY_LEFTCTRL, 1));
        assert_eq!(held.remove(1), [KeyCode::KEY_A]);
        assert!(held.key(0, KeyCode::KEY_LEFTCTRL, 0));
    }

    #[test]
    fn three_keyboards_share_mouse_mode_and_release_without_stuck_keys() {
        let mut held = HeldKeys::new(3);
        let mut config = Config::default();
        config.easing.movement = 0.0;
        let mut engine = Engine::new(config).unwrap();
        let mut keyboard = Vec::new();
        let mut mouse = Vec::new();
        for (source, key, value) in [
            (0, KeyCode::KEY_LEFTCTRL, 1),
            (1, KeyCode::KEY_LEFTCTRL, 1),
            (2, KeyCode::KEY_A, 1),
            (0, KeyCode::KEY_LEFTCTRL, 0),
            (2, KeyCode::KEY_A, 0),
            (1, KeyCode::KEY_LEFTCTRL, 0),
            (0, KeyCode::KEY_F3, 1),
            (1, KeyCode::KEY_F3, 1),
            (0, KeyCode::KEY_F3, 0),
            (2, KeyCode::KEY_H, 1),
            (2, KeyCode::KEY_SPACE, 1),
        ] {
            if held.key(source, key, value) {
                let output = engine.key(key, value);
                keyboard.extend(output.keyboard);
                mouse.extend(output.mouse);
            }
        }
        assert_eq!(
            keyboard
                .iter()
                .map(|event| (KeyCode(event.code()), event.value()))
                .collect::<Vec<_>>(),
            [
                (KeyCode::KEY_LEFTCTRL, 1),
                (KeyCode::KEY_A, 1),
                (KeyCode::KEY_A, 0),
                (KeyCode::KEY_LEFTCTRL, 0),
            ]
        );
        assert!(engine.active());
        assert_eq!(mouse.len(), 1);
        assert_eq!(
            (mouse[0].code(), mouse[0].value()),
            (KeyCode::BTN_LEFT.0, 1)
        );
        assert!(engine
            .advance(Duration::from_millis(20))
            .mouse
            .iter()
            .any(|event| { event.code() == RelativeAxisCode::REL_X.0 && event.value() < 0 }));
        for key in held.remove(1) {
            let output = engine.key(key, 0);
            assert!(output.keyboard.is_empty());
            mouse.extend(output.mouse);
        }
        assert!(!engine.active());
        assert_eq!(
            (mouse[1].code(), mouse[1].value()),
            (KeyCode::BTN_LEFT.0, 0)
        );
        assert!(held.key(1, KeyCode::KEY_H, 0));
        assert!(engine.key(KeyCode::KEY_H, 0).keyboard.is_empty());
        assert!(held.key(1, KeyCode::KEY_SPACE, 0));
        assert!(engine.key(KeyCode::KEY_SPACE, 0).mouse.is_empty());
        let release = engine.release_all();
        assert!(release.keyboard.is_empty());
        assert!(release.mouse.is_empty());
    }

    #[test]
    fn cli_defaults_to_all_inputs_but_accepts_one_device() {
        assert!(Args::try_parse_from(["fievel"]).unwrap().device.is_none());
        assert_eq!(
            Args::try_parse_from(["fievel", "--device", "/dev/input/event3"])
                .unwrap()
                .device,
            Some(PathBuf::from("/dev/input/event3"))
        );
    }

    #[test]
    fn rejects_invalid_speeds() {
        for value in ["0", "-1", "NaN", "inf", "100001", "garbage"] {
            assert!(positive_speed(value).is_err(), "{value}");
        }
        for value in ["0.1", "800", "100000"] {
            assert!(positive_speed(value).is_ok(), "{value}");
        }
    }

    #[test]
    fn cli_only_overrides_explicit_speed_options() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/fievel.config");
        let args = Args::try_parse_from(["fievel", "--config", path]).unwrap();
        let (_, config) = load_config(&args).unwrap();
        assert_eq!(config.speeds.normal, 300.0);
        assert_eq!(config.speeds.scroll, 6.0);
        let args = Args::try_parse_from([
            "fievel",
            "--config",
            path,
            "--speed",
            "1200",
            "--scroll-speed",
            "12",
        ])
        .unwrap();
        let (_, config) = load_config(&args).unwrap();
        assert_eq!(config.speeds.normal, 1200.0);
        assert_eq!(config.speeds.scroll, 12.0);
        assert_eq!(config.speeds.slow, 100.0);
        assert_eq!(config.speeds.fast, 900.0);
        assert_eq!(config.speeds.scroll_slow, 1.5);
        assert_eq!(config.speeds.scroll_fast, 24.0);
    }

    #[test]
    #[ignore = "requires /dev/uinput and /dev/input access; creates isolated virtual devices"]
    fn uinput_round_trip() -> Result<(), Box<dyn Error>> {
        fn open_output(device: &mut VirtualDevice) -> Result<Device, Box<dyn Error>> {
            let path = device
                .enumerate_dev_nodes_blocking()?
                .next()
                .ok_or("virtual device has no event node")??;
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut input = loop {
                match Device::open(&path) {
                    Ok(input) => break input,
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
                        ) && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            input.set_nonblocking(true)?;
            // Never emit test input unless the desktop is excluded from receiving it.
            input.grab()?;
            Ok(input)
        }

        fn read_events(input: &mut Device) -> io::Result<Vec<(u16, u16, i32)>> {
            let mut received = Vec::new();
            loop {
                match input.fetch_events() {
                    Ok(events) => received.extend(
                        events
                            .filter(|event| event.event_type() != EventType::SYNCHRONIZATION)
                            .map(|event| (event.event_type().0, event.code(), event.value())),
                    ),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(received),
                    Err(error) => return Err(error),
                }
            }
        }

        let keys: AttributeSet<KeyCode> = [
            KeyCode::KEY_A,
            KeyCode::KEY_F3,
            KeyCode::KEY_H,
            KeyCode::KEY_N,
            KeyCode::KEY_COMMA,
            KeyCode::KEY_DOT,
            KeyCode::KEY_SPACE,
            KeyCode::KEY_I,
            KeyCode::KEY_M,
        ]
        .into_iter()
        .collect();
        let mut source = VirtualDevice::builder()?
            .name("fievel test source")
            .with_keys(&keys)?
            .build()?;
        let mut input = open_output(&mut source)?;
        let mut outputs = Outputs::new(input.supported_keys().unwrap(), &Config::default())?;
        let mut keyboard = open_output(&mut outputs.keyboard)?;
        let mut mouse = open_output(&mut outputs.mouse)?;
        let axes = mouse
            .supported_relative_axes()
            .expect("pointer relative axes");
        for axis in [
            RelativeAxisCode::REL_WHEEL,
            RelativeAxisCode::REL_HWHEEL,
            RelativeAxisCode::REL_WHEEL_HI_RES,
            RelativeAxisCode::REL_HWHEEL_HI_RES,
        ] {
            assert!(axes.contains(axis));
        }
        let mut config = Config::default();
        config.speeds.normal = 1000.0;
        config.speeds.scroll = 10.0;
        let mut engine = Engine::new(config.clone())?;
        let mut expected_keyboard = Vec::new();
        let mut expected_mouse = Vec::new();
        for (key, value) in [
            (KeyCode::KEY_A, 1),
            (KeyCode::KEY_A, 0),
            (KeyCode::KEY_F3, 1),
            (KeyCode::KEY_M, 1),
            (KeyCode::KEY_COMMA, 1),
            (KeyCode::KEY_M, 0),
            (KeyCode::KEY_COMMA, 0),
            (KeyCode::KEY_N, 1),
            (KeyCode::KEY_DOT, 1),
            (KeyCode::KEY_N, 0),
            (KeyCode::KEY_DOT, 0),
            (KeyCode::KEY_H, 1),
            (KeyCode::KEY_SPACE, 1),
            (KeyCode::KEY_I, 1),
            (KeyCode::KEY_M, 1),
            (KeyCode::KEY_F3, 0),
            (KeyCode::KEY_H, 0),
            (KeyCode::KEY_SPACE, 0),
            (KeyCode::KEY_I, 0),
            (KeyCode::KEY_M, 0),
        ] {
            source.emit(&[evdev::InputEvent::new(EventType::KEY.0, key.0, value)])?;
            let received = read_events(&mut input)?;
            assert_eq!(received, vec![(EventType::KEY.0, key.0, value)]);
            for (_, code, value) in received {
                for output in [
                    engine.key(KeyCode(code), value),
                    engine.advance(Duration::from_millis(10)),
                ] {
                    expected_keyboard.extend(
                        output
                            .keyboard
                            .iter()
                            .map(|event| (event.event_type().0, event.code(), event.value())),
                    );
                    expected_mouse.extend(
                        output
                            .mouse
                            .iter()
                            .map(|event| (event.event_type().0, event.code(), event.value())),
                    );
                    outputs.emit(output)?;
                }
            }
        }
        assert_eq!(
            expected_keyboard,
            vec![
                (EventType::KEY.0, KeyCode::KEY_A.0, 1),
                (EventType::KEY.0, KeyCode::KEY_A.0, 0),
                (EventType::KEY.0, KeyCode::KEY_HOME.0, 1),
                (EventType::KEY.0, KeyCode::KEY_HOME.0, 0),
                (EventType::KEY.0, KeyCode::KEY_END.0, 1),
                (EventType::KEY.0, KeyCode::KEY_END.0, 0),
            ]
        );
        assert!(!expected_mouse.is_empty());
        assert!(expected_mouse.iter().any(|event| {
            event.0 == EventType::RELATIVE.0 && event.1 == RelativeAxisCode::REL_WHEEL_HI_RES.0
        }));
        assert_eq!(read_events(&mut keyboard)?, expected_keyboard);
        assert_eq!(read_events(&mut mouse)?, expected_mouse);
        assert!(engine.release_all().mouse.is_empty());

        source.emit(&[
            evdev::InputEvent::new(EventType::KEY.0, KeyCode::KEY_A.0, 1),
            evdev::InputEvent::new(EventType::KEY.0, KeyCode::KEY_A.0, 0),
            evdev::InputEvent::new(EventType::KEY.0, KeyCode::KEY_H.0, 1),
        ])?;
        let held = prepare_reconnected_keyboard(&mut input)?;
        assert_eq!(held.iter().collect::<Vec<_>>(), [KeyCode::KEY_H]);
        assert!(read_events(&mut input)?.is_empty());
        source.emit(&[evdev::InputEvent::new(
            EventType::KEY.0,
            KeyCode::KEY_H.0,
            0,
        )])?;
        assert_eq!(
            read_events(&mut input)?,
            [(EventType::KEY.0, KeyCode::KEY_H.0, 0)]
        );
        Ok(())
    }
}
