// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Results from a blood cell counter (HL7 messages): reading them, the inbox, and attaching a result to a report.
//!
//! Nothing here approves anything. An attached result only fills in the typed values of a test (and keeps the graphs);
//! the doctor still checks and approves as usual.
use crate::calc::flag_for;
use crate::error::{check, Error, Result};
use crate::model::*;
use crate::store::{allow, audit, load_order, now, Store};
use rusqlite::{params, OptionalExtension};
use serde_json::json;

/// What the analyzer prints for each item: our result-line id, unit, decimals and the reference range it uses.
pub struct Item { pub code: &'static str, pub id: &'static str, pub unit: &'static str, pub dec: usize, pub low: f64, pub high: f64 }
pub const ITEMS: &[Item] = &[
    Item { code: "WBC", id: "wbc", unit: "10^9/L", dec: 2, low: 3.5, high: 9.5 },
    Item { code: "LYM%", id: "lym_p", unit: "%", dec: 2, low: 20.0, high: 50.0 },
    Item { code: "MID%", id: "mid_p", unit: "%", dec: 2, low: 3.0, high: 10.0 },
    Item { code: "GRAN%", id: "gran_p", unit: "%", dec: 2, low: 50.0, high: 75.0 },
    Item { code: "LYM#", id: "lym_n", unit: "10^9/L", dec: 3, low: 0.8, high: 4.0 },
    Item { code: "MID#", id: "mid_n", unit: "10^9/L", dec: 3, low: 0.12, high: 1.2 },
    Item { code: "GRAN#", id: "gran_n", unit: "10^9/L", dec: 3, low: 2.0, high: 7.0 },
    Item { code: "RBC", id: "rbc", unit: "10^12/L", dec: 2, low: 3.8, high: 5.1 },
    Item { code: "HGB", id: "hgb", unit: "g/dL", dec: 1, low: 11.5, high: 15.0 },
    Item { code: "HCT", id: "hct", unit: "%", dec: 1, low: 35.0, high: 45.0 },
    Item { code: "MCV", id: "mcv", unit: "fL", dec: 1, low: 82.0, high: 100.0 },
    Item { code: "MCH", id: "mch", unit: "pg", dec: 1, low: 27.0, high: 34.0 },
    Item { code: "MCHC", id: "mchc", unit: "g/dL", dec: 0, low: 32.0, high: 35.0 },
    Item { code: "RDW_SD", id: "rdw_sd", unit: "fL", dec: 1, low: 37.0, high: 54.0 },
    Item { code: "RDW_CV", id: "rdw_cv", unit: "%", dec: 1, low: 11.5, high: 14.5 },
    Item { code: "PLT", id: "plt", unit: "10^9/L", dec: 0, low: 125.0, high: 350.0 },
    Item { code: "MPV", id: "mpv", unit: "fL", dec: 1, low: 7.4, high: 10.4 },
    Item { code: "PCT", id: "pct", unit: "%", dec: 2, low: 0.10, high: 0.28 },
    Item { code: "P_LCR", id: "p_lcr", unit: "%", dec: 2, low: 13.0, high: 43.0 },
    Item { code: "P_LCC", id: "p_lcc", unit: "10^9/L", dec: 0, low: 13.0, high: 129.0 },
];

/// The analyzer item a result line of a test belongs to. Only the ready-made analyzer test (CBCA) uses these ids:
/// other blood count tests (for example the five-part CBC) are never filled with 3-part values by guesswork.
pub fn item_for(p: &Parameter) -> Option<&'static Item> { ITEMS.iter().find(|i| i.id == p.id) }
fn item_of_code(code: &str) -> Option<&'static Item> { ITEMS.iter().find(|i| i.code.eq_ignore_ascii_case(code)) }

/// "3.50~9.50" as the analyzer writes it.
pub fn analyzer_range(i: &Item) -> String { format!("{:.d$}~{:.d$}", i.low, i.high, d = i.dec) }
/// "H" / "L" from a printed range like "3.50~9.50" (empty when the value is inside or the text cannot be read).
pub fn flag_from_range(value: &str, range: &str) -> &'static str {
    let Ok(v) = value.trim().parse::<f64>() else { return "" };
    let Some((a, b)) = range.split_once('~').or_else(|| range.split_once(" - ")).or_else(|| range.split_once('-')) else { return "" };
    let (Ok(lo), Ok(hi)) = (a.trim().parse::<f64>(), b.trim().parse::<f64>()) else { return "" };
    if v < lo { "L" } else if v > hi { "H" } else { "" }
}

