use evdev::{
    raw_stream::RawDevice, AbsoluteAxisCode as A, EventType, InputEvent, KeyCode as K, PropType,
    RelativeAxisCode as R, SynchronizationCode as S,
};
use rustix::fs::{flock, FlockOperation, OFlags};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const SAVE_INTERVAL: Duration = Duration::from_secs(1);
const SCAN_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Totals {
    fievel_input_units: f64,
    physical_input_units: f64,
}

fn state_path() -> io::Result<PathBuf> {
    if let Some(base) = std::env::var_os("XDG_STATE_HOME").filter(|s| !s.is_empty()) {
        let base = PathBuf::from(base);
        if base.is_absolute() {
            return Ok(base.join("fievel/odometer.toml"));
        }
    }
    let home = std::env::var_os("HOME")
        .filter(|s| !s.is_empty())
        .ok_or_else(|| io::Error::other("HOME is unset; set HOME or an absolute XDG_STATE_HOME"))?;
    Ok(PathBuf::from(home).join(".local/state/fievel/odometer.toml"))
}

fn load(path: &Path) -> io::Result<Totals> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Totals::default()),
        Err(error) => return Err(error),
    };
    let totals: Totals =
        toml::from_str(&text).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if [totals.fievel_input_units, totals.physical_input_units]
        .iter()
        .any(|n| !n.is_finite() || *n < 0.0)
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
    let temporary = path.with_extension("tmp");
    // The writer lock protects this temporary file; rename gives readers a whole snapshot.
    let mut file = File::create(&temporary)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    File::open(
        path.parent()
            .ok_or_else(|| io::Error::other("missing state directory"))?,
    )?
    .sync_all()
}

fn miles(input_units: f64, units_per_inch: f64) -> f64 {
    input_units / units_per_inch / 63_360.0
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
    println!("Reference scale: {units_per_inch} input units/inch; not measured physical or screen distance.");
    println!("State: {}", path.display());
    Ok(())
}

fn writer_lock(path: &Path) -> io::Result<File> {
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| io::Error::other("missing state directory"))?,
    )?;
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.with_extension("lock"))
}

pub fn reset() -> io::Result<()> {
    reset_at(&state_path()?)?;
    println!("Odometer reset: Fievel and physical mouse/trackpad totals are now zero.");
    Ok(())
}

