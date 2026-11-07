// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Automatic updates.
//!
//! LabDesk looks at the public release page, downloads a newer installer in the background,
//! checks it against its published SHA-256 checksum, and installs it the next time the program
//! starts (or when the person presses INSTALL NOW). Nothing but the version request leaves this
//! computer, and no patient data is ever sent. The lab's data file is never touched by an update;
//! a backup is made first anyway.
use labdesk_core::update;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Public repository that holds only installers (no source code).
pub const UPDATE_REPO: &str = "mstudio005/labdesk-releases";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    UpToDate,
    /// A newer version has been downloaded and checked; it installs on the next start.
    Ready(String),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pending { pub version: String, pub file: PathBuf, pub sha256: String, pub attempts: u32 }

fn dir(data: &Path) -> PathBuf { data.join("updates") }
fn pending_file(data: &Path) -> PathBuf { dir(data).join("pending.txt") }
fn off_file(data: &Path) -> PathBuf { dir(data).join("auto-update-off") }
fn stamp_file(data: &Path) -> PathBuf { dir(data).join("last-check.txt") }

pub fn auto_enabled(data: &Path) -> bool { !off_file(data).exists() }
pub fn set_auto(data: &Path, on: bool) {
    let _ = std::fs::create_dir_all(dir(data));
    if on { let _ = std::fs::remove_file(off_file(data)); } else { let _ = std::fs::write(off_file(data), "off"); }
}

/// True if a check has not been made in the last 20 hours (so opening the program all day does not keep asking).
pub fn check_due(data: &Path) -> bool {
    let last = std::fs::read_to_string(stamp_file(data)).ok().and_then(|s| s.trim().parse::<i64>().ok()).unwrap_or(0);
    chrono::Utc::now().timestamp() - last > 20 * 3600
}
fn mark_checked(data: &Path) { let _ = std::fs::create_dir_all(dir(data)); let _ = std::fs::write(stamp_file(data), chrono::Utc::now().timestamp().to_string()); }

fn curl(args: &[&str]) -> Result<Vec<u8>, String> {
    let mut c = Command::new(if cfg!(windows) { "curl.exe" } else { "curl" });
    c.args(["-fsSL", "--proto", "=https", "--max-time", "600", "-A", &format!("LabDesk/{VERSION}")]).args(args).stdin(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    { use std::os::windows::process::CommandExt; c.creation_flags(0x0800_0000); }
    let out = c.output().map_err(|_| "This computer cannot reach the internet from LabDesk (curl is missing).".to_string())?;
    if out.status.success() { Ok(out.stdout) } else { Err("Could not reach the update server. Check the internet connection.".into()) }
}

fn write_pending(data: &Path, p: &Pending) {
    let _ = std::fs::write(pending_file(data), format!("{}\n{}\n{}\n{}\n", p.version, p.file.display(), p.sha256, p.attempts));
}

/// Download the newest installer if there is one. Blocking: run it on a worker thread.
pub fn check_and_download(data: &Path, current: &str, repo: &str) -> Outcome {
    let _ = std::fs::create_dir_all(dir(data));
    let api = format!("https://api.github.com/repos/{repo}/releases/latest");
    let json = match curl(&["-H", "Accept: application/vnd.github+json", &api]) { Ok(b) => String::from_utf8_lossy(&b).into_owned(), Err(e) => return Outcome::Failed(e) };
    mark_checked(data);
    let Some(rel) = update::pick_release(&json, repo, current) else { return Outcome::UpToDate };
    if let Some(p) = read_pending(data, current) { if p.version == rel.version { return Outcome::Ready(rel.version); } }
    let sha = match curl(&[&rel.sha256_url]).map(|b| String::from_utf8_lossy(&b).into_owned()).ok().and_then(|t| update::parse_sha256_file(&t)) {
        Some(s) => s, None => return Outcome::Failed("The update's checksum could not be read, so it was not installed.".into()),
    };
    let file = dir(data).join(&rel.installer_name);
    let part = dir(data).join(format!("{}.part", rel.installer_name));
    let _ = std::fs::remove_file(&part);
    if let Err(e) = curl(&["-o", &part.to_string_lossy(), &rel.installer_url]) { let _ = std::fs::remove_file(&part); return Outcome::Failed(e); }
    match update::sha256_file(&part) {
        Ok(h) if h == sha => {}
        _ => { let _ = std::fs::remove_file(&part); return Outcome::Failed("The downloaded update did not match its checksum, so it was thrown away.".into()); }
    }
    if std::fs::rename(&part, &file).is_err() { return Outcome::Failed("Could not save the update on this computer.".into()); }
    write_pending(data, &Pending { version: rel.version.clone(), file, sha256: sha, attempts: 0 });
    Outcome::Ready(rel.version)
}

/// The downloaded update that is waiting, if it is still newer than this program, the file is
/// intact and it has not already failed twice. Anything else is cleaned away.
pub fn read_pending(data: &Path, current: &str) -> Option<Pending> {
    let text = std::fs::read_to_string(pending_file(data)).ok()?;
    let mut l = text.lines();
    let p = Pending { version: l.next()?.into(), file: PathBuf::from(l.next()?), sha256: l.next()?.into(), attempts: l.next()?.parse().ok()? };
    let ok = update::is_newer(&p.version, current) && p.attempts < 2
        && p.file.parent() == Some(dir(data).as_path()) && update::sha256_file(&p.file).map(|h| h == p.sha256).unwrap_or(false);
    if ok { Some(p) } else { forget(data); None }
}

/// Remove downloaded installers and the waiting note.
pub fn forget(data: &Path) {
    if let Ok(rd) = std::fs::read_dir(dir(data)) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("LabDesk-Setup-") || n == "pending.txt" { let _ = std::fs::remove_file(e.path()); }
        }
    }
}

