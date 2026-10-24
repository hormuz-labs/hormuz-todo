use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, NaiveDate, Timelike};
use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::model::{Agenda, AgendaItem, Task, TaskInput, fmt_hm, parse_hm};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS tasks (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    title         TEXT    NOT NULL,
    notes         TEXT    NOT NULL DEFAULT '',
    url           TEXT,
    days          TEXT    NOT NULL DEFAULT '',
    times         TEXT    NOT NULL DEFAULT '',
    nag_minutes   INTEGER,
    enabled       INTEGER NOT NULL DEFAULT 1,
    snoozed_until TEXT,
    created_at    TEXT    NOT NULL
);
CREATE TABLE IF NOT EXISTS completions (
    task_id  INTEGER NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
    date     TEXT    NOT NULL,
    done_at  TEXT    NOT NULL,
    PRIMARY KEY (task_id, date)
);
";

/// SQLite-backed task store. The daemon, CLI and desktop app can open the same file at once.
pub struct Store {
    conn: Connection,
}

/// `~/Library/Application Support/hormuz/hormuz.db` on macOS, `~/.local/share/hormuz/hormuz.db` on Linux.
/// Overridable with `HORMUZ_DB`.
pub fn default_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("HORMUZ_DB") {
        return PathBuf::from(p);
    }
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("hormuz").join("hormuz.db")
}

impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn open_default() -> Result<Self> {
        Self::open(default_db_path())
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn list_tasks(&self) -> Result<Vec<Task>> {
        let mut stmt = self.conn.prepare("SELECT * FROM tasks ORDER BY id")?;
        let mut tasks = stmt.query_map([], task_from_row)?.collect::<rusqlite::Result<Vec<_>>>()?;
        tasks.sort_by_key(|t| (t.first_time(), t.id));
        Ok(tasks)
    }

    pub fn get_task(&self, id: i64) -> Result<Task> {
        self.conn
            .query_row("SELECT * FROM tasks WHERE id = ?1", [id], task_from_row)
            .optional()?
            .with_context(|| format!("no task with id {id}"))
    }

    pub fn add_task(&self, input: &TaskInput) -> Result<Task> {
        validate(input)?;
        self.conn.execute(
            "INSERT INTO tasks (title, notes, url, days, times, nag_minutes, enabled, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                input.title.trim(),
                input.notes,
                clean_url(&input.url),
                join_days(&input.days),
                join_times(input),
                input.nag_minutes,
                input.enabled,
                Local::now().to_rfc3339(),
            ],
        )?;
        self.get_task(self.conn.last_insert_rowid())
    }

    pub fn update_task(&self, id: i64, input: &TaskInput) -> Result<Task> {
        validate(input)?;
        let n = self.conn.execute(
            "UPDATE tasks SET title = ?1, notes = ?2, url = ?3, days = ?4, times = ?5,
                    nag_minutes = ?6, enabled = ?7
             WHERE id = ?8",
            params![
                input.title.trim(),
                input.notes,
                clean_url(&input.url),
                join_days(&input.days),
                join_times(input),
                input.nag_minutes,
                input.enabled,
                id,
            ],
        )?;
        if n == 0 {
            bail!("no task with id {id}");
        }
        self.get_task(id)
    }

    pub fn delete_task(&self, id: i64) -> Result<()> {
        if self.conn.execute("DELETE FROM tasks WHERE id = ?1", [id])? == 0 {
            bail!("no task with id {id}");
        }
        Ok(())
    }

    pub fn set_done(&self, id: i64, date: NaiveDate, done: bool) -> Result<()> {
        self.get_task(id)?;
        if done {
            self.conn.execute(
                "INSERT OR IGNORE INTO completions (task_id, date, done_at) VALUES (?1, ?2, ?3)",
                params![id, date.to_string(), Local::now().to_rfc3339()],
            )?;
            self.conn.execute("UPDATE tasks SET snoozed_until = NULL WHERE id = ?1", [id])?;
        } else {
            self.conn
                .execute("DELETE FROM completions WHERE task_id = ?1 AND date = ?2", params![id, date.to_string()])?;
        }
        Ok(())
    }

    pub fn snooze(&self, id: i64, until: Option<DateTime<Local>>) -> Result<()> {
        let n = self.conn.execute(
            "UPDATE tasks SET snoozed_until = ?1 WHERE id = ?2",
            params![until.map(|u| u.to_rfc3339()), id],
        )?;
        if n == 0 {
            bail!("no task with id {id}");
        }
        Ok(())
    }

    /// Ids of tasks completed on `date`.
    pub fn done_on(&self, date: NaiveDate) -> Result<HashSet<i64>> {
        let mut stmt = self.conn.prepare("SELECT task_id FROM completions WHERE date = ?1")?;
        let ids = stmt.query_map([date.to_string()], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    fn done_at(&self, id: i64, date: NaiveDate) -> Result<Option<DateTime<Local>>> {
        let s: Option<String> = self
            .conn
            .query_row(
                "SELECT done_at FROM completions WHERE task_id = ?1 AND date = ?2",
                params![id, date.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        Ok(s.and_then(|s| parse_dt(&s)))
    }

    fn completion_dates(&self, id: i64) -> Result<HashSet<NaiveDate>> {
        let mut stmt = self.conn.prepare("SELECT date FROM completions WHERE task_id = ?1")?;
        let dates = stmt
            .query_map([id], |r| r.get::<_, String>(0))?
            .filter_map(|r| r.ok().and_then(|s| s.parse().ok()))
            .collect();
        Ok(dates)
    }

    /// Consecutive scheduled days the task was done, counting back from `today`.
    /// An unfinished `today` doesn't break the streak; the day isn't over yet.
    pub fn streak(&self, task: &Task, today: NaiveDate) -> Result<u32> {
        let dates = self.completion_dates(task.id)?;
        let created = task.created_at.date_naive();
        let mut day = today;
        let mut streak = 0;
        if !dates.contains(&today) {
            day = day.pred_opt().unwrap();
        }
        for _ in 0..3660 {
            if day < created && !dates.contains(&day) {
                break;
            }
            if task.is_scheduled_on(day) {
                if dates.contains(&day) {
                    streak += 1;
                } else {
                    break;
                }
            }
            day = day.pred_opt().unwrap();
        }
        Ok(streak)
    }

    /// Everything due on `date`, with done state. `now` decides "overdue" and "next reminder".
    pub fn agenda(&self, date: NaiveDate, now: DateTime<Local>) -> Result<Agenda> {
        let mut items = Vec::new();
        for task in self.list_tasks()? {
            if !task.enabled || !task.is_scheduled_on(date) {
                continue;
            }
            let done_at = self.done_at(task.id, date)?;
            let done = done_at.is_some();
            let is_today = date == now.date_naive();
            let now_t = now.time().with_second(0).unwrap();
            let next_time = if is_today && !done {
                task.times.iter().find(|t| **t > now_t).map(|t| fmt_hm(*t))
            } else {
                None
            };
            let overdue = !done
                && (date < now.date_naive() || (is_today && task.first_time().is_some_and(|t| t <= now.time())));
            let streak = self.streak(&task, date)?;
            items.push(AgendaItem { task, done, done_at, streak, next_time, overdue });
        }
        let done = items.iter().filter(|i| i.done).count();
        Ok(Agenda { date, total: items.len(), done, items })
    }

    pub fn is_empty(&self) -> Result<bool> {
        let n: i64 = self.conn.query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))?;
        Ok(n == 0)
    }
}

fn validate(input: &TaskInput) -> Result<()> {
    if input.title.trim().is_empty() {
        bail!("title can't be empty");
    }
    if input.days.iter().any(|d| *d > 6) {
        bail!("days must be 0 (Mon) to 6 (Sun)");
    }
    if input.nag_minutes == Some(0) {
        bail!("nag interval must be at least 1 minute");
    }
    Ok(())
}

fn clean_url(url: &Option<String>) -> Option<String> {
    url.as_ref().map(|u| u.trim().to_string()).filter(|u| !u.is_empty())
}

fn join_days(days: &[u8]) -> String {
    let mut d = days.to_vec();
    d.sort_unstable();
    d.dedup();
    d.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(",")
}

fn join_times(input: &TaskInput) -> String {
    let mut t = input.times.clone();
    t.sort();
    t.dedup();
    t.iter().map(|t| fmt_hm(*t)).collect::<Vec<_>>().join(",")
}

fn parse_dt(s: &str) -> Option<DateTime<Local>> {
    DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&Local))
}

fn task_from_row(r: &Row) -> rusqlite::Result<Task> {
    let days: String = r.get("days")?;
    let times: String = r.get("times")?;
    let snoozed: Option<String> = r.get("snoozed_until")?;
    let created: String = r.get("created_at")?;
    Ok(Task {
        id: r.get("id")?,
        title: r.get("title")?,
        notes: r.get("notes")?,
        url: r.get("url")?,
        days: days.split(',').filter_map(|d| d.parse().ok()).collect(),
        times: times.split(',').filter_map(parse_hm).collect(),
        nag_minutes: r.get("nag_minutes")?,
        enabled: r.get("enabled")?,
        snoozed_until: snoozed.as_deref().and_then(parse_dt),
        created_at: parse_dt(&created).unwrap_or_else(Local::now),
    })
}
