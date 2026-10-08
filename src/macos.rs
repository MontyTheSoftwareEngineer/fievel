use crate::{
    config::{self, Config},
    engine::Output,
    hint_input::{HintInput, HintInputEvent},
    hints::{ActiveHints, HintResult},
    indicator::Overlays,
    odometer,
    pipeline::InputEngine,
};
use clap::Parser;
use evdev::{EventType, KeyCode as K, RelativeAxisCode as R};
use std::{
    collections::BTreeSet,
    error::Error,
    ffi::c_void,
    io,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

const TICK: Duration = Duration::from_millis(4);

#[derive(Parser)]
#[command(
    version,
    about = "Keyboard remapping and mouse control for macOS (default: hold F3)"
)]
struct Args {
    /// Report saved mouse-movement totals without capturing input
    #[arg(long, conflicts_with_all = ["reset_odometer", "check_config"])]
    odometer: bool,

    /// Reset both odometer totals, including in a running instance
    #[arg(long, conflicts_with_all = ["odometer", "check_config"])]
    reset_odometer: bool,

    /// Reference scale for estimated miles (macOS points per inch)
    #[arg(long, default_value = "96", requires = "odometer", value_parser = positive_speed)]
    odometer_units_per_inch: f64,

    /// Config file (default: ~/.config/fievel/fievel.config)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Print the effective config and exit without capturing input
    #[arg(long)]
    check_config: bool,

    /// Override normal pointer speed from config (macOS points per second)
    #[arg(long, value_parser = positive_speed)]
    speed: Option<f64>,

    /// Override normal scroll speed from config (notches per second)
    #[arg(long, value_parser = positive_speed)]
    scroll_speed: Option<f64>,
}

#[repr(C)]
struct NativeInputEvent {
    kind: i32,
    keycode: u16,
    value: i32,
    repeat: bool,
    dx: f64,
    dy: f64,
    original: *mut c_void,
}

struct NativeInputEventGuard(NativeInputEvent);

impl Drop for NativeInputEventGuard {
    fn drop(&mut self) {
        unsafe { fievel_input_release(&mut self.0) };
    }
}

unsafe extern "C" {
    fn fievel_input_start() -> bool;
    fn fievel_input_next(event: *mut NativeInputEvent, wait_millis: u32) -> bool;
    fn fievel_input_release(event: *mut NativeInputEvent);
    fn fievel_input_repost(event: *mut NativeInputEvent);
    fn fievel_send_key(keycode: u16, value: i32) -> bool;
    fn fievel_move_pointer(dx: f64, dy: f64) -> bool;
    fn fievel_mouse_button(right: bool, down: bool) -> bool;
    fn fievel_scroll(vertical: i32, horizontal: i32) -> bool;
    fn fievel_ui_pump();
    fn fievel_ui_locate_cursor();
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
    validate_key_mappings(&config)?;
    Ok((path, config))
}

fn validate_key_mappings(config: &Config) -> Result<(), Box<dyn Error>> {
    let mut keys: Vec<K> = config
        .keys
        .named()
        .into_iter()
        .map(|(_, key)| key)
        .chain(config.hints.keys.left.iter().copied())
        .chain(config.hints.keys.right.iter().copied())
        .chain([
            config.hints.keys.cancel,
            config.hints.keys.debug,
            config.hints.keys.toggle_background,
        ])
        .collect();
    keys.extend(config.remap.input_keys()?);
    keys.extend(config.remap.output_keys()?);
    if config.keycast.enabled {
        keys.push(config.keycast.hotkey);
    }
    for key in keys {
        if mac_keycode(key).is_none() {
            return Err(format!(
                "Key {key:?} is not supported by the macOS keyboard backend; choose a standard keyboard key"
            )
            .into());
        }
    }
    Ok(())
}

