use anyhow::Result;

/// Shows a desktop notification (Notification Center on macOS, D-Bus on Linux, toast on Windows).
pub fn send(title: &str, body: &str) -> Result<()> {
    notify_rust::Notification::new()
        .appname("Hormuz")
        .summary(title)
        .body(body)
        .show()?;
    Ok(())
}
