mod config;
mod config_screen;
mod detect;
mod engine;
#[cfg_attr(target_os = "macos", allow(dead_code))]
mod font;
mod grid;
mod hint_input;
mod input;
mod keycast;
mod label;
mod notifications;
mod pipeline;
mod reload;
mod remap;

#[cfg(target_os = "linux")]
#[path = "hints.rs"]
mod hints;
#[cfg(target_os = "linux")]
#[path = "indicator.rs"]
mod indicator;
#[cfg(target_os = "linux")]
mod indicator_render;
#[cfg(target_os = "linux")]
mod locator;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;
#[cfg(all(target_os = "linux", test))]
pub(crate) use platform::handle_hint_input;

#[cfg(target_os = "macos")]
#[path = "macos_hints.rs"]
mod hints;
#[cfg(target_os = "macos")]
#[path = "macos_indicator.rs"]
mod indicator;
#[cfg(target_os = "macos")]
#[path = "odometer.rs"]
mod odometer;
#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod platform;

#[cfg(target_os = "windows")]
#[path = "windows_hints.rs"]
mod hints;
#[cfg(target_os = "windows")]
#[path = "odometer_windows.rs"]
mod odometer;
#[cfg(target_os = "windows")]
#[path = "windows.rs"]
mod platform;
#[cfg(target_os = "windows")]
mod windows_ui;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("fievel supports Linux, macOS, and Windows");

fn main() {
    platform::run();
}
