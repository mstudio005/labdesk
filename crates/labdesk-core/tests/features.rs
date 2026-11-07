// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

use labdesk_core::*;

const CAT: &str = r#"{"tests":[{"code":"CBC","name":"Complete Blood Count","department":"HAEMATOLOGY","sample":"EDTA","price_paise":30000,"config":{"parameters":[
   {"id":"hb","name":"Hb","field":"Numeric","unit":"g/dL","required":true,"low":13,"high":17}]}}]}"#;
fn admin() -> Actor { Actor::new("Dr Admin", Role::Admin) }
fn desk() -> Actor { Actor::new("Front", Role::Reception) }
fn path() -> Actor { Actor::new("Dr Path", Role::Pathologist) }

fn lab() -> (Store, Order) {
    let mut s = Store::open_in_memory().unwrap();
    s.import_catalogue(&admin(), CAT).unwrap();
    let t = s.tests().unwrap()[0].id;
    let o = s.register(&desk(), NewOrder {
        patient: NewPatient { first_name: "Ali".into(), last_name: "Khan".into(), age: 40.0, age_type: "Year".into(), gender: "Male".into(), address: "Srinagar".into(), email: "ali@example.com".into(), ..Default::default() },
        items: vec![OrderItem { test_id: t, price_paise: None, discount_paise: 0 }], referral_doctor: "Dr. Rashid".into(),
        payments: vec![(10000, "Cash".into(), "".into())], ..Default::default() }).unwrap();
    (s, o)
}
fn approve(s: &mut Store, o: &Order) -> Order {
    let t = &o.tests[0];
    let o = s.save_results(&admin(), o.id, o.version, &[ResultInput { order_test_id: t.id, parameter_id: "hb".into(), value: "14".into(), comment: None, manual: false }], None).unwrap();
    s.approve(&path(), o.id, o.version, None).unwrap()
}

#[test]
fn referral_doctor_and_email_on_registration() {
    let (mut s, o) = lab();
    assert_eq!(o.referral_name, "Dr. Rashid");
    assert_eq!(o.patient.email, "ali@example.com");
    assert_eq!(s.referral_names("rash", 10).unwrap(), vec!["Dr. Rashid"]);
    let t = s.tests().unwrap()[0].id;
    let mk = |doc: &str, email: &str| NewOrder {
        patient: NewPatient { first_name: "B".into(), age: 3.0, age_type: "Year".into(), gender: "Female".into(), address: "x".into(), email: email.into(), ..Default::default() },
        items: vec![OrderItem { test_id: t, price_paise: None, discount_paise: 0 }], referral_doctor: doc.into(), ..Default::default() };
    // same doctor typed differently is not duplicated; empty means Self
    assert_eq!(s.register(&desk(), mk("  dr.  RASHID ", "")).unwrap().referral_name, "Dr. Rashid");
    assert_eq!(s.referral_names("", 10).unwrap().len(), 1);
    assert_eq!(s.register(&desk(), mk("", "")).unwrap().referral_name, "");
    assert!(s.register(&desk(), mk("", "not-an-email")).unwrap_err().to_string().contains("email"));
}

