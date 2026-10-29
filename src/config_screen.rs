use crate::{
    config::{Config, Mode},
    font::Color,
    input::KeyCode as K,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{hash_map::RandomState, BTreeMap, HashMap},
    fs::{self, OpenOptions},
    hash::{BuildHasher, Hash, Hasher},
    io::{self, BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    process::Command,
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread::{self, JoinHandle},
    time::{Duration, SystemTime},
};

const MAX_HEADERS: usize = 8 * 1024;
const MAX_CONFIG_SIZE: usize = 1024 * 1024;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EditorConfig {
    win_super_is_alt: bool,
    mode: String,
    notify: bool,
    home_end_enabled: bool,
    speeds: EditorSpeeds,
    easing: EditorEasing,
    keys: EditorKeys,
    keycast: EditorKeycast,
    hints: EditorHints,
    remap: EditorRemap,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EditorSpeeds {
    normal: f64,
    slow: f64,
    fast: f64,
    scroll: f64,
    scroll_slow: f64,
    scroll_fast: f64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EditorEasing {
    movement: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    movement_stop: Option<f64>,
    scroll: f64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EditorKeys {
    free_mouse: String,
    reload_config: String,
    config_screen: String,
    toggle_hold: String,
    left: String,
    down: String,
    up: String,
    right: String,
    left_click: String,
    right_click: String,
    scroll_left: String,
    scroll_down: String,
    scroll_up: String,
    scroll_right: String,
    slow: String,
    fast: String,
    locate: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EditorKeycast {
    enabled: bool,
    hotkey: String,
    max_keys: usize,
    timeout: f64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EditorHints {
    recursive_grid_enabled: bool,
    keys: EditorHintKeys,
    label_symbols: String,
    border_color: String,
    fill_color: String,
    grid_colors: EditorGridColors,
    readability_color: String,
    label_color: String,
    label_highlight_color: String,
    min_width: u32,
    max_width: u32,
    min_height: u32,
    max_height: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EditorGridColors {
    #[serde(skip_serializing_if = "Option::is_none")]
    upper_left: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upper_right: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lower_left: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lower_right: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EditorHintKeys {
    left: String,
    right: String,
    grid: String,
    grid_upper_left: String,
    grid_upper_right: String,
    grid_lower_left: String,
    grid_lower_right: String,
    grid_left_click: String,
    grid_right_click: String,
    cancel: String,
    toggle_background: String,
    debug: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EditorRemap {
    chord_timeout: u64,
    main: BTreeMap<String, String>,
    layers: BTreeMap<String, BTreeMap<String, String>>,
}

fn editor_key(key: K) -> String {
    format!("{key:?}").trim_start_matches("KEY_").to_owned()
}

fn editor_chord(chord: &[K]) -> String {
    chord
        .iter()
        .copied()
        .map(editor_key)
        .collect::<Vec<_>>()
        .join(" + ")
}

fn editor_color(color: Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}{:02x}",
        color.r, color.g, color.b, color.a
    )
}

impl From<&Config> for EditorConfig {
    fn from(config: &Config) -> Self {
        let keys = &config.keys;
        let hint_keys = &config.hints.keys;
        Self {
            win_super_is_alt: config.win_super_is_alt,
            mode: match config.mode {
                Mode::Hold => "hold",
                Mode::Toggle => "toggle",
            }
            .to_owned(),
            notify: config.notify,
            home_end_enabled: config.home_end_enabled,
            speeds: EditorSpeeds {
                normal: config.speeds.normal,
                slow: config.speeds.slow,
                fast: config.speeds.fast,
                scroll: config.speeds.scroll,
                scroll_slow: config.speeds.scroll_slow,
                scroll_fast: config.speeds.scroll_fast,
            },
            easing: EditorEasing {
                movement: config.easing.movement,
                movement_stop: config.easing.movement_stop,
                scroll: config.easing.scroll,
            },
            keys: EditorKeys {
                free_mouse: editor_chord(&keys.free_mouse),
                reload_config: editor_chord(&keys.reload_config),
                config_screen: editor_chord(&keys.config_screen),
                toggle_hold: editor_key(keys.toggle_hold),
                left: editor_key(keys.left),
                down: editor_key(keys.down),
                up: editor_key(keys.up),
                right: editor_key(keys.right),
                left_click: editor_key(keys.left_click),
                right_click: editor_key(keys.right_click),
                scroll_left: editor_key(keys.scroll_left),
                scroll_down: editor_key(keys.scroll_down),
                scroll_up: editor_key(keys.scroll_up),
                scroll_right: editor_key(keys.scroll_right),
                slow: editor_key(keys.slow),
                fast: editor_key(keys.fast),
                locate: editor_key(keys.locate),
            },
            keycast: EditorKeycast {
                enabled: config.keycast.enabled,
                hotkey: editor_key(config.keycast.hotkey),
                max_keys: config.keycast.max_keys,
                timeout: config.keycast.timeout,
            },
            hints: EditorHints {
                recursive_grid_enabled: config.hints.recursive_grid_enabled,
                keys: EditorHintKeys {
                    left: editor_chord(&hint_keys.left),
                    right: editor_chord(&hint_keys.right),
                    grid: editor_chord(&hint_keys.grid),
                    grid_upper_left: editor_key(hint_keys.grid_upper_left),
                    grid_upper_right: editor_key(hint_keys.grid_upper_right),
                    grid_lower_left: editor_key(hint_keys.grid_lower_left),
                    grid_lower_right: editor_key(hint_keys.grid_lower_right),
                    grid_left_click: editor_key(hint_keys.grid_left_click),
                    grid_right_click: editor_key(hint_keys.grid_right_click),
                    cancel: editor_key(hint_keys.cancel),
                    toggle_background: editor_key(hint_keys.toggle_background),
                    debug: editor_key(hint_keys.debug),
                },
                label_symbols: config.hints.label_symbols.clone(),
                border_color: editor_color(config.hints.border_color),
                fill_color: editor_color(config.hints.fill_color),
                grid_colors: EditorGridColors {
                    upper_left: config.hints.grid_colors.upper_left.map(editor_color),
                    upper_right: config.hints.grid_colors.upper_right.map(editor_color),
                    lower_left: config.hints.grid_colors.lower_left.map(editor_color),
                    lower_right: config.hints.grid_colors.lower_right.map(editor_color),
                },
                readability_color: editor_color(config.hints.readability_color),
                label_color: editor_color(config.hints.label_color),
                label_highlight_color: editor_color(config.hints.label_highlight_color),
                min_width: config.hints.min_width,
                max_width: config.hints.max_width,
                min_height: config.hints.min_height,
                max_height: config.hints.max_height,
            },
            remap: EditorRemap {
                chord_timeout: config.remap.chord_timeout,
                main: config.remap.main.clone(),
                layers: config.remap.layers.clone(),
            },
        }
    }
}

enum WorkerCommand {
    Open,
    Stop,
}

pub struct ConfigScreen {
    command_sender: Sender<WorkerCommand>,
    reload_receiver: Receiver<()>,
    worker: Option<JoinHandle<io::Result<()>>>,
}

impl ConfigScreen {
    pub fn start(config_path: &Path) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let address = listener.local_addr()?;
        let token = make_token(address.port());
        let config_path = config_path.to_path_buf();
        let (command_sender, command_receiver) = mpsc::channel();
        let (reload_sender, reload_receiver) = mpsc::channel();
        let url = format!("http://127.0.0.1:{}/?token={token}", address.port());
        let worker = thread::Builder::new()
            .name("fievel-config-screen".into())
            .spawn(move || {
                serve(
                    listener,
                    &url,
                    &token,
                    &config_path,
                    command_receiver,
                    reload_sender,
                )
            })?;

        Ok(Self {
            command_sender,
            reload_receiver,
            worker: Some(worker),
        })
    }

    pub fn open(&self) -> io::Result<()> {
        self.command_sender
            .send(WorkerCommand::Open)
            .map_err(|_| io::Error::other("configuration screen stopped unexpectedly"))
    }

    pub fn take_reload_request(&self) -> io::Result<bool> {
        let mut requested = false;
        loop {
            match self.reload_receiver.try_recv() {
                Ok(()) => requested = true,
                Err(TryRecvError::Empty) => return Ok(requested),
                Err(TryRecvError::Disconnected) => {
                    return Err(io::Error::other(
                        "configuration screen stopped unexpectedly",
                    ))
                }
            }
        }
    }
}

impl Drop for ConfigScreen {
    fn drop(&mut self) {
        let _ = self.command_sender.send(WorkerCommand::Stop);
        if let Some(worker) = self.worker.take() {
            match worker.join() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => eprintln!("fievel: config screen stopped: {error}"),
                Err(_) => eprintln!("fievel: config screen thread panicked"),
            }
        }
    }
}

fn make_token(port: u16) -> String {
    let material = (SystemTime::now(), std::process::id(), port);
    [RandomState::new(), RandomState::new()]
        .into_iter()
        .map(|state| {
            let mut hasher = state.build_hasher();
            material.hash(&mut hasher);
            format!("{:016x}", hasher.finish())
        })
        .collect::<Vec<_>>()
        .join("")
}

fn serve(
    listener: TcpListener,
    url: &str,
    token: &str,
    config_path: &Path,
    command_receiver: Receiver<WorkerCommand>,
    reload_sender: Sender<()>,
) -> io::Result<()> {
    let port = listener.local_addr()?.port();
    loop {
        match command_receiver.try_recv() {
            Ok(WorkerCommand::Open) => {
                if let Err(error) = open_browser(url) {
                    eprintln!("fievel: could not open configuration screen: {error}");
                }
            }
            Ok(WorkerCommand::Stop) => return Ok(()),
            Err(TryRecvError::Disconnected) => return Ok(()),
            Err(TryRecvError::Empty) => {}
        }

        match listener.accept() {
            Ok((mut stream, _)) => {
                if let Err(error) =
                    handle_connection(&mut stream, port, token, config_path, &reload_sender)
                {
                    eprintln!("fievel: configuration screen request failed: {error}");
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

fn open_browser(url: &str) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("rundll32.exe");
        command.args(["url.dll,FileProtocolHandler", url]);
        command
    };
    command.spawn().map(|_| ())
}

struct HttpRequest {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn read_request(stream: &TcpStream) -> io::Result<HttpRequest> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut header_size = 0;
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "empty HTTP request",
        ));
    }
    header_size += line.len();
    if header_size > MAX_HEADERS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "HTTP request line is too large",
        ));
    }
    let mut request_line = line.split_whitespace();
    let method = request_line.next().unwrap_or_default().to_owned();
    let target = request_line.next().unwrap_or_default().to_owned();
    if request_line.next().is_none() || method.is_empty() || target.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid HTTP request line",
        ));
    }

    let mut headers = HashMap::new();
    loop {
        line.clear();
        let length = reader.read_line(&mut line)?;
        if length == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "incomplete HTTP headers",
            ));
        }
        header_size += length;
        if header_size > MAX_HEADERS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP headers are too large",
            ));
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP header"))?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_owned());
    }

    if headers.contains_key("transfer-encoding") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "chunked requests are not supported",
        ));
    }
    let content_length = headers.get("content-length").map_or(Ok(0), |value| {
        value
            .parse::<usize>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid content length"))
    })?;
    if content_length > MAX_CONFIG_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "configuration is too large",
        ));
    }
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body)?;
    Ok(HttpRequest {
        method,
        target,
        headers,
        body,
    })
}

