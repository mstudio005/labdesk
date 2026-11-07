// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Dev tool: write the sample report. Usage: cargo run -p labdesk-core --example sample_report -- OUT.pdf [letterhead.png] [signature.png]
use labdesk_core::report::*;
use labdesk_core::Profile;
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let lh = a.get(1).map(|p| std::fs::read(p).unwrap());
    let sig = a.get(2).map(|p| std::fs::read(p).unwrap());
    let profile = Profile { name: "Sample Clinical Laboratory".into(), address: "123 Health Avenue, City, State 000000".into(), phone: "+91 99060 00000".into(), email: "lab@example.com".into(),
        signatory: "Dr. Sample Pathologist".into(), qualification: "MD (Pathology) · Reg. No. 12345".into(), header_design: std::env::var("DESIGN").ok().and_then(|v| v.parse().ok()).unwrap_or(0), ..Default::default() };
    let mut order = sample_order();
    if std::env::var_os("MANY").is_some() {
        let base = order.tests.clone();
        for i in 0..5 { for t in &base { let mut c = t.clone(); c.id += 10 * (i + 1); c.name = format!("{} {}", t.name, i + 2); order.tests.push(c); } }
    }
    let out = render(&ReportInput { order: &order, profile: &profile, letterhead: lh.as_deref(), signature: sig.as_deref(), use_letterhead: true }).unwrap();
    std::fs::write(&a[0], &out.pdf).unwrap();
    println!("{} bytes, {} page(s), code {}", out.pdf.len(), out.pages, out.verification);
}
