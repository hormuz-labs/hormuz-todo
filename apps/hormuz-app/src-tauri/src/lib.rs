//! Hormuz app shell for desktop (macOS, Windows, Linux) and mobile (iOS, Android).
//!
//! The UI in `/ui` talks to `rpc`, which runs the same command set as the daemon.
//! Desktop shares the daemon's database, so ticking a task here or in the CLI is the same thing.
//! Reminders:
//! - desktop: the daemon sends them; if it isn't running, this app sends them while it's running.
//!   Closing the window only hides it: the app stays in the menu bar, showing today's progress,
//!   until you choose Quit.
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
            #[cfg(desktop)]
            tray::setup(app.handle(), store.clone())?;
            start_reminders(app.handle(), store);
            Ok(())
        })
        .on_window_event(|window, event| {
            // Keep running in the menu bar instead of quitting when the window is closed.
            #[cfg(desktop)]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
            #[cfg(mobile)]
            let _ = (window, event);
        })
        .invoke_handler(tauri::generate_handler![rpc])
        .build(tauri::generate_context!())
        .expect("error while building Hormuz")
        .run(|_app, _event| {
            // Clicking the Dock icon brings the hidden window back.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                tray::show_main(_app);
            }
        });
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
fn on_tasks_changed(app: &AppHandle, store: &Arc<Mutex<Store>>) {
    tray::refresh(app, store);
}

/// The menu bar icon: today's progress as its title, and today's tasks right in its menu, ticked off
/// with one click, plus links to open, whether reminders survive quitting, Open and Quit.
#[cfg(desktop)]
mod tray {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use chrono::Local;
    use hormuz_core::{Agenda, Store};
    use tauri::image::Image;
    use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use tauri::{AppHandle, Emitter, Manager, Wry};
    use tauri_plugin_opener::OpenerExt;

    const ID: &str = "main";
    /// Longest task title shown in the menu before it's cut with an ellipsis.
    const MAX_LABEL: usize = 52;

    /// What the menu was last built from; it's only rebuilt when this changes, so an open menu
    /// isn't swapped out from under the pointer by the periodic refresh.
    #[derive(Default)]
    struct Shown(Mutex<String>);

