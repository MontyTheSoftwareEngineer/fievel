use crate::{
    config::{self, Config},
    engine::Output,
    hint_input::{HintInput, HintInputEvent},
    hints::{ActiveHints, HintResult},
    input::{EventType, KeyCode as K, RelativeAxisCode as R},
    odometer,
    pipeline::{HintRequest, InputEngine},
    windows_ui::{Overlay, INPUT_MARKER},
};
use clap::Parser;
use std::{
    collections::BTreeSet,
    error::Error,
    io,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender},
        Arc, Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::core::BOOL;
use windows_sys::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::{
        Console::{
            SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT,
            CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
        },
        LibraryLoader::GetModuleHandleW,
        Threading::GetCurrentThreadId,
    },
    UI::{
        Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
            KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_HWHEEL,
            MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
            MOUSEEVENTF_WHEEL, MOUSEINPUT, VK_F13, VK_PAUSE, VK_SNAPSHOT, VK_VOLUME_UP,
        },
        WindowsAndMessaging::{
            CallNextHookEx, GetCursorPos, GetMessageW, PeekMessageW, PostThreadMessageW,
            SetWindowsHookExW, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, LLKHF_EXTENDED,
            LLKHF_INJECTED, LLKHF_UP, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, WH_KEYBOARD_LL,
            WH_MOUSE_LL, WM_KEYUP, WM_MOUSEMOVE, WM_QUIT, WM_SYSKEYUP,
        },
    },
};

const TICK: Duration = Duration::from_millis(4);
static HOOK_SENDER: OnceLock<Sender<HookEvent>> = OnceLock::new();
static PRESSED_KEYS: OnceLock<Mutex<BTreeSet<K>>> = OnceLock::new();
static PREVIOUS_MOUSE: OnceLock<Mutex<Option<(i32, i32)>>> = OnceLock::new();
static STOP_SIGNAL: OnceLock<Arc<AtomicBool>> = OnceLock::new();

#[derive(Parser)]
#[command(
    version,
    about = "Keyboard remapping and mouse control for Windows (default: hold F3)"
)]
struct Args {
    /// Report saved mouse-movement totals without capturing input
    #[arg(long, conflicts_with_all = ["reset_odometer", "check_config"])]
    odometer: bool,

    /// Reset both odometer totals, including in a running instance
    #[arg(long, conflicts_with_all = ["odometer", "check_config"])]
    reset_odometer: bool,

    /// Reference scale for estimated miles (Windows screen pixels per inch)
    #[arg(long, default_value = "96", requires = "odometer", value_parser = positive_speed)]
    odometer_units_per_inch: f64,

    /// Config file (default: %APPDATA%\\Fievel\\fievel.config)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Print the effective config and exit without capturing input
    #[arg(long)]
    check_config: bool,

    /// Override normal pointer speed from config (screen pixels per second)
    #[arg(long, value_parser = positive_speed)]
    speed: Option<f64>,

    /// Override normal scroll speed from config (notches per second)
    #[arg(long, value_parser = positive_speed)]
    scroll_speed: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
enum HookEvent {
    Key(K, i32),
    PhysicalPointer(f64, f64),
}

struct Hooks {
    receiver: Receiver<HookEvent>,
    thread_id: u32,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl Hooks {
    fn start() -> io::Result<Self> {
        let (sender, receiver) = mpsc::channel();
        HOOK_SENDER
            .set(sender)
            .map_err(|_| io::Error::other("Windows input hooks were already started"))?;
        PRESSED_KEYS.get_or_init(|| Mutex::new(BTreeSet::new()));
        PREVIOUS_MOUSE.get_or_init(|| Mutex::new(None));
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("fievel-windows-input".into())
            .spawn(move || hook_thread(ready_sender))?;
        match ready_receiver.recv() {
            Ok(Ok(thread_id)) => Ok(Self {
                receiver,
                thread_id,
                worker: Some(worker),
            }),
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(error) => {
                let _ = worker.join();
                Err(io::Error::other(format!(
                    "Windows input hook thread exited during startup: {error}"
                )))
            }
        }
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                eprintln!("fievel: Windows input hook thread panicked");
            }
        }
    }
}

