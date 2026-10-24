//! Installs the daemon as a per-user login service.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};

const LABEL: &str = "com.hormuz.daemon";

pub fn ping(port: u16) -> bool {
    let Ok(mut s) = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(500)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(1)));
    let req = "POST /rpc/version HTTP/1.1\r\nHost: 127.0.0.1\r\nX-Hormuz: 1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    buf.starts_with("HTTP/1.1 200")
}

pub fn open_url(url: &str) -> Result<()> {
    let cmd = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    Command::new(cmd).arg(url).status()?;
    Ok(())
}

fn exe() -> Result<String> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let s = exe.to_string_lossy().to_string();
    if s.contains("/target/debug/") {
        eprintln!("warning: installing a debug build from {s}.\n         `cargo install --path crates/hormuz` gives the service a stable path.");
    }
    Ok(s)
}

fn run(cmd: &mut Command) -> Result<()> {
    let out = cmd.output().with_context(|| format!("running {cmd:?}"))?;
    if !out.status.success() {
        bail!("{cmd:?} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn install(db: Option<&Path>, port: u16) -> Result<()> {
    let home = dirs::home();
    let plist = home.join("Library/LaunchAgents").join(format!("{LABEL}.plist"));
    let log = home.join("Library/Logs/hormuz.log");
    let env = db
        .map(|d| {
            format!(
                "<key>EnvironmentVariables</key><dict><key>HORMUZ_DB</key><string>{}</string></dict>",
                xml(&d.canonicalize().unwrap_or(d.to_path_buf()).to_string_lossy())
            )
        })
        .unwrap_or_default();
    let body = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array><string>{exe}</string><string>daemon</string><string>run</string><string>--port</string><string>{port}</string></array>
  {env}
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Interactive</string>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        exe = xml(&exe()?),
        log = xml(&log.to_string_lossy()),
    );
    std::fs::create_dir_all(plist.parent().unwrap())?;
    std::fs::write(&plist, body)?;
    let domain = format!("gui/{}", uid()?);
    let _ = Command::new("launchctl").args(["bootout", &format!("{domain}/{LABEL}")]).output();
    run(Command::new("launchctl").args(["bootstrap", &domain]).arg(&plist))?;
    println!("installed {}\nlogs: {}", plist.display(), log.display());
    println!("the daemon starts at every login; UI at http://127.0.0.1:{port}");
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn uninstall() -> Result<()> {
    let plist = dirs::home().join("Library/LaunchAgents").join(format!("{LABEL}.plist"));
    let _ = Command::new("launchctl").args(["bootout", &format!("gui/{}/{LABEL}", uid()?)]).output();
    if plist.exists() {
        std::fs::remove_file(&plist)?;
    }
    println!("removed {LABEL}");
    Ok(())
}

#[cfg(target_os = "macos")]
fn uid() -> Result<String> {
    let out = Command::new("id").arg("-u").output()?;
    Ok(String::from_utf8(out.stdout)?.trim().to_string())
}

#[cfg(target_os = "macos")]
fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(target_os = "linux")]
pub fn install(db: Option<&Path>, port: u16) -> Result<()> {
    let unit = dirs::home().join(".config/systemd/user/hormuz.service");
    let env = db
        .map(|d| format!("Environment=HORMUZ_DB={}\n", d.canonicalize().unwrap_or(d.to_path_buf()).display()))
        .unwrap_or_default();
    let body = format!(
        "[Unit]\nDescription=Hormuz daily task reminders\n\n[Service]\nExecStart={} daemon run --port {port}\n{env}Restart=on-failure\n\n[Install]\nWantedBy=default.target\n",
        exe()?
    );
    std::fs::create_dir_all(unit.parent().unwrap())?;
    std::fs::write(&unit, body)?;
    run(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
    run(Command::new("systemctl").args(["--user", "enable", "--now", "hormuz.service"]))?;
    println!("installed {}\nlogs: journalctl --user -u hormuz", unit.display());
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn uninstall() -> Result<()> {
    let _ = Command::new("systemctl").args(["--user", "disable", "--now", "hormuz.service"]).output();
    let unit = dirs::home().join(".config/systemd/user/hormuz.service");
    if unit.exists() {
        std::fs::remove_file(unit)?;
    }
    let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).output();
    println!("removed hormuz.service");
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn install(_db: Option<&Path>, _port: u16) -> Result<()> {
    bail!("service install isn't supported on this OS yet; run `hormuz daemon run` from your startup apps")
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn uninstall() -> Result<()> {
    bail!("service install isn't supported on this OS yet")
}

mod dirs {
    pub fn home() -> std::path::PathBuf {
        std::env::var_os("HOME").map(Into::into).unwrap_or_else(|| ".".into())
    }
}
