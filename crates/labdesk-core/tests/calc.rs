// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

use labdesk_core::calc::*;
use labdesk_core::model::*;
use std::collections::HashMap;

fn vals(pairs: &[(&str, &str)]) -> HashMap<String, String> { pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
fn patient(age: f64, age_type: &str, gender: &str) -> Patient {
    Patient { id: 1, patient_code: "x".into(), designation: "MR.".into(), first_name: "A".into(), last_name: "B".into(), age, age_type: age_type.into(), gender: gender.into(), phone: "".into(), email: "".into(), address: "a".into() }
}
fn param(id: &str, name: &str) -> Parameter { Parameter { id: id.into(), name: name.into(), field: "Numeric".into(), ..Default::default() } }

#[test]
fn formulas() {
    let v = vals(&[("Hb", "14"), ("RBC", "4.5")]);
    assert_eq!(calculate("<Hb> / <RBC> * 10", &v).unwrap(), "31.11");
    assert_eq!(calculate("round(<Hb> / 3, 1)", &v).unwrap(), "4.7");
    assert_eq!(calculate("2 + 3 * 4", &v).unwrap(), "14");
    assert_eq!(calculate("2 ^ 3 ^ 2", &v).unwrap(), "512");
    assert_eq!(calculate("max(1, 7, 3) + min(4, 2)", &v).unwrap(), "9");
    assert_eq!(calculate("<Hb", &v), Err(CalcError("Unsupported formula")));
    assert_eq!(calculate("<Nope> + 1", &v), Err(CalcError("Missing formula input")));
    assert!(calculate("1 / 0", &v).is_err());
    assert!(calculate("system(1)", &v).is_err()); // only whitelisted functions
    assert!(calculate("1; 2", &v).is_err());
}

#[test]
fn name_with_underscore_matches_spaces() {
    let v = vals(&[("Total Protein", "7")]);
    assert_eq!(calculate("<Total_Protein> * 2", &v).unwrap(), "14");
}

#[test]
fn ranges_flags_by_age_and_sex() {
    let mut hb = param("hb", "Hb");
    hb.gender_ranges.insert("Male".into(), RangeOverlay { low: Some(13.0), high: Some(17.0), ..Default::default() });
    hb.gender_ranges.insert("Female".into(), RangeOverlay { low: Some(12.0), high: Some(15.0), ..Default::default() });
    hb.gender_ranges.insert("Child".into(), RangeOverlay { low: Some(11.0), high: Some(14.0), ..Default::default() });
    hb.critical_low = Some(7.0);
    let m = patient(40.0, "Year", "Male");
    assert_eq!(flag_for(&hb, "12.5", &m), "L");
    assert_eq!(flag_for(&hb, "12.5", &patient(40.0, "Year", "Female")), "");
    assert_eq!(flag_for(&hb, "15.5", &patient(40.0, "Year", "Female")), "H");
    assert_eq!(flag_for(&hb, "6.9", &m), "Critical");
    assert_eq!(flag_for(&hb, "", &m), "");
    assert_eq!(flag_for(&hb, "abc", &m), "");
    assert_eq!(printable_range(&hb, &m), "13 - 17");
    // "Other" gender under 18 years falls back to the Child range.
    assert_eq!(printable_range(&hb, &patient(5.0, "Year", "Other")), "11 - 14");
    assert_eq!(printable_range(&hb, &patient(40.0, "Year", "Other")), "");
}

#[test]
fn exclusive_bounds_and_age_bands() {
    let mut p = param("x", "X");
    p.ranges = vec![
        RangeOverlay { age_max_days: Some(30.0), low: Some(10.0), high: Some(20.0), ..Default::default() },
        RangeOverlay { age_min_days: Some(30.5), low: Some(5.0), high: Some(8.0), low_exclusive: Some(true), ..Default::default() },
    ];
    assert_eq!(printable_range(&p, &patient(10.0, "Day", "Male")), "10 - 20");
    let adult = patient(30.0, "Year", "Male");
    assert_eq!(flag_for(&p, "5", &adult), "L"); // exclusive low bound: equal counts as low
    assert_eq!(flag_for(&p, "6", &adult), "");
    assert_eq!(printable_range(&Parameter { low: Some(5.0), low_exclusive: true, ..param("a", "A") }, &adult), "> 5");
    assert_eq!(printable_range(&Parameter { high: Some(200.0), ..param("a", "A") }, &adult), "≤ 200");
}

#[test]
fn dependent_formulas_resolve_in_order_and_manual_override_is_kept() {
    let mut mcv = param("mcv", "MCV"); mcv.formula = "<hct> / <rbc> * 10".into();
    let mut mch = param("mch", "MCH"); mch.formula = "<hb> / <rbc> * 10".into();
    let mut twice = param("twice", "Twice"); twice.formula = "<MCV> * 2".into(); // depends on another formula, listed after it
    let params = vec![param("hb", "Hb"), param("hct", "Hct"), param("rbc", "RBC"), twice.clone(), mcv.clone(), mch.clone()];
    let row = |id: &str, v: &str, manual: bool| ResultRow { parameter_id: id.into(), value: v.into(), flag: "".into(), range_text: None, comment: "".into(), manual };
    let out = calculate_results(&params, &[row("hb", "15", false), row("hct", "45", false), row("rbc", "5", false)]);
    let get = |id: &str| out.iter().find(|r| r.parameter_id == id).map(|r| r.value.clone());
    assert_eq!(get("mcv").as_deref(), Some("90"));
    assert_eq!(get("mch").as_deref(), Some("30"));
    assert_eq!(get("twice").as_deref(), Some("180"));
    // A technician-typed value on a formula field is kept when marked manual.
    let out = calculate_results(&params, &[row("hb", "15", false), row("hct", "45", false), row("rbc", "5", false), row("mcv", "88", true)]);
    assert_eq!(out.iter().find(|r| r.parameter_id == "mcv").unwrap().value, "88");
    // Missing input leaves the formula blank rather than failing.
    let out = calculate_results(&params, &[row("hb", "15", false)]);
    assert_eq!(out.iter().find(|r| r.parameter_id == "mcv").unwrap().value, "");
}
