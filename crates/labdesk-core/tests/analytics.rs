// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

use labdesk_core::*;

const CAT: &str = r#"{"tests":[
 {"code":"CBC","name":"Complete Blood Count","department":"HAEMATOLOGY","sample":"EDTA","price_paise":30000,"config":{"parameters":[{"id":"hb","name":"Hb","field":"Numeric","unit":"g/dL","required":true,"low":13,"high":17}]}},
 {"code":"UREA","name":"Urea","department":"BIOCHEMISTRY","sample":"serum","price_paise":10000,"config":{"parameters":[{"id":"u","name":"Urea","field":"Numeric","unit":"mg/dl","low":10,"high":50}]}}]}"#;
fn admin() -> Actor { Actor::new("Dr Admin", Role::Admin) }
fn desk() -> Actor { Actor::new("Front", Role::Reception) }
fn today() -> String { chrono::Utc::now().with_timezone(&chrono::FixedOffset::east_opt(19800).unwrap()).format("%Y-%m-%d").to_string() }

fn visit(s: &mut Store, name: &str, tests: &[&str], pay: i64, method: &str) -> Order {
    let all = s.tests().unwrap();
    let items = tests.iter().map(|c| OrderItem { test_id: all.iter().find(|t| t.code == *c).unwrap().id, price_paise: None, discount_paise: 0 }).collect();
    s.register(&desk(), NewOrder {
        patient: NewPatient { first_name: name.into(), last_name: "Khan".into(), phone: "9876543210".into(), age: 30.0, age_type: "Year".into(), gender: "Male".into(), address: "Srinagar".into(), ..Default::default() },
        items, referral_doctor: "Dr. Rashid".into(), payments: if pay > 0 { vec![(pay, method.into(), "".into())] } else { vec![] }, ..Default::default() }).unwrap()
}

#[test]
fn analytics_adds_up_and_only_money_roles_may_see_it() {
    let mut s = Store::open_in_memory().unwrap();
    s.import_catalogue(&admin(), CAT).unwrap();
    visit(&mut s, "Ali", &["CBC", "UREA"], 40000, "Cash");
    visit(&mut s, "Bashir", &["CBC"], 10000, "UPI");
    let gone = visit(&mut s, "Gone", &["UREA"], 10000, "Cash");
    s.delete_order(&admin(), gone.id, "typed wrongly").unwrap();
    let d = today();
    let a = s.analytics(&desk(), &d, &d).unwrap();
    assert_eq!((a.totals.visits, a.totals.tests, a.totals.billed_paise, a.totals.received_paise, a.totals.due_paise), (2, 3, 70000, 50000, 20000));
    assert_eq!(a.by_method, vec![("Cash".to_string(), 40000), ("UPI".to_string(), 10000)]);
    assert_eq!(a.top_tests[0], ("Complete Blood Count".to_string(), 2));
    let dept: Vec<_> = a.by_dept.iter().map(|d| (d.name.as_str(), d.tests, d.value_paise)).collect();
    assert_eq!(dept, vec![("HAEMATOLOGY", 2, 60000), ("BIOCHEMISTRY", 1, 10000)]);
    assert_eq!(a.buckets.len(), 1);
    assert_eq!((a.buckets[0].visits, a.buckets[0].received_paise), (2, 50000));
    assert_eq!(a.hours.iter().sum::<i64>(), 2);
    assert_eq!(a.waiting_reports, 2);
    assert!(a.previous.visits == 0 && !a.previous_label.is_empty());
    assert_eq!(a.totals.new_patients + a.totals.returning_patients, 2);
    assert!(s.analytics(&Actor::new("Tech", Role::Technician), &d, &d).is_err());
    assert!(s.analytics(&desk(), "2026-10-25", "2026-10-01").is_err());
    assert!(s.analytics(&desk(), "last week", "today").is_err());
}

#[test]
fn long_ranges_are_grouped_by_month_and_empty_days_still_show() {
    let mut s = Store::open_in_memory().unwrap();
    s.import_catalogue(&admin(), CAT).unwrap();
    visit(&mut s, "Ali", &["CBC"], 30000, "Cash");
    let week = s.analytics(&desk(), "2020-01-01", "2020-01-07").unwrap();
    assert_eq!((week.buckets.len(), week.monthly, week.totals.visits), (7, false, 0));
    let year = s.analytics(&desk(), "2020-01-01", "2020-12-31").unwrap();
    assert_eq!((year.buckets.len(), year.monthly), (12, true));
    assert_eq!(year.buckets[0].label, "2020-01");
}

#[test]
fn a_returning_patient_is_counted_as_returning_and_turnaround_is_measured() {
    let mut s = Store::open_in_memory().unwrap();
    s.import_catalogue(&admin(), CAT).unwrap();
    let first = visit(&mut s, "Ali", &["UREA"], 10000, "Cash");
    let urea = s.tests().unwrap().iter().find(|t| t.code == "UREA").unwrap().id;
    s.register(&desk(), NewOrder { existing_patient_id: Some(first.patient.id), patient: NewPatient { first_name: "Ali".into(), last_name: "Khan".into(), age: 31.0, age_type: "Year".into(), gender: "Male".into(), address: "Srinagar".into(), ..Default::default() },
        items: vec![OrderItem { test_id: urea, price_paise: None, discount_paise: 0 }], ..Default::default() }).unwrap();
    visit(&mut s, "Bashir", &["UREA"], 0, "Cash");
    let d = today();
    let a = s.analytics(&desk(), &d, &d).unwrap();
    assert_eq!((a.totals.visits, a.totals.new_patients, a.totals.returning_patients), (3, 2, 1));
    assert_eq!(a.totals.avg_turnaround_min, None, "nothing approved yet");
}
