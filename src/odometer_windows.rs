use crate::input::{EventType, InputEvent, RelativeAxisCode as R};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Write},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_ABANDONED, WAIT_FAILED,
        WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    System::Threading::{
        CreateEventW, CreateMutexW, ReleaseMutex, ResetEvent, SetEvent, WaitForSingleObject,
    },
};

const SAVE_INTERVAL: Duration = Duration::from_secs(1);
const MUTEX_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Totals {
    fievel_input_units: f64,
    physical_input_units: f64,
}

fn state_path() -> io::Result<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::env::var_os("USERPROFILE")
                .filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join("AppData/Local").into_os_string())
        })
        .ok_or_else(|| io::Error::other("LOCALAPPDATA and USERPROFILE are unset"))?;
    Ok(PathBuf::from(base).join("Fievel/odometer.toml"))
}

fn load(path: &Path) -> io::Result<Totals> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Totals::default()),
        Err(error) => return Err(error),
    };
    let totals: Totals =
        toml::from_str(&text).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if [totals.fievel_input_units, totals.physical_input_units]
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid odometer totals",
        ));
    }
    Ok(totals)
}

fn save(path: &Path, totals: &Totals) -> io::Result<()> {
    let text = toml::to_string(totals).map_err(io::Error::other)?;
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| io::Error::other("missing state directory"))?,
    )?;
    let temporary = path.with_extension("tmp");
    let mut file = File::create(&temporary)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(temporary, path)
}

fn miles(input_units: f64, units_per_inch: f64) -> f64 {
    input_units / units_per_inch / 63_360.0
}

pub fn miles_totals(units_per_inch: f64) -> io::Result<(f64, f64)> {
    let totals = load(&state_path()?)?;
    Ok((
        miles(totals.fievel_input_units, units_per_inch),
        miles(totals.physical_input_units, units_per_inch),
    ))
}

pub fn report(units_per_inch: f64) -> io::Result<()> {
    let path = state_path()?;
    let totals = load(&path)?;
    println!(
        "Fievel:                  {:.6} miles (estimated)",
        miles(totals.fievel_input_units, units_per_inch)
    );
    println!(
        "Physical mouse/trackpad:  {:.6} miles (estimated)",
        miles(totals.physical_input_units, units_per_inch)
    );
    println!("\nTotals since reset while Fievel is running; saved every second.");
    println!(
        "Reference scale: {units_per_inch} Windows screen pixels/inch; not measured physical distance."
    );
    println!("State: {}", path.display());
    Ok(())
}

fn object_name(path: &Path, purpose: &str) -> Vec<u16> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hasher);
    std::ffi::OsStr::new(&format!(
        "Local\\FievelOdometer{purpose}-{:016x}",
        hasher.finish()
    ))
    .encode_wide()
    .chain(Some(0))
    .collect()
}

fn create_mutex(name: &[u16], initial_owner: bool) -> io::Result<HANDLE> {
    let handle = unsafe { CreateMutexW(ptr::null(), initial_owner as i32, name.as_ptr()) };
    if handle.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

fn create_event(name: &[u16]) -> io::Result<HANDLE> {
    let handle = unsafe { CreateEventW(ptr::null(), 0, 0, name.as_ptr()) };
    if handle.is_null() {
        Err(io::Error::last_os_error())
    } else {
        Ok(handle)
    }
}

struct NamedMutex {
    handle: HANDLE,
    owns_mutex: bool,
}

impl NamedMutex {
    fn create(name: &[u16], initial_owner: bool) -> io::Result<(Self, bool)> {
        let handle = create_mutex(name, initial_owner)?;
        let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        Ok((
            Self {
                handle,
                owns_mutex: initial_owner && !exists,
            },
            exists,
        ))
    }

    fn acquire(&mut self, timeout: u32, operation: &str) -> io::Result<bool> {
        match unsafe { WaitForSingleObject(self.handle, timeout) } {
            WAIT_OBJECT_0 | WAIT_ABANDONED => {
                self.owns_mutex = true;
                Ok(true)
            }
            WAIT_TIMEOUT => Ok(false),
            WAIT_FAILED => Err(io::Error::other(format!(
                "{operation}: {}",
                io::Error::last_os_error()
            ))),
            result => Err(io::Error::other(format!(
                "{operation}: unexpected wait result {result}"
            ))),
        }
    }
}

impl Drop for NamedMutex {
    fn drop(&mut self) {
        unsafe {
            if self.owns_mutex {
                ReleaseMutex(self.handle);
            }
            CloseHandle(self.handle);
        }
    }
}

struct EventHandle(HANDLE);

unsafe impl Send for EventHandle {}

impl EventHandle {
    fn create(name: &[u16]) -> io::Result<Self> {
        create_event(name).map(Self)
    }
}

impl Drop for EventHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn check_wait(result: u32, operation: &str) -> io::Result<bool> {
    match result {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        WAIT_FAILED => Err(io::Error::other(format!(
            "{operation}: {}",
            io::Error::last_os_error()
        ))),
        _ => Err(io::Error::other(format!(
            "{operation}: unexpected wait result {result}"
        ))),
    }
}

pub fn reset() -> io::Result<()> {
    let path = state_path()?;
    let (mut reset_mutex, _) = NamedMutex::create(&object_name(&path, "ResetLock"), false)?;
    if !reset_mutex.acquire(MUTEX_TIMEOUT.as_millis() as u32, "locking odometer reset")? {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "another odometer reset did not finish in time",
        ));
    }
    let (mut mutex, _) = NamedMutex::create(&object_name(&path, "Lock"), false)?;
    let already_running = !mutex.acquire(0, "checking for a running Fievel process")?;
    let result = if already_running {
        let request = EventHandle::create(&object_name(&path, "ResetRequest"))?;
        let acknowledge = EventHandle::create(&object_name(&path, "ResetAcknowledge"))?;
        if unsafe { ResetEvent(acknowledge.0) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if unsafe { SetEvent(request.0) } == 0 {
            Err(io::Error::last_os_error())
        } else if !check_wait(
            unsafe { WaitForSingleObject(acknowledge.0, MUTEX_TIMEOUT.as_millis() as u32) },
            "waiting for the running Fievel process to reset its odometer",
        )? {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "running Fievel did not acknowledge the odometer reset",
            ))
        } else {
            Ok(())
        }
    } else {
        save(&path, &Totals::default())
    };
    if result.is_ok() {
        println!("Odometer reset: Fievel and physical mouse/trackpad totals are now zero.");
    }
    result
}

