// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

// Hide the console window on Windows release builds.
#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]
use labdesk_core::{Actor, Role, Store};
use labdesk_desktop::{app, diag, updater, AppWindow};
use slint::ComponentHandle;
use std::path::PathBuf;

fn default_db() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("LabDesk").join("lab.db")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // labdesk [--db FILE] [--import CATALOGUE.json] [--demo]
    let mut args = std::env::args().skip(1);
    let (mut db, mut import, mut demo) = (std::env::var_os("LABDESK_DB").map(PathBuf::from).unwrap_or_else(default_db), None, false);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--db" => db = args.next().ok_or("--db needs a file path")?.into(),
            "--import" => import = Some(args.next().ok_or("--import needs a catalogue file")?),
            "--demo" => demo = true,
            other => return Err(format!("unknown option {other}").into()),
        }
    }
    let data_dir = db.parent().map(|d| d.to_path_buf()).filter(|d| !d.as_os_str().is_empty()).unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&data_dir)?;
    diag::install_panic_hook(data_dir.clone());
    let mut store = Store::open(&db)?;
    let admin = Actor::new("Setup", Role::Admin);
    if let Some(file) = import { let n = store.import_catalogue(&admin, &std::fs::read_to_string(file)?)?; eprintln!("Imported {n} tests"); }
    if demo && store.tests()?.is_empty() { store.import_catalogue(&admin, app::DEMO_CATALOGUE)?; }
    // An update that was downloaded and checked earlier installs now, before anyone starts working:
    // back up, start the installer quietly and leave. The installer opens LabDesk again when it is done.
    if !demo && std::env::var_os("LABDESK_DB").is_none() {
        if let Some(p) = updater::read_pending(&data_dir, updater::VERSION) {
            let _ = store.backup_now(&Actor::new("LabDesk (before update)", Role::Admin));
            if updater::launch_installer(&data_dir, &p, true).is_ok() { return Ok(()); }
        }
    }
    let ui = AppWindow::new()?;
    let opts = app::Options { data_dir, db_path: db.clone(), use_sessions: true, ..app::Options::new() };
    let _ctl = app::wire(&ui, store, opts);
    // open filling the screen like other programs do (the window buttons still restore it)
    ui.window().set_maximized(true);
    ui.run()?;
    Ok(())
}
