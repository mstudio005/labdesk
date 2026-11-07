// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Send a PDF straight to the default printer.
//!
//! On Windows the PDF is handed to SumatraPDF (a small PDF viewer that is installed next to labdesk.exe by the
//! installer) with its silent "print to the default printer" switch. If SumatraPDF is not there, Windows is asked
//! to print the file with the program that opens PDFs. If neither works the caller keeps the saved file and tells
//! the person to open it and print from there.
use std::path::Path;
use std::process::Command;

/// Print `pdf` on the default printer. Returns a short, plain message on failure.
pub fn print_pdf(pdf: &Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const NO_WINDOW: u32 = 0x0800_0000;
        let sumatra = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("SumatraPDF.exe"))).filter(|p| p.exists());
        if let Some(s) = sumatra {
            let st = Command::new(s).args(["-print-to-default", "-silent"]).arg(pdf).creation_flags(NO_WINDOW).status().map_err(|e| e.to_string())?;
            return match st.code() {
                Some(0) => Ok(()),
                Some(4) => Err("There is no default printer. Set one in Windows (Settings > Printers) and try again.".into()),
                Some(c) => Err(format!("The printer did not accept the report (code {c}).")),
                None => Err("Printing was interrupted.".into()),
            };
        }
        let cmd = format!("Start-Process -FilePath '{}' -Verb Print -WindowStyle Hidden", pdf.display().to_string().replace('\'', "''"));
        let st = Command::new("powershell").args(["-NoProfile", "-Command", &cmd]).creation_flags(NO_WINDOW).status().map_err(|e| e.to_string())?;
        if st.success() { Ok(()) } else { Err("Windows could not print the PDF by itself.".into()) }
    }
    #[cfg(not(windows))]
    {
        let st = Command::new("lp").arg(pdf).status().map_err(|e| e.to_string())?;
        if st.success() { Ok(()) } else { Err("The printer did not accept the file.".into()) }
    }
}

/// Put the file itself on the Windows clipboard, so Ctrl+V in a WhatsApp chat attaches it.
pub fn copy_file_to_clipboard(file: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let cmd = format!("Set-Clipboard -Path '{}'", file.display().to_string().replace('\'', "''"));
        Command::new("powershell").args(["-NoProfile", "-Command", &cmd]).creation_flags(0x0800_0000).status().map(|s| s.success()).unwrap_or(false)
    }
    #[cfg(not(windows))]
    { let _ = file; false }
}

/// The WhatsApp "click to chat" address for a phone number typed the Indian way. None if it cannot be a phone number.
/// A 10-digit number gets 91 in front; a leading 0 or +91 is understood.
pub fn whatsapp_url(phone: &str, text: &str) -> Option<String> {
    let digits = whatsapp_digits(phone)?;
    Some(format!("https://wa.me/{digits}?text={}", url_encode(text)))
}

/// The chat address for WhatsApp Web. `share` is the one-time code the LabDesk Bridge extension uses to fetch the report.
pub fn whatsapp_web_url(phone: &str, text: &str, share: &str) -> Option<String> {
    let digits = whatsapp_digits(phone)?;
    Some(format!("https://web.whatsapp.com/send?phone={digits}&text={}&ld={share}", url_encode(text)))
}

fn whatsapp_digits(phone: &str) -> Option<String> {
    let mut digits: String = phone.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() == 11 && digits.starts_with('0') { digits.remove(0); }
    if digits.len() == 10 { digits.insert_str(0, "91"); }
    if !(11..=15).contains(&digits.len()) { return None; }
    Some(digits)
}

/// Percent-encode text for use inside a web address or an email link.
pub fn url_encode(text: &str) -> String {
    let mut enc = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { enc.push(b as char) } else { enc.push_str(&format!("%{b:02X}")) }
    }
    enc
}

#[cfg(test)]
mod tests {
    use super::{whatsapp_url, whatsapp_web_url};
    #[test]
    fn phone_numbers_become_whatsapp_links() {
        assert_eq!(whatsapp_url("98060 12345", "Hi there").unwrap(), "https://wa.me/919806012345?text=Hi%20there");
        assert_eq!(whatsapp_url("+91-98060-12345", "").unwrap(), "https://wa.me/919806012345?text=");
        assert_eq!(whatsapp_url("09806012345", "x").unwrap(), "https://wa.me/919806012345?text=x");
        assert!(whatsapp_url("12345", "x").is_none());
        assert!(whatsapp_url("", "x").is_none());
        assert_eq!(whatsapp_web_url("98060 12345", "Hi", "abc").unwrap(), "https://web.whatsapp.com/send?phone=919806012345&text=Hi&ld=abc");
        assert!(whatsapp_url("98060 12345", "Dr. Ali's report").unwrap().contains("Ali%27s"));
    }
}
