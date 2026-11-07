// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! "Capture machine messages": listens on a network port like a LIS would, saves exactly what an analyzer sends
//! (HL7 over TCP) into files in a folder, and answers with a standard "received" acknowledgement.
//! It only listens and writes files. It never sends commands to the machine and never touches LabDesk's database.
use crate::error::{Error, Result};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SB: u8 = 0x0B; // start of an HL7 message on the wire
const EB: u8 = 0x1C; // end block
const CR: u8 = 0x0D;

#[derive(Default)]
struct Shared { messages: AtomicU64, last: Mutex<String>, keep_files: bool, tx: Mutex<Option<std::sync::mpsc::Sender<String>>> }

pub struct Capture { stop: Arc<AtomicBool>, shared: Arc<Shared>, folder: PathBuf, port: u16, handle: Option<std::thread::JoinHandle<()>> }

impl Capture {
    /// Start listening on `port` (all network cards). Fails with a plain message if another program (such as the analyzer's own software) already uses it.
    pub fn start(port: u16, folder: &Path) -> Result<Capture> { Self::start_with(port, folder, true, None) }
    /// Like `start`, but every complete message is also handed to `tx` (the app saves it in the inbox), and
    /// `keep_files` says whether the raw files are written to `folder` as well.
    pub fn start_with(port: u16, folder: &Path, keep_files: bool, tx: Option<std::sync::mpsc::Sender<String>>) -> Result<Capture> {
        if keep_files { std::fs::create_dir_all(folder).map_err(|e| Error::Rule(format!("Could not create the folder for machine messages: {e}")))?; }
        let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| Error::Rule(format!(
            "Port {port} could not be opened ({e}). Close the analyzer's own software or any other program that listens on this port, then try again.")))?;
        listener.set_nonblocking(true).map_err(|e| Error::Rule(e.to_string()))?;
        let (stop, shared) = (Arc::new(AtomicBool::new(false)), Arc::new(Shared { messages: AtomicU64::new(0), last: Mutex::new(String::new()), keep_files, tx: Mutex::new(tx) }));
        let (s2, sh2, dir) = (stop.clone(), shared.clone(), folder.to_path_buf());
        let handle = std::thread::spawn(move || {
            let mut workers = Vec::new();
            while !s2.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((conn, peer)) => {
                        let (s3, sh3, d3) = (s2.clone(), sh2.clone(), dir.clone());
                        workers.push(std::thread::spawn(move || serve(conn, peer.to_string(), &d3, &sh3, &s3)));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
                    Err(_) => std::thread::sleep(Duration::from_millis(500)),
                }
            }
            for w in workers { let _ = w.join(); }
        });
        Ok(Capture { stop, shared, folder: folder.to_path_buf(), port, handle: Some(handle) })
    }
    pub fn port(&self) -> u16 { self.port }
    pub fn folder(&self) -> &Path { &self.folder }
    /// How many messages have arrived, and a one-line note about the latest.
    pub fn status(&self) -> (u64, String) { (self.shared.messages.load(Ordering::Relaxed), self.shared.last.lock().map(|l| l.clone()).unwrap_or_default()) }
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() { let _ = h.join(); }
    }
}
impl Drop for Capture { fn drop(&mut self) { self.stop(); } }

fn stamp() -> String { chrono::Local::now().format("%Y%m%d-%H%M%S").to_string() }