/// Start the installer quietly and let the caller close the program. The installer closes
/// LabDesk itself if it is still open and installs over the old version. With `relaunch` it opens
/// LabDesk again afterwards (start-up and INSTALL NOW); when the person just closed LabDesk it does not.
pub fn launch_installer(data: &Path, p: &Pending, relaunch: bool) -> Result<(), String> {
    #[cfg(not(windows))]
    { let _ = (data, p, relaunch); Err("Updates install on Windows only.".into()) }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        write_pending(data, &Pending { attempts: p.attempts + 1, ..p.clone() });
        Command::new(&p.file).args(["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART", "/CLOSEAPPLICATIONS", if relaunch { "/RELAUNCH=1" } else { "/RELAUNCH=0" }])
            .creation_flags(0x0000_0008 | 0x0800_0000).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn().map(|_| ()).map_err(|e| format!("Could not start the update: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_waiting_update_is_used_once_and_only_while_it_is_valid() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir(d.path())).unwrap();
        let f = dir(d.path()).join("LabDesk-Setup-9.9.9.exe");
        std::fs::write(&f, b"installer").unwrap();
        let sha = update::sha256_file(&f).unwrap();
        write_pending(d.path(), &Pending { version: "9.9.9".into(), file: f.clone(), sha256: sha.clone(), attempts: 0 });
        assert_eq!(read_pending(d.path(), "0.1.0").unwrap().version, "9.9.9");
        assert!(read_pending(d.path(), "9.9.9").is_none(), "already installed: forgotten");
        assert!(!f.exists());
        std::fs::write(&f, b"installer").unwrap();
        write_pending(d.path(), &Pending { version: "9.9.9".into(), file: f.clone(), sha256: sha.clone(), attempts: 0 });
        std::fs::write(&f, b"tampered").unwrap();
        assert!(read_pending(d.path(), "0.1.0").is_none(), "a changed file is never run");
        std::fs::write(&f, b"installer").unwrap();
        write_pending(d.path(), &Pending { version: "9.9.9".into(), file: f, sha256: sha, attempts: 2 });
        assert!(read_pending(d.path(), "0.1.0").is_none(), "gives up after two failed tries");
    }
    #[test]
    fn auto_update_can_be_switched_off() {
        let d = tempfile::tempdir().unwrap();
        assert!(auto_enabled(d.path()));
        set_auto(d.path(), false); assert!(!auto_enabled(d.path()));
        set_auto(d.path(), true); assert!(auto_enabled(d.path()));
        assert!(check_due(d.path())); mark_checked(d.path()); assert!(!check_due(d.path()));
    }
}