#[derive(Clone, Default)]
pub struct Counter(Arc<AtomicU64>);

impl Counter {
    pub fn record(&self, events: &[InputEvent]) {
        let mut delta = [0.0_f64; 2];
        for event in events {
            if event.event_type() == EventType::RELATIVE {
                match R(event.code()) {
                    R::REL_X => delta[0] += f64::from(event.value()),
                    R::REL_Y => delta[1] += f64::from(event.value()),
                    _ => {}
                }
            }
        }
        self.record_distance(delta[0].hypot(delta[1]));
    }

    pub fn record_pointer_motion(&self, dx: f64, dy: f64) {
        self.record_distance(dx.hypot(dy));
    }

    #[allow(deprecated)]
    fn record_distance(&self, distance: f64) {
        if distance != 0.0 {
            self.0
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |bits| {
                    Some((f64::from_bits(bits) + distance).to_bits())
                })
                .expect("counter update always succeeds");
        }
    }

    fn total(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::Relaxed))
    }
}

pub struct Odometer {
    pub counter: Counter,
    pub physical_counter: Counter,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<io::Result<()>>>,
    _lock: NamedMutex,
}

impl Odometer {
    pub fn start(application_stop: Arc<AtomicBool>) -> io::Result<Self> {
        let path = state_path()?;
        let (lock, already_running) = NamedMutex::create(&object_name(&path, "Lock"), true)?;
        if already_running {
            return Err(io::Error::other(
                "Cannot lock odometer: Fievel is already running. Use --odometer to read totals.",
            ));
        }
        let request = EventHandle::create(&object_name(&path, "ResetRequest"))?;
        let acknowledge = EventHandle::create(&object_name(&path, "ResetAcknowledge"))?;
        let totals = load(&path)?;
        save(&path, &totals)?;
        let counter = Counter::default();
        let physical_counter = Counter::default();
        let worker_counter = counter.clone();
        let worker_physical_counter = physical_counter.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let request_handle = request.0 as usize;
        let acknowledge_handle = acknowledge.0 as usize;
        let worker = match thread::Builder::new()
            .name("fievel-odometer".into())
            .spawn(move || {
                let result = monitor(
                    &path,
                    totals,
                    &worker_counter,
                    &worker_physical_counter,
                    &worker_stop,
                    request_handle as HANDLE,
                    acknowledge_handle as HANDLE,
                );
                drop(request);
                drop(acknowledge);
                if result.is_err() {
                    application_stop.store(true, Ordering::Relaxed);
                }
                result
            }) {
            Ok(worker) => worker,
            Err(error) => return Err(error),
        };
        Ok(Self {
            counter,
            physical_counter,
            stop,
            worker: Some(worker),
            _lock: lock,
        })
    }

    pub fn finish(&mut self) -> io::Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| io::Error::other("odometer worker panicked"))??;
        }
        Ok(())
    }
}

fn monitor(
    path: &Path,
    mut totals: Totals,
    counter: &Counter,
    physical_counter: &Counter,
    stop: &AtomicBool,
    request: HANDLE,
    acknowledge: HANDLE,
) -> io::Result<()> {
    let mut fievel_base = totals.fievel_input_units;
    let mut physical_base = totals.physical_input_units;
    let mut fievel_origin = counter.total();
    let mut physical_origin = physical_counter.total();
    let mut last_save = Instant::now();
    loop {
        let reset = check_wait(
            unsafe { WaitForSingleObject(request, 20) },
            "waiting for an odometer reset request",
        )?;
        if reset {
            totals = Totals::default();
            fievel_base = 0.0;
            physical_base = 0.0;
            fievel_origin = counter.total();
            physical_origin = physical_counter.total();
            save(path, &totals)?;
            if unsafe { SetEvent(acknowledge) } == 0 {
                return Err(io::Error::last_os_error());
            }
            last_save = Instant::now();
        }
        if last_save.elapsed() >= SAVE_INTERVAL || stop.load(Ordering::Acquire) {
            totals.fievel_input_units = fievel_base + (counter.total() - fievel_origin).max(0.0);
            totals.physical_input_units =
                physical_base + (physical_counter.total() - physical_origin).max(0.0);
            save(path, &totals)?;
            last_save = Instant::now();
        }
        if stop.load(Ordering::Acquire) {
            return Ok(());
        }
    }
}

impl Drop for Odometer {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            eprintln!("Cannot save odometer: {error}");
        }
    }
}
