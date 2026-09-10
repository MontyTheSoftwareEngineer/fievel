mod config;
mod engine;

use clap::Parser;
use config::Config;
use engine::{Engine, Output};
use evdev::{
    uinput::VirtualDevice, AttributeSet, BusType, Device, EventType, InputId, KeyCode,
    RelativeAxisCode,
};
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
#[command(version, about = "Keyboard-driven mouse control (default: hold F3; F3+Escape exits)")]
struct Args {
    /// Keyboard event node (default: keyd output, otherwise the sole physical keyboard)
    #[arg(short, long)]
    device: Option<PathBuf>,

    /// List accessible keyboards without grabbing them
    #[arg(long)]
    list: bool,

    /// Config file (default: ~/.config/fievel/fievel.config)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Print the effective config and exit without opening input devices
    #[arg(long, conflicts_with = "list")]
    check_config: bool,

    /// Override normal pointer speed from config (input units per second)
    #[arg(long, value_parser = positive_speed)]
    speed: Option<f64>,

    /// Override scroll speed from config (notches per second)
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

fn is_virtual(path: &Path) -> io::Result<bool> {
    let node = path
        .file_name()
        .ok_or_else(|| io::Error::other("input device has no file name"))?;
    let sys_path = fs::canonicalize(Path::new("/sys/class/input").join(node))?;
    Ok(sys_path.starts_with("/sys/devices/virtual"))
}

fn keyboards() -> io::Result<Vec<(PathBuf, Device)>> {
    let mut devices = Vec::new();
    for entry in fs::read_dir("/dev/input")? {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().starts_with("event") {
            continue;
        }
        let path = entry.path();
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

fn select_device(path: Option<PathBuf>) -> Result<(PathBuf, Device), Box<dyn Error>> {
    if let Some(path) = path {
        let device = Device::open(&path)
            .map_err(|error| format!("Cannot open {}: {error}", path.display()))?;
        if is_ours(&device) {
            return Err("Refusing to read our own virtual output device".into());
        }
        if !is_keyboard(&device) {
            return Err(format!("{} is not a keyboard", path.display()).into());
        }
        return Ok((path, device));
    }
    let mut devices = keyboards()?;
    let mut keyd = Vec::new();
    let mut physical = Vec::new();
    for (index, (path, device)) in devices.iter().enumerate() {
        let virtual_device = is_virtual(path)?;
        if device.name() == Some(KEYD_KEYBOARD_NAME) && virtual_device {
            keyd.push(index);
        } else if !virtual_device {
            physical.push(index);
        }
    }
    let index = preferred_device_index(&keyd, &physical)?;
    Ok(devices.remove(index))
}

fn preferred_device_index(keyd: &[usize], physical: &[usize]) -> Result<usize, String> {
    let (candidates, kind) = if keyd.is_empty() {
        (physical, "physical keyboards")
    } else {
        (keyd, "keyd virtual keyboards")
    };
    if let [index] = candidates {
        Ok(*index)
    } else {
        Err(format!(
            "Found {} {kind}; use --list, then --device PATH to select one",
            candidates.len()
        ))
    }
}

struct Outputs {
    keyboard: VirtualDevice,
    mouse: VirtualDevice,
}

impl Outputs {
    fn new(input: &Device) -> io::Result<Self> {
        let keys = input
            .supported_keys()
            .ok_or_else(|| io::Error::other("keyboard has no supported keys"))?;
        let keyboard = VirtualDevice::builder()?
            .name(KEYBOARD_NAME)
            .input_id(InputId::new(BusType::BUS_USB, 0x1209, 0xf301, 1))
            .with_keys(keys)?
            .build()?;
        let buttons: AttributeSet<KeyCode> = [KeyCode::BTN_LEFT, KeyCode::BTN_RIGHT]
            .into_iter()
            .collect();
        let axes: AttributeSet<RelativeAxisCode> = [
            RelativeAxisCode::REL_X,
            RelativeAxisCode::REL_Y,
            RelativeAxisCode::REL_WHEEL,
            RelativeAxisCode::REL_HWHEEL,
        ]
        .into_iter()
        .collect();
        let mouse = VirtualDevice::builder()?
            .name(MOUSE_NAME)
            .input_id(InputId::new(BusType::BUS_USB, 0x1209, 0xf302, 1))
            .with_keys(&buttons)?
            .with_relative_axes(&axes)?
            .build()?;
        Ok(Self { keyboard, mouse })
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
        keyboard.and(mouse)
    }
}

fn event_loop(
    input: &mut Device,
    outputs: &mut Outputs,
    engine: &mut Engine,
    stop: &AtomicBool,
) -> io::Result<()> {
    let mut last = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        outputs.emit(engine.advance(now.duration_since(last)))?;
        last = now;

        match input.fetch_events() {
            Ok(events) => {
                for event in events {
                    if event.event_type() == EventType::KEY {
                        outputs.emit(engine.key(KeyCode(event.code()), event.value()))?;
                        if engine.emergency_exit() {
                            return Ok(());
                        }
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
        thread::sleep(TICK.saturating_sub(now.elapsed()));
    }
    Ok(())
}

fn run(args: Args) -> Result<(), Box<dyn Error>> {
    if args.list {
        for (path, device) in keyboards()? {
            println!(
                "{}\t{}\t{}",
                path.display(),
                if is_virtual(&path)? { "virtual" } else { "physical" },
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
    let (path, mut input) = select_device(args.device)?;
    let supported = input.supported_keys().ok_or("keyboard has no supported keys")?;
    for (action, key) in config.keys.named() {
        if !supported.contains(key) {
            return Err(format!(
                "{} does not support keys.{action} = {key:?}; choose another input or binding",
                path.display()
            )
            .into());
        }
    }
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
    let mut outputs = Outputs::new(&input)
        .map_err(|error| format!("Cannot create uinput devices: {error}. Check /dev/uinput access"))?;
    // Give udev/compositors a chance to discover the outputs before grabbing input.
    thread::sleep(Duration::from_millis(500));
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    input.set_nonblocking(true)?;
    input.grab().map_err(|error| {
        format!(
            "Cannot grab {}: {error}. Another remapper (such as keyd) may own it",
            path.display()
        )
    })?;
    eprintln!(
        "Reading {} ({}). Hold {:?} for Free Mouse Mode; {:?}+{:?} or Ctrl+C exits.",
        path.display(),
        input.name().unwrap_or("unnamed"),
        config.keys.free_mouse,
        config.keys.free_mouse,
        config.keys.exit,
    );
    let mut engine = Engine::new(config);
    // Seeding held keys preserves modifiers if the process starts mid-keypress.
    let result = (|| -> io::Result<()> {
        for key in input.get_key_state()?.iter() {
            outputs.emit(engine.key(key, 1))?;
        }
        if engine.emergency_exit() {
            return Ok(());
        }
        event_loop(&mut input, &mut outputs, &mut engine, &stop)
    })();
    let release = outputs.emit(engine.release_all());
    let ungrab = input.ungrab();
    if let Err(error) = release {
        eprintln!("Failed to release virtual keys/buttons: {error}");
        result?;
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
    fn defaults_to_keyd_even_with_multiple_physical_keyboards() {
        assert_eq!(preferred_device_index(&[2], &[0, 1]), Ok(2));
        assert_eq!(preferred_device_index(&[2], &[]), Ok(2));
    }

    #[test]
    fn falls_back_to_sole_physical_keyboard_without_keyd() {
        assert_eq!(preferred_device_index(&[], &[1]), Ok(1));
    }

    #[test]
    fn ambiguous_or_missing_inputs_require_explicit_selection() {
        assert!(preferred_device_index(&[], &[]).is_err());
        assert!(preferred_device_index(&[], &[0, 1]).is_err());
        assert!(preferred_device_index(&[0, 1], &[2]).is_err());
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
        assert_eq!(config.speeds.normal, 800.0);
        assert_eq!(config.speeds.scroll, 8.0);
        let args = Args::try_parse_from([
            "fievel", "--config", path, "--speed", "1200", "--scroll-speed", "12",
        ])
        .unwrap();
        let (_, config) = load_config(&args).unwrap();
        assert_eq!(config.speeds.normal, 1200.0);
        assert_eq!(config.speeds.scroll, 12.0);
        assert_eq!(config.speeds.slow, 200.0);
        assert_eq!(config.speeds.fast, 1600.0);
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
        let mut outputs = Outputs::new(&input)?;
        let mut keyboard = open_output(&mut outputs.keyboard)?;
        let mut mouse = open_output(&mut outputs.mouse)?;
        let mut config = Config::default();
        config.speeds.normal = 1000.0;
        config.speeds.scroll = 10.0;
        let mut engine = Engine::new(config);
        let mut expected_keyboard = Vec::new();
        let mut expected_mouse = Vec::new();
        for (key, value) in [
            (KeyCode::KEY_A, 1),
            (KeyCode::KEY_A, 0),
            (KeyCode::KEY_F3, 1),
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
                    expected_keyboard.extend(output.keyboard.iter().map(|event| {
                        (event.event_type().0, event.code(), event.value())
                    }));
                    expected_mouse.extend(output.mouse.iter().map(|event| {
                        (event.event_type().0, event.code(), event.value())
                    }));
                    outputs.emit(output)?;
                }
            }
        }
        assert_eq!(
            expected_keyboard,
            vec![
                (EventType::KEY.0, KeyCode::KEY_A.0, 1),
                (EventType::KEY.0, KeyCode::KEY_A.0, 0),
            ]
        );
        assert!(!expected_mouse.is_empty());
        assert_eq!(read_events(&mut keyboard)?, expected_keyboard);
        assert_eq!(read_events(&mut mouse)?, expected_mouse);
        assert!(engine.release_all().mouse.is_empty());
        Ok(())
    }
}
