// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

use labdesk_core::*;

const CAT: &str = r#"{"tests":[
 {"code":"RBS","name":"Random Blood Sugar Test with a long name that has to wrap on a narrow roll","department":"BIOCHEMISTRY","sample":"serum","price_paise":5000,"config":{"parameters":[{"id":"g","name":"Glucose","field":"Numeric","unit":"mg/dl","low":70,"high":140}]}}
]}"#;
fn admin() -> Actor { Actor::new("Dr Admin", Role::Admin) }
fn desk() -> Actor { Actor::new("Front", Role::Reception) }

fn store_with_bill() -> (Store, i64) {
    let mut s = Store::open_in_memory().unwrap();
    s.import_catalogue(&admin(), CAT).unwrap();
    let all = s.tests().unwrap();
    let o = s.register(&desk(), NewOrder {
        patient: NewPatient { first_name: "Fozia".into(), last_name: "".into(), phone: "9876543210".into(), age: 33.0, age_type: "Year".into(), gender: "Female".into(), address: "Baramulla".into(), ..Default::default() },
        items: vec![OrderItem { test_id: all[0].id, price_paise: None, discount_paise: 500 }], payments: vec![(4500, "Cash".into(), "".into())], ..Default::default() }).unwrap();
    (s, o.id)
}
fn media_box(pdf: &[u8]) -> (f32, f32) {
    let t = String::from_utf8_lossy(pdf);
    let i = t.find("/MediaBox").expect("a page size");
    let nums: Vec<f32> = t[i..].split(|c| c == '[' || c == ']').nth(1).unwrap().split_whitespace().filter_map(|x| x.parse().ok()).collect();
    (nums[2] - nums[0], nums[3] - nums[1])
}

#[test]
fn bill_prefs_default_to_no_header_on_a4_and_are_remembered() {
    let mut s = Store::open_in_memory().unwrap();
    let p = s.bill_prefs().unwrap();
    assert_eq!((p.header, p.paper.as_str(), p.roll_mm()), (false, "a4", None));
    s.set_bill_prefs(&desk(), &BillPrefs { header: true, paper: "roll80".into() }).unwrap();
    let p = s.bill_prefs().unwrap();
    assert_eq!((p.header, p.roll_mm()), (true, Some(80.0)));
    s.set_bill_prefs(&desk(), &BillPrefs { header: false, paper: "nonsense".into() }).unwrap();
    assert_eq!(s.bill_prefs().unwrap().paper, "a4");
    assert!(s.set_bill_prefs(&Actor::new("Tech", Role::Technician), &BillPrefs::default()).is_err());
}

#[test]
fn roll_bills_are_narrow_single_pages_and_a4_is_unchanged() {
    let (mut s, id) = store_with_bill();
    let a4 = s.bill_pdf_on(&desk(), id, false, None).unwrap();
    let (w, h) = media_box(&a4.pdf);
    assert!((w - 595.28).abs() < 1.0 && (h - 841.89).abs() < 1.0);
    for (mm, header) in [(80.0f32, false), (80.0, true), (58.0, false), (58.0, true)] {
        let r = s.bill_pdf_on(&desk(), id, header, Some(mm)).unwrap();
        assert_eq!(r.pages, 1);
        let (w, h) = media_box(&r.pdf);
        assert!((w - mm * 2.8346457).abs() < 1.0, "width {w} for {mm} mm");
        assert!(h > 120.0 && h < 600.0, "height {h}");
    }
    assert!(s.bill_pdf_on(&Actor::new("Tech", Role::Technician), id, false, Some(80.0)).is_err());
}

#[test]
#[ignore]
fn write_samples_for_a_look() {
    let (mut s, id) = store_with_bill();
    s.save_profile(&admin(), &Profile { name: "Sample Diagnostic Centre".into(), address: "Main Road, Srinagar, Kashmir".into(), phone: "0194-0000000".into(), ..Default::default() }).unwrap();
    for (name, mm, h) in [("r80", Some(80.0f32), true), ("r58", Some(58.0), false), ("r58h", Some(58.0), true), ("a4", None, false)] {
        std::fs::write(format!("/tmp/bill-{name}.pdf"), s.bill_pdf_on(&desk(), id, h, mm).unwrap().pdf).unwrap();
    }
}

#[test]
fn terms_must_be_accepted_once_per_version() {
    let mut s = Store::open_in_memory().unwrap();
    assert!(!s.terms_accepted().unwrap());
    s.accept_terms().unwrap();
    assert!(s.terms_accepted().unwrap());
    s.accept_terms().unwrap(); // accepting twice is harmless
    assert!(s.terms_accepted().unwrap());
}
