<p align="center">
  <img src="ui/icon.svg" width="88" alt="Hormuz">
</p>

<h1 align="center">Hormuz</h1>

<h3 align="center">The daily routine that keeps reminding you until it's done.</h3>

<p align="center">
  <a href="https://github.com/hormuz-labs/hormuz-todo/releases/latest"><img alt="release" src="https://img.shields.io/github/v/release/hormuz-labs/hormuz-todo?style=flat&color=0f766e&label=release"></a>
  <img alt="Rust · Tauri 2" src="https://img.shields.io/badge/Rust-Tauri%202-0f766e?style=flat">
  <img alt="macOS · Windows · Linux · iOS · Android" src="https://img.shields.io/badge/macOS%20·%20Windows%20·%20Linux%20·%20iOS%20·%20Android-262626?style=flat">
</p>

<p align="center">
  <a href="#get-hormuz"><b>Download</b></a> ·
  <a href="#the-cli">CLI</a> ·
  <a href="#how-it-fits-together">How it works</a> ·
  <a href="#development">Develop</a>
</p>

---

Some things belong in every day: *check Product Hunt and upvote a few*, *add something useful on Reddit*, *read Hacker News and leave a comment*. Hormuz holds that list, reminds you at the times you choose, and, if you ask it to, **keeps nudging every few minutes until you tick it off**. Done tasks go quiet for the day and build a streak.

<table>
<tr>
<td width="33%" valign="top">

### 🔁 Routines, not to-dos
Tasks repeat daily, on weekdays, at weekends or on the days you pick, each with one or more reminder times and an optional link to open.

</td>
<td width="33%" valign="top">

### 🔔 Nags until done
Turn on "every N minutes until done" and the reminder keeps coming back. Snooze when you need to; tick it off and it stops.

</td>
<td width="33%" valign="top">

### 🔥 Streaks
Every day you finish a task adds to its streak, so the routine actually sticks.

</td>
</tr>
<tr>
<td width="33%" valign="top">

### 🖥️ Desktop, CLI and web
A native app, a `hormuz` command and a local web page, all reading the same SQLite file, so they always agree.

</td>
<td width="33%" valign="top">

### 📱 On your phone
The same app on iOS and Android, with reminders scheduled as native notifications.

</td>
<td width="33%" valign="top">

### 🔒 Yours, locally
No account and no cloud. Your list lives in one file on your machine, and the local API only answers your own computer.

</td>
</tr>
</table>

---

## Get Hormuz

| Platform | Install |
| --- | --- |
| **macOS** (Apple silicon + Intel) | `Hormuz-<version>-mac-universal.dmg` from the [latest release](https://github.com/hormuz-labs/hormuz-todo/releases/latest): signed and notarized by Apple, so it opens without a warning |
| **CLI + daemon** (macOS, Linux) | `cargo install --path crates/hormuz` ([quick start](#the-cli)) |
| **Windows / Linux app** | [build it from source](#desktop-app) |
| **iOS / Android** | [build it from source](#mobile-app) |

On macOS, allow notifications the first time. If none appear, check **System Settings → Notifications**.

---

## The CLI

```bash
cargo install --path crates/hormuz
hormuz seed                 # Product Hunt + Reddit starter routine
hormuz daemon install       # launchd on macOS, systemd --user on Linux; starts at login
hormuz                      # today's checklist
```

<details>
<summary><b>Every command</b></summary>
<br>

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

</details>

The database lives at `~/Library/Application Support/hormuz/hormuz.db` (macOS) or `~/.local/share/hormuz/hormuz.db` (Linux). Set `HORMUZ_DB` or pass `--db` to use another file. Daemon logs: `~/Library/Logs/hormuz.log`.

---

## How it fits together

```
crates/hormuz-core   tasks, schedules, completions, streaks, reminder engine, SQLite store, RPC commands
crates/hormuz        `hormuz` CLI + background daemon (notifications, local API, web UI)
apps/hormuz-app      Tauri 2 app for macOS / Windows / Linux / iOS / Android
ui/                  the one UI, shared by the app and the daemon's web page
```

- **The daemon** (`hormuz daemon`) checks every 15 s and sends native notifications. If you were away, it sends one summary of what you missed instead of a burst. It serves the API and the UI on `http://127.0.0.1:7878`, loopback only: requests need an `X-Hormuz` header and a loopback `Host`, so other websites can't call it.
- **The desktop app** opens the same SQLite file as the daemon and CLI. If the daemon isn't running, the app sends reminders itself while its window is open.
- **The mobile app** keeps its own database on the phone. Phones don't allow background daemons, so each reminder time is registered with the OS as a repeating local notification and rescheduled when tasks change.

> [!NOTE]
> On phones, reminders fire at each scheduled time even if you already did the task that day, because the OS schedule can't see your checklist. Phone and desktop don't sync yet.

---

## Desktop app

```bash
cargo run -p hormuz-app                                 # run it
cd apps/hormuz-app && npm install && npm run build      # .app / .dmg (macOS), .msi (Windows), .deb / .AppImage (Linux)
```

Tagging a release (`git tag v0.2.0 && git push origin v0.2.0`) runs [`release.yml`](.github/workflows/release.yml): it builds one universal Mac app, signs it with the Developer ID certificate, notarizes it with Apple, checks that Gatekeeper accepts it, and publishes the `.dmg`. Without the signing secrets the release stops before building, so nothing unsigned ever ships.

## Mobile app

Needs Xcode (iOS) or Android Studio + NDK (Android), then:

```bash
cd apps/hormuz-app && npm install
rustup target add aarch64-apple-ios aarch64-apple-ios-sim     # iOS
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android i686-linux-android  # Android
npm run ios:init && npm run ios:dev
npm run android:init && npm run android:dev
```

---

## Development

```bash
cargo test
cargo run -p hormuz -- daemon run --port 7979   # then open http://127.0.0.1:7979
```

The UI is plain HTML, CSS and JS in `ui/` with no build step. Inside Tauri it calls `invoke("rpc", { cmd, args })`; in a browser it calls `POST /rpc/<cmd>`. Both end up in `hormuz_core::rpc::dispatch`.

<br>

<p align="center">
  <sub>Made by <a href="https://github.com/hormuz-labs">Hormuz Labs</a></sub>
</p>
