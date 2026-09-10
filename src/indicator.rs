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
        atomic::{AtomicBool, Ordering},
        Arc,
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

const WIDTH: i32 = 88;
const HEIGHT: i32 = 30;
const STARTUP_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Default)]
struct State {
    active: AtomicBool,
    stop: AtomicBool,
    mapped: AtomicBool,
}

struct Worker {
    state: Arc<State>,
    wake: UnixStream,
    thread: Option<JoinHandle<()>>,
}

/// No display connection, socket, or thread is created when disabled.
pub struct Indicator(Option<Worker>);

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
        let (wake, receiver) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        receiver.set_nonblocking(true)?;
        let state = Arc::new(State::default());
        let shared = Arc::clone(&state);
        let thread = thread::Builder::new()
            .name("fievel-indicator".into())
            .spawn(move || {
                if let Err(error) = display(shared, receiver) {
                    warn(error.as_ref());
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
}

impl Worker {
    fn wake(&mut self) {
        // A full socket already has a wakeup pending; the atomic is authoritative.
        match self.wake.write(&[1]) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                if !self.thread.as_ref().is_some_and(|thread| thread.is_finished()) {
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

struct Surface {
    surface: wl_surface::WlSurface,
    layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    configured: bool,
    created: Instant,
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
}

impl Display {
    fn initialize(&mut self, qh: &QueueHandle<Self>) -> Result<(), Box<dyn Error>> {
        self.compositor.as_ref().ok_or("missing wl_compositor")?;
        self.shell
            .as_ref()
            .ok_or("compositor does not support zwlr_layer_shell_v1")?;
        let shm = self.shm.as_ref().ok_or("missing wl_shm")?;
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
    }

    fn update(&mut self, qh: &QueueHandle<Self>) {
        if !self.shared.active.load(Ordering::SeqCst) {
            self.hide();
        } else if self.surface.is_none() {
            let compositor = self.compositor.as_ref().unwrap();
            let surface = compositor.create_surface(qh, ());
            let region = compositor.create_region(qh, ());
            surface.set_input_region(Some(&region));
            region.destroy();
            let layer = self.shell.as_ref().unwrap().get_layer_surface(
                &surface,
                None,
                Layer::Overlay,
                "fievel-indicator".into(),
                qh,
                (),
            );
            layer.set_size(WIDTH as u32, HEIGHT as u32);
            layer.set_anchor(Anchor::Bottom | Anchor::Left);
            layer.set_margin(0, 0, 12, 12);
            layer.set_exclusive_zone(-1);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            surface.commit();
            self.surface = Some(Surface {
                surface,
                layer,
                configured: false,
                created: Instant::now(),
            });
        }
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
        if display.globals_ready {
            if display.buffer.is_none() {
                display.initialize(&qh)?;
            }
            display.update(&qh);
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
            zwlr_layer_surface_v1::Event::Configure { serial, .. } => {
                layer.ack_configure(serial);
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
delegate_noop!(Display: ignore wl_buffer::WlBuffer);
delegate_noop!(Display: ignore wl_surface::WlSurface);
delegate_noop!(Display: ignore wl_region::WlRegion);
delegate_noop!(Display: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

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
                indicator.set_active(engine.active() && !engine.emergency_exit());
                assert_eq!(
                    shared.active.load(Ordering::SeqCst),
                    engine.active() && !engine.emergency_exit(),
                );
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
            assert!(indicator.0.as_ref().unwrap().state.mapped.load(Ordering::SeqCst));
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
            assert!(!indicator.0.as_ref().unwrap().state.mapped.load(Ordering::SeqCst));
        }
    }
}
