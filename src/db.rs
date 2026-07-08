//! SQLite storage — the single source of truth shared by the tray and the GUI.

use chrono::{DateTime, Datelike, Duration, Local, NaiveDate};
use rusqlite::Connection;
use std::path::PathBuf;

/// A standard workday, in seconds (8 hours).
pub const WORKDAY_SECS: i64 = 8 * 60 * 60;

/// One work session: a check-in and (once you leave) a check-out.
#[derive(Clone, Debug)]
pub struct Session {
    pub id: i64,
    pub check_in: DateTime<Local>,
    pub check_out: Option<DateTime<Local>>,
    pub workday_secs: i64,
}

impl Session {
    /// When you are expected to leave (check_in + a full workday).
    pub fn expected_check_out(&self) -> DateTime<Local> {
        self.check_in + Duration::seconds(self.workday_secs)
    }

    /// Seconds actually worked so far (uses `now` while still active).
    pub fn worked_secs(&self, now: DateTime<Local>) -> i64 {
        let end = self.check_out.unwrap_or(now);
        (end - self.check_in).num_seconds().max(0)
    }

    /// Seconds left in the workday (negative once you have gone overtime).
    pub fn remaining_secs(&self, now: DateTime<Local>) -> i64 {
        self.workday_secs - self.worked_secs(now)
    }
}

/// Location of the database file: ~/.local/share/workday_timer/workday.db
pub fn db_path() -> PathBuf {
    let mut dir = dirs_data_home();
    dir.push("workday_timer");
    let _ = std::fs::create_dir_all(&dir);
    dir.push("workday.db");
    dir
}

fn dirs_data_home() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let mut p = PathBuf::from(home);
    p.push(".local");
    p.push("share");
    p
}

/// Create the schema if it does not exist yet.
pub fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            check_in      TEXT    NOT NULL,
            check_out     TEXT,
            workday_secs  INTEGER NOT NULL DEFAULT 28800
        );",
    )
}

/// Open the database and make sure the schema exists.
pub fn open() -> rusqlite::Result<Connection> {
    let conn = Connection::open(db_path())?;
    init_schema(&conn)?;
    Ok(conn)
}

fn parse(ts: &str) -> DateTime<Local> {
    DateTime::parse_from_rfc3339(ts)
        .map(|dt| dt.with_timezone(&Local))
        .unwrap_or_else(|_| Local::now())
}

fn row_to_session(row: &rusqlite::Row) -> rusqlite::Result<Session> {
    let check_in: String = row.get(1)?;
    let check_out: Option<String> = row.get(2)?;
    Ok(Session {
        id: row.get(0)?,
        check_in: parse(&check_in),
        check_out: check_out.map(|s| parse(&s)),
        workday_secs: row.get(3)?,
    })
}

/// The currently open session, if you are checked in.
pub fn active_session(conn: &Connection) -> rusqlite::Result<Option<Session>> {
    conn.query_row(
        "SELECT id, check_in, check_out, workday_secs
           FROM sessions WHERE check_out IS NULL
           ORDER BY id DESC LIMIT 1",
        [],
        row_to_session,
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        other => Err(other),
    })
}

/// Check in now. Returns false (and does nothing) if already checked in.
pub fn check_in(conn: &Connection) -> rusqlite::Result<bool> {
    check_in_at(conn, Local::now())
}

/// Check in as of a specific time (e.g. you started earlier and forgot).
/// Returns false (and does nothing) if already checked in.
pub fn check_in_at(conn: &Connection, when: DateTime<Local>) -> rusqlite::Result<bool> {
    if active_session(conn)?.is_some() {
        return Ok(false);
    }
    conn.execute(
        "INSERT INTO sessions (check_in, workday_secs) VALUES (?1, ?2)",
        (when.to_rfc3339(), WORKDAY_SECS),
    )?;
    Ok(true)
}

/// Check out of the active session now. Returns false if not checked in.
pub fn check_out(conn: &Connection) -> rusqlite::Result<bool> {
    check_out_at(conn, Local::now())
}

