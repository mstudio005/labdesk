// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Speed test: fills a scratch database with many patients and reports, then times the everyday actions.
//! Run:  cargo run --release --example bench -- 10000
//! It uses a temporary database file that is deleted at the end. Your real LabDesk data is never touched.
use labdesk_core::*;
use std::time::Instant;

const CAT: &str = r#"{"tests":[
 {"code":"CBC","name":"Complete Blood Count","department":"HAEMATOLOGY","sample":"EDTA","price_paise":30000,"config":{"parameters":[
   {"id":"hb","name":"Hb","field":"Numeric","unit":"g/dL","required":true,"low":13,"high":17},
   {"id":"wbc","name":"WBC","field":"Numeric","unit":"/uL","required":true,"low":4000,"high":11000},
   {"id":"plt","name":"Platelets","field":"Numeric","unit":"/uL","required":true,"low":150000,"high":450000},
   {"id":"rbc","name":"RBC","field":"Numeric","unit":"m/uL","low":4.5,"high":5.9}]}},
 {"code":"FBS","name":"Fasting Blood Sugar","department":"BIOCHEMISTRY","sample":"Serum","price_paise":8000,"config":{"parameters":[
   {"id":"glu","name":"Glucose","field":"Numeric","unit":"mg/dL","required":true,"low":70,"high":100}]}}]}"#;

fn t(label: &str, start: Instant, n: usize) {
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    if n > 1 { println!("{label:<46} {ms:>9.1} ms total   {:>8.3} ms each  (x{n})", ms / n as f64); } else { println!("{label:<46} {ms:>9.1} ms"); }
}

fn main() {
    let n: usize = std::env::args().nth(1).and_then(|a| a.parse().ok()).unwrap_or(1000);
    let dir = std::env::temp_dir().join(format!("labdesk-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp folder");
    let db = dir.join("bench.db");
    let mut s = Store::open(&db).expect("open");
    let admin = Actor::new("Bench Admin", Role::Admin);
    let desk = Actor::new("Front", Role::Reception);
    s.import_catalogue(&admin, CAT).expect("catalogue");
    let tests = s.tests().expect("tests");
    let first = ["Ali", "Bilal", "Sara", "Nasir", "Ishrat", "Mudasir", "Zainab", "Aamir", "Rubina", "Tariq", "Insha", "Faizan"];
    let last = ["Khan", "Dar", "Lone", "Mir", "Bhat", "Wani", "Shah", "Malik", "Rather", "Ganai"];
    println!("LabDesk speed test with {n} reports (database file on disk)\n");

    let st = Instant::now();
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let o = s.register(&desk, NewOrder {
            patient: NewPatient { first_name: first[i % first.len()].into(), last_name: format!("{}{}", last[(i / 7) % last.len()], i / 50), age: (18 + i % 60) as f64, age_type: "Year".into(),
                gender: if i % 2 == 0 { "Male" } else { "Female" }.into(), address: "Srinagar".into(), phone: format!("99{:08}", i), ..Default::default() },
            items: tests.iter().map(|t| OrderItem { test_id: t.id, price_paise: None, discount_paise: 0 }).collect(),
            referral_doctor: if i % 3 == 0 { "Dr. Rashid".into() } else { String::new() },
            payments: vec![(20000, "Cash".into(), String::new())], ..Default::default() }).expect("register");
        ids.push(o.id);
    }
    t("Register patient + 2 tests + payment", st, n);

    let st = Instant::now();
    for (i, id) in ids.iter().enumerate() {
        let o = s.order(*id).expect("order");
        let mut inputs = Vec::new();
        for tst in &o.tests {
            for p in &tst.config.parameters {
                let v = match p.id.as_str() { "hb" => format!("{}", 11 + i % 6), "wbc" => format!("{}", 5000 + (i % 9) * 900), "plt" => format!("{}", 180000 + (i % 7) * 30000), "rbc" => "4.8".into(), _ => format!("{}", 80 + i % 40) };
                inputs.push(ResultInput { order_test_id: tst.id, parameter_id: p.id.clone(), value: v, comment: None, manual: false });
            }
        }
        let o = s.save_results(&admin, *id, o.version, &inputs, None).expect("save");
        s.approve(&admin, *id, o.version, None).expect("approve");
    }
    t("Type results + save + approve", st, n);

    let probe = ids[n / 2];
    let st = Instant::now(); for _ in 0..50 { s.orders("", 50).expect("orders"); } t("Reports list (latest 50)", st, 50);
    let st = Instant::now(); for _ in 0..50 { s.orders("Khan", 50).expect("orders"); } t("Reports search by name", st, 50);
    let st = Instant::now(); for _ in 0..50 { s.orders("9900", 50).expect("orders"); } t("Reports search by phone", st, 50);
    let st = Instant::now(); for _ in 0..20 { s.patients("", 300).expect("patients"); } t("Patients list (300 rows)", st, 20);
    let st = Instant::now(); for _ in 0..20 { s.patients("ali", 300).expect("patients"); } t("Patients search", st, 20);
    let st = Instant::now(); for _ in 0..50 { s.stats().expect("stats"); } t("Home screen numbers", st, 50);
    let pid = s.order(probe).expect("order").patient.id;
    let st = Instant::now(); for _ in 0..50 { s.patient_history(pid).expect("history"); } t("Open one patient's history", st, 50);
    let st = Instant::now(); for _ in 0..50 { s.order(probe).expect("order"); } t("Open one report", st, 50);

    let k = n.min(200);
    let st = Instant::now(); let mut bytes = 0; for id in ids.iter().take(k) { bytes += s.report_pdf(&admin, *id, true).expect("pdf").pdf.len(); } t("Make report PDF", st, k);
    println!("   (average report PDF size {} KB)", bytes / k / 1024);
    let st = Instant::now(); for id in ids.iter().take(k) { s.bill_pdf(&admin, *id, true).expect("bill"); } t("Make bill PDF", st, k);

    let st = Instant::now(); let _ = s.patient_bill_pdf(&admin, pid, true).expect("full bill"); t("Make one combined bill", st, 1);

    let copy = dir.join("backup.ldbk");
    let st = Instant::now(); s.backup_to(&copy).expect("backup"); t("Back up the whole database", st, 1);
    let size = |p: &std::path::Path| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0) as f64 / 1_048_576.0;
    println!("\nDatabase size: {:.1} MB   Backup size: {:.1} MB", size(&db), size(&copy));
    drop(s);
    let _ = std::fs::remove_dir_all(&dir);
}
