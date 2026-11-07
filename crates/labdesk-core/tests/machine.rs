// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

use labdesk_core::machine::*;
use labdesk_core::*;

const SAMPLE: &str = include_str!("fixtures/urit-3part-sample.hl7.txt");
fn admin() -> Actor { Actor::new("Dr Admin", Role::Admin) }
fn desk() -> Actor { Actor::new("Front", Role::Reception) }

fn lab_with_analyzer_test(name: &str) -> (Store, Order) {
    let mut s = Store::open_in_memory().unwrap();
    let t = s.ensure_analyzer_test(&admin()).unwrap();
    let o = s.register(&desk(), NewOrder {
        patient: NewPatient { designation: "MRS.".into(), first_name: name.into(), last_name: "Patient".into(), age: 40.0, age_type: "Year".into(), gender: "Female".into(), address: "Srinagar".into(), ..Default::default() },
        items: vec![OrderItem { test_id: t.id, price_paise: None, discount_paise: 0 }], ..Default::default() }).unwrap();
    (s, o)
}

#[test]
fn reads_the_real_urit_message() {
    let d = parse_hl7(SAMPLE).expect("result message");
    assert_eq!((d.sample_id.as_str(), d.patient_name.as_str(), d.sex.as_str(), d.age_text.as_str()), ("000000000096", "Mrs Sample Patient", "F", "40"));
    assert_eq!(d.test_time, "2026-10-23 11:07");
    assert_eq!(d.items.len(), 22, "20 analyzer items plus PDW_SD and PDW_CV");
    let h: Vec<_> = d.histograms.iter().map(|h| (h.name.as_str(), h.data.len(), h.lines.clone())).collect();
    assert_eq!(h, vec![("WBC", 256, vec![18, 65, 77]), ("RBC", 256, vec![41, 250]), ("PLT", 128, vec![4, 178])]);
    // what the printed analyzer report shows for the marker lines of the WBC graph: 26, 90, 106
    assert_eq!(d.histograms[0].lines.iter().map(|c| line_label("WBC", *c)).collect::<Vec<_>>(), vec![26, 90, 106]);
    assert_eq!(d.histograms[1].lines.iter().map(|c| line_label("RBC", *c)).collect::<Vec<_>>(), vec![44, 271]);
    assert_eq!(d.histograms[2].lines.iter().map(|c| line_label("PLT", *c)).collect::<Vec<_>>(), vec![1, 27]);
    assert!(parse_hl7("MSH|^~\\&|a|b|c|d|2026||ACK^R01|1|P|2.3.1\rMSA|AA|1").is_none());
    assert!(parse_hl7("hello").is_none());
}

#[test]
fn values_ranges_and_marks_match_the_printed_analyzer_report() {
    let d = parse_hl7(SAMPLE).unwrap();
    let get = |c: &str| d.items.iter().find(|i| i.code == c).unwrap();
    // exactly the figures on the printed analyzer report
    for (code, shown) in [("WBC", "5.50"), ("LYM%", "34.00"), ("GRAN%", "62.20"), ("LYM#", "1.900"), ("RBC", "4.20"), ("HGB", "11.9"), ("MCHC", "30"), ("PLT", "149"), ("MPV", "11.1"), ("PCT", "0.16"), ("P_LCR", "47.70"), ("P_LCC", "71")] {
        assert_eq!(shown_value(code, &get(code).value), if code == "LYM#" { "1.900" } else { shown }, "{code}");
    }
    let marks: Vec<_> = ITEMS.iter().filter(|i| analyzer_flag(get(i.code)) != "").map(|i| (i.code, analyzer_flag(get(i.code)))).collect();
    assert_eq!(marks, vec![("MCHC", "L"), ("MPV", "H"), ("P_LCR", "H")], "the Hit column of the analyzer print");
    assert_eq!(analyzer_range(ITEMS.iter().find(|i| i.code == "LYM#").unwrap()), "0.800~4.000");
    assert_eq!(analyzer_range(ITEMS.iter().find(|i| i.code == "MCHC").unwrap()), "32~35");
}

