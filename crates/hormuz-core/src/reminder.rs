//! Decides which reminders to send. Pure logic: the daemon calls [`ReminderEngine::tick`]
//! every few seconds and delivers whatever comes back.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, Local, TimeZone};

use crate::model::Task;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderKind {
    /// One of the task's reminder times was reached.
    Scheduled,
    /// The task is still not done, `nag_minutes` after the last reminder.
    Nag,
    /// A snooze ran out.
    SnoozeOver,
}

#[derive(Debug, Clone)]
pub struct Reminder {
    pub task: Task,
    pub kind: ReminderKind,
}

impl Reminder {
    pub fn body(&self) -> String {
        let lead = match self.kind {
            ReminderKind::Scheduled => "Time for this",
            ReminderKind::Nag => "Still not done",
            ReminderKind::SnoozeOver => "Snooze is over",
        };
        let mut body = lead.to_string();
        if !self.task.notes.trim().is_empty() {
            body = format!("{body}: {}", self.task.notes.trim());
        }
        if let Some(url) = &self.task.url {
            body = format!("{body}\n{url}");
        }
        body
    }
}

pub struct ReminderEngine {
    last_tick: DateTime<Local>,
    last_notified: HashMap<i64, DateTime<Local>>,
}

impl ReminderEngine {
    /// Starts counting from `now`: reminder times that already passed are not replayed
    /// (use [`ReminderEngine::pending`] for a "you have N things left" summary instead).
    pub fn new(now: DateTime<Local>) -> Self {
        Self { last_tick: now, last_notified: HashMap::new() }
    }

    /// Tasks scheduled today whose first reminder time has passed and that aren't done.
    /// Also marks them as just notified so nags count from now.
    pub fn pending(&mut self, tasks: &[Task], done_today: &HashSet<i64>, now: DateTime<Local>) -> Vec<Task> {
        let today = now.date_naive();
        let out: Vec<Task> = tasks
            .iter()
            .filter(|t| t.enabled && t.is_scheduled_on(today) && !done_today.contains(&t.id))
            .filter(|t| t.first_time().is_some_and(|ft| ft <= now.time()))
            .filter(|t| t.snoozed_until.is_none_or(|s| s <= now))
            .cloned()
            .collect();
        for t in &out {
            self.last_notified.insert(t.id, now);
        }
        out
    }

    pub fn tick(&mut self, tasks: &[Task], done_today: &HashSet<i64>, now: DateTime<Local>) -> Vec<Reminder> {
        let since = self.last_tick;
        self.last_tick = now;
        if now <= since {
            return Vec::new();
        }
        let today = now.date_naive();
        let mut out = Vec::new();

        for task in tasks {
            if !task.enabled || !task.is_scheduled_on(today) || done_today.contains(&task.id) {
                continue;
            }
            if let Some(until) = task.snoozed_until {
                if until > now {
                    continue;
                }
                if until > since {
                    out.push(Reminder { task: task.clone(), kind: ReminderKind::SnoozeOver });
                    continue;
                }
            }
            let hit_time = task.times.iter().any(|t| {
                let at = local_at(today, *t);
                at.is_some_and(|at| since < at && at <= now)
            });
            if hit_time {
                out.push(Reminder { task: task.clone(), kind: ReminderKind::Scheduled });
                continue;
            }
            let Some(nag) = task.nag_minutes else { continue };
            let Some(first) = task.first_time().and_then(|t| local_at(today, t)) else { continue };
            if first > now {
                continue;
            }
            let last = self.last_notified.get(&task.id).copied().filter(|l| *l >= first).unwrap_or(first);
            let last = task.snoozed_until.filter(|s| *s > last).unwrap_or(last);
            if now >= last + Duration::minutes(nag as i64) {
                out.push(Reminder { task: task.clone(), kind: ReminderKind::Nag });
            }
        }

        for r in &out {
            self.last_notified.insert(r.task.id, now);
        }
        out
    }
}

