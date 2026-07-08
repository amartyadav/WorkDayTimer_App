# Workday Timer

A tiny Rust app for Ubuntu that lives in the GNOME menu bar and counts down the
time left in your 8-hour workday. It records every check-in / check-out in a
local SQLite database and shows a reports window with weekly totals.

## Features

- **Menu-bar (tray) icon** with a live countdown:
  - hover to see the remaining time, your check-in time, and your expected
    checkout (check-in + 8h);
  - the icon changes when you go into overtime.
- **Tray menu**: *Check In Now*, *Check Out Now* (check out early anytime),
  *Open Reports*, and *Quit*.
- **Reports window** (also has Check In / Check Out buttons):
  - big live countdown and your expected leave time;
  - **hour balance** — time you *owe* (checked out early) vs. time you're
    *ahead* (worked late), shown **this week** and **all-time**; working extra
    repays the debt and both counters update. Only completed sessions count;
    amounts are minute-level ("even" under a minute);
  - **check in at an earlier time today** (forgot to check in? enter HH:MM);
  - **check out at an earlier time** (forgot to click when you left? enter HH:MM);
  - **reset today's check-in** — discard the current session to redo it with a
    corrected time (two-step confirm);
  - **weekly totals** (hours summed per ISO week);
  - full **session history** — check-in, check-out, expected-out, hours — with
    per-row **Edit** (correct a past day's check-in/check-out via
    `YYYY-MM-DD HH:MM`, or blank the check-out to mark it in-progress) and
    **Delete** (two-step confirm).
- **SQLite storage** at `~/.local/share/workday_timer/workday.db` — inspect it
  anytime with `sqlite3`.
- Closing the window **minimizes it** and the tray timer keeps running (hiding a
  window isn't supported on Wayland). Reopen it from the tray; use the tray's
  *Quit* to exit completely.
- **Single instance**: launching it again while it's already running just raises
  the existing window instead of starting a second copy (coordinated through a
  socket at `$XDG_RUNTIME_DIR/workday_timer.sock`).

## Threads

- main thread — the egui reports window (required on Linux);
- a DBus thread — the `ksni` tray icon;
- a ticker thread — refreshes the countdown once per second.

SQLite is the single source of truth, so the tray and the window always agree.

## Build & install

Requires a recent Rust toolchain (built and tested with Rust 1.96 on
Ubuntu 26.04 / GNOME 50, Wayland).

```bash
./install.sh
```

This builds the release binary, copies it to `~/.local/bin/workday-timer`, and
adds a "Workday Timer" launcher to your app grid.

To start it automatically at login:

```bash
cp ~/.local/share/applications/workday-timer.desktop ~/.config/autostart/
```

Or just run it directly:

```bash
cargo run --release
```

## Notes

- The tray icon relies on the **AppIndicator** GNOME extension, which ships
  enabled by default on Ubuntu (`ubuntu-appindicators@ubuntu.com`). If you don't
  see the icon, make sure that extension is on.
- GNOME shows the countdown in the icon's **tooltip / menu** rather than as
  always-on text next to the icon (the Shell doesn't render live tray labels).
  Hover the icon or open its menu to see the time remaining.

## Data / reset

All data is in one SQLite file:

```bash
sqlite3 ~/.local/share/workday_timer/workday.db 'SELECT * FROM sessions;'
```

Delete that file to start fresh.
