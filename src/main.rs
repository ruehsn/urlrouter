//! URL Router: registers as the system's web browser on Windows or macOS,
//! and when a link is opened hands it to the real browser (or the Slack app)
//! picked by the rules in urlrouter.txt.
//!
//! On Windows the GUI subsystem means no console window flashes on every
//! click; feedback goes through message boxes.
#![cfg_attr(windows, windows_subsystem = "windows")]
// On other platforms only the tests use the rule engine.
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
mod rules;
#[cfg_attr(not(any(windows, target_os = "macos")), allow(dead_code))]
mod slack;

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
mod mac_app;
#[cfg(windows)]
mod win;
#[cfg(windows)]
mod win_app;

#[cfg_attr(not(windows), allow(dead_code))]
pub const DEFAULT_CONFIG: &str = include_str!("default_config.txt");
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub const DEFAULT_CONFIG_MAC: &str = include_str!("default_config_mac.txt");

#[cfg(windows)]
fn main() {
    std::process::exit(win_app::run());
}

#[cfg(target_os = "macos")]
fn main() {
    std::process::exit(mac_app::run());
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    eprintln!("urlrouter runs on Windows and macOS; see the README for how to build it.");
    std::process::exit(1);
}
