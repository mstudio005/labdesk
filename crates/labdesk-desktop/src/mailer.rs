// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Sending a report by email through the lab's own mail account (SMTP over an encrypted connection).
//! Nothing here runs unless the Admin turned email on and the person pressed EMAIL on a report.
use labdesk_core::EmailSettings;
use lettre::message::{header::ContentType, Attachment, Mailbox, MultiPart, SinglePart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{Message, SmtpTransport, Transport};
use std::time::Duration;

/// Build the message: a short text and the PDF attached. Fails with a plain sentence when an address is not usable.
pub fn build_message(cfg: &EmailSettings, lab_name: &str, to: &str, subject: &str, body: &str, file_name: Option<(&str, &[u8])>) -> Result<Message, String> {
    let shown = if cfg.from_name.trim().is_empty() { lab_name.trim() } else { cfg.from_name.trim() };
    let shown: String = shown.chars().filter(|c| !c.is_control() && !matches!(c, '<' | '>' | '"')).collect();
    let from_addr = cfg.username.trim().parse().map_err(|_| "The sender address in Settings > Email is not a valid email address".to_string())?;
    let to_addr = to.trim().parse().map_err(|_| format!("\"{}\" is not a valid email address", to.trim()))?;
    let b = Message::builder()
        .from(Mailbox::new(if shown.is_empty() { None } else { Some(shown) }, from_addr))
        .to(Mailbox::new(None, to_addr))
        .subject(subject);
    let text = SinglePart::plain(body.to_string());
    let msg = match file_name {
        Some((name, bytes)) => b.multipart(MultiPart::mixed().singlepart(text).singlepart(Attachment::new(name.to_string()).body(bytes.to_vec(), ContentType::parse("application/pdf").map_err(|e| e.to_string())?))),
        None => b.singlepart(text),
    };
    msg.map_err(|e| format!("Could not prepare the email: {e}"))
}

/// Connect, log in and send. Blocks, so call it from a thread.
pub fn send(cfg: &EmailSettings, msg: &Message) -> Result<(), String> {
    let builder = if cfg.port == 465 { SmtpTransport::relay(cfg.host.trim()) } else { SmtpTransport::starttls_relay(cfg.host.trim()) };
    let transport = builder.map_err(|e| format!("Could not use the mail server \"{}\": {e}", cfg.host.trim()))?
        .port(cfg.port)
        .credentials(Credentials::new(cfg.username.trim().to_string(), cfg.password.clone()))
        .timeout(Some(Duration::from_secs(45)))
        .build();
    transport.send(msg).map(|_| ()).map_err(|e| explain(&e.to_string(), cfg))
}

/// Turn a mail-server complaint into something the front desk can act on.
pub fn explain(raw: &str, cfg: &EmailSettings) -> String {
    let l = raw.to_lowercase();
    let what = match cfg.provider.as_str() { "gmail" => "For Gmail make an App Password (Google Account > Security > 2-Step Verification > App passwords).", "outlook" => "For Outlook make an App Password in your Microsoft account security settings. Some Microsoft accounts no longer allow sending this way.", "proton" => "For Proton create an SMTP token (Settings > IMAP/SMTP > SMTP tokens). SMTP needs a paid Proton plan with a custom-domain address, and the user name must be that address.", _ => "Check the user name and password with your email provider." };
    if l.contains("535") || l.contains("authentication") || l.contains("username and password") || l.contains("534") || l.contains("530") { return format!("The mail server did not accept the user name or password. {what}"); }
    if l.contains("timed out") || l.contains("timeout") { return "The mail server did not answer in time. Check the internet connection and try again.".into(); }
    if l.contains("dns") || l.contains("resolve") || l.contains("failed to lookup") || l.contains("no such host") { return format!("Could not find the mail server \"{}\". Check the internet connection and the server name in Settings > Email.", cfg.host.trim()); }
    if l.contains("connection refused") || l.contains("os error 10061") || l.contains("unreachable") { return format!("Could not connect to {} on port {}. Check the port (usually 587) and that the internet works.", cfg.host.trim(), cfg.port); }
    if l.contains("tls") || l.contains("certificate") || l.contains("handshake") { return "A secure connection to the mail server could not be made. Check the port: use 587 (or 465 for some providers).".into(); }
    if l.contains("552") || l.contains("size") { return "The mail server says the email is too large.".into(); }
    if l.contains("550") || l.contains("553") || l.contains("recipient") { return "The mail server did not accept the address. Please check the patient's email address.".into(); }
    format!("The email could not be sent: {}", raw.chars().take(200).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cfg() -> EmailSettings { EmailSettings { enabled: true, provider: "proton".into(), host: "smtp.protonmail.ch".into(), port: 587, username: "lab@example.com".into(), password: "tok".into(), from_name: "City Lab".into() } }
    #[test]
    fn the_message_has_the_pdf_attached_and_the_right_people() {
        let m = build_message(&cfg(), "Lab", "ali@example.com", "Your report", "Hello", Some(("P-1 Ali Khan.pdf", b"%PDF-1.4 test"))).unwrap();
        let raw = String::from_utf8_lossy(&m.formatted()).to_string();
        assert!(raw.contains("City Lab") && raw.contains("lab@example.com") && raw.contains("ali@example.com"));
        assert!(raw.contains("application/pdf") && raw.contains("P-1 Ali Khan.pdf"));
    }
    #[test]
    fn bad_addresses_and_header_tricks_are_refused() {
        assert!(build_message(&cfg(), "Lab", "not an address", "s", "b", None).is_err());
        assert!(build_message(&cfg(), "Lab", "a@b.com\r\nBcc: z@z.com", "s", "b", None).is_err());
        let m = build_message(&cfg(), "Lab", "a@b.com", "Report\r\nBcc: z@z.com", "b", None).unwrap();
        assert!(!String::from_utf8_lossy(&m.formatted()).contains("\nBcc:"));
    }
    #[test]
    fn complaints_are_explained() {
        assert!(explain("permanent error (535): authentication failed", &cfg()).contains("SMTP token"));
        assert!(explain("connection timed out", &cfg()).contains("did not answer"));
    }
}
