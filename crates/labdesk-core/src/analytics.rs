// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Numbers for the Analytics page: how busy the lab was and how much money came in, for a range of Indian days,
//! with the same-length period before it for comparison. Only Admin and Reception can ask (it shows money).
use crate::error::{check, Error, Result};
use crate::model::*;
use crate::store::{allow, Store};
use rusqlite::params;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Totals {
    pub visits: i64,
    pub tests: i64,
    pub billed_paise: i64,
    pub discount_paise: i64,
    pub received_paise: i64,
    /// Billed on these visits minus what was paid on them (so far).
    pub due_paise: i64,
    pub new_patients: i64,
    pub returning_patients: i64,
    /// Average minutes from registration to the doctor's approval, over the visits that are approved.
    pub avg_turnaround_min: Option<i64>,
}
/// One bar of a chart: a day (or a month when the range is long).
#[derive(Debug, Clone, PartialEq)]
pub struct Bucket { pub label: String, pub visits: i64, pub tests: i64, pub billed_paise: i64, pub received_paise: i64 }
#[derive(Debug, Clone, PartialEq)]
pub struct DeptLine { pub name: String, pub tests: i64, pub value_paise: i64 }
#[derive(Debug, Clone, PartialEq)]
pub struct Analytics {
    pub from: String, pub to: String, pub days: i64,
    pub totals: Totals,
    pub previous: Totals,
    /// "2026-10-16 to 2026-10-22": the period the totals are compared with.
    pub previous_label: String,
    pub buckets: Vec<Bucket>,
    /// True when the buckets are months (the range is longer than about three months).
    pub monthly: bool,
    pub by_method: Vec<(String, i64)>,
    pub by_dept: Vec<DeptLine>,
    pub top_tests: Vec<(String, i64)>,
    pub referrals: Vec<crate::summary::ReferralLine>,
    /// Visits registered in each hour of the Indian day, 0..24.
    pub hours: [i64; 24],
    /// Reports not yet approved, whatever the range (right now).
    pub waiting_reports: i64,
}

fn ist() -> chrono::FixedOffset { chrono::FixedOffset::east_opt(5 * 3600 + 1800).expect("valid offset") }
fn at(d: chrono::NaiveDate) -> String {
    d.and_hms_opt(0, 0, 0).expect("midnight").and_local_timezone(ist()).single().expect("fixed offset").with_timezone(&chrono::Utc).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
fn parse_day(t: &str) -> Result<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(t.trim(), "%Y-%m-%d").map_err(|_| Error::Rule("Please give the date as year-month-day, for example 2026-10-25".into()))
}

impl Store {
    fn totals_between(&self, a: &str, b: &str) -> Result<Totals> {
        let one = |sql: &str| -> Result<i64> { Ok(self.conn.query_row(sql, params![a, b], |r| r.get(0))?) };
        let live = "o.deleted_at IS NULL AND o.created_at>=?1 AND o.created_at<?2";
        let visits = one(&format!("SELECT COUNT(*) FROM orders o WHERE {live}"))?;
        let tests = one(&format!("SELECT COUNT(*) FROM order_tests t JOIN orders o ON o.id=t.order_id WHERE {live}"))?;
        let billed = one(&format!("SELECT COALESCE(SUM(o.total_paise),0) FROM orders o WHERE {live}"))?;
        let discount = one(&format!("SELECT COALESCE(SUM(o.discount_paise),0) FROM orders o WHERE {live}"))?;
        let received = one("SELECT COALESCE(SUM(p.amount_paise),0) FROM payments p JOIN orders o ON o.id=p.order_id WHERE o.deleted_at IS NULL AND p.created_at>=?1 AND p.created_at<?2")?;
        let paid_on_these = one(&format!("SELECT COALESCE(SUM(p.amount_paise),0) FROM payments p JOIN orders o ON o.id=p.order_id WHERE {live}"))?;
        let returning = one(&format!("SELECT COALESCE(SUM(CASE WHEN EXISTS(SELECT 1 FROM orders x WHERE x.patient_id=o.patient_id AND x.id<o.id AND x.deleted_at IS NULL) THEN 1 ELSE 0 END),0) FROM orders o WHERE {live}"))?;
        let avg: Option<f64> = self.conn.query_row(&format!("SELECT AVG((julianday(o.approved_at)-julianday(o.created_at))*1440.0) FROM orders o WHERE {live} AND o.approved_at IS NOT NULL"), params![a, b], |r| r.get(0))?;
        Ok(Totals { visits, tests, billed_paise: billed, discount_paise: discount, received_paise: received, due_paise: billed - paid_on_these,
            new_patients: visits - returning, returning_patients: returning, avg_turnaround_min: avg.filter(|v| v.is_finite() && *v >= 0.0).map(|v| v.round() as i64) })
    }