#[test]
fn inbox_suggests_by_name_and_attaching_fills_the_report() {
    let (mut s, o) = lab_with_analyzer_test("Sample");
    let id = s.save_machine_message(SAMPLE).unwrap().unwrap();
    assert_eq!(s.save_machine_message(SAMPLE).unwrap(), Some(id), "a repeat is not stored twice");
    assert_eq!(s.machine_inbox(true, 10).unwrap().len(), 1);
    assert_eq!(s.machine_inbox(true, 10).unwrap()[0].abnormal, 3);
    let c = s.machine_candidates(id).unwrap();
    assert_eq!(c[0].order_id, o.id);
    assert_eq!(c[0].score, 100, "Sample + Patient both match");
    let done = s.attach_machine_result(&Actor::new("Tech", Role::Technician), id, c[0].order_test_id).unwrap();
    let t = &done.tests[0];
    let val = |p: &str| t.results.iter().find(|r| r.parameter_id == p).unwrap().clone();
    assert_eq!(val("wbc").value, "5.50");
    assert_eq!((val("mchc").value.as_str(), val("mchc").flag.as_str(), val("mchc").range_text.as_deref()), ("30", "L", Some("32~35")));
    assert_eq!(t.machine.as_ref().unwrap().histograms.len(), 3);
    assert_eq!(done.status, "Partial", "nothing is approved by itself");
    assert!(s.attach_machine_result(&admin(), id, c[0].order_test_id).is_err(), "cannot attach twice");
    assert_eq!(s.machine_inbox(true, 10).unwrap().len(), 0);
    // saving another value keeps the analyzer range and mark
    let saved = s.save_results(&admin(), done.id, done.version, &[ResultInput { order_test_id: t.id, parameter_id: "mchc".into(), value: "33".into(), comment: None, manual: false }], None).unwrap();
    let m = saved.tests[0].results.iter().find(|r| r.parameter_id == "mchc").unwrap();
    assert_eq!((m.flag.as_str(), m.range_text.as_deref()), ("", Some("32~35")));
    // switch to the lab's own ranges: ranges by gender, marks recomputed
    s.set_machine_range_mode(&admin(), "labdesk").unwrap();
    let after = s.order(done.id).unwrap();
    let h = after.tests[0].results.iter().find(|r| r.parameter_id == "hgb").unwrap();
    assert_eq!(h.range_text, None);
    let pdf = s.report_pdf(&Actor::new("Dr", Role::Pathologist), after.id, true);
    assert!(pdf.is_err(), "not approved yet");
    s.set_machine_range_mode(&admin(), "analyzer").unwrap();
    assert_eq!(s.order(done.id).unwrap().tests[0].results.iter().find(|r| r.parameter_id == "hgb").unwrap().range_text.as_deref(), Some("11.5~15.0"));
}

#[test]
fn only_staff_who_enter_results_can_attach() {
    let (mut s, _o) = lab_with_analyzer_test("Sample");
    let id = s.save_machine_message(SAMPLE).unwrap().unwrap();
    let tid = s.machine_candidates(id).unwrap()[0].order_test_id;
    assert!(matches!(s.attach_machine_result(&desk(), id, tid), Err(Error::Forbidden(_))));
    s.dismiss_machine_result(&desk(), id).unwrap();
    assert_eq!(s.machine_inbox(true, 5).unwrap().len(), 0);
}

#[test]
fn approved_machine_report_prints_with_graphs() {
    let (mut s, o) = lab_with_analyzer_test("Sample");
    let id = s.save_machine_message(SAMPLE).unwrap().unwrap();
    let tid = s.machine_candidates(id).unwrap()[0].order_test_id;
    let done = s.attach_machine_result(&admin(), id, tid).unwrap();
    let ok = s.approve(&Actor::new("Dr Path", Role::Pathologist), o.id, done.version, None).unwrap();
    assert!(ok.tests[0].machine.is_some(), "graphs stay with the approved test");
    let r = s.report_pdf(&Actor::new("Dr Path", Role::Pathologist), o.id, true).unwrap();
    assert!(r.pdf.starts_with(b"%PDF"));
    if let Ok(dir) = std::env::var("LABDESK_DUMP") { std::fs::write(format!("{dir}/machine-report.pdf"), &r.pdf).unwrap(); }
}

fn new_patient(name: &str) -> NewPatient {
    NewPatient { first_name: name.into(), last_name: "Sample".into(), age: 40.0, age_type: "Year".into(), gender: "Female".into(), ..Default::default() }
}

#[test]
fn one_step_report_from_an_analyzer_result_can_be_approved_and_printed() {
    let mut s = Store::open_in_memory().unwrap();
    let id = s.save_machine_message(SAMPLE).unwrap().unwrap();
    let doctor = Actor::new("Dr Doc", Role::Pathologist);
    let tech = Actor::new("Tech", Role::Technician);
    // a technician (who cannot register patients by hand) can do the one-step flow
    let o = s.report_from_machine(&tech, id, new_patient("Sample")).unwrap();
    assert_eq!(o.tests.len(), 1);
    assert!(o.tests[0].machine.is_some(), "values attached");
    assert!(o.tests[0].results.len() >= 20, "{} lines filled", o.tests[0].results.len());
    assert!(s.report_from_machine(&tech, id, new_patient("Again")).is_err(), "the same result cannot be used twice");
    // the doctor approves straight away and the PDF can be made
    let o = s.approve(&doctor, o.id, o.version, None).unwrap();
    assert_eq!(o.status, "Completed");
    let pdf = s.report_pdf(&doctor, o.id, true).unwrap();
    assert!(pdf.pdf.starts_with(b"%PDF"));
    // running it again tops up nothing and does not duplicate the test
    s.ensure_analyzer_test(&admin()).unwrap();
    assert_eq!(s.tests().unwrap().iter().filter(|t| t.code == "CBCA").count(), 1);
}

