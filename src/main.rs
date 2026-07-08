//! Workday Timer — a menu-bar countdown of your remaining 8-hour workday,
//! with a reports window, all backed by SQLite.
//!
//! Threads:
//!   * main             — the eframe/egui reports window (required on Linux)
//!   * ksni DBus thread — the system-tray icon (spawned by `ksni`)
//!   * ticker thread    — refreshes the tray countdown once per second
//!   * instance thread  — listens for later launches and raises the window

mod db;
mod gui;
mod shared;
mod tray;

use ksni::blocking::TrayMethods;
use shared::Shared;
use std::io::Write;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

/// Path of the single-instance control socket.
fn sock_path() -> PathBuf {
    let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(dir).join("workday_timer.sock")
}

enum Instance {
    /// We are the first instance; hold the listener.
    Primary(UnixListener),
    /// Another instance is already running (we asked it to show its window).
    Secondary,
    /// Could not set up the socket; run without the guard.
    Unguarded,
}

/// Make sure only one instance runs. A later launch connects to the running
/// one, which treats the connection as "please show your window".
fn acquire_instance() -> Instance {
    let path = sock_path();
    // If something is listening, we're a duplicate launch: signal and bow out.
    if let Ok(mut stream) = UnixStream::connect(&path) {
        let _ = stream.write_all(b"show");
        return Instance::Secondary;
    }
    // Otherwise the socket is missing or stale; recreate it.
    let _ = std::fs::remove_file(&path);
    match UnixListener::bind(&path) {
        Ok(listener) => Instance::Primary(listener),
        Err(e) => {
            eprintln!("Single-instance guard unavailable ({e}); continuing anyway.");
            Instance::Unguarded
        }
    }
}

fn main() -> eframe::Result<()> {
    let listener = match acquire_instance() {
        Instance::Secondary => return Ok(()), // already running; just focus it
        Instance::Primary(l) => Some(l),
        Instance::Unguarded => None,
    };

    let conn = db::open().expect("open workday database");
    let shared = Shared::new(conn);

    // Raise the window whenever another launch pings our socket.
    if let Some(listener) = listener {
        let s = shared.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                if stream.is_ok() {
                    s.show_window();
                }
            }
        });
    }

    // System-tray icon runs on its own DBus thread, managed by ksni.
    let tray = tray::WorkdayTray {
        shared: shared.clone(),
    };
    match tray.spawn() {
        Ok(handle) => {
            // Ticker thread: nudge the tray once a second so the countdown
            // in the tooltip/label stays live.
            thread::spawn(move || loop {
                thread::sleep(Duration::from_secs(1));
                if handle.update(|_| {}).is_none() {
                    break; // tray has been shut down
                }
            });
        }
        Err(e) => {
            eprintln!("Could not start the menu-bar icon: {e}\nContinuing with the window only.");
        }
    }

    // Reports window on the main thread.
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Workday Timer")
            .with_app_id("workday_timer")
            .with_inner_size([520.0, 660.0])
            .with_min_inner_size([420.0, 420.0]),
        ..Default::default()
    };
    let gui_shared = shared.clone();
    let result = eframe::run_native(
        "Workday Timer",
        options,
        Box::new(move |_cc| Ok(Box::new(gui::ReportsApp::new(gui_shared)))),
    );

    // Best-effort cleanup so the next launch rebinds cleanly.
    let _ = std::fs::remove_file(sock_path());
    result
}
