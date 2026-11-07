// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Dev tool: starts the bridge, offers a tiny PDF and prints the one-time code, for testing the browser extension.
fn main() {
    let port = labdesk_desktop::bridge::start().expect("a free port");
    let id = labdesk_desktop::bridge::global().offer("P-1 Test Patient.pdf", b"%PDF-1.4\n1 0 obj<<>>endobj\ntrailer<<>>\n%%EOF\n".to_vec());
    println!("PORT {port}\nCODE {id}");
    std::io::Write::flush(&mut std::io::stdout()).ok();
    std::thread::sleep(std::time::Duration::from_secs(90));
}
