//! The Ubuntu menu-bar (system tray) icon: a live countdown plus quick actions.

use crate::db;
use crate::shared::{fmt_clock, fmt_hms, Shared};
use chrono::Local;
use ksni::menu::StandardItem;
use ksni::{MenuItem, ToolTip, Tray};
use std::sync::Arc;

pub struct WorkdayTray {
    pub shared: Arc<Shared>,
}

impl WorkdayTray {
    /// Read current state from the DB and describe it for the tray.
    /// Returns (icon_name, one_line_summary, detail_line).
    fn snapshot(&self) -> (&'static str, String, String) {
        let now = Local::now();
        let active = self.shared.with_db(|c| db::active_session(c)).ok().flatten();

        match active {
            None => (
                "appointment-soon-symbolic",
                "Not checked in".to_string(),
                "Use \u{201c}Check In Now\u{201d} to start your workday.".to_string(),
            ),
            Some(s) => {
                let remaining = s.remaining_secs(now);
                let out = fmt_clock(s.expected_check_out());
                if remaining >= 0 {
                    (
                        "alarm-symbolic",
                        format!("{} left", fmt_hms(remaining)),
                        format!("Checked in {} \u{2022} leave at {}", fmt_clock(s.check_in), out),
                    )
                } else {
                    (
                        "appointment-missed-symbolic",
                        format!("Overtime {}", fmt_hms(remaining.abs())),
                        format!("Workday ended at {} \u{2022} you can check out", out),
                    )
                }
            }
        }
    }

    fn is_checked_in(&self) -> bool {
        self.shared
            .with_db(|c| db::active_session(c))
            .ok()
            .flatten()
            .is_some()
    }
}

impl Tray for WorkdayTray {
    fn id(&self) -> String {
        "workday_timer".into()
    }

    fn title(&self) -> String {
        // Shown as the tray label / accessible name on hosts that display it.
        self.snapshot().1
    }

    fn icon_name(&self) -> String {
        self.snapshot().0.to_string()
    }

    fn tool_tip(&self) -> ToolTip {
        let (icon, summary, detail) = self.snapshot();
        ToolTip {
            icon_name: icon.to_string(),
            icon_pixmap: Vec::new(),
            title: format!("Workday \u{2014} {summary}"),
            description: detail,
        }
    }

    /// Left-click opens the reports window.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.shared.show_window();
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let (_, summary, detail) = self.snapshot();
        let checked_in = self.is_checked_in();

        vec![
            // Status header (non-clickable).
            StandardItem {
                label: summary,
                enabled: false,
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: detail,
                enabled: false,
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Check In Now".into(),
                icon_name: "media-playback-start-symbolic".into(),
                enabled: !checked_in,
                activate: Box::new(|t: &mut WorkdayTray| t.shared.check_in()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Check Out Now".into(),
                icon_name: "media-playback-stop-symbolic".into(),
                enabled: checked_in,
                activate: Box::new(|t: &mut WorkdayTray| t.shared.check_out()),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Open Reports\u{2026}".into(),
                icon_name: "document-properties-symbolic".into(),
                activate: Box::new(|t: &mut WorkdayTray| t.shared.show_window()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                icon_name: "application-exit-symbolic".into(),
                activate: Box::new(|t: &mut WorkdayTray| t.shared.request_quit()),
                ..Default::default()
            }
            .into(),
        ]
    }
}
