use rustix::{
    event::{poll, PollFd, PollFlags, Timespec},
    fs::{memfd_create, MemfdFlags},
};
use std::{
    error::Error,
    fs::File,
    io::{self, Read, Write},
    os::{fd::AsFd, unix::net::UnixStream},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use wayland_client::{
    delegate_noop,
    protocol::{
        wl_buffer, wl_callback, wl_compositor, wl_region, wl_registry, wl_shm, wl_shm_pool,
        wl_surface,
    },
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, Layer},
    zwlr_layer_surface_v1::{self, Anchor, KeyboardInteractivity},
};
use crate::engine::SpeedMode;

const WIDTH: i32 = 88;
const HEIGHT: i32 = 30;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(3);

#[path = "indicator_render.rs"]
mod render;

#[derive(Clone, Default)]
struct KeycastFrame {
    keys: Vec<evdev::KeyCode>,
    speed: SpeedMode,
}

#[derive(Default)]
struct State {
    active: AtomicBool,
    stop: AtomicBool,
    mapped: AtomicBool,
    keycast: bool,
    frame: Mutex<KeycastFrame>,
    revision: AtomicU64,
}

struct Worker {
    state: Arc<State>,
    wake: UnixStream,
    thread: Option<JoinHandle<()>>,
}

/// No display connection, socket, or thread is created when disabled.
pub struct Indicator(Option<Worker>);

/// The mode label and key history have independent display workers.
pub struct Overlays {
    indicator: Indicator,
    keycast: Indicator,
}

impl Overlays {
    pub fn new(notify: bool, keycast_enabled: bool) -> Self {
        Self {
            indicator: Indicator::new(notify),
            keycast: if keycast_enabled {
                Indicator::start_kind(true).unwrap_or_else(|error| {
                    warn_keycast(&error);
                    Indicator(None)
                })
            } else {
                Indicator(None)
            },
        }
    }

    pub fn update(&mut self, active: bool, keys: &[evdev::KeyCode], speed: SpeedMode) {
        self.indicator.set_active(active);
        self.keycast.set_keys(if active { keys } else { &[] }, speed);
    }

    pub fn clear(&mut self) {
        self.update(false, &[], SpeedMode::Normal);
    }
}

impl Indicator {
    pub fn new(enabled: bool) -> Self {
        if !enabled {
            return Self(None);
        }
        match Self::start() {
            Ok(indicator) => indicator,
            Err(error) => {
                warn(&error);
                Self(None)
            }
        }
    }

    fn start() -> io::Result<Self> {
        Self::start_kind(false)
    }

    fn start_kind(keycast: bool) -> io::Result<Self> {
        let (wake, receiver) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        receiver.set_nonblocking(true)?;
        let state = Arc::new(State {
            keycast,
            ..State::default()
        });
        let shared = Arc::clone(&state);
        let thread = thread::Builder::new()
            .name(
                if keycast {
                    "fievel-keycast"
                } else {
                    "fievel-indicator"
                }
                .into(),
            )
            .spawn(move || {
                let result = display(Arc::clone(&shared), receiver);
                shared.mapped.store(false, Ordering::SeqCst);
                if let Err(error) = result {
                    if keycast {
                        warn_keycast(error.as_ref());
                    } else {
                        warn(error.as_ref());
                    }
                }
            })?;
        Ok(Self(Some(Worker {
            state,
            wake,
            thread: Some(thread),
        })))
    }

    pub fn set_active(&mut self, active: bool) {
        if let Some(worker) = &mut self.0 {
            if worker.state.active.swap(active, Ordering::SeqCst) != active {
                worker.wake();
            }
        }
    }

    fn set_keys(&mut self, keys: &[evdev::KeyCode], speed: SpeedMode) {
        if let Some(worker) = &mut self.0 {
            let keys = &keys[keys.len().saturating_sub(32)..];
            let speed = if keys.is_empty() { SpeedMode::Normal } else { speed };
            let mut current = worker.state.frame.lock().unwrap();
            if current.keys.as_slice() == keys && current.speed == speed {
                return;
            }
            current.keys.clear();
            current.keys.extend_from_slice(keys);
            current.speed = speed;
            worker
                .state
                .active
                .store(!keys.is_empty(), Ordering::SeqCst);
            worker.state.revision.fetch_add(1, Ordering::SeqCst);
            drop(current);
            worker.wake();
        }
    }
}

