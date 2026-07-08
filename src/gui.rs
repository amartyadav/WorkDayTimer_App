//! The reports window (eframe/egui): live status, check in/out buttons, a log
//! of every session, and weekly hour totals.

use crate::db;
use crate::shared::{fmt_clock, fmt_hm, fmt_hms, fmt_stamp, Shared};
use chrono::{DateTime, Local, NaiveDateTime, NaiveTime, TimeZone};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

/// The format used for editing a full timestamp in the history editor.
const EDIT_FMT: &str = "%Y-%m-%d %H:%M";

/// In-progress edit of one past session.
struct EditState {
    id: i64,
    check_in: String,
    check_out: String,
    confirm_delete: bool,
    msg: Option<(String, egui::Color32)>,
}

impl EditState {
    fn from_session(s: &db::Session) -> Self {
        EditState {
            id: s.id,
            check_in: s.check_in.format(EDIT_FMT).to_string(),
            check_out: s
                .check_out
                .map(|c| c.format(EDIT_FMT).to_string())
                .unwrap_or_default(),
            confirm_delete: false,
            msg: None,
        }
    }
}

pub struct ReportsApp {
    shared: Arc<Shared>,
    ctx_registered: bool,
    /// "HH:MM" buffer for checking in at an earlier time.
    checkin_input: String,
    /// Feedback shown under the past-time check-in row.
    checkin_msg: Option<(String, egui::Color32)>,
    /// "HH:MM" buffer for checking out at an earlier time.
    checkout_input: String,
    /// Feedback shown under the past-time check-out row.
    checkout_msg: Option<(String, egui::Color32)>,
    /// Two-step guard for the destructive "reset check-in" action.
    confirm_reset: bool,
    /// The session currently being edited in the history editor, if any.
    editing: Option<EditState>,
}

const OK_GREEN: egui::Color32 = egui::Color32::from_rgb(60, 170, 90);
const ERR_RED: egui::Color32 = egui::Color32::from_rgb(200, 90, 70);

impl ReportsApp {
    pub fn new(shared: Arc<Shared>) -> Self {
        let now = Local::now().format("%H:%M").to_string();
        Self {
            shared,
            ctx_registered: false,
            checkin_input: now.clone(),
            checkin_msg: None,
            checkout_input: now,
            checkout_msg: None,
            confirm_reset: false,
            editing: None,
        }
    }

    /// Parse an "HH:MM" buffer into a moment earlier today, if valid.
    fn parse_today(text: &str) -> Result<chrono::DateTime<Local>, String> {
        let time = NaiveTime::parse_from_str(text.trim(), "%H:%M")
            .or_else(|_| NaiveTime::parse_from_str(text.trim(), "%H.%M"))
            .map_err(|_| "Enter a time as HH:MM (24-hour), e.g. 09:15.".to_string())?;
        let today = Local::now().date_naive();
        let when = Local
            .from_local_datetime(&today.and_time(time))
            .single()
            .ok_or_else(|| "That time is ambiguous today.".to_string())?;
        if when > Local::now() + chrono::Duration::seconds(60) {
            return Err("That time is in the future — pick an earlier time.".to_string());
        }
        Ok(when)
    }

    /// Parse a full "YYYY-MM-DD HH:MM" timestamp.
    fn parse_dt(text: &str) -> Result<DateTime<Local>, String> {
        let ndt = NaiveDateTime::parse_from_str(text.trim(), EDIT_FMT)
            .map_err(|_| "Use the format YYYY-MM-DD HH:MM.".to_string())?;
        Local
            .from_local_datetime(&ndt)
            .single()
            .ok_or_else(|| "That local time is ambiguous.".to_string())
    }

    /// Validate the edit form and, if it checks out, write it to the DB.
    fn apply_edit(shared: &Shared, edit: &EditState) -> Result<(), String> {
        let check_in = Self::parse_dt(&edit.check_in)?;
        let check_out = if edit.check_out.trim().is_empty() {
            None
        } else {
            Some(Self::parse_dt(&edit.check_out)?)
        };
        match check_out {
            Some(co) if co < check_in => {
                return Err("Check-out can't be before check-in.".into());
            }
            None => {
                // Reopening a session must not create a second active one.
                if let Some(active) = shared.with_db(|c| db::active_session(c)).ok().flatten() {
                    if active.id != edit.id {
                        return Err("Another session is already in progress.".into());
                    }
                }
            }
            _ => {}
        }
        shared.update_session(edit.id, check_in, check_out);
        Ok(())
    }

    /// Check in as of the time typed in the check-in buffer.
    fn try_check_in_at(&mut self) {
        match Self::parse_today(&self.checkin_input) {
            Err(e) => self.checkin_msg = Some((e, ERR_RED)),
            Ok(when) => {
                self.checkin_msg = if self.shared.check_in_at(when) {
                    Some((format!("Checked in as of {}.", fmt_clock(when)), OK_GREEN))
                } else {
                    Some(("Already checked in.".into(), ERR_RED))
                };
            }
        }
    }

