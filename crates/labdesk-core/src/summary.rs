// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Money and work summaries for a range of days, and CSV files (they open in Excel) for the accountant.
//! Days are Indian days (UTC+5:30), the same as the Home screen.
use crate::error::{check, Result};
use crate::model::*;
use crate::store::{allow, audit, Store};
use rusqlite::params;
use serde_json::json;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct ReferralLine { pub name: String, pub visits: i64, pub billed_paise: i64, pub received_paise: i64 }
pub struct Summary {
    pub from: String, pub to: String,
    pub visits: i64, pub billed_paise: i64, pub discount_paise: i64, pub received_paise: i64,
    /// What is still unpaid on the visits of these days.
    pub due_paise: i64,
    pub by_method: Vec<(String, i64)>,
    pub tests: Vec<(String, i64)>,
    pub referrals: Vec<ReferralLine>,
}

fn ist() -> chrono::FixedOffset { chrono::FixedOffset::east_opt(5 * 3600 + 1800).expect("valid offset") }
/// "2026-10-25" .. "2026-10-26" (both days included) as UTC time stamps [start, end).
fn bounds(from: &str, to: &str) -> Result<(String, String)> {
    let parse = |t: &str| chrono::NaiveDate::parse_from_str(t.trim(), "%Y-%m-%d").map_err(|_| crate::Error::Rule("Please give the date as year-month-day, for example 2026-10-25".into()));
    let (a, b) = (parse(from)?, parse(to)?);
    check(a <= b, "The first day is after the last day")?;
    check((b - a).num_days() <= 3660, "Please choose at most ten years")?;
    let at = |d: chrono::NaiveDate| d.and_hms_opt(0, 0, 0).expect("midnight").and_local_timezone(ist()).single().expect("fixed offset").with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    Ok((at(a), at(b + chrono::Duration::days(1))))
}
/// A UTC time stamp as an Indian date and time, "25/10/2026 14:05".
fn when(ts: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(ts).map(|d| d.with_timezone(&ist()).format("%d/%m/%Y %H:%M").to_string()).unwrap_or_else(|_| ts.to_string())
}
fn money(paise: i64) -> String { format!("{}.{:02}", paise / 100, (paise % 100).abs()) }
fn csv_cell(v: &str) -> String {
    // a cell starting with = + - @ would be run as a formula by Excel
    let safe = if v.starts_with(['=', '+', '@']) || (v.starts_with('-') && v.len() > 1 && !v[1..].chars().all(|c| c.is_ascii_digit() || c == '.')) { format!("'{v}") } else { v.to_string() };
    if safe.contains([',', '"', '\n', '\r']) { format!("\"{}\"", safe.replace('"', "\"\"")) } else { safe }
}
fn csv(rows: Vec<Vec<String>>) -> String {
    let mut out = String::from("\u{feff}"); // so Excel reads the letters correctly
    for r in rows { out.push_str(&r.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(",")); out.push_str("\r\n"); }
    out
}