#[test]
fn edit_patient_rules() {
    let (mut s, o) = lab();
    let mut p = NewPatient { designation: o.patient.designation.clone(), first_name: "Ali".into(), last_name: "Khan".into(), age: 41.0, age_type: "Year".into(), gender: "Male".into(), phone: "99".into(), email: "".into(), address: "Srinagar".into() };
    let e = s.update_patient(&desk(), o.id, &p, &Referral::SelfReferred).unwrap();
    assert_eq!((e.patient.age, e.referral_name.as_str(), e.patient.phone.as_str()), (41.0, "", "99"));
    assert!(matches!(s.update_patient(&Actor::new("T", Role::Technician), o.id, &p, &Referral::SelfReferred), Err(Error::Forbidden(_))));
    p.first_name = "  ".into();
    assert!(s.update_patient(&desk(), o.id, &p, &Referral::SelfReferred).is_err());
    p.first_name = "Alí".into();
    s.update_patient(&desk(), o.id, &p, &Referral::Doctor("Dr. New".into())).unwrap();
    assert_eq!(s.order(o.id).unwrap().referral_name, "Dr. New");
    // after approval: name may change, age/gender may not
    let cur = s.order(o.id).unwrap(); let o = approve(&mut s, &cur);
    p.first_name = "Ali".into();
    s.update_patient(&desk(), o.id, &p, &Referral::Doctor("Dr. New".into())).unwrap();
    p.gender = "Female".into();
    assert!(s.update_patient(&desk(), o.id, &p, &Referral::Doctor("Dr. New".into())).unwrap_err().to_string().contains("ranges"));
    p.gender = "Male".into();
    // delivered: only contact details
    let o = s.mark_delivered(&desk(), o.id, s.order(o.id).unwrap().version).unwrap();
    p.phone = "11".into(); p.address = "Anantnag".into();
    s.update_patient(&desk(), o.id, &p, &Referral::Doctor("Dr. New".into())).unwrap();
    p.first_name = "Other".into();
    assert!(s.update_patient(&desk(), o.id, &p, &Referral::Doctor("Dr. New".into())).unwrap_err().to_string().contains("locked"));
}

#[test]
fn delete_and_restore_report() {
    let (mut s, o) = lab();
    assert_eq!(s.stats().unwrap().due_paise, 20000);
    assert!(matches!(s.delete_order(&desk(), o.id, "mistake"), Err(Error::Forbidden(_))));
    assert!(s.delete_order(&admin(), o.id, "x").is_err(), "reason needed");
    s.delete_order(&admin(), o.id, "entered twice").unwrap();
    assert!(s.orders("", 50).unwrap().is_empty());
    assert_eq!((s.stats().unwrap().due_paise, s.stats().unwrap().patients_today), (0, 0));
    let d = s.deleted_orders().unwrap();
    assert_eq!((d.len(), d[0].reason.as_str(), d[0].deleted_by.as_str()), (1, "entered twice", "Dr Admin"));
    assert!(s.save_results(&admin(), o.id, o.version, &[], None).is_err(), "deleted reports cannot change");
    assert!(s.bill_pdf(&desk(), o.id, true).is_err());
    s.restore_order(&admin(), o.id).unwrap();
    assert_eq!(s.orders("", 50).unwrap().len(), 1);
    // delivered reports can never be deleted
    let cur = s.order(o.id).unwrap(); let o = approve(&mut s, &cur);
    let o = s.mark_delivered(&desk(), o.id, o.version).unwrap();
    assert!(s.delete_order(&admin(), o.id, "too late").unwrap_err().to_string().contains("handed"));
}

#[test]
fn bill_pdf_renders() {
    let (mut s, o) = lab();
    let b = s.bill_pdf(&desk(), o.id, true).unwrap();
    assert!(b.pdf.starts_with(b"%PDF") && b.pages == 1);
    assert_eq!(report::rupees(1234567), "₹12,345.67");
    assert_eq!(report::rupees(30000), "₹300");
}

#[test]
fn five_wrong_passwords_lock_sign_in_even_for_the_right_one() {
    let mut s = Store::open_in_memory().unwrap();
    s.create_first_admin("boss", "Boss", "secret1").unwrap();
    s.set_lock_seconds(1);
    for i in 0..4 { assert!(matches!(s.authenticate("boss", "bad"), Err(Error::Rule(_))), "try {i}"); }
    assert_eq!(s.attempts_left().unwrap(), 1);
    assert!(matches!(s.authenticate("boss", "bad"), Err(Error::Locked(n)) if n == 1));
    assert!(matches!(s.authenticate("boss", "secret1"), Err(Error::Locked(_))));
    assert!(s.lock_seconds_left().unwrap() > 0);
    std::thread::sleep(std::time::Duration::from_millis(1200)); // time passes
    assert_eq!(s.lock_seconds_left().unwrap(), 0);
    s.authenticate("boss", "secret1").unwrap();
    assert_eq!(s.attempts_left().unwrap(), 5, "success resets the count");
    // password re-checks count too
    let id = s.authenticate("boss", "secret1").unwrap().id;
    s.set_lock_seconds(60);
    for _ in 0..4 { assert!(s.confirm_password(id, "no").is_err()); }
    assert!(matches!(s.confirm_password(id, "no"), Err(Error::Locked(_))));
}

