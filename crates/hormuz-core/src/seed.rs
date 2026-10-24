use anyhow::Result;

use crate::model::{Task, TaskInput, parse_hm};
use crate::store::Store;

/// Starter routine: daily community engagement. Edit or delete freely.
pub fn defaults() -> Vec<TaskInput> {
    let t = |s: &str| parse_hm(s).unwrap();
    vec![
        TaskInput {
            title: "Product Hunt: browse today's launches, upvote a few".into(),
            notes: "Upvote 3–5 products you'd actually use and leave one genuine comment.".into(),
            url: Some("https://www.producthunt.com".into()),
            days: vec![],
            times: vec![t("10:00")],
            nag_minutes: Some(60),
            enabled: true,
        },
        TaskInput {
            title: "Reddit: add something useful to a thread".into(),
            notes: "Answer a question or share experience in a subreddit you know well. Helpful comments earn karma."
                .into(),
            url: Some("https://www.reddit.com".into()),
            days: vec![],
            times: vec![t("12:30"), t("19:00")],
            nag_minutes: Some(90),
            enabled: true,
        },
    ]
}

/// Adds the starter tasks whose titles don't exist yet. Returns what was added.
pub fn seed(store: &Store) -> Result<Vec<Task>> {
    let existing: Vec<String> = store.list_tasks()?.into_iter().map(|t| t.title).collect();
    let mut added = Vec::new();
    for input in defaults() {
        if !existing.contains(&input.title) {
            added.push(store.add_task(&input)?);
        }
    }
    Ok(added)
}