fn reset_at(path: &Path) -> io::Result<()> {
    let lock = writer_lock(path)?;
    match flock(&lock, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => save(path, &Totals::default()),
        Err(rustix::io::Errno::WOULDBLOCK) => {
            let mut stream = UnixStream::connect(path.with_extension("sock")).map_err(|error| {
                io::Error::new(error.kind(), format!(
                    "Cannot contact running Fievel: {error}. Restart it with the updated binary before resetting."
                ))
            })?;
            stream.set_read_timeout(Some(Duration::from_secs(5)))?;
            stream.set_write_timeout(Some(Duration::from_secs(5)))?;
            stream.write_all(b"R")?;
            let mut response = [0];
            stream.read_exact(&mut response)?;
            if response != *b"O" {
                return Err(io::Error::other("Running Fievel could not save the reset"));
            }
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

struct ResetListener {
    listener: UnixListener,
    path: PathBuf,
}

impl ResetListener {
    fn bind(state_path: &Path) -> io::Result<Self> {
        let path = state_path.with_extension("sock");
        // Only the lifetime writer-lock owner can replace a stale socket.
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(&path)?;
        let server = Self { listener, path };
        fs::set_permissions(&server.path, fs::Permissions::from_mode(0o600))?;
        server.listener.set_nonblocking(true)?;
        Ok(server)
    }

    fn accept_reset(&self) -> io::Result<Option<UnixStream>> {
        let (mut stream, _) = match self.listener.accept() {
            Ok(connection) => connection,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) =>
            {
                return Ok(None)
            }
            Err(error) => return Err(error),
        };
        stream.set_read_timeout(Some(Duration::from_millis(100)))?;
        stream.set_write_timeout(Some(Duration::from_millis(100)))?;
        let mut request = [0];
        match stream.read_exact(&mut request) {
            Ok(()) if request == *b"R" => Ok(Some(stream)),
            result => {
                eprintln!("Odometer: invalid reset request {request:?}: {result:?}");
                Ok(None)
            }
        }
    }
}

impl Drop for ResetListener {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_file(&self.path) {
            eprintln!(
                "Cannot remove odometer socket {}: {error}",
                self.path.display()
            );
        }
    }
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
        let distance = delta[0].hypot(delta[1]);
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
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl Odometer {
    pub fn start(stop: Arc<AtomicBool>) -> io::Result<Self> {
        Self::start_at(state_path()?, stop, PathBuf::from("/dev/input"))
    }

    fn start_at(
        path: PathBuf,
        application_stop: Arc<AtomicBool>,
        input_directory: PathBuf,
    ) -> io::Result<Self> {
        let lock = writer_lock(&path)?;
        flock(&lock, FlockOperation::NonBlockingLockExclusive).map_err(|e| {
            io::Error::other(format!(
                "Cannot lock odometer: {e}. Is Fievel already running? Use --odometer to read totals."
            ))
        })?;
        let totals = load(&path)?;
        save(&path, &totals)?;
        let reset_listener = ResetListener::bind(&path)?;
        let counter = Counter::default();
        let worker_counter = counter.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("odometer".into())
            .spawn(move || {
                let _lock = lock;
                let reset_listener = reset_listener;
                let result = monitor(
                    &path,
                    totals,
                    &worker_counter,
                    &worker_stop,
                    &input_directory,
                    &reset_listener,
                );
                if result.is_err() {
                    application_stop.store(true, Ordering::Relaxed);
                }
                result
            })?;
        Ok(Self {
            counter,
            stop,
            worker: Some(worker),
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

impl Drop for Odometer {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            eprintln!("Cannot save odometer: {error}");
        }
    }
}

#[derive(Clone, Copy, Default)]
struct Contact {
    position: [Option<i32>; 2],
    previous: Option<[i32; 2]>,
}

impl Contact {
    fn distance(&mut self) -> f64 {
        let [Some(x), Some(y)] = self.position else {
            return 0.0;
        };
        let current = [x, y];
        let distance = self.previous.map_or(0.0, |previous| {
            (f64::from(x) - f64::from(previous[0])).hypot(f64::from(y) - f64::from(previous[1]))
        });
        self.previous = Some(current);
        distance
    }
}

#[derive(Clone, Copy)]
enum Kind {
    Mouse,
    Touchpad,
    Multitouch,
}

struct Motion {
    kind: Kind,
    relative: [f64; 2],
    contact: Contact,
    touching: bool,
    multiple: BTreeSet<K>,
    slot: i32,
    contacts: BTreeMap<i32, Contact>,
    previous_slot: Option<i32>,
    dropped: bool,
}

impl Motion {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            relative: [0.0; 2],
            contact: Contact::default(),
            touching: false,
            multiple: BTreeSet::new(),
            slot: 0,
            contacts: BTreeMap::new(),
            previous_slot: None,
            dropped: false,
        }
    }

    fn event(&mut self, event: InputEvent) -> f64 {
        if event.event_type() == EventType::SYNCHRONIZATION {
            if event.code() == S::SYN_DROPPED.0 {
                *self = Self::new(self.kind);
                self.dropped = true;
                eprintln!("Odometer: input events dropped; resetting motion baseline");
            } else if event.code() == S::SYN_REPORT.0 {
                if self.dropped {
                    self.dropped = false;
                } else {
                    return self.frame();
                }
            }
            return 0.0;
        }
        if self.dropped {
            return 0.0;
        }
        match self.kind {
            Kind::Mouse if event.event_type() == EventType::RELATIVE => match R(event.code()) {
                R::REL_X => self.relative[0] += f64::from(event.value()),
                R::REL_Y => self.relative[1] += f64::from(event.value()),
                _ => {}
            },
            Kind::Touchpad | Kind::Multitouch => self.touch_event(event),
            _ => {}
        }
        0.0
    }

    fn touch_event(&mut self, event: InputEvent) {
        if event.event_type() == EventType::KEY {
            match K(event.code()) {
                K::BTN_TOUCH => {
                    self.touching = event.value() != 0;
                    if !self.touching {
                        self.contact = Contact::default();
                        for contact in self.contacts.values_mut() {
                            contact.previous = None;
                        }
                    }
                }
                key @ (K::BTN_TOOL_DOUBLETAP
                | K::BTN_TOOL_TRIPLETAP
                | K::BTN_TOOL_QUADTAP
                | K::BTN_TOOL_QUINTTAP) => {
                    if event.value() != 0 {
                        self.multiple.insert(key);
                    } else {
                        self.multiple.remove(&key);
                    }
                }
                _ => {}
            }
        } else if event.event_type() == EventType::ABSOLUTE {
            match (self.kind, A(event.code())) {
                (Kind::Touchpad, A::ABS_X) => self.contact.position[0] = Some(event.value()),
                (Kind::Touchpad, A::ABS_Y) => self.contact.position[1] = Some(event.value()),
                (Kind::Multitouch, A::ABS_MT_SLOT) => self.slot = event.value(),
                (Kind::Multitouch, A::ABS_MT_TRACKING_ID) => {
                    if event.value() < 0 {
                        self.contacts.remove(&self.slot);
                    } else {
                        self.contacts.insert(self.slot, Contact::default());
                    }
                }
                (Kind::Multitouch, axis @ (A::ABS_MT_POSITION_X | A::ABS_MT_POSITION_Y)) => {
                    if let Some(contact) = self.contacts.get_mut(&self.slot) {
                        let index = usize::from(axis == A::ABS_MT_POSITION_Y);
                        contact.position[index] = Some(event.value());
                    }
                }
                _ => {}
            }
        }
    }

    fn frame(&mut self) -> f64 {
        match self.kind {
            Kind::Mouse => {
                let distance = self.relative[0].hypot(self.relative[1]);
                self.relative = [0.0; 2];
                distance
            }
            Kind::Touchpad => {
                if self.touching && self.multiple.is_empty() {
                    self.contact.distance()
                } else {
                    self.contact.previous = None;
                    0.0
                }
            }
            Kind::Multitouch => {
                if self.touching && self.contacts.len() == 1 && self.multiple.is_empty() {
                    let (&slot, contact) = self.contacts.iter_mut().next().expect("one contact");
                    if self.previous_slot != Some(slot) {
                        contact.previous = None;
                    }
                    self.previous_slot = Some(slot);
                    contact.distance()
                } else {
                    self.previous_slot = None;
                    for contact in self.contacts.values_mut() {
                        contact.previous = None;
                    }
                    0.0
                }
            }
        }
    }
}

fn pointer_kind(device: &RawDevice) -> Option<Kind> {
    if device
        .supported_relative_axes()
        .is_some_and(|axes| axes.contains(R::REL_X) && axes.contains(R::REL_Y))
    {
        return Some(Kind::Mouse);
    }
    if device.properties().contains(PropType::DIRECT)
        || !device
            .supported_keys()
            .is_some_and(|keys| keys.contains(K::BTN_TOOL_FINGER) && keys.contains(K::BTN_TOUCH))
    {
        return None;
    }
    let axes = device.supported_absolute_axes()?;
    if [
        A::ABS_MT_SLOT,
        A::ABS_MT_TRACKING_ID,
        A::ABS_MT_POSITION_X,
        A::ABS_MT_POSITION_Y,
    ]
    .iter()
    .all(|axis| axes.contains(*axis))
    {
        Some(Kind::Multitouch)
    } else if axes.contains(A::ABS_X) && axes.contains(A::ABS_Y) {
        Some(Kind::Touchpad)
    } else {
        eprintln!(
            "Odometer: unsupported touchpad protocol: {}",
            device.name().unwrap_or("unnamed")
        );
        None
    }
}

struct Pointer {
    device: RawDevice,
    motion: Motion,
}

fn is_virtual_pointer(sys_path: &Path) -> bool {
    // Bluetooth hardware can live under virtual/misc/uhid. Only parentless
    // virtual input devices (uinput/remappers) must be excluded here.
    sys_path.starts_with("/sys/devices/virtual/input")
}

fn discover(
    devices: &mut BTreeMap<PathBuf, Pointer>,
    warned: &mut BTreeSet<PathBuf>,
    input_directory: &Path,
) -> io::Result<()> {
    let mut present = BTreeSet::new();
    for entry in fs::read_dir(input_directory)? {
        let entry = entry?;
        if !entry.file_name().to_string_lossy().starts_with("event") {
            continue;
        }
        let path = entry.path();
        present.insert(path.clone());
        if devices.contains_key(&path) {
            continue;
        }
        let opened = (|| -> io::Result<Option<Pointer>> {
            if is_virtual_pointer(&super::input_sys_path(&path)?) {
                return Ok(None);
            }
            let device = RawDevice::open(&path)?;
            if matches!(
                device.name(),
                Some(super::KEYBOARD_NAME | super::MOUSE_NAME)
            ) {
                return Ok(None);
            }
            let Some(kind) = pointer_kind(&device) else {
                return Ok(None);
            };
            let flags = rustix::fs::fcntl_getfl(&device)?;
            rustix::fs::fcntl_setfl(&device, flags | OFlags::NONBLOCK)?;
            Ok(Some(Pointer {
                device,
                motion: Motion::new(kind),
            }))
        })();
        match opened {
            Ok(Some(pointer)) => {
                eprintln!(
                    "Odometer tracking {} ({})",
                    path.display(),
                    pointer.device.name().unwrap_or("unnamed")
                );
                devices.insert(path.clone(), pointer);
                warned.remove(&path);
            }
            Ok(None) => {}
            Err(error) => {
                if warned.insert(path.clone()) {
                    eprintln!("Odometer cannot inspect {}: {error}", path.display());
                }
            }
        }
    }
    devices.retain(|path, _| present.contains(path));
    warned.retain(|path| present.contains(path));
    Ok(())
}

fn monitor(
    path: &Path,
    mut totals: Totals,
    counter: &Counter,
    stop: &AtomicBool,
    input_directory: &Path,
    reset_listener: &ResetListener,
) -> io::Result<()> {
    let mut initial_fievel = totals.fievel_input_units;
    let mut counter_baseline = 0.0;
    let mut devices = BTreeMap::new();
    let mut warned = BTreeSet::new();
    discover(&mut devices, &mut warned, input_directory)?;
    if devices.is_empty() {
        eprintln!("Odometer: no accessible physical mice/trackpads; check /dev/input permissions");
    }
    let mut scanned = Instant::now();
    let mut saved = Instant::now();
    loop {
        devices.retain(|path, pointer| {
            // Bound each drain so a noisy device cannot starve other devices or shutdown.
            for _ in 0..32 {
                match pointer.device.fetch_events() {
                    Ok(events) => {
                        for event in events {
                            totals.physical_input_units += pointer.motion.event(event);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => break,
                    Err(error) => {
                        eprintln!("Odometer stopped reading {}: {error}", path.display());
                        return false;
                    }
                }
            }
            true
        });
        let stopping = stop.load(Ordering::Acquire);
        let reset_request = reset_listener.accept_reset()?;
        if reset_request.is_some() {
            totals = Totals::default();
            initial_fievel = 0.0;
            counter_baseline = counter.total();
            for pointer in devices.values_mut() {
                pointer.motion.relative = [0.0; 2];
                pointer.motion.contact.previous = None;
                pointer.motion.previous_slot = None;
                for contact in pointer.motion.contacts.values_mut() {
                    contact.previous = None;
                }
            }
        }
        totals.fievel_input_units = initial_fievel + (counter.total() - counter_baseline);
        if stopping || reset_request.is_some() || saved.elapsed() >= SAVE_INTERVAL {
            save(path, &totals)?;
            saved = Instant::now();
        }
        if let Some(mut stream) = reset_request {
            if let Err(error) = stream.write_all(b"O") {
                eprintln!("Odometer reset saved, but could not acknowledge it: {error}");
            }
        }
        if stopping {
            return Ok(());
        }
        if scanned.elapsed() >= SCAN_INTERVAL {
            discover(&mut devices, &mut warned, input_directory)?;
            scanned = Instant::now();
        }
        thread::sleep(Duration::from_millis(8));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: EventType, code: u16, value: i32) -> InputEvent {
        InputEvent::new(kind.0, code, value)
    }

    fn frame(motion: &mut Motion, events: &[(EventType, u16, i32)]) -> f64 {
        for &(kind, code, value) in events {
            assert_eq!(motion.event(event(kind, code, value)), 0.0);
        }
        motion.event(event(EventType::SYNCHRONIZATION, S::SYN_REPORT.0, 0))
    }

    #[test]
    fn relative_distance_counts_diagonals_per_frame_and_ignores_scroll() {
        let events = [
            event(EventType::RELATIVE, R::REL_X.0, -3),
            event(EventType::RELATIVE, R::REL_Y.0, 4),
            event(EventType::RELATIVE, R::REL_WHEEL.0, 12),
            event(EventType::KEY, K::BTN_LEFT.0, 1),
        ];
        let counter = Counter::default();
        counter.record(&events);
        counter.record(&[]);
        assert_eq!(counter.total(), 5.0);
        let mut motion = Motion::new(Kind::Mouse);
        for event in events {
            assert_eq!(motion.event(event), 0.0);
        }
        assert_eq!(frame(&mut motion, &[]), 5.0);
        assert_eq!(frame(&mut motion, &[]), 0.0);
        assert_eq!(
            frame(&mut motion, &[(EventType::RELATIVE, R::REL_X.0, 3)]),
            3.0
        );
    }

    #[test]
    fn touchpad_ignores_lifts_repositioning_and_two_finger_gestures() {
        let mut motion = Motion::new(Kind::Touchpad);
        assert_eq!(
            frame(
                &mut motion,
                &[
                    (EventType::KEY, K::BTN_TOUCH.0, 1),
                    (EventType::ABSOLUTE, A::ABS_X.0, 100),
                    (EventType::ABSOLUTE, A::ABS_Y.0, 100),
                ]
            ),
            0.0
        );
        assert_eq!(
            frame(
                &mut motion,
                &[
                    (EventType::ABSOLUTE, A::ABS_X.0, 103),
                    (EventType::ABSOLUTE, A::ABS_Y.0, 104),
                ]
            ),
            5.0
        );
        assert_eq!(
            frame(
                &mut motion,
                &[
                    (EventType::KEY, K::BTN_TOOL_DOUBLETAP.0, 1),
                    (EventType::ABSOLUTE, A::ABS_X.0, 200),
                ]
            ),
            0.0
        );
        assert_eq!(
            frame(&mut motion, &[(EventType::KEY, K::BTN_TOOL_DOUBLETAP.0, 0)]),
            0.0
        );
        assert_eq!(
            frame(&mut motion, &[(EventType::KEY, K::BTN_TOUCH.0, 0)]),
            0.0
        );
        assert_eq!(
            frame(
                &mut motion,
                &[
                    (EventType::KEY, K::BTN_TOUCH.0, 1),
                    (EventType::ABSOLUTE, A::ABS_X.0, 1000),
                    (EventType::ABSOLUTE, A::ABS_Y.0, 2000),
                ]
            ),
            0.0
        );
    }

    #[test]
    fn multitouch_tracks_one_contact_without_double_counting_legacy_axes() {
        let mut motion = Motion::new(Kind::Multitouch);
        assert_eq!(
            frame(
                &mut motion,
                &[
                    (EventType::KEY, K::BTN_TOUCH.0, 1),
                    (EventType::ABSOLUTE, A::ABS_MT_TRACKING_ID.0, 42),
                    (EventType::ABSOLUTE, A::ABS_MT_POSITION_X.0, 100),
                    (EventType::ABSOLUTE, A::ABS_MT_POSITION_Y.0, 100),
                ]
            ),
            0.0
        );
        assert_eq!(
            frame(
                &mut motion,
                &[
                    (EventType::ABSOLUTE, A::ABS_MT_POSITION_X.0, 103),
                    (EventType::ABSOLUTE, A::ABS_MT_POSITION_Y.0, 104),
                    (EventType::ABSOLUTE, A::ABS_X.0, 103),
                    (EventType::ABSOLUTE, A::ABS_Y.0, 104),
                ]
            ),
            5.0
        );
        assert_eq!(
            frame(
                &mut motion,
                &[
                    (EventType::ABSOLUTE, A::ABS_MT_SLOT.0, 1),
                    (EventType::ABSOLUTE, A::ABS_MT_TRACKING_ID.0, 43),
                    (EventType::ABSOLUTE, A::ABS_MT_POSITION_X.0, 500),
                    (EventType::ABSOLUTE, A::ABS_MT_POSITION_Y.0, 500),
                ]
            ),
            0.0
        );
        assert_eq!(
            frame(
                &mut motion,
                &[
                    (EventType::ABSOLUTE, A::ABS_MT_SLOT.0, 0),
                    (EventType::ABSOLUTE, A::ABS_MT_TRACKING_ID.0, -1),
                ]
            ),
            0.0
        );
        assert_eq!(
            frame(
                &mut motion,
                &[
                    (EventType::ABSOLUTE, A::ABS_MT_SLOT.0, 1),
                    (EventType::ABSOLUTE, A::ABS_MT_POSITION_X.0, 503),
                    (EventType::ABSOLUTE, A::ABS_MT_POSITION_Y.0, 504),
                ]
            ),
            5.0
        );
        assert_eq!(
            frame(
                &mut motion,
                &[
                    (EventType::ABSOLUTE, A::ABS_MT_TRACKING_ID.0, 44),
                    (EventType::ABSOLUTE, A::ABS_MT_POSITION_X.0, 900),
                    (EventType::ABSOLUTE, A::ABS_MT_POSITION_Y.0, 900),
                ]
            ),
            0.0
        );
    }

    #[test]
    fn dropped_events_discard_incomplete_frames() {
        let mut motion = Motion::new(Kind::Mouse);
        assert_eq!(
            frame(
                &mut motion,
                &[
                    (EventType::RELATIVE, R::REL_X.0, 100),
                    (EventType::SYNCHRONIZATION, S::SYN_DROPPED.0, 0),
                    (EventType::RELATIVE, R::REL_Y.0, 100),
                ]
            ),
            0.0
        );
        assert_eq!(
            frame(&mut motion, &[(EventType::RELATIVE, R::REL_X.0, 3)]),
            3.0
        );
    }

    #[test]
    fn snapshots_persist_and_readers_do_not_need_writer_lock() {
        let directory =
            std::env::temp_dir().join(format!("fievel-odometer-test-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("odometer.toml");
        let lock = File::create(path.with_extension("lock")).unwrap();
        flock(&lock, FlockOperation::NonBlockingLockExclusive).unwrap();
        assert_eq!(load(&path).unwrap(), Totals::default());
        let totals = Totals {
            fievel_input_units: 123.5,
            physical_input_units: 678.0,
        };
        save(&path, &totals).unwrap();
        assert_eq!(load(&path).unwrap(), totals);
        assert!(Odometer::start_at(
            path.clone(),
            Arc::new(AtomicBool::new(false)),
            directory.clone(),
        )
        .is_err());
        assert_eq!(load(&path).unwrap(), totals);
        fs::write(&path, "fievel_input_units = nan\nphysical_input_units = 0").unwrap();
        assert_eq!(load(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);
        fs::write(&path, "broken").unwrap();
        assert_eq!(load(&path).unwrap_err().kind(), io::ErrorKind::InvalidData);
        fs::remove_file(&path).unwrap();
        fs::remove_file(path.with_extension("lock")).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn excludes_remappers_but_includes_bluetooth_uhid_hardware() {
        assert!(is_virtual_pointer(Path::new(
            "/sys/devices/virtual/input/input42/event42"
        )));
        assert!(!is_virtual_pointer(Path::new(
            "/sys/devices/pci0000:00/usb1/input/input1/event1"
        )));
        assert!(!is_virtual_pointer(Path::new(
            "/sys/devices/virtual/misc/uhid/0005:1234/input/input1/event1"
        )));
    }

    #[test]
    fn worker_publishes_live_totals_flushes_final_movement_and_resumes() {
        let directory =
            std::env::temp_dir().join(format!("fievel-odometer-worker-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let path = directory.join("odometer.toml");
        let original = Totals {
            fievel_input_units: 10.0,
            physical_input_units: 20.0,
        };
        save(&path, &original).unwrap();
        let application_stop = Arc::new(AtomicBool::new(false));
        let mut odometer = Odometer::start_at(
            path.clone(),
            Arc::clone(&application_stop),
            directory.clone(),
        )
        .unwrap();
        let movement = [event(EventType::RELATIVE, R::REL_X.0, 5)];
        odometer.counter.record(&movement);
        let deadline = Instant::now() + Duration::from_secs(5);
        while load(&path).unwrap().fievel_input_units != 15.0 {
            assert!(
                Instant::now() < deadline,
                "worker must publish without being stopped"
            );
            thread::sleep(Duration::from_millis(10));
        }
        reset_at(&path).unwrap();
        assert_eq!(load(&path).unwrap(), Totals::default());
        assert!(!application_stop.load(Ordering::Relaxed));
        odometer.counter.record(&movement);
        reset_at(&path).unwrap();
        assert_eq!(load(&path).unwrap(), Totals::default());
        application_stop.store(true, Ordering::Relaxed);
        odometer.counter.record(&movement);
        odometer.finish().unwrap();
        assert_eq!(
            load(&path).unwrap(),
            Totals {
                fievel_input_units: 5.0,
                physical_input_units: 0.0
            }
        );
        let mut restarted = Odometer::start_at(
            path.clone(),
            Arc::new(AtomicBool::new(false)),
            directory.clone(),
        )
        .unwrap();
        restarted.counter.record(&movement);
        restarted.finish().unwrap();
        assert_eq!(
            load(&path).unwrap(),
            Totals {
                fievel_input_units: 10.0,
                physical_input_units: 0.0
            }
        );
        fs::remove_file(&path).unwrap();
        fs::remove_file(path.with_extension("lock")).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn converts_reference_input_distance_to_miles() {
        assert_eq!(miles(6_082_560.0, 96.0), 1.0);
        assert_eq!(miles(50_688_000.0, 800.0), 1.0);
        assert_eq!(miles(0.0, 96.0), 0.0);
        assert_eq!(miles(3_041_280.0, 96.0), 0.5);
    }
}