    pub fn setup(app: &AppHandle, store: Arc<Mutex<Store>>) -> tauri::Result<()> {
        app.manage(Shown::default());
        TrayIconBuilder::with_id(ID)
            .icon(Image::from_bytes(include_bytes!("../icons/tray.png"))?)
            .icon_as_template(true)
            .tooltip("Hormuz")
            .menu(&Menu::new(app)?)
            .show_menu_on_left_click(cfg!(target_os = "macos"))
            .on_menu_event(|app, event| on_menu(app, event.id.as_ref()))
            .on_tray_icon_event(|tray, event| {
                // Windows and Linux: left click opens the window, right click shows the menu.
                if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event
                    && !cfg!(target_os = "macos")
                {
                    show_main(tray.app_handle());
                }
            })
            .build(app)?;
        refresh(app, &store);

        // Keep the title and list current as the day goes on and as the CLI or daemon change things.
        let app = app.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(20));
                refresh(&app, &store);
            }
        });
        Ok(())
    }

    fn on_menu(app: &AppHandle, id: &str) {
        let store = app.state::<super::AppState>().store.clone();
        if let Some(task) = id.strip_prefix("task:").and_then(|n| n.parse::<i64>().ok()) {
            let today = Local::now().date_naive();
            {
                let s = store.lock().unwrap();
                let done = s.done_on(today).map(|d| d.contains(&task)).unwrap_or(false);
                if let Err(e) = s.set_done(task, today, !done) {
                    eprintln!("hormuz: ticking task {task} failed: {e:#}");
                }
            }
            refresh(app, &store);
            // Let an open window redraw straight away instead of on its next poll.
            let _ = app.emit("tasks-changed", ());
        } else if let Some(task) = id.strip_prefix("link:").and_then(|n| n.parse::<i64>().ok()) {
            let url = store.lock().unwrap().get_task(task).ok().and_then(|t| t.url);
            if let Some(url) = url {
                let _ = app.opener().open_url(url, None::<&str>);
            }
        } else if id == "open" {
            show_main(app);
        } else if id == "quit" {
            app.exit(0);
        }
    }

    pub fn refresh(app: &AppHandle, store: &Arc<Mutex<Store>>) {
        let now = Local::now();
        let Ok(agenda) = store.lock().unwrap().agenda(now.date_naive(), now) else { return };
        let daemon = super::daemon_running();

        let left = agenda.total - agenda.done;
        let title = (agenda.total > 0).then(|| format!("{}/{}", agenda.done, agenda.total));
        let summary = match (agenda.total, left) {
            (0, _) => "Nothing scheduled today".to_string(),
            (t, 0) => format!("All {t} done today"),
            (t, l) => format!("{l} of {t} left today"),
        };
        let Some(tray) = app.tray_by_id(ID) else { return };
        let _ = tray.set_title(title.as_deref());
        let _ = tray.set_tooltip(Some(format!("Hormuz · {summary}")));

        let key = format!(
            "{}|{daemon}|{}",
            agenda.date,
            agenda
                .items
                .iter()
                .map(|i| format!("{}:{}:{}:{}:{}", i.task.id, i.done, i.overdue, i.task.title, i.task.url.is_some()))
                .collect::<Vec<_>>()
                .join(";")
        );
        let shown = app.state::<Shown>();
        let mut last = shown.0.lock().unwrap();
        if *last == key {
            return;
        }
        match build_menu(app, &agenda, &summary, daemon) {
            Ok(menu) => {
                let _ = tray.set_menu(Some(menu));
                *last = key;
            }
            Err(e) => eprintln!("hormuz: building the menu bar menu failed: {e}"),
        }
    }

    fn build_menu(app: &AppHandle, agenda: &Agenda, summary: &str, daemon: bool) -> tauri::Result<Menu<Wry>> {
        let menu = Menu::new(app)?;
        menu.append(&MenuItem::with_id(app, "summary", summary, false, None::<&str>)?)?;
        for item in &agenda.items {
            let time = item.task.first_time().map(hormuz_core::model::fmt_hm).unwrap_or_default();
            let due = if item.overdue { "  · overdue" } else { "" };
            let label = format!("{time}   {}{due}", clip(&item.task.title));
            menu.append(&CheckMenuItem::with_id(app, format!("task:{}", item.task.id), label, true, item.done, None::<&str>)?)?;
        }

        let links: Vec<_> = agenda.items.iter().filter(|i| i.task.url.is_some()).collect();
        if !links.is_empty() {
            menu.append(&PredefinedMenuItem::separator(app)?)?;
            let sub = Submenu::with_id(app, "links", "Open link", true)?;
            for item in links {
                let host = item.task.url.as_deref().map(host).unwrap_or_default();
                sub.append(&MenuItem::with_id(
                    app,
                    format!("link:{}", item.task.id),
                    format!("{}  ({host})", clip(&item.task.title)),
                    true,
                    None::<&str>,
                )?)?;
            }
            menu.append(&sub)?;
        }

        let reminders =
            if daemon { "Reminders keep coming after you quit" } else { "Reminders stop if you quit Hormuz" };
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        menu.append(&MenuItem::with_id(app, "reminders", reminders, false, None::<&str>)?)?;
        menu.append(&MenuItem::with_id(app, "open", "Open Hormuz…", true, None::<&str>)?)?;
        menu.append(&MenuItem::with_id(app, "quit", "Quit Hormuz", true, None::<&str>)?)?;
        Ok(menu)
    }

    fn clip(s: &str) -> String {
        if s.chars().count() <= MAX_LABEL {
            s.to_string()
        } else {
            format!("{}…", s.chars().take(MAX_LABEL - 1).collect::<String>().trim_end())
        }
    }

    /// "https://www.producthunt.com/posts" -> "producthunt.com"
    fn host(url: &str) -> String {
        let rest = url.split_once("://").map_or(url, |(_, r)| r);
        let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        host.strip_prefix("www.").unwrap_or(host).to_string()
    }

    pub fn show_main(app: &AppHandle) {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
}

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
