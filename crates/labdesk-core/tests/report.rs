// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

use labdesk_core::report::*;
use labdesk_core::*;

fn admin() -> Actor { Actor::new("Admin", Role::Admin) }
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x % 255) as u8, (y % 255) as u8, 120]));
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}
fn profile() -> Profile { Profile { name: "Test Lab".into(), address: "Srinagar".into(), signatory: "Dr X".into(), ..Default::default() } }

#[test]
fn sample_report_is_a_pdf_with_one_page() {
    let r = render(&ReportInput { order: &sample_order(), profile: &profile(), letterhead: None, signature: None, use_letterhead: true }).unwrap();
    assert!(r.pdf.starts_with(b"%PDF-"));
    assert!(r.pdf.len() > 5_000);
    assert_eq!(r.pages, 1);
    assert_eq!(r.verification.len(), 10);
}

#[test]
fn long_reports_flow_onto_more_pages() {
    let mut o = sample_order();
    let base = o.tests.clone();
    for i in 0..6 { for t in &base { let mut c = t.clone(); c.id += 10 * (i + 1); c.name = format!("{} {}", t.name, i + 2); o.tests.push(c); } }
    let r = render(&ReportInput { order: &o, profile: &profile(), letterhead: None, signature: None, use_letterhead: true }).unwrap();
    assert!(r.pages >= 3, "pages: {}", r.pages);
}

#[test]
fn letterhead_and_signature_images_are_used_and_bad_ones_are_refused_politely() {
    let (lh, sig) = (png(600, 850), png(300, 100));
    let with = render(&ReportInput { order: &sample_order(), profile: &profile(), letterhead: Some(&lh), signature: Some(&sig), use_letterhead: true }).unwrap();
    let without = render(&ReportInput { order: &sample_order(), profile: &profile(), letterhead: Some(&lh), signature: Some(&sig), use_letterhead: false }).unwrap();
    assert!(with.pdf.len() > without.pdf.len(), "letterhead should add the image");
    let bad = render(&ReportInput { order: &sample_order(), profile: &profile(), letterhead: Some(b"not an image"), signature: None, use_letterhead: true });
    assert!(bad.is_err());
}

#[test]
fn works_with_an_empty_lab_profile_and_awkward_text() {
    let mut o = sample_order();
    o.tests[0].results[0].value = "A very long text result that goes on and on and on, far wider than the result column, to prove it wraps instead of running off the page".into();
    o.tests[0].results[1].value = "Supercalifragilisticexpialidocious_Supercalifragilisticexpialidocious_Supercalifragilisticexpialidocious".into();
    o.patient.first_name = "Muhammad-Abdul-Rahman-Ibn-Abdullah-Al-Kashmiri-Sahib".into();
    let r = render(&ReportInput { order: &o, profile: &Profile::default(), letterhead: None, signature: None, use_letterhead: true });
    assert!(r.is_ok(), "{:?}", r.err());
}

#[test]
fn verification_code_tracks_the_results() {
    let a = sample_order();
    let mut b = sample_order();
    assert_eq!(verification_code(&a), verification_code(&b));
    b.tests[0].results[0].value = "13.1".into();
    assert_ne!(verification_code(&a), verification_code(&b));
}

fn lab_with_one_order() -> (Store, i64, i64) {
    let mut s = Store::open_in_memory().unwrap();
    let cat = r#"{"tests":[{"code":"FBS","name":"Fasting Blood Sugar","department":"BIOCHEMISTRY","sample":"F","price_paise":8000,"config":{"parameters":[{"id":"glu","name":"Glucose","field":"Numeric","unit":"mg/dL","high":100,"required":true}]}}]}"#;
    s.import_catalogue(&admin(), cat).unwrap();
    let id = s.tests().unwrap()[0].id;
    let n = NewOrder { patient: NewPatient { first_name: "Ali".into(), age: 40.0, age_type: "Year".into(), gender: "Male".into(), address: "Srinagar".into(), ..Default::default() },
        items: vec![OrderItem { test_id: id, price_paise: None, discount_paise: 0 }], ..Default::default() };
    let o = s.register(&admin(), n).unwrap();
    (s, o.id, o.version)
}

#[test]
fn store_prints_only_after_approval_and_audits_it() {
    let (mut s, oid, v) = lab_with_one_order();
    s.save_profile(&admin(), &profile()).unwrap();
    assert!(s.report_pdf(&admin(), oid, true).unwrap_err().to_string().contains("Approve"));
    let o = s.order(oid).unwrap();
    let inp = ResultInput { order_test_id: o.tests[0].id, parameter_id: "glu".into(), value: "112".into(), comment: None, manual: false };
    let o = s.save_results(&admin(), oid, v, &[inp], None).unwrap();
    s.approve(&Actor::new("Dr P", Role::Pathologist), oid, o.version, None).unwrap();
    let r = s.report_pdf(&admin(), oid, true).unwrap();
    assert!(r.pdf.starts_with(b"%PDF"));
    assert!(s.audit_log(20).unwrap().iter().any(|l| l.2 == "report_pdf"));
    assert!(s.test_page_pdf(true).unwrap().pdf.starts_with(b"%PDF"));
}

