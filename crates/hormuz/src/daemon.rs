//! The background process: checks reminders every few seconds and serves the API + web UI
//! on 127.0.0.1 so the desktop app, browser and CLI all see the same state.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use chrono::Local;
use hormuz_core::reminder::ReminderEngine;
use hormuz_core::{Store, rpc};
use serde_json::{Value, json};

use crate::notify;

const TICK: Duration = Duration::from_secs(15);

type Shared = Arc<Mutex<Store>>;

pub fn run(db: PathBuf, port: u16) -> Result<()> {
    let store = Store::open(&db)?;
    if store.is_empty()? {
        hormuz_core::seed::seed(&store)?;
    }
    let store: Shared = Arc::new(Mutex::new(store));
    eprintln!("hormuz daemon: db {}", db.display());

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async move {
        tokio::spawn(reminder_loop(store.clone()));

        let app = Router::new()
            .route("/", get(|| async { asset("index.html") }))
            .route("/{file}", get(|Path(f): Path<String>| async move { asset(&f) }))
            .route("/rpc/{cmd}", post(rpc_handler))
            .with_state(store);

        let addr = ("127.0.0.1", port);
        let listener = tokio::net::TcpListener::bind(addr)
            .await
            .with_context(|| format!("binding 127.0.0.1:{port} (is another daemon running?)"))?;
        eprintln!("hormuz daemon: listening on http://127.0.0.1:{port}");
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = tokio::signal::ctrl_c().await;
            })
            .await?;
        Ok(())
    })
}

async fn reminder_loop(store: Shared) {
    let mut engine = ReminderEngine::new(Local::now());

    // Catch up on startup (e.g. after login): one summary instead of a burst of old reminders.
    let pending = {
        let s = store.lock().unwrap();
        let now = Local::now();
        match (s.list_tasks(), s.done_on(now.date_naive())) {
            (Ok(tasks), Ok(done)) => engine.pending(&tasks, &done, now),
            _ => Vec::new(),
        }
    };
    if !pending.is_empty() {
        let names = pending.iter().map(|t| format!("• {}", t.title)).collect::<Vec<_>>().join("\n");
        deliver(format!("{} task(s) waiting today", pending.len()), names).await;
    }

    let mut interval = tokio::time::interval(TICK);
    loop {
        interval.tick().await;
        let reminders = {
            let s = store.lock().unwrap();
            let now = Local::now();
            match (s.list_tasks(), s.done_on(now.date_naive())) {
                (Ok(tasks), Ok(done)) => engine.tick(&tasks, &done, now),
                (Err(e), _) | (_, Err(e)) => {
                    eprintln!("hormuz daemon: reading tasks failed: {e:#}");
                    continue;
                }
            }
        };
        for r in reminders {
            deliver(r.task.title.clone(), r.body()).await;
        }
    }
}

async fn deliver(title: String, body: String) {
    eprintln!("[{}] remind: {title}", Local::now().format("%F %T"));
    let res = tokio::task::spawn_blocking(move || notify::send(&title, &body)).await;
    if let Ok(Err(e)) = res {
        eprintln!("hormuz daemon: notification failed: {e:#}");
    }
}

/// Only same-machine pages may call the API: the Host must be loopback (blocks DNS rebinding)
/// and the custom header forces a CORS preflight that we never approve (blocks other sites).
fn allowed(headers: &HeaderMap) -> bool {
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    let host = host.rsplit_once(':').map_or(host, |(h, _)| h);
    let loopback = matches!(host, "127.0.0.1" | "localhost" | "[::1]");
    loopback && headers.contains_key("x-hormuz")
}

async fn rpc_handler(State(store): State<Shared>, Path(cmd): Path<String>, headers: HeaderMap, body: Bytes) -> Response {
    if !allowed(&headers) {
        return (StatusCode::FORBIDDEN, "forbidden").into_response();
    }
    let args: Value = if body.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return (StatusCode::BAD_REQUEST, axum::Json(json!({ "error": e.to_string() }))).into_response(),
        }
    };
    let result = {
        let s = store.lock().unwrap();
        rpc::dispatch(&s, &cmd, args)
    };
    match result {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, axum::Json(json!({ "error": format!("{e:#}") }))).into_response(),
    }
}

/// The same UI the Tauri app ships, compiled into the binary.
fn asset(name: &str) -> Response {
    let (body, ty) = match name {
        "index.html" => (include_str!("../../../ui/index.html"), "text/html; charset=utf-8"),
        "app.js" => (include_str!("../../../ui/app.js"), "text/javascript; charset=utf-8"),
        "style.css" => (include_str!("../../../ui/style.css"), "text/css; charset=utf-8"),
        "icon.svg" => (include_str!("../../../ui/icon.svg"), "image/svg+xml"),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    ([(header::CONTENT_TYPE, ty), (header::CACHE_CONTROL, "no-cache")], body).into_response()
}
