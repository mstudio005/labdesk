// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Dev tool: reads parity cases (generated from the web app's calculations.mjs) and prints
//! mismatches against this crate's implementation. Usage: cargo run -p labdesk-core --example parity -- cases.json
use labdesk_core::calc::*;
use labdesk_core::model::*;
use serde_json::Value;
use std::collections::HashMap;
fn main() {
    let path = std::env::args().nth(1).expect("cases.json path");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let vals: HashMap<String, String> = v["vals"].as_object().unwrap().iter().map(|(k, x)| (k.clone(), x.as_str().unwrap().to_string())).collect();
    let (mut n, mut bad) = (0, 0);
    for c in v["exprs"].as_array().unwrap() {
        n += 1;
        let e = c["e"].as_str().unwrap();
        let got = calculate(e, &vals).unwrap_or_else(|_| "ERR".into());
        let want = c["r"].as_str().unwrap();
        if got != want { bad += 1; println!("EXPR  {e:?}: js={want:?} rust={got:?}"); }
    }
    let f = &v["flag"];
    let ps: Vec<Parameter> = serde_json::from_value(f["ps"].clone()).unwrap();
    let pats = f["pats"].as_array().unwrap();
    let values: Vec<String> = f["values"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().into()).collect();
    for c in f["cases"].as_array().unwrap() {
        let (pi, ai) = (c["pi"].as_u64().unwrap() as usize, c["ai"].as_u64().unwrap() as usize);
        let pa = &pats[ai];
        let patient = Patient { id: 0, patient_code: "".into(), designation: "".into(), first_name: "".into(), last_name: "".into(), age: pa["age"].as_f64().unwrap(), age_type: pa["age_type"].as_str().unwrap().into(), gender: pa["gender"].as_str().unwrap().into(), phone: "".into(), email: "".into(), address: "".into() };
        n += 1;
        let r = printable_range(&ps[pi], &patient);
        if r != c["range"].as_str().unwrap() { bad += 1; println!("RANGE p{pi} pat{ai}: js={:?} rust={r:?}", c["range"]); }
        for (i, val) in values.iter().enumerate() {
            n += 1;
            let g = flag_for(&ps[pi], val, &patient);
            if g != c["flags"][i].as_str().unwrap() { bad += 1; println!("FLAG p{pi} pat{ai} value={val:?}: js={:?} rust={g:?}", c["flags"][i]); }
        }
    }
    println!("{n} checks, {bad} mismatches");
}
