//! Shared core for Hormuz: recurring daily tasks, completions, streaks and reminder logic.

pub mod model;
pub mod reminder;
pub mod rpc;
pub mod seed;
pub mod store;

pub use model::{Agenda, AgendaItem, Task, TaskInput};
pub use store::{Store, default_db_path};

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Local};
    use serde_json::json;

    #[test]
    fn crud_done_and_streak() {
        let s = Store::open_in_memory().unwrap();
        let input: TaskInput =
            serde_json::from_value(json!({ "title": "Check PH", "times": ["10:00"], "url": " " })).unwrap();
        let t = s.add_task(&input).unwrap();
        assert_eq!(t.url, None);
        assert!(t.enabled);

        let today = Local::now().date_naive();
        assert_eq!(s.agenda(today, Local::now()).unwrap().total, 1);
        s.set_done(t.id, today, true).unwrap();
        s.set_done(t.id, today - Duration::days(1), true).unwrap();
        s.set_done(t.id, today - Duration::days(2), true).unwrap();
        let a = s.agenda(today, Local::now()).unwrap();
        assert_eq!(a.done, 1);
        assert_eq!(a.items[0].streak, 3);

        s.set_done(t.id, today, false).unwrap();
        assert_eq!(s.agenda(today, Local::now()).unwrap().items[0].streak, 2);

        s.delete_task(t.id).unwrap();
        assert!(s.is_empty().unwrap());
    }

    #[test]
    fn rpc_round_trip() {
        let s = Store::open_in_memory().unwrap();
        let added = rpc::dispatch(&s, "seed_defaults", json!(null)).unwrap();
        assert_eq!(added.as_array().unwrap().len(), 2);
        // Seeding twice doesn't duplicate.
        assert_eq!(rpc::dispatch(&s, "seed_defaults", json!(null)).unwrap().as_array().unwrap().len(), 0);
        let tasks = rpc::dispatch(&s, "list_tasks", json!(null)).unwrap();
        let id = tasks[0]["id"].as_i64().unwrap();
        assert_eq!(tasks[1]["times"], json!(["12:30", "19:00"]));
        rpc::dispatch(&s, "set_done", json!({ "id": id, "done": true })).unwrap();
        let agenda = rpc::dispatch(&s, "agenda", json!({})).unwrap();
        assert_eq!(agenda["done"], 1);
        assert!(rpc::dispatch(&s, "nope", json!({})).is_err());
        assert!(rpc::dispatch(&s, "add_task", json!({ "input": { "title": "", "times": [] } })).is_err());
    }

    #[test]
    fn day_parsing() {
        use model::{fmt_days, parse_days};
        assert_eq!(parse_days("mon,wed").unwrap(), vec![0, 2]);
        assert_eq!(parse_days("weekdays").unwrap(), vec![0, 1, 2, 3, 4]);
        assert_eq!(parse_days("weekdays,weekends").unwrap(), Vec::<u8>::new());
        assert!(parse_days("funday").is_err());
        assert_eq!(fmt_days(&[0, 1, 2, 3, 4]), "weekdays");
        assert_eq!(fmt_days(&[1, 3]), "Tue,Thu");
    }
}