impl Worker {
    fn wake(&mut self) {
        // A full socket already has a wakeup pending; the atomic is authoritative.
        match self.wake.write(&[1]) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                if !self
                    .thread
                    .as_ref()
                    .is_some_and(|thread| thread.is_finished())
                {
                    eprintln!("fievel: indicator wakeup failed: {error}");
                }
            }
        }
    }
}

impl Drop for Indicator {
    fn drop(&mut self) {
        if let Some(worker) = &mut self.0 {
            worker.state.active.store(false, Ordering::SeqCst);
            worker.state.stop.store(true, Ordering::SeqCst);
            worker.wake();
            if let Some(thread) = worker.thread.take() {
                if thread.join().is_err() {
                    eprintln!("fievel: indicator worker panicked; keyboard cleanup is unaffected");
                }
            }
        }
    }
}

fn warn(error: &dyn Error) {
    eprintln!(
        "fievel: Free Mouse Mode indicator unavailable: {error}. \
         Continuing without the indicator; keyboard control is unaffected. \
         Requires a Wayland session with wlr-layer-shell (e.g. Hyprland/Sway); \
         set notify = false to disable it."
    );
}

fn warn_keycast(error: &dyn Error) {
    eprintln!(
        "fievel: keycast overlay unavailable: {error}. \
         Continuing without keycast; keyboard control is unaffected. \
         Requires a Wayland session with wlr-layer-shell (e.g. Hyprland/Sway)."
    );
}

struct Surface {
    surface: wl_surface::WlSurface,
    layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    configured: bool,
    created: Instant,
    requested_size: (u32, u32),
    size: (u32, u32),
}

#[derive(Default)]
struct Display {
    shared: Arc<State>,
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    buffer: Option<wl_buffer::WlBuffer>,
    surface: Option<Surface>,
    globals_ready: bool,
    closed: bool,
    initialized: bool,
    revision: u64,
    layout: Option<render::Layout>,
    speed: SpeedMode,
    dirty: bool,
    buffer_busy: bool,
    retired: Vec<wl_buffer::WlBuffer>,
    error: Option<&'static str>,
}

impl Display {
    fn initialize(&mut self, qh: &QueueHandle<Self>) -> Result<(), Box<dyn Error>> {
        self.compositor.as_ref().ok_or("missing wl_compositor")?;
        self.shell
            .as_ref()
            .ok_or("compositor does not support zwlr_layer_shell_v1")?;
        let shm = self.shm.as_ref().ok_or("missing wl_shm")?;
        self.initialized = true;
        if self.shared.keycast {
            return Ok(());
        }
        let mut file = File::from(memfd_create("fievel-indicator", MemfdFlags::CLOEXEC)?);
        let pixels = pixels();
        file.write_all(&pixels)?;
        let pool = shm.create_pool(file.as_fd(), pixels.len() as i32, qh, ());
        self.buffer = Some(pool.create_buffer(
            0,
            WIDTH,
            HEIGHT,
            WIDTH * 4,
            wl_shm::Format::Argb8888,
            qh,
            (),
        ));
        pool.destroy();
        Ok(())
    }

    fn hide(&mut self) {
        if let Some(surface) = self.surface.take() {
            surface.layer.destroy();
            surface.surface.destroy();
        }
        self.shared.mapped.store(false, Ordering::SeqCst);
        if self.shared.keycast {
            self.retire_buffer();
        }
    }

