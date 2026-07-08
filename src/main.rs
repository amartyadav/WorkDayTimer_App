//! Workday Timer — a menu-bar countdown of your remaining 8-hour workday,
//! with a reports window, all backed by SQLite.
//!
//! Threads:
//!   * main             — the eframe/egui reports window (required on Linux)
//!   * ksni DBus thread — the system-tray icon (spawned by `ksni`)
//!   * ticker thread    — refreshes the tray countdown once per second

mod db;
mod gui;
mod shared;
mod tray;

use ksni::blocking::TrayMethods;
use shared::Shared;
use std::thread;
use std::time::Duration;

fn main() -> eframe::Result<()> {
    let conn = db::open().expect("open workday database");
    let shared = Shared::new(conn);

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
            .with_inner_size([520.0, 660.0])
            .with_min_inner_size([420.0, 420.0]),
        ..Default::default()
    };
    let gui_shared = shared.clone();
    eframe::run_native(
        "Workday Timer",
        options,
        Box::new(move |_cc| Ok(Box::new(gui::ReportsApp::new(gui_shared)))),
    )
}