/// Check out of the active session as of a specific time (e.g. you left the
/// office earlier and forgot to click). Returns false if not checked in.
pub fn check_out_at(conn: &Connection, when: DateTime<Local>) -> rusqlite::Result<bool> {
    let Some(active) = active_session(conn)? else {
        return Ok(false);
    };
    conn.execute(
        "UPDATE sessions SET check_out = ?1 WHERE id = ?2",
        (when.to_rfc3339(), active.id),
    )?;
    Ok(true)
}

/// Discard the active session entirely, so you can check in again with a
/// corrected time. Returns false if there was nothing to reset.
pub fn reset_active(conn: &Connection) -> rusqlite::Result<bool> {
    let Some(active) = active_session(conn)? else {
        return Ok(false);
    };
    conn.execute("DELETE FROM sessions WHERE id = ?1", (active.id,))?;
    Ok(true)
}

/// Edit a past session's check-in and check-out (None = still in progress).
pub fn update_session(
    conn: &Connection,
    id: i64,
    check_in: DateTime<Local>,
    check_out: Option<DateTime<Local>>,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE sessions SET check_in = ?1, check_out = ?2 WHERE id = ?3",
        (
            check_in.to_rfc3339(),
            check_out.map(|c| c.to_rfc3339()),
            id,
        ),
    )?;
    Ok(())
}

/// Delete a session outright.
pub fn delete_session(conn: &Connection, id: i64) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM sessions WHERE id = ?1", (id,))?;
    Ok(())
}

/// All sessions, newest first.
pub fn all_sessions(conn: &Connection) -> rusqlite::Result<Vec<Session>> {
    let mut stmt = conn.prepare(
        "SELECT id, check_in, check_out, workday_secs FROM sessions ORDER BY id DESC",
    )?;
    let rows = stmt.query_map([], row_to_session)?;
    rows.collect()
}

/// A week's worth of totals for the report view.
#[derive(Clone, Debug)]
pub struct WeekTotal {
    /// Monday of the ISO week.
    pub week_start: NaiveDate,
    pub total_secs: i64,
    pub session_count: usize,
}