#[test]
fn lines_that_are_calculated_do_not_block_approval_when_their_inputs_are_missing() {
    // a 3-part analyzer never gives the five-part differential, so formula lines stay empty
    let mut s = Store::open_in_memory().unwrap();
    let t = s.save_test(&admin(), &TestDef { id: 0, code: "T3".into(), name: "Three part".into(), department: "HAEMATOLOGY".into(), sample: "EDTA".into(), price_paise: 100, disabled: false,
        config: TestConfig { parameters: vec![
            Parameter { id: "a".into(), name: "A".into(), field: "Numeric".into(), ..Default::default() },
            Parameter { id: "b".into(), name: "B".into(), field: "Numeric".into(), formula: "<C> * 2".into(), ..Default::default() },
            Parameter { id: "c".into(), name: "C".into(), field: "Numeric".into(), ..Default::default() }], ..Default::default() } }).unwrap();
    let o = s.register(&desk(), NewOrder { patient: NewPatient { address: "x".into(), ..new_patient("Q") }, items: vec![OrderItem { test_id: t.id, price_paise: None, discount_paise: 0 }], ..Default::default() }).unwrap();
    let tid = o.tests[0].id;
    s.save_results(&Actor::new("Tech", Role::Technician), o.id, o.version, &[ResultInput { order_test_id: tid, parameter_id: "a".into(), value: "5".into(), comment: None, manual: false }], None).unwrap();
    let o = s.order(o.id).unwrap();
    s.approve(&Actor::new("Doc", Role::Pathologist), o.id, o.version, None).unwrap();
}

#[test]
fn imported_gender_labels_are_not_printed_in_the_range() {
    use labdesk_core::calc::strip_gender_label;
    assert_eq!(strip_gender_label("Female: 50 - 500"), "50 - 500");
    assert_eq!(strip_gender_label("Male:<150"), "<150");
    assert_eq!(strip_gender_label("Male: Men: 0.2 - 2.3\nChildren: 0.2 - 0.9"), "Men: 0.2 - 2.3\nChildren: 0.2 - 0.9");
    assert_eq!(strip_gender_label("Males are 5-9"), "Males are 5-9");
    assert_eq!(strip_gender_label("13 - 17"), "13 - 17");
}

#[test]
fn only_what_ut_lis_prints_is_shown_and_five_part_tests_are_never_filled_by_guesswork() {
    let mut s = Store::open_in_memory().unwrap();
    let id = s.save_machine_message(SAMPLE).unwrap().unwrap();
    assert_eq!(s.machine_inbox(false, 10).unwrap()[0].items, 20, "the 20 lines the analyzer prints, not the extra PDW lines");
    // a five-part style CBC with the usual names is not offered for attaching
    let five = TestDef { id: 0, code: "CBC5".into(), name: "Complete Blood Count (CBC)".into(), department: "HAEMATOLOGY".into(), sample: "EDTA".into(), price_paise: 100, disabled: false,
        config: TestConfig { parameters: ["Haemoglobin", "Total Leucocyte Count", "Neutrophils", "Lymphocytes", "Monocytes", "Platelet Count"].iter().map(|n|
            Parameter { id: n.to_lowercase().replace(' ', "_"), name: n.to_string(), field: "Numeric".into(), ..Default::default() }).collect(), ..Default::default() } };
    let t = s.save_test(&admin(), &five).unwrap();
    s.register(&desk(), NewOrder { patient: NewPatient { address: "x".into(), ..new_patient("Sample") }, items: vec![OrderItem { test_id: t.id, price_paise: None, discount_paise: 0 }], ..Default::default() }).unwrap();
    assert!(s.machine_candidates(id).unwrap().is_empty());
}

#[test]
fn a_lone_end_of_report_line_never_goes_to_a_second_page() {
    use labdesk_core::report::{render, render_at_scale, ReportInput};
    let (mut s, o) = lab_with_analyzer_test("Sample");
    let id = s.save_machine_message(SAMPLE).unwrap().unwrap();
    let tid = s.machine_candidates(id).unwrap()[0].order_test_id;
    let done = s.attach_machine_result(&admin(), id, tid).unwrap();
    let doc = Actor::new("Dr Path", Role::Pathologist);
    s.approve(&doc, o.id, done.version, None).unwrap();
    let order = s.order(o.id).unwrap();
    let base = Profile { name: "Lab".into(), ..Default::default() };
    // find a letterhead margin where the report at normal size just spills over to a second page
    let mut found = None;
    for bottom in 28..90 {
        let p = Profile { bottom_mm: bottom as f32, ..base.clone() };
        let input = ReportInput { order: &order, profile: &p, letterhead: None, signature: None, use_letterhead: false };
        if render_at_scale(&input, 1.0).unwrap().pages == 2 { found = Some((bottom, render(&input).unwrap().pages)); break; }
    }
    let (bottom, pages) = found.expect("some margin makes the normal size spill over");
    assert_eq!(pages, 1, "with {bottom} mm at the bottom the report shrinks a little and stays on one page");
    // a smaller chosen size also fits more easily
    let p = Profile { bottom_mm: bottom as f32, text_size: 80, ..base };
    let input = ReportInput { order: &order, profile: &p, letterhead: None, signature: None, use_letterhead: false };
    assert_eq!(render(&input).unwrap().pages, 1);
}