#[test]
fn saved_sign_in_survives_restart_and_ends_properly() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("lab.db");
    let (token, uid) = {
        let mut s = Store::open(&db).unwrap();
        let u = s.create_first_admin("boss", "Boss", "secret1").unwrap();
        (s.create_session(u.id).unwrap(), u.id)
    };
    let mut s = Store::open(&db).unwrap(); // "closed and reopened the app"
    assert_eq!(s.resume_session(&token).unwrap().unwrap().id, uid);
    assert!(s.resume_session("garbage").unwrap().is_none());
    // the token itself is not stored
    let n: i64 = rusqlite::Connection::open(&db).unwrap().query_row("SELECT COUNT(*) FROM sessions WHERE token_hash=?1", [&token], |r| r.get(0)).unwrap();
    assert_eq!(n, 0);
    s.end_session(&token).unwrap();
    assert!(s.resume_session(&token).unwrap().is_none());
    // password change ends saved sign-ins
    let t2 = s.create_session(uid).unwrap();
    s.change_password(uid, "secret1", "secret2").unwrap();
    assert!(s.resume_session(&t2).unwrap().is_none());
    // switching an account off ends it too
    let admin = s.users().unwrap()[0].actor();
    let d = s.create_user(&admin, "desk", "Desk", Role::Reception, "pass123").unwrap();
    let t3 = s.create_session(d.id).unwrap();
    s.update_user(&admin, d.id, "Desk", Role::Reception, false).unwrap();
    assert!(s.resume_session(&t3).unwrap().is_none());
}

fn set_backup(s: &mut Store, local: &std::path::Path, cloud: &std::path::Path, pw: Option<&str>) {
    s.save_backup_settings(&admin(), &BackupSettings { folder: local.to_string_lossy().into(), cloud_folder: cloud.to_string_lossy().into(),
        encrypt: pw.is_some(), password: pw.unwrap_or("").into(), keep: 30 }).unwrap();
}

#[test]
fn backup_restore_round_trip_plain_and_locked() {
    let (mut s, o) = lab();
    let dir = tempfile::tempdir().unwrap();
    let (local, cloud, safety) = (dir.path().join("local"), dir.path().join("cloud"), dir.path().join("safety"));
    std::fs::create_dir_all(&local).unwrap(); std::fs::create_dir_all(&cloud).unwrap();
    set_backup(&mut s, &local, &cloud, None);
    let (done, failed) = s.backup_now(&admin()).unwrap();
    assert!(failed.is_empty() && done.len() == 2, "copy reached the cloud folder");
    let plain = done[0].clone();
    assert!(!is_encrypted_backup(&plain));
    set_backup(&mut s, &local, &cloud, Some("my backup pw"));
    let locked = s.backup_now(&admin()).unwrap().0[0].clone();
    assert!(is_encrypted_backup(&locked));
    assert!(s.backup_now(&desk()).is_err(), "Admin only");
    // the lab keeps working, then we go back to the backup
    s.delete_order(&admin(), o.id, "oops, wrong").unwrap();
    assert!(s.orders("", 10).unwrap().is_empty());
    assert!(s.restore_from(&desk(), &plain, None, &safety).is_err(), "Admin only");
    assert!(s.restore_from(&admin(), &locked, None, &safety).unwrap_err().to_string().contains("locked"));
    assert!(s.restore_from(&admin(), &locked, Some("wrong"), &safety).unwrap_err().to_string().contains("password"));
    let kept = s.restore_from(&admin(), &locked, Some("my backup pw"), &safety).unwrap();
    assert!(kept.exists(), "safety copy of the state before restoring");
    assert_eq!(s.orders("", 10).unwrap().len(), 1, "report is back");
    // a backup never carries the password that locks it
    assert_eq!(s.backup_settings().unwrap().password, "", "password was not inside the backup");
    // garbage is refused without touching anything
    let junk = dir.path().join("junk.ldbk"); std::fs::write(&junk, b"hello world, not a backup").unwrap();
    assert!(s.restore_from(&admin(), &junk, None, &safety).is_err());
    assert_eq!(s.orders("", 10).unwrap().len(), 1);
}