    fn retire_buffer(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            if self.buffer_busy {
                self.retired.push(buffer);
            } else {
                buffer.destroy();
            }
        }
        self.buffer_busy = false;
    }

    fn update(&mut self, qh: &QueueHandle<Self>) -> Result<(), Box<dyn Error>> {
        if self.shared.keycast && self.shared.revision.load(Ordering::SeqCst) != self.revision {
            let shared = self.shared.frame.lock().unwrap();
            let frame = shared.clone();
            self.revision = self.shared.revision.load(Ordering::SeqCst);
            drop(shared);
            self.layout = if frame.keys.is_empty() {
                None
            } else {
                Some(render::Layout::new(&frame.keys))
            };
            self.speed = frame.speed;
            self.dirty = true;
        }
        if !self.shared.active.load(Ordering::SeqCst) {
            self.hide();
            return Ok(());
        }
        let size = if self.shared.keycast {
            let Some(layout) = &self.layout else {
                return Ok(());
            };
            layout.size()
        } else {
            (WIDTH as u32, HEIGHT as u32)
        };
        if self.surface.is_none() {
            let compositor = self.compositor.as_ref().unwrap();
            let surface = compositor.create_surface(qh, ());
            let region = compositor.create_region(qh, ());
            surface.set_input_region(Some(&region));
            region.destroy();
            let layer = self.shell.as_ref().unwrap().get_layer_surface(
                &surface,
                None,
                Layer::Overlay,
                if self.shared.keycast {
                    "fievel-keycast"
                } else {
                    "fievel-indicator"
                }
                .into(),
                qh,
                (),
            );
            layer.set_size(size.0, size.1);
            layer.set_anchor(
                Anchor::Bottom
                    | if self.shared.keycast {
                        Anchor::Right
                    } else {
                        Anchor::Left
                    },
            );
            layer.set_margin(
                0,
                if self.shared.keycast { 12 } else { 0 },
                12,
                if self.shared.keycast { 0 } else { 12 },
            );
            layer.set_exclusive_zone(-1);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            surface.commit();
            self.surface = Some(Surface {
                surface,
                layer,
                configured: false,
                created: Instant::now(),
                requested_size: size,
                size,
            });
            self.dirty = true;
        }
        if self.shared.keycast {
            let surface = self.surface.as_mut().unwrap();
            if !surface.configured {
                return Ok(());
            }
            if surface.requested_size != size {
                surface.layer.set_size(size.0, size.1);
                surface.surface.commit();
                surface.requested_size = size;
                surface.configured = false;
                surface.created = Instant::now();
                return Ok(());
            }
            // Bound outstanding buffers even when a compositor delays releases.
            // A release event retries the latest revision, not intermediate frames.
            if self.dirty && self.retired.len() < 2 {
                let (width, height) = surface.size;
                let pixels = self.layout.as_ref().unwrap().pixels(width, height, self.speed);
                let mut file = File::from(memfd_create("fievel-keycast", MemfdFlags::CLOEXEC)?);
                file.write_all(&pixels)?;
                let pool = self.shm.as_ref().unwrap().create_pool(
                    file.as_fd(),
                    pixels.len() as i32,
                    qh,
                    (),
                );
                let buffer = pool.create_buffer(
                    0,
                    width as i32,
                    height as i32,
                    width as i32 * 4,
                    wl_shm::Format::Argb8888,
                    qh,
                    (),
                );
                pool.destroy();
                self.retire_buffer();
                let surface = self.surface.as_ref().unwrap();
                surface.surface.attach(Some(&buffer), 0, 0);
                surface.surface.damage(0, 0, width as i32, height as i32);
                surface.surface.commit();
                self.buffer = Some(buffer);
                self.buffer_busy = true;
                self.dirty = false;
                self.shared.mapped.store(true, Ordering::SeqCst);
            }
        }
        Ok(())
    }
}

fn display(shared: Arc<State>, mut wake: UnixStream) -> Result<(), Box<dyn Error>> {
    let connection = Connection::connect_to_env()?;
    display_connection(connection, shared, &mut wake)
}

