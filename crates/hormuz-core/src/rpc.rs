//! One command surface shared by every front end: the daemon's HTTP API (`POST /rpc/<cmd>`),
//! the Tauri app (`invoke("rpc", { cmd, args })`) and the web UI that talks to either.

use anyhow::{Context, Result, bail};
use chrono::{Duration, Local, NaiveDate};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::model::TaskInput;
use crate::seed;
use crate::store::Store;

/// Commands that change data; front ends use this to refresh scheduled notifications.
pub fn is_mutation(cmd: &str) -> bool {
    matches!(cmd, "add_task" | "update_task" | "delete_task" | "set_done" | "snooze" | "seed_defaults")
}

#[derive(Deserialize)]
struct Id {
    id: i64,
}

#[derive(Deserialize)]
struct Update {
    id: i64,
    input: TaskInput,
}

#[derive(Deserialize)]
struct Add {
    input: TaskInput,
}

#[derive(Deserialize)]
struct SetDone {
    id: i64,
    #[serde(default)]
    date: Option<NaiveDate>,
    done: bool,
}

#[derive(Deserialize)]
struct Snooze {
    id: i64,
    /// Minutes from now; 0 or missing clears the snooze.
    #[serde(default)]
    minutes: u32,
}

#[derive(Deserialize, Default)]
struct AgendaArgs {
    #[serde(default)]
    date: Option<NaiveDate>,
}

fn args<T: for<'de> Deserialize<'de>>(cmd: &str, v: Value) -> Result<T> {
    serde_json::from_value(v).with_context(|| format!("bad arguments for '{cmd}'"))
}

pub fn dispatch(store: &Store, cmd: &str, a: Value) -> Result<Value> {
    let now = Local::now();
    let a = if a.is_null() { json!({}) } else { a };
    Ok(match cmd {
        "agenda" => {
            let a: AgendaArgs = args(cmd, a)?;
            json!(store.agenda(a.date.unwrap_or(now.date_naive()), now)?)
        }
        "list_tasks" => json!(store.list_tasks()?),
        "get_task" => json!(store.get_task(args::<Id>(cmd, a)?.id)?),
        "add_task" => json!(store.add_task(&args::<Add>(cmd, a)?.input)?),
        "update_task" => {
            let a: Update = args(cmd, a)?;
            json!(store.update_task(a.id, &a.input)?)
        }
        "delete_task" => {
            store.delete_task(args::<Id>(cmd, a)?.id)?;
            json!({ "ok": true })
        }
        "set_done" => {
            let a: SetDone = args(cmd, a)?;
            store.set_done(a.id, a.date.unwrap_or(now.date_naive()), a.done)?;
            json!({ "ok": true })
        }
        "snooze" => {
            let a: Snooze = args(cmd, a)?;
            let until = (a.minutes > 0).then(|| now + Duration::minutes(a.minutes as i64));
            store.snooze(a.id, until)?;
            json!({ "ok": true, "until": until })
        }
        "seed_defaults" => json!(seed::seed(store)?),
        "version" => json!({ "version": env!("CARGO_PKG_VERSION") }),
        other => bail!("unknown command '{other}'"),
    })
}