    /// Check out as of the time typed in the check-out buffer.
    fn try_check_out_at(&mut self, check_in: chrono::DateTime<Local>) {
        match Self::parse_today(&self.checkout_input) {
            Err(e) => self.checkout_msg = Some((e, ERR_RED)),
            Ok(when) if when < check_in => {
                self.checkout_msg =
                    Some(("Check-out can't be before check-in.".into(), ERR_RED));
            }
            Ok(when) => {
                self.checkout_msg = if self.shared.check_out_at(when) {
                    Some((format!("Checked out as of {}.", fmt_clock(when)), OK_GREEN))
                } else {
                    Some(("Not checked in.".into(), ERR_RED))
                };
            }
        }
    }
}

impl eframe::App for ReportsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Let background threads (the tray) drive this window.
        if !self.ctx_registered {
            self.shared.set_ctx(ctx.clone());
            self.ctx_registered = true;
        }

        // Closing the window minimizes it to keep the tray timer running,
        // unless the user chose "Quit" from the tray. (Hiding a window is a
        // no-op on Wayland, so we minimize rather than set it invisible.)
        if ctx.input(|i| i.viewport().close_requested())
            && !self.shared.really_quit.load(Ordering::SeqCst)
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
        }

        // Keep the live countdown ticking.
        ctx.request_repaint_after(Duration::from_secs(1));

        let now = Local::now();
        let active = self.shared.with_db(|c| db::active_session(c)).ok().flatten();
        let sessions = self.shared.with_db(|c| db::all_sessions(c)).unwrap_or_default();
        let weeks = self
            .shared
            .with_db(|c| db::weekly_totals(c, now))
            .unwrap_or_default();

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.add_space(4.0);
                ui.heading("Workday Timer");
                ui.separator();

                // ---- current status ---------------------------------------
                match &active {
                    Some(s) => {
                        let remaining = s.remaining_secs(now);
                        let (label, color) = if remaining >= 0 {
                            (format!("{} left", fmt_hms(remaining)), egui::Color32::from_rgb(60, 170, 90))
                        } else {
                            (
                                format!("Overtime {}", fmt_hms(remaining.abs())),
                                egui::Color32::from_rgb(200, 90, 70),
                            )
                        };
                        ui.add_space(4.0);
                        ui.label(egui::RichText::new(label).size(34.0).strong().color(color));
                        ui.add_space(6.0);
                        egui::Grid::new("status_grid").spacing([16.0, 4.0]).show(ui, |ui| {
                            ui.label("Checked in:");
                            ui.strong(fmt_stamp(s.check_in));
                            ui.end_row();
                            ui.label("Leave at:");
                            ui.strong(fmt_clock(s.expected_check_out()));
                            ui.end_row();
                            ui.label("Worked so far:");
                            ui.strong(fmt_hms(s.worked_secs(now)));
                            ui.end_row();
                        });
                        ui.add_space(8.0);
                        if ui
                            .add(egui::Button::new(
                                egui::RichText::new("\u{23F9}  Check Out Now").size(16.0),
                            ))
                            .clicked()
                        {
                            self.shared.check_out();
                            self.checkout_msg = None;
                            self.confirm_reset = false;
                        }

                        ui.add_space(10.0);
                        ui.label("Left the office earlier? Set the time you checked out today:");
                        ui.add_space(2.0);
                        let mut submit_out = false;
                        ui.horizontal(|ui| {
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut self.checkout_input)
                                    .desired_width(70.0)
                                    .hint_text("HH:MM"),
                            );
                            submit_out = resp.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if ui.button("Check out at this time").clicked() {
                                submit_out = true;
                            }
                        });
                        if submit_out {
                            self.try_check_out_at(s.check_in);
                        }
                        if let Some((msg, color)) = &self.checkout_msg {
                            ui.add_space(2.0);
                            ui.colored_label(*color, msg);
                        }

                        // Discard this session to redo the check-in.
                        ui.add_space(10.0);
                        if self.confirm_reset {
                            ui.horizontal(|ui| {
                                ui.colored_label(ERR_RED, "Delete this check-in and start over?");
                                if ui.button("Yes, reset").clicked() {
                                    self.shared.reset_active();
                                    self.confirm_reset = false;
                                    self.checkin_input = Local::now().format("%H:%M").to_string();
                                    self.checkin_msg = None;
                                    self.checkout_msg = None;
                                }
                                if ui.button("Cancel").clicked() {
                                    self.confirm_reset = false;
                                }
                            });
                        } else if ui
                            .button(egui::RichText::new("\u{21BA}  Reset today's check-in").size(13.0))
                            .on_hover_text(
                                "Discards the current check-in and progress so you can \
                                 check in again with a corrected time.",
                            )
                            .clicked()
                        {
                            self.confirm_reset = true;
                        }
                    }
                    None => {
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new("Not checked in")
                                .size(28.0)
                                .color(egui::Color32::GRAY),
                        );
                        ui.add_space(8.0);
                        if ui
                            .add(egui::Button::new(
                                egui::RichText::new("\u{25B6}  Check In Now").size(16.0),
                            ))
                            .clicked()
                        {
                            self.checkin_input = Local::now().format("%H:%M").to_string();
                            self.checkin_msg = None;
                            self.shared.check_in();
                        }

                        ui.add_space(10.0);
                        ui.label("Forgot to check in earlier? Set the time you started today:");
                        ui.add_space(2.0);
                        let mut submit = false;
                        ui.horizontal(|ui| {
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut self.checkin_input)
                                    .desired_width(70.0)
                                    .hint_text("HH:MM"),
                            );
                            submit = resp.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            if ui.button("Check in at this time").clicked() {
                                submit = true;
                            }
                        });
                        if submit {
                            self.try_check_in_at();
                        }
                        if let Some((msg, color)) = &self.checkin_msg {
                            ui.add_space(2.0);
                            ui.colored_label(*color, msg);
                        }
                    }
                }

                ui.add_space(14.0);

                // ---- weekly totals ----------------------------------------
                ui.heading("Weekly totals");
                ui.separator();
                if weeks.is_empty() {
                    ui.label("No sessions yet.");
                } else {
                    egui::Grid::new("weeks_grid")
                        .striped(true)
                        .spacing([24.0, 4.0])
                        .show(ui, |ui| {
                            ui.strong("Week starting");
                            ui.strong("Sessions");
                            ui.strong("Total hours");
                            ui.end_row();
                            for w in &weeks {
                                ui.label(w.week_start.format("%a %d %b %Y").to_string());
                                ui.label(w.session_count.to_string());
                                ui.strong(fmt_hm(w.total_secs));
                                ui.end_row();
                            }
                        });
                }

                ui.add_space(14.0);

                // ---- session history --------------------------------------
                ui.heading("Session history");
                ui.separator();

                // Editor for a selected past session (shown above the table).
                let shared = self.shared.clone();
                let mut close_edit = false;
                if let Some(edit) = &mut self.editing {
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.label(egui::RichText::new(format!("Editing session #{}", edit.id)).strong());
                        ui.add_space(2.0);
                        egui::Grid::new("edit_grid").spacing([10.0, 6.0]).show(ui, |ui| {
                            ui.label("Check in:");
                            ui.add(
                                egui::TextEdit::singleline(&mut edit.check_in)
                                    .desired_width(160.0)
                                    .hint_text(EDIT_FMT),
                            );
                            ui.end_row();
                            ui.label("Check out:");
                            ui.add(
                                egui::TextEdit::singleline(&mut edit.check_out)
                                    .desired_width(160.0)
                                    .hint_text("blank = in progress"),
                            );
                            ui.end_row();
                        });
                        ui.label(
                            egui::RichText::new("Format: YYYY-MM-DD HH:MM (24-hour).")
                                .small()
                                .color(egui::Color32::GRAY),
                        );
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui.button("Save").clicked() {
                                match Self::apply_edit(shared.as_ref(), edit) {
                                    Ok(()) => close_edit = true,
                                    Err(e) => edit.msg = Some((e, ERR_RED)),
                                }
                            }
                            if edit.confirm_delete {
                                if ui
                                    .button(egui::RichText::new("Confirm delete").color(ERR_RED))
                                    .clicked()
                                {
                                    shared.delete_session(edit.id);
                                    close_edit = true;
                                }
                                if ui.button("Keep").clicked() {
                                    edit.confirm_delete = false;
                                }
                            } else if ui.button("Delete").clicked() {
                                edit.confirm_delete = true;
                            }
                            if ui.button("Cancel").clicked() {
                                close_edit = true;
                            }
                        });
                        if let Some((m, c)) = &edit.msg {
                            ui.add_space(2.0);
                            ui.colored_label(*c, m.clone());
                        }
                    });
                    ui.add_space(6.0);
                }
                if close_edit {
                    self.editing = None;
                }

                if sessions.is_empty() {
                    ui.label("No sessions yet.");
                } else {
                    egui::Grid::new("sessions_grid")
                        .striped(true)
                        .spacing([18.0, 4.0])
                        .show(ui, |ui| {
                            ui.strong("Check in");
                            ui.strong("Check out");
                            ui.strong("Expected out");
                            ui.strong("Hours");
                            ui.strong("");
                            ui.end_row();
                            for s in &sessions {
                                ui.label(fmt_stamp(s.check_in));
                                match s.check_out {
                                    Some(co) => {
                                        ui.label(fmt_stamp(co));
                                    }
                                    None => {
                                        ui.colored_label(OK_GREEN, "in progress");
                                    }
                                }
                                ui.label(fmt_clock(s.expected_check_out()));
                                ui.strong(fmt_hm(s.worked_secs(now)));
                                if ui.small_button("Edit").clicked() {
                                    self.editing = Some(EditState::from_session(s));
                                }
                                ui.end_row();
                            }
                        });
                }

                ui.add_space(10.0);
                ui.separator();
                ui.label(
                    egui::RichText::new(
                        "Tip: closing this window minimizes it and the tray timer keeps \
                         running. Reopen it from the menu-bar icon; use the tray's Quit to exit.",
                    )
                    .small()
                    .color(egui::Color32::GRAY),
                );
            });
        });
    }
}
