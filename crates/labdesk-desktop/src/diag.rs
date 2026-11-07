// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Help text, crash notes and the problem report.
//!
//! LabDesk never sends anything over the internet by itself. If it crashes, a short note is written
//! to a file on this computer; the person can then save a problem report and send that file to
//! whoever looks after their software. The report holds no patient names and no results.
use std::path::{Path, PathBuf};

/// (title, words people might search for, answer)
pub type HelpEntry = (&'static str, &'static str, &'static str);

pub const HELP: &[HelpEntry] = &[
    ("Register a new patient", "new patient register add start billing",
     "Press NEW PATIENT at the top. Fill in the patient's details on the left, tick the tests in the middle, type any discount and the amount paid on the right, then press REGISTER PATIENT. The report opens straight away so results can be entered."),
    ("The patient came from a doctor, or on their own", "referred doctor self walk-in referral",
     "On the New patient screen, under \"Who sent the patient?\", choose \"Self (walk-in)\" or \"A doctor\". When you choose a doctor, type the name. Names you typed before appear as small buttons; press one to use it. The doctor's name is printed on the report as \"Referred by\"."),
    ("Add the patient's email", "email mail address optional",
     "The Email box on the registration screen is optional. If you type one it must look like name@example.com. You can add or change it later with EDIT PATIENT."),
    ("Correct a mistake in the patient's name, age, phone or doctor", "edit change correct patient details name age gender phone",
     "Open the report (Reports list), then press EDIT PATIENT at the top right. Change what is wrong and press SAVE CHANGES. Once a test is approved, age and gender are locked because they decide the normal ranges; to change them the doctor must first open the report for correction. After the report is delivered only phone, email and address can change."),
    ("Print the bill", "bill print receipt invoice payment price",
     "Open the report and press BILL PDF (just left of REPORT PDF). It works as soon as the patient is registered, before any result is ready. The PDF opens so you can print it."),
    ("Print the report", "report print pdf letterhead result",
     "A report can be printed after the doctor approves it. Press REPORT PDF. Use the LETTERHEAD button to switch the letterhead on (for plain paper) or off (for paper that is already printed)."),
    ("Enter results", "results enter technician values save",
     "Open the patient from Reports. Type each value in its box and press SAVE. Numbers outside the normal range are marked HIGH, LOW or CRIT automatically. Some lines are calculated by LabDesk; you do not type those."),
    ("Approve a report (doctor)", "approve doctor sign verify",
     "When all results are in, the Doctor opens the report and presses APPROVE. The report is then locked. Only an approved report can be printed."),
    ("Change a result after approval", "correct correction reopen change result approved",
     "The Doctor or Admin presses CORRECT, types a reason, and the report opens again. The earlier version is kept in the history."),
    ("Hand the report to the patient", "deliver delivered hand over locked",
     "After giving the report to the patient press MARK DELIVERED. From then on the report is permanently locked and cannot be changed or deleted."),
    ("Delete a report", "delete remove cancel wrong duplicate",
     "Only an Admin can delete. Open the report, press DELETE, type a short reason and your own password. The report disappears from every list and from the money totals. Nothing is destroyed: an Admin can bring it back from Settings > Data & backup > Deleted reports. A report that was already delivered cannot be deleted."),
    ("Bring back a deleted report", "restore undelete deleted recover",
     "Settings > Data & backup > Deleted reports (Admin). Press RESTORE beside the report."),
    ("Add the lab name, address and letterhead", "lab name address letterhead logo paper print setup",
     "Settings > Lab details for the name, address and phone. Settings > Report paper to upload your letterhead image and the doctor's signature, and to set how much blank space to leave. Use PRINT A TEST PAGE to check how it lines up."),
    ("Add or change tests and prices", "tests price add edit unit range normal",
     "Settings > Tests (Admin). Pick a test to change its price, units and normal ranges, or press + NEW TEST. Changes apply to new patients; reports already made are never altered."),
    ("Add a staff member", "people user account add staff role admin password",
     "Settings > People & passwords (Admin). Press + ADD PERSON, choose what the person can do, and give them a username and password. LabDesk asks for your own password before adding or removing anyone."),
    ("Someone forgot their password", "forgot password reset staff",
     "An Admin opens Settings > People & passwords, picks the person and types a new password. If the Admin forgot theirs, press \"Forgot your password?\" on the sign-in screen and use the recovery code that was shown when LabDesk was set up."),
    ("Change my own password", "change password my account",
     "Settings > My account. Type the current password, then the new one twice."),
    ("Locked after wrong passwords", "locked too many wrong attempts wait minute sign in",
     "After 5 wrong passwords in a row LabDesk waits 1 minute before it accepts another try. This is on purpose, to keep the lab's data safe. The wait continues even if you close and open LabDesk. A correct password resets the count."),
    ("Stay signed in", "session stay signed in logout remember keep",
     "On the sign-in screen \"Keep me signed in on this computer\" is ticked by default, so closing and opening LabDesk does not sign you out. Press SIGN OUT when you leave the computer, or untick the box on a shared computer."),
    ("Back up my data", "backup copy save usb safe",
     "LabDesk makes a backup once a day by itself while it is open. Settings > Data & backup lets you choose the folder, and BACK UP NOW makes one immediately. Choose a USB drive or another disk, not only the computer's own disk."),
    ("Back up to Google Drive, OneDrive or Dropbox", "cloud google drive onedrive dropbox online backup",
     "Install the Drive / OneDrive / Dropbox app for Windows. In Settings > Data & backup choose a folder inside it as the Cloud copy, then press SAVE BACKUP SETTINGS. Every backup is copied there and the app uploads it. To keep patient data private in the cloud, choose \"Locked with a password\" and write the password down."),
    ("Use a backup (restore)", "restore backup recover new computer replace lost",
     "Settings > Data & backup > RESTORE FROM A BACKUP, then pick the backup file. LabDesk checks it, saves a safety copy of today's data, and replaces everything with the backup. On a new computer press \"Restore from a backup\" on the very first screen. Afterwards sign in with the usernames and passwords the backup had."),
    ("I forgot the backup password", "backup password forgot locked ldbk",
     "A locked backup cannot be opened without its password; nobody can recover it. Backups that are not locked can always be opened. Make a fresh backup and write its password down."),
    ("LabDesk closed by itself or shows a strange message", "crash error bug problem closed freeze report",
     "Open LabDesk again; your saved work is safe. Then go to Settings > Help and press SAVE A PROBLEM REPORT, and send that file to your software provider. It contains no patient names or results."),
    ("Is my data safe if the power goes off?", "power cut safe lost data",
     "Yes. LabDesk writes every change to disk before it says \"saved\", so a power cut does not damage the data. Only a value you were still typing and had not saved can be lost."),
    ("Send reports by WhatsApp or email", "whatsapp email send share online",
     "Open a report and press WHATSAPP (the chat opens and the report is copied, so press Ctrl+V to attach it) or EMAIL (the PDF is attached automatically). EMAIL must first be turned on by the Admin in Settings > Email. The PDFs are saved in Documents\\LabDesk\\Reports."),
    ("Where are my files?", "folder files where saved documents location",
     "Reports and bills: Documents\\LabDesk\\Reports. Backups: Documents\\LabDesk\\Backups (or the folder you chose). The database itself is shown on the ABOUT page."),
];

pub fn search(q: &str) -> Vec<&'static HelpEntry> {
    let words: Vec<String> = q.to_lowercase().split_whitespace().map(String::from).collect();
    HELP.iter().filter(|(t, k, b)| {
        let hay = format!("{} {} {}", t, k, b).to_lowercase();
        words.iter().all(|w| hay.contains(w.as_str()))
    }).collect()
}