#[test]
fn restore_into_a_new_computer() {
    let dir = tempfile::tempdir().unwrap();
    let mut a = Store::open_in_memory().unwrap();
    let u = a.create_first_admin("boss", "Boss", "secret1").unwrap();
    set_backup(&mut a, dir.path(), dir.path(), None);
    let f = a.backup_now(&u.actor()).unwrap().0[0].clone();
    let mut fresh = Store::open(dir.path().join("new.db")).unwrap();
    fresh.restore_on_first_run(&f, None).unwrap();
    fresh.authenticate("boss", "secret1").unwrap();
    assert!(fresh.restore_on_first_run(&f, None).is_err(), "once people exist, only an Admin can restore");
}

#[test]
fn daily_auto_backup_once_a_day_and_pruned() {
    let (mut s, _) = lab();
    let dir = tempfile::tempdir().unwrap();
    assert!(s.auto_backup().unwrap().is_none(), "off until a folder is chosen");
    set_backup(&mut s, dir.path(), std::path::Path::new(""), None);
    assert!(s.auto_backup().unwrap().is_some());
    assert!(s.auto_backup().unwrap().is_none(), "already done today");
    assert!(s.last_backup().unwrap().is_some());
    assert!(s.save_backup_settings(&admin(), &BackupSettings { folder: "/no/such/dir".into(), ..Default::default() }).is_err());
    assert!(s.save_backup_settings(&admin(), &BackupSettings { encrypt: true, password: "123".into(), ..Default::default() }).unwrap_err().to_string().contains("password"));
    // keeps only the newest N
    s.save_backup_settings(&admin(), &BackupSettings { folder: dir.path().to_string_lossy().into(), keep: 3, ..Default::default() }).unwrap();
    for _ in 0..5 { s.backup_now(&admin()).unwrap(); }
    assert_eq!(list_backups(dir.path()).unwrap().len(), 3);
}

#[test]
fn upgrading_a_version_1_database_keeps_everything() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("old.db");
    {
        let c = rusqlite::Connection::open(&db).unwrap();
        c.execute_batch(include_str!("../src/schema.sql")).unwrap();
        c.pragma_update(None, "user_version", 1).unwrap();
        c.execute("INSERT INTO users(username,name,password_hash,role,active) VALUES('a','A','x','admin',1)", []).unwrap();
    }
    let s = Store::open(&db).unwrap();
    assert!(s.has_users().unwrap());
    assert_eq!(s.lock_seconds_left().unwrap(), 0);
}