fn handle_connection(
    stream: &mut TcpStream,
    port: u16,
    token: &str,
    config_path: &Path,
    reload_sender: &Sender<()>,
) -> io::Result<()> {
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let request = match read_request(stream) {
        Ok(request) => request,
        Err(error) => {
            return write_response(
                stream,
                400,
                "text/plain; charset=utf-8",
                error.to_string().as_bytes(),
            )
        }
    };
    let expected_host = format!("127.0.0.1:{port}");
    if request.headers.get("host").map(String::as_str) != Some(expected_host.as_str()) {
        return write_response(stream, 403, "text/plain; charset=utf-8", b"Forbidden");
    }
    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((&request.target, ""));
    if !query
        .split('&')
        .any(|part| part.strip_prefix("token=") == Some(token))
    {
        return write_response(stream, 403, "text/plain; charset=utf-8", b"Forbidden");
    }

    match (request.method.as_str(), path) {
        ("GET", "/") => write_response(
            stream,
            200,
            "text/html; charset=utf-8",
            CONFIG_PAGE.as_bytes(),
        ),
        ("GET", "/api/config") => match load_editor_config(config_path) {
            Ok(config) => write_response(
                stream,
                200,
                "application/json; charset=utf-8",
                config.as_bytes(),
            ),
            Err(error) => write_response(
                stream,
                500,
                "text/plain; charset=utf-8",
                error.to_string().as_bytes(),
            ),
        },
        ("POST", "/api/config") => {
            let expected_origin = format!("http://127.0.0.1:{port}");
            if request.headers.get("origin").map(String::as_str) != Some(&expected_origin) {
                return write_response(stream, 403, "text/plain; charset=utf-8", b"Forbidden");
            }
            match save_config(config_path, &request.body) {
                Ok(canonical) => {
                    reload_sender
                        .send(())
                        .map_err(|_| io::Error::other("configuration reload receiver stopped"))?;
                    write_response(
                        stream,
                        200,
                        "application/json; charset=utf-8",
                        canonical.as_bytes(),
                    )
                }
                Err(error) => write_response(
                    stream,
                    400,
                    "text/plain; charset=utf-8",
                    error.to_string().as_bytes(),
                ),
            }
        }
        _ => write_response(stream, 404, "text/plain; charset=utf-8", b"Not found"),
    }
}