/// Sum completed + in-progress work per ISO week, newest week first.
pub fn weekly_totals(conn: &Connection, now: DateTime<Local>) -> rusqlite::Result<Vec<WeekTotal>> {
    let sessions = all_sessions(conn)?;
    // Preserve newest-first ordering of weeks via an index map.
    let mut order: Vec<NaiveDate> = Vec::new();
    let mut totals: std::collections::HashMap<NaiveDate, (i64, usize)> = std::collections::HashMap::new();
    for s in &sessions {
        let d = s.check_in.date_naive();
        // Monday as the start of the ISO week.
        let week_start = d - Duration::days(d.weekday().num_days_from_monday() as i64);
        let entry = totals.entry(week_start).or_insert_with(|| {
            order.push(week_start);
            (0, 0)
        });
        entry.0 += s.worked_secs(now);
        entry.1 += 1;
    }
    Ok(order
        .into_iter()
        .map(|week_start| {
            let (total_secs, session_count) = totals[&week_start];
            WeekTotal {
                week_start,
                total_secs,
                session_count,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn mem() -> Connection {
        let c = Connection::open_in_memory().unwrap();
        init_schema(&c).unwrap();
        c
    }

    #[test]
    fn check_in_is_idempotent_while_active() {
        let c = mem();
        assert!(check_in(&c).unwrap()); // first one starts a session
        assert!(!check_in(&c).unwrap()); // second is ignored
        assert!(active_session(&c).unwrap().is_some());
        assert_eq!(all_sessions(&c).unwrap().len(), 1);
    }

    #[test]
    fn check_out_closes_the_session() {
        let c = mem();
        assert!(!check_out(&c).unwrap()); // nothing to close
        check_in(&c).unwrap();
        assert!(check_out(&c).unwrap());
        assert!(active_session(&c).unwrap().is_none());
        // After checkout a new check-in opens a fresh session.
        assert!(check_in(&c).unwrap());
        assert_eq!(all_sessions(&c).unwrap().len(), 2);
    }

    #[test]
    fn check_in_at_past_time_counts_from_then() {
        let c = mem();
        let two_hours_ago = Local::now() - Duration::hours(2);
        assert!(check_in_at(&c, two_hours_ago).unwrap());
        let s = active_session(&c).unwrap().unwrap();
        // ~2h already worked, ~6h remaining.
        assert!((s.worked_secs(Local::now()) - 2 * 3600).abs() < 5);
        assert!((s.remaining_secs(Local::now()) - 6 * 3600).abs() < 5);
        // A second check-in (even with a time) is ignored while active.
        assert!(!check_in_at(&c, Local::now()).unwrap());
    }

    #[test]
    fn check_out_at_sets_earlier_time() {
        let c = mem();
        let ci = Local::now() - Duration::hours(9);
        check_in_at(&c, ci).unwrap();
        let co = ci + Duration::hours(8); // left after a full day, an hour ago
        assert!(check_out_at(&c, co).unwrap());
        let s = &all_sessions(&c).unwrap()[0];
        assert!(s.check_out.is_some());
        assert_eq!(s.worked_secs(Local::now()), 8 * 3600);
        assert!(active_session(&c).unwrap().is_none());
    }

    #[test]
    fn reset_active_discards_current_session() {
        let c = mem();
        check_in(&c).unwrap();
        assert!(reset_active(&c).unwrap());
        assert!(active_session(&c).unwrap().is_none());
        assert_eq!(all_sessions(&c).unwrap().len(), 0);
        // A completed session is not touched by reset.
        check_in(&c).unwrap();
        check_out(&c).unwrap();
        assert!(!reset_active(&c).unwrap());
        assert_eq!(all_sessions(&c).unwrap().len(), 1);
    }

    #[test]
    fn update_and_delete_past_sessions() {
        let c = mem();
        let ci = Local::now() - Duration::days(2);
        check_in_at(&c, ci).unwrap();
        check_out_at(&c, ci + Duration::hours(7)).unwrap();
        let id = all_sessions(&c).unwrap()[0].id;

        // Edit both times.
        let new_ci = Local::now() - Duration::days(2) - Duration::hours(1);
        update_session(&c, id, new_ci, Some(new_ci + Duration::hours(8))).unwrap();
        let s = &all_sessions(&c).unwrap()[0];
        assert_eq!(s.worked_secs(Local::now()), 8 * 3600);
        assert_eq!(s.check_in.timestamp(), new_ci.timestamp());

        // Delete it.
        delete_session(&c, id).unwrap();
        assert_eq!(all_sessions(&c).unwrap().len(), 0);
    }

    #[test]
    fn remaining_and_expected_checkout() {
        let check_in = Local::now() - Duration::hours(3);
        let s = Session {
            id: 1,
            check_in,
            check_out: None,
            workday_secs: WORKDAY_SECS,
        };
        let now = Local::now();
        // Worked ~3h, so ~5h (18000s) remaining; allow a small clock delta.
        assert!((s.remaining_secs(now) - 5 * 3600).abs() < 5);
        assert_eq!(
            s.expected_check_out().timestamp(),
            (check_in + Duration::seconds(WORKDAY_SECS)).timestamp()
        );
    }

    #[test]
    fn weekly_totals_group_by_iso_week() {
        let c = mem();
        // Two sessions of 2h and 3h in the same (current) week.
        let now = Local::now();
        for (start_ago, len_h) in [(6, 2i64), (4, 3i64)] {
            let ci = now - Duration::hours(start_ago);
            let co = ci + Duration::hours(len_h);
            c.execute(
                "INSERT INTO sessions (check_in, check_out, workday_secs) VALUES (?1, ?2, ?3)",
                (ci.to_rfc3339(), co.to_rfc3339(), WORKDAY_SECS),
            )
            .unwrap();
        }
        let weeks = weekly_totals(&c, now).unwrap();
        assert_eq!(weeks.len(), 1);
        assert_eq!(weeks[0].session_count, 2);
        assert_eq!(weeks[0].total_secs, 5 * 3600);
    }
}