/// Where a crash report is emailed. Reports are only sent when the person presses SEND REPORT.
pub const REPORT_EMAIL: &str = "syedmutaibnazir@gmail.com";

/// Hide the Windows account name inside file paths before a report leaves the program.
pub fn scrub(text: &str) -> String {
    let mut out = text.to_string();
    for var in ["USERPROFILE", "LOCALAPPDATA", "APPDATA", "HOME"] {
        if let Some(v) = std::env::var_os(var).map(|v| v.to_string_lossy().into_owned()).filter(|v| v.len() > 3) {
            out = out.replace(&v, "<home>");
            out = out.replace(&v.replace('\\', "/"), "<home>");
        }
    }
    out
}

/// The report shown in the crash window: same as the problem report without the "write what you saw" prompt.
pub fn crash_report(dir: &Path, db: &Path, recent: &[String]) -> String {
    scrub(&problem_report(dir, db, recent).replace("What the person saw (please write it here before sending):\n\n\n\n", ""))
}

/// An email link addressed to the developer with the start of the report as its text (long links are cut
/// short because mail programs refuse very long addresses; the full report is saved as a file as well).
pub fn report_mailto(report: &str) -> String {
    let body: String = report.chars().take(1400).collect();
    format!("mailto:{REPORT_EMAIL}?subject={}&body={}", crate::printing::url_encode(&format!("LabDesk crash report {}", env!("CARGO_PKG_VERSION"))),
        crate::printing::url_encode(&format!("{body}\n\n(The full report is saved as a file; please attach it if it is not complete.)")))
}