// ---------------- reading a message ----------------
fn field<'a>(seg: &'a [&'a str], i: usize) -> &'a str { seg.get(i).copied().unwrap_or("").trim() }
fn hex_bytes(h: &str) -> Vec<u8> {
    let h: Vec<u8> = h.bytes().filter(|b| b.is_ascii_hexdigit()).collect();
    h.chunks(2).filter(|c| c.len() == 2).filter_map(|c| u8::from_str_radix(std::str::from_utf8(c).ok()?, 16).ok()).collect()
}
fn stamp(t: &str) -> String {
    if t.len() >= 12 && t.chars().take(12).all(|c| c.is_ascii_digit()) { format!("{}-{}-{} {}:{}", &t[0..4], &t[4..6], &t[6..8], &t[8..10], &t[10..12]) } else { t.to_string() }
}

/// Read one HL7 result message. None when it is not a result message (for example a status or acknowledgement message).
pub fn parse_hl7(text: &str) -> Option<MachineData> {
    let segs: Vec<Vec<&str>> = text.split(['\r', '\n']).map(str::trim).filter(|l| l.len() > 3).map(|l| l.split('|').collect()).collect();
    let msh = segs.iter().find(|s| s[0] == "MSH")?;
    if !field(msh, 8).starts_with("ORU") { return None; }
    let mut d = MachineData::default();
    for s in &segs {
        match s[0] {
            "PID" => { d.patient_name = field(s, 5).replace('^', " ").split_whitespace().collect::<Vec<_>>().join(" "); d.age_text = field(s, 7).to_string(); d.sex = field(s, 8).to_string(); }
            "OBR" => { d.sample_id = field(s, 3).to_string(); d.test_time = stamp(field(s, 7)); }
            "OBX" => {
                let code = field(s, 3);
                if let Some((name, sub)) = code.split_once("Histogram") {
                    let name = name.trim().to_uppercase();
                    let h = match d.histograms.iter().position(|h| h.name == name) { Some(i) => i, None => { d.histograms.push(Histogram { name: name.clone(), ..Default::default() }); d.histograms.len() - 1 } };
                    if field(s, 2) == "ED" { if let Some(hex) = field(s, 5).split('^').nth(4) { d.histograms[h].data = hex_bytes(hex); } }
                    else if sub.contains("Line") { if let Ok(v) = field(s, 5).parse::<u16>() { d.histograms[h].lines.push(v); } }
                } else if !code.is_empty() && field(s, 2) == "NM" {
                    d.items.push(MachineItem { code: code.to_string(), value: field(s, 5).to_string(), unit: field(s, 6).to_string(), range: field(s, 7).to_string(), flag: field(s, 8).to_string() });
                }
            }
            _ => {}
        }
    }
    for h in &mut d.histograms { h.lines.sort_unstable(); }
    d.histograms.retain(|h| !h.data.is_empty());
    if d.items.is_empty() { return None; }
    Some(d)
}

/// The value as the analyzer prints it (fixed number of decimals per item).
pub fn shown_value(code: &str, raw: &str) -> String {
    match (item_of_code(code), raw.trim().parse::<f64>()) { (Some(i), Ok(v)) => format!("{:.d$}", v, d = i.dec), _ => raw.trim().to_string() }
}

// ---------------- graphs ----------------
/// Where a marker line sits on the printed axis, and the number written next to it (fL). Worked out from real analyzer prints.
pub fn line_label(hist: &str, channel: u16) -> i32 {
    let c = channel as f64;
    match hist { "WBC" => (1.356 * c + 1.59).round() as i32, "RBC" => (1.0859 * c - 0.5).round() as i32, _ => (0.149 * c + 0.4).round() as i32 }
}
/// The other way round: which position (0-255) a value on the printed scale belongs to.
pub fn channel_of(hist: &str, fl: f64) -> f64 {
    match hist { "WBC" => (fl - 1.59) / 1.356, "RBC" => (fl + 0.5) / 1.0859, _ => (fl - 0.4) / 0.149 }
}
/// How much of the 0-255 width the analyzer actually draws (it cuts the long tail of the platelet graph).
pub fn visible_fraction(hist: &str) -> f32 { if hist == "PLT" { 0.80 } else { 1.0 } }
/// (axis maximum, tick step) of the printed graph.
pub fn axis(hist: &str) -> (f64, f64) { match hist { "WBC" => (347.0, 50.0), "RBC" => (278.0, 50.0), _ => (31.0, 5.0) } }

// ---------------- the inbox ----------------
#[derive(Debug, Clone, PartialEq)]
pub struct MachineEntry { pub id: i64, pub received_at: String, pub status: String, pub sample_id: String, pub patient_name: String, pub test_time: String, pub items: usize, pub abnormal: usize, pub order_id: Option<i64> }
#[derive(Debug, Clone, PartialEq)]
pub struct MachineCandidate { pub order_id: i64, pub order_test_id: i64, pub patient_name: String, pub patient_code: String, pub test_name: String, pub registered: String, pub score: u32 }

fn words(s: &str) -> Vec<String> {
    const TITLES: [&str; 12] = ["mr", "mrs", "ms", "miss", "dr", "baby", "master", "shri", "smt", "late", "mst", "prof"];
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty() && !TITLES.contains(w)).map(str::to_string).collect()
}

