// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! The lab database and every operation that changes it. One SQLite file, one process,
//! every change in a transaction and recorded in the append-only audit log.
use crate::calc::{calculate_results, flag_for};
use crate::error::{check, Error, Result};
use crate::model::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::path::Path;

const SCHEMA: &str = include_str!("schema.sql");
pub(crate) const SCHEMA_VERSION: i64 = 4;
const METHODS: [&str; 4] = ["Cash", "UPI", "Card", "Bank"];

pub struct Store { pub(crate) conn: Connection, pub(crate) lock_secs: i64 }

pub(crate) fn now() -> String { chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true) }
/// Today's date in India (the lab's day boundary), as YYYY-MM-DD.
fn india_day() -> String {
    let ist = chrono::FixedOffset::east_opt(5 * 3600 + 1800).expect("valid offset");
    chrono::Utc::now().with_timezone(&ist).format("%Y-%m-%d").to_string()
}
/// Title used on reports when none is typed (the web version always used "MR.", even for women).
fn default_designation(gender: &str) -> &'static str { match gender { "Male" => "MR.", "Female" => "MS.", _ => "" } }
pub(crate) fn s(v: &str, max: usize) -> String { v.trim().chars().take(max).collect() }
pub(crate) fn allow(a: &Actor, roles: &[Role]) -> Result<()> {
    if roles.contains(&a.role) { Ok(()) } else { Err(Error::Forbidden("Your account cannot perform this action".into())) }
}
pub(crate) fn audit(c: &Connection, actor: &Actor, action: &str, entity: &str, id: i64, details: Value) -> Result<()> {
    c.execute("INSERT INTO audit(actor,action,entity,entity_id,details,created_at) VALUES(?,?,?,?,?,?)",
        params![format!("{}:{}", actor.role.as_str(), actor.name), action, entity, id.to_string(), details.to_string(), now()])?;
    Ok(())
}

