// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! The doorway for the LabDesk Bridge browser extension.
//!
//! When the person presses WHATSAPP on a report, LabDesk puts the report PDF in a one-time slot here and opens WhatsApp
//! Web with a secret one-time code in the address (`ld=...`). The extension reads that code from the page address and
//! asks this listener for the file. Rules that keep it safe:
//! * it listens on 127.0.0.1 only (this computer, never the network);
//! * the file is only handed out to someone who knows the 128-bit one-time code, once, within ten minutes;
//! * web pages cannot call it (requests from a web page carry a foreign Origin and are refused; only browser
//!   extensions are accepted, and the extra header makes browsers ask first);
//! * nothing is ever sent to WhatsApp by this program: the extension only attaches the file, the person presses Send.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The extension tries these ports in order.
pub const FIRST_PORT: u16 = 47653;
pub const PORT_COUNT: u16 = 10;
const LIFETIME: Duration = Duration::from_secs(600);

struct Share { id: String, file: String, pdf: Vec<u8>, expires: Instant }
pub struct Bridge { slot: Mutex<Option<Share>> }

impl Bridge {
    pub fn new() -> Bridge { Bridge { slot: Mutex::new(None) } }
    /// Put a report in the slot (replacing any older one) and return the one-time code.
    pub fn offer(&self, file: &str, pdf: Vec<u8>) -> String {
        let mut raw = [0u8; 16];
        getrandom::fill(&mut raw).expect("the system has random numbers");
        let id: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        *self.slot.lock().expect("bridge lock") = Some(Share { id: id.clone(), file: file.to_string(), pdf, expires: Instant::now() + LIFETIME });
        id
    }
    /// Hand the file out once. A wrong code or an old one gets nothing, and a wrong code does not burn the real one.
    pub fn take(&self, id: &str) -> Option<(String, Vec<u8>)> {
        let mut g = self.slot.lock().expect("bridge lock");
        let ok = matches!(&*g, Some(s) if s.id == id && s.expires > Instant::now() && !id.is_empty());
        if ok { g.take().map(|s| (s.file, s.pdf)) } else { None }
    }
    pub fn clear_expired(&self) {
        let mut g = self.slot.lock().expect("bridge lock");
        if matches!(&*g, Some(s) if s.expires <= Instant::now()) { *g = None; }
    }
}
impl Default for Bridge { fn default() -> Self { Bridge::new() } }

pub fn global() -> &'static Bridge { static B: OnceLock<Bridge> = OnceLock::new(); B.get_or_init(Bridge::new) }

pub struct Reply { pub status: u16, pub headers: Vec<(String, String)>, pub body: Vec<u8> }
impl Reply {
    fn plain(status: u16) -> Reply { Reply { status, headers: vec![], body: vec![] } }
}

fn extension_origin(origin: &str) -> bool { origin.starts_with("chrome-extension://") || origin.starts_with("moz-extension://") }

/// Decide what to answer. `headers` are lower-case names. Pure, so it is tested without a network.
pub fn handle(b: &Bridge, port: u16, method: &str, target: &str, headers: &[(String, String)]) -> Reply {
    let get = |n: &str| headers.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
    // A web page on another site that is rebinding its name to 127.0.0.1 would send its own name as Host.
    let host_ok = matches!(get("host"), Some(h) if h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}"));
    if !host_ok { return Reply::plain(403); }
    let origin = get("origin");
    if let Some(o) = origin { if !extension_origin(o) { return Reply::plain(403); } }
    let mut cors = vec![];
    if let Some(o) = origin {
        cors.push(("Access-Control-Allow-Origin".to_string(), o.to_string()));
        cors.push(("Access-Control-Expose-Headers".to_string(), "X-File-Name".to_string()));
        cors.push(("Vary".to_string(), "Origin".to_string()));
    }
    if method == "OPTIONS" {
        cors.push(("Access-Control-Allow-Headers".to_string(), "x-labdesk-bridge".to_string()));
        cors.push(("Access-Control-Allow-Methods".to_string(), "GET".to_string()));
        return Reply { status: 204, headers: cors, body: vec![] };
    }
    if method != "GET" || get("x-labdesk-bridge") != Some("1") { return Reply { status: 403, headers: cors, body: vec![] }; }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match path {
        "/hello" => {
            cors.push(("Content-Type".into(), "application/json".into()));
            Reply { status: 200, headers: cors, body: format!("{{\"app\":\"LabDesk\",\"bridge\":1,\"version\":\"{}\"}}", env!("CARGO_PKG_VERSION")).into_bytes() }
        }
        "/take" => {
            let id = query.split('&').find_map(|kv| kv.strip_prefix("id=")).unwrap_or("");
            match b.take(id) {
                Some((file, pdf)) => {
                    cors.push(("Content-Type".into(), "application/pdf".into()));
                    cors.push(("X-File-Name".into(), crate::printing::url_encode(&file)));
                    cors.push(("Cache-Control".into(), "no-store".into()));
                    Reply { status: 200, headers: cors, body: pdf }
                }
                None => Reply { status: 404, headers: cors, body: vec![] },
            }
        }
        _ => Reply { status: 404, headers: cors, body: vec![] },
    }
}

