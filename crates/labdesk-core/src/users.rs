// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Sign-in accounts. Passwords are never stored: only a salted PBKDF2-HMAC-SHA256 hash.
use crate::error::{check, Error, Result};
use crate::model::{Actor, Role, User};
use crate::store::{allow, audit, now, s, Store};
use rusqlite::{params, OptionalExtension};
use serde_json::json;
use sha2::{Digest, Sha256};

#[cfg(debug_assertions)]
const ITERATIONS: u32 = 2_000; // fast for tests and development builds; the count is stored per password
#[cfg(not(debug_assertions))]
const ITERATIONS: u32 = 150_000;

/// Wrong passwords allowed in a row, and how long sign-in stays locked after that.
pub const MAX_ATTEMPTS: i64 = 5;
pub const LOCK_SECONDS: i64 = 60;

fn hmac(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 { k[..32].copy_from_slice(&Sha256::digest(key)); } else { k[..key.len()].copy_from_slice(key); }
    let (mut ipad, mut opad) = ([0x36u8; 64], [0x5cu8; 64]);
    for i in 0..64 { ipad[i] ^= k[i]; opad[i] ^= k[i]; }
    let mut h = Sha256::new(); h.update(ipad); h.update(msg);
    let inner = h.finalize();
    let mut h = Sha256::new(); h.update(opad); h.update(inner);
    h.finalize().into()
}
pub(crate) fn pbkdf2(password: &[u8], salt: &[u8], iters: u32) -> [u8; 32] {
    let mut first = salt.to_vec(); first.extend_from_slice(&1u32.to_be_bytes());
    let mut u = hmac(password, &first);
    let mut out = u;
    for _ in 1..iters { u = hmac(password, &u); for i in 0..32 { out[i] ^= u[i]; } }
    out
}
pub(crate) fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }
fn unhex(t: &str) -> Option<Vec<u8>> { (0..t.len()).step_by(2).map(|i| u8::from_str_radix(t.get(i..i + 2)?, 16).ok()).collect() }

fn hash_password(password: &str) -> Result<String> {
    let mut salt = [0u8; 16];
    getrandom::fill(&mut salt).map_err(|e| Error::Rule(format!("The computer could not make a secure random number: {e}")))?;
    Ok(format!("pbkdf2-sha256${ITERATIONS}${}${}", hex(&salt), hex(&pbkdf2(password.as_bytes(), &salt, ITERATIONS))))
}
fn verify_password(password: &str, stored: &str) -> bool {
    let p: Vec<&str> = stored.split('$').collect();
    let (Some(iters), Some(salt), Some(want)) = (p.get(1).and_then(|i| i.parse::<u32>().ok()), p.get(2).and_then(|x| unhex(x)), p.get(3).and_then(|x| unhex(x))) else { return false };
    if p.first() != Some(&"pbkdf2-sha256") || iters == 0 { return false; }
    let got = pbkdf2(password.as_bytes(), &salt, iters);
    got.len() == want.len() && got.iter().zip(&want).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0
}

