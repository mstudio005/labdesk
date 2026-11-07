// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Backups: a single `.ldbk` file holding a consistent copy of the whole lab database,
//! optionally locked with a backup password. Restoring checks the file first and keeps a safety copy.
//!
//! "Cloud backup" works through a folder that Google Drive, OneDrive, Dropbox (etc.) keep in sync:
//! every backup is also copied into that folder.
use crate::error::{check, Error, Result};
use crate::model::{Actor, Role};
use crate::store::{allow, audit, Store};
use crate::users::pbkdf2;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};

const MAGIC: &[u8; 4] = b"LDBK";
#[cfg(debug_assertions)]
const KDF_ITERS: u32 = 2_000;
#[cfg(not(debug_assertions))]
const KDF_ITERS: u32 = 200_000;
/// How many automatic backups are kept in each folder.
pub const KEEP: usize = 30;

/// Where backups go and how they are protected.
/// The password is kept only on this computer: it is removed from every backup copy, so a backup never contains its own key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct BackupSettings {
    /// Folder on this computer. Empty = not set up yet.
    pub folder: String,
    /// Optional second folder that a cloud app keeps in sync (Google Drive, OneDrive, Dropbox...).
    pub cloud_folder: String,
    /// Lock backups with `password`.
    pub encrypt: bool,
    pub password: String,
    /// How many backups to keep in each folder (older ones are removed).
    pub keep: usize,
}
impl Default for BackupSettings {
    fn default() -> Self { Self { folder: String::new(), cloud_folder: String::new(), encrypt: false, password: String::new(), keep: KEEP } }
}

#[derive(Debug, Clone)]
pub struct BackupInfo { pub path: PathBuf, pub name: String, pub size: u64, pub encrypted: bool, pub modified: String }

fn ist_stamp() -> String {
    let ist = chrono::FixedOffset::east_opt(5 * 3600 + 1800).expect("valid offset");
    chrono::Utc::now().with_timezone(&ist).format("%Y%m%d-%H%M%S").to_string()
}
fn io(e: std::io::Error) -> Error { Error::Rule(format!("Could not read or write the backup file: {e}")) }

fn seal(plain: &[u8], password: Option<&str>) -> Result<Vec<u8>> {
    let mut out = MAGIC.to_vec();
    out.push(1);
    match password.filter(|p| !p.is_empty()) {
        None => { out.push(0); out.extend_from_slice(plain); }
        Some(pw) => {
            let (mut salt, mut nonce) = ([0u8; 16], [0u8; 12]);
            getrandom::fill(&mut salt).and_then(|_| getrandom::fill(&mut nonce)).map_err(|e| Error::Rule(format!("The computer could not make a secure random number: {e}")))?;
            let key = pbkdf2(pw.as_bytes(), &salt, KDF_ITERS);
            let ct = ChaCha20Poly1305::new(Key::from_slice(&key)).encrypt(Nonce::from_slice(&nonce), plain).map_err(|_| Error::Rule("Could not lock the backup".into()))?;
            out.push(1); out.extend_from_slice(&KDF_ITERS.to_be_bytes()); out.extend_from_slice(&salt); out.extend_from_slice(&nonce); out.extend_from_slice(&ct);
        }
    }
    Ok(out)
}
fn is_raw_sqlite(bytes: &[u8]) -> bool { bytes.starts_with(b"SQLite format 3\0") }
fn header(bytes: &[u8]) -> Result<bool> {
    check(bytes.len() > 6 && &bytes[..4] == MAGIC && bytes[4] == 1, "This is not a LabDesk backup file")?;
    Ok(bytes[5] == 1)
}
fn open_seal(bytes: &[u8], password: Option<&str>) -> Result<Vec<u8>> {
    if is_raw_sqlite(bytes) { return Ok(bytes.to_vec()); }
    if !header(bytes)? { return Ok(bytes[6..].to_vec()); }
    let pw = password.filter(|p| !p.is_empty()).ok_or_else(|| Error::Rule("This backup is locked. Please type the backup password.".into()))?;
    check(bytes.len() > 6 + 4 + 16 + 12 + 16, "This backup file is damaged")?;
    let iters = u32::from_be_bytes(bytes[6..10].try_into().expect("4 bytes"));
    check((1..=5_000_000).contains(&iters), "This backup file is damaged")?;
    let (salt, nonce, ct) = (&bytes[10..26], &bytes[26..38], &bytes[38..]);
    let key = pbkdf2(pw.as_bytes(), salt, iters);
    ChaCha20Poly1305::new(Key::from_slice(&key)).decrypt(Nonce::from_slice(nonce), ct)
        .map_err(|_| Error::Rule("The backup password is not correct, or the file is damaged.".into()))
}

