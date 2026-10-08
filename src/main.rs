mod config;
mod detect;
mod engine;
#[cfg_attr(target_os = "macos", allow(dead_code))]
mod font;
mod hint_input;
mod keycast;
mod label;
mod pipeline;
mod remap;

#[cfg(target_os = "linux")]
mod locator;
#[cfg(target_os = "linux")]
#[path = "hints.rs"]
mod hints;
#[cfg(target_os = "linux")]
#[path = "indicator.rs"]
mod indicator;
#[cfg(target_os = "linux")]
mod indicator_render;
#[cfg(target_os = "linux")]
#[path = "linux.rs"]
mod platform;

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

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("fievel supports Linux and macOS");

fn main() {
    platform::run();
}
