// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

use labdesk_core::*;

const CATALOGUE: &str = r#"{"tests":[
 {"code":"CBC","name":"Complete Blood Count","department":"HAEMATOLOGY","sample":"EDTA","price_paise":30000,"config":{"parameters":[
   {"id":"hb","name":"Hb","field":"Numeric","unit":"g/dL","required":true,"gender_ranges":{"Male":{"low":13,"high":17},"Female":{"low":12,"high":15}},"critical_low":7},
   {"id":"hct","name":"Hct","field":"Numeric"},
   {"id":"rbc","name":"RBC","field":"Numeric"},
   {"id":"mcv","name":"MCV","field":"Numeric","formula":"<hct> / <rbc> * 10"},
   {"id":"note","name":"Remark","field":"Dropdown","options":["Normal","Review"]}]}},
 {"code":"FBS","name":"Fasting Blood Sugar","department":"BIOCHEMISTRY","sample":"Fluoride","price_paise":8000,"config":{"parameters":[
   {"id":"glu","name":"Glucose","field":"Numeric","high":100,"critical_high":400}]}}
]}"#;

fn admin() -> Actor { Actor::new("Dr Admin", Role::Admin) }
fn reception() -> Actor { Actor::new("Front", Role::Reception) }
fn tech() -> Actor { Actor::new("Tech", Role::Technician) }
fn pathologist() -> Actor { Actor::new("Dr Path", Role::Pathologist) }

fn lab() -> Store {
    let mut s = Store::open_in_memory().unwrap();
    assert_eq!(s.import_catalogue(&admin(), CATALOGUE).unwrap(), 2);
    s
}
fn new_order(s: &Store, tests: &[&str], paid: i64) -> NewOrder {
    let all = s.tests().unwrap();
    NewOrder {
        patient: NewPatient { designation: "MR.".into(), first_name: "Ali".into(), last_name: "Khan".into(), age: 40.0, age_type: "Year".into(), gender: "Male".into(), phone: "9906000000".into(), email: "".into(), address: "Srinagar".into() },
        items: tests.iter().map(|c| OrderItem { test_id: all.iter().find(|t| t.code == *c).unwrap().id, price_paise: None, discount_paise: 0 }).collect(),
        payments: if paid > 0 { vec![(paid, "Cash".into(), "".into())] } else { vec![] },
        ..Default::default()
    }
}
fn input(o: &Order, test: &str, param: &str, v: &str) -> ResultInput {
    let t = o.tests.iter().find(|t| t.name == test).unwrap();
    ResultInput { order_test_id: t.id, parameter_id: param.into(), value: v.into(), comment: None, manual: false }
}
fn msg<T: std::fmt::Debug>(r: Result<T>) -> String { r.unwrap_err().to_string() }

#[test]
fn registration_billing_and_numbering() {
    let mut s = lab();
    let mut n = new_order(&s, &["CBC", "FBS"], 20000);
    n.items[0].discount_paise = 5000;
    n.discount_paise = 1000;
    let o = s.register(&reception(), n).unwrap();
    assert_eq!((o.subtotal_paise, o.discount_paise, o.total_paise), (38000, 6000, 32000));
    assert_eq!((o.paid_paise(), o.due_paise()), (20000, 12000));
    assert_eq!(o.patient.patient_code.len(), 9); // yymmdd + 3 digits
    assert!(o.patient.patient_code.ends_with("001"));
    let mut n2 = new_order(&s, &["FBS"], 0);
    n2.patient.gender = "Female".into(); n2.patient.designation = String::new();
    let o2 = s.register(&reception(), n2).unwrap();
    assert!(o2.patient.patient_code.ends_with("002"));
    assert_eq!((o.patient.designation.as_str(), o2.patient.designation.as_str()), ("MR.", "MS."));
    let o = s.add_payment(&reception(), o.id, 12000, "UPI", "ref1").unwrap();
    assert_eq!(o.due_paise(), 0);
    assert!(msg(s.add_payment(&reception(), o.id, 1, "Cash", "")).contains("Invalid payment"));
}