fn run_inner(args: Args) -> Result<(), Box<dyn Error>> {
    if args.odometer {
        odometer::report(args.odometer_units_per_inch)?;
        return Ok(());
    }
    if args.reset_odometer {
        odometer::reset()?;
        return Ok(());
    }
    let (config_path, config) = load_config(&args)?;
    if args.check_config {
        println!("Config: {}\n{config:#?}", config_path.display());
        return Ok(());
    }
    let stop = Arc::new(AtomicBool::new(false));
    for signal in [
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
        signal_hook::consts::SIGQUIT,
    ] {
        signal_hook::flag::register(signal, Arc::clone(&stop))?;
    }
    let mut indicator = Overlays::new(config.notify, config.keycast.enabled);
    let mut odometer = odometer::Odometer::start(Arc::clone(&stop))?;
    if !unsafe { fievel_input_start() } {
        return Err(
            "cannot capture or synthesize global input. Grant Fievel/Terminal Input Monitoring \
             and Accessibility permission in System Settings > Privacy & Security, then quit and restart it"
                .into(),
        );
    }
    eprintln!(
        "{} {:?} for Free Mouse Mode; Ctrl+C exits. \
         macOS may ask for Input Monitoring and Accessibility permission.",
        match config.mode {
            config::Mode::Hold => "Hold",
            config::Mode::Toggle => "Press to toggle",
        },
        config.keys.free_mouse,
    );
    let mut engine = InputEngine::new(config.clone())?;
    let mut hint_input = HintInput::new(&config).map_err(io::Error::other)?;
    let mut hints: Option<ActiveHints> = None;
    let mut held = BTreeSet::new();
    let mut last = Instant::now();
    let result = (|| -> Result<(), Box<dyn Error>> {
        while !stop.load(Ordering::Relaxed) {
            let now = Instant::now();
            let elapsed = now.saturating_duration_since(last);
            last = now;
            if hints.is_none() {
                emit(engine.advance(elapsed), &mut odometer)?;
                indicator.update(
                    engine.active(),
                    engine.keycast_keys(),
                    engine.speed_mode(),
                    engine.hold_locked(),
                );
                activate_pending_hints(&mut engine, &mut hints, &mut hint_input, &held, &config);
            } else {
                let actions = hint_input.advance(elapsed);
                handle_hint_input(actions, &mut hints, &config);
            }

            let mut event = NativeInputEvent {
                kind: 0,
                keycode: 0,
                value: 0,
                repeat: false,
                dx: 0.0,
                dy: 0.0,
                original: std::ptr::null_mut(),
            };
            if unsafe { fievel_input_next(&mut event, TICK.as_millis() as u32) } {
                let mut event = NativeInputEventGuard(event);
                match event.0.kind {
                    1 => {
                        let Some(key) = linux_keycode(event.0.keycode) else {
                            unsafe { fievel_input_repost(&mut event.0) };
                            continue;
                        };
                        let value = if event.0.value == 1 && event.0.repeat {
                            2
                        } else {
                            event.0.value
                        };
                        let changed = match value {
                            1 => {
                                let fresh = held.insert(key);
                                fresh
                            }
                            0 => held.remove(&key),
                            2 => held.contains(&key),
                            _ => false,
                        };
                        if changed {
                            if hints.is_some() {
                                let actions = hint_input.key(key, value);
                                handle_hint_input(actions, &mut hints, &config);
                            } else {
                                emit(engine.key(key, value), &mut odometer)?;
                                indicator.update(
                                    engine.active(),
                                    engine.keycast_keys(),
                                    engine.speed_mode(),
                                    engine.hold_locked(),
                                );
                                activate_pending_hints(
                                    &mut engine,
                                    &mut hints,
                                    &mut hint_input,
                                    &held,
                                    &config,
                                );
                            }
                        }
                    }
                    2 => {
                        odometer
                            .physical_counter
                            .record_pointer_motion(event.0.dx, event.0.dy);
                    }
                    _ => {}
                }
            }
            unsafe { fievel_ui_pump() };
            thread::sleep(TICK.saturating_sub(last.elapsed()));
        }
        Ok(())
    })();
    if let Some(active) = hints.as_mut() {
        active.cancel();
    }
    indicator.clear();
    let release = emit(engine.release_all(), &mut odometer);
    let saved = odometer.finish();
    result?;
    release?;
    saved?;
    Ok(())
}

