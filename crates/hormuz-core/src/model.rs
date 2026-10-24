use chrono::{DateTime, Datelike, Local, NaiveDate, NaiveTime};
use serde::{Deserialize, Serialize};

/// A recurring daily habit / chore, e.g. "Check Product Hunt and upvote a few launches".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub notes: String,
    pub url: Option<String>,
    /// Weekdays the task is due on, 0 = Monday .. 6 = Sunday. Empty means every day.
    pub days: Vec<u8>,
    /// Times of day to send a reminder, sorted.
    #[serde(with = "hm_list")]
    pub times: Vec<NaiveTime>,
    /// Re-remind every N minutes after the first reminder until the task is done.
    pub nag_minutes: Option<u32>,
    pub enabled: bool,
    pub snoozed_until: Option<DateTime<Local>>,
    pub created_at: DateTime<Local>,
}

/// The editable part of a task, used for create and update.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskInput {
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub days: Vec<u8>,
    #[serde(with = "hm_list")]
    pub times: Vec<NaiveTime>,
    #[serde(default)]
    pub nag_minutes: Option<u32>,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

impl Task {
    pub fn is_scheduled_on(&self, date: NaiveDate) -> bool {
        let wd = date.weekday().num_days_from_monday() as u8;
        self.days.is_empty() || self.days.contains(&wd)
    }

    pub fn first_time(&self) -> Option<NaiveTime> {
        self.times.first().copied()
    }
}

/// One row of "what do I have to do today".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgendaItem {
    pub task: Task,
    pub done: bool,
    pub done_at: Option<DateTime<Local>>,
    /// Consecutive scheduled days completed, ending today (or yesterday if today isn't done yet).
    pub streak: u32,
    /// Next reminder time later today, "HH:MM".
    pub next_time: Option<String>,
    /// The first reminder time has passed and the task is not done.
    pub overdue: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agenda {
    pub date: NaiveDate,
    pub items: Vec<AgendaItem>,
    pub done: usize,
    pub total: usize,
}

pub fn parse_hm(s: &str) -> Option<NaiveTime> {
    NaiveTime::parse_from_str(s.trim(), "%H:%M")
        .or_else(|_| NaiveTime::parse_from_str(s.trim(), "%H:%M:%S"))
        .ok()
}

pub fn fmt_hm(t: NaiveTime) -> String {
    t.format("%H:%M").to_string()
}

/// Parses "mon,tue" / "weekdays" / "weekends" / "daily" into weekday indexes.
pub fn parse_days(s: &str) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for part in s.split(',').map(|p| p.trim().to_ascii_lowercase()) {
        match part.as_str() {
            "" | "daily" | "everyday" | "all" => {}
            "weekdays" => out.extend(0..5),
            "weekends" => out.extend(5..7),
            p => {
                let idx = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
                    .iter()
                    .position(|d| p.starts_with(d))
                    .ok_or_else(|| format!("unknown day '{p}'"))?;
                out.push(idx as u8);
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    if out.len() == 7 {
        out.clear();
    }
    Ok(out)
}

pub fn fmt_days(days: &[u8]) -> String {
    const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    match days {
        [] => "daily".into(),
        [0, 1, 2, 3, 4] => "weekdays".into(),
        [5, 6] => "weekends".into(),
        d => d.iter().filter_map(|&i| NAMES.get(i as usize)).copied().collect::<Vec<_>>().join(","),
    }
}

/// Serializes `Vec<NaiveTime>` as `["09:00", "18:30"]`.
pub(crate) mod hm_list {
    use chrono::NaiveTime;
    use serde::{Deserialize, Deserializer, Serializer, de::Error, ser::SerializeSeq};

    pub fn serialize<S: Serializer>(v: &[NaiveTime], s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(v.len()))?;
        for t in v {
            seq.serialize_element(&super::fmt_hm(*t))?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<NaiveTime>, D::Error> {
        let raw = Vec::<String>::deserialize(d)?;
        let mut out = raw
            .iter()
            .map(|s| super::parse_hm(s).ok_or_else(|| D::Error::custom(format!("bad time '{s}', expected HH:MM"))))
            .collect::<Result<Vec<_>, _>>()?;
        out.sort();
        out.dedup();
        Ok(out)
    }
}