impl Store {
    pub fn summary(&self, actor: &Actor, from: &str, to: &str) -> Result<Summary> {
        allow(actor, &[Role::Admin, Role::Reception])?;
        let (a, b) = bounds(from, to)?;
        let one = |sql: &str| -> Result<i64> { Ok(self.conn.query_row(sql, params![a, b], |r| r.get(0))?) };
        let visits = one("SELECT COUNT(*) FROM orders WHERE deleted_at IS NULL AND created_at>=?1 AND created_at<?2")?;
        let billed = one("SELECT COALESCE(SUM(total_paise),0) FROM orders WHERE deleted_at IS NULL AND created_at>=?1 AND created_at<?2")?;
        let discount = one("SELECT COALESCE(SUM(discount_paise),0) FROM orders WHERE deleted_at IS NULL AND created_at>=?1 AND created_at<?2")?;
        let received = one("SELECT COALESCE(SUM(p.amount_paise),0) FROM payments p JOIN orders o ON o.id=p.order_id WHERE o.deleted_at IS NULL AND p.created_at>=?1 AND p.created_at<?2")?;
        let paid_on_these = one("SELECT COALESCE(SUM(p.amount_paise),0) FROM payments p JOIN orders o ON o.id=p.order_id WHERE o.deleted_at IS NULL AND o.created_at>=?1 AND o.created_at<?2")?;
        let mut st = self.conn.prepare("SELECT p.method,SUM(p.amount_paise) FROM payments p JOIN orders o ON o.id=p.order_id WHERE o.deleted_at IS NULL AND p.created_at>=?1 AND p.created_at<?2 GROUP BY p.method ORDER BY 2 DESC")?;
        let by_method = st.query_map(params![a, b], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
        let mut tests: BTreeMap<String, i64> = BTreeMap::new();
        let mut st = self.conn.prepare("SELECT t.snapshot FROM order_tests t JOIN orders o ON o.id=t.order_id WHERE o.deleted_at IS NULL AND o.created_at>=?1 AND o.created_at<?2")?;
        for snap in st.query_map(params![a, b], |r| r.get::<_, String>(0))? {
            let v: serde_json::Value = serde_json::from_str(&snap?)?;
            *tests.entry(v["name"].as_str().unwrap_or("?").to_string()).or_default() += 1;
        }
        let mut tests: Vec<(String, i64)> = tests.into_iter().collect();
        tests.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
        let referrals = self.referral_lines(&a, &b)?;
        Ok(Summary { from: from.trim().into(), to: to.trim().into(), visits, billed_paise: billed, discount_paise: discount, received_paise: received, due_paise: billed - paid_on_these, by_method, tests, referrals })
    }

    pub(crate) fn referral_lines(&self, a: &str, b: &str) -> Result<Vec<ReferralLine>> {
        let mut st = self.conn.prepare("SELECT CASE WHEN o.referral_name='' THEN 'Self' ELSE o.referral_name END, COUNT(*), SUM(o.total_paise),
            COALESCE(SUM((SELECT SUM(p.amount_paise) FROM payments p WHERE p.order_id=o.id)),0)
            FROM orders o WHERE o.deleted_at IS NULL AND o.created_at>=?1 AND o.created_at<?2 GROUP BY 1 ORDER BY 3 DESC")?;
        let rows = st.query_map(params![a, b], |r| Ok(ReferralLine { name: r.get(0)?, visits: r.get(1)?, billed_paise: r.get(2)?, received_paise: r.get(3)? }))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// A CSV file (as text) for Excel. `what` is "bills", "payments", "patients", "doctors" or "tests".
    pub fn export_csv(&self, actor: &Actor, what: &str, from: &str, to: &str) -> Result<String> {
        allow(actor, &[Role::Admin, Role::Reception])?;
        let (a, b) = bounds(from, to)?;
        let rows: Vec<Vec<String>> = match what {
            "bills" => {
                let mut r = vec![["Date", "Report no", "Patient ID", "Patient", "Age", "Gender", "Phone", "Referred by", "Tests", "Total", "Discount", "Paid", "Due", "Status"].iter().map(|s| s.to_string()).collect::<Vec<_>>()];
                let mut st = self.conn.prepare("SELECT o.id,o.created_at,p.patient_code,p.designation||' '||p.first_name||' '||p.last_name,COALESCE(o.age,p.age),COALESCE(o.age_type,p.age_type),p.gender,p.phone,o.referral_name,o.total_paise,o.discount_paise,
                    COALESCE((SELECT SUM(x.amount_paise) FROM payments x WHERE x.order_id=o.id),0),o.status
                    FROM orders o JOIN patients p ON p.id=o.patient_id WHERE o.deleted_at IS NULL AND o.created_at>=?1 AND o.created_at<?2 ORDER BY o.id")?;
                let list = st.query_map(params![a, b], |x| Ok((x.get::<_, i64>(0)?, x.get::<_, String>(1)?, x.get::<_, String>(2)?, x.get::<_, String>(3)?, x.get::<_, f64>(4)?, x.get::<_, String>(5)?, x.get::<_, String>(6)?, x.get::<_, String>(7)?, x.get::<_, String>(8)?, x.get::<_, i64>(9)?, x.get::<_, i64>(10)?, x.get::<_, i64>(11)?, x.get::<_, String>(12)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;
                for (id, at, code, name, age, age_type, gender, phone, refd, total, disc, paid, status) in list {
                    let mut names = Vec::new();
                    let mut ts = self.conn.prepare("SELECT snapshot FROM order_tests WHERE order_id=? ORDER BY id")?;
                    for snap in ts.query_map([id], |x| x.get::<_, String>(0))? { let v: serde_json::Value = serde_json::from_str(&snap?)?; names.push(v["name"].as_str().unwrap_or("").to_string()); }
                    r.push(vec![when(&at), format!("B{id:05}"), code, name.split_whitespace().collect::<Vec<_>>().join(" "), format!("{age} {age_type}"), gender, phone, if refd.is_empty() { "Self".into() } else { refd }, names.join("; "), money(total), money(disc), money(paid), money(total - paid), status]);
                }
                r
            }
            "payments" => {
                let mut r = vec![["Date", "Report no", "Patient ID", "Patient", "Method", "Amount", "Reference"].iter().map(|s| s.to_string()).collect::<Vec<_>>()];
                let mut st = self.conn.prepare("SELECT x.created_at,o.id,p.patient_code,p.first_name||' '||p.last_name,x.method,x.amount_paise,x.reference FROM payments x JOIN orders o ON o.id=x.order_id JOIN patients p ON p.id=o.patient_id
                    WHERE o.deleted_at IS NULL AND x.created_at>=?1 AND x.created_at<?2 ORDER BY x.id")?;
                for row in st.query_map(params![a, b], |x| Ok((x.get::<_, String>(0)?, x.get::<_, i64>(1)?, x.get::<_, String>(2)?, x.get::<_, String>(3)?, x.get::<_, String>(4)?, x.get::<_, i64>(5)?, x.get::<_, String>(6)?)))? {
                    let (at, id, code, name, method, amt, reference) = row?;
                    r.push(vec![when(&at), format!("B{id:05}"), code, name, method, money(amt), reference]);
                }
                r
            }
            "patients" => {
                let mut r = vec![["Patient ID", "Name", "Age", "Gender", "Phone", "Email", "Address", "Visits", "Last visit"].iter().map(|s| s.to_string()).collect::<Vec<_>>()];
                let mut st = self.conn.prepare("SELECT p.patient_code,p.designation||' '||p.first_name||' '||p.last_name,p.age,p.age_type,p.gender,p.phone,p.email,p.address,COUNT(o.id),MAX(o.created_at)
                    FROM patients p JOIN orders o ON o.patient_id=p.id AND o.deleted_at IS NULL AND o.created_at>=?1 AND o.created_at<?2 GROUP BY p.id ORDER BY MAX(o.created_at) DESC")?;
                for row in st.query_map(params![a, b], |x| Ok((x.get::<_, String>(0)?, x.get::<_, String>(1)?, x.get::<_, f64>(2)?, x.get::<_, String>(3)?, x.get::<_, String>(4)?, x.get::<_, String>(5)?, x.get::<_, String>(6)?, x.get::<_, String>(7)?, x.get::<_, i64>(8)?, x.get::<_, String>(9)?)))? {
                    let (code, name, age, age_type, gender, phone, email, addr, n, last) = row?;
                    r.push(vec![code, name.split_whitespace().collect::<Vec<_>>().join(" "), format!("{age} {age_type}"), gender, phone, email, addr, n.to_string(), when(&last)]);
                }
                r
            }
            "doctors" => {
                let mut r = vec![["Referred by", "Visits", "Billed", "Received"].iter().map(|s| s.to_string()).collect::<Vec<_>>()];
                for l in self.referral_lines(&a, &b)? { r.push(vec![l.name, l.visits.to_string(), money(l.billed_paise), money(l.received_paise)]); }
                r
            }
            "tests" => {
                let s = self.summary(actor, from, to)?;
                let mut r = vec![vec!["Test".to_string(), "Times done".to_string()]];
                for (n, c) in s.tests { r.push(vec![n, c.to_string()]); }
                r
            }
            _ => return Err(crate::Error::Rule("Choose what to export".into())),
        };
        audit(&self.conn, actor, "export_csv", "export", 0, json!({ "what": what, "from": from, "to": to, "rows": rows.len() - 1 }))?;
        Ok(csv(rows))
    }
}
