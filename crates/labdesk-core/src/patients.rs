// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Finding a returning patient, a patient's full history, and checking a scanned report QR.
use crate::error::Result;
use crate::model::*;
use crate::report::verification_code;
use crate::store::{audit, allow, load_order, now, s, Store};
use rusqlite::params;

/// One line in a list of patients.
#[derive(Debug, Clone, PartialEq)]
pub struct PatientSummary {
    pub id: i64,
    pub patient_code: String,
    pub name: String,
    pub age_gender: String,
    pub phone: String,
    pub visits: i64,
    pub last_visit: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct VisitTest { pub name: String, pub status: String, pub entered: i64, pub abnormal: i64 }

/// One time the patient came to the lab.
#[derive(Debug, Clone, PartialEq)]
pub struct Visit {
    pub order_id: i64,
    pub created_at: String,
    pub status: String,
    pub total_paise: i64,
    pub paid_paise: i64,
    pub referral: String,
    pub age_text: String,
    pub tests: Vec<VisitTest>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TestCount { pub name: String, pub times: i64, pub last: String }

#[derive(Debug, Clone, PartialEq)]
pub struct PatientHistory {
    pub patient: Patient,
    pub first_visit: String,
    pub last_visit: String,
    /// Newest first.
    pub visits: Vec<Visit>,
    /// Most repeated first.
    pub test_counts: Vec<TestCount>,
    pub billed_paise: i64,
    pub paid_paise: i64,
    pub due_paise: i64,
}

/// One payment taken from a patient, with the bill it went towards.
#[derive(Debug, Clone, PartialEq)]
pub struct PaymentEntry { pub order_id: i64, pub created_at: String, pub method: String, pub reference: String, pub amount_paise: i64 }

/// What a scanned report QR turned out to be.
#[derive(Debug, Clone, PartialEq)]
pub struct Verification { pub order_id: i64, pub patient_name: String, pub genuine: bool }

fn full_name(d: &str, f: &str, l: &str) -> String { format!("{d} {f} {l}").split_whitespace().collect::<Vec<_>>().join(" ") }

impl Store {
    /// Patients (newest visit first) whose name, ID or phone contains `query`. Reports that were deleted do not count.
    pub fn patients(&self, query: &str, limit: usize) -> Result<Vec<PatientSummary>> {
        let like = format!("%{}%", s(query, 100));
        let mut st = self.conn.prepare("SELECT p.id,p.patient_code,p.designation,p.first_name,p.last_name,p.age,p.age_type,p.gender,p.phone,COUNT(o.id),MAX(o.created_at)
            FROM patients p JOIN orders o ON o.patient_id=p.id AND o.deleted_at IS NULL
            WHERE ?1='%%' OR p.first_name LIKE ?1 OR p.last_name LIKE ?1 OR (p.first_name||' '||p.last_name) LIKE ?1 OR p.patient_code LIKE ?1 OR p.phone LIKE ?1
            GROUP BY p.id ORDER BY MAX(o.created_at) DESC LIMIT ?2")?;
        let rows = st.query_map(params![like, limit as i64], |r| {
            let (age, ty): (f64, String) = (r.get(5)?, r.get(6)?);
            Ok(PatientSummary { id: r.get(0)?, patient_code: r.get(1)?, name: full_name(&r.get::<_, String>(2)?, &r.get::<_, String>(3)?, &r.get::<_, String>(4)?),
                age_gender: format!("{} {} / {}", age, ty.chars().next().map(|c| c.to_ascii_lowercase()).unwrap_or('y'), r.get::<_, String>(7)?),
                phone: r.get(8)?, visits: r.get(9)?, last_visit: r.get(10)? })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// People already registered who might be the one being typed in right now. Same phone number is the
    /// strongest sign; a name that starts the same way (first name, and last name if typed) is the other.
    /// Used for the "Is this the same person?" hint while registering. Best matches first.
    pub fn possible_matches(&self, first: &str, last: &str, phone: &str, limit: usize) -> Result<Vec<PatientSummary>> {
        let digits: String = phone.chars().filter(|c| c.is_ascii_digit()).collect();
        let tail = if digits.len() >= 7 { digits[digits.len().saturating_sub(10)..].to_string() } else { String::new() };
        let (f, l) = (s(first, 100), s(last, 100));
        let by_name = f.chars().count() >= 2;
        if tail.is_empty() && !by_name { return Ok(vec![]); }
        let mut st = self.conn.prepare("SELECT p.id,p.patient_code,p.designation,p.first_name,p.last_name,p.age,p.age_type,p.gender,p.phone,COUNT(o.id),MAX(o.created_at),
              (?1<>'' AND replace(replace(replace(p.phone,' ',''),'-',''),'+','') LIKE '%'||?1) AS phone_hit
            FROM patients p JOIN orders o ON o.patient_id=p.id AND o.deleted_at IS NULL
            WHERE (?1<>'' AND replace(replace(replace(p.phone,' ',''),'-',''),'+','') LIKE '%'||?1)
               OR (?2=1 AND p.first_name LIKE ?3||'%' AND (?4='' OR p.last_name LIKE ?4||'%'))
            GROUP BY p.id ORDER BY phone_hit DESC, MAX(o.created_at) DESC LIMIT ?5")?;
        let rows = st.query_map(params![tail, by_name as i64, f, l, limit as i64], |r| {
            let (age, ty): (f64, String) = (r.get(5)?, r.get(6)?);
            Ok(PatientSummary { id: r.get(0)?, patient_code: r.get(1)?, name: full_name(&r.get::<_, String>(2)?, &r.get::<_, String>(3)?, &r.get::<_, String>(4)?),
                age_gender: format!("{} {} / {}", age, ty.chars().next().map(|c| c.to_ascii_lowercase()).unwrap_or('y'), r.get::<_, String>(7)?),
                phone: r.get(8)?, visits: r.get(9)?, last_visit: r.get(10)? })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// What this patient still owes over all visits (deleted reports do not count).
    pub fn patient_due(&self, patient_id: i64) -> Result<i64> {
        Ok(self.conn.query_row("SELECT COALESCE(SUM(o.total_paise - COALESCE((SELECT SUM(amount_paise) FROM payments WHERE order_id=o.id),0)),0)
            FROM orders o WHERE o.patient_id=? AND o.deleted_at IS NULL", [patient_id], |r| r.get(0))?)
    }

    /// Take a payment from a patient who owes money on several visits: it settles the oldest visit first.
    /// Returns (visit id, amount applied) for each visit touched.
    pub fn receive_patient_payment(&mut self, actor: &Actor, patient_id: i64, amount_paise: i64, method: &str, reference: &str) -> Result<Vec<(i64, i64)>> {
        allow(actor, &[Role::Admin, Role::Reception])?;
        crate::error::check(["Cash", "UPI", "Card", "Bank"].contains(&method), "Choose a payment method")?;
        let tx = self.conn.transaction()?;
        let mut st = tx.prepare("SELECT o.id,o.total_paise - COALESCE((SELECT SUM(amount_paise) FROM payments WHERE order_id=o.id),0) AS due
            FROM orders o WHERE o.patient_id=? AND o.deleted_at IS NULL ORDER BY o.created_at,o.id")?;
        let owing: Vec<(i64, i64)> = st.query_map([patient_id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<Vec<_>, _>>()?.into_iter().filter(|x| x.1 > 0).collect();
        drop(st);
        let total: i64 = owing.iter().map(|x| x.1).sum();
        crate::error::check(amount_paise >= 1, "Please enter the amount received")?;
        crate::error::check(amount_paise <= total, "That is more than this patient owes")?;
        let (mut left, mut applied) = (amount_paise, Vec::new());
        for (id, due) in owing {
            if left == 0 { break; }
            let part = left.min(due);
            tx.execute("INSERT INTO payments(order_id,amount_paise,method,created_at,reference) VALUES(?,?,?,?,?)", params![id, part, method, now(), s(reference, 200)])?;
            audit(&tx, actor, "payment", "order", id, serde_json::json!({ "amount_paise": part, "method": method }))?;
            applied.push((id, part));
            left -= part;
        }
        tx.commit()?;
        Ok(applied)
    }

    /// Every payment this patient ever made (newest first), each with the bill it was for. Deleted reports are left out.
    pub fn patient_payments(&self, patient_id: i64) -> Result<Vec<PaymentEntry>> {
        let mut st = self.conn.prepare("SELECT pa.order_id,pa.created_at,pa.method,pa.reference,pa.amount_paise FROM payments pa
            JOIN orders o ON o.id=pa.order_id WHERE o.patient_id=? AND o.deleted_at IS NULL ORDER BY pa.created_at DESC, pa.id DESC")?;
        let rows = st.query_map([patient_id], |r| Ok(PaymentEntry { order_id: r.get(0)?, created_at: r.get(1)?, method: r.get(2)?, reference: r.get(3)?, amount_paise: r.get(4)? }))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Every visit of one patient with the tests done, plus counts and totals.
    pub fn patient_history(&self, patient_id: i64) -> Result<PatientHistory> {
        let mut st = self.conn.prepare("SELECT id FROM orders WHERE patient_id=? AND deleted_at IS NULL ORDER BY created_at DESC, id DESC")?;
        let ids = st.query_map([patient_id], |r| r.get::<_, i64>(0))?.collect::<std::result::Result<Vec<_>, _>>()?;
        if ids.is_empty() { return Err(crate::error::Error::NotFound("Patient not found".into())); }
        let mut visits = Vec::new();
        let mut counts: Vec<TestCount> = Vec::new();
        let mut patient = None;
        let (mut billed, mut paid) = (0, 0);
        for id in ids {
            let o = load_order(&self.conn, id)?;
            billed += o.total_paise; paid += o.paid_paise();
            let tests: Vec<VisitTest> = o.tests.iter().map(|t| VisitTest { name: t.name.clone(), status: t.status.clone(),
                entered: t.results.iter().filter(|r| !r.value.trim().is_empty()).count() as i64,
                abnormal: t.results.iter().filter(|r| !r.value.trim().is_empty() && !r.flag.is_empty()).count() as i64 }).collect();
            for t in &tests {
                match counts.iter_mut().find(|c| c.name == t.name) {
                    Some(c) => { c.times += 1; if o.created_at > c.last { c.last = o.created_at.clone(); } }
                    None => counts.push(TestCount { name: t.name.clone(), times: 1, last: o.created_at.clone() }),
                }
            }
            visits.push(Visit { order_id: id, created_at: o.created_at.clone(), status: o.status.clone(), total_paise: o.total_paise, paid_paise: o.paid_paise(), referral: o.referral_name.clone(),
                age_text: format!("{} {}", o.patient.age, o.patient.age_type.to_lowercase()), tests });
            if patient.is_none() { patient = Some(o.patient.clone()); }
        }
        counts.sort_by(|a, b| b.times.cmp(&a.times).then(b.last.cmp(&a.last)));
        let (first, last) = (visits.last().map(|v| v.created_at.clone()).unwrap_or_default(), visits.first().map(|v| v.created_at.clone()).unwrap_or_default());
        Ok(PatientHistory { patient: patient.expect("at least one visit"), first_visit: first, last_visit: last, visits, test_counts: counts, billed_paise: billed, paid_paise: paid, due_paise: billed - paid })
    }

    /// Look at the text a report's QR code holds (typed in, or "typed" by a USB scanner) and say which report it
    /// is and whether it still matches what the lab approved. None if it is not a LabDesk report or is not in this lab.
    pub fn verify_report_text(&self, text: &str) -> Result<Option<Verification>> {
        let get = |key: &str| text.split(|c| c == '|' || c == '\n').find_map(|l| l.trim().strip_prefix(key)).map(|v| v.trim().to_string());
        let (Some(no), Some(code)) = (get("Report no:"), get("Ref:")) else { return Ok(None) };
        let Ok(id) = no.parse::<i64>() else { return Ok(None) };
        let Ok(o) = load_order(&self.conn, id) else { return Ok(None) };
        if o.deleted_at.is_some() { return Ok(None); }
        if let Some(pid) = get("ID:") { if pid != o.patient.patient_code { return Ok(Some(Verification { order_id: id, patient_name: full_name(&o.patient.designation, &o.patient.first_name, &o.patient.last_name), genuine: false })); } }
        let genuine = o.tests.iter().any(|t| t.is_approved()) && verification_code(&o).eq_ignore_ascii_case(&code);
        Ok(Some(Verification { order_id: id, patient_name: full_name(&o.patient.designation, &o.patient.first_name, &o.patient.last_name), genuine }))
    }
}

/// One result line followed over the visits of a patient.
#[derive(Debug, Clone, PartialEq)]
pub struct Trend { pub name: String, pub unit: String, pub low: Option<f64>, pub high: Option<f64>, pub points: Vec<TrendPoint> }
#[derive(Debug, Clone, PartialEq)]
pub struct TrendPoint { pub when: String, pub value: f64, pub flag: String }

impl Store {
    /// The numeric results of approved tests, grouped by result name, for every visit of one patient (oldest first).
    /// Only lines that were measured at least twice are returned.
    pub fn trends(&self, patient_id: i64) -> Result<Vec<Trend>> {
        let mut st = self.conn.prepare("SELECT o.id,t.approved_snapshot,t.approved_at FROM order_tests t JOIN orders o ON o.id=t.order_id
            WHERE o.patient_id=? AND o.deleted_at IS NULL AND t.approved_snapshot IS NOT NULL ORDER BY t.approved_at,t.id")?;
        let rows = st.query_map([patient_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
        let mut out: Vec<Trend> = Vec::new();
        for (oid, snap, at) in rows {
            let t: OrderTest = serde_json::from_str(&snap)?;
            let patient = load_order(&self.conn, oid)?.patient;
            for p in t.config.parameters.iter().filter(|p| p.is_result_field() && p.is_numeric()) {
                let Some(r) = t.results.iter().find(|r| r.parameter_id == p.id) else { continue };
                let Ok(v) = r.value.trim().parse::<f64>() else { continue };
                if !v.is_finite() { continue; }
                let key = p.name.trim().to_lowercase();
                let ranged = crate::calc::range_for(p, &patient);
                let entry = match out.iter().position(|x| x.name.to_lowercase() == key) {
                    Some(i) => i,
                    None => { out.push(Trend { name: p.name.trim().to_string(), unit: p.unit.clone(), low: ranged.low, high: ranged.high, points: vec![] }); out.len() - 1 }
                };
                out[entry].low = ranged.low.or(out[entry].low); out[entry].high = ranged.high.or(out[entry].high);
                out[entry].points.push(TrendPoint { when: at.clone(), value: v, flag: r.flag.clone() });
            }
        }
        out.retain(|t| t.points.len() >= 2);
        Ok(out)
    }
}
