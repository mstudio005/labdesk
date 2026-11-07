// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Settings for sending a report by email (SMTP). Off until the Admin turns it on.
//! The sending itself lives in the desktop crate; this file keeps the settings, the provider presets and the rules.
use crate::error::{check, Result};
use crate::model::*;
use crate::store::{allow, audit, Store};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmailSettings {
    pub enabled: bool,
    /// "proton", "gmail", "outlook" or "custom".
    pub provider: String,
    pub host: String,
    pub port: u16,
    /// The SMTP user name: usually the full email address.
    pub username: String,
    /// An app password (Gmail, Outlook) or an SMTP token (Proton). Never the normal account password.
    pub password: String,
    /// The name patients see, for example the lab's name. Empty means the lab name.
    pub from_name: String,
}
impl Default for EmailSettings {
    fn default() -> Self { Self { enabled: false, provider: "proton".into(), host: "smtp.protonmail.ch".into(), port: 587, username: String::new(), password: String::new(), from_name: String::new() } }
}
impl EmailSettings {
    /// True when everything needed to send is filled in.
    pub fn ready(&self) -> bool { self.enabled && !self.host.trim().is_empty() && self.port != 0 && valid_address(&self.username) && !self.password.is_empty() }
}

/// Server name and port for a provider; None for "custom".
pub fn preset(provider: &str) -> Option<(&'static str, u16)> {
    match provider { "proton" => Some(("smtp.protonmail.ch", 587)), "gmail" => Some(("smtp.gmail.com", 587)), "outlook" => Some(("smtp-mail.outlook.com", 587)), _ => None }
}
/// A plain check that something looks like name@domain.tld. The mail server has the last word.
pub fn valid_address(a: &str) -> bool {
    let a = a.trim();
    if a.len() > 254 || a.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ',' | ';' | '"' | '(' | ')')) { return false; }
    let mut parts = a.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else { return false };
    !local.is_empty() && local.len() <= 64 && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') && !domain.contains("..")
}

impl Store {
    pub fn email_settings(&self) -> Result<EmailSettings> {
        let v: Option<String> = self.conn.query_row("SELECT value FROM settings WHERE key='email'", [], |r| r.get(0)).optional()?;
        Ok(match v { Some(v) => serde_json::from_str(&v)?, None => EmailSettings::default() })
    }
    pub fn save_email_settings(&mut self, actor: &Actor, e: &EmailSettings) -> Result<()> {
        allow(actor, &[Role::Admin])?;
        let mut clean = EmailSettings { enabled: e.enabled, provider: e.provider.trim().to_string(), host: e.host.trim().to_string(), port: e.port, username: e.username.trim().to_string(), password: e.password.clone(), from_name: e.from_name.trim().chars().take(80).collect() };
        if let Some((h, p)) = preset(&clean.provider) { clean.host = h.into(); clean.port = p; } else { clean.provider = "custom".into(); }
        if clean.enabled {
            check(valid_address(&clean.username), "Please type your full email address (for example lab@example.com) as the user name")?;
            check(!clean.password.is_empty(), "Please type the app password (or SMTP token) from your email provider")?;
            check(!clean.host.is_empty() && !clean.host.contains(char::is_whitespace), "Please type the mail server name")?;
            check(clean.port != 0, "Please type the mail server port, usually 587")?;
        }
        self.conn.execute("INSERT INTO settings(key,value) VALUES('email',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value", [serde_json::to_string(&clean)?])?;
        audit(&self.conn, actor, "email_settings", "settings", 0, json!({ "enabled": clean.enabled, "provider": clean.provider, "host": clean.host }))?;
        Ok(())
    }
    /// Write down in the audit log that a report was emailed (or that sending failed). The address is kept, the contents are not.
    pub fn log_email(&mut self, actor: &Actor, order_id: i64, to: &str, sent: bool) -> Result<()> {
        allow(actor, &[Role::Admin, Role::Technician, Role::Pathologist, Role::Reception])?;
        audit(&self.conn, actor, if sent { "email_sent" } else { "email_failed" }, "order", order_id, json!({ "to": to }))
    }
}