#[test]
fn returning_patient_keeps_id_and_each_visit_keeps_its_age() {
    let (mut s, first) = lab();
    let t = s.tests().unwrap()[0].id;
    let found = s.patients("ali", 10).unwrap();
    assert_eq!((found.len(), found[0].visits, found[0].patient_code.as_str()), (1, 1, first.patient.patient_code.as_str()));
    assert_eq!(s.patients("srinagar", 10).unwrap().len(), 0, "address is not searched");
    let again = s.register(&desk(), NewOrder {
        patient: NewPatient { first_name: "ignored".into(), age: 42.0, age_type: "Year".into(), gender: "Male".into(), address: "Anantnag".into(), phone: "777".into(), ..Default::default() },
        items: vec![OrderItem { test_id: t, price_paise: None, discount_paise: 0 }], existing_patient_id: Some(first.patient.id), ..Default::default() }).unwrap();
    assert_eq!(again.patient.patient_code, first.patient.patient_code, "same patient ID");
    assert_eq!((again.patient.first_name.as_str(), again.patient.age, again.patient.phone.as_str()), ("Ali", 42.0, "777"));
    assert_eq!(s.order(first.id).unwrap().patient.age, 40.0, "the old visit still shows the age it had");
    assert!(s.register(&desk(), NewOrder { patient: NewPatient { first_name: "x".into(), age: 1.0, age_type: "Year".into(), gender: "Male".into(), address: "a".into(), ..Default::default() },
        items: vec![OrderItem { test_id: t, price_paise: None, discount_paise: 0 }], existing_patient_id: Some(9999), ..Default::default() }).is_err());
    // history
    let h = s.patient_history(first.patient.id).unwrap();
    assert_eq!((h.visits.len(), h.test_counts[0].times, h.test_counts[0].name.as_str()), (2, 2, "Complete Blood Count"));
    assert_eq!((h.billed_paise, h.paid_paise, h.due_paise), (60000, 10000, 50000));
    assert_eq!(h.visits[0].order_id, again.id, "newest first");
    assert_eq!(s.patients("", 10).unwrap()[0].visits, 2);
    // deleting a visit removes it from the history
    s.delete_order(&admin(), again.id, "duplicate").unwrap();
    assert_eq!(s.patient_history(first.patient.id).unwrap().visits.len(), 1);
}

#[test]
fn name_cannot_change_if_other_visits_are_approved() {
    let (mut s, first) = lab();
    let cur = s.order(first.id).unwrap(); approve(&mut s, &cur);
    let t = s.tests().unwrap()[0].id;
    let second = s.register(&desk(), NewOrder {
        patient: NewPatient { first_name: "Ali".into(), age: 41.0, age_type: "Year".into(), gender: "Male".into(), address: "x".into(), ..Default::default() },
        items: vec![OrderItem { test_id: t, price_paise: None, discount_paise: 0 }], existing_patient_id: Some(first.patient.id), ..Default::default() }).unwrap();
    let mut p = NewPatient { designation: "MR.".into(), first_name: "Aly".into(), last_name: "Khan".into(), age: 41.0, age_type: "Year".into(), gender: "Male".into(), address: "x".into(), ..Default::default() };
    assert!(s.update_patient(&desk(), second.id, &p, &Referral::SelfReferred).unwrap_err().to_string().contains("other reports"));
    p.first_name = "Ali".into(); p.phone = "123".into(); p.age = 43.0;
    s.update_patient(&desk(), second.id, &p, &Referral::SelfReferred).unwrap();
    assert_eq!((s.order(second.id).unwrap().patient.age, s.order(first.id).unwrap().patient.age), (43.0, 40.0));
}

#[test]
fn scanned_qr_text_finds_and_verifies_the_report() {
    let (mut s, o) = lab();
    let cur = s.order(o.id).unwrap();
    let done = approve(&mut s, &cur);
    let prof = s.profile().unwrap();
    let reported = done.tests[0].approved_at.clone().unwrap();
    let text = report::qr_text(&prof, &done, &reported, &report::verification_code(&done));
    assert!(text.contains("Patient: MR. Ali Khan") && text.contains(&format!("Report no: {}", o.id)) && text.len() < 200, "{text}");
    let v = s.verify_report_text(&text).unwrap().unwrap();
    assert!(v.genuine && v.order_id == o.id);
    let forged = text.replace("Ref: ", "Ref: 0");
    assert!(!s.verify_report_text(&forged).unwrap().unwrap().genuine);
    assert!(s.verify_report_text("hello").unwrap().is_none());
}