pub fn clean_username(u: &str) -> Result<String> {
    let u = u.trim().to_lowercase();
    check(u.len() >= 3, "The username must have at least 3 letters or numbers")?;
    check(u.len() <= 32 && u.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')), "The username can only use letters, numbers, dot, dash and underscore (no spaces)")?;
    Ok(u)
}
pub fn check_password(p: &str) -> Result<()> {
    check(p.chars().count() >= 6, "The password must have at least 6 characters")?;
    check(p.chars().count() <= 100, "That password is too long")
}
fn row_user(r: &rusqlite::Row) -> rusqlite::Result<User> {
    let role: String = r.get(3)?;
    Ok(User { id: r.get(0)?, username: r.get(1)?, name: r.get(2)?, role: Role::parse(&role).unwrap_or(Role::Reception), active: r.get::<_, i64>(4)? != 0 })
}
const COLS: &str = "id,username,name,role,active,password_hash";

/// Letters and digits that cannot be mistaken for each other (no 0/O, 1/I/L).
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
fn new_code() -> Result<String> {
    let mut raw = [0u8; 20];
    getrandom::fill(&mut raw).map_err(|e| Error::Rule(format!("The computer could not make a secure random number: {e}")))?;
    let chars: Vec<char> = raw.iter().map(|b| CODE_ALPHABET[*b as usize % CODE_ALPHABET.len()] as char).collect();
    Ok(chars.chunks(5).map(|c| c.iter().collect::<String>()).collect::<Vec<_>>().join("-"))
}
fn normalise_code(c: &str) -> String { c.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_uppercase() }


impl Store {
    // ---------- wrong-password limit (shared by every place a password is typed) ----------
    /// Seconds left before passwords are accepted again; 0 when not locked. Survives closing the app.
    pub fn lock_seconds_left(&self) -> Result<u64> {
        let until: Option<String> = self.conn.query_row("SELECT locked_until FROM login_guard WHERE id=1", [], |r| r.get(0)).optional()?.flatten();
        let Some(until) = until else { return Ok(0) };
        let Ok(t) = chrono::DateTime::parse_from_rfc3339(&until) else { return Ok(0) };
        let left = (t.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_milliseconds();
        Ok(if left > 0 { (left as u64).div_ceil(1000) } else { 0 })
    }
    fn guard_check(&self) -> Result<()> {
        match self.lock_seconds_left()? { 0 => Ok(()), n => Err(Error::Locked(n)) }
    }
    /// Count a wrong password. The 5th in a row starts the lock and the error says how long.
    fn guard_fail(&self) -> Result<()> {
        let fails: i64 = self.conn.query_row("SELECT fails FROM login_guard WHERE id=1", [], |r| r.get(0)).optional()?.unwrap_or(0) + 1;
        if fails >= MAX_ATTEMPTS {
            let until = (chrono::Utc::now() + chrono::Duration::seconds(self.lock_secs)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            self.conn.execute("INSERT INTO login_guard(id,fails,locked_until) VALUES(1,0,?1) ON CONFLICT(id) DO UPDATE SET fails=0,locked_until=?1", [until])?;
            let _ = audit(&self.conn, &Actor::new("system", Role::Reception), "sign_in_locked", "user", 0, json!({ "seconds": self.lock_secs }));
            Err(Error::Locked(self.lock_secs.max(0) as u64))
        } else {
            self.conn.execute("INSERT INTO login_guard(id,fails) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET fails=?1", [fails])?;
            Ok(())
        }
    }
    fn guard_ok(&self) -> Result<()> {
        self.conn.execute("UPDATE login_guard SET fails=0,locked_until=NULL WHERE id=1", [])?;
        Ok(())
    }
    #[doc(hidden)]
    pub fn set_lock_seconds(&mut self, n: i64) { self.lock_secs = n; }
    /// Wrong attempts left before the lock (for a gentle hint on the sign-in screen).
    pub fn attempts_left(&self) -> Result<i64> {
        let fails: i64 = self.conn.query_row("SELECT fails FROM login_guard WHERE id=1", [], |r| r.get(0)).optional()?.unwrap_or(0);
        Ok((MAX_ATTEMPTS - fails).max(0))
    }

    // ---------- staying signed in ----------
    /// Remember a sign-in. Returns a secret token for the app to keep in a file; only its hash is stored here.
    /// The sign-in is kept for 30 days after last use, or until the person signs out.
    pub fn create_session(&mut self, user_id: i64) -> Result<String> {
        let mut raw = [0u8; 32];
        getrandom::fill(&mut raw).map_err(|e| Error::Rule(format!("The computer could not make a secure random number: {e}")))?;
        let token = hex(&raw);
        let ts = |d: i64| (chrono::Utc::now() + chrono::Duration::days(d)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        self.conn.execute("DELETE FROM sessions WHERE expires_at<?", [now()])?;
        self.conn.execute("INSERT INTO sessions(user_id,token_hash,created_at,expires_at) VALUES(?,?,?,?)", params![user_id, hex(&Sha256::digest(token.as_bytes())), now(), ts(SESSION_DAYS)])?;
        Ok(token)
    }
    /// The person this saved sign-in belongs to, if it is still valid and their account is still switched on.
    pub fn resume_session(&mut self, token: &str) -> Result<Option<User>> {
        let h = hex(&Sha256::digest(token.trim().as_bytes()));
        let row: Option<(i64, i64, String)> = self.conn.query_row("SELECT id,user_id,expires_at FROM sessions WHERE token_hash=?", [&h], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional()?;
        let Some((sid, uid, exp)) = row else { return Ok(None) };
        let alive = chrono::DateTime::parse_from_rfc3339(&exp).is_ok_and(|t| t.with_timezone(&chrono::Utc) > chrono::Utc::now());
        let user = self.conn.query_row(&format!("SELECT {COLS} FROM users WHERE id=?"), [uid], row_user).optional()?;
        match user {
            Some(u) if alive && u.active => {
                let exp = (chrono::Utc::now() + chrono::Duration::days(SESSION_DAYS)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
                self.conn.execute("UPDATE sessions SET expires_at=? WHERE id=?", params![exp, sid])?;
                Ok(Some(u))
            }
            _ => { self.conn.execute("DELETE FROM sessions WHERE id=?", [sid])?; Ok(None) }
        }
    }
    /// Forget one saved sign-in (the person pressed Sign out).
    pub fn end_session(&mut self, token: &str) -> Result<()> {
        self.conn.execute("DELETE FROM sessions WHERE token_hash=?", [hex(&Sha256::digest(token.trim().as_bytes()))])?;
        Ok(())
    }
}
const SESSION_DAYS: i64 = 30;

impl Store {
    /// Make a new recovery code (replacing any older one) and return it. Show it once; only a hash is kept.
    /// With it, an Admin who forgot their password can choose a new one from the sign-in screen.
    pub fn new_recovery_code(&mut self, actor: &Actor) -> Result<String> {
        allow(actor, &[Role::Admin])?;
        let code = new_code()?;
        let hash = hash_password(&normalise_code(&code))?;
        self.conn.execute("INSERT INTO settings(key,value) VALUES('recovery',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [hash])?;
        audit(&self.conn, actor, "new_recovery_code", "user", 0, json!({}))?;
        Ok(code)
    }
    pub fn has_recovery_code(&self) -> Result<bool> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM settings WHERE key='recovery'", [], |r| r.get::<_, i64>(0))? > 0)
    }
    /// An Admin forgot their password: the username plus the recovery code set a new password.
    /// The used code stops working; the returned user and a fresh code are handed back.
    pub fn recover_admin(&mut self, username: &str, code: &str, new_password: &str) -> Result<(User, String)> {
        self.guard_check()?;
        let bad = || Error::Rule("The username or the recovery code is not correct".into());
        let stored: Option<String> = self.conn.query_row("SELECT value FROM settings WHERE key='recovery'", [], |r| r.get(0)).optional()?;
        let u = username.trim().to_lowercase();
        let user: Option<User> = self.conn.query_row(&format!("SELECT {COLS} FROM users WHERE username=?"), [&u], row_user).optional()?;
        let code_ok = verify_password(&normalise_code(code), stored.as_deref().unwrap_or("pbkdf2-sha256$1$00$00"));
        let user = match (user, code_ok) { (Some(x), true) if x.role == Role::Admin && x.active => x, _ => { self.guard_fail()?; return Err(bad()); } };
        self.guard_ok()?;
        check_password(new_password)?;
        self.conn.execute("DELETE FROM sessions WHERE user_id=?", [user.id])?;
        self.conn.execute("UPDATE users SET password_hash=? WHERE id=?", params![hash_password(new_password)?, user.id])?;
        audit(&self.conn, &user.actor(), "recover_password", "user", user.id, json!({}))?;
        let fresh = self.new_recovery_code(&user.actor())?;
        Ok((user, fresh))
    }
    /// An Admin forgot their username: the recovery code shows the usernames of the active Admins.
    /// Wrong codes count towards the same sign-in lock as wrong passwords. The code is not used up.
    pub fn admin_usernames(&mut self, code: &str) -> Result<Vec<String>> {
        self.guard_check()?;
        let stored: Option<String> = self.conn.query_row("SELECT value FROM settings WHERE key='recovery'", [], |r| r.get(0)).optional()?;
        let ok = verify_password(&normalise_code(code), stored.as_deref().unwrap_or("pbkdf2-sha256$1$00$00"));
        if !ok { self.guard_fail()?; return Err(Error::Rule("The recovery code is not correct".into())); }
        self.guard_ok()?;
        let mut st = self.conn.prepare(&format!("SELECT {COLS} FROM users ORDER BY id"))?;
        let names: Vec<String> = st.query_map([], row_user)?.filter_map(|u| u.ok()).filter(|u| u.role == Role::Admin && u.active).map(|u| u.username).collect();
        drop(st);
        let _ = audit(&self.conn, &Actor::new("recovery code", Role::Reception), "show_admin_usernames", "user", 0, json!({}));
        Ok(names)
    }
    /// True once at least one account exists. Until then the app asks for the first Admin to be created.
    pub fn has_users(&self) -> Result<bool> {
        Ok(self.conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))? > 0)
    }
    /// Create the very first account (always an Admin). Refused if any account already exists.
    pub fn create_first_admin(&mut self, username: &str, name: &str, password: &str) -> Result<User> {
        check(!self.has_users()?, "An Admin account already exists. Please sign in.")?;
        let (u, n) = (clean_username(username)?, s(name, 120));
        check(!n.is_empty(), "Please enter your name")?;
        check_password(password)?;
        let h = hash_password(password)?;
        self.conn.execute("INSERT INTO users(username,name,password_hash,role,active) VALUES(?,?,?,?,1)", params![u, n, h, Role::Admin.as_str()])?;
        let id = self.conn.last_insert_rowid();
        let user = User { id, username: u, name: n, role: Role::Admin, active: true };
        audit(&self.conn, &user.actor(), "create", "user", id, json!({ "first_admin": true }))?;
        Ok(user)
    }
    /// Check a username and password. The error never says which of the two was wrong.
    pub fn authenticate(&mut self, username: &str, password: &str) -> Result<User> {
        self.guard_check()?;
        let u = username.trim().to_lowercase();
        let row: Option<(User, String)> = self.conn.query_row(&format!("SELECT {COLS} FROM users WHERE username=?"), [&u], |r| Ok((row_user(r)?, r.get::<_, String>(5)?))).optional()?;
        let bad = || Error::Rule("The username or password is not correct".into());
        let (user, hash) = match row { Some(x) => x, None => { let _ = verify_password(password, "pbkdf2-sha256$1$00$00"); self.guard_fail()?; return Err(bad()); } };
        if !verify_password(password, &hash) { self.guard_fail()?; return Err(bad()); }
        self.guard_ok()?;
        check(user.active, "This account has been switched off. Please ask the Admin.")?;
        audit(&self.conn, &user.actor(), "sign_in", "user", user.id, json!({}))?;
        Ok(user)
    }
    pub fn users(&self) -> Result<Vec<User>> {
        let mut st = self.conn.prepare(&format!("SELECT {COLS} FROM users ORDER BY active DESC, lower(name)"))?;
        let rows = st.query_map([], row_user)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }
    pub fn create_user(&mut self, actor: &Actor, username: &str, name: &str, role: Role, password: &str) -> Result<User> {
        allow(actor, &[Role::Admin])?;
        let (u, n) = (clean_username(username)?, s(name, 120));
        check(!n.is_empty(), "Please enter the person's name")?;
        check_password(password)?;
        let taken: Option<i64> = self.conn.query_row("SELECT id FROM users WHERE username=?", [&u], |r| r.get(0)).optional()?;
        check(taken.is_none(), format!("The username \"{u}\" is already taken. Please choose another."))?;
        self.conn.execute("INSERT INTO users(username,name,password_hash,role,active) VALUES(?,?,?,?,1)", params![u, n, hash_password(password)?, role.as_str()])?;
        let id = self.conn.last_insert_rowid();
        audit(&self.conn, actor, "create", "user", id, json!({ "username": u, "role": role.as_str() }))?;
        Ok(User { id, username: u, name: n, role, active: true })
    }
    /// Change a person's name, role, or switch the account off/on. At least one active Admin must always remain.
    pub fn update_user(&mut self, actor: &Actor, id: i64, name: &str, role: Role, active: bool) -> Result<()> {
        allow(actor, &[Role::Admin])?;
        let n = s(name, 120);
        check(!n.is_empty(), "Please enter the person's name")?;
        let cur = self.conn.query_row(&format!("SELECT {COLS} FROM users WHERE id=?"), [id], row_user).optional()?.ok_or_else(|| Error::NotFound("user".into()))?;
        if cur.role == Role::Admin && cur.active && (role != Role::Admin || !active) {
            let others: i64 = self.conn.query_row("SELECT COUNT(*) FROM users WHERE role='admin' AND active=1 AND id<>?", [id], |r| r.get(0))?;
            check(others > 0, "There must always be at least one active Admin. First make another person an Admin, then you can change or switch off this account.")?;
        }
        self.conn.execute("UPDATE users SET name=?,role=?,active=? WHERE id=?", params![n, role.as_str(), active as i64, id])?;
        if !active { self.conn.execute("DELETE FROM sessions WHERE user_id=?", [id])?; }
        audit(&self.conn, actor, "edit", "user", id, json!({ "role": role.as_str(), "active": active }))?;
        Ok(())
    }
    /// Admin sets a new password for someone else (for example when they forgot theirs).
    /// An Admin's own password can only be changed by that Admin (with the current password), never by someone else.
    pub fn reset_password(&mut self, actor: &Actor, id: i64, new_password: &str) -> Result<()> {
        allow(actor, &[Role::Admin])?;
        check_password(new_password)?;
        let target = self.conn.query_row(&format!("SELECT {COLS} FROM users WHERE id=?"), [id], row_user).optional()?.ok_or_else(|| Error::NotFound("user".into()))?;
        check(target.role != Role::Admin, "An Admin's password can only be changed by that Admin, from My account, using the current password.")?;
        let n = self.conn.execute("UPDATE users SET password_hash=? WHERE id=?", params![hash_password(new_password)?, id])?;
        self.conn.execute("DELETE FROM sessions WHERE user_id=?", [id])?;
        check(n == 1, "That account no longer exists")?;
        audit(&self.conn, actor, "reset_password", "user", id, json!({}))?;
        Ok(())
    }
    /// Ask a signed-in person to type their own password again before something important.
    pub fn confirm_password(&mut self, user_id: i64, password: &str) -> Result<()> {
        self.guard_check()?;
        let hash: Option<String> = self.conn.query_row("SELECT password_hash FROM users WHERE id=? AND active=1", [user_id], |r| r.get(0)).optional()?;
        if hash.is_some_and(|h| verify_password(password, &h)) { self.guard_ok() } else { self.guard_fail()?; Err(Error::Rule("That password is not correct".into())) }
    }
    /// Remove an account for good. You cannot remove yourself, and the last Admin can never be removed.
    pub fn delete_user(&mut self, actor: &Actor, my_id: i64, id: i64) -> Result<()> {
        allow(actor, &[Role::Admin])?;
        check(id != my_id, "You cannot remove your own account while you are signed in with it.")?;
        let target = self.conn.query_row(&format!("SELECT {COLS} FROM users WHERE id=?"), [id], row_user).optional()?.ok_or_else(|| Error::NotFound("user".into()))?;
        if target.role == Role::Admin && target.active {
            let others: i64 = self.conn.query_row("SELECT COUNT(*) FROM users WHERE role='admin' AND active=1 AND id<>?", [id], |r| r.get(0))?;
            check(others > 0, "There must always be at least one active Admin.")?;
        }
        let used: i64 = self.conn.query_row("SELECT COUNT(*) FROM orders WHERE approved_by=?", [id], |r| r.get(0))?;
        check(used == 0, "This person has approved reports, so the account cannot be removed. Switch it off instead.")?;
        self.conn.execute("DELETE FROM users WHERE id=?", [id])?;
        audit(&self.conn, actor, "remove", "user", id, json!({ "username": target.username, "name": target.name }))?;
        Ok(())
    }
    /// A signed-in person changes their own password; the old one must be right.
    pub fn change_password(&mut self, user_id: i64, old: &str, new: &str) -> Result<()> {
        let (user, hash): (User, String) = self.conn.query_row(&format!("SELECT {COLS} FROM users WHERE id=?"), [user_id], |r| Ok((row_user(r)?, r.get(5)?)))
            .optional()?.ok_or_else(|| Error::NotFound("user".into()))?;
        self.guard_check()?;
        if !verify_password(old, &hash) { self.guard_fail()?; return Err(Error::Rule("The current password is not correct".into())); }
        self.guard_ok()?;
        check_password(new)?;
        self.conn.execute("UPDATE users SET password_hash=? WHERE id=?", params![hash_password(new)?, user_id])?;
        self.conn.execute("DELETE FROM sessions WHERE user_id=?", [user_id])?;
        audit(&self.conn, &user.actor(), "change_password", "user", user_id, json!({ "at": now() }))?;
        Ok(())
    }
}