fn display_connection(
    connection: Connection,
    shared: Arc<State>,
    wake: &mut UnixStream,
) -> Result<(), Box<dyn Error>> {
    let mut queue = connection.new_event_queue::<Display>();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());
    connection.display().sync(&qh, ());
    let mut display = Display {
        shared,
        ..Display::default()
    };
    let started = Instant::now();
    loop {
        if display.shared.stop.load(Ordering::SeqCst) {
            display.hide();
            // Dropping the connection also removes surfaces if the server is stalled.
            match connection.flush() {
                Ok(()) => {}
                Err(wayland_client::backend::WaylandError::Io(error))
                    if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
            return Ok(());
        }
        queue.dispatch_pending(&mut display)?;
        if display.closed {
            return Err("compositor closed the indicator layer surface".into());
        }
        if let Some(error) = display.error {
            return Err(error.into());
        }
        if display.globals_ready {
            if !display.initialized {
                display.initialize(&qh)?;
            }
            display.update(&qh)?;
            if display
                .surface
                .as_ref()
                .is_some_and(|s| !s.configured && s.created.elapsed() > STARTUP_TIMEOUT)
            {
                return Err("timed out waiting for layer-surface configuration".into());
            }
        } else if started.elapsed() > STARTUP_TIMEOUT {
            return Err("timed out waiting for Wayland globals".into());
        }
        let writable = match connection.flush() {
            Ok(()) => false,
            Err(wayland_client::backend::WaylandError::Io(error))
                if error.kind() == io::ErrorKind::WouldBlock =>
            {
                true
            }
            Err(error) => return Err(error.into()),
        };
        let Some(read) = queue.prepare_read() else {
            continue;
        };
        let mut fds = [
            PollFd::new(
                &connection,
                PollFlags::IN
                    | if writable {
                        PollFlags::OUT
                    } else {
                        PollFlags::empty()
                    },
            ),
            PollFd::new(&*wake, PollFlags::IN),
        ];
        // A bounded wait also checks setup deadlines and guarantees prompt shutdown.
        match poll(
            &mut fds,
            Some(&Timespec {
                tv_sec: 0,
                tv_nsec: 100_000_000,
            }),
        ) {
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        }
        let socket_ready = fds[0]
            .revents()
            .intersects(PollFlags::IN | PollFlags::ERR | PollFlags::HUP);
        let wake_ready = fds[1].revents().contains(PollFlags::IN);
        if socket_ready {
            match read.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(error))
                    if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.into()),
            }
        } else {
            drop(read);
        }
        if wake_ready {
            let mut bytes = [0; 64];
            loop {
                match wake.read(&mut bytes) {
                    Ok(0) => return Ok(()),
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => return Err(error.into()),
                }
            }
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for Display {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind(name, version.min(4), qh, ()))
                }
                "wl_shm" => state.shm = Some(registry.bind(name, 1, qh, ())),
                "zwlr_layer_shell_v1" => state.shell = Some(registry.bind(name, 1, qh, ())),
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for Display {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.globals_ready = true;
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for Display {
    fn event(
        state: &mut Self,
        layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(surface) = &mut state.surface else {
            return;
        };
        if &surface.layer != layer {
            return;
        }
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer.ack_configure(serial);
                if state.shared.keycast {
                    let size = (
                        if width == 0 {
                            surface.requested_size.0
                        } else {
                            width
                        },
                        if height == 0 {
                            surface.requested_size.1
                        } else {
                            height
                        },
                    );
                    if size.0 > 4096 || size.1 > 4096 {
                        state.error = Some("keycast surface exceeds supported dimensions");
                        return;
                    }
                    surface.size = size;
                    surface.configured = true;
                    state.dirty = true;
                    return;
                }
                if state.shared.active.load(Ordering::SeqCst)
                    && !state.shared.stop.load(Ordering::SeqCst)
                {
                    surface.surface.attach(state.buffer.as_ref(), 0, 0);
                    surface.surface.damage(0, 0, WIDTH, HEIGHT);
                    surface.surface.commit();
                    surface.configured = true;
                    state.shared.mapped.store(true, Ordering::SeqCst);
                }
            }
            zwlr_layer_surface_v1::Event::Closed => state.closed = true,
            _ => {}
        }
    }
}