/// Rules for a patient's details, used when registering and when editing.
fn check_patient(p: &NewPatient) -> Result<()> {
    check(!s(&p.first_name, 200).is_empty(), "First name is required")?;
    check(!s(&p.address, 1000).is_empty(), "Patient address is required")?;
    let max_age = match p.age_type.as_str() { "Year" => 150.0, "Month" => 1800.0, "Day" => 55000.0, _ => return Err(Error::Rule("Choose an age type".into())) };
    check(p.age.is_finite() && p.age >= 0.0 && p.age <= max_age, "Age is invalid")?;
    check(matches!(p.gender.as_str(), "Male" | "Female" | "Other"), "Choose a gender")?;
    check_email(&p.email)
}
/// An email is optional; if one is typed it must look like one.
pub fn check_email(e: &str) -> Result<()> {
    let e = e.trim();
    if e.is_empty() { return Ok(()); }
    let ok = e.len() <= 200 && !e.contains(char::is_whitespace) && e.split_once('@').is_some_and(|(a, b)| !a.is_empty() && b.contains('.') && !b.starts_with('.') && !b.ends_with('.') && !b.contains('@'));
    check(ok, "This email does not look right. Use the form name@example.com, or leave it empty.")
}
/// The doctor's name if already known (any capitalisation), otherwise a new entry.
fn find_or_add_referral(c: &Connection, name: &str) -> Result<(i64, String)> {
    let name = s(name, 200);
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let found: Option<(i64, String)> = c.query_row("SELECT id,name FROM referrals WHERE lower(name)=lower(?) ORDER BY id LIMIT 1", [&name], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
    if let Some(f) = found { return Ok(f); }
    c.execute("INSERT INTO referrals(name,phone) VALUES(?,'')", [&name])?;
    Ok((c.last_insert_rowid(), name))
}

/// The rules every test definition must follow, whether imported or edited by hand.
fn validate_test(code: &str, name: &str, dept: &str, price: i64, cfg: &TestConfig) -> Result<()> {
    check(!code.is_empty(), "Please enter a short code for the test, for example CBC")?;
    check(!name.is_empty(), "Please enter the test name")?;
    check(!dept.is_empty(), "Please enter the department, for example Biochemistry")?;
    let mut seen = std::collections::HashSet::new();
    check(cfg.parameters.iter().all(|p| !p.name.trim().is_empty() && seen.insert(p.id.clone())), format!("Every result line of {code} needs a name"))?;
    for p in &cfg.parameters {
        check(matches!(p.field.as_str(), "Numeric" | "Text" | "Dropdown" | "Multiple ranges" | "Title" | "Rich Text"), format!("Unsupported field type in {code}"))?;
        check(p.low.zip(p.high).is_none_or(|(l, h)| l <= h), format!("In {code}, the normal range of \"{}\" is reversed: low is above high, so it is the wrong way round", p.name))?;
    }
    check(price >= 0, "Price cannot be negative")?;
    Ok(())
}

/// Bring any database (new, old, or just restored from a backup) up to the current layout.
pub(crate) fn migrate(conn: &Connection) -> Result<()> {
    let v: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    check(v <= SCHEMA_VERSION, "This database was created by a newer version of the app")?;
    conn.execute_batch(SCHEMA)?; // version 1 base (every statement is IF NOT EXISTS)
    if v < 2 {
        // v2: soft-deleted reports, saved sign-ins, and the sign-in attempt counter.
        conn.execute_batch("BEGIN;
            ALTER TABLE orders ADD COLUMN deleted_at TEXT;
            ALTER TABLE orders ADD COLUMN deleted_by TEXT;
            ALTER TABLE orders ADD COLUMN delete_reason TEXT NOT NULL DEFAULT '';
            CREATE INDEX IF NOT EXISTS orders_deleted ON orders(deleted_at);
            CREATE TABLE IF NOT EXISTS sessions(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
              token_hash TEXT NOT NULL UNIQUE,
              created_at TEXT NOT NULL,
              expires_at TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS login_guard(
              id INTEGER PRIMARY KEY CHECK(id=1),
              fails INTEGER NOT NULL DEFAULT 0,
              locked_until TEXT);
            INSERT OR IGNORE INTO login_guard(id,fails) VALUES(1,0);
            COMMIT;")?;
    }
    if v < 3 {
        // v3: each visit keeps the age it was registered with (a returning patient is older each time).
        conn.execute_batch("BEGIN;
            ALTER TABLE orders ADD COLUMN age REAL;
            ALTER TABLE orders ADD COLUMN age_type TEXT;
            UPDATE orders SET age=(SELECT age FROM patients WHERE id=patient_id), age_type=(SELECT age_type FROM patients WHERE id=patient_id);
            CREATE INDEX IF NOT EXISTS orders_patient ON orders(patient_id);
            COMMIT;")?;
    }
    if v < 4 {
        // v4: results received from a blood cell counter wait here until someone attaches them to a report.
        conn.execute_batch("BEGIN;
            CREATE TABLE IF NOT EXISTS machine_results(
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              received_at TEXT NOT NULL,
              sample_id TEXT NOT NULL,
              patient_name TEXT NOT NULL,
              test_time TEXT NOT NULL,
              status TEXT NOT NULL DEFAULT 'New',
              order_test_id INTEGER REFERENCES order_tests(id),
              payload TEXT NOT NULL,
              raw TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS machine_results_test ON machine_results(order_test_id);
            COMMIT;")?;
    }
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    Ok(())
}

impl Store {
    /// Open (creating if needed) the lab database. WAL + full sync keeps data safe through power cuts.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        Self::init(conn)
    }
    pub fn open_in_memory() -> Result<Self> { Self::init(Connection::open_in_memory()?) }
    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "FULL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        migrate(&conn)?;
        Ok(Self { conn, lock_secs: crate::users::LOCK_SECONDS })
    }
}

impl Store {

    /// Consistent copy of the whole database (safe while the app is running). Use for daily backups.
    pub fn backup_to(&self, dest: impl AsRef<Path>) -> Result<()> {
        let dest = dest.as_ref();
        if dest.exists() { return Err(Error::Rule("Backup file already exists".into())); }
        self.conn.execute("VACUUM INTO ?1", [dest.to_string_lossy().as_ref()])?;
        Ok(())
    }

    // ---------- catalogue ----------
    /// Import test definitions from a catalogue JSON file's contents (`{"tests":[...]}`), in one
    /// transaction. Existing codes are updated; nothing is deleted. Returns how many were imported.
    pub fn import_catalogue(&mut self, actor: &Actor, json_text: &str) -> Result<usize> {
        allow(actor, &[Role::Admin])?;
        let doc: Value = serde_json::from_str(json_text)?;
        let list = doc.get("tests").and_then(Value::as_array).ok_or_else(|| Error::Rule("Catalogue has no \"tests\" list".into()))?;
        let tx = self.conn.transaction()?;
        for t in list {
            let code = s(t["code"].as_str().unwrap_or(""), 200);
            let name = s(t["name"].as_str().unwrap_or(""), 200);
            let dept = s(t["department"].as_str().unwrap_or(""), 80);
            let cfg: TestConfig = serde_json::from_value(t.get("config").cloned().unwrap_or(json!({})))?;
            let price = t["price_paise"].as_i64().unwrap_or(0);
            validate_test(&code, &name, &dept, price, &cfg)?;
            tx.execute("INSERT INTO tests(code,name,department,sample,price_paise,config,disabled) VALUES(?,?,?,?,?,?,?)
                ON CONFLICT(code) DO UPDATE SET name=excluded.name,department=excluded.department,sample=excluded.sample,
                price_paise=excluded.price_paise,config=excluded.config,disabled=excluded.disabled",
                params![code, name, dept, s(t["sample"].as_str().unwrap_or(""), 80), price, serde_json::to_string(&cfg)?, t["disabled"].as_i64().unwrap_or(0)])?;
        }
        audit(&tx, actor, "import_catalogue", "test", 0, json!({ "count": list.len() }))?;
        tx.commit()?;
        Ok(list.len())
    }

    pub fn tests(&self) -> Result<Vec<TestDef>> {
        let mut st = self.conn.prepare("SELECT id,code,name,department,sample,price_paise,disabled,config FROM tests ORDER BY lower(name)")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get::<_, i64>(6)?, r.get::<_, String>(7)?)))?;
        rows.map(|r| { let (id, code, name, department, sample, price_paise, d, cfg) = r?;
            Ok(TestDef { id, code, name, department, sample, price_paise, disabled: d != 0, config: serde_json::from_str(&cfg)? }) }).collect()
    }

    /// Create (`t.id == 0`) or change a test. Past orders keep the version of the test they were
    /// registered with, so editing never alters an existing report. Returns the saved test.
    pub fn save_test(&mut self, actor: &Actor, t: &TestDef) -> Result<TestDef> {
        allow(actor, &[Role::Admin])?;
        let (code, name, dept) = (s(&t.code, 200).to_uppercase(), s(&t.name, 200), s(&t.department, 80));
        let mut cfg = t.config.clone();
        // Give new result lines an id (p1, p2, ...) that is not already used in this test.
        let mut used: std::collections::HashSet<String> = cfg.parameters.iter().map(|p| p.id.clone()).filter(|i| !i.is_empty()).collect();
        let mut n = 0;
        for p in cfg.parameters.iter_mut().filter(|p| p.id.is_empty()) {
            loop { n += 1; let id = format!("p{n}"); if used.insert(id.clone()) { p.id = id; break; } }
            p.name = s(&p.name, 200);
        }
        validate_test(&code, &name, &dept, t.price_paise, &cfg)?;
        for p in cfg.parameters.iter_mut().filter(|p| p.field == "Dropdown") {
            let mut seen = std::collections::HashSet::new();
            p.options = p.options.iter().map(|o| s(o, 200)).filter(|o| !o.is_empty() && seen.insert(o.to_lowercase())).collect();
            check(!p.options.is_empty(), format!("The choice list of \"{}\" is empty. Type the choices separated by commas, for example: Pale yellow, Yellow, Amber", p.name))?;
        }
        let clash: Option<i64> = self.conn.query_row("SELECT id FROM tests WHERE code=? AND id<>?", params![code, t.id], |r| r.get(0)).optional()?;
        check(clash.is_none(), format!("Another test already uses the code {code}. Please choose a different code."))?;
        let cfg_text = serde_json::to_string(&cfg)?;
        let id = if t.id == 0 {
            self.conn.execute("INSERT INTO tests(code,name,department,sample,price_paise,config,disabled) VALUES(?,?,?,?,?,?,?)",
                params![code, name, dept, s(&t.sample, 80), t.price_paise, cfg_text, t.disabled as i64])?;
            self.conn.last_insert_rowid()
        } else {
            let n = self.conn.execute("UPDATE tests SET code=?,name=?,department=?,sample=?,price_paise=?,config=?,disabled=? WHERE id=?",
                params![code, name, dept, s(&t.sample, 80), t.price_paise, cfg_text, t.disabled as i64, t.id])?;
            check(n == 1, "That test no longer exists")?;
            t.id
        };
        audit(&self.conn, actor, if t.id == 0 { "create" } else { "edit" }, "test", id, json!({ "code": code }))?;
        self.tests()?.into_iter().find(|x| x.id == id).ok_or_else(|| Error::NotFound("test".into()))
    }

    pub fn add_referral(&mut self, actor: &Actor, name: &str, phone: &str) -> Result<i64> {
        allow(actor, &[Role::Admin, Role::Reception])?;
        check(!s(name, 200).is_empty(), "Referral name is required")?;
        self.conn.execute("INSERT INTO referrals(name,phone) VALUES(?,?)", params![s(name, 200), s(phone, 30)])?;
        let id = self.conn.last_insert_rowid();
        audit(&self.conn, actor, "create", "referral", id, json!({}))?;
        Ok(id)
    }

    // ---------- registration & billing ----------
    pub fn register(&mut self, actor: &Actor, n: NewOrder) -> Result<Order> {
        allow(actor, &[Role::Admin, Role::Reception])?;
        let p = &n.patient;
        check_patient(p)?;
        let mut ids = Vec::new();
        for it in &n.items { if !ids.contains(&it.test_id) { ids.push(it.test_id) } }
        check(!ids.is_empty() && ids.len() <= 100, "Select between 1 and 100 tests")?;
        check(n.discount_paise >= 0, "Invalid discount")?;

        let tx = self.conn.transaction()?;
        let mut lines = Vec::new(); // (test_id, snapshot_json, price, line_discount)
        for id in &ids {
            let item = n.items.iter().find(|i| i.test_id == *id).expect("id came from items");
            let row: Option<(String, i64)> = tx.query_row("SELECT config,price_paise FROM tests WHERE id=? AND disabled=0", [id], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
            let (_, list_price) = row.ok_or_else(|| Error::Rule("One of the selected tests is unavailable".into()))?;
            let price = item.price_paise.unwrap_or(list_price);
            check(price >= 0 && item.discount_paise >= 0 && item.discount_paise <= price, "Invalid price or discount")?;
            let t = load_test(&tx, *id)?;
            let snapshot = json!({ "id": t.id, "code": t.code, "name": t.name, "department": t.department, "sample": t.sample,
                "list_price_paise": list_price, "price_paise": price, "discount_paise": item.discount_paise, "config": t.config });
            lines.push((*id, snapshot.to_string(), price, item.discount_paise));
        }
        let subtotal: i64 = lines.iter().map(|l| l.2).sum();
        let line_discount: i64 = lines.iter().map(|l| l.3).sum();
        check(n.discount_paise <= subtotal - line_discount, "Invalid discount")?;
        let discount = line_discount + n.discount_paise;
        let total = subtotal - discount;
        check(n.payments.len() <= 4 && n.payments.iter().all(|p| p.0 >= 0 && METHODS.contains(&p.1.as_str())), "Choose valid payment modes")?;
        check(n.payments.iter().map(|p| p.0).sum::<i64>() <= total, "Payment exceeds the bill total")?;
        let (referral_id, referral_name): (Option<i64>, String) = if !s(&n.referral_doctor, 200).is_empty() {
            let (id, name) = find_or_add_referral(&tx, &n.referral_doctor)?;
            (Some(id), name)
        } else {
            match n.referral_id {
                Some(id) => (Some(id), tx.query_row("SELECT name FROM referrals WHERE id=?", [id], |r| r.get(0)).optional()?.ok_or_else(|| Error::Rule("Referral not found".into()))?),
                None => (None, String::new()),
            }
        };

        // A returning patient keeps their ID; a new one gets yymmdd + 3-digit day sequence, as in the web version.
        let pid = match n.existing_patient_id {
            Some(pid) => {
                let found: Option<(String, String)> = tx.query_row("SELECT first_name,gender FROM patients WHERE id=?", [pid], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
                check(found.is_some(), "That patient was not found. Please search again.")?;
                tx.execute("UPDATE patients SET age=?,age_type=?,phone=?,email=?,address=? WHERE id=?",
                    params![p.age, p.age_type, s(&p.phone, 30), s(&p.email, 200), s(&p.address, 1000), pid])?;
                pid
            }
            None => {
                let day = india_day();
                tx.execute("INSERT INTO sequences(day,value) VALUES(?,1) ON CONFLICT(day) DO UPDATE SET value=value+1", [&day])?;
                let seq: i64 = tx.query_row("SELECT value FROM sequences WHERE day=?", [&day], |r| r.get(0))?;
                let code = format!("{}{:03}", &day.replace('-', "")[2..], seq);
                tx.execute("INSERT INTO patients(patient_code,designation,first_name,last_name,age,age_type,gender,phone,email,address) VALUES(?,?,?,?,?,?,?,?,?,?)",
                    params![code, s(if p.designation.trim().is_empty() { default_designation(&p.gender) } else { &p.designation }, 20), s(&p.first_name, 200), s(&p.last_name, 200), p.age, p.age_type, p.gender, s(&p.phone, 30), s(&p.email, 200), s(&p.address, 1000)])?;
                tx.last_insert_rowid()
            }
        };
        tx.execute("INSERT INTO orders(patient_id,referral_id,referral_name,created_at,subtotal_paise,discount_paise,total_paise,dispatch,bill_note,discount_reason,age,age_type) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)",
            params![pid, referral_id, referral_name, now(), subtotal, discount, total, "[]", s(&n.bill_note, 4000), s(&n.discount_reason, 1000), p.age, p.age_type])?;
        let oid = tx.last_insert_rowid();
        for (tid, snap, _, _) in &lines {
            tx.execute("INSERT INTO order_tests(order_id,test_id,snapshot) VALUES(?,?,?)", params![oid, tid, snap])?;
        }
        for (amt, method, reference) in &n.payments {
            if *amt > 0 { tx.execute("INSERT INTO payments(order_id,amount_paise,method,created_at,reference) VALUES(?,?,?,?,?)", params![oid, amt, method, now(), s(reference, 200)])?; }
        }
        audit(&tx, actor, "register", "order", oid, json!({ "test_count": ids.len() }))?;
        let order = load_order(&tx, oid)?;
        tx.commit()?;
        Ok(order)
    }

    pub fn add_payment(&mut self, actor: &Actor, order_id: i64, amount_paise: i64, method: &str, reference: &str) -> Result<Order> {
        allow(actor, &[Role::Admin, Role::Reception])?;
        let tx = self.conn.transaction()?;
        let o = load_order(&tx, order_id)?;
        check(o.deleted_at.is_none(), "This report was deleted. Restore it first.")?;
        check(amount_paise >= 1 && amount_paise <= o.due_paise(), "Invalid payment amount")?;
        check(METHODS.contains(&method), "Choose a payment method")?;
        tx.execute("INSERT INTO payments(order_id,amount_paise,method,created_at,reference) VALUES(?,?,?,?,?)", params![order_id, amount_paise, method, now(), s(reference, 200)])?;
        audit(&tx, actor, "payment", "order", order_id, json!({ "amount_paise": amount_paise }))?;
        let o = load_order(&tx, order_id)?;
        tx.commit()?;
        Ok(o)
    }

    /// Doctors seen before whose name contains `query` (all of them if empty), for the suggestions list.
    pub fn referral_names(&self, query: &str, limit: usize) -> Result<Vec<String>> {
        let mut st = self.conn.prepare("SELECT name FROM referrals WHERE name LIKE ?1 ORDER BY name COLLATE NOCASE LIMIT ?2")?;
        let rows = st.query_map(params![format!("%{}%", s(query, 100)), limit as i64], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    // ---------- editing a patient ----------
    /// Correct a patient's details and who referred them. Front desk and Admin only.
    /// - Once any test is approved, age and gender can no longer change (they decide the normal ranges).
    /// - After the report is Delivered only phone, email and address can change.
    /// The person's name on an approved report changes the report's Ref code, so a reprint is visibly new.
    pub fn update_patient(&mut self, actor: &Actor, order_id: i64, new: &NewPatient, referral: &Referral) -> Result<Order> {
        allow(actor, &[Role::Admin, Role::Reception])?;
        check_patient(new)?;
        let tx = self.conn.transaction()?;
        let o = load_order(&tx, order_id)?;
        check(o.deleted_at.is_none(), "This report was deleted. Restore it first.")?;
        let p = &o.patient;
        let designation = if new.designation.trim().is_empty() { default_designation(&new.gender).to_string() } else { s(&new.designation, 20) };
        let identity_same = s(&new.first_name, 200) == p.first_name && s(&new.last_name, 200) == p.last_name && designation == p.designation;
        let body_same = new.age == p.age && new.age_type == p.age_type && new.gender == p.gender;
        let ref_same = match referral { Referral::SelfReferred => o.referral_name.is_empty(), Referral::Doctor(d) => s(d, 200).eq_ignore_ascii_case(&o.referral_name) && !o.referral_name.is_empty() };
        if o.status == "Delivered" {
            check(identity_same && body_same && ref_same, "This report was already handed over, so it is locked. Only the phone number, email and address can still be changed.")?;
        }
        if o.tests.iter().any(|t| t.is_approved()) {
            check(body_same, "Age and gender cannot be changed after a test is approved, because they decide the normal ranges. Ask the doctor to open the report for correction first.")?;
        }
        let name_or_sex_changes = !identity_same || new.gender != p.gender;
        if name_or_sex_changes {
            let finished_elsewhere: i64 = tx.query_row("SELECT COUNT(*) FROM orders o WHERE o.patient_id=? AND o.id<>? AND o.deleted_at IS NULL AND (o.status IN ('Completed','Delivered') OR EXISTS(SELECT 1 FROM order_tests t WHERE t.order_id=o.id AND t.approved_at IS NOT NULL))", params![p.id, order_id], |r| r.get(0))?;
            check(finished_elsewhere == 0, "This patient has other reports that are already approved, so the name and gender cannot be changed here. Phone, email, address and age can still be changed.")?;
        }
        tx.execute("UPDATE orders SET age=?,age_type=? WHERE id=?", params![new.age, new.age_type, order_id])?;
        tx.execute("UPDATE patients SET designation=?,first_name=?,last_name=?,age=?,age_type=?,gender=?,phone=?,email=?,address=? WHERE id=?",
            params![designation, s(&new.first_name, 200), s(&new.last_name, 200), new.age, new.age_type, new.gender, s(&new.phone, 30), s(&new.email, 200), s(&new.address, 1000), p.id])?;
        let (rid, rname): (Option<i64>, String) = match referral {
            Referral::SelfReferred => (None, String::new()),
            Referral::Doctor(d) if s(d, 200).is_empty() => (None, String::new()),
            Referral::Doctor(d) => { let (i, n) = find_or_add_referral(&tx, d)?; (Some(i), n) }
        };
        tx.execute("UPDATE orders SET referral_id=?,referral_name=? WHERE id=?", params![rid, rname, order_id])?;
        let after = load_order(&tx, order_id)?;
        audit(&tx, actor, "edit_patient", "order", order_id, json!({
            "before": { "name": format!("{} {} {}", p.designation, p.first_name, p.last_name).trim(), "age": p.age, "age_type": p.age_type, "gender": p.gender, "phone": p.phone, "email": p.email, "referred_by": o.referral_name },
            "after": { "name": format!("{} {} {}", after.patient.designation, after.patient.first_name, after.patient.last_name).trim(), "age": after.patient.age, "age_type": after.patient.age_type, "gender": after.patient.gender, "phone": after.patient.phone, "email": after.patient.email, "referred_by": after.referral_name } }))?;
        tx.commit()?;
        Ok(after)
    }

    // ---------- deleting a report ----------
    /// Remove a report from every list. Nothing is destroyed: an Admin can bring it back from "Deleted reports".
    /// A report that was handed to the patient (Delivered) can never be deleted.
    pub fn delete_order(&mut self, actor: &Actor, order_id: i64, reason: &str) -> Result<()> {
        allow(actor, &[Role::Admin])?;
        check(s(reason, 500).chars().count() >= 3, "Please write a short reason for deleting this report")?;
        let tx = self.conn.transaction()?;
        let o = load_order(&tx, order_id)?;
        check(o.deleted_at.is_none(), "This report is already deleted")?;
        check(o.status != "Delivered", "This report was already handed to the patient, so it cannot be deleted.")?;
        tx.execute("UPDATE orders SET deleted_at=?,deleted_by=?,delete_reason=? WHERE id=?", params![now(), actor.name, s(reason, 500), order_id])?;
        audit(&tx, actor, "delete", "order", order_id, json!({ "reason": s(reason, 500), "patient_code": o.patient.patient_code, "total_paise": o.total_paise, "paid_paise": o.paid_paise() }))?;
        tx.commit()?;
        Ok(())
    }
    pub fn restore_order(&mut self, actor: &Actor, order_id: i64) -> Result<()> {
        allow(actor, &[Role::Admin])?;
        let tx = self.conn.transaction()?;
        let o = load_order(&tx, order_id)?;
        check(o.deleted_at.is_some(), "This report is not deleted")?;
        tx.execute("UPDATE orders SET deleted_at=NULL,deleted_by=NULL,delete_reason='' WHERE id=?", [order_id])?;
        audit(&tx, actor, "restore", "order", order_id, json!({}))?;
        tx.commit()?;
        Ok(())
    }
    /// Deleted reports with who deleted them and why, newest first.
    pub fn deleted_orders(&self) -> Result<Vec<DeletedOrder>> {
        let mut st = self.conn.prepare("SELECT o.id,o.deleted_at,COALESCE(o.deleted_by,''),o.delete_reason,p.patient_code,p.designation,p.first_name,p.last_name,o.total_paise
            FROM orders o JOIN patients p ON p.id=o.patient_id WHERE o.deleted_at IS NOT NULL ORDER BY o.deleted_at DESC")?;
        let rows = st.query_map([], |r| Ok(DeletedOrder { id: r.get(0)?, deleted_at: r.get(1)?, deleted_by: r.get(2)?, reason: r.get(3)?, patient_code: r.get(4)?,
            patient_name: format!("{} {} {}", r.get::<_, String>(5)?, r.get::<_, String>(6)?, r.get::<_, String>(7)?).split_whitespace().collect::<Vec<_>>().join(" "), total_paise: r.get(8)? }))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    // ---------- result entry ----------
    /// Save entered values as a draft. Flags and formula results are computed here, on the
    /// authoritative side. `version` must match the order's current version (stale-save protection).
    pub fn save_results(&mut self, actor: &Actor, order_id: i64, version: i64, inputs: &[ResultInput], notes: Option<&str>) -> Result<Order> {
        allow(actor, &[Role::Admin, Role::Technician, Role::Pathologist])?;
        check(inputs.len() <= 5000, "Invalid results")?;
        let tx = self.conn.transaction()?;
        let order = load_order(&tx, order_id)?;
        check(order.deleted_at.is_none(), "This report was deleted. Restore it first.")?;
        check(order.status != "Delivered", "Delivered reports are permanently locked. Corrections are not allowed.")?;
        check(!order_fully_approved(&tx, order_id)?, "Approved reports are locked. Start a correction first.")?;
        check(order.version == version, "This report changed. Reload before saving.")?;
        for r in inputs {
            let t = order.tests.iter().find(|t| t.id == r.order_test_id).ok_or_else(|| Error::Rule("Unknown test".into()))?;
            let p = t.config.parameters.iter().find(|p| p.id == r.parameter_id).ok_or_else(|| Error::Rule("Unknown test parameter".into()))?;
            check(!t.is_approved(), "Approved test is locked. Un-approve it first.")?;
            check(p.is_result_field(), "This field is not a result")?;
            let value = s(&r.value, 8000);
            if !value.is_empty() && p.is_numeric() { check(value.parse::<f64>().is_ok_and(f64::is_finite), format!("Numeric parameter requires a number: {}", p.name))?; }
            if !value.is_empty() && p.field == "Dropdown" { check(p.options.is_empty() || p.options.contains(&value), format!("Select a configured value: {}", p.name))?; }
            let prev = t.results.iter().find(|x| x.parameter_id == p.id);
            let comment = r.comment.as_deref().map(|c| s(c, 4000)).unwrap_or_else(|| prev.map(|x| x.comment.clone()).unwrap_or_default());
            let manual = !p.formula.is_empty() && r.manual;
            let flag = match prev.and_then(|x| x.range_text.as_deref()).filter(|t| !t.trim().is_empty()) { Some(t) => crate::machine::flag_from_range(&value, t).to_string(), None => flag_for(p, &value, &order.patient).to_string() };
            tx.execute("INSERT INTO results(order_test_id,parameter_id,value,flag,range_text,comment,manual) VALUES(?,?,?,?,?,?,?)
                ON CONFLICT(order_test_id,parameter_id) DO UPDATE SET value=excluded.value,flag=excluded.flag,comment=excluded.comment,manual=excluded.manual",
                params![t.id, p.id, value, flag, Option::<String>::None, comment, manual])?;
        }
        // Recompute formulas and per-test status for every test that is still open.
        let fresh = load_order(&tx, order_id)?;
        for t in fresh.tests.iter().filter(|t| !t.is_approved()) {
            let computed = calculate_results(&t.config.parameters, &t.results);
            for p in t.config.parameters.iter().filter(|p| !p.formula.is_empty() && !t.results.iter().any(|r| r.parameter_id == p.id && r.manual)) {
                let value = computed.iter().find(|r| r.parameter_id == p.id).map(|r| r.value.clone()).unwrap_or_default();
                tx.execute("INSERT INTO results(order_test_id,parameter_id,value,flag) VALUES(?,?,?,?)
                    ON CONFLICT(order_test_id,parameter_id) DO UPDATE SET value=excluded.value,flag=excluded.flag",
                    params![t.id, p.id, value, flag_for(p, &value, &order.patient)])?;
            }
            let filled: i64 = tx.query_row("SELECT COUNT(*) FROM results WHERE order_test_id=? AND value<>''", [t.id], |r| r.get(0))?;
            tx.execute("UPDATE order_tests SET status=? WHERE id=?", params![if filled > 0 { "Partial" } else { "Pending" }, t.id])?;
        }
        let started: i64 = tx.query_row("SELECT COUNT(*) FROM order_tests WHERE order_id=? AND status<>'Pending'", [order_id], |r| r.get(0))?;
        tx.execute("UPDATE orders SET status=?,version=version+1,notes=? WHERE id=?",
            params![if started > 0 { "Partial" } else { "Pending" }, s(notes.unwrap_or(&order.notes), 8000), order_id])?;
        audit(&tx, actor, "save_draft", "order", order_id, json!({ "previous_version": order.version, "changed_values": inputs.len() }))?;
        let o = load_order(&tx, order_id)?;
        tx.commit()?;
        Ok(o)
    }

    /// Pathologist sign-off on all open tests, or just `test_ids`.
    pub fn approve(&mut self, actor: &Actor, order_id: i64, version: i64, test_ids: Option<&[i64]>) -> Result<Order> {
        allow(actor, &[Role::Admin, Role::Pathologist])?;
        let tx = self.conn.transaction()?;
        let o = load_order(&tx, order_id)?;
        check(o.version == version, "Report changed. Reload before approval.")?;
        check(!order_fully_approved(&tx, order_id)?, "Report already approved")?;
        let ids: Vec<i64> = match test_ids { Some(x) => x.to_vec(), None => o.tests.iter().filter(|t| !t.is_approved()).map(|t| t.id).collect() };
        check(!ids.is_empty() && ids.iter().all(|i| o.tests.iter().any(|t| t.id == *i)), "Choose valid tests to approve")?;
        for t in o.tests.iter().filter(|t| ids.contains(&t.id)) {
            check(!t.is_approved(), "Test already approved")?;
            check(t.config.only_billing || !t.config.parameters.is_empty(), format!("Configure parameters before approving {}", t.name))?;
            for p in &t.config.parameters {
                if p.required && p.is_result_field() {
                    check(t.results.iter().any(|r| r.parameter_id == p.id && !r.value.is_empty()), format!("Missing required result: {}", p.name))?;
                }
            }
        }
        let at = now();
        for t in o.tests.iter().filter(|t| ids.contains(&t.id)) {
            let mut snap = serde_json::to_value(t)?;
            snap["status"] = json!("Completed"); snap["approved_at"] = json!(at); snap["approved_by_name"] = json!(actor.name);
            tx.execute("UPDATE order_tests SET status='Completed',approved_snapshot=?,approved_at=?,approved_by_name=? WHERE id=?", params![snap.to_string(), at, actor.name, t.id])?;
        }
        let after = load_order(&tx, order_id)?;
        let complete = after.tests.iter().all(|t| t.is_approved());
        let snapshot = if complete { let mut v = serde_json::to_value(&after)?; v["status"] = json!("Completed"); v["approved_at"] = json!(at); v["approved_by_name"] = json!(actor.name); Some(v.to_string()) } else { None };
        tx.execute("UPDATE orders SET status=?,approved_snapshot=?,approved_at=?,version=version+1 WHERE id=?",
            params![if complete { "Completed" } else { "Partial" }, snapshot, if complete { Some(at.clone()) } else { None }, order_id])?;
        audit(&tx, actor, "approve", "order", order_id, json!({ "version": o.version, "test_ids": ids }))?;
        let o = load_order(&tx, order_id)?;
        tx.commit()?;
        Ok(o)
    }

    /// Re-open approved tests for correction. The previous approved version is kept, with the reason.
    pub fn correct(&mut self, actor: &Actor, order_id: i64, version: i64, reason: &str, test_ids: Option<&[i64]>) -> Result<Order> {
        allow(actor, &[Role::Admin, Role::Pathologist])?;
        check(s(reason, 1000).chars().count() >= 5, "Give a correction reason")?;
        let tx = self.conn.transaction()?;
        let o = load_order(&tx, order_id)?;
        check(o.deleted_at.is_none(), "This report was deleted. Restore it first.")?;
        check(o.status != "Delivered", "Delivered reports are permanently locked. Corrections are not allowed.")?;
        let ids: Vec<i64> = match test_ids { Some(x) => x.to_vec(), None => o.tests.iter().filter(|t| t.is_approved()).map(|t| t.id).collect() };
        check(!ids.is_empty() && ids.iter().all(|i| o.tests.iter().any(|t| t.id == *i && t.is_approved())), "Choose approved tests to correct")?;
        check(o.version == version, "Report changed")?;
        tx.execute("INSERT INTO report_versions(order_id,version,snapshot,reason,created_at) VALUES(?,?,?,?,?)", params![order_id, o.version, serde_json::to_string(&o)?, s(reason, 1000), now()])?;
        for id in &ids { tx.execute("UPDATE order_tests SET approved_snapshot=NULL,approved_at=NULL,approved_by_name=NULL,status='Partial' WHERE id=?", [id])?; }
        tx.execute("UPDATE orders SET approved_snapshot=NULL,approved_by=NULL,approved_at=NULL,status='Partial',version=version+1 WHERE id=?", [order_id])?;
        audit(&tx, actor, "correction", "order", order_id, json!({ "reason": s(reason, 1000), "test_ids": ids }))?;
        let o = load_order(&tx, order_id)?;
        tx.commit()?;
        Ok(o)
    }

    /// Record that the report was handed over. After this the report is permanently locked.
    pub fn mark_delivered(&mut self, actor: &Actor, order_id: i64, version: i64) -> Result<Order> {
        allow(actor, &[Role::Admin, Role::Reception])?;
        let tx = self.conn.transaction()?;
        let o = load_order(&tx, order_id)?;
        check(o.deleted_at.is_none(), "This report was deleted. Restore it first.")?;
        check(order_fully_approved(&tx, order_id)?, "Approve the report first")?;
        check(o.status != "Delivered", "Report has already been delivered.")?;
        check(o.version == version, "Report changed. Reload before marking delivered.")?;
        tx.execute("UPDATE orders SET status='Delivered',version=version+1 WHERE id=?", [order_id])?;
        audit(&tx, actor, "mark_delivered", "order", order_id, json!({}))?;
        let o = load_order(&tx, order_id)?;
        tx.commit()?;
        Ok(o)
    }


    // ---------- lab profile & images ----------
    pub fn profile(&self) -> Result<Profile> {
        let row: Option<String> = self.conn.query_row("SELECT value FROM settings WHERE key='profile'", [], |r| r.get(0)).optional()?;
        Ok(match row { Some(v) => serde_json::from_str(&v)?, None => Profile::default() })
    }
    pub fn save_profile(&mut self, actor: &Actor, p: &Profile) -> Result<()> {
        allow(actor, &[Role::Admin])?;
        check(!p.name.trim().is_empty(), "Enter the lab name")?;
        for (label, v) in [("Top", p.top_mm), ("Bottom", p.bottom_mm), ("Left", p.left_mm), ("Right", p.right_mm)] {
            check(v.is_finite() && (0.0..=120.0).contains(&v), format!("{label} margin must be between 0 and 120 mm"))?;
        }
        let clean = Profile { name: s(&p.name, 200), address: s(&p.address, 500), phone: s(&p.phone, 80), email: s(&p.email, 120),
            signatory: s(&p.signatory, 120), qualification: s(&p.qualification, 200), ending_line: s(&p.ending_line, 80), header_design: p.header_design.min(7), text_size: p.text_size.clamp(70, 120), ..p.clone() };
        self.conn.execute("INSERT INTO settings(key,value) VALUES('profile',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [serde_json::to_string(&clean)?])?;
        audit(&self.conn, actor, "save", "profile", 0, json!({}))?;
        Ok(())
    }
    /// kind: "letterhead" or "signature". PNG or JPEG only.
    pub fn set_asset(&mut self, actor: &Actor, kind: &str, bytes: Vec<u8>) -> Result<()> {
        allow(actor, &[Role::Admin])?;
        check(matches!(kind, "letterhead" | "signature"), "Unknown image type")?;
        check(bytes.len() <= 12 * 1024 * 1024, "That image is larger than 12 MB. Please use a smaller file.")?;
        let mime = match image::guess_format(&bytes) {
            Ok(image::ImageFormat::Png) => "image/png",
            Ok(image::ImageFormat::Jpeg) => "image/jpeg",
            _ => return Err(Error::Rule("Please choose a PNG or JPG image.".into())),
        };
        image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format().ok()
            .and_then(|r| r.into_dimensions().ok()).ok_or_else(|| Error::Rule("That image file could not be read.".into()))?;
        self.conn.execute("INSERT INTO assets(kind,mime,data,updated_at) VALUES(?,?,?,?) ON CONFLICT(kind) DO UPDATE SET mime=excluded.mime,data=excluded.data,updated_at=excluded.updated_at",
            params![kind, mime, bytes, now()])?;
        audit(&self.conn, actor, "upload", "image", 0, json!({ "kind": kind }))?;
        Ok(())
    }
    pub fn clear_asset(&mut self, actor: &Actor, kind: &str) -> Result<()> {
        allow(actor, &[Role::Admin])?;
        self.conn.execute("DELETE FROM assets WHERE kind=?", [kind])?;
        audit(&self.conn, actor, "remove", "image", 0, json!({ "kind": kind }))?;
        Ok(())
    }
    pub fn asset(&self, kind: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.conn.query_row("SELECT data FROM assets WHERE kind=?", [kind], |r| r.get(0)).optional()?)
    }

    // ---------- home screen numbers ----------
    pub fn stats(&self) -> Result<Stats> {
        let ist = chrono::FixedOffset::east_opt(5 * 3600 + 1800).expect("valid offset");
        let midnight = chrono::Utc::now().with_timezone(&ist).date_naive().and_hms_opt(0, 0, 0).expect("midnight");
        let start = midnight.and_local_timezone(ist).single().expect("fixed offset").with_timezone(&chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let one = |sql: &str, p: &[&dyn rusqlite::ToSql]| -> Result<i64> { Ok(self.conn.query_row(sql, p, |r| r.get(0))?) };
        Ok(Stats {
            patients_today: one("SELECT COUNT(*) FROM orders WHERE created_at>=? AND deleted_at IS NULL", &[&start])?,
            reports_pending: one("SELECT COUNT(*) FROM orders WHERE deleted_at IS NULL AND status IN ('Pending','Partial')", &[])?,
            ready_to_deliver: one("SELECT COUNT(*) FROM orders WHERE deleted_at IS NULL AND status='Completed'", &[])?,
            due_paise: one("SELECT COALESCE(SUM(o.total_paise),0)-COALESCE((SELECT SUM(pa.amount_paise) FROM payments pa JOIN orders x ON x.id=pa.order_id WHERE x.deleted_at IS NULL),0) FROM orders o WHERE o.deleted_at IS NULL", &[])?,
            collected_today_paise: one("SELECT COALESCE(SUM(pa.amount_paise),0) FROM payments pa JOIN orders x ON x.id=pa.order_id WHERE x.deleted_at IS NULL AND pa.created_at>=?", &[&start])?,
        })
    }

    // ---------- report ----------
    /// Build the PDF for an order. Only approved tests are printed. `letterhead`: draw the lab's
    /// uploaded letterhead (or a simple text header if none was uploaded); false leaves the top
    /// and bottom margins blank for pre-printed paper.
    pub fn report_pdf(&mut self, actor: &Actor, order_id: i64, letterhead: bool) -> Result<crate::report::Rendered> {
        let order = self.order(order_id)?;
        check(order.deleted_at.is_none(), "This report was deleted. Restore it first.")?;
        check(order.tests.iter().any(|t| t.is_approved()), "Approve at least one test before printing a report")?;
        let profile = self.profile()?;
        let (lh, sig) = (self.asset("letterhead")?, self.asset("signature")?);
        let out = crate::report::render(&crate::report::ReportInput { order: &order, profile: &profile, letterhead: lh.as_deref(), signature: sig.as_deref(), use_letterhead: letterhead })?;
        audit(&self.conn, actor, "report_pdf", "order", order_id, json!({ "pages": out.pages }))?;
        Ok(out)
    }
    /// The bill for an order (works before any result exists). Front desk and Admin.
    pub fn bill_pdf(&mut self, actor: &Actor, order_id: i64, letterhead: bool) -> Result<crate::report::Rendered> {
        self.bill_pdf_on(actor, order_id, letterhead, None)
    }
    /// The same bill on a roll printer when `roll_mm` is `Some(58.0)` or `Some(80.0)`; `None` is A4.
    pub fn bill_pdf_on(&mut self, actor: &Actor, order_id: i64, letterhead: bool, roll_mm: Option<f32>) -> Result<crate::report::Rendered> {
        allow(actor, &[Role::Admin, Role::Reception])?;
        let order = self.order(order_id)?;
        check(order.deleted_at.is_none(), "This report was deleted. Restore it first.")?;
        let profile = self.profile()?;
        let lh = self.asset("letterhead")?;
        let input = crate::report::ReportInput { order: &order, profile: &profile, letterhead: lh.as_deref(), signature: None, use_letterhead: letterhead };
        let out = match roll_mm { Some(mm) => crate::report::render_bill_thermal(&input, mm)?, None => crate::report::render_bill(&input)? };
        audit(&self.conn, actor, "bill_pdf", "order", order_id, json!({ "pages": out.pages }))?;
        Ok(out)
    }
    /// Has this computer's user accepted the current Terms of Use and Privacy text (`TERMS_VERSION`)?
    pub fn terms_accepted(&self) -> Result<bool> {
        let v: Option<String> = self.conn.query_row("SELECT value FROM settings WHERE key='terms_accepted'", [], |r| r.get(0)).optional()?;
        Ok(v.and_then(|v| serde_json::from_str::<serde_json::Value>(&v).ok()).and_then(|j| j["version"].as_u64()).is_some_and(|n| n >= TERMS_VERSION as u64))
    }
    /// Remember that the Terms were accepted, with the version and the time. Needs no sign-in: it happens before the first account exists.
    pub fn accept_terms(&mut self) -> Result<()> {
        let v = json!({ "version": TERMS_VERSION, "at": chrono::Utc::now().to_rfc3339() }).to_string();
        self.conn.execute("INSERT INTO settings(key,value) VALUES('terms_accepted',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [v])?;
        Ok(())
    }
    /// How the lab likes its bills: header on or off (default OFF) and the paper: "a4", "roll80" or "roll58".
    pub fn bill_prefs(&self) -> Result<BillPrefs> {
        let v: Option<String> = self.conn.query_row("SELECT value FROM settings WHERE key='bill_prefs'", [], |r| r.get(0)).optional()?;
        Ok(match v { Some(v) => serde_json::from_str(&v).unwrap_or_default(), None => BillPrefs::default() })
    }
    pub fn set_bill_prefs(&mut self, actor: &Actor, prefs: &BillPrefs) -> Result<()> {
        allow(actor, &[Role::Admin, Role::Reception])?;
        let clean = BillPrefs { header: prefs.header, paper: match prefs.paper.as_str() { "roll80" | "roll58" => prefs.paper.clone(), _ => "a4".into() } };
        self.conn.execute("INSERT INTO settings(key,value) VALUES('bill_prefs',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [serde_json::to_string(&clean)?])?;
        Ok(())
    }
    /// ONE combined bill for every visit (all tests, all payments, total due) of a patient. Front desk and Admin.
    pub fn patient_bill_pdf(&mut self, actor: &Actor, patient_id: i64, letterhead: bool) -> Result<crate::report::Rendered> {
        allow(actor, &[Role::Admin, Role::Reception])?;
        let ids: Vec<i64> = {
            let mut st = self.conn.prepare("SELECT id FROM orders WHERE patient_id=? AND deleted_at IS NULL ORDER BY created_at,id")?;
            let v = st.query_map([patient_id], |r| r.get(0))?.collect::<std::result::Result<Vec<i64>, _>>()?;
            v
        };
        check(!ids.is_empty(), "This patient has no bills yet")?;
        let orders = ids.into_iter().map(|id| self.order(id)).collect::<Result<Vec<_>>>()?;
        let profile = self.profile()?;
        let lh = self.asset("letterhead")?;
        let out = crate::report::render_statement(&crate::report::ReportInput { order: &orders[orders.len() - 1], profile: &profile, letterhead: lh.as_deref(), signature: None, use_letterhead: letterhead }, &orders)?;
        audit(&self.conn, actor, "patient_bill_pdf", "patient", patient_id, json!({ "visits": orders.len(), "pages": out.pages }))?;
        Ok(out)
    }
    /// A made-up report so the lab can check how its letterhead lines up before printing real ones.
    pub fn test_page_pdf(&self, letterhead: bool) -> Result<crate::report::Rendered> {
        let profile = self.profile()?;
        let (lh, sig) = (self.asset("letterhead")?, self.asset("signature")?);
        crate::report::render(&crate::report::ReportInput { order: &crate::report::sample_order(), profile: &profile, letterhead: lh.as_deref(), signature: sig.as_deref(), use_letterhead: letterhead })
    }

    /// How one test will print, with made-up values (the Tests editor's Preview Report).
    pub fn test_preview_pdf(&self, def: &TestDef, letterhead: bool) -> Result<crate::report::Rendered> {
        let profile = self.profile()?;
        let (lh, sig) = (self.asset("letterhead")?, self.asset("signature")?);
        crate::report::render(&crate::report::ReportInput { order: &crate::report::preview_order(def), profile: &profile, letterhead: lh.as_deref(), signature: sig.as_deref(), use_letterhead: letterhead })
    }

    // ---------- reading ----------
    pub fn order(&self, id: i64) -> Result<Order> { load_order(&self.conn, id) }

    /// Newest first. `query` matches patient name, patient code or phone.
    pub fn orders(&self, query: &str, limit: usize) -> Result<Vec<OrderSummary>> {
        let like = format!("%{}%", s(query, 100));
        let mut st = self.conn.prepare("SELECT o.id,o.created_at,o.status,o.total_paise,
            COALESCE((SELECT SUM(amount_paise) FROM payments WHERE order_id=o.id),0),
            p.patient_code,p.designation,p.first_name,p.last_name,p.age,p.age_type,p.gender
            FROM orders o JOIN patients p ON p.id=o.patient_id
            WHERE o.deleted_at IS NULL AND (?1='%%' OR p.first_name LIKE ?1 OR p.last_name LIKE ?1 OR p.patient_code LIKE ?1 OR p.phone LIKE ?1)
            ORDER BY o.id DESC LIMIT ?2")?;
        let rows = st.query_map(params![like, limit as i64], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, i64>(4)?,
            r.get::<_, String>(5)?, r.get::<_, String>(6)?, r.get::<_, String>(7)?, r.get::<_, String>(8)?, r.get::<_, f64>(9)?, r.get::<_, String>(10)?, r.get::<_, String>(11)?)))?;
        let mut out = Vec::new();
        for r in rows {
            let (id, created_at, status, total_paise, paid_paise, patient_code, des, first, last, age, age_type, gender) = r?;
            let mut names = self.conn.prepare("SELECT snapshot FROM order_tests WHERE order_id=? ORDER BY id")?;
            let test_names = names.query_map([id], |r| r.get::<_, String>(0))?
                .map(|x| Ok(serde_json::from_str::<Value>(&x?)?["name"].as_str().unwrap_or("").to_string())).collect::<Result<Vec<_>>>()?;
            out.push(OrderSummary { id, created_at, status, total_paise, paid_paise, patient_code,
                patient_name: format!("{des} {first} {last}").trim().to_string(),
                age_gender: format!("{} {} / {gender}", age, age_type.chars().next().map(|c| c.to_ascii_lowercase()).unwrap_or('y')), test_names });
        }
        Ok(out)
    }

    pub fn audit_log(&self, limit: usize) -> Result<Vec<AuditEntry>> {
        let mut st = self.conn.prepare("SELECT created_at,actor,action,entity,entity_id FROM audit ORDER BY id DESC LIMIT ?")?;
        let rows = st.query_map([limit as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }
}

#[derive(Debug, Clone)]
pub struct DeletedOrder { pub id: i64, pub deleted_at: String, pub deleted_by: String, pub reason: String, pub patient_code: String, pub patient_name: String, pub total_paise: i64 }

/// (time, actor, action, entity, entity id)
pub type AuditEntry = (String, String, String, String, String);

#[derive(Debug, Clone)]
pub struct ResultInput { pub order_test_id: i64, pub parameter_id: String, pub value: String, pub comment: Option<String>, pub manual: bool }

fn order_fully_approved(c: &Connection, id: i64) -> Result<bool> {
    Ok(c.query_row("SELECT approved_snapshot IS NOT NULL FROM orders WHERE id=?", [id], |r| r.get::<_, bool>(0)).optional()?.unwrap_or(false))
}

fn load_test(c: &Connection, id: i64) -> Result<TestDef> {
    let (code, name, department, sample, price_paise, d, cfg): (String, String, String, String, i64, i64, String) = c
        .query_row("SELECT code,name,department,sample,price_paise,disabled,config FROM tests WHERE id=?", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))
        .optional()?.ok_or_else(|| Error::NotFound("Test not found".into()))?;
    Ok(TestDef { id, code, name, department, sample, price_paise, disabled: d != 0, config: serde_json::from_str(&cfg)? })
}

pub(crate) fn load_order(c: &Connection, id: i64) -> Result<Order> {
    let row = c.query_row("SELECT patient_id,referral_name,created_at,status,subtotal_paise,discount_paise,total_paise,version,notes,interpretation,bill_note,deleted_at,age,age_type FROM orders WHERE id=?", [id],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)?, r.get::<_, i64>(5)?, r.get::<_, i64>(6)?, r.get::<_, i64>(7)?, r.get::<_, String>(8)?, r.get::<_, String>(9)?, r.get::<_, String>(10)?, r.get::<_, Option<String>>(11)?, r.get::<_, Option<f64>>(12)?, r.get::<_, Option<String>>(13)?)))
        .optional()?.ok_or_else(|| Error::NotFound("Order not found".into()))?;
    let mut patient = c.query_row("SELECT id,patient_code,designation,first_name,last_name,age,age_type,gender,phone,email,address FROM patients WHERE id=?", [row.0],
        |r| Ok(Patient { id: r.get(0)?, patient_code: r.get(1)?, designation: r.get(2)?, first_name: r.get(3)?, last_name: r.get(4)?, age: r.get(5)?, age_type: r.get(6)?, gender: r.get(7)?, phone: r.get(8)?, email: r.get(9)?, address: r.get(10)? }))?;
    if let (Some(a), Some(t)) = (row.12, row.13.clone()) { patient.age = a; patient.age_type = t; }
    let mut st = c.prepare("SELECT id,test_id,snapshot,status,approved_at,approved_by_name FROM order_tests WHERE order_id=? ORDER BY id")?;
    let raw = st.query_map([id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, Option<String>>(5)?)))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut tests = Vec::new();
    for (tid, test_id, snap, status, approved_at, approved_by_name) in raw {
        let snap: Value = serde_json::from_str(&snap)?;
        let mut rs = c.prepare("SELECT parameter_id,value,flag,range_text,comment,manual FROM results WHERE order_test_id=?")?;
        let results = rs.query_map([tid], |r| Ok(ResultRow { parameter_id: r.get(0)?, value: r.get(1)?, flag: r.get(2)?, range_text: r.get(3)?, comment: r.get(4)?, manual: r.get::<_, i64>(5)? != 0 }))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let machine: Option<MachineData> = c.query_row("SELECT payload FROM machine_results WHERE order_test_id=? AND status='Attached' ORDER BY id DESC LIMIT 1", [tid], |r| r.get::<_, String>(0)).optional()?
            .and_then(|p| serde_json::from_str(&p).ok());
        tests.push(OrderTest { id: tid, test_id, name: snap["name"].as_str().unwrap_or("").into(),
            department: snap["department"].as_str().unwrap_or("").into(), sample: snap["sample"].as_str().unwrap_or("").into(), status,
            config: serde_json::from_value(snap["config"].clone())?, results, approved_at, approved_by_name,
            price_paise: snap["price_paise"].as_i64().unwrap_or(0), discount_paise: snap["discount_paise"].as_i64().unwrap_or(0), machine });
    }
    let mut ps = c.prepare("SELECT id,amount_paise,method,created_at,reference FROM payments WHERE order_id=? ORDER BY id")?;
    let payments = ps.query_map([id], |r| Ok(Payment { id: r.get(0)?, amount_paise: r.get(1)?, method: r.get(2)?, created_at: r.get(3)?, reference: r.get(4)? }))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(Order { id, patient, referral_name: row.1, created_at: row.2, status: row.3, subtotal_paise: row.4, discount_paise: row.5, total_paise: row.6, version: row.7, notes: row.8, interpretation: row.9, bill_note: row.10, deleted_at: row.11, tests, payments })
}


/// Bill printing choices, remembered for the lab.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BillPrefs {
    /// Print the lab's name and address (or letterhead) on bills. Off by default: most labs use printed paper.
    pub header: bool,
    /// "a4", "roll80" (80 mm roll) or "roll58" (58 mm roll).
    pub paper: String,
}
impl Default for BillPrefs { fn default() -> Self { Self { header: false, paper: "a4".into() } } }
impl BillPrefs {
    pub fn roll_mm(&self) -> Option<f32> { match self.paper.as_str() { "roll80" => Some(80.0), "roll58" => Some(58.0), _ => None } }
}

/// Version of the Terms of Use and Privacy text (`TERMS.txt`). Raise it when the text changes, and everyone is asked to accept again.
pub const TERMS_VERSION: u32 = 1;