/// Look inside a database copy: is it whole, and is it a LabDesk database we understand?
fn validate(path: &Path) -> Result<()> {
    let bad = || Error::Rule("This file is not a usable LabDesk backup.".into());
    let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|_| bad())?;
    let ok: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0)).map_err(|_| bad())?;
    check(ok == "ok", "This backup is damaged and cannot be used.")?;
    let v: i64 = c.pragma_query_value(None, "user_version", |r| r.get(0)).map_err(|_| bad())?;
    check(v >= 1, "This is not a LabDesk backup.")?;
    check(v <= crate::store::SCHEMA_VERSION, "This backup was made by a newer LabDesk. Please update the app first.")?;
    for t in ["users", "orders", "patients", "tests"] {
        let n: Option<i64> = c.query_row("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?", [t], |r| r.get(0)).optional().map_err(|_| bad())?;
        check(n.is_some(), "This is not a LabDesk backup.")?;
    }
    Ok(())
}

/// Does this backup file need a password?
pub fn is_encrypted_backup(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 8];
    std::fs::File::open(path).ok().and_then(|mut f| f.read_exact(&mut head).ok()).is_some() && header(&head).unwrap_or(false)
}
pub fn backup_is_locked(path: &Path) -> Result<bool> { Ok(is_encrypted_backup(path)) }

fn today_ist() -> String { ist_stamp()[..8].to_string() }

impl Store {
    pub fn backup_settings(&self) -> Result<BackupSettings> {
        let v: Option<String> = self.conn.query_row("SELECT value FROM settings WHERE key='backup'", [], |r| r.get(0)).optional()?;
        Ok(match v { Some(v) => serde_json::from_str(&v)?, None => BackupSettings::default() })
    }
    pub fn save_backup_settings(&mut self, actor: &Actor, b: &BackupSettings) -> Result<()> {
        allow(actor, &[Role::Admin])?;
        for f in [&b.folder, &b.cloud_folder] {
            if !f.trim().is_empty() { check(Path::new(f.trim()).is_dir(), format!("The folder \"{}\" does not exist", f.trim()))?; }
        }
        if b.encrypt { check(b.password.chars().count() >= 6, "Please choose a backup password of at least 6 characters")?; }
        let clean = BackupSettings { folder: b.folder.trim().into(), cloud_folder: b.cloud_folder.trim().into(), encrypt: b.encrypt, password: b.password.clone(), keep: b.keep.clamp(3, 365) };
        self.conn.execute("INSERT INTO settings(key,value) VALUES('backup',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [serde_json::to_string(&clean)?])?;
        audit(&self.conn, actor, "backup_settings", "settings", 0, json!({ "locked": clean.encrypt, "cloud": !clean.cloud_folder.is_empty() }))?;
        Ok(())
    }
    /// When the last successful backup was made (ISO time).
    pub fn last_backup(&self) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT value FROM settings WHERE key='last_backup'", [], |r| r.get(0)).optional()?)
    }

