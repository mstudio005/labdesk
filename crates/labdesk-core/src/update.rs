// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Update rules: which release is newer, which download is safe to use, and whether the file is intact.
//! No network code lives here. The desktop crate fetches; this module decides.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

/// "1.2.3" or "v1.2.3" (anything after a "-" such as "-dev" is ignored) -> (1, 2, 3).
pub fn parse_version(v: &str) -> Option<(u32, u32, u32)> {
    let core = v.trim().trim_start_matches(['v', 'V']).split(['-', '+']).next()?;
    let mut it = core.split('.');
    let a = it.next()?.parse().ok()?;
    let b = it.next().unwrap_or("0").parse().ok()?;
    let c = it.next().unwrap_or("0").parse().ok()?;
    if it.next().is_some() { return None; }
    Some((a, b, c))
}

/// True when `candidate` is a higher version than `current`. Anything unreadable is "not newer".
pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) { (Some(a), Some(b)) => a > b, _ => false }
}

/// A release the program may download.
#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    pub version: String,
    pub installer_url: String,
    pub installer_name: String,
    pub sha256_url: String,
    pub notes: String,
}

/// Pick the installer from GitHub's "latest release" answer. Only files that live in `repo`'s own
/// release downloads are accepted, drafts and pre-releases are skipped, and a release with no
/// checksum file next to the installer is refused. Returns None when nothing newer is available.
pub fn pick_release(json: &str, repo: &str, current: &str) -> Option<Release> {
    let v: Value = serde_json::from_str(json).ok()?;
    if v["draft"].as_bool().unwrap_or(false) || v["prerelease"].as_bool().unwrap_or(false) { return None; }
    let tag = v["tag_name"].as_str()?;
    if !is_newer(tag, current) { return None; }
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    let prefix = format!("https://github.com/{repo}/releases/download/");
    let mut installer: Option<(String, String)> = None;
    let mut sha: Option<String> = None;
    for a in v["assets"].as_array()? {
        let (Some(name), Some(url)) = (a["name"].as_str(), a["browser_download_url"].as_str()) else { continue };
        if !url.starts_with(&prefix) || name.contains(['/', '\\']) { continue; }
        if name == format!("LabDesk-Setup-{version}.exe") { installer = Some((name.to_string(), url.to_string())); }
        if name == format!("LabDesk-Setup-{version}.exe.sha256") { sha = Some(url.to_string()); }
    }
    let (installer_name, installer_url) = installer?;
    Some(Release { version, installer_url, installer_name, sha256_url: sha?, notes: v["body"].as_str().unwrap_or("").chars().take(600).collect() })
}

/// The 64 hex digits at the start of a checksum file ("<hash>  <file name>").
pub fn parse_sha256_file(text: &str) -> Option<String> {
    let h = text.split_whitespace().next()?.to_ascii_lowercase();
    (h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())).then_some(h)
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 65536];
    loop { let n = f.read(&mut buf)?; if n == 0 { break; } h.update(&buf[..n]); }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    const REPO: &str = "mstudio005/labdesk-releases";
    fn release(tag: &str, extra: &str) -> String {
        format!(r#"{{"tag_name":"{tag}","draft":false,"prerelease":false,"body":"Fixes","assets":[
          {{"name":"LabDesk-Setup-0.2.0.exe","browser_download_url":"https://github.com/{REPO}/releases/download/{tag}/LabDesk-Setup-0.2.0.exe"}},
          {{"name":"LabDesk-Setup-0.2.0.exe.sha256","browser_download_url":"https://github.com/{REPO}/releases/download/{tag}/LabDesk-Setup-0.2.0.exe.sha256"}}{extra}]}}"#)
    }
    #[test]
    fn versions_compare_as_numbers() {
        assert_eq!(parse_version("v0.10.2"), Some((0, 10, 2)));
        assert_eq!(parse_version("0.0.0-dev"), Some((0, 0, 0)));
        assert!(is_newer("0.10.0", "0.9.9") && is_newer("v1.0.0", "0.99.99"));
        assert!(!is_newer("0.2.0", "0.2.0") && !is_newer("0.1.9", "0.2.0") && !is_newer("junk", "0.1.0"));
    }
    #[test]
    fn only_a_newer_release_with_a_checksum_is_offered() {
        let r = pick_release(&release("v0.2.0", ""), REPO, "0.1.0").unwrap();
        assert_eq!(r.version, "0.2.0");
        assert!(r.installer_url.ends_with("LabDesk-Setup-0.2.0.exe") && r.sha256_url.ends_with(".sha256"));
        assert!(pick_release(&release("v0.2.0", ""), REPO, "0.2.0").is_none(), "same version is not an update");
        assert!(pick_release(&release("v0.2.0", "").replace(".sha256\"", ".txt\""), REPO, "0.1.0").is_none(), "no checksum, no update");
    }
    #[test]
    fn downloads_from_anywhere_else_are_refused() {
        let evil = release("v0.2.0", "").replace("github.com/mstudio005", "evil.example/mstudio005");
        assert!(pick_release(&evil, REPO, "0.1.0").is_none());
        let other = release("v0.2.0", "").replace("labdesk-releases", "other-repo");
        assert!(pick_release(&other, REPO, "0.1.0").is_none());
        let pre = release("v0.2.0", "").replace("\"prerelease\":false", "\"prerelease\":true");
        assert!(pick_release(&pre, REPO, "0.1.0").is_none());
        assert!(pick_release("not json", REPO, "0.1.0").is_none());
    }
    #[test]
    fn checksum_files_and_hashing() {
        let h = "a".repeat(64);
        assert_eq!(parse_sha256_file(&format!("{}  LabDesk-Setup-0.2.0.exe\n", h.to_uppercase())), Some(h));
        assert_eq!(parse_sha256_file("short"), None);
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x");
        std::fs::write(&f, b"abc").unwrap();
        assert_eq!(sha256_file(&f).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