fn hook_thread(ready: SyncSender<io::Result<u32>>) -> io::Result<()> {
    let mut message: MSG = unsafe { std::mem::zeroed() };
    unsafe { PeekMessageW(&mut message, std::ptr::null_mut(), 0, 0, 0) };
    let thread_id = unsafe { GetCurrentThreadId() };
    let module = unsafe { GetModuleHandleW(std::ptr::null()) };
    let keyboard = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), module, 0) };
    if keyboard.is_null() {
        let error = io::Error::last_os_error();
        let _ = ready.send(Err(error));
        return Ok(());
    }
    let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0) };
    if mouse.is_null() {
        let error = io::Error::last_os_error();
        unsafe { UnhookWindowsHookEx(keyboard) };
        let _ = ready.send(Err(error));
        return Ok(());
    }
    if ready.send(Ok(thread_id)).is_err() {
        unsafe {
            UnhookWindowsHookEx(mouse);
            UnhookWindowsHookEx(keyboard);
        }
        return Ok(());
    }
    loop {
        let status = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
        if status == 0 {
            break;
        }
        if status == -1 {
            unsafe {
                UnhookWindowsHookEx(mouse);
                UnhookWindowsHookEx(keyboard);
            }
            return Err(io::Error::last_os_error());
        }
    }
    unsafe {
        UnhookWindowsHookEx(mouse);
        UnhookWindowsHookEx(keyboard);
    }
    Ok(())
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    let event = &*(lparam as *const KBDLLHOOKSTRUCT);
    if event.flags & LLKHF_INJECTED != 0 && event.dwExtraInfo == INPUT_MARKER {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    let Some(key) = key_from_scan(
        event.scanCode,
        event.flags & LLKHF_EXTENDED != 0,
        event.vkCode,
    ) else {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    };
    let is_release = event.flags & LLKHF_UP != 0
        || wparam == WM_KEYUP as usize
        || wparam == WM_SYSKEYUP as usize;
    let pressed = PRESSED_KEYS.get_or_init(|| Mutex::new(BTreeSet::new()));
    let value = {
        let Ok(mut pressed) = pressed.lock() else {
            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
        };
        if is_release {
            pressed.remove(&key);
            0
        } else if pressed.insert(key) {
            1
        } else {
            2
        }
    };
    match HOOK_SENDER
        .get()
        .map(|sender| sender.send(HookEvent::Key(key, value)))
    {
        Some(Ok(())) => 1,
        _ => CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam),
    }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code < 0 || wparam != WM_MOUSEMOVE as usize {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }
    let event = &*(lparam as *const MSLLHOOKSTRUCT);
    let injected = event.flags & LLMHF_INJECTED != 0 || event.dwExtraInfo == INPUT_MARKER;
    let previous = PREVIOUS_MOUSE.get_or_init(|| Mutex::new(None));
    if let Ok(mut previous) = previous.lock() {
        let current = (event.pt.x, event.pt.y);
        if let Some((dx, dy)) = physical_pointer_delta(&mut previous, current, injected) {
            if let Some(sender) = HOOK_SENDER.get() {
                let _ = sender.send(HookEvent::PhysicalPointer(dx, dy));
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

fn physical_pointer_delta(
    previous: &mut Option<(i32, i32)>,
    current: (i32, i32),
    injected: bool,
) -> Option<(f64, f64)> {
    let old = previous.replace(current)?;
    if injected {
        return None;
    }
    Some((
        f64::from(current.0.saturating_sub(old.0)),
        f64::from(current.1.saturating_sub(old.1)),
    ))
}

fn key_from_scan(scan: u32, extended: bool, vk: u32) -> Option<K> {
    if extended {
        return Some(match scan {
            0x1c => K::KEY_KPENTER,
            0x1d => K::KEY_RIGHTCTRL,
            0x35 => K::KEY_KPSLASH,
            0x38 => K::KEY_RIGHTALT,
            0x47 => K::KEY_HOME,
            0x48 => K::KEY_UP,
            0x49 => K::KEY_PAGEUP,
            0x4b => K::KEY_LEFT,
            0x4d => K::KEY_RIGHT,
            0x4f => K::KEY_END,
            0x50 => K::KEY_DOWN,
            0x51 => K::KEY_PAGEDOWN,
            0x52 => K::KEY_INSERT,
            0x53 => K::KEY_DELETE,
            0x5b => K::KEY_LEFTMETA,
            0x5c => K::KEY_RIGHTMETA,
            0x5d => K::KEY_MENU,
            _ => return key_from_virtual_key(vk),
        });
    }
    Some(match scan {
        0x01 => K::KEY_ESC,
        0x02..=0x0b => K::new((scan - 0x02 + K::KEY_1.0 as u32) as u16),
        0x0c => K::KEY_MINUS,
        0x0d => K::KEY_EQUAL,
        0x0e => K::KEY_BACKSPACE,
        0x0f => K::KEY_TAB,
        0x10 => K::KEY_Q,
        0x11 => K::KEY_W,
        0x12 => K::KEY_E,
        0x13 => K::KEY_R,
        0x14 => K::KEY_T,
        0x15 => K::KEY_Y,
        0x16 => K::KEY_U,
        0x17 => K::KEY_I,
        0x18 => K::KEY_O,
        0x19 => K::KEY_P,
        0x1a => K::KEY_LEFTBRACE,
        0x1b => K::KEY_RIGHTBRACE,
        0x1c => K::KEY_ENTER,
        0x1d => K::KEY_LEFTCTRL,
        0x1e => K::KEY_A,
        0x1f => K::KEY_S,
        0x20 => K::KEY_D,
        0x21 => K::KEY_F,
        0x22 => K::KEY_G,
        0x23 => K::KEY_H,
        0x24 => K::KEY_J,
        0x25 => K::KEY_K,
        0x26 => K::KEY_L,
        0x27 => K::KEY_SEMICOLON,
        0x28 => K::KEY_APOSTROPHE,
        0x29 => K::KEY_GRAVE,
        0x2a => K::KEY_LEFTSHIFT,
        0x2b => K::KEY_BACKSLASH,
        0x2c => K::KEY_Z,
        0x2d => K::KEY_X,
        0x2e => K::KEY_C,
        0x2f => K::KEY_V,
        0x30 => K::KEY_B,
        0x31 => K::KEY_N,
        0x32 => K::KEY_M,
        0x33 => K::KEY_COMMA,
        0x34 => K::KEY_DOT,
        0x35 => K::KEY_SLASH,
        0x36 => K::KEY_RIGHTSHIFT,
        0x37 => K::KEY_KPASTERISK,
        0x38 => K::KEY_LEFTALT,
        0x39 => K::KEY_SPACE,
        0x3a => K::KEY_CAPSLOCK,
        0x3b..=0x44 => K::new((scan - 0x3b + K::KEY_F1.0 as u32) as u16),
        0x45 => K::KEY_NUMLOCK,
        0x47 => K::KEY_KP7,
        0x48 => K::KEY_KP8,
        0x49 => K::KEY_KP9,
        0x4a => K::KEY_KPMINUS,
        0x4b => K::KEY_KP4,
        0x4c => K::KEY_KP5,
        0x4d => K::KEY_KP6,
        0x4e => K::KEY_KPPLUS,
        0x4f => K::KEY_KP1,
        0x50 => K::KEY_KP2,
        0x51 => K::KEY_KP3,
        0x52 => K::KEY_KP0,
        0x53 => K::KEY_KPDOT,
        0x56 => K::KEY_102ND,
        0x57 => K::KEY_F11,
        0x58 => K::KEY_F12,
        0x73 => K::KEY_RO,
        0x7d => K::KEY_YEN,
        0x7e => K::KEY_KPCOMMA,
        _ => return key_from_virtual_key(vk),
    })
}

fn key_from_virtual_key(vk: u32) -> Option<K> {
    match vk {
        0x70..=0x79 => Some(K::new(K::KEY_F1.0 + (vk - 0x70) as u16)),
        0x7a..=0x7b => Some(K::new(K::KEY_F11.0 + (vk - 0x7a) as u16)),
        0x7c..=0x87 => Some(K::new(K::KEY_F13.0 + (vk - 0x7c) as u16)),
        value if value == VK_VOLUME_UP as u32 => Some(K::KEY_VOLUMEUP),
        0x13 => Some(K::KEY_PAUSE),
        0x2c => Some(K::KEY_SYSRQ),
        _ => None,
    }
}

fn scan_code_for_key(key: K) -> Option<(u16, bool)> {
    let code = key.0;
    let keycode = match code {
        1 => Some((0x01, false)),
        2..=11 => Some((0x02 + code - 2, false)),
        16..=27 => Some((0x10 + code - 16, false)),
        30..=38 => Some((0x1e + code - 30, false)),
        44..=50 => Some((0x2c + code - 44, false)),
        59..=68 => Some((0x3b + code - 59, false)),
        71..=83 => Some((
            match code {
                71 => 0x47,
                72 => 0x48,
                73 => 0x49,
                74 => 0x4a,
                75 => 0x4b,
                76 => 0x4c,
                77 => 0x4d,
                78 => 0x4e,
                79 => 0x4f,
                80 => 0x50,
                81 => 0x51,
                82 => 0x52,
                83 => 0x53,
                _ => unreachable!(),
            },
            false,
        )),
        12 => Some((0x0c, false)),
        13 => Some((0x0d, false)),
        14 => Some((0x0e, false)),
        15 => Some((0x0f, false)),
        28 => Some((0x1c, false)),
        29 => Some((0x1d, false)),
        39 => Some((0x27, false)),
        40 => Some((0x28, false)),
        41 => Some((0x29, false)),
        42 => Some((0x2a, false)),
        43 => Some((0x2b, false)),
        51 => Some((0x33, false)),
        52 => Some((0x34, false)),
        53 => Some((0x35, false)),
        54 => Some((0x36, false)),
        55 => Some((0x37, false)),
        56 => Some((0x38, false)),
        57 => Some((0x39, false)),
        58 => Some((0x3a, false)),
        69 => Some((0x45, false)),
        86 => Some((0x56, false)),
        87 => Some((0x57, false)),
        88 => Some((0x58, false)),
        89 => Some((0x73, false)),
        96 => Some((0x1c, true)),
        97 => Some((0x1d, true)),
        98 => Some((0x35, true)),
        100 => Some((0x38, true)),
        102 => Some((0x47, true)),
        103 => Some((0x48, true)),
        104 => Some((0x49, true)),
        105 => Some((0x4b, true)),
        106 => Some((0x4d, true)),
        107 => Some((0x4f, true)),
        108 => Some((0x50, true)),
        109 => Some((0x51, true)),
        110 => Some((0x52, true)),
        111 => Some((0x53, true)),
        117 => Some((0x59, false)),
        121 => Some((0x7e, false)),
        124 => Some((0x7d, false)),
        125 => Some((0x5b, true)),
        126 => Some((0x5c, true)),
        139 => Some((0x5d, true)),
        _ => None,
    };
    keycode.map(|(scan, extended)| (scan as u16, extended))
}

fn virtual_key_for_key(key: K) -> Option<u16> {
    match key.0 {
        183..=194 => Some(VK_F13 + (key.0 - K::KEY_F13.0)),
        99 => Some(VK_SNAPSHOT),
        119 => Some(VK_PAUSE),
        115 => Some(VK_VOLUME_UP),
        _ => None,
    }
}

fn positive_speed(value: &str) -> Result<f64, String> {
    let speed: f64 = value.parse().map_err(|_| "expected a number".to_owned())?;
    config::validate_speed(speed)?;
    Ok(speed)
}

fn config_path() -> Result<PathBuf, Box<dyn Error>> {
    let base = std::env::var_os("APPDATA")
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join("AppData/Roaming").into_os_string())
        })
        .ok_or("APPDATA and USERPROFILE are unset; specify a config file with --config PATH")?;
    Ok(PathBuf::from(base).join("Fievel/fievel.config"))
}