    /// A consistent copy of the database as backup-file bytes. The backup password is removed from the copy.
    fn sealed_copy(&self, scratch: &Path, password: Option<&str>) -> Result<Vec<u8>> {
        let tmp = scratch.join(format!(".labdesk-tmp-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        self.conn.execute("VACUUM INTO ?1", [tmp.to_string_lossy().as_ref()])?;
        let strip = Connection::open(&tmp).and_then(|c| c.execute("UPDATE settings SET value=json_remove(value,'$.password') WHERE key IN ('backup','email')", []).map(|_| ()));
        let plain = strip.map_err(Error::from).and_then(|_| std::fs::read(&tmp).map_err(io));
        let _ = std::fs::remove_file(&tmp);
        seal(&plain?, password)
    }

    /// Write one backup into every chosen folder. Returns (files written, problems). One folder failing
    /// (a USB drive unplugged, say) does not stop the other from being written.
    fn write_backups(&mut self, who: &Actor) -> Result<(Vec<PathBuf>, Vec<String>)> {
        let cfg = self.backup_settings()?;
        let targets: Vec<PathBuf> = [&cfg.folder, &cfg.cloud_folder].into_iter().filter(|f| !f.trim().is_empty()).map(PathBuf::from).collect();
        check(!targets.is_empty(), "Choose a backup folder first (Settings, Data & backup)")?;
        let pw = if cfg.encrypt { check(!cfg.password.is_empty(), "The backup password is missing. Please enter it in Data & backup.")?; Some(cfg.password.as_str()) } else { None };
        let first = targets.iter().find(|t| t.is_dir()).cloned().unwrap_or_else(std::env::temp_dir);
        let sealed = self.sealed_copy(&first, pw)?;
        let (mut done, mut failed) = (Vec::new(), Vec::new());
        let stamp = ist_stamp();
        for dir in &targets {
            if !dir.is_dir() { failed.push(format!("The folder {} is not available. Is the drive plugged in?", dir.display())); continue; }
            let mut name = format!("labdesk-backup-{stamp}.ldbk");
            let mut n = 1;
            while dir.join(&name).exists() { n += 1; name = format!("labdesk-backup-{stamp}-{n}.ldbk"); }
            let part = dir.join(format!("{name}.part"));
            match std::fs::write(&part, &sealed).and_then(|_| std::fs::rename(&part, dir.join(&name))) {
                Ok(()) => { done.push(dir.join(&name)); prune(dir, cfg.keep); }
                Err(e) => { let _ = std::fs::remove_file(&part); failed.push(format!("Could not write the backup to {}: {e}", dir.display())); }
            }
        }
        if !done.is_empty() {
            self.conn.execute("INSERT INTO settings(key,value) VALUES('last_backup',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [crate::store::now()])?;
        }
        let _ = audit(&self.conn, who, "backup", "backup", 0, json!({ "files": done.len(), "problems": failed.len() }));
        Ok((done, failed))
    }
    /// Back up now to the chosen folder and the cloud-synced folder.
    pub fn backup_now(&mut self, actor: &Actor) -> Result<(Vec<PathBuf>, Vec<String>)> {
        allow(actor, &[Role::Admin])?;
        self.write_backups(actor)
    }
    /// Once a day: does nothing if no folder is set up or a backup was already made today (lab day, India time).
    pub fn auto_backup(&mut self) -> Result<Option<(Vec<PathBuf>, Vec<String>)>> {
        let cfg = self.backup_settings()?;
        if cfg.folder.trim().is_empty() && cfg.cloud_folder.trim().is_empty() { return Ok(None); }
        if cfg.encrypt && cfg.password.is_empty() { return Ok(None); }
        if let Some(t) = self.last_backup()? {
            let ist = chrono::FixedOffset::east_opt(5 * 3600 + 1800).expect("valid offset");
            let day = chrono::DateTime::parse_from_rfc3339(&t).map(|d| d.with_timezone(&ist).format("%Y%m%d").to_string()).unwrap_or_default();
            if day == today_ist() { return Ok(None); }
        }
        self.write_backups(&Actor::new("LabDesk (automatic)", Role::Admin)).map(Some)
    }

    /// First start on a new computer: load a backup into the empty database. Refused once any account exists.
    pub fn restore_on_first_run(&mut self, file: &Path, password: Option<&str>) -> Result<()> {
        check(!self.has_users()?, "Restoring from the first screen is only possible before anyone has been added.")?;
        self.restore_inner(&Actor::new("first start", Role::Admin), file, password, None).map(|_| ())
    }
    /// Replace everything with the contents of a backup. Admin only. A safety copy of the current data is
    /// written into `safety_dir` first (returned), so a wrong choice can be undone.
    pub fn restore_from(&mut self, actor: &Actor, file: &Path, password: Option<&str>, safety_dir: &Path) -> Result<PathBuf> {
        allow(actor, &[Role::Admin])?;
        self.restore_inner(actor, file, password, Some(safety_dir))?.ok_or_else(|| Error::Rule("Could not save a safety copy first".into()))
    }
    fn restore_inner(&mut self, who: &Actor, file: &Path, password: Option<&str>, safety_dir: Option<&Path>) -> Result<Option<PathBuf>> {
        let bytes = std::fs::read(file).map_err(io)?;
        let plain = open_seal(&bytes, password)?;
        let tmp = std::env::temp_dir().join(format!("labdesk-restore-{}.db", std::process::id()));
        std::fs::write(&tmp, &plain).map_err(io)?;
        let result = (|| -> Result<Option<PathBuf>> {
            validate(&tmp)?;
            let mut kept = None;
            if let Some(dir) = safety_dir {
                std::fs::create_dir_all(dir).map_err(io)?;
                let sealed = self.sealed_copy(dir, None)?;
                let p = dir.join(format!("labdesk-before-restore-{}.ldbk", ist_stamp()));
                std::fs::write(&p, sealed).map_err(io)?;
                kept = Some(p);
            }
            let src = Connection::open_with_flags(&tmp, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            {
                let bk = rusqlite::backup::Backup::new(&src, &mut self.conn)?;
                bk.run_to_completion(256, std::time::Duration::from_millis(0), None)?;
            }
            drop(src);
            crate::store::migrate(&self.conn)?;
            // nobody stays signed in from before the restore
            self.conn.execute("DELETE FROM sessions", [])?;
            self.conn.execute("UPDATE login_guard SET fails=0,locked_until=NULL", [])?;
            Ok(kept)
        })();
        let _ = std::fs::remove_file(&tmp);
        let kept = result?;
        let _ = audit(&self.conn, who, "restore_backup", "backup", 0, json!({ "file": file.file_name().map(|n| n.to_string_lossy().to_string()) }));
        Ok(kept)
    }
}

/// Backup files in a folder, newest first.
pub fn list_backups(dir: &Path) -> Result<Vec<BackupInfo>> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return Ok(out) };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !(name.ends_with(".ldbk") || name.ends_with(".db")) || !name.starts_with("labdesk-backup") { continue; }
        let Ok(md) = e.metadata() else { continue };
        let modified = md.modified().ok().map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)).unwrap_or_default();
        out.push(BackupInfo { encrypted: is_encrypted_backup(&e.path()), path: e.path(), name, size: md.len(), modified });
    }
    out.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(out)
}
fn prune(dir: &Path, keep: usize) {
    if let Ok(list) = list_backups(dir) {
        for old in list.into_iter().skip(keep) { let _ = std::fs::remove_file(old.path); }
    }
}