fn write_reply(s: &mut TcpStream, r: &Reply) {
    let reason = match r.status { 200 => "OK", 204 => "No Content", 403 => "Forbidden", 404 => "Not Found", _ => "Bad Request" };
    let mut head = format!("HTTP/1.1 {} {}\r\nContent-Length: {}\r\nConnection: close\r\n", r.status, reason, r.body.len());
    for (k, v) in &r.headers { if !v.contains(['\r', '\n']) { head.push_str(&format!("{k}: {v}\r\n")); } }
    head.push_str("\r\n");
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(&r.body);
}

fn serve_one(mut s: TcpStream, port: u16, b: &Bridge) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(3)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(10)));
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match s.read(&mut chunk) { Ok(0) | Err(_) => return, Ok(n) => buf.extend_from_slice(&chunk[..n]) }
        if buf.len() > 8192 { return write_reply(&mut s, &Reply::plain(400)); }
    }
    let text = String::from_utf8_lossy(&buf).to_string();
    let mut lines = text.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, target) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
    let headers: Vec<(String, String)> = lines.take_while(|l| !l.is_empty()).filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string())).collect();
    write_reply(&mut s, &handle(b, port, method, target, &headers));
}

/// Start listening in the background. Returns the port, or None when every port is taken (the clipboard way still works).
pub fn start() -> Option<u16> {
    for port in FIRST_PORT..FIRST_PORT + PORT_COUNT {
        if let Ok(l) = TcpListener::bind(("127.0.0.1", port)) {
            std::thread::Builder::new().name("labdesk-bridge".into()).spawn(move || {
                for conn in l.incoming().flatten() {
                    let b = global();
                    b.clear_expired();
                    // One request at a time is plenty, and a slow client cannot hold up more than a few seconds.
                    serve_one(conn, port, b);
                }
            }).ok()?;
            return Some(port);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn h(list: &[(&str, &str)]) -> Vec<(String, String)> { list.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    const OK: &[(&str, &str)] = &[("host", "127.0.0.1:47653"), ("x-labdesk-bridge", "1"), ("origin", "chrome-extension://abc")];

    #[test]
    fn the_file_is_handed_out_once_to_the_right_code_only() {
        let b = Bridge::new();
        let id = b.offer("P-1 Ali Khan.pdf", b"%PDF".to_vec());
        assert!(b.take("wrong").is_none() && b.take("").is_none());
        let r = handle(&b, 47653, "GET", &format!("/take?id={id}"), &h(OK));
        assert_eq!(r.status, 200);
        assert_eq!(r.body, b"%PDF");
        assert!(r.headers.iter().any(|(k, v)| k == "X-File-Name" && v == "P-1%20Ali%20Khan.pdf"));
        assert_eq!(handle(&b, 47653, "GET", &format!("/take?id={id}"), &h(OK)).status, 404, "second time gets nothing");
    }
    #[test]
    fn a_newer_report_replaces_the_older_one() {
        let b = Bridge::new();
        let a = b.offer("a.pdf", vec![1]);
        let c = b.offer("c.pdf", vec![2]);
        assert!(b.take(&a).is_none());
        assert_eq!(b.take(&c).unwrap().0, "c.pdf");
    }
    #[test]
    fn web_pages_and_strangers_are_refused() {
        let b = Bridge::new();
        let id = b.offer("a.pdf", vec![1]);
        let t = format!("/take?id={id}");
        // a web page: foreign origin
        assert_eq!(handle(&b, 47653, "GET", &t, &h(&[("host", "127.0.0.1:47653"), ("x-labdesk-bridge", "1"), ("origin", "https://evil.example")])).status, 403);
        // DNS rebinding: another host name
        assert_eq!(handle(&b, 47653, "GET", &t, &h(&[("host", "evil.example:47653"), ("x-labdesk-bridge", "1")])).status, 403);
        // missing the marker header
        assert_eq!(handle(&b, 47653, "GET", &t, &h(&[("host", "127.0.0.1:47653"), ("origin", "chrome-extension://abc")])).status, 403);
        // wrong method
        assert_eq!(handle(&b, 47653, "POST", &t, &h(OK)).status, 403);
        // the report is still there after all of that
        assert_eq!(handle(&b, 47653, "GET", &t, &h(OK)).status, 200);
    }
    #[test]
    fn hello_and_preflight() {
        let b = Bridge::new();
        let r = handle(&b, 47653, "GET", "/hello", &h(OK));
        assert_eq!(r.status, 200);
        assert!(String::from_utf8_lossy(&r.body).contains("LabDesk"));
        let p = handle(&b, 47653, "OPTIONS", "/take", &h(&[("host", "127.0.0.1:47653"), ("origin", "moz-extension://x")]));
        assert_eq!(p.status, 204);
        assert_eq!(handle(&b, 47653, "OPTIONS", "/take", &h(&[("host", "127.0.0.1:47653"), ("origin", "https://evil.example")])).status, 403);
    }
    #[test]
    fn a_real_connection_works_end_to_end() {
        let port = start().expect("a free port");
        let id = global().offer("r.pdf", b"%PDF-test".to_vec());
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(s, "GET /take?id={id} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: moz-extension://t\r\nX-LabDesk-Bridge: 1\r\n\r\n").unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).unwrap();
        let text = String::from_utf8_lossy(&out).to_string();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"), "{text}");
        assert!(text.ends_with("%PDF-test"));
        assert!(text.to_lowercase().contains("access-control-allow-origin: moz-extension://t"));
    }
}