fn load_config(args: &Args) -> Result<(PathBuf, Config), Box<dyn Error>> {
    let path = args.config.clone().map_or_else(config_path, Ok)?;
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
        if scan_code_for_key(key).is_none() && virtual_key_for_key(key).is_none() {
            return Err(
                format!("Key {key:?} is not supported by the Windows keyboard backend").into(),
            );
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
    STOP_SIGNAL
        .set(Arc::clone(&stop))
        .map_err(|_| io::Error::other("console control handler was already installed"))?;
    if unsafe { SetConsoleCtrlHandler(Some(console_control), 1) } == 0 {
        return Err(io::Error::last_os_error().into());
    }
    let mut overlay = Overlay::new()?;
    let mut odometer = odometer::Odometer::start(Arc::clone(&stop))?;
    let hooks = Hooks::start()?;
    eprintln!(
        "{} {:?} for Free Mouse Mode; Ctrl+C exits.",
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
                emit(engine.advance(elapsed), &mut odometer, &mut overlay)?;
                update_indicator(&mut overlay, &config, &engine)?;
                activate_pending_hints(
                    &mut engine,
                    &mut hints,
                    &mut hint_input,
                    &held,
                    &config,
                    &mut overlay,
                );
            } else {
                let actions = hint_input.advance(elapsed);
                handle_hint_input(actions, &mut hints, &config, &mut overlay)?;
            }

            match hooks.receiver.recv_timeout(TICK) {
                Ok(HookEvent::PhysicalPointer(dx, dy)) => {
                    odometer.physical_counter.record_pointer_motion(dx, dy);
                }
                Ok(HookEvent::Key(key, value)) => {
                    let changed = match value {
                        1 => held.insert(key),
                        0 => held.remove(&key),
                        2 => held.contains(&key),
                        _ => false,
                    };
                    if changed {
                        if hints.is_some() {
                            let actions = hint_input.key(key, value);
                            handle_hint_input(actions, &mut hints, &config, &mut overlay)?;
                        } else {
                            emit(engine.key(key, value), &mut odometer, &mut overlay)?;
                            update_indicator(&mut overlay, &config, &engine)?;
                            activate_pending_hints(
                                &mut engine,
                                &mut hints,
                                &mut hint_input,
                                &held,
                                &config,
                                &mut overlay,
                            );
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Windows input hook stopped unexpectedly".into())
                }
            }
            overlay.pump()?;
        }
        Ok(())
    })();
    if let Some(active) = hints.as_mut() {
        if let Err(error) = active.cancel(&mut overlay) {
            eprintln!("fievel: could not clear hints during shutdown: {error}");
        }
    }
    let mut cleanup_errors = Vec::new();
    if let Err(error) = overlay.set_indicator(None) {
        cleanup_errors.push(error);
    }
    if let Err(error) = emit(engine.release_all(), &mut odometer, &mut overlay) {
        cleanup_errors.push(error);
    }
    if let Err(error) = odometer.finish() {
        cleanup_errors.push(error);
    }
    drop(hooks);
    if unsafe { SetConsoleCtrlHandler(Some(console_control), 0) } == 0 {
        cleanup_errors.push(io::Error::last_os_error());
    }
    if let Err(error) = result {
        for cleanup_error in cleanup_errors {
            eprintln!("fievel: shutdown cleanup failed: {cleanup_error}");
        }
        return Err(error);
    }
    let mut cleanup_errors = cleanup_errors.into_iter();
    if let Some(error) = cleanup_errors.next() {
        for cleanup_error in cleanup_errors {
            eprintln!("fievel: additional shutdown cleanup failure: {cleanup_error}");
        }
        return Err(error.into());
    }
    Ok(())
}

