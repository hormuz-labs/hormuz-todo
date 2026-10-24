mod daemon;
mod notify;
mod service;

use std::path::PathBuf;

use anyhow::{Result, bail};
use chrono::{Duration, Local};
use clap::{Parser, Subcommand};
use hormuz_core::model::{fmt_days, fmt_hm, parse_days, parse_hm};
use hormuz_core::{Store, TaskInput, default_db_path, seed};

pub const DEFAULT_PORT: u16 = 7878;

#[derive(Parser)]
#[command(name = "hormuz", version, about = "Daily routine reminders: a todo list that nags you until it's done")]
struct Cli {
    /// Database file (default: platform data dir, or $HORMUZ_DB)
    #[arg(long, global = true)]
    db: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show today's checklist (default)
    Today,
    /// List all tasks with their schedules
    List,
    /// Add a recurring task
    Add {
        title: String,
        /// Reminder time, HH:MM (repeatable)
        #[arg(long = "at", required = true)]
        at: Vec<String>,
        /// Days: daily, weekdays, weekends, or mon,tue,...
        #[arg(long, default_value = "daily")]
        days: String,
        #[arg(long)]
        url: Option<String>,
        #[arg(long, default_value = "")]
        notes: String,
        /// Re-remind every N minutes until done
        #[arg(long)]
        nag: Option<u32>,
    },
    /// Mark a task done for today
    Done { id: i64 },
    /// Un-mark a task for today
    Undo { id: i64 },
    /// Silence a task's reminders for N minutes (0 clears)
    Snooze { id: i64, minutes: u32 },
    /// Turn a task's reminders on or off without deleting it
    Toggle { id: i64 },
    /// Delete a task and its history
    Rm { id: i64 },
    /// Add the starter routine (Product Hunt, Reddit)
    Seed,
    /// Run or manage the background reminder daemon
    Daemon {
        #[command(subcommand)]
        action: Option<DaemonCmd>,
    },
    /// Open the web UI served by the daemon
    Open {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// Send a test notification
    NotifyTest,
}

#[derive(Subcommand)]
enum DaemonCmd {
    /// Run in the foreground (what the service runs)
    Run {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// Install as a login service (launchd on macOS, systemd --user on Linux) and start it
    Install {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
    /// Stop and remove the login service
    Uninstall,
    /// Is the daemon reachable?
    Status {
        #[arg(long, default_value_t = DEFAULT_PORT)]
        port: u16,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let db = cli.db.clone().unwrap_or_else(default_db_path);
    let open = || Store::open(&db);

    match cli.cmd.unwrap_or(Cmd::Today) {
        Cmd::Today => print_today(&open()?)?,
        Cmd::List => print_list(&open()?)?,
        Cmd::Add { title, at, days, url, notes, nag } => {
            let times = at
                .iter()
                .map(|s| parse_hm(s).ok_or_else(|| anyhow::anyhow!("bad time '{s}', expected HH:MM")))
                .collect::<Result<Vec<_>>>()?;
            let days = parse_days(&days).map_err(anyhow::Error::msg)?;
            let t = open()?.add_task(&TaskInput { title, notes, url, days, times, nag_minutes: nag, enabled: true })?;
            println!("added #{} {}", t.id, t.title);
        }
        Cmd::Done { id } => {
            open()?.set_done(id, Local::now().date_naive(), true)?;
            print_today(&open()?)?;
        }
        Cmd::Undo { id } => {
            open()?.set_done(id, Local::now().date_naive(), false)?;
            print_today(&open()?)?;
        }
        Cmd::Snooze { id, minutes } => {
            let until = (minutes > 0).then(|| Local::now() + Duration::minutes(minutes as i64));
            open()?.snooze(id, until)?;
            match until {
                Some(u) => println!("#{id} snoozed until {}", u.format("%H:%M")),
                None => println!("#{id} snooze cleared"),
            }
        }
        Cmd::Toggle { id } => {
            let store = open()?;
            let t = store.get_task(id)?;
            let input = TaskInput {
                title: t.title,
                notes: t.notes,
                url: t.url,
                days: t.days,
                times: t.times,
                nag_minutes: t.nag_minutes,
                enabled: !t.enabled,
            };
            let t = store.update_task(id, &input)?;
            println!("#{id} {}", if t.enabled { "enabled" } else { "disabled" });
        }
        Cmd::Rm { id } => {
            open()?.delete_task(id)?;
            println!("deleted #{id}");
        }
        Cmd::Seed => {
            let added = seed::seed(&open()?)?;
            if added.is_empty() {
                println!("starter tasks already present");
            }
            for t in added {
                println!("added #{} {}", t.id, t.title);
            }
        }
        Cmd::Daemon { action } => match action.unwrap_or(DaemonCmd::Run { port: DEFAULT_PORT }) {
            DaemonCmd::Run { port } => daemon::run(db, port)?,
            DaemonCmd::Install { port } => service::install(cli.db.as_deref(), port)?,
            DaemonCmd::Uninstall => service::uninstall()?,
            DaemonCmd::Status { port } => {
                if service::ping(port) {
                    println!("daemon is running on http://127.0.0.1:{port}");
                } else {
                    println!("daemon is not reachable on port {port}");
                    std::process::exit(1);
                }
            }
        },
        Cmd::Open { port } => {
            if !service::ping(port) {
                bail!("daemon isn't running on port {port}; start it with `hormuz daemon install` or `hormuz daemon run`");
            }
            service::open_url(&format!("http://127.0.0.1:{port}"))?;
        }
        Cmd::NotifyTest => notify::send("Hormuz", "Notifications are working.")?,
    }
    Ok(())
}

fn print_today(store: &Store) -> Result<()> {
    let now = Local::now();
    let a = store.agenda(now.date_naive(), now)?;
    println!("{} — {}/{} done", now.format("%a %d %b"), a.done, a.total);
    if a.items.is_empty() {
        println!("  nothing scheduled today. `hormuz seed` adds a starter routine.");
    }
    for i in &a.items {
        let mark = if i.done { "[x]" } else { "[ ]" };
        let time = i.task.first_time().map(fmt_hm).unwrap_or_else(|| "--:--".into());
        let mut extra = Vec::new();
        if i.streak > 0 {
            extra.push(format!("streak {}", i.streak));
        }
        if i.overdue {
            extra.push("overdue".into());
        }
        if let Some(s) = i.task.snoozed_until.filter(|s| *s > now) {
            extra.push(format!("snoozed until {}", s.format("%H:%M")));
        }
        let extra = if extra.is_empty() { String::new() } else { format!("  ({})", extra.join(", ")) };
        println!("  {mark} #{:<3} {time}  {}{extra}", i.task.id, i.task.title);
    }
    Ok(())
}

fn print_list(store: &Store) -> Result<()> {
    for t in store.list_tasks()? {
        let times = t.times.iter().map(|t| fmt_hm(*t)).collect::<Vec<_>>().join(",");
        let nag = t.nag_minutes.map(|n| format!(" nag {n}m")).unwrap_or_default();
        let off = if t.enabled { "" } else { " [off]" };
        println!("#{:<3} {:<12} {:<9} {}{nag}{off}", t.id, times, fmt_days(&t.days), t.title);
        if let Some(u) = &t.url {
            println!("     {u}");
        }
    }
    Ok(())
}