fn serve(mut conn: TcpStream, peer: String, dir: &Path, sh: &Shared, stop: &AtomicBool) {
    let _ = conn.set_read_timeout(Some(Duration::from_millis(300)));
    let note = |t: String| { if let Ok(mut l) = sh.last.lock() { *l = t; } };
    note(format!("Machine {peer} connected"));
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let raw_path = dir.join(format!("raw-stream-{}.bin", stamp()));
    loop {
        if stop.load(Ordering::Relaxed) { break; }
        match conn.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                if sh.keep_files { if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&raw_path) { let _ = f.write_all(&chunk[..n]); } }
                buf.extend_from_slice(&chunk[..n]);
                while let Some(msg) = take_message(&mut buf) {
                    let n = sh.messages.fetch_add(1, Ordering::Relaxed) + 1;
                    let readable = String::from_utf8_lossy(&msg).replace('\r', "\n");
                    let name = format!("message-{}-{n}.hl7.txt", stamp());
                    if sh.keep_files { let _ = std::fs::write(dir.join(&name), readable.as_bytes()); }
                    if let Ok(g) = sh.tx.lock() { if let Some(tx) = g.as_ref() { let _ = tx.send(String::from_utf8_lossy(&msg).to_string()); } }
                    let head = readable.lines().next().unwrap_or("").chars().take(60).collect::<String>();
                    note(format!("Message {n} saved ({} bytes): {head}", msg.len()));
                    let _ = conn.write_all(&ack_for(&readable));
                }
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
    }
    if !buf.is_empty() {
        let n = sh.messages.fetch_add(1, Ordering::Relaxed) + 1;
        if sh.keep_files { let _ = std::fs::write(dir.join(format!("unframed-{}-{n}.txt", stamp())), String::from_utf8_lossy(&buf).replace('\r', "\n").as_bytes()); }
        note(format!("Data without the usual HL7 frame saved ({} bytes)", buf.len()));
    }
}

/// Cut one complete `<VT> message <FS><CR>` out of the front of `buf`. The message is returned without the frame bytes.
fn take_message(buf: &mut Vec<u8>) -> Option<Vec<u8>> {
    let start = buf.iter().position(|&b| b == SB)?;
    let end = buf[start..].iter().position(|&b| b == EB)? + start;
    let msg = buf[start + 1..end].to_vec();
    let mut cut = end + 1;
    if buf.get(cut) == Some(&CR) { cut += 1; }
    buf.drain(..cut);
    Some(msg)
}

/// The standard "message received" answer: MSH + MSA|AA|<control id of the message>.
fn ack_for(msg: &str) -> Vec<u8> {
    let msh: Vec<&str> = msg.lines().find(|l| l.starts_with("MSH")).map(|l| l.split('|').collect()).unwrap_or_default();
    let control = msh.get(9).copied().unwrap_or("");
    let ts = chrono::Local::now().format("%Y%m%d%H%M%S");
    let text = format!("MSH|^~\\&|LabDesk||{}|{}|{ts}||ACK^R01|{ts}|P|2.3.1\rMSA|AA|{control}\r", msh.get(2).copied().unwrap_or(""), msh.get(3).copied().unwrap_or(""));
    let mut out = vec![SB];
    out.extend_from_slice(text.as_bytes());
    out.push(EB); out.push(CR);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saves_message_and_acknowledges() {
        let dir = tempfile::tempdir().unwrap();
        let port = { let l = TcpListener::bind("127.0.0.1:0").unwrap(); l.local_addr().unwrap().port() };
        let mut cap = Capture::start(port, dir.path()).unwrap();
        assert!(Capture::start(port, dir.path()).is_err(), "port already in use is reported");
        let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        c.write_all(b"\x0bMSH|^~\\&|Analyzer|Lab|||2026||ORU^R01|77|P|2.3.1\rPID|1||000080||Miss Ishrat\rOBX|1|NM|WBC||9.00\r\x1c\r").unwrap();
        let mut got = [0u8; 512];
        let n = c.read(&mut got).unwrap();
        let ack = String::from_utf8_lossy(&got[..n]).to_string();
        assert!(ack.contains("MSA|AA|77"), "{ack}");
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(cap.status().0, 1);
        let saved: Vec<_> = std::fs::read_dir(dir.path()).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert!(saved.iter().any(|n| n.starts_with("message-")), "{saved:?}");
        assert!(saved.iter().any(|n| n.starts_with("raw-stream-")), "{saved:?}");
        drop(c);
        cap.stop();
    }
    #[test]
    fn two_messages_in_one_read() {
        let mut b = b"\x0bAAA\x1c\r\x0bBBB\x1c\r".to_vec();
        assert_eq!(take_message(&mut b).unwrap(), b"AAA");
        assert_eq!(take_message(&mut b).unwrap(), b"BBB");
        assert!(take_message(&mut b).is_none());
    }
}