fn local_at(date: chrono::NaiveDate, t: chrono::NaiveTime) -> Option<DateTime<Local>> {
    Local.from_local_datetime(&date.and_time(t)).earliest()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveTime;

    fn task(id: i64, times: &[&str], nag: Option<u32>) -> Task {
        Task {
            id,
            title: format!("t{id}"),
            notes: String::new(),
            url: None,
            days: vec![],
            times: times.iter().map(|s| NaiveTime::parse_from_str(s, "%H:%M").unwrap()).collect(),
            nag_minutes: nag,
            enabled: true,
            snoozed_until: None,
            created_at: Local::now(),
        }
    }

    fn at(h: u32, m: u32) -> DateTime<Local> {
        let d = chrono::NaiveDate::from_ymd_opt(2026, 3, 10).unwrap();
        local_at(d, NaiveTime::from_hms_opt(h, m, 0).unwrap()).unwrap()
    }

    #[test]
    fn fires_once_at_scheduled_time() {
        let tasks = vec![task(1, &["10:00"], None)];
        let done = HashSet::new();
        let mut e = ReminderEngine::new(at(9, 59));
        assert!(e.tick(&tasks, &done, at(9, 59)).is_empty());
        let r = e.tick(&tasks, &done, at(10, 0));
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].kind, ReminderKind::Scheduled);
        assert!(e.tick(&tasks, &done, at(10, 1)).is_empty());
    }

    #[test]
    fn skips_done_tasks() {
        let tasks = vec![task(1, &["10:00"], Some(5))];
        let done = HashSet::from([1]);
        let mut e = ReminderEngine::new(at(9, 0));
        assert!(e.tick(&tasks, &done, at(11, 0)).is_empty());
    }

    #[test]
    fn nags_until_done() {
        let tasks = vec![task(1, &["10:00"], Some(30))];
        let mut done = HashSet::new();
        let mut e = ReminderEngine::new(at(9, 0));
        assert_eq!(e.tick(&tasks, &done, at(10, 0))[0].kind, ReminderKind::Scheduled);
        assert!(e.tick(&tasks, &done, at(10, 29)).is_empty());
        assert_eq!(e.tick(&tasks, &done, at(10, 30))[0].kind, ReminderKind::Nag);
        assert!(e.tick(&tasks, &done, at(10, 45)).is_empty());
        done.insert(1);
        assert!(e.tick(&tasks, &done, at(11, 0)).is_empty());
    }

    #[test]
    fn snooze_suppresses_then_fires() {
        let mut t = task(1, &["10:00"], Some(10));
        t.snoozed_until = Some(at(10, 20));
        let tasks = vec![t];
        let done = HashSet::new();
        let mut e = ReminderEngine::new(at(9, 0));
        assert!(e.tick(&tasks, &done, at(10, 0)).is_empty());
        assert!(e.tick(&tasks, &done, at(10, 15)).is_empty());
        assert_eq!(e.tick(&tasks, &done, at(10, 20))[0].kind, ReminderKind::SnoozeOver);
        assert!(e.tick(&tasks, &done, at(10, 25)).is_empty());
        assert_eq!(e.tick(&tasks, &done, at(10, 30))[0].kind, ReminderKind::Nag);
    }

    #[test]
    fn second_time_of_day_fires() {
        let tasks = vec![task(1, &["10:00", "18:00"], None)];
        let done = HashSet::new();
        let mut e = ReminderEngine::new(at(9, 0));
        assert_eq!(e.tick(&tasks, &done, at(10, 0)).len(), 1);
        assert!(e.tick(&tasks, &done, at(17, 0)).is_empty());
        assert_eq!(e.tick(&tasks, &done, at(18, 0)).len(), 1);
    }

    #[test]
    fn pending_lists_missed_tasks() {
        let tasks = vec![task(1, &["08:00"], None), task(2, &["20:00"], None)];
        let mut e = ReminderEngine::new(at(12, 0));
        let p = e.pending(&tasks, &HashSet::new(), at(12, 0));
        assert_eq!(p.iter().map(|t| t.id).collect::<Vec<_>>(), vec![1]);
    }
}