delegate_noop!(Display: ignore wl_compositor::WlCompositor);
delegate_noop!(Display: ignore wl_shm::WlShm);
delegate_noop!(Display: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Display: ignore wl_surface::WlSurface);
delegate_noop!(Display: ignore wl_region::WlRegion);
delegate_noop!(Display: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

impl Dispatch<wl_buffer::WlBuffer, ()> for Display {
    fn event(
        state: &mut Self,
        buffer: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_buffer::Event::Release = event {
            if let Some(index) = state.retired.iter().position(|old| old == buffer) {
                state.retired.swap_remove(index).destroy();
            } else if state.buffer.as_ref() == Some(buffer) {
                state.buffer_busy = false;
            }
        }
    }
}

fn pixels() -> Vec<u8> {
    // Native-endian premultiplied ARGB: 8% white background, 35% white lettering.
    let mut pixels = vec![0x14141414u32; (WIDTH * HEIGHT) as usize];
    let glyphs = [
        [
            0b00110, 0b01000, 0b11100, 0b01000, 0b01000, 0b01000, 0b01000,
        ], // f
        [0b00100, 0, 0b01100, 0b00100, 0b00100, 0b00100, 0b01110], // i
        [0, 0, 0b01110, 0b10001, 0b11111, 0b10000, 0b01110],       // e
        [0, 0, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100],       // v
        [0, 0, 0b01110, 0b10001, 0b11111, 0b10000, 0b01110],       // e
        [
            0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ], // l
    ];
    for (letter, glyph) in glyphs.iter().enumerate() {
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..5 {
                if bits & (1 << (4 - col)) != 0 {
                    for y in 0..2 {
                        for x in 0..2 {
                            pixels[(8 + row * 2 + y) * WIDTH as usize
                                + 9
                                + letter * 12
                                + col * 2
                                + x] = 0x59595959;
                        }
                    }
                }
            }
        }
    }
    pixels.into_iter().flat_map(u32::to_ne_bytes).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::{Config, Mode},
        engine::Engine,
    };
    use evdev::KeyCode as K;

    #[test]
    fn disabled_indicator_never_starts_a_worker() {
        let mut indicator = Indicator::new(false);
        indicator.set_active(true);
        indicator.set_active(false);
        assert!(indicator.0.is_none());
    }

    #[test]
    fn disabled_overlays_never_start_workers() {
        let mut overlays = Overlays::new(false, false);
        overlays.update(true, &[K::KEY_K], SpeedMode::Normal);
        overlays.clear();
        assert!(overlays.indicator.0.is_none());
        assert!(overlays.keycast.0.is_none());
    }

    #[test]
    fn speed_changes_publish_without_new_keys_and_empty_history_stays_hidden() {
        let (wake, mut receiver) = UnixStream::pair().unwrap();
        wake.set_nonblocking(true).unwrap();
        receiver.set_nonblocking(true).unwrap();
        let shared = Arc::new(State {
            keycast: true,
            ..State::default()
        });
        let mut overlays = Overlays {
            indicator: Indicator(None),
            keycast: Indicator(Some(Worker {
                state: Arc::clone(&shared),
                wake,
                thread: None,
            })),
        };
        let keys = [K::KEY_K, K::KEY_L];
        for (index, speed) in [
            SpeedMode::Normal, SpeedMode::Fast, SpeedMode::Slow, SpeedMode::Normal,
        ].into_iter().enumerate() {
            overlays.update(true, &keys, speed);
            assert_eq!(shared.revision.load(Ordering::SeqCst), index as u64 + 1);
            assert_eq!(shared.frame.lock().unwrap().speed, speed);
            assert_eq!(shared.frame.lock().unwrap().keys, keys);
            assert_eq!(receiver.read(&mut [0; 64]).unwrap(), 1);
            overlays.update(true, &keys, speed);
            assert_eq!(receiver.read(&mut [0; 64]).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        }
        overlays.clear();
        let revision = shared.revision.load(Ordering::SeqCst);
        overlays.update(true, &[], SpeedMode::Fast);
        assert_eq!(shared.revision.load(Ordering::SeqCst), revision);
        assert!(!shared.active.load(Ordering::SeqCst));
    }

    #[test]
    fn keycast_is_independent_and_suppresses_unchanged_updates() {
        let (wake, mut receiver) = UnixStream::pair().unwrap();
        wake.set_nonblocking(true).unwrap();
        receiver.set_nonblocking(true).unwrap();
        let shared = Arc::new(State {
            keycast: true,
            ..State::default()
        });
        let mut overlays = Overlays {
            indicator: Indicator(None),
            keycast: Indicator(Some(Worker {
                state: Arc::clone(&shared),
                wake,
                thread: None,
            })),
        };
        let keys = [K::KEY_K, K::KEY_L, K::KEY_COMMA, K::KEY_DOT, K::KEY_SPACE];
        overlays.update(true, &keys, SpeedMode::Normal);
        assert!(shared.active.load(Ordering::SeqCst));
        assert_eq!(shared.frame.lock().unwrap().keys, keys);
        assert_eq!(shared.revision.load(Ordering::SeqCst), 1);
        assert_eq!(receiver.read(&mut [0; 64]).unwrap(), 1);
        let allocation = shared.frame.lock().unwrap().keys.as_ptr();
        for _ in 0..1000 {
            overlays.update(true, &keys, SpeedMode::Normal);
        }
        assert_eq!(shared.frame.lock().unwrap().keys.as_ptr(), allocation);
        assert_eq!(shared.revision.load(Ordering::SeqCst), 1);
        assert_eq!(
            receiver.read(&mut [0; 64]).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        overlays.update(true, &[K::KEY_L, K::KEY_K], SpeedMode::Normal);
        assert_eq!(shared.frame.lock().unwrap().keys, [K::KEY_L, K::KEY_K]);
        assert_eq!(shared.revision.load(Ordering::SeqCst), 2);
        overlays.update(true, &[], SpeedMode::Normal);
        assert!(!shared.active.load(Ordering::SeqCst));
        assert!(shared.frame.lock().unwrap().keys.is_empty());
        overlays.update(true, &keys, SpeedMode::Normal);
        overlays.update(false, &keys, SpeedMode::Normal);
        assert!(!shared.active.load(Ordering::SeqCst));
        assert!(shared.frame.lock().unwrap().keys.is_empty());
        overlays.update(true, &[K::KEY_A; 40], SpeedMode::Normal);
        assert_eq!(shared.frame.lock().unwrap().keys.len(), 32);
        let revision = shared.revision.load(Ordering::SeqCst);
        overlays.update(true, &[K::KEY_A; 32], SpeedMode::Normal);
        assert_eq!(shared.revision.load(Ordering::SeqCst), revision);
        overlays.clear();
        assert!(!shared.active.load(Ordering::SeqCst));
        assert!(shared.frame.lock().unwrap().keys.is_empty());
        drop(overlays);
        assert!(shared.stop.load(Ordering::SeqCst));
    }

    #[test]
    fn hold_toggle_and_shutdown_publish_latest_state() {
        for mode in [Mode::Hold, Mode::Toggle] {
            let (wake, mut receiver) = UnixStream::pair().unwrap();
            wake.set_nonblocking(true).unwrap();
            receiver.set_nonblocking(true).unwrap();
            let shared = Arc::new(State::default());
            let mut indicator = Indicator(Some(Worker {
                state: Arc::clone(&shared),
                wake,
                thread: None,
            }));
            let mut engine = Engine::new(Config {
                mode,
                ..Config::default()
            });
            assert!(!shared.active.load(Ordering::SeqCst));
            for (key, value) in [
                (K::KEY_F3, 1),
                (K::KEY_F3, 2),
                (K::KEY_F3, 0),
                (K::KEY_F3, 1),
                (K::KEY_F3, 0),
                (K::KEY_F3, 1),
                (K::KEY_ESC, 1),
            ] {
                engine.key(key, value);
                indicator.set_active(engine.active());
                assert_eq!(shared.active.load(Ordering::SeqCst), engine.active(),);
            }
            engine.release_all();
            indicator.set_active(engine.active());
            assert!(!shared.active.load(Ordering::SeqCst));
            indicator.set_active(true);
            drop(indicator);
            assert!(!shared.active.load(Ordering::SeqCst));
            assert!(shared.stop.load(Ordering::SeqCst));
            assert!(receiver.read(&mut [0; 64]).unwrap() > 0);
        }
    }

    #[test]
    fn pixels_are_translucent_premultiplied_argb() {
        let pixels = pixels();
        assert_eq!(pixels.len(), (WIDTH * HEIGHT * 4) as usize);
        assert!(pixels.chunks_exact(4).any(|pixel| pixel == [0x59; 4]));
        for pixel in pixels.chunks_exact(4) {
            assert!(pixel == [0x14; 4] || pixel == [0x59; 4]);
        }
    }

    #[test]
    fn stalled_compositor_does_not_block_shutdown() {
        let (client, _server) = UnixStream::pair().unwrap();
        let connection = Connection::from_socket(client).unwrap();
        let (mut wake, mut receiver) = UnixStream::pair().unwrap();
        receiver.set_nonblocking(true).unwrap();
        let shared = Arc::new(State::default());
        let worker_shared = Arc::clone(&shared);
        let worker = thread::spawn(move || {
            display_connection(connection, worker_shared, &mut receiver).is_ok()
        });
        thread::sleep(Duration::from_millis(30));
        let started = Instant::now();
        shared.stop.store(true, Ordering::SeqCst);
        wake.write_all(&[1]).unwrap();
        assert!(worker.join().unwrap());
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn stalled_compositor_startup_times_out() {
        let (client, _server) = UnixStream::pair().unwrap();
        let connection = Connection::from_socket(client).unwrap();
        let (_wake, mut receiver) = UnixStream::pair().unwrap();
        receiver.set_nonblocking(true).unwrap();
        let error =
            display_connection(connection, Arc::new(State::default()), &mut receiver).unwrap_err();
        assert!(error
            .to_string()
            .contains("timed out waiting for Wayland globals"));
    }

    #[test]
    #[ignore = "requires a Wayland compositor with wlr-layer-shell; no input devices are opened"]
    fn wayland_indicator_lifecycle() {
        let mut indicator = Indicator::new(true);
        assert!(indicator.0.is_some());
        for _ in 0..2 {
            indicator.set_active(true);
            thread::sleep(Duration::from_secs(2));
            assert!(indicator
                .0
                .as_ref()
                .unwrap()
                .state
                .mapped
                .load(Ordering::SeqCst));
            assert!(!indicator
                .0
                .as_ref()
                .unwrap()
                .thread
                .as_ref()
                .unwrap()
                .is_finished());
            indicator.set_active(false);
            thread::sleep(Duration::from_secs(1));
            assert!(!indicator
                .0
                .as_ref()
                .unwrap()
                .state
                .mapped
                .load(Ordering::SeqCst));
        }
    }

    #[test]
    #[ignore = "requires a Wayland compositor with wlr-layer-shell; no input devices are opened"]
    fn wayland_keycast_lifecycle_without_mode_indicator() {
        let mut overlays = Overlays::new(false, true);
        assert!(overlays.indicator.0.is_none());
        assert!(overlays.keycast.0.is_some());
        for _ in 0..2 {
            for keys in [
                vec![K::KEY_K, K::KEY_L, K::KEY_SPACE, K::KEY_COMMA, K::KEY_DOT],
                vec![K::KEY_BRIGHTNESSUP; 32],
                vec![K::KEY_A],
            ] {
                overlays.update(true, &keys, SpeedMode::Normal);
                thread::sleep(Duration::from_secs(1));
                let worker = overlays.keycast.0.as_ref().unwrap();
                assert!(worker.state.mapped.load(Ordering::SeqCst));
                assert!(!worker.thread.as_ref().unwrap().is_finished());
            }
            for count in 1..=128 {
                overlays.update(true, &vec![K::KEY_LEFTCTRL; count % 32 + 1], SpeedMode::Normal);
                thread::sleep(Duration::from_millis(4));
            }
            overlays.update(true, &[K::KEY_K, K::KEY_L], SpeedMode::Normal);
            thread::sleep(Duration::from_millis(200));
            assert!(!overlays
                .keycast
                .0
                .as_ref()
                .unwrap()
                .thread
                .as_ref()
                .unwrap()
                .is_finished());
            overlays.clear();
            thread::sleep(Duration::from_millis(200));
            assert!(!overlays
                .keycast
                .0
                .as_ref()
                .unwrap()
                .state
                .mapped
                .load(Ordering::SeqCst));
        }
        let started = Instant::now();
        drop(overlays);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