fn activate_pending_hints(
    engine: &mut InputEngine,
    mode: &mut Option<ActiveHints>,
    input: &mut HintInput,
    held: &BTreeSet<K>,
    config: &Config,
) {
    if let Some(click) = engine.take_hint_request() {
        match ActiveHints::activate(&config.hints, click) {
            Ok(hints) => {
                input.begin(held.iter().copied());
                *mode = Some(hints);
                if input.readability_held() {
                    handle_hint_input(
                        vec![HintInputEvent::Key(config.hints.keys.toggle_background, 1)],
                        mode,
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
    mode: &mut Option<ActiveHints>,
    config: &Config,
) {
    for action in actions {
        let Some(hints) = mode.as_mut() else {
            break;
        };
        match action {
            HintInputEvent::Shortcut(click) => {
                if hints.click_kind() == click {
                    hints.cancel();
                    *mode = None;
                } else {
                    hints.set_click_kind(click);
                }
            }
            HintInputEvent::Key(key, value) => match hints.handle_key(key, value, &config.hints) {
                Ok(HintResult::Continue) => {}
                Ok(HintResult::Cancelled | HintResult::Clicked) => *mode = None,
                Err(error) => {
                    ActiveHints::warning(error.as_ref());
                    *mode = None;
                }
            },
        }
    }
}

fn emit(output: Output, odometer: &mut odometer::Odometer) -> io::Result<()> {
    if output.locate {
        unsafe { fievel_ui_locate_cursor() };
    }
    for event in &output.keyboard {
        if event.event_type() != EventType::KEY {
            continue;
        }
        let key = K(event.code());
        let code = mac_keycode(key).ok_or_else(|| {
            io::Error::other(format!("no macOS virtual keycode exists for {key:?}"))
        })?;
        if !unsafe { fievel_send_key(code, event.value()) } {
            return Err(io::Error::other(format!(
                "failed to emit keyboard event for {key:?}"
            )));
        }
    }

    let (mut dx, mut dy) = (0.0, 0.0);
    for event in &output.mouse {
        match event.event_type() {
            EventType::KEY => {
                flush_motion(&mut dx, &mut dy)?;
                match K(event.code()) {
                    K::BTN_LEFT => emit_mouse_button(false, event.value())?,
                    K::BTN_RIGHT => emit_mouse_button(true, event.value())?,
                    _ => {}
                }
            }
            EventType::RELATIVE => match R(event.code()) {
                R::REL_X => dx += f64::from(event.value()),
                R::REL_Y => dy += f64::from(event.value()),
                R::REL_HWHEEL => {
                    flush_motion(&mut dx, &mut dy)?;
                    if !unsafe { fievel_scroll(0, event.value()) } {
                        return Err(io::Error::other("failed to emit macOS scroll event"));
                    }
                }
                R::REL_WHEEL => {
                    flush_motion(&mut dx, &mut dy)?;
                    if !unsafe { fievel_scroll(event.value(), 0) } {
                        return Err(io::Error::other("failed to emit macOS scroll event"));
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    flush_motion(&mut dx, &mut dy)?;
    odometer.counter.record(&output.mouse);
    Ok(())
}

fn flush_motion(dx: &mut f64, dy: &mut f64) -> io::Result<()> {
    if *dx != 0.0 || *dy != 0.0 {
        if !unsafe { fievel_move_pointer(*dx, *dy) } {
            return Err(io::Error::other("failed to move the macOS pointer"));
        }
        *dx = 0.0;
        *dy = 0.0;
    }
    Ok(())
}

fn emit_mouse_button(right: bool, value: i32) -> io::Result<()> {
    if !unsafe { fievel_mouse_button(right, value != 0) } {
        return Err(io::Error::other("failed to emit macOS mouse-button event"));
    }
    Ok(())
}

fn linux_keycode(keycode: u16) -> Option<K> {
    Some(match keycode {
        0 => K::KEY_A,
        1 => K::KEY_S,
        2 => K::KEY_D,
        3 => K::KEY_F,
        4 => K::KEY_H,
        5 => K::KEY_G,
        6 => K::KEY_Z,
        7 => K::KEY_X,
        8 => K::KEY_C,
        9 => K::KEY_V,
        10 => K::KEY_102ND,
        11 => K::KEY_B,
        12 => K::KEY_Q,
        13 => K::KEY_W,
        14 => K::KEY_E,
        15 => K::KEY_R,
        16 => K::KEY_Y,
        17 => K::KEY_T,
        18 => K::KEY_1,
        19 => K::KEY_2,
        20 => K::KEY_3,
        21 => K::KEY_4,
        22 => K::KEY_6,
        23 => K::KEY_5,
        24 => K::KEY_EQUAL,
        25 => K::KEY_9,
        26 => K::KEY_7,
        27 => K::KEY_MINUS,
        28 => K::KEY_8,
        29 => K::KEY_0,
        30 => K::KEY_RIGHTBRACE,
        31 => K::KEY_O,
        32 => K::KEY_U,
        33 => K::KEY_LEFTBRACE,
        34 => K::KEY_I,
        35 => K::KEY_P,
        36 => K::KEY_ENTER,
        37 => K::KEY_L,
        38 => K::KEY_J,
        39 => K::KEY_APOSTROPHE,
        40 => K::KEY_K,
        41 => K::KEY_SEMICOLON,
        42 => K::KEY_BACKSLASH,
        43 => K::KEY_COMMA,
        44 => K::KEY_SLASH,
        45 => K::KEY_N,
        46 => K::KEY_M,
        47 => K::KEY_DOT,
        48 => K::KEY_TAB,
        49 => K::KEY_SPACE,
        50 => K::KEY_GRAVE,
        51 => K::KEY_BACKSPACE,
        53 => K::KEY_ESC,
        54 => K::KEY_RIGHTMETA,
        55 => K::KEY_LEFTMETA,
        56 => K::KEY_LEFTSHIFT,
        57 => K::KEY_CAPSLOCK,
        58 => K::KEY_LEFTALT,
        59 => K::KEY_LEFTCTRL,
        60 => K::KEY_RIGHTSHIFT,
        61 => K::KEY_RIGHTALT,
        62 => K::KEY_RIGHTCTRL,
        96 => K::KEY_F5,
        97 => K::KEY_F6,
        98 => K::KEY_F7,
        99 => K::KEY_F3,
        100 => K::KEY_F8,
        101 => K::KEY_F9,
        103 => K::KEY_F11,
        105 => K::KEY_F13,
        106 => K::KEY_F16,
        107 => K::KEY_F14,
        109 => K::KEY_F10,
        111 => K::KEY_F12,
        113 => K::KEY_F15,
        118 => K::KEY_F4,
        114 => K::KEY_INSERT,
        120 => K::KEY_F2,
        122 => K::KEY_F1,
        64 => K::KEY_F17,
        65 => K::KEY_KPDOT,
        67 => K::KEY_KPASTERISK,
        69 => K::KEY_KPPLUS,
        71 => K::KEY_NUMLOCK,
        75 => K::KEY_KPSLASH,
        76 => K::KEY_KPENTER,
        78 => K::KEY_KPMINUS,
        81 => K::KEY_KPEQUAL,
        82 => K::KEY_KP0,
        83 => K::KEY_KP1,
        84 => K::KEY_KP2,
        85 => K::KEY_KP3,
        86 => K::KEY_KP4,
        87 => K::KEY_KP5,
        88 => K::KEY_KP6,
        89 => K::KEY_KP7,
        91 => K::KEY_KP8,
        92 => K::KEY_KP9,
        93 => K::KEY_YEN,
        94 => K::KEY_RO,
        95 => K::KEY_KPCOMMA,
        79 => K::KEY_F18,
        80 => K::KEY_F19,
        90 => K::KEY_F20,
        115 => K::KEY_HOME,
        116 => K::KEY_PAGEUP,
        117 => K::KEY_DELETE,
        119 => K::KEY_END,
        121 => K::KEY_PAGEDOWN,
        123 => K::KEY_LEFT,
        124 => K::KEY_RIGHT,
        125 => K::KEY_DOWN,
        126 => K::KEY_UP,
        _ => return None,
    })
}

fn mac_keycode(key: K) -> Option<u16> {
    Some(match key {
        K::KEY_A => 0,
        K::KEY_S => 1,
        K::KEY_D => 2,
        K::KEY_F => 3,
        K::KEY_H => 4,
        K::KEY_G => 5,
        K::KEY_Z => 6,
        K::KEY_X => 7,
        K::KEY_C => 8,
        K::KEY_V => 9,
        K::KEY_102ND => 10,
        K::KEY_B => 11,
        K::KEY_Q => 12,
        K::KEY_W => 13,
        K::KEY_E => 14,
        K::KEY_R => 15,
        K::KEY_Y => 16,
        K::KEY_T => 17,
        K::KEY_1 => 18,
        K::KEY_2 => 19,
        K::KEY_3 => 20,
        K::KEY_4 => 21,
        K::KEY_6 => 22,
        K::KEY_5 => 23,
        K::KEY_EQUAL => 24,
        K::KEY_9 => 25,
        K::KEY_7 => 26,
        K::KEY_MINUS => 27,
        K::KEY_8 => 28,
        K::KEY_0 => 29,
        K::KEY_RIGHTBRACE => 30,
        K::KEY_O => 31,
        K::KEY_U => 32,
        K::KEY_LEFTBRACE => 33,
        K::KEY_I => 34,
        K::KEY_P => 35,
        K::KEY_ENTER => 36,
        K::KEY_L => 37,
        K::KEY_J => 38,
        K::KEY_APOSTROPHE => 39,
        K::KEY_K => 40,
        K::KEY_SEMICOLON => 41,
        K::KEY_BACKSLASH => 42,
        K::KEY_COMMA => 43,
        K::KEY_SLASH => 44,
        K::KEY_N => 45,
        K::KEY_M => 46,
        K::KEY_DOT => 47,
        K::KEY_TAB => 48,
        K::KEY_SPACE => 49,
        K::KEY_GRAVE => 50,
        K::KEY_BACKSPACE => 51,
        K::KEY_ESC => 53,
        K::KEY_RIGHTMETA => 54,
        K::KEY_LEFTMETA => 55,
        K::KEY_LEFTSHIFT => 56,
        K::KEY_CAPSLOCK => 57,
        K::KEY_LEFTALT => 58,
        K::KEY_LEFTCTRL => 59,
        K::KEY_RIGHTSHIFT => 60,
        K::KEY_RIGHTALT => 61,
        K::KEY_RIGHTCTRL => 62,
        K::KEY_F5 => 96,
        K::KEY_F6 => 97,
        K::KEY_F7 => 98,
        K::KEY_F3 => 99,
        K::KEY_F8 => 100,
        K::KEY_F9 => 101,
        K::KEY_F13 => 105,
        K::KEY_F16 => 106,
        K::KEY_F14 => 107,
        K::KEY_F11 => 103,
        K::KEY_F10 => 109,
        K::KEY_F12 => 111,
        K::KEY_F15 => 113,
        K::KEY_F4 => 118,
        K::KEY_F17 => 64,
        K::KEY_F18 => 79,
        K::KEY_F19 => 80,
        K::KEY_F20 => 90,
        K::KEY_KPDOT => 65,
        K::KEY_KPASTERISK => 67,
        K::KEY_KPPLUS => 69,
        K::KEY_NUMLOCK => 71,
        K::KEY_KPSLASH => 75,
        K::KEY_KPENTER => 76,
        K::KEY_KPMINUS => 78,
        K::KEY_KPEQUAL => 81,
        K::KEY_KP0 => 82,
        K::KEY_KP1 => 83,
        K::KEY_KP2 => 84,
        K::KEY_KP3 => 85,
        K::KEY_KP4 => 86,
        K::KEY_KP5 => 87,
        K::KEY_KP6 => 88,
        K::KEY_KP7 => 89,
        K::KEY_KP8 => 91,
        K::KEY_KP9 => 92,
        K::KEY_YEN => 93,
        K::KEY_RO => 94,
        K::KEY_KPCOMMA => 95,
        K::KEY_HOME => 115,
        K::KEY_PAGEUP => 116,
        K::KEY_DELETE => 117,
        K::KEY_INSERT => 114,
        K::KEY_END => 119,
        K::KEY_F2 => 120,
        K::KEY_PAGEDOWN => 121,
        K::KEY_F1 => 122,
        K::KEY_LEFT => 123,
        K::KEY_RIGHT => 124,
        K::KEY_DOWN => 125,
        K::KEY_UP => 126,
        _ => return None,
    })
}

pub fn run() {
    if let Err(error) = run_inner(Args::parse()) {
        eprintln!("fievel: {error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_controls_round_trip_between_linux_and_macos_keycodes() {
        for key in [
            K::KEY_A,
            K::KEY_H,
            K::KEY_J,
            K::KEY_K,
            K::KEY_L,
            K::KEY_0,
            K::KEY_COMMA,
            K::KEY_DOT,
            K::KEY_SPACE,
            K::KEY_F2,
            K::KEY_F3,
            K::KEY_F8,
            K::KEY_LEFTCTRL,
            K::KEY_LEFTMETA,
            K::KEY_LEFT,
            K::KEY_HOME,
            K::KEY_END,
        ] {
            assert_eq!(linux_keycode(mac_keycode(key).unwrap()), Some(key));
        }
    }

    #[test]
    fn macos_cli_keeps_standalone_odometer_options() {
        assert!(
            Args::try_parse_from(["fievel", "--odometer"])
                .unwrap()
                .odometer
        );
        assert!(
            Args::try_parse_from(["fievel", "--reset-odometer"])
                .unwrap()
                .reset_odometer
        );
        assert!(Args::try_parse_from(["fievel", "--odometer", "--check-config"]).is_err());
    }
}
