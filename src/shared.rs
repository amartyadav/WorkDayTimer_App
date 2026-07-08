//! State shared across threads: the DB connection, a handle to the GUI, and
//! the actions (check in / out / show / quit) that both the tray and the GUI
//! trigger. SQLite is the source of truth, so these are thin wrappers.

use crate::db;
use chrono::{DateTime, Local};
use rusqlite::Connection;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub struct Shared {
    conn: Arc<Mutex<Connection>>,
    /// Set once the GUI has started, so background threads can drive the window.
    ctx: Mutex<Option<egui::Context>>,
    /// True only when the user picked "Quit" — lets us tell a real quit apart
    /// from a window-close (which just hides to the tray).
    pub really_quit: AtomicBool,
}

impl Shared {
    pub fn new(conn: Connection) -> Arc<Self> {
        Arc::new(Shared {
            conn: Arc::new(Mutex::new(conn)),
            ctx: Mutex::new(None),
            really_quit: AtomicBool::new(false),
        })
    }

    /// Run a closure with the locked DB connection.
    pub fn with_db<R>(&self, f: impl FnOnce(&Connection) -> R) -> R {
        let guard = self.conn.lock().unwrap();
        f(&guard)
    }

    pub fn set_ctx(&self, ctx: egui::Context) {
        *self.ctx.lock().unwrap() = Some(ctx);
    }

    fn ctx(&self) -> Option<egui::Context> {
        self.ctx.lock().unwrap().clone()
    }

    /// Nudge the GUI to redraw (e.g. after a check in/out from the tray).
    pub fn wake_gui(&self) {
        if let Some(ctx) = self.ctx() {
            ctx.request_repaint();
        }
    }

    pub fn check_in(&self) {
        self.with_db(|c| db::check_in(c)).ok();
        self.wake_gui();
    }

    /// Check in as of an earlier time. Returns false if already checked in.
    pub fn check_in_at(&self, when: DateTime<Local>) -> bool {
        let ok = self.with_db(|c| db::check_in_at(c, when)).unwrap_or(false);
        self.wake_gui();
        ok
    }

    pub fn check_out(&self) {
        self.with_db(|c| db::check_out(c)).ok();
        self.wake_gui();
    }

    /// Check out as of an earlier time. Returns false if not checked in.
    pub fn check_out_at(&self, when: DateTime<Local>) -> bool {
        let ok = self.with_db(|c| db::check_out_at(c, when)).unwrap_or(false);
        self.wake_gui();
        ok
    }

    /// Discard the active session so you can check in again. Returns false if
    /// there was nothing to reset.
    pub fn reset_active(&self) -> bool {
        let ok = self.with_db(|c| db::reset_active(c)).unwrap_or(false);
        self.wake_gui();
        ok
    }

    /// Edit a past session's times.
    pub fn update_session(&self, id: i64, check_in: DateTime<Local>, check_out: Option<DateTime<Local>>) {
        self.with_db(|c| db::update_session(c, id, check_in, check_out)).ok();
        self.wake_gui();
    }

    /// Delete a session outright.
    pub fn delete_session(&self, id: i64) {
        self.with_db(|c| db::delete_session(c, id)).ok();
        self.wake_gui();
    }

    /// Bring the reports window back into view (used by the tray).
    /// We un-minimize rather than toggle visibility: hiding a window is a
    /// no-op on Wayland, so the window is minimized when "closed" instead.
    pub fn show_window(&self) {
        if let Some(ctx) = self.ctx() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.request_repaint();
        }
    }

    /// Fully exit the application (tray + GUI).
    pub fn request_quit(&self) {
        self.really_quit.store(true, Ordering::SeqCst);
        match self.ctx() {
            Some(ctx) => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            None => std::process::exit(0),
        }
    }
}

// ---- formatting helpers ---------------------------------------------------

/// Seconds -> "H:MM:SS", with a leading "-" when overtime (negative).
pub fn fmt_hms(secs: i64) -> String {
    let neg = secs < 0;
    let s = secs.abs();
    let out = format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60);
    if neg {
        format!("-{out}")
    } else {
        out
    }
}

/// Seconds -> "Xh Ym" for weekly totals.
pub fn fmt_hm(secs: i64) -> String {
    let s = secs.max(0);
    format!("{}h {:02}m", s / 3600, (s % 3600) / 60)
}

/// Describe an hour balance: positive seconds mean you owe time, negative
/// means you are ahead. Sub-minute amounts read as "even".
pub fn fmt_balance(secs: i64) -> String {
    if secs >= 60 {
        format!("owe {}", fmt_hm(secs))
    } else if secs <= -60 {
        format!("ahead {}", fmt_hm(-secs))
    } else {
        "even".to_string()
    }
}

/// A clock time like "17:30".
pub fn fmt_clock(dt: DateTime<Local>) -> String {
    dt.format("%H:%M").to_string()
}

/// A full timestamp like "Wed 08 Jul, 17:30".
pub fn fmt_stamp(dt: DateTime<Local>) -> String {
    dt.format("%a %d %b, %H:%M").to_string()
}