#[test]
fn registration_rules() {
    let mut s = lab();
    let mut n = new_order(&s, &["CBC"], 0); n.patient.address = " ".into();
    assert!(msg(s.register(&reception(), n)).contains("address is required"));
    let mut n = new_order(&s, &["CBC"], 0); n.patient.age = 200.0;
    assert!(msg(s.register(&reception(), n)).contains("Age is invalid"));
    assert!(msg(s.register(&reception(), new_order(&s, &["CBC"], 99999))).contains("exceeds"));
    let mut n = new_order(&s, &["CBC"], 0); n.discount_paise = 30001;
    assert!(msg(s.register(&reception(), n)).contains("Invalid discount"));
    let mut n = new_order(&s, &["CBC"], 0); n.items.clear();
    assert!(msg(s.register(&reception(), n)).contains("Select between"));
    assert!(matches!(s.register(&tech(), new_order(&s, &["CBC"], 0)), Err(Error::Forbidden(_))));
    // nothing half-written after a failed registration
    assert!(s.orders("", 10).unwrap().is_empty());
}

#[test]
fn result_entry_flags_formulas_and_status() {
    let mut s = lab();
    let o = s.register(&reception(), new_order(&s, &["CBC", "FBS"], 0)).unwrap();
    assert_eq!(o.status, "Pending");
    let o = s.save_results(&tech(), o.id, o.version, &[input(&o, "Complete Blood Count", "hb", "12.1"), input(&o, "Complete Blood Count", "hct", "45"), input(&o, "Complete Blood Count", "rbc", "5")], None).unwrap();
    let cbc = o.tests.iter().find(|t| t.name == "Complete Blood Count").unwrap();
    let r = |id: &str| cbc.results.iter().find(|r| r.parameter_id == id).unwrap();
    assert_eq!(r("hb").flag, "L"); // male 13-17
    assert_eq!(r("mcv").value, "90"); // calculated by the store, not the UI
    assert_eq!(cbc.status, "Partial");
    assert_eq!(o.tests.iter().find(|t| t.name == "Fasting Blood Sugar").unwrap().status, "Pending");
    assert_eq!(o.status, "Partial");
    // validation
    assert!(msg(s.save_results(&tech(), o.id, o.version, &[input(&o, "Complete Blood Count", "hb", "abc")], None)).contains("requires a number"));
    assert!(msg(s.save_results(&tech(), o.id, o.version, &[input(&o, "Complete Blood Count", "note", "Maybe")], None)).contains("configured value"));
    // stale version is refused (two people / two windows editing)
    assert!(msg(s.save_results(&tech(), o.id, o.version - 1, &[], None)).contains("changed"));
    assert!(matches!(s.save_results(&reception(), o.id, o.version, &[], None), Err(Error::Forbidden(_))));
}

#[test]
fn approval_correction_delivery_lock() {
    let mut s = lab();
    let o = s.register(&reception(), new_order(&s, &["CBC", "FBS"], 38000)).unwrap();
    // required result missing -> cannot approve
    assert!(msg(s.approve(&pathologist(), o.id, o.version, None)).contains("Missing required result: Hb"));
    let o = s.save_results(&tech(), o.id, o.version, &[input(&o, "Complete Blood Count", "hb", "14"), input(&o, "Complete Blood Count", "hct", "42"), input(&o, "Complete Blood Count", "rbc", "5"), input(&o, "Fasting Blood Sugar", "glu", "90")], Some("fasting sample")).unwrap();
    assert!(matches!(s.approve(&tech(), o.id, o.version, None), Err(Error::Forbidden(_))));
    // approve one test first
    let cbc = o.tests.iter().find(|t| t.name == "Complete Blood Count").unwrap().id;
    let o = s.approve(&pathologist(), o.id, o.version, Some(&[cbc])).unwrap();
    assert_eq!(o.status, "Partial");
    assert!(o.tests.iter().find(|t| t.id == cbc).unwrap().is_approved());
    // an approved test is locked against edits
    assert!(msg(s.save_results(&tech(), o.id, o.version, &[input(&o, "Complete Blood Count", "hb", "15")], None)).contains("locked"));
    let o = s.approve(&pathologist(), o.id, o.version, None).unwrap();
    assert_eq!(o.status, "Completed");
    assert!(msg(s.save_results(&tech(), o.id, o.version, &[], None)).contains("Approved reports are locked"));
    // correction needs a reason, re-opens, then the cycle repeats
    assert!(msg(s.correct(&pathologist(), o.id, o.version, "no", None)).contains("reason"));
    let o = s.correct(&pathologist(), o.id, o.version, "Transcription error in Hb", Some(&[cbc])).unwrap();
    assert_eq!(o.status, "Partial");
    let o = s.save_results(&tech(), o.id, o.version, &[input(&o, "Complete Blood Count", "hb", "13.5")], None).unwrap();
    let o = s.approve(&pathologist(), o.id, o.version, None).unwrap();
    assert_eq!(o.status, "Completed");
    // delivery: reception only, after approval; then permanently locked
    assert!(matches!(s.mark_delivered(&tech(), o.id, o.version), Err(Error::Forbidden(_))));
    let o = s.mark_delivered(&reception(), o.id, o.version).unwrap();
    assert_eq!(o.status, "Delivered");
    assert!(msg(s.correct(&admin(), o.id, o.version, "try anyway", None)).contains("permanently locked"));
    assert!(msg(s.save_results(&admin(), o.id, o.version, &[], None)).contains("permanently locked"));
    assert!(msg(s.mark_delivered(&admin(), o.id, o.version)).contains("already been delivered"));
}