fn update_indicator(
    overlay: &mut Overlay,
    config: &Config,
    engine: &InputEngine,
) -> io::Result<()> {
    let keys = if config.keycast.enabled {
        engine.keycast_keys()
    } else {
        &[]
    };
    let active = engine.active() && (config.notify || (config.keycast.enabled && !keys.is_empty()));
    overlay.set_indicator_state(
        active,
        keys,
        engine.speed_mode(),
        config.notify && engine.hold_locked(),
    )
}

fn activate_pending_hints(
    engine: &mut InputEngine,
    mode: &mut Option<ActiveHints>,
    input: &mut HintInput,
    held: &BTreeSet<K>,
    config: &Config,
    overlay: &mut Overlay,
) {
    if let Some(request) = engine.take_hint_request() {
        let activated = match request {
            HintRequest::Click(click) => ActiveHints::activate(&config.hints, click, overlay),
            HintRequest::Grid => ActiveHints::activate_grid(
                &config.hints,
                crate::hints::ClickKind::Left,
                overlay,
            ),
        };
        match activated {
            Ok(hints) => {
                input.begin(held.iter().copied());
                input.set_grid_mode(hints.is_grid_mode());
                *mode = Some(hints);
                if input.readability_held() {
                    handle_hint_input(
                        vec![HintInputEvent::Key(config.hints.keys.toggle_background, 1)],
                        mode,
                        config,
                        overlay,
                    )
                    .unwrap_or_else(|error| ActiveHints::warning(error.as_ref()));
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
    overlay: &mut Overlay,
) -> Result<(), Box<dyn Error>> {
    for action in actions {
        let Some(hints) = mode.as_mut() else {
            break;
        };
        match action {
            HintInputEvent::Shortcut(click) => {
                if hints.click_kind() == click {
                    hints.cancel(overlay)?;
                    *mode = None;
                } else {
                    hints.set_click_kind(click);
                }
            }
            HintInputEvent::GridShortcut => {
                if hints.is_grid_mode() {
                    hints.cancel(overlay)?;
                    *mode = None;
                } else {
                    let click = hints.click_kind();
                    hints.cancel(overlay)?;
                    match ActiveHints::activate_grid(&config.hints, click, overlay) {
                        Ok(grid) => *mode = Some(grid),
                        Err(error) => {
                            ActiveHints::warning(error.as_ref());
                            *mode = None;
                        }
                    }
                }
            }
            HintInputEvent::Key(key, value) => {
                match hints.handle_key(key, value, &config.hints, overlay)? {
                    HintResult::Continue => {}
                    HintResult::Cancelled | HintResult::Clicked => *mode = None,
                }
            }
        }
    }
    Ok(())
}

fn emit(
    output: Output,
    odometer: &mut odometer::Odometer,
    overlay: &mut Overlay,
) -> io::Result<()> {
    if output.locate {
        overlay.locate_cursor()?;
    }
    for event in &output.keyboard {
        if event.event_type() != EventType::KEY {
            continue;
        }
        send_key(K::new(event.code()), event.value())?;
    }
    let (mut dx, mut dy) = (0, 0);
    for event in &output.mouse {
        match event.event_type() {
            EventType::KEY => {
                flush_motion(&mut dx, &mut dy)?;
                match K::new(event.code()) {
                    K::BTN_LEFT => send_mouse_button(false, event.value() != 0)?,
                    K::BTN_RIGHT => send_mouse_button(true, event.value() != 0)?,
                    _ => {}
                }
            }
            EventType::RELATIVE => match R(event.code()) {
                R::REL_X => dx = dx.saturating_add(event.value()),
                R::REL_Y => dy = dy.saturating_add(event.value()),
                R::REL_HWHEEL_HI_RES => {
                    flush_motion(&mut dx, &mut dy)?;
                    send_scroll(0, event.value())?;
                }
                R::REL_WHEEL_HI_RES => {
                    flush_motion(&mut dx, &mut dy)?;
                    send_scroll(event.value(), 0)?;
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

fn flush_motion(dx: &mut i32, dy: &mut i32) -> io::Result<()> {
    if *dx != 0 || *dy != 0 {
        let mut point = windows_sys::Win32::Foundation::POINT { x: 0, y: 0 };
        if unsafe { GetCursorPos(&mut point) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Overlay::move_cursor_to(point.x.saturating_add(*dx), point.y.saturating_add(*dy))?;
        *dx = 0;
        *dy = 0;
    }
    Ok(())
}

fn send_key(key: K, value: i32) -> io::Result<()> {
    let mut input = INPUT::default();
    input.r#type = INPUT_KEYBOARD;
    let mut flags = 0;
    let (vk, scan) = if let Some((scan, extended)) = scan_code_for_key(key) {
        flags |= KEYEVENTF_SCANCODE;
        if extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        (0, scan)
    } else if let Some(vk) = virtual_key_for_key(key) {
        (vk, 0)
    } else {
        return Err(io::Error::other(format!(
            "no Windows keyboard mapping exists for {key:?}"
        )));
    };
    if value == 0 {
        flags |= KEYEVENTF_KEYUP;
    }
    input.Anonymous = INPUT_0 {
        ki: KEYBDINPUT {
            wVk: vk,
            wScan: scan,
            dwFlags: flags,
            dwExtraInfo: INPUT_MARKER,
            ..KEYBDINPUT::default()
        },
    };
    if unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) } != 1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn send_mouse_button(right: bool, down: bool) -> io::Result<()> {
    let flags = match (right, down) {
        (false, true) => MOUSEEVENTF_LEFTDOWN,
        (false, false) => MOUSEEVENTF_LEFTUP,
        (true, true) => MOUSEEVENTF_RIGHTDOWN,
        (true, false) => MOUSEEVENTF_RIGHTUP,
    };
    send_mouse_input(flags, 0)
}

fn send_scroll(vertical: i32, horizontal: i32) -> io::Result<()> {
    if vertical != 0 {
        send_mouse_input(MOUSEEVENTF_WHEEL, vertical)?;
    }
    if horizontal != 0 {
        send_mouse_input(MOUSEEVENTF_HWHEEL, horizontal)?;
    }
    Ok(())
}

fn send_mouse_input(flags: u32, data: i32) -> io::Result<()> {
    let input = INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                mouseData: data as u32,
                dwFlags: flags,
                dwExtraInfo: INPUT_MARKER,
                ..unsafe { std::mem::zeroed() }
            },
        },
    };
    if unsafe { SendInput(1, &input, std::mem::size_of::<INPUT>() as i32) } != 1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

unsafe extern "system" fn console_control(control: u32) -> BOOL {
    match control {
        CTRL_C_EVENT | CTRL_BREAK_EVENT | CTRL_CLOSE_EVENT | CTRL_LOGOFF_EVENT
        | CTRL_SHUTDOWN_EVENT => {
            if let Some(stop) = STOP_SIGNAL.get() {
                stop.store(true, Ordering::Release);
            }
            1
        }
        _ => 0,
    }
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
    fn configured_default_keys_have_windows_scan_codes() {
        validate_key_mappings(&Config::default()).unwrap();
        let config = Config::parse(include_str!("../fievel.config")).unwrap();
        validate_key_mappings(&config).unwrap();
    }

    #[test]
    fn physical_scan_codes_keep_letters_and_modifier_sides_distinct() {
        for (scan, extended, expected) in [
            (0x1e, false, K::KEY_A),
            (0x23, false, K::KEY_H),
            (0x2c, false, K::KEY_Z),
            (0x1d, false, K::KEY_LEFTCTRL),
            (0x1d, true, K::KEY_RIGHTCTRL),
            (0x38, false, K::KEY_LEFTALT),
            (0x38, true, K::KEY_RIGHTALT),
            (0x47, false, K::KEY_KP7),
            (0x47, true, K::KEY_HOME),
            (0x1c, false, K::KEY_ENTER),
            (0x1c, true, K::KEY_KPENTER),
        ] {
            assert_eq!(key_from_scan(scan, extended, 0), Some(expected));
            assert_eq!(scan_code_for_key(expected), Some((scan as u16, extended)));
        }
    }

    #[test]
    fn virtual_key_fallback_maps_function_keys_correctly() {
        for (vk, expected) in [
            (0x70, K::KEY_F1),
            (0x79, K::KEY_F10),
            (0x7a, K::KEY_F11),
            (0x7b, K::KEY_F12),
            (0x7c, K::KEY_F13),
            (0x87, K::KEY_F24),
        ] {
            assert_eq!(key_from_virtual_key(vk), Some(expected));
        }
        assert_eq!(virtual_key_for_key(K::KEY_SYSRQ), Some(VK_SNAPSHOT));
        assert_eq!(virtual_key_for_key(K::KEY_PAUSE), Some(VK_PAUSE));
        assert_eq!(virtual_key_for_key(K::KEY_VOLUMEUP), Some(VK_VOLUME_UP));
        assert_eq!(scan_code_for_key(K::KEY_SYSRQ), None);
        assert_eq!(scan_code_for_key(K::KEY_PAUSE), None);
        assert_eq!(scan_code_for_key(K::KEY_VOLUMEUP), None);
    }

    #[test]
    fn injected_pointer_motion_resets_the_physical_odometer_origin() {
        let mut previous = None;
        assert_eq!(physical_pointer_delta(&mut previous, (10, 20), false), None);
        assert_eq!(
            physical_pointer_delta(&mut previous, (100, 200), true),
            None
        );
        assert_eq!(
            physical_pointer_delta(&mut previous, (103, 205), false),
            Some((3.0, 5.0))
        );
    }

    #[test]
    fn unsupported_special_key_names_fail_validation_explicitly() {
        let config = Config::parse("[keys]\nleft = 'brightnessup'").unwrap();
        assert!(validate_key_mappings(&config)
            .unwrap_err()
            .to_string()
            .contains("KEY_BRIGHTNESSUP"));
    }
}