#[test]
fn profile_and_images_validation_and_permissions() {
    let (mut s, _, _) = lab_with_one_order();
    assert_eq!(s.profile().unwrap(), Profile::default()); // nothing saved yet
    assert!(s.save_profile(&admin(), &Profile { name: " ".into(), ..Default::default() }).unwrap_err().to_string().contains("lab name"));
    assert!(s.save_profile(&admin(), &Profile { name: "L".into(), top_mm: 500.0, ..Default::default() }).unwrap_err().to_string().contains("Top margin"));
    assert!(matches!(s.save_profile(&Actor::new("T", Role::Technician), &profile()), Err(Error::Forbidden(_))));
    s.save_profile(&admin(), &profile()).unwrap();
    assert_eq!(s.profile().unwrap().name, "Test Lab");
    // images
    assert!(s.set_asset(&admin(), "letterhead", b"hello".to_vec()).unwrap_err().to_string().contains("PNG or JPG"));
    assert!(s.set_asset(&admin(), "wallpaper", png(10, 10)).is_err());
    assert!(matches!(s.set_asset(&Actor::new("R", Role::Reception), "letterhead", png(10, 10)), Err(Error::Forbidden(_))));
    s.set_asset(&admin(), "letterhead", png(40, 60)).unwrap();
    assert!(s.asset("letterhead").unwrap().is_some() && s.asset("signature").unwrap().is_none());
    s.clear_asset(&admin(), "letterhead").unwrap();
    assert!(s.asset("letterhead").unwrap().is_none());
}

#[test]
fn home_screen_numbers() {
    let (mut s, oid, _) = lab_with_one_order();
    let st = s.stats().unwrap();
    assert_eq!((st.patients_today, st.reports_pending, st.ready_to_deliver, st.due_paise, st.collected_today_paise), (1, 1, 0, 8000, 0));
    s.add_payment(&admin(), oid, 3000, "Cash", "").unwrap();
    let st = s.stats().unwrap();
    assert_eq!((st.due_paise, st.collected_today_paise), (5000, 3000));
}

#[test]
fn every_header_design_renders_and_is_saved() {
    let mut s = Store::open_in_memory().unwrap();
    for d in 0..8u8 {
        let p = Profile { header_design: d, ..profile() };
        s.save_profile(&admin(), &p).unwrap();
        assert_eq!(s.profile().unwrap().header_design, d);
        let out = labdesk_core::report::render(&labdesk_core::report::ReportInput { order: &labdesk_core::report::sample_order(), profile: &p, letterhead: None, signature: None, use_letterhead: true }).unwrap();
        assert!(out.pdf.starts_with(b"%PDF") && out.pages >= 1);
    }
    // an out-of-range design is clamped, not rejected
    s.save_profile(&admin(), &Profile { header_design: 99, ..profile() }).unwrap();
    assert_eq!(s.profile().unwrap().header_design, 7);
}

#[test]
fn test_preview_and_new_test_fields_round_trip() {
    let mut s = Store::open_in_memory().unwrap();
    let mut def = TestDef { id: 0, code: "LFT".into(), name: "Liver".into(), department: "BIOCHEMISTRY".into(), sample: "Serum".into(), price_paise: 0, disabled: false, config: TestConfig::default() };
    def.config.sample_color = "Red".into(); def.config.lmp_required = true; def.config.pre_collection = "Fast 10 hours".into(); def.config.labdesk_id = "FLT1".into();
    def.config.parameters = vec![
        Parameter { id: "a".into(), name: "Bilirubin Total".into(), field: "Numeric".into(), low: Some(0.1), high: Some(1.2), omit_dlc: true, code: "BT".into(), ..Default::default() },
        Parameter { id: "b".into(), name: "Bilirubin Direct".into(), field: "Numeric".into(), low: Some(0.0), high: Some(0.4), ..Default::default() },
        Parameter { id: "c".into(), name: "Indirect".into(), field: "Numeric".into(), formula: "<Bilirubin_Total> - <Bilirubin_Direct>".into(), ..Default::default() },
    ];
    let saved = s.save_test(&admin(), &def).unwrap();
    let back = s.tests().unwrap().into_iter().find(|t| t.id == saved.id).unwrap();
    assert_eq!((back.config.sample_color.as_str(), back.config.lmp_required, back.config.pre_collection.as_str(), back.config.labdesk_id.as_str()), ("Red", true, "Fast 10 hours", "FLT1"));
    assert!(back.config.parameters[0].omit_dlc && back.config.parameters[0].code == "BT");
    let pdf = s.test_preview_pdf(&back, true).unwrap();
    assert!(pdf.pdf.starts_with(b"%PDF"));
    let o = labdesk_core::report::preview_order(&back);
    assert!(o.tests[0].results.iter().any(|r| r.parameter_id == "c" && !r.value.is_empty()), "the formula line is filled in the preview");
    assert_eq!(labdesk_core::report::strip_html("<table><tr><th>A&nbsp;</th><th>B</th></tr><tr><td>1</td><td>2 &amp; 3</td></tr></table>"), "A | B\n1 | 2 & 3");
}
