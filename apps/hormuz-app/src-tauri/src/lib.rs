//! Hormuz app shell for desktop (macOS, Windows, Linux) and mobile (iOS, Android).
//!
//! The UI in `/ui` talks to `rpc`, which runs the same command set as the daemon.
//! Desktop shares the daemon's database, so ticking a task here or in the CLI is the same thing.
//! Reminders:
//! - desktop: the daemon sends them; if it isn't running, this app sends them while it's open.
//! - mobile: phones don't allow long-running daemons, so each reminder time is handed to the OS
//!   as a repeating local notification and rescheduled whenever tasks change.

use std::sync::{Arc, Mutex};

use hormuz_core::{Store, rpc};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager, State};

struct AppState {
    store: Arc<Mutex<Store>>,
}

#[tauri::command]
fn rpc(app: AppHandle, state: State<AppState>, cmd: String, args: Value) -> Result<Value, String> {
    if cmd == "host_info" {
        return Ok(host_info());
    }
    let out = {
        let store = state.store.lock().unwrap();
        rpc::dispatch(&store, &cmd, args).map_err(|e| format!("{e:#}"))?
    };
    if rpc::is_mutation(&cmd) {
        on_tasks_changed(&app, &state.store);
    }
    Ok(out)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let store = open_store(app.handle())?;
            if store.is_empty()? {
                hormuz_core::seed::seed(&store)?;
            }
            let store = Arc::new(Mutex::new(store));
            app.manage(AppState { store: store.clone() });
            start_reminders(app.handle(), store);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![rpc])
        .run(tauri::generate_context!())
        .expect("error while running Hormuz");
}

#[cfg(desktop)]
fn open_store(_app: &AppHandle) -> anyhow::Result<Store> {
    Store::open_default()
}

#[cfg(mobile)]
fn open_store(app: &AppHandle) -> anyhow::Result<Store> {
    Store::open(app.path().app_data_dir()?.join("hormuz.db"))
}

// ---------------- desktop ----------------

#[cfg(desktop)]
const DAEMON_PORT: u16 = 7878;

#[cfg(desktop)]
fn daemon_running() -> bool {
    use std::net::TcpStream;
    use std::time::Duration;
    TcpStream::connect_timeout(&([127, 0, 0, 1], DAEMON_PORT).into(), Duration::from_millis(300)).is_ok()
}

#[cfg(desktop)]
fn host_info() -> Value {
    json!({ "mobile": false, "daemon": daemon_running(), "db": hormuz_core::default_db_path() })
}

#[cfg(desktop)]
fn on_tasks_changed(_app: &AppHandle, _store: &Arc<Mutex<Store>>) {}

/// Fallback reminder loop for when the daemon isn't installed: runs while the app is open
/// and stays quiet whenever the daemon is up, so nothing is sent twice.
#[cfg(desktop)]
fn start_reminders(app: &AppHandle, store: Arc<Mutex<Store>>) {
    use chrono::Local;
    use hormuz_core::reminder::ReminderEngine;
    use tauri_plugin_notification::NotificationExt;

    let app = app.clone();
    std::thread::spawn(move || {
        let mut engine = ReminderEngine::new(Local::now());
        loop {
            std::thread::sleep(std::time::Duration::from_secs(15));
            let reminders = {
                let s = store.lock().unwrap();
                let now = Local::now();
                match (s.list_tasks(), s.done_on(now.date_naive())) {
                    (Ok(tasks), Ok(done)) => engine.tick(&tasks, &done, now),
                    _ => continue,
                }
            };
            if daemon_running() {
                continue;
            }
            for r in reminders {
                let _ = app.notification().builder().title(&r.task.title).body(r.body()).show();
            }
        }
    });
}

// ---------------- mobile ----------------

#[cfg(mobile)]
fn host_info() -> Value {
    json!({ "mobile": true, "daemon": false })
}

#[cfg(mobile)]
fn start_reminders(app: &AppHandle, store: Arc<Mutex<Store>>) {
    use tauri_plugin_notification::{NotificationExt, PermissionState};
    let n = app.notification();
    if !matches!(n.permission_state(), Ok(PermissionState::Granted)) {
        let _ = n.request_permission();
    }
    on_tasks_changed(app, &store);
}

/// Replaces every scheduled OS notification with one per (task, weekday, time).
#[cfg(mobile)]
fn on_tasks_changed(app: &AppHandle, store: &Arc<Mutex<Store>>) {
    use chrono::Timelike;
    use tauri_plugin_notification::{NotificationExt, Schedule, ScheduleInterval};

    let tasks = match store.lock().unwrap().list_tasks() {
        Ok(t) => t,
        Err(e) => return eprintln!("hormuz: listing tasks failed: {e:#}"),
    };
    let n = app.notification();
    let _ = n.cancel_all();
    for task in tasks.iter().filter(|t| t.enabled) {
        let body = if task.notes.is_empty() { "Time for this".to_string() } else { task.notes.clone() };
        // Empty `days` means every day: leave weekday unset so it matches all of them.
        let days: Vec<Option<u8>> =
            if task.days.is_empty() { vec![None] } else { task.days.iter().map(|d| Some(*d)).collect() };
        for day in days {
            for (ti, t) in task.times.iter().enumerate().take(10) {
                // Ours: 0 = Monday. The plugin's: 1 = Sunday, 2 = Monday ... 7 = Saturday.
                let weekday = day.map(|d| (d + 1) % 7 + 1);
                let id = (task.id as i32) * 100 + day.map_or(9, |d| d as i32) * 10 + ti as i32;
                let interval = ScheduleInterval {
                    weekday,
                    hour: Some(t.hour() as u8),
                    minute: Some(t.minute() as u8),
                    second: Some(0),
                    ..Default::default()
                };
                let res = n
                    .builder()
                    .id(id)
                    .title(&task.title)
                    .body(&body)
                    .schedule(Schedule::Interval { interval, allow_while_idle: true })
                    .show();
                if let Err(e) = res {
                    eprintln!("hormuz: scheduling '{}' failed: {e}", task.title);
                }
            }
        }
    }
}