impl Store {
    pub fn machine_range_mode(&self) -> Result<String> {
        let v: Option<String> = self.conn.query_row("SELECT value FROM settings WHERE key='machine_ranges'", [], |r| r.get(0)).optional()?;
        Ok(match v.as_deref() { Some("labdesk") => "labdesk", _ => "analyzer" }.to_string())
    }
    /// "analyzer" prints the ranges and H/L marks exactly as the analyzer does; "labdesk" uses the lab's own ranges (by age and gender).
    /// Results that are still waiting for approval are switched at once.
    pub fn set_machine_range_mode(&mut self, actor: &Actor, mode: &str) -> Result<()> {
        allow(actor, &[Role::Admin, Role::Technician, Role::Pathologist])?;
        check(mode == "analyzer" || mode == "labdesk", "Unknown range choice")?;
        self.conn.execute("INSERT INTO settings(key,value) VALUES('machine_ranges',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [mode])?;
        let open: Vec<(i64, i64)> = {
            let mut st = self.conn.prepare("SELECT m.id,m.order_test_id FROM machine_results m JOIN order_tests t ON t.id=m.order_test_id JOIN orders o ON o.id=t.order_id
                WHERE m.status='Attached' AND t.approved_at IS NULL AND o.status<>'Delivered' AND o.deleted_at IS NULL")?;
            let v = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
            v
        };
        for (_, tid) in open { self.apply_machine_values(tid, mode, false)?; }
        audit(&self.conn, actor, "machine_ranges", "setting", 0, json!({ "mode": mode }))?;
        Ok(())
    }

    /// Whether LabDesk should listen for the analyzer all the time, and on which port.
    pub fn machine_auto(&self) -> Result<(bool, u16)> {
        let get = |k: &str| -> Result<Option<String>> { Ok(self.conn.query_row("SELECT value FROM settings WHERE key=?", [k], |r| r.get(0)).optional()?) };
        Ok((get("machine_auto")?.as_deref() == Some("1"), get("machine_port")?.and_then(|p| p.parse().ok()).unwrap_or(5000)))
    }
    pub fn set_machine_auto(&mut self, actor: &Actor, on: bool, port: u16) -> Result<()> {
        allow(actor, &[Role::Admin])?;
        check(port >= 1024, "Please choose a port number of 1024 or more")?;
        for (k, v) in [("machine_auto", if on { "1".to_string() } else { "0".to_string() }), ("machine_port", port.to_string())] {
            self.conn.execute("INSERT INTO settings(key,value) VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![k, v])?;
        }
        audit(&self.conn, actor, "machine_auto", "setting", 0, json!({ "on": on, "port": port }))?;
        Ok(())
    }

    /// Keep a message that arrived from the analyzer. A repeat of the same sample is not stored twice.
    /// Returns None when it was not a result message. (No password or person is involved: it is the machine talking.)
    pub fn save_machine_message(&mut self, raw: &str) -> Result<Option<i64>> {
        let Some(d) = parse_hl7(raw) else { return Ok(None) };
        let dup: Option<i64> = self.conn.query_row("SELECT id FROM machine_results WHERE sample_id=? AND test_time=? AND patient_name=?", params![d.sample_id, d.test_time, d.patient_name], |r| r.get(0)).optional()?;
        if let Some(id) = dup { return Ok(Some(id)); }
        self.conn.execute("INSERT INTO machine_results(received_at,sample_id,patient_name,test_time,status,payload,raw) VALUES(?,?,?,?,'New',?,?)",
            params![now(), d.sample_id, d.patient_name, d.test_time, serde_json::to_string(&d)?, raw])?;
        let id = self.conn.last_insert_rowid();
        audit(&self.conn, &Actor::new("Analyzer", Role::Technician), "machine_result", "machine_result", id, json!({ "sample": d.sample_id }))?;
        Ok(Some(id))
    }

    /// Newest first. `only_new` leaves out results that were already attached or dismissed.
    pub fn machine_inbox(&self, only_new: bool, limit: usize) -> Result<Vec<MachineEntry>> {
        let mut st = self.conn.prepare("SELECT m.id,m.received_at,m.status,m.sample_id,m.patient_name,m.test_time,m.payload,t.order_id FROM machine_results m LEFT JOIN order_tests t ON t.id=m.order_test_id
            WHERE (?1=0 OR m.status='New') ORDER BY m.id DESC LIMIT ?2")?;
        let rows = st.query_map(params![only_new as i64, limit as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?, r.get::<_, String>(6)?, r.get::<_, Option<i64>>(7)?)))?;
        let mut out = Vec::new();
        for r in rows {
            let (id, received_at, status, sample_id, patient_name, test_time, payload, order_id) = r?;
            let d: MachineData = serde_json::from_str(&payload)?;
            let shown: Vec<&MachineItem> = d.items.iter().filter(|i| item_of_code(&i.code).is_some()).collect();
            let abnormal = shown.iter().filter(|i| analyzer_flag(i) != "").count();
            out.push(MachineEntry { id, received_at, status, sample_id, patient_name, test_time, items: shown.len(), abnormal, order_id });
        }
        Ok(out)
    }
    pub fn machine_data(&self, id: i64) -> Result<MachineData> {
        let p: String = self.conn.query_row("SELECT payload FROM machine_results WHERE id=?", [id], |r| r.get(0)).optional()?.ok_or_else(|| Error::NotFound("Machine result not found".into()))?;
        Ok(serde_json::from_str(&p)?)
    }
    pub fn machine_new_count(&self) -> Result<i64> { Ok(self.conn.query_row("SELECT COUNT(*) FROM machine_results WHERE status='New'", [], |r| r.get(0))?) }

    /// Tests that could take this result: not yet approved, not deleted or delivered, with lines the analyzer fills. Best name match first.
    pub fn machine_candidates(&self, id: i64) -> Result<Vec<MachineCandidate>> {
        let d = self.machine_data(id)?;
        let theirs = words(&d.patient_name);
        let mut st = self.conn.prepare("SELECT o.id,t.id,o.created_at,t.snapshot FROM order_tests t JOIN orders o ON o.id=t.order_id
            WHERE t.approved_at IS NULL AND o.deleted_at IS NULL AND o.status<>'Delivered' ORDER BY o.id DESC LIMIT 400")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
        let mut out = Vec::new();
        for (oid, tid, created, snap) in rows {
            let v: serde_json::Value = serde_json::from_str(&snap)?;
            let cfg: TestConfig = serde_json::from_value(v["config"].clone())?;
            if cfg.parameters.iter().filter(|p| item_for(p).is_some()).count() < 3 { continue; }
            let o = load_order(&self.conn, oid)?;
            let mine = words(&format!("{} {}", o.patient.first_name, o.patient.last_name));
            let hit = theirs.iter().filter(|w| mine.contains(w)).count() as u32;
            let score = if theirs.is_empty() { 0 } else { hit * 100 / theirs.len() as u32 };
            out.push(MachineCandidate { order_id: oid, order_test_id: tid, patient_name: format!("{} {} {}", o.patient.designation, o.patient.first_name, o.patient.last_name).split_whitespace().collect::<Vec<_>>().join(" "),
                patient_code: o.patient.patient_code.clone(), test_name: v["name"].as_str().unwrap_or("").to_string(), registered: created, score });
        }
        out.sort_by(|a, b| b.score.cmp(&a.score).then(b.registered.cmp(&a.registered)));
        out.truncate(30);
        Ok(out)
    }

    /// Put the analyzer's values into a test of a report (and keep its graphs with it). The doctor still approves.
    pub fn attach_machine_result(&mut self, actor: &Actor, id: i64, order_test_id: i64) -> Result<Order> {
        allow(actor, &[Role::Admin, Role::Technician, Role::Pathologist])?;
        let status: String = self.conn.query_row("SELECT status FROM machine_results WHERE id=?", [id], |r| r.get(0)).optional()?.ok_or_else(|| Error::NotFound("Machine result not found".into()))?;
        check(status != "Attached", "This result was already attached to a report")?;
        let order_id: i64 = self.conn.query_row("SELECT order_id FROM order_tests WHERE id=?", [order_test_id], |r| r.get(0)).optional()?.ok_or_else(|| Error::NotFound("Test not found".into()))?;
        let o = load_order(&self.conn, order_id)?;
        check(o.deleted_at.is_none(), "This report was deleted. Restore it first.")?;
        check(o.status != "Delivered", "Delivered reports are permanently locked.")?;
        let t = o.tests.iter().find(|t| t.id == order_test_id).ok_or_else(|| Error::NotFound("Test not found".into()))?;
        check(!t.is_approved(), "This test is already approved. Start a correction first.")?;
        check(t.machine.is_none(), "This test already has a machine result")?;
        check(t.config.parameters.iter().filter(|p| item_for(p).is_some()).count() >= 3, "This test has no lines that the analyzer fills (WBC, HGB, PLT …)")?;
        self.conn.execute("UPDATE machine_results SET status='Attached',order_test_id=? WHERE id=?", params![order_test_id, id])?;
        let mode = self.machine_range_mode()?;
        let filled = self.apply_machine_values(order_test_id, &mode, true)?;
        let tx = self.conn.transaction()?;
        tx.execute("UPDATE order_tests SET status='Partial' WHERE id=? AND status='Pending'", [order_test_id])?;
        tx.execute("UPDATE orders SET version=version+1, status=CASE WHEN status='Pending' THEN 'Partial' ELSE status END WHERE id=?", [order_id])?;
        audit(&tx, actor, "machine_attach", "order", order_id, json!({ "machine_result": id, "lines": filled }))?;
        let out = load_order(&tx, order_id)?;
        tx.commit()?;
        Ok(out)
    }

    pub fn dismiss_machine_result(&mut self, actor: &Actor, id: i64) -> Result<()> {
        allow(actor, &[Role::Admin, Role::Technician, Role::Pathologist, Role::Reception])?;
        let n = self.conn.execute("UPDATE machine_results SET status='Dismissed' WHERE id=? AND status='New'", [id])?;
        check(n == 1, "This result is not waiting any more")?;
        audit(&self.conn, actor, "machine_dismiss", "machine_result", id, json!({}))?;
        Ok(())
    }

    /// Write the attached values (value, range text and mark) into the result lines. Returns how many lines were filled.
    fn apply_machine_values(&mut self, order_test_id: i64, mode: &str, first: bool) -> Result<usize> {
        let payload: String = self.conn.query_row("SELECT payload FROM machine_results WHERE order_test_id=? AND status='Attached' ORDER BY id DESC LIMIT 1", [order_test_id], |r| r.get(0))?;
        let d: MachineData = serde_json::from_str(&payload)?;
        let snap: String = self.conn.query_row("SELECT snapshot FROM order_tests WHERE id=?", [order_test_id], |r| r.get(0))?;
        let v: serde_json::Value = serde_json::from_str(&snap)?;
        let cfg: TestConfig = serde_json::from_value(v["config"].clone())?;
        let order_id: i64 = self.conn.query_row("SELECT order_id FROM order_tests WHERE id=?", [order_test_id], |r| r.get(0))?;
        let patient = load_order(&self.conn, order_id)?.patient;
        let mut n = 0;
        for p in cfg.parameters.iter().filter(|p| p.is_result_field()) {
            let Some(item) = item_for(p) else { continue };
            let Some(m) = d.items.iter().find(|m| m.code.eq_ignore_ascii_case(item.code)) else { continue };
            let value = shown_value(item.code, &m.value);
            let (range, flag) = if mode == "labdesk" { (None, flag_for(p, &value, &patient).to_string()) }
                else { let r = analyzer_range(item); let f = flag_from_range(&value, &r).to_string(); (Some(r), f) };
            if first || self.conn.query_row("SELECT 1 FROM results WHERE order_test_id=? AND parameter_id=?", params![order_test_id, p.id], |r| r.get::<_, i64>(0)).optional()?.is_some() {
                self.conn.execute("INSERT INTO results(order_test_id,parameter_id,value,flag,range_text,comment,manual) VALUES(?,?,?,?,?,'',0)
                    ON CONFLICT(order_test_id,parameter_id) DO UPDATE SET value=CASE WHEN ?6 THEN excluded.value ELSE results.value END, flag=excluded.flag, range_text=excluded.range_text",
                    params![order_test_id, p.id, value, flag, range, first])?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// One step from an analyzer result to a report: register the patient (name, age and sex come from the
    /// analyzer unless the person changed them), add the ready-made analyzer blood count test and attach the values.
    /// The doctor still approves. Returns the new order.
    pub fn report_from_machine(&mut self, actor: &Actor, id: i64, patient: NewPatient) -> Result<Order> {
        allow(actor, &[Role::Admin, Role::Technician, Role::Pathologist, Role::Reception])?;
        let status: String = self.conn.query_row("SELECT status FROM machine_results WHERE id=?", [id], |r| r.get(0)).optional()?.ok_or_else(|| Error::NotFound("Machine result not found".into()))?;
        check(status == "New", "This result was already used or dismissed")?;
        // the steps below are the same ones the front desk and admin do by hand
        let admin = Actor::new(&actor.name, Role::Admin);
        let test = self.ensure_analyzer_test(&admin)?;
        let mut patient = patient;
        if patient.address.trim().is_empty() { patient.address = "-".into(); }
        let o = self.register(&admin, NewOrder { patient, items: vec![OrderItem { test_id: test.id, price_paise: None, discount_paise: 0 }], ..Default::default() })?;
        let tid = o.tests.first().map(|t| t.id).ok_or_else(|| Error::Rule("The report has no test".into()))?;
        self.attach_machine_result(actor, id, tid)
    }

    /// Create the ready-made "CBC (analyzer)" test, with the same lines and units the analyzer prints. Does nothing if it exists.
    pub fn ensure_analyzer_test(&mut self, actor: &Actor) -> Result<TestDef> {
        allow(actor, &[Role::Admin])?;
        if let Some(mut t) = self.tests()?.into_iter().find(|t| t.code == "CBCA") {
            // older copies of this test miss lines added later: add them at the end
            let missing: Vec<&Item> = ITEMS.iter().filter(|i| !t.config.parameters.iter().any(|p| p.id == i.id)).collect();
            if missing.is_empty() { return Ok(t); }
            for i in missing { t.config.parameters.push(Parameter { id: i.id.into(), name: i.code.into(), field: "Numeric".into(), unit: i.unit.into(), low: Some(i.low), high: Some(i.high), ..Default::default() }); }
            return self.save_test(actor, &t);
        }
        let gender = |m: (f64, f64), f: (f64, f64)| -> std::collections::BTreeMap<String, RangeOverlay> {
            let mut g = std::collections::BTreeMap::new();
            g.insert("Male".to_string(), RangeOverlay { low: Some(m.0), high: Some(m.1), ..Default::default() });
            g.insert("Female".to_string(), RangeOverlay { low: Some(f.0), high: Some(f.1), ..Default::default() });
            g
        };
        let parameters: Vec<Parameter> = ITEMS.iter().map(|i| {
            let mut p = Parameter { id: i.id.into(), name: i.code.into(), field: "Numeric".into(), unit: i.unit.into(), low: Some(i.low), high: Some(i.high), required: false, ..Default::default() };
            match i.id {
                "hgb" => { p.gender_ranges = gender((13.0, 17.0), (12.0, 15.0)); }
                "rbc" => { p.gender_ranges = gender((4.5, 5.9), (3.8, 5.1)); }
                "hct" => { p.gender_ranges = gender((40.0, 52.0), (36.0, 46.0)); }
                _ => {}
            }
            p
        }).collect();
        self.save_test(actor, &TestDef { id: 0, code: "CBCA".into(), name: "Complete Blood Count (analyzer)".into(), department: "HAEMATOLOGY".into(), sample: "EDTA".into(), price_paise: 20000, disabled: false,
            config: TestConfig { parameters, ..Default::default() } })
    }
}

/// The mark the analyzer shows next to an item.
pub fn analyzer_flag(m: &MachineItem) -> &'static str {
    match item_of_code(&m.code) { Some(i) => flag_from_range(&shown_value(&m.code, &m.value), &analyzer_range(i)), None => "" }
}