#[test]
fn database_refuses_edits_to_delivered_results_even_with_raw_sql() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lab.db");
    let mut s = Store::open(&path).unwrap();
    s.import_catalogue(&admin(), CATALOGUE).unwrap();
    let o = s.register(&reception(), new_order(&s, &["FBS"], 0)).unwrap();
    let o = s.save_results(&tech(), o.id, o.version, &[input(&o, "Fasting Blood Sugar", "glu", "90")], None).unwrap();
    let o = s.approve(&pathologist(), o.id, o.version, None).unwrap();
    let c = rusqlite::Connection::open(&path).unwrap();
    // before delivery a raw edit is possible; after delivery it is not
    assert!(c.execute("UPDATE results SET comment='ok'", []).is_ok());
    s.mark_delivered(&reception(), o.id, o.version).unwrap();
    let err = c.execute("UPDATE results SET value='999'", []).unwrap_err().to_string();
    assert!(err.contains("permanently locked"), "{err}");
}

#[test]
fn audit_trail_is_append_only_and_complete() {
    let mut s = lab();
    let o = s.register(&reception(), new_order(&s, &["FBS"], 0)).unwrap();
    s.save_results(&tech(), o.id, o.version, &[input(&o, "Fasting Blood Sugar", "glu", "95")], None).unwrap();
    let log = s.audit_log(50).unwrap();
    let actions: Vec<&str> = log.iter().map(|l| l.2.as_str()).collect();
    assert!(actions.contains(&"register") && actions.contains(&"save_draft") && actions.contains(&"import_catalogue"));
    assert!(log.iter().any(|l| l.1 == "technician:Tech"));
}

#[test]
fn db_triggers_and_file_persistence_and_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lab.db");
    let id;
    {
        let mut s = Store::open(&path).unwrap();
        s.import_catalogue(&admin(), CATALOGUE).unwrap();
        id = s.register(&reception(), new_order(&s, &["FBS"], 0)).unwrap().id;
        let bk = dir.path().join("backup.db");
        s.backup_to(&bk).unwrap();
        assert!(s.backup_to(&bk).is_err()); // never overwrite an existing backup
        assert!(Store::open(&bk).unwrap().order(id).is_ok());
    }
    // reopen: data is still there
    let s = Store::open(&path).unwrap();
    assert_eq!(s.order(id).unwrap().patient.first_name, "Ali");
    // audit rows cannot be edited or deleted, even with raw SQL
    let c = rusqlite::Connection::open(&path).unwrap();
    assert!(c.execute("UPDATE audit SET actor='x'", []).is_err());
    assert!(c.execute("DELETE FROM audit", []).is_err());
}

#[test]
fn catalogue_import_is_atomic_and_validated() {
    let mut s = lab();
    let bad = r#"{"tests":[{"code":"OK1","name":"Fine","department":"X","config":{"parameters":[]}},
        {"code":"BAD","name":"Reversed","department":"X","config":{"parameters":[{"id":"a","name":"A","low":9,"high":1}]}}]}"#;
    assert!(msg(s.import_catalogue(&admin(), bad)).contains("reversed"));
    assert!(s.tests().unwrap().iter().all(|t| t.code != "OK1")); // all-or-nothing
    assert!(matches!(s.import_catalogue(&reception(), CATALOGUE), Err(Error::Forbidden(_))));
    assert_eq!(s.import_catalogue(&admin(), CATALOGUE).unwrap(), 2); // re-import updates, no duplicates
    assert_eq!(s.tests().unwrap().len(), 2);
}