#[test]
fn possible_matches_by_phone_or_name() {
    let (mut s, _) = lab(); // Ali Khan, phone empty in lab()
    let t = s.tests().unwrap()[0].id;
    let mk = |first: &str, last: &str, phone: &str| NewOrder {
        patient: NewPatient { first_name: first.into(), last_name: last.into(), age: 30.0, age_type: "Year".into(), gender: "Male".into(), address: "x".into(), phone: phone.into(), ..Default::default() },
        items: vec![OrderItem { test_id: t, price_paise: None, discount_paise: 0 }], ..Default::default() };
    s.register(&desk(), mk("Bilal", "Dar", "+91 99060-12345")).unwrap();
    let m = s.possible_matches("", "", "9906012345", 5).unwrap();
    assert_eq!(m.len(), 1);
    assert_eq!(m[0].name.contains("Bilal"), true);
    assert_eq!(s.possible_matches("", "", "99060", 5).unwrap().len(), 0, "too short to trust");
    assert_eq!(s.possible_matches("ali", "", "", 5).unwrap()[0].name.contains("Ali"), true);
    assert_eq!(s.possible_matches("Ali", "Zed", "", 5).unwrap().len(), 0);
    assert_eq!(s.possible_matches("A", "", "", 5).unwrap().len(), 0, "one letter is not enough");
}

#[test]
fn paying_later_settles_oldest_visit_first_even_after_delivery() {
    let (mut s, o) = lab(); // total 30000, paid 10000
    let t = s.tests().unwrap()[0].id;
    let pid = o.patient.id;
    let o2 = s.register(&desk(), NewOrder { existing_patient_id: Some(pid), patient: NewPatient { first_name: "Ali".into(), age: 41.0, age_type: "Year".into(), gender: "Male".into(), address: "x".into(), ..Default::default() },
        items: vec![OrderItem { test_id: t, price_paise: None, discount_paise: 0 }], ..Default::default() }).unwrap();
    assert_eq!(s.patient_due(pid).unwrap(), 20000 + 30000);
    // one visit paid later
    let done = s.add_payment(&desk(), o.id, 5000, "UPI", "t1").unwrap();
    assert_eq!(done.due_paise(), 15000);
    // a lump sum goes to the oldest first
    let applied = s.receive_patient_payment(&desk(), pid, 20000, "Cash", "").unwrap();
    assert_eq!(applied, vec![(o.id, 15000), (o2.id, 5000)]);
    assert_eq!(s.patient_due(pid).unwrap(), 25000);
    assert!(s.receive_patient_payment(&desk(), pid, 25001, "Cash", "").is_err(), "cannot take more than owed");
    assert!(s.receive_patient_payment(&desk(), pid, 100, "Gold", "").is_err());
    assert!(matches!(s.receive_patient_payment(&Actor::new("T", Role::Technician), pid, 100, "Cash", ""), Err(Error::Forbidden(_))));
    // a delivered report can still be paid for
    let cur = s.order(o2.id).unwrap();
    let cur = approve(&mut s, &cur);
    s.mark_delivered(&desk(), cur.id, cur.version).unwrap();
    s.add_payment(&desk(), o2.id, 25000, "Bank", "").unwrap();
    assert_eq!(s.patient_due(pid).unwrap(), 0);
}

#[test]
fn payment_ledger_and_one_combined_bill() {
    let (mut s, o) = lab();
    let t = s.tests().unwrap()[0].id;
    let pid = o.patient.id;
    s.register(&desk(), NewOrder { existing_patient_id: Some(pid), patient: NewPatient { first_name: "Ali".into(), age: 41.0, age_type: "Year".into(), gender: "Male".into(), address: "x".into(), ..Default::default() },
        items: vec![OrderItem { test_id: t, price_paise: None, discount_paise: 0 }], ..Default::default() }).unwrap();
    s.receive_patient_payment(&desk(), pid, 20000, "UPI", "ref-9").unwrap();
    let ledger = s.patient_payments(pid).unwrap();
    assert_eq!(ledger.len(), 2, "the 10000 taken at registration plus the lump sum");
    assert_eq!(ledger.iter().map(|p| p.amount_paise).sum::<i64>(), 30000);
    let bill = s.patient_bill_pdf(&desk(), pid, false).unwrap();
    assert!(bill.pdf.starts_with(b"%PDF"));
    assert!(matches!(s.patient_bill_pdf(&Actor::new("T", Role::Technician), pid, false), Err(Error::Forbidden(_))));
    assert!(s.patient_bill_pdf(&desk(), 99999, false).is_err());
}