    pub fn analytics(&self, actor: &Actor, from: &str, to: &str) -> Result<Analytics> {
        allow(actor, &[Role::Admin, Role::Reception])?;
        let (d1, d2) = (parse_day(from)?, parse_day(to)?);
        check(d1 <= d2, "The first day is after the last day")?;
        let days = (d2 - d1).num_days() + 1;
        check(days <= 3660, "Please choose at most ten years")?;
        let (a, b) = (at(d1), at(d2 + chrono::Duration::days(1)));
        let (p1, p2) = (d1 - chrono::Duration::days(days), d1 - chrono::Duration::days(1));
        let (pa, pb) = (at(p1), a.clone());
        let totals = self.totals_between(&a, &b)?;
        let previous = self.totals_between(&pa, &pb)?;

        // one bar per day, or per month for a long range; days with nothing still get a bar
        let monthly = days > 92;
        let key = |d: chrono::NaiveDate| if monthly { d.format("%Y-%m").to_string() } else { d.format("%Y-%m-%d").to_string() };
        let mut map: BTreeMap<String, Bucket> = BTreeMap::new();
        let mut d = d1;
        while d <= d2 { let k = key(d); map.entry(k.clone()).or_insert(Bucket { label: k, visits: 0, tests: 0, billed_paise: 0, received_paise: 0 }); d += chrono::Duration::days(1); }
        let cut = if monthly { 7 } else { 10 };
        {
            let mut st = self.conn.prepare("SELECT strftime('%Y-%m-%d', o.created_at, '+330 minutes'), COUNT(*), SUM(o.total_paise), (SELECT COUNT(*) FROM order_tests t WHERE t.order_id=o.id) FROM orders o WHERE o.deleted_at IS NULL AND o.created_at>=?1 AND o.created_at<?2 GROUP BY o.id")?;
            for r in st.query_map(params![a, b], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?, r.get::<_, i64>(3)?)))? {
                let (day, n, billed, tests) = r?;
                if let Some(x) = map.get_mut(&day[..cut]) { x.visits += n; x.billed_paise += billed; x.tests += tests; }
            }
            let mut st = self.conn.prepare("SELECT strftime('%Y-%m-%d', p.created_at, '+330 minutes'), p.amount_paise FROM payments p JOIN orders o ON o.id=p.order_id WHERE o.deleted_at IS NULL AND p.created_at>=?1 AND p.created_at<?2")?;
            for r in st.query_map(params![a, b], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
                let (day, amt) = r?;
                if let Some(x) = map.get_mut(&day[..cut]) { x.received_paise += amt; }
            }
        }
        let mut st = self.conn.prepare("SELECT p.method,SUM(p.amount_paise) FROM payments p JOIN orders o ON o.id=p.order_id WHERE o.deleted_at IS NULL AND p.created_at>=?1 AND p.created_at<?2 GROUP BY p.method ORDER BY 2 DESC")?;
        let by_method = st.query_map(params![a, b], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?.collect::<std::result::Result<Vec<_>, _>>()?;

        let mut depts: BTreeMap<String, (i64, i64)> = BTreeMap::new();
        let mut tests: BTreeMap<String, i64> = BTreeMap::new();
        let mut st = self.conn.prepare("SELECT t.snapshot FROM order_tests t JOIN orders o ON o.id=t.order_id WHERE o.deleted_at IS NULL AND o.created_at>=?1 AND o.created_at<?2")?;
        for snap in st.query_map(params![a, b], |r| r.get::<_, String>(0))? {
            let v: serde_json::Value = serde_json::from_str(&snap?)?;
            let e = depts.entry(v["department"].as_str().filter(|s| !s.is_empty()).unwrap_or("Other").to_string()).or_default();
            e.0 += 1; e.1 += v["price_paise"].as_i64().unwrap_or(0);
            *tests.entry(v["name"].as_str().unwrap_or("?").to_string()).or_default() += 1;
        }
        let mut by_dept: Vec<DeptLine> = depts.into_iter().map(|(name, (tests, value_paise))| DeptLine { name, tests, value_paise }).collect();
        by_dept.sort_by(|x, y| y.tests.cmp(&x.tests).then(x.name.cmp(&y.name)));
        let mut top_tests: Vec<(String, i64)> = tests.into_iter().collect();
        top_tests.sort_by(|x, y| y.1.cmp(&x.1).then(x.0.cmp(&y.0)));
        top_tests.truncate(10);

        let referrals = self.referral_lines(&a, &b)?;
        let mut hours = [0i64; 24];
        let mut st = self.conn.prepare("SELECT CAST(strftime('%H', created_at, '+330 minutes') AS INTEGER), COUNT(*) FROM orders WHERE deleted_at IS NULL AND created_at>=?1 AND created_at<?2 GROUP BY 1")?;
        for r in st.query_map(params![a, b], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))? { let (h, n) = r?; if (0..24).contains(&h) { hours[h as usize] = n; } }
        let waiting_reports: i64 = self.conn.query_row("SELECT COUNT(*) FROM orders WHERE deleted_at IS NULL AND status IN ('Pending','Partial')", [], |r| r.get(0))?;
        Ok(Analytics { from: d1.format("%Y-%m-%d").to_string(), to: d2.format("%Y-%m-%d").to_string(), days, totals, previous,
            previous_label: if days == 1 { p1.format("%Y-%m-%d").to_string() } else { format!("{} to {}", p1.format("%Y-%m-%d"), p2.format("%Y-%m-%d")) },
            buckets: map.into_values().collect(), monthly, by_method, by_dept, top_tests, referrals, hours, waiting_reports })
    }
}