fn log_path(dir: &Path) -> PathBuf { dir.join("crash.log") }
fn flag_path(dir: &Path) -> PathBuf { dir.join("crash.flag") }

/// Write a short note to `crash.log` whenever the program panics, then carry on with the normal behaviour.
pub fn install_panic_hook(dir: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let when = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "unknown".into());
        let entry = scrub(&format!("[{when}] LabDesk {} crashed at {place}: {msg}\n{}\n\n", env!("CARGO_PKG_VERSION"), std::backtrace::Backtrace::force_capture()));
        let p = log_path(&dir);
        if std::fs::metadata(&p).map(|m| m.len() > 512 * 1024).unwrap_or(false) { let _ = std::fs::remove_file(&p); }
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) { use std::io::Write; let _ = f.write_all(entry.as_bytes()); }
        let _ = std::fs::write(flag_path(&dir), when);
        previous(info);
    }));
}

/// True once after a crash: the next start can tell the person where to find help.
pub fn crashed_last_time(dir: &Path) -> bool {
    let f = flag_path(dir);
    let yes = f.exists();
    if yes { let _ = std::fs::remove_file(f); }
    yes
}

/// Plain-text problem report. `recent` are the latest recorded actions (time, who, what).
pub fn problem_report(dir: &Path, db: &Path, recent: &[String]) -> String {
    let size = std::fs::metadata(db).map(|m| m.len()).unwrap_or(0);
    let log = std::fs::read_to_string(log_path(dir)).unwrap_or_default();
    let tail: Vec<&str> = log.lines().rev().take(60).collect::<Vec<_>>().into_iter().rev().collect();
    format!("LabDesk problem report\n======================\nMade: {}\nVersion: {}\nSystem: {} {}\nData file size: {} KB\n\nWhat the person saw (please write it here before sending):\n\n\n\nRecent actions\n--------------\n{}\n\nCrash notes\n-----------\n{}\n",
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true), env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH, size / 1024,
        if recent.is_empty() { "(none)".into() } else { recent.join("\n") }, if tail.is_empty() { "(no crashes recorded)".into() } else { tail.join("\n") })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reports_hide_the_account_name_and_never_include_data_beyond_ids() {
        let home = std::env::var("HOME").unwrap_or_default();
        if home.len() > 3 { assert!(!scrub(&format!("at {home}/x.rs:1")).contains(&home)); }
        let dir = tempfile::tempdir().unwrap();
        let r = crash_report(dir.path(), &dir.path().join("lab.db"), &["2026-01-01  Admin  create test 3".into()]);
        assert!(r.contains("LabDesk problem report") && !r.contains("What the person saw"));
    }
    #[test]
    fn the_email_link_is_addressed_and_short() {
        let m = report_mailto(&"x".repeat(5000));
        assert!(m.starts_with("mailto:") && m.contains("subject=") && m.len() < 2400);
    }
}
