#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs;
use std::io::Write;
use std::panic;
use std::path::PathBuf;
use std::time::SystemTime;

/// Deterministic crash-log path:
///   1. $HOME/Desktop/crasp_crash_log.txt   (via dirs crate)
///   2. <exe_dir>/crasp_crash_log.txt       (fallback)
///   3. ./crasp_crash_log.txt               (last resort)
fn crash_log_path() -> PathBuf {
    dirs::desktop_dir()
        .map(|d| d.join("crasp_crash_log.txt"))
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("crasp_crash_log.txt")))
        })
        .unwrap_or_else(|| PathBuf::from("crasp_crash_log.txt"))
}

/// Install a global panic hook **before** the Tauri runtime starts.
/// Every unhandled Rust panic (including from tokio::spawn tasks,
/// .expect() failures, and missing Tauri state) will be appended to
/// the crash log file AND printed to stderr (visible if run from
/// a terminal even in release builds with windows_subsystem=windows).
fn init_crash_logger() {
    // Ensure RUST_BACKTRACE is enabled so std::backtrace::Backtrace::capture()
    // returns a full trace instead of "disabled backtrace".
    // Only force it if the user hasn't set it themselves.
    if std::env::var("RUST_BACKTRACE").is_err() {
        std::env::set_var("RUST_BACKTRACE", "1");
    }

    let log_path = crash_log_path();

    panic::set_hook(Box::new(move |panic_info| {
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let backtrace = std::backtrace::Backtrace::capture();

        let separator = "=".repeat(60);
        let payload = format!(
            "[Crasp CRASH — unix_ts={timestamp}]\n\
             Panic: {panic_info}\n\n\
             Backtrace:\n{backtrace}\n\n\
             {separator}\n\n"
        );

        // Always write to stderr (visible if user runs from terminal)
        eprintln!("{payload}");

        // Persist to the crash log file
        if let Ok(mut f) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            let _ = f.write_all(payload.as_bytes());
        }
    }));
}

fn main() {
    // MUST run before anything else — before Tauri, before tokio,
    // before any .expect() that could panic and get swallowed by
    // windows_subsystem = "windows" in release builds.
    init_crash_logger();

    crasp_lib::run()
}