fn load_editor_config(config_path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    let config = Config::load(config_path, false)?;
    config.validate().map_err(io::Error::other)?;
    Ok(serde_json::to_string(&EditorConfig::from(&config))?)
}

fn save_config(config_path: &Path, source: &[u8]) -> Result<String, Box<dyn std::error::Error>> {
    let editor: EditorConfig = serde_json::from_slice(source)?;
    let toml = toml::to_string(&editor)?;
    let config = Config::parse_strict(&toml)?;
    let canonical = config.to_toml()?;
    write_atomically(config_path, canonical.as_bytes())?;
    Ok(serde_json::to_string(&EditorConfig::from(&config))?)
}

fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| io::Error::other("config path has no file name"))?
        .to_string_lossy();
    let temp_path = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)?;
    let result = (|| {
        file.write_all(contents)?;
        file.sync_all()?;
        fs::rename(&temp_path, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Content-Security-Policy: default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'\r\n\
         Referrer-Policy: no-referrer\r\n\
         Connection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

const CONFIG_PAGE: &str = r##"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Fievel Settings</title>
  <style>
    :root { color-scheme: dark; font: 14px/1.5 system-ui, sans-serif; background: #101318; color: #e6e9ef; }
    * { box-sizing: border-box; }
    body { margin: 0; min-height: 100vh; padding: 24px; }
    main { width: min(1100px, 100%); margin: 0 auto; background: #1a1f27; border: 1px solid #303846; border-radius: 14px; padding: 24px; box-shadow: 0 24px 80px #0008; }
    h1 { margin: 0; font-size: 22px; }
    h2 { margin: 0 0 14px; font-size: 17px; }
    p { color: #aab3c2; margin: 6px 0 16px; }
    section { border-top: 1px solid #303846; padding: 20px 0; }
    .fields { display: grid; grid-template-columns: repeat(auto-fit, minmax(min(100%, 260px), 1fr)); gap: 14px; }
    .field { display: flex; min-width: 0; flex-direction: column; gap: 6px; color: #d7e1ef; }
    .field > span, .field-title { font-weight: 600; }
    .field small, .section-help { color: #9aa6b8; font-size: 12px; font-weight: 400; }
    input, select { min-width: 0; width: 100%; border: 1px solid #384354; border-radius: 6px; padding: 8px 9px; background: #0c0f13; color: #e6e9ef; font: inherit; }
    input[type="checkbox"] { width: 17px; height: 17px; accent-color: #68a7ff; }
    input[type="color"] { width: 56px; height: 36px; padding: 3px; }
    input[type="range"] { padding: 0; accent-color: #68a7ff; }
    .check-row, .color-row, .mapping-row, .remap-heading { display: flex; align-items: center; gap: 10px; }
    .check-row { min-height: 38px; }
    .check-row input { flex: 0 0 auto; }
    .color-row input[type="range"] { flex: 1; }
    .color-row output { width: 36px; color: #aab3c2; text-align: right; }
    .field-check { flex-direction: row; align-items: center; gap: 10px; }
    .field-check input { flex: 0 0 auto; }
    .map-list { display: grid; gap: 8px; }
    .mapping-row input { flex: 1; }
    .mapping-row button { flex: 0 0 auto; }
    .layer { border: 1px solid #384354; border-radius: 8px; padding: 12px; margin-top: 12px; }
    .layer .map-list { margin-top: 10px; }
    .remap-heading input { max-width: 260px; }
    footer { display: flex; align-items: center; justify-content: space-between; gap: 12px; border-top: 1px solid #303846; padding-top: 16px; }
    #status { color: #aab3c2; min-height: 1.5em; }
    button { border: 0; border-radius: 7px; padding: 9px 15px; background: #68a7ff; color: #111820; font: inherit; font-weight: 650; cursor: pointer; }
    button.secondary { background: #303846; color: #e6e9ef; }
    button.compact { padding: 6px 10px; font-size: 12px; }
    button:disabled { opacity: .6; cursor: wait; }
    @media (max-width: 600px) { body { padding: 10px; } main { padding: 16px; } footer { align-items: stretch; flex-direction: column-reverse; } footer > div { display: flex; gap: 8px; } footer button { flex: 1; } .mapping-row { flex-wrap: wrap; } }
  </style>
</head>
<body>
  <main>
    <h1>Fievel Settings</h1>
    <p>View and change individual settings and keybinds. Key chords use names joined with +. Saving validates and applies changes immediately.</p>
    <div id="settings"></div>
    <section>
      <h2>Remapping</h2>
      <p class="section-help">Triggers are key chords. Actions are key names, <code>free_mouse</code>, <code>layer(name)</code>, or <code>timeout(short, milliseconds, held)</code>.</p>
      <div id="remap-timeout"></div>
      <h3>Main mappings</h3>
      <div id="main-mappings" class="map-list"></div>
      <button id="add-main-mapping" class="secondary compact" type="button">Add mapping</button>
      <div id="layers"></div>
      <button id="add-layer" class="secondary compact" type="button">Add layer</button>
    </section>
    <footer>
      <span id="status" role="status">Loading settings…</span>
      <div>
        <button id="reload" class="secondary" type="button">Discard changes</button>
        <button id="save" type="button">Save and apply</button>
      </div>
    </footer>
  </main>
  <script>
    const token = new URLSearchParams(location.search).get("token") || "";
    const status = document.querySelector("#status");
    const saveButton = document.querySelector("#save");
    const endpoint = `/api/config?token=${encodeURIComponent(token)}`;
    const fieldElements = new Map();
    let currentConfig = null;
    const sections = [
      { title: "General", fields: [
        { path: "mode", label: "Free Mouse mode", kind: "select", options: [["hold", "Hold"], ["toggle", "Toggle"]] },
        { path: "notify", label: "Show notifications", kind: "checkbox" },
        { path: "home_end_enabled", label: "Enable Home/End navigation", kind: "checkbox" },
        { path: "win_super_is_alt", label: "Treat Super as Alt on Windows", kind: "checkbox", help: "Platform compatibility setting." }
      ]},
      { title: "Movement and scrolling", fields: [
        { path: "speeds.normal", label: "Movement speed", kind: "number", min: 0.0001, max: 100000, step: "any" },
        { path: "speeds.slow", label: "Slow movement speed", kind: "number", min: 0.0001, max: 100000, step: "any" },
        { path: "speeds.fast", label: "Fast movement speed", kind: "number", min: 0.0001, max: 100000, step: "any" },
        { path: "speeds.scroll", label: "Scroll speed", kind: "number", min: 0.0001, max: 100000, step: "any" },
        { path: "speeds.scroll_slow", label: "Slow scroll speed", kind: "number", min: 0.0001, max: 100000, step: "any" },
        { path: "speeds.scroll_fast", label: "Fast scroll speed", kind: "number", min: 0.0001, max: 100000, step: "any" },
        { path: "easing.movement", label: "Movement easing", kind: "number", min: 0, max: 1, step: 0.01 },
        { path: "easing.movement_stop", label: "Separate easing while stopping", kind: "number", min: 0, max: 1, step: 0.01, optional: true },
        { path: "easing.scroll", label: "Scroll easing", kind: "number", min: 0, max: 1, step: 0.01 }
      ]},
      { title: "Main keybinds", fields: [
        { path: "keys.free_mouse", label: "Free Mouse activation", kind: "chord" },
        { path: "keys.reload_config", label: "Reload configuration", kind: "chord" },
        { path: "keys.config_screen", label: "Open settings screen", kind: "chord" },
        { path: "keys.toggle_hold", label: "Toggle Hold mode", kind: "key" },
        { path: "keys.left", label: "Move left", kind: "key" },
        { path: "keys.down", label: "Move down", kind: "key" },
        { path: "keys.up", label: "Move up", kind: "key" },
        { path: "keys.right", label: "Move right", kind: "key" },
        { path: "keys.left_click", label: "Left click", kind: "key" },
        { path: "keys.right_click", label: "Right click", kind: "key" },
        { path: "keys.scroll_left", label: "Scroll left", kind: "key" },
        { path: "keys.scroll_down", label: "Scroll down", kind: "key" },
        { path: "keys.scroll_up", label: "Scroll up", kind: "key" },
        { path: "keys.scroll_right", label: "Scroll right", kind: "key" },
        { path: "keys.slow", label: "Slow modifier", kind: "key" },
        { path: "keys.fast", label: "Fast modifier", kind: "key" },
        { path: "keys.locate", label: "Locate pointer", kind: "key" }
      ]},
      { title: "Keycast", fields: [
        { path: "keycast.enabled", label: "Enable keycast", kind: "checkbox" },
        { path: "keycast.hotkey", label: "Keycast hotkey", kind: "key" },
        { path: "keycast.max_keys", label: "Maximum displayed keys", kind: "number", min: 1, max: 32, step: 1 },
        { path: "keycast.timeout", label: "Display timeout (seconds)", kind: "number", min: 0.0001, max: 60, step: 0.1 }
      ]},
      { title: "Hints and appearance", fields: [
        { path: "hints.recursive_grid_enabled", label: "Enable recursive grid", kind: "checkbox" },
        { path: "hints.label_symbols", label: "Label symbols", kind: "text", help: "Each character is used as a hint label." },
        { path: "hints.border_color", label: "Border color", kind: "color" },
        { path: "hints.fill_color", label: "Fill color", kind: "color" },
        { path: "hints.grid_colors.upper_left", label: "Upper-left grid color override", kind: "color", optional: true },
        { path: "hints.grid_colors.upper_right", label: "Upper-right grid color override", kind: "color", optional: true },
        { path: "hints.grid_colors.lower_left", label: "Lower-left grid color override", kind: "color", optional: true },
        { path: "hints.grid_colors.lower_right", label: "Lower-right grid color override", kind: "color", optional: true },
        { path: "hints.readability_color", label: "Readability overlay color", kind: "color" },
        { path: "hints.label_color", label: "Label color", kind: "color" },
        { path: "hints.label_highlight_color", label: "Highlighted label color", kind: "color" },
        { path: "hints.min_width", label: "Minimum grid width", kind: "number", min: 1, step: 1 },
        { path: "hints.max_width", label: "Maximum grid width", kind: "number", min: 1, step: 1 },
        { path: "hints.min_height", label: "Minimum grid height", kind: "number", min: 1, step: 1 },
        { path: "hints.max_height", label: "Maximum grid height", kind: "number", min: 1, step: 1 }
      ]},
      { title: "Hint keybinds", fields: [
        { path: "hints.keys.left", label: "Previous hint page", kind: "chord" },
        { path: "hints.keys.right", label: "Next hint page", kind: "chord" },
        { path: "hints.keys.grid", label: "Toggle hint grid", kind: "chord" },
        { path: "hints.keys.grid_upper_left", label: "Select upper-left grid", kind: "key" },
        { path: "hints.keys.grid_upper_right", label: "Select upper-right grid", kind: "key" },
        { path: "hints.keys.grid_lower_left", label: "Select lower-left grid", kind: "key" },
        { path: "hints.keys.grid_lower_right", label: "Select lower-right grid", kind: "key" },
        { path: "hints.keys.grid_left_click", label: "Grid left click", kind: "key" },
        { path: "hints.keys.grid_right_click", label: "Grid right click", kind: "key" },
        { path: "hints.keys.cancel", label: "Cancel hints", kind: "key" },
        { path: "hints.keys.toggle_background", label: "Toggle hint background", kind: "key" },
        { path: "hints.keys.debug", label: "Debug key", kind: "key" }
      ]},
    ];
    const remapTimeoutField = {
      path: "remap.chord_timeout", label: "Chord timeout (milliseconds)",
      kind: "number", min: 1, step: 1
    };

    function getPath(object, path) {
      return path.split(".").reduce((value, key) => value == null ? undefined : value[key], object);
    }

    function setPath(object, path, value) {
      const keys = path.split(".");
      const leaf = keys.pop();
      const parent = keys.reduce((value, key) => value[key], object);
      parent[leaf] = value;
    }

    function makeInput(type, value, field) {
      const input = document.createElement("input");
      input.type = type;
      input.value = value ?? "";
      if (field.min !== undefined) input.min = field.min;
      if (field.max !== undefined) input.max = field.max;
      if (field.step !== undefined) input.step = field.step;
      if (field.kind === "key" || field.kind === "chord") {
        input.autocomplete = "off";
        input.spellcheck = false;
        input.placeholder = field.kind === "chord" ? "super + shift + slash" : "SPACE";
      }
      return input;
    }

    function renderField(field, value) {
      const root = document.createElement("div");
      root.className = "field";
      root.dataset.path = field.path;
      const id = `setting-${field.path.replaceAll(".", "-")}`;
      const title = document.createElement("label");
      title.className = "field-title";
      title.textContent = field.label;
      title.htmlFor = id;
      root.append(title);
      if (field.kind === "checkbox") {
        root.classList.add("field-check");
        const input = makeInput("checkbox", "", field);
        input.id = id;
        input.checked = Boolean(value);
        root.prepend(input);
        title.className = "";
      } else if (field.kind === "select") {
        const select = document.createElement("select");
        select.id = id;
        field.options.forEach(([optionValue, label]) => {
          const option = document.createElement("option");
          option.value = optionValue;
          option.textContent = label;
          select.append(option);
        });
        select.value = value;
        root.append(select);
      } else if (field.kind === "color") {
        const enabled = document.createElement("input");
        const optional = Boolean(field.optional);
        if (optional) {
          const toggle = document.createElement("div");
          toggle.className = "check-row";
          enabled.type = "checkbox";
          enabled.checked = value != null;
          enabled.setAttribute("aria-label", `Use custom ${field.label.toLowerCase()}`);
          const label = document.createElement("span");
          label.textContent = "Use custom color";
          toggle.append(enabled, label);
          root.append(toggle);
        }
        const color = makeInput("color", value ? value.slice(0, 7) : "#ffffff", field);
        color.id = id;
        const alpha = makeInput("range", value ? parseInt(value.slice(7, 9), 16) : 255, field);
        alpha.setAttribute("aria-label", `${field.label} alpha`);
        alpha.min = "0";
        alpha.max = "255";
        alpha.step = "1";
        const output = document.createElement("output");
        const updateAlpha = () => { output.value = `${Math.round(Number(alpha.value) / 255 * 100)}%`; };
        alpha.addEventListener("input", updateAlpha);
        updateAlpha();
        const row = document.createElement("div");
        row.className = "color-row";
        row.append(color, alpha, output);
        root.append(row);
        if (optional) {
          const updateEnabled = () => {
            color.disabled = !enabled.checked;
            alpha.disabled = !enabled.checked;
          };
          enabled.addEventListener("change", updateEnabled);
          updateEnabled();
          root._enabled = enabled;
        }
        root._color = color;
        root._alpha = alpha;
      } else {
        const inputType = field.kind === "number" ? "number" : "text";
        const input = makeInput(inputType, value, field);
        input.id = id;
        if (field.optional) {
          const toggle = document.createElement("div");
          toggle.className = "check-row";
          const enabled = document.createElement("input");
          enabled.type = "checkbox";
          enabled.checked = value != null;
          enabled.setAttribute("aria-label", `Use custom ${field.label.toLowerCase()}`);
          const label = document.createElement("span");
          label.textContent = "Use custom value";
          toggle.append(enabled, label);
          const updateEnabled = () => {
            input.disabled = !enabled.checked;
            if (!enabled.checked) input.value = "";
          };
          enabled.addEventListener("change", updateEnabled);
          updateEnabled();
          root.append(toggle);
          root._enabled = enabled;
        }
        root.append(input);
        root._input = input;
      }
      if (field.help) {
        const help = document.createElement("small");
        help.textContent = field.help;
        root.append(help);
      }
      fieldElements.set(field.path, { field, root });
      return root;
    }

    function renderSettings(config) {
      fieldElements.clear();
      const container = document.querySelector("#settings");
      container.replaceChildren();
      sections.forEach(section => {
        const element = document.createElement("section");
        const heading = document.createElement("h2");
        heading.textContent = section.title;
        const fields = document.createElement("div");
        fields.className = "fields";
        section.fields.forEach(field => {
          fields.append(renderField(field, getPath(config, field.path)));
        });
        element.append(heading, fields);
        container.append(element);
      });
      document.querySelector("#remap-timeout").replaceChildren(
        renderField(remapTimeoutField, getPath(config, remapTimeoutField.path))
      );
      renderRemaps(config.remap);
    }

    function mappingRow(trigger = "", action = "") {
      const row = document.createElement("div");
      row.className = "mapping-row";
      const triggerInput = document.createElement("input");
      triggerInput.className = "trigger";
      triggerInput.placeholder = "Trigger chord (e.g. CTRL + J)";
      triggerInput.value = trigger;
      triggerInput.setAttribute("aria-label", "Mapping trigger chord");
      const actionInput = document.createElement("input");
      actionInput.className = "action";
      actionInput.placeholder = "Action (e.g. ESC)";
      actionInput.value = action;
      actionInput.setAttribute("aria-label", "Mapping action");
      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "secondary compact";
      remove.textContent = "Remove";
      remove.addEventListener("click", () => row.remove());
      row.append(triggerInput, actionInput, remove);
      return row;
    }

    function renderRemaps(remap) {
      const main = document.querySelector("#main-mappings");
      main.replaceChildren(...Object.entries(remap.main).map(([trigger, action]) => mappingRow(trigger, action)));
      const layers = document.querySelector("#layers");
      layers.replaceChildren();
      Object.entries(remap.layers).forEach(([name, entries]) => {
        const layer = document.createElement("fieldset");
        layer.className = "layer";
        const heading = document.createElement("div");
        heading.className = "remap-heading";
        const nameInput = document.createElement("input");
        nameInput.className = "layer-name";
        nameInput.value = name;
        nameInput.placeholder = "Layer name";
        nameInput.setAttribute("aria-label", "Layer name");
        const remove = document.createElement("button");
        remove.type = "button";
        remove.className = "secondary compact";
        remove.textContent = "Remove layer";
        remove.addEventListener("click", () => layer.remove());
        heading.append(nameInput, remove);
        const list = document.createElement("div");
        list.className = "map-list";
        Object.entries(entries).forEach(([trigger, action]) => list.append(mappingRow(trigger, action)));
        const add = document.createElement("button");
        add.type = "button";
        add.className = "secondary compact";
        add.textContent = "Add mapping";
        add.addEventListener("click", () => list.append(mappingRow()));
        layer.append(heading, list, add);
        layers.append(layer);
      });
    }

    function readMappings(container) {
      const entries = Object.create(null);
      for (const row of container.querySelectorAll(".mapping-row")) {
        const trigger = row.querySelector(".trigger").value.trim();
        const action = row.querySelector(".action").value.trim();
        if (!trigger && !action) continue;
        if (!trigger || !action) throw new Error("Each remapping needs both a trigger and an action.");
        if (Object.prototype.hasOwnProperty.call(entries, trigger)) {
          throw new Error(`Duplicate remapping trigger: ${trigger}`);
        }
        entries[trigger] = action;
      }
      return entries;
    }

    function collectSettings(config) {
      const updated = JSON.parse(JSON.stringify(config));
      for (const { field, root } of fieldElements.values()) {
        let value;
        if (field.kind === "checkbox") {
          value = root.querySelector('input[type="checkbox"]').checked;
        } else if (field.kind === "color") {
          if (field.optional && !root._enabled.checked) {
            value = null;
          } else {
            const alpha = Number(root._alpha.value).toString(16).padStart(2, "0");
            value = `${root._color.value}${alpha}`;
          }
        } else if (field.optional && !root._enabled.checked) {
          value = null;
        } else if (field.kind === "number") {
          const text = root._input.value;
          value = text === "" && field.optional ? null : Number(text);
        } else if (field.kind === "select") {
          value = root.querySelector("select").value;
        } else {
          value = root._input.value.trim();
        }
        setPath(updated, field.path, value);
      }

      updated.remap.main = readMappings(document.querySelector("#main-mappings"));
      const layers = Object.create(null);
      for (const layer of document.querySelectorAll("#layers .layer")) {
        const name = layer.querySelector(".layer-name").value.trim();
        if (!/^[A-Za-z0-9_-]+$/.test(name)) throw new Error(`Invalid remapping layer name: ${name || "(empty)"}`);
        if (Object.prototype.hasOwnProperty.call(layers, name)) {
          throw new Error(`Duplicate remapping layer: ${name}`);
        }
        layers[name] = readMappings(layer.querySelector(".map-list"));
      }
      updated.remap.layers = layers;
      return updated;
    }

    async function loadConfig() {
      saveButton.disabled = true;
      status.textContent = "Loading settings…";
      try {
        const response = await fetch(endpoint, { cache: "no-store" });
        if (!response.ok) throw new Error(await response.text());
        currentConfig = await response.json();
        renderSettings(currentConfig);
        status.textContent = "All settings loaded.";
      } finally {
        saveButton.disabled = !currentConfig;
      }
    }
    document.querySelector("#reload").addEventListener("click", async () => {
      try { await loadConfig(); } catch (error) { status.textContent = error.message; }
    });
    document.querySelector("#add-main-mapping").addEventListener("click", () => {
      document.querySelector("#main-mappings").append(mappingRow());
    });
    document.querySelector("#add-layer").addEventListener("click", () => {
      const layers = document.querySelector("#layers");
      const layer = document.createElement("fieldset");
      layer.className = "layer";
      const heading = document.createElement("div");
      heading.className = "remap-heading";
      const name = document.createElement("input");
      name.className = "layer-name";
      name.placeholder = "Layer name";
      name.setAttribute("aria-label", "Layer name");
      const remove = document.createElement("button");
      remove.type = "button";
      remove.className = "secondary compact";
      remove.textContent = "Remove layer";
      remove.addEventListener("click", () => layer.remove());
      heading.append(name, remove);
      const list = document.createElement("div");
      list.className = "map-list";
      const add = document.createElement("button");
      add.type = "button";
      add.className = "secondary compact";
      add.textContent = "Add mapping";
      add.addEventListener("click", () => list.append(mappingRow()));
      layer.append(heading, list, add);
      layers.append(layer);
      name.focus();
    });
    saveButton.addEventListener("click", async () => {
      saveButton.disabled = true;
      status.textContent = "Validating and saving…";
      try {
        const updated = collectSettings(currentConfig);
        const response = await fetch(endpoint, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(updated)
        });
        if (!response.ok) throw new Error(await response.text());
        const result = await response.json();
        currentConfig = result;
        renderSettings(result);
        status.textContent = "Saved. Fievel is applying the new settings.";
      } catch (error) {
        status.textContent = error.message;
      } finally {
        saveButton.disabled = !currentConfig;
      }
    });
    loadConfig().catch(error => { status.textContent = error.message; });
  </script>
</body>
</html>"##;

#[cfg(test)]
mod tests {
    use super::*;

    fn send_request(
        config_path: &Path,
        method: &str,
        target: &str,
        headers: &[(&str, &str)],
        body: &str,
    ) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let config_path = config_path.to_path_buf();
        let token = "test-token";
        let (reload_sender, _reload_receiver) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            handle_connection(&mut stream, port, token, &config_path, &reload_sender).unwrap();
        });

        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            stream,
            "{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n"
        )
        .unwrap();
        for (name, value) in headers {
            let value = value.replace("{port}", &port.to_string());
            write!(stream, "{name}: {value}\r\n").unwrap();
        }
        write!(
            stream,
            "Content-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        stream.write_all(body.as_bytes()).unwrap();

        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        server.join().unwrap();
        response
    }

    #[test]
    fn saves_are_validated_and_write_a_complete_config() {
        let directory = std::env::temp_dir().join(format!(
            "fievel-config-screen-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("fievel.config");

        let mut config = Config::default();
        config.notify = false;
        config.keys.config_screen = vec![K::KEY_LEFTMETA, K::KEY_LEFTSHIFT, K::KEY_F9];
        let source = serde_json::to_vec(&EditorConfig::from(&config)).unwrap();
        let saved = save_config(&path, &source).unwrap();
        assert!(saved.contains("\"config_screen\":\"LEFTMETA + LEFTSHIFT + F9\""));
        let loaded = Config::load(&path, true).unwrap();
        assert!(!loaded.notify);
        assert_eq!(
            loaded.keys.config_screen,
            [
                crate::input::KeyCode::KEY_LEFTMETA,
                crate::input::KeyCode::KEY_LEFTSHIFT,
                crate::input::KeyCode::KEY_F9,
            ]
        );
        assert!(saved.contains("\"hints\""));

        fs::remove_file(&path).unwrap();
        fs::remove_dir(&directory).unwrap();
    }

    #[test]
    fn invalid_config_is_not_saved() {
        let directory = std::env::temp_dir().join(format!(
            "fievel-config-screen-invalid-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("fievel.config");

        let mut config = Config::default();
        config.speeds.normal = -1.0;
        let source = serde_json::to_vec(&EditorConfig::from(&config)).unwrap();
        assert!(save_config(&path, &source).is_err());
        assert!(!path.exists());

        fs::remove_dir(&directory).unwrap();
    }

    #[test]
    fn config_page_serves_all_settings_and_rejects_cross_origin_writes() {
        let directory = std::env::temp_dir().join(format!(
            "fievel-config-screen-http-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("fievel.config");
        let page_response = send_request(&path, "GET", "/?token=test-token", &[], "");
        assert!(page_response.starts_with("HTTP/1.1 200 OK"));
        assert!(page_response.contains("Main keybinds"));
        assert!(page_response.contains("hints.grid_colors.upper_left"));
        assert!(!page_response.contains("<textarea"));

        let get_response = send_request(&path, "GET", "/api/config?token=test-token", &[], "");
        assert!(get_response.starts_with("HTTP/1.1 200 OK"));
        assert!(get_response.contains("Content-Type: application/json"));
        let body = get_response.split_once("\r\n\r\n").unwrap().1;
        let editor: EditorConfig = serde_json::from_str(body).unwrap();
        assert_eq!(editor.keys.config_screen, "LEFTMETA + LEFTSHIFT + SLASH");
        assert_eq!(editor.hints.keys.debug, "F8");
        assert_eq!(editor.hints.grid_colors.upper_left, None);
        assert!(editor.remap.main.is_empty());

        let mut changed = Config::default();
        changed.notify = false;
        changed.keys.config_screen = vec![K::KEY_LEFTMETA, K::KEY_LEFTSHIFT, K::KEY_F9];
        let body = serde_json::to_string(&EditorConfig::from(&changed)).unwrap();
        let saved_response = send_request(
            &path,
            "POST",
            "/api/config?token=test-token",
            &[("Origin", "http://127.0.0.1:{port}")],
            &body,
        );
        assert!(saved_response.starts_with("HTTP/1.1 200 OK"));
        let persisted = Config::load(&path, true).unwrap();
        assert!(!persisted.notify);
        assert_eq!(persisted.keys.config_screen[2], K::KEY_F9);

        let post_response = send_request(
            &path,
            "POST",
            "/api/config?token=test-token",
            &[("Origin", "http://attacker.example")],
            "notify = false",
        );
        assert!(post_response.starts_with("HTTP/1.1 403 Forbidden"));

        fs::remove_dir_all(&directory).unwrap();
    }
}
