// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

use labdesk_core::*;

const CAT: &str = r#"{"tests":[{"code":"CBC","name":"Complete Blood Count","department":"HAEMATOLOGY","sample":"EDTA","price_paise":30000,"config":{"parameters":[
   {"id":"hb","name":"Hb","field":"Numeric","unit":"g/dL","required":true,"low":13,"high":17}]}}]}"#;
fn admin() -> Actor { Actor::new("Dr Admin", Role::Admin) }
fn desk() -> Actor { Actor::new("Front", Role::Reception) }

fn visit(s: &mut Store, name: &str, doctor: &str, pay: i64, method: &str) -> Order {
    let t = s.tests().unwrap()[0].id;
    s.register(&desk(), NewOrder {
        patient: NewPatient { first_name: name.into(), last_name: "Khan".into(), age: 30.0, age_type: "Year".into(), gender: "Male".into(), address: "Srinagar, J&K".into(), ..Default::default() },
        items: vec![OrderItem { test_id: t, price_paise: None, discount_paise: 0 }], referral_doctor: doctor.into(),
        payments: if pay > 0 { vec![(pay, method.into(), "".into())] } else { vec![] }, ..Default::default() }).unwrap()
}
fn today() -> String { chrono::Utc::now().with_timezone(&chrono::FixedOffset::east_opt(19800).unwrap()).format("%Y-%m-%d").to_string() }

#[test]
fn day_summary_adds_up_and_csv_is_safe_for_excel() {
    let mut s = Store::open_in_memory().unwrap();
    s.import_catalogue(&admin(), CAT).unwrap();
    visit(&mut s, "Ali", "Dr. Rashid", 30000, "Cash");
    visit(&mut s, "Bashir", "", 10000, "UPI");
    visit(&mut s, "Dina", "=SUM(1)", 0, "Cash");
    let gone = visit(&mut s, "Gone", "Dr. Rashid", 30000, "Cash");
    s.delete_order(&admin(), gone.id, "typed wrongly").unwrap();
    let d = today();
    let sm = s.summary(&desk(), &d, &d).unwrap();
    assert_eq!((sm.visits, sm.billed_paise, sm.received_paise, sm.due_paise), (3, 90000, 40000, 50000));
    assert_eq!(sm.by_method, vec![("Cash".to_string(), 30000), ("UPI".to_string(), 10000)]);
    assert_eq!(sm.tests, vec![("Complete Blood Count".to_string(), 3)]);
    let docs: Vec<_> = sm.referrals.iter().map(|r| (r.name.as_str(), r.visits)).collect();
    assert!(docs.contains(&("Dr. Rashid", 1)) && docs.contains(&("Self", 1)), "{docs:?}");
    // another day has nothing
    assert_eq!(s.summary(&desk(), "2020-01-01", "2020-01-31").unwrap().visits, 0);
    assert!(s.summary(&desk(), "2026-10-25", "2026-10-01").is_err());
    assert!(s.summary(&desk(), "yesterday", "today").is_err());
    // csv
    let csv = s.export_csv(&desk(), "bills", &d, &d).unwrap();
    assert!(csv.starts_with('\u{feff}') && csv.contains("Date,Report no"));
    assert_eq!(csv.lines().count(), 4, "header and three visits (the deleted one is left out)");
    assert!(s.export_csv(&desk(), "doctors", &d, &d).unwrap().contains("'=SUM(1)"), "text that looks like a formula is made harmless");
    assert!(csv.contains("Complete Blood Count") && csv.contains("300.00") && csv.contains("200.00"), "{csv}");
    assert!(s.export_csv(&desk(), "payments", &d, &d).unwrap().contains("UPI,100.00"));
    assert!(s.export_csv(&desk(), "patients", &d, &d).unwrap().contains("\"Srinagar, J&K\""));
    assert!(s.export_csv(&desk(), "doctors", &d, &d).unwrap().contains("Dr. Rashid,1,300.00,300.00"));
    assert!(s.export_csv(&desk(), "tests", &d, &d).unwrap().contains("Complete Blood Count,3"));
    assert!(s.export_csv(&desk(), "nonsense", &d, &d).is_err());
    // money is for the desk and the admin only
    assert!(s.summary(&Actor::new("Tech", Role::Technician), &d, &d).is_err());
}

#[test]
fn a_result_is_followed_over_visits() {
    let mut s = Store::open_in_memory().unwrap();
    s.import_catalogue(&admin(), CAT).unwrap();
    let doctor = Actor::new("Dr Path", Role::Pathologist);
    let first = visit(&mut s, "Ali", "", 0, "Cash");
    let pid = first.patient.id;
    let t = s.tests().unwrap()[0].id;
    let run = |s: &mut Store, o: &Order, v: &str| {
        let o = s.save_results(&admin(), o.id, o.version, &[ResultInput { order_test_id: o.tests[0].id, parameter_id: "hb".into(), value: v.into(), comment: None, manual: false }], None).unwrap();
        s.approve(&doctor, o.id, o.version, None).unwrap();
    };
    run(&mut s, &first, "12");
    assert!(s.trends(pid).unwrap().is_empty(), "one measurement is not a trend");
    let second = s.register(&desk(), NewOrder { existing_patient_id: Some(pid), items: vec![OrderItem { test_id: t, price_paise: None, discount_paise: 0 }],
        patient: NewPatient { first_name: "Ali".into(), age: 30.0, age_type: "Year".into(), gender: "Male".into(), address: "x".into(), ..Default::default() }, ..Default::default() }).unwrap();
    run(&mut s, &second, "14.5");
    let tr = s.trends(pid).unwrap();
    assert_eq!(tr.len(), 1);
    assert_eq!((tr[0].name.as_str(), tr[0].unit.as_str(), tr[0].points.len()), ("Hb", "g/dL", 2));
    assert_eq!((tr[0].points[0].value, tr[0].points[1].value), (12.0, 14.5));
    assert_eq!((tr[0].low, tr[0].high), (Some(13.0), Some(17.0)));
    assert_eq!(tr[0].points[0].flag, "L");
}
