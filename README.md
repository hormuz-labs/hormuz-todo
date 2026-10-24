# Hormuz

A daily-routine todo app that reminds you until things are done: "check Product Hunt and upvote a few", "add something useful on Reddit", and whatever else belongs in your day.

```
crates/hormuz-core   tasks, schedules, completions, streaks, reminder engine, SQLite store, RPC commands
crates/hormuz        `hormuz` CLI + background daemon (notifications, local API, web UI)
apps/hormuz-app      Tauri 2 app for macOS / Windows / Linux / iOS / Android
ui/                  the one UI, shared by the app and the daemon's web page
```

## How it fits together

- **Tasks** repeat daily, on weekdays, weekends or chosen days, with one or more reminder times and an optional
  "nag every N minutes until done". Ticking a task off stops its reminders for the day and builds a streak.
- **Daemon** (`hormuz daemon`) checks every 15 s and sends native notifications. On startup it sends one
  summary of anything you've missed instead of a burst. It serves the API and the UI on `http://127.0.0.1:7878`
  (loopback only; requests need an `X-Hormuz` header and a loopback `Host`, so other websites can't call it).
- **Desktop app** opens the same SQLite file as the daemon and CLI, so all three agree. If the daemon isn't
  running, the app sends reminders itself while its window is open.
- **Mobile app** keeps its own database on the phone. Phones don't allow background daemons, so each reminder
  time is registered with the OS as a repeating local notification and rescheduled when tasks change.

## Quick start (CLI + daemon)

```bash
cargo install --path crates/hormuz
hormuz seed                 # Product Hunt + Reddit starter routine
hormuz daemon install       # launchd on macOS, systemd --user on Linux; starts at login
hormuz                      # today's checklist
```

Other commands:

```bash
hormuz add "Hacker News: read & comment" --at 16:00 --days weekdays --url https://news.ycombinator.com --nag 45
hormuz list
hormuz done 1               # or: hormuz undo 1
hormuz snooze 2 30          # minutes; 0 clears
hormuz toggle 3             # reminders on/off
hormuz rm 3
hormuz open                 # web UI in your browser
hormuz notify-test
hormuz daemon status | uninstall | run
```

The database lives at `~/Library/Application Support/hormuz/hormuz.db` (macOS) or `~/.local/share/hormuz/hormuz.db`
(Linux). Set `HORMUZ_DB` or pass `--db` to use another file. Daemon logs: `~/Library/Logs/hormuz.log`.

On macOS, allow notifications the first time; if none appear, check System Settings → Notifications.

## Desktop app

```bash
cargo run -p hormuz-app                     # run it
cd apps/hormuz-app && npm install && npm run build   # .app / .dmg (macOS), .msi (Windows), .deb/.AppImage (Linux)
```

## Mobile app

Needs Xcode (iOS) or Android Studio + NDK (Android), then:

```bash
cd apps/hormuz-app && npm install
rustup target add aarch64-apple-ios aarch64-apple-ios-sim     # iOS
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android  # Android
npm run ios:init && npm run ios:dev
npm run android:init && npm run android:dev
```

Mobile reminders fire at each scheduled time even if you already did the task that day (the OS schedule can't
see your checklist). Phone and desktop don't sync yet.

## Development

```bash
cargo test
cargo run -p hormuz -- daemon run --port 7979   # then open http://127.0.0.1:7979
```

The UI is plain HTML/CSS/JS in `ui/` with no build step. It calls `invoke("rpc", { cmd, args })` inside Tauri and
`POST /rpc/<cmd>` in a browser; both go to `hormuz_core::rpc::dispatch`.
