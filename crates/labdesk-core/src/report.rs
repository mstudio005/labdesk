// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! PDF lab report.
//!
//! Layout is built in two passes so every page can say "Page 2 of 3": first the report is laid
//! out into drawing instructions per page, then those are rendered with krilla.
//!
//! The lab's own letterhead (a PNG/JPG, uploaded once in Settings) is drawn full-page behind the
//! content; LabDesk only prints what is specific to the patient: the patient box with QR code,
//! the result tables, the signature and the page footer.
use crate::calc::result_range;
use crate::error::{Error, Result};
use crate::model::*;
use krilla::color::rgb;
use krilla::geom::{PathBuilder, Point, Rect, Size, Transform};
use krilla::image::Image;
use krilla::metadata::Metadata;
use krilla::num::NormalizedF32;
use krilla::page::PageSettings;
use krilla::paint::{Fill, Stroke};
use krilla::surface::Surface;
use krilla::text::{Font, TextDirection};
use krilla::{Data, Document};
use sha2::{Digest, Sha256};

const PW: f32 = 595.28; // A4 in points
const PH: f32 = 841.89;
const MM: f32 = 2.834_645_7;
static REGULAR: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");
static BOLD: &[u8] = include_bytes!("../assets/fonts/DejaVuSans-Bold.ttf");

pub struct ReportInput<'a> {
    pub order: &'a Order,
    pub profile: &'a Profile,
    pub letterhead: Option<&'a [u8]>,
    pub signature: Option<&'a [u8]>,
    /// Draw the letterhead (or a plain text header if the lab has not uploaded one).
    /// False leaves the top/bottom margins empty, for paper that is already printed.
    pub use_letterhead: bool,
}
#[derive(Debug)]
pub struct Rendered { pub pdf: Vec<u8>, pub pages: usize, pub verification: String }

type Color = (u8, u8, u8);
const INK: Color = (20, 19, 15);
const MUTED: Color = (110, 106, 98);
const RULE: Color = (150, 146, 138);
const RED: Color = (170, 30, 30);

#[derive(Clone, Copy, PartialEq)]
enum Align { Left, Center, Right }
#[derive(Clone)]
enum Op {
    Text { x: f32, y: f32, s: String, size: f32, bold: bool, color: Color, align: Align },
    Line { x1: f32, y1: f32, x2: f32, y2: f32, w: f32, color: Color },
    Box { x: f32, y: f32, w: f32, h: f32, line: f32, color: Color },
    Qr { x: f32, y: f32, size: f32, payload: String },
    Sig { x: f32, y: f32, w: f32, h: f32 },
    /// A closed shape (the analyzer graphs): optional fill, outline colour and width.
    Curve { pts: Vec<(f32, f32)>, fill: Option<Color>, color: Color, w: f32 },
}

/// Short code printed under the QR and stored inside it. It changes if any approved value changes,
/// so a reprinted or altered report can be told apart from the original.
pub fn verification_code(o: &Order) -> String {
    let mut h = Sha256::new();
    h.update(format!("{}|{}|{}", o.patient.patient_code, o.id, o.patient.first_name).as_bytes());
    for t in o.tests.iter().filter(|t| t.is_approved()) {
        h.update(format!("|{}|{}", t.name, t.approved_at.as_deref().unwrap_or("")).as_bytes());
        let mut rows: Vec<_> = t.results.iter().collect();
        rows.sort_by(|a, b| a.parameter_id.cmp(&b.parameter_id));
        for r in rows { h.update(format!("|{}={}", r.parameter_id, r.value).as_bytes()); }
    }
    h.finalize().iter().take(5).map(|b| format!("{b:02X}")).collect()
}

/// What a phone shows after scanning the QR: plain readable text, no internet needed.
/// It identifies the report; the lab's LabDesk can check it (see `Store::verify_report_text`).
pub fn qr_text(p: &Profile, o: &Order, reported: &str, code: &str) -> String {
    let name = format!("{} {} {}", o.patient.designation, o.patient.first_name, o.patient.last_name).split_whitespace().collect::<Vec<_>>().join(" ");
    let mut parts = vec!["LabDesk report".to_string()];
    if !p.name.trim().is_empty() { parts.push(p.name.trim().to_string()); }
    parts.push(format!("Patient: {name}"));
    parts.push(format!("ID: {}", o.patient.patient_code));
    parts.push(format!("Report no: {}", o.id));
    parts.push(format!("Date: {}", ist(reported, "%d %b %Y")));
    // What a phone shows when it scans: which tests, and the results that are outside the normal range.
    // Kept short so the printed code stays easy to scan.
    let approved: Vec<&OrderTest> = o.tests.iter().filter(|t| t.is_approved()).collect();
    let tests = approved.iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ");
    let mut abnormal: Vec<String> = Vec::new();
    for t in &approved {
        for r in rows_for(t, &o.patient).iter().filter(|r| !r.title && !r.flag.is_empty()) {
            abnormal.push(format!("{} {} {} ({})", r.name, r.value, r.unit, if r.flag == "Critical" { "CRITICAL" } else { r.flag.as_str() }).split_whitespace().collect::<Vec<_>>().join(" "));
        }
    }
    let tail = format!("Ref: {code}");
    let fits = |parts: &[String]| parts.iter().map(|x| x.len() + 3).sum::<usize>() + tail.len() <= 420;
    if !tests.is_empty() {
        let mut t = format!("Tests: {tests}");
        while t.len() > 120 { t.pop(); }
        parts.push(t);
    }
    if !abnormal.is_empty() {
        let mut shown = 0;
        let mut line = String::new();
        for a in &abnormal {
            let mut trial = parts.clone();
            trial.push(format!("Abnormal: {}{}{}", line, if line.is_empty() { "" } else { "; " }, a));
            if !fits(&trial) { break; }
            if !line.is_empty() { line.push_str("; "); }
            line.push_str(a); shown += 1;
        }
        if shown > 0 { parts.push(format!("Abnormal: {line}{}", if shown < abnormal.len() { format!("; +{} more", abnormal.len() - shown) } else { String::new() })); }
    }
    parts.push(tail);
    parts.join(" | ")
}

fn ist(iso: &str, fmt: &str) -> String {
    let tz = chrono::FixedOffset::east_opt(5 * 3600 + 1800).expect("valid offset");
    chrono::DateTime::parse_from_rfc3339(iso).map(|d| d.with_timezone(&tz).format(fmt).to_string()).unwrap_or_default()
}

struct Metrics { reg: ttf_parser::Face<'static>, bold: ttf_parser::Face<'static> }
impl Metrics {
    fn new() -> Self { Self { reg: ttf_parser::Face::parse(REGULAR, 0).expect("bundled font"), bold: ttf_parser::Face::parse(BOLD, 0).expect("bundled font") } }
    fn width(&self, s: &str, size: f32, bold: bool) -> f32 {
        let f = if bold { &self.bold } else { &self.reg };
        let upem = f.units_per_em() as f32;
        s.chars().map(|c| f.glyph_index(c).and_then(|g| f.glyph_hor_advance(g)).map_or(upem * 0.5, f32::from)).sum::<f32>() * size / upem
    }
    /// Greedy word wrap; very long words are split.
    fn wrap(&self, s: &str, max: f32, size: f32, bold: bool) -> Vec<String> {
        let mut lines = Vec::new();
        for para in s.split('\n') {
            let mut cur = String::new();
            for word in para.split_whitespace() {
                let mut word = word.to_string();
                while self.width(&word, size, bold) > max && word.chars().count() > 1 {
                    let mut cut = word.chars().count() - 1;
                    while cut > 1 && self.width(&word.chars().take(cut).collect::<String>(), size, bold) > max { cut -= 1; }
                    if !cur.is_empty() { lines.push(std::mem::take(&mut cur)); }
                    lines.push(word.chars().take(cut).collect());
                    word = word.chars().skip(cut).collect();
                }
                let trial = if cur.is_empty() { word.clone() } else { format!("{cur} {word}") };
                if self.width(&trial, size, bold) <= max || cur.is_empty() { cur = trial } else { lines.push(std::mem::replace(&mut cur, word)); }
            }
            lines.push(cur);
        }
        if lines.is_empty() { lines.push(String::new()); }
        lines
    }
}

#[derive(PartialEq)]
enum Header { None, Text }

struct Layout<'a> {
    m: Metrics,
    pages: Vec<Vec<Op>>,
    y: f32,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32, // lowest y that content may reach (above the footer line)
    header: Header,
    p: &'a Profile,
}
impl<'a> Layout<'a> {
    fn cw(&self) -> f32 { self.right - self.left }
    fn ops(&mut self) -> &mut Vec<Op> { self.pages.last_mut().expect("a page exists") }
    fn text(&mut self, x: f32, y: f32, s: impl Into<String>, size: f32, bold: bool, color: Color, align: Align) {
        self.ops().push(Op::Text { x, y, s: s.into(), size, bold, color, align });
    }
    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, w: f32, color: Color) { self.ops().push(Op::Line { x1, y1, x2, y2, w, color }); }
    fn new_page(&mut self) {
        self.pages.push(Vec::new());
        self.y = self.top;
        if self.header == Header::Text { self.text_header(); }
    }
    fn need(&mut self, h: f32) -> bool {
        if self.y + h > self.bottom { self.new_page(); true } else { false }
    }
    /// Used only when the lab has not uploaded a letterhead: lab name, address and contact line.
    fn text_header(&mut self) {
        let (l, r, c) = (self.left, self.right, (self.left + self.right) / 2.0);
        let mut y = self.y + 14.0;
        let name = self.p.name.clone();
        self.text(c, y, name, 17.0, true, INK, Align::Center);
        y += 15.0;
        for line in self.m.wrap(&self.p.address.clone(), r - l, 8.5, false) {
            if line.is_empty() { continue; }
            self.text(c, y, line, 8.5, false, MUTED, Align::Center); y += 11.0;
        }
        let contact = [self.p.phone.as_str(), self.p.email.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("   |   ");
        if !contact.is_empty() { self.text(c, y, contact, 8.5, false, MUTED, Align::Center); y += 11.0; }
        y += 4.0;
        self.line(l, y, r, y, 0.8, INK);
        self.y = y + 14.0;
    }
}

struct Row { name: String, value: String, flag: String, unit: String, range: String, title: bool }

fn rows_for(t: &OrderTest, patient: &Patient) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    for p in t.config.parameters.iter().filter(|p| !p.hidden) {
        if p.field == "Rich Text" { continue; }
        if p.field == "Title" { out.push(Row { name: p.name.clone(), value: String::new(), flag: String::new(), unit: String::new(), range: String::new(), title: true }); continue; }
        let Some(r) = t.results.iter().find(|r| r.parameter_id == p.id).filter(|r| !r.value.trim().is_empty()) else { continue };
        out.push(Row { name: p.name.clone(), value: r.value.clone(), flag: r.flag.clone(), unit: p.unit.clone(), range: result_range(p, r.range_text.as_deref(), patient), title: false });
    }
    // drop section titles that have nothing under them
    let keep: Vec<bool> = (0..out.len()).map(|i| !out[i].title || out.get(i + 1).is_some_and(|n| !n.title)).collect();
    let mut k = keep.into_iter();
    out.retain(|_| k.next().unwrap_or(true));
    out
}

/// Names of the eight patient-box designs, in the order shown in Settings.
pub const HEADER_DESIGNS: [&str; 8] = ["Boxed", "Boxed, code in the middle", "Underline", "Heavy box", "Grey band", "Grey panel", "Plain", "Divider lines"];

fn rect_fill(l: &mut Layout, x: f32, y: f32, w: f32, h: f32, color: Color) {
    l.ops().push(Op::Curve { pts: vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h)], fill: Some(color), color, w: 0.1 });
}

/// The patient details box. Left: name, age/gender, referred by, phone. Right: patient id, report id,
/// collection date and report date. The QR code sits on the right (or in the middle for design 2).
fn patient_box(l: &mut Layout, design: u8, o: &Order, reported: &str, code: &str, payload: String) {
    let design = (design as usize).min(7);
    let (left, right) = (l.left, l.right);
    let cw = right - left;
    let (box_h, qr) = (88.0f32, 62.0f32);
    l.need(box_h + 10.0);
    let by = l.y;
    let grey: Color = (238, 237, 233);
    // the QR code position, and where the two text blocks start
    let mid = design == 1;
    let qx = if mid { left + cw * 0.44 } else { right - qr - 12.0 };
    let (lx, rx, rend) = if mid { (left + 12.0, qx + qr + 30.0, right - 10.0) } else { (left + 12.0, left + cw * 0.47, qx - 14.0) };
    let lend = if mid { qx - 14.0 } else { rx - 14.0 };
    match design {
        0 | 1 => l.ops().push(Op::Box { x: left, y: by, w: cw, h: box_h, line: 0.9, color: INK }),
        2 => l.line(left, by + box_h, right, by + box_h, 0.9, INK),
        3 => { l.ops().push(Op::Box { x: left, y: by, w: cw, h: box_h, line: 1.8, color: INK }); }
        4 => { rect_fill(l, left, by, cw, box_h, grey); l.line(left, by, right, by, 0.9, RULE); l.line(left, by + box_h, right, by + box_h, 0.9, RULE); }
        5 => rect_fill(l, left, by, cw, box_h, grey),
        7 => { l.line(rx - 10.0, by + 6.0, rx - 10.0, by + box_h - 6.0, 0.9, INK); l.line(qx - 10.0, by + 6.0, qx - 10.0, by + box_h - 6.0, 0.9, INK); }
        _ => {}
    }
    if mid { l.line(qx + qr + 14.0, by + 8.0, qx + qr + 14.0, by + box_h - 8.0, 0.6, RULE); }
    let name = format!("{} {} {}", o.patient.designation, o.patient.first_name, o.patient.last_name).split_whitespace().collect::<Vec<_>>().join(" ");
    let age = format!("{}/{}", o.patient.age, o.patient.gender);
    let age = if o.patient.age.fract() == 0.0 { format!("{} {}{}/{}", o.patient.age as i64, o.patient.age_type, if o.patient.age == 1.0 { "" } else { "s" }, o.patient.gender) } else { age };
    let referred = if o.referral_name.trim().is_empty() { "Self".to_string() } else { o.referral_name.clone() };
    let phone = if o.patient.phone.trim().is_empty() { "-".to_string() } else { o.patient.phone.clone() };
    let report_id = format!("R{:05}", o.id);
    let lefts = [("Name", name, true), ("Age/Gender", age, false), ("Referred By", referred, false), ("Phone No.", phone, false)];
    let rights = [("Patient ID", o.patient.patient_code.clone(), false), ("Report ID", report_id, false),
        ("Collection Date", ist(&o.created_at, "%d/%m/%Y %I:%M %p"), false), ("Report Date", ist(reported, "%d/%m/%Y %I:%M %p"), false)];
    for (x0, xend, rows) in [(lx, lend, &lefts), (rx, rend, &rights)] {
        for (i, (label, value, strong)) in rows.iter().enumerate() {
            let y = by + 20.0 + i as f32 * 17.0;
            l.text(x0, y, *label, 8.6, false, INK, Align::Left);
            l.text(x0 + 70.0, y, ":", 8.6, false, INK, Align::Left);
            let mut v = value.clone();
            let maxw = xend - x0 - 80.0;
            while l.m.width(&v, 9.0, *strong) > maxw && v.chars().count() > 2 { v.pop(); }
            l.text(x0 + 80.0, y, v, 9.0, *strong, INK, Align::Left);
        }
    }
    l.ops().push(Op::Qr { x: qx, y: by + 8.0, size: qr, payload });
    l.text(qx + qr / 2.0, by + 8.0 + qr + 10.0, format!("Ref {code}"), 6.8, false, MUTED, Align::Center);
    l.y = by + box_h + 14.0;
}

fn make_layout<'a>(input: &ReportInput<'a>) -> (Layout<'a>, f32) {
    let p = input.profile;
    let text_header = input.use_letterhead && input.letterhead.is_none() && !p.name.trim().is_empty();
    let (top, bottom_margin) = if text_header { (14.0 * MM, 14.0 * MM) } else { (p.top_mm * MM, p.bottom_mm * MM) };
    let mut l = Layout { m: Metrics::new(), pages: Vec::new(), y: 0.0, left: p.left_mm * MM, right: PW - p.right_mm * MM, top, bottom: PH - bottom_margin - 18.0,
        header: if text_header { Header::Text } else { Header::None }, p };
    l.new_page();
    (l, bottom_margin)
}

/// Build the report at the lab's chosen text size. If the result would only spill its closing lines (end of report,
/// signature) onto a second page, the text is made a little smaller until it fits on one page.
fn build(input: &ReportInput) -> Result<(Vec<Vec<Op>>, String)> {
    let base = (input.profile.text_size.clamp(70, 120) as f32) / 100.0;
    let first = build_at(input, base)?;
    if first.0.len() != 2 || first.0[1].len() > 26 { return Ok(first); }
    for step in [0.95f32, 0.90, 0.85, 0.80] {
        let k = base * step;
        if k < 0.68 { break; }
        let t = build_at(input, k)?;
        if t.0.len() == 1 { return Ok(t); }
    }
    Ok(first)
}

fn build_at(input: &ReportInput, k: f32) -> Result<(Vec<Vec<Op>>, String)> {
    let (o, p) = (input.order, input.profile);
    let (mut l, bottom_margin) = make_layout(input);
    let (left, right, cw) = (l.left, l.right, l.cw());
    let code = verification_code(o);
    let approved: Vec<&OrderTest> = o.tests.iter().filter(|t| t.is_approved()).collect();
    let reported = approved.iter().filter_map(|t| t.approved_at.as_deref()).max().unwrap_or("").to_string();

    // ---- patient box: one of eight designs chosen in Settings, details plus the QR code ----
    let payload = qr_text(p, o, &reported, &code);
    patient_box(&mut l, p.header_design, o, &reported, &code, payload);
    if approved.len() < o.tests.len() {
        let msg = "Partial report: results for the remaining tests are not yet available.";
        l.text(left, l.y + 8.0, msg, 8.5, true, RED, Align::Left);
        l.y += 22.0;
    }

    // ---- results ----
    let mut last_dept = String::new();
    for t in &approved {
        let rows = rows_for(t, &o.patient);
        if rows.is_empty() { continue; }
        // a result that came from the analyzer is printed like the analyzer's own report: table on the left, the three graphs on the right
        let graphs = t.machine.as_ref().filter(|m| !m.histograms.is_empty());
        let tr = if graphs.is_some() { left + cw * 0.64 } else { right };
        let cwt = tr - left;
        let cols = if graphs.is_some() { [0.0, 0.33 * cwt, 0.55 * cwt, 0.70 * cwt] } else { [0.0, 0.40 * cwt, 0.60 * cwt, 0.74 * cwt] };
        let dept_changed = t.department.trim().to_uppercase() != last_dept;
        // keep department + test name + column headings + first row together
        l.need(if graphs.is_some() { 340.0 * k } else if dept_changed { 92.0 * k } else { 70.0 * k });
        if dept_changed && !t.department.trim().is_empty() {
            last_dept = t.department.trim().to_uppercase();
            let y0 = l.y + 2.0;
            rect_fill(&mut l, left, y0, cw, 18.0, INK);
            l.text((left + right) / 2.0, y0 + 12.5, last_dept.clone(), 9.5, true, (255, 255, 255), Align::Center);
            l.y += 28.0;
        }
        let heading = |l: &mut Layout, cont: bool| {
            let y = l.y + 8.0;
            l.text(left, y, if cont { format!("{} (continued)", t.name) } else { t.name.clone() }, 10.5 * k, true, INK, Align::Left);
            l.y += 16.0 * k;
            let (y1, y2) = (l.y, l.y + 17.0 * k);
            rect_fill(l, left, y1, tr - left, 17.0, (238, 237, 233));
            l.line(left, y1, tr, y1, 0.6, INK);
            for (i, h) in ["TEST", "RESULT", "UNIT", "REFERENCE RANGE"].iter().enumerate() { l.text(left + cols[i] + 4.0, y1 + 11.5, *h, 7.2, true, MUTED, Align::Left); }
            l.line(left, y2, tr, y2, 0.6, INK);
            l.y = y2 + 3.0;
        };
        let graph_top = l.y + 16.0 * k;
        heading(&mut l, false);
        if let Some(m) = graphs {
            let (gx, gw, gh, gap) = (tr + 16.0, right - tr - 16.0, 92.0 * k, 22.0 * k);
            for (k, h) in m.histograms.iter().take(3).enumerate() { draw_histogram(&mut l, h, gx, graph_top + k as f32 * (gh + gap), gw, gh); }
        }
        let table_top = l.y;
        for r in &rows {
            if r.title {
                if l.need(34.0 * k) { heading(&mut l, true); }
                let y = l.y + 14.0 * k;
                l.text(left + 4.0, y, r.name.clone(), 9.5 * k, true, INK, Align::Left);
                l.y += 19.0 * k;
                continue;
            }
            let name_w = cols[1] - 10.0;
            let flagged = !r.flag.is_empty();
            let val_w = cols[2] - cols[1] - 8.0 - if flagged { 26.0 } else { 0.0 };
            let name_lines = l.m.wrap(&r.name, name_w, 9.5 * k, false);
            let val_lines = l.m.wrap(&r.value, val_w, 9.5 * k, flagged);
            let range_lines = l.m.wrap(&r.range, cwt - cols[3] - 8.0, 8.8 * k, false);
            let n = name_lines.len().max(val_lines.len()).max(range_lines.len());
            let lh = 12.0 * k;
            let h = n as f32 * lh + 6.0 * k;
            if l.need(h) { heading(&mut l, true); }
            if flagged { let ty = l.y - 1.0; rect_fill(&mut l, left, ty, tr - left, h - 0.5, if r.flag == "Critical" { (251, 228, 226) } else { (252, 241, 226) }); }
            let y0 = l.y + 12.0 * k;
            let big = graphs.is_some() && matches!(r.name.as_str(), "WBC" | "RBC" | "HGB" | "PLT");
            for (i, s) in name_lines.iter().enumerate() { l.text(left + 4.0, y0 + i as f32 * lh, s.clone(), if big { 10.5 * k } else { 9.5 * k }, big, INK, Align::Left); }
            let color = if r.flag == "Critical" { RED } else { INK };
            for (i, s) in val_lines.iter().enumerate() { l.text(left + cols[1] + 4.0, y0 + i as f32 * lh, s.clone(), 9.5 * k, flagged, color, Align::Left); }
            if flagged {
                let w = l.m.width(&val_lines[0], 9.5 * k, true);
                let tag = match r.flag.as_str() { "Critical" => "CRIT", f => f };
                l.text(left + cols[1] + 4.0 + w + 5.0, y0, tag, 7.5 * k, true, color, Align::Left);
            }
            l.text(left + cols[2] + 4.0, y0, r.unit.clone(), 9.0 * k, false, MUTED, Align::Left);
            for (i, s) in range_lines.iter().enumerate() { l.text(left + cols[3] + 4.0, y0 + i as f32 * lh, s.clone(), 8.8 * k, false, MUTED, Align::Left); }
            l.y += h;
            l.line(left, l.y - 1.5, tr, l.y - 1.5, 0.25, (225, 222, 215));
        }
        if graphs.is_some() { let end = graph_top + 3.0 * 92.0 * k + 2.0 * 22.0 * k + 10.0; if l.y < end { l.y = end; } let _ = table_top; }
        let note = t.config.comment.trim().to_string();
        if !note.is_empty() {
            for line in l.m.wrap(&format!("Note: {note}"), cw - 8.0, 8.5, false) {
                l.need(14.0);
                let y = l.y + 11.0;
                l.text(left + 4.0, y, line, 8.5, false, MUTED, Align::Left);
                l.y += 11.0;
            }
        }
        l.y += 14.0 * k;
    }
    if !o.notes.trim().is_empty() {
        let label_w = l.m.width("Remarks: ", 9.0, true);
        let lines = l.m.wrap(o.notes.trim(), cw - 8.0 - label_w, 9.0, false);
        for (i, line) in lines.into_iter().enumerate() {
            l.need(14.0);
            let y = l.y + 11.0;
            if i == 0 { l.text(left + 4.0, y, "Remarks:", 9.0, true, INK, Align::Left); }
            l.text(left + 4.0 + label_w, y, line, 9.0, false, INK, Align::Left);
            l.y += 12.0;
        }
        l.y += 8.0;
    }

    // ---- end of report + signature ----
    let sig_dims = input.signature.and_then(|b| image::ImageReader::new(std::io::Cursor::new(b)).with_guessed_format().ok()?.into_dimensions().ok());
    let (sw, sh) = sig_dims.map_or((0.0, 0.0), |(w, h)| { let k = (120.0 / w as f32).min(46.0 / h as f32); (w as f32 * k, h as f32 * k) });
    l.need(30.0 + sh.max(24.0) + 40.0);
    if !p.ending_line.trim().is_empty() {
        let y = l.y + 10.0;
        l.text((left + right) / 2.0, y, p.ending_line.clone(), 8.5, false, MUTED, Align::Center);
        l.y += 22.0;
    }
    let verified: Vec<String> = { let mut v: Vec<String> = approved.iter().filter_map(|t| t.approved_by_name.clone()).collect(); v.sort(); v.dedup(); v };
    let sy = l.y;
    if !verified.is_empty() { l.text(left, sy + 12.0, format!("Approved by: {}", verified.join(", ")), 8.0, false, MUTED, Align::Left); }
    let sig_y = sy + sh.max(26.0);
    if sh > 0.0 { l.ops().push(Op::Sig { x: right - sw, y: sy, w: sw, h: sh }); }
    if !p.signatory.trim().is_empty() { l.text(right, sig_y + 12.0, p.signatory.clone(), 10.0, true, INK, Align::Right); }
    if !p.qualification.trim().is_empty() { l.text(right, sig_y + 24.0, p.qualification.clone(), 8.5, false, MUTED, Align::Right); }
    l.y = sig_y + 30.0;

    // ---- footer on every page ----
    let total = l.pages.len();
    let fy = PH - bottom_margin - 4.0;
    let foot = format!("{} · {}", name_or(&o.patient), o.patient.patient_code);
    for (i, page) in l.pages.iter_mut().enumerate() {
        page.push(Op::Line { x1: left, y1: fy - 10.0, x2: right, y2: fy - 10.0, w: 0.4, color: (200, 196, 188) });
        page.push(Op::Text { x: left, y: fy, s: foot.clone(), size: 7.5, bold: false, color: MUTED, align: Align::Left });
        page.push(Op::Text { x: right, y: fy, s: format!("Page {} of {}", i + 1, total), size: 7.5, bold: false, color: MUTED, align: Align::Right });
    }
    Ok((l.pages, code))
}
/// The report at exactly this text size (1.0 = normal), without the automatic shrinking. Used by tests and previews.
pub fn render_at_scale(input: &ReportInput, k: f32) -> Result<Rendered> {
    let (pages, code) = build_at(input, k)?;
    render_pages(input, pages, code, format!("Report {}", input.order.patient.patient_code))
}
/// 1234567 paise -> "₹12,345.67" (Indian digit grouping; whole rupees drop the decimals).
pub fn rupees(paise: i64) -> String {
    let p = paise.unsigned_abs();
    let (r, ps) = (p / 100, p % 100);
    let digits = r.to_string();
    let grouped = if digits.len() <= 3 { digits } else {
        let (head, tail) = digits.split_at(digits.len() - 3);
        let mut parts: Vec<String> = Vec::new();
        let mut end = head.len();
        while end > 0 { let start = end.saturating_sub(2); parts.push(head[start..end].to_string()); end = start; }
        parts.reverse();
        format!("{},{}", parts.join(","), tail)
    };
    format!("{}₹{}{}", if paise < 0 { "-" } else { "" }, grouped, if ps > 0 { format!(".{ps:02}") } else { String::new() })
}

fn build_bill(input: &ReportInput) -> Result<Vec<Vec<Op>>> {
    let o = input.order;
    let (mut l, bottom_margin) = make_layout(input);
    let (left, right, cw) = (l.left, l.right, l.cw());
    let mid = (left + right) / 2.0;
    l.text(mid, l.y + 14.0, "BILL / RECEIPT", 13.0, true, INK, Align::Center);
    l.y += 28.0;

    // patient box
    let box_h = 74.0;
    let by = l.y;
    l.ops().push(Op::Box { x: left, y: by, w: cw, h: box_h, line: 0.9, color: INK });
    let name = format!("{} {} {}", o.patient.designation, o.patient.first_name, o.patient.last_name).split_whitespace().collect::<Vec<_>>().join(" ");
    let age = format!("{} {}{} / {}", o.patient.age, o.patient.age_type, if o.patient.age == 1.0 { "" } else { "s" }, o.patient.gender);
    let referred = if o.referral_name.trim().is_empty() { "Self".to_string() } else { o.referral_name.clone() };
    let col2 = left + cw * 0.58;
    let cells: [(f32, f32, &str, String, bool); 4] = [
        (left + 12.0, by + 27.0, "PATIENT NAME", name, true),
        (left + 12.0, by + 58.0, "AGE / GENDER", age, false),
        (col2, by + 27.0, "PATIENT ID", o.patient.patient_code.clone(), false),
        (col2, by + 58.0, "BILL NO. / DATE", format!("B{:05}  ·  {}", o.id, ist(&o.created_at, "%d %b %Y, %I:%M %p")), false),
    ];
    for (x, y, label, value, strong) in cells {
        l.text(x, y - 8.5, label, 6.8, true, MUTED, Align::Left);
        let maxw = if x < col2 { col2 - x - 8.0 } else { right - x - 8.0 };
        let mut v = value;
        while l.m.width(&v, 10.0, strong) > maxw && v.chars().count() > 2 { v.pop(); }
        l.text(x, y + 3.5, v, if strong { 11.0 } else { 10.0 }, strong, INK, Align::Left);
    }
    l.y = by + box_h + 8.0;
    l.text(left + 12.0, l.y + 8.0, format!("Referred by: {referred}"), 8.8, false, MUTED, Align::Left);
    l.y += 22.0;

    // line items
    let cols = [0.0, 0.07 * cw, 0.58 * cw, 0.78 * cw]; // # | test | price | discount, amount ends at the right edge
    let head = |l: &mut Layout| {
        let (y1, y2) = (l.y, l.y + 18.0);
        l.line(left, y1, right, y1, 0.6, INK);
        l.text(left + 4.0, y1 + 12.0, "#", 7.2, true, MUTED, Align::Left);
        l.text(left + cols[1], y1 + 12.0, "TEST", 7.2, true, MUTED, Align::Left);
        l.text(left + cols[3] - 8.0, y1 + 12.0, "PRICE", 7.2, true, MUTED, Align::Right);
        l.text(right - 70.0, y1 + 12.0, "DISCOUNT", 7.2, true, MUTED, Align::Right);
        l.text(right - 4.0, y1 + 12.0, "AMOUNT", 7.2, true, MUTED, Align::Right);
        l.line(left, y2, right, y2, 0.6, INK);
        l.y = y2 + 3.0;
    };
    head(&mut l);
    for (i, t) in o.tests.iter().enumerate() {
        let lines = l.m.wrap(&t.name, cols[2] - cols[1] - 10.0, 9.5, false);
        let h = lines.len() as f32 * 12.0 + 8.0;
        if l.need(h) { head(&mut l); }
        let y0 = l.y + 12.0;
        l.text(left + 4.0, y0, (i + 1).to_string(), 9.0, false, MUTED, Align::Left);
        for (k, s) in lines.iter().enumerate() { l.text(left + cols[1], y0 + k as f32 * 12.0, s.clone(), 9.5, false, INK, Align::Left); }
        l.text(left + cols[3] - 8.0, y0, rupees(t.price_paise), 9.5, false, INK, Align::Right);
        l.text(right - 70.0, y0, if t.discount_paise > 0 { rupees(t.discount_paise) } else { "-".into() }, 9.5, false, MUTED, Align::Right);
        l.text(right - 4.0, y0, rupees(t.price_paise - t.discount_paise), 9.5, false, INK, Align::Right);
        l.y += h;
        l.line(left, l.y - 2.0, right, l.y - 2.0, 0.25, (225, 222, 215));
    }

    // totals
    l.need(120.0);
    l.y += 8.0;
    let lab = right - 150.0;
    let row = |l: &mut Layout, label: &str, val: String, bold: bool, color: Color| {
        let y = l.y + 11.0;
        l.text(lab, y, label, if bold { 10.5 } else { 9.5 }, bold, color, Align::Left);
        l.text(right - 4.0, y, val, if bold { 10.5 } else { 9.5 }, bold, color, Align::Right);
        l.y += 16.0;
    };
    row(&mut l, "Subtotal", rupees(o.subtotal_paise), false, INK);
    if o.discount_paise > 0 {
        row(&mut l, "Discount", format!("-{}", rupees(o.discount_paise)), false, MUTED);
    }
    l.line(lab, l.y, right, l.y, 0.6, INK);
    l.y += 3.0;
    row(&mut l, "Total", rupees(o.total_paise), true, INK);
    row(&mut l, "Paid", rupees(o.paid_paise()), false, INK);
    let due = o.due_paise();
    row(&mut l, if due > 0 { "Balance due" } else { "Balance" }, rupees(due), true, if due > 0 { RED } else { INK });
    l.y += 6.0;

    if !o.payments.is_empty() {
        l.need(30.0 + o.payments.len() as f32 * 12.0);
        l.text(left + 4.0, l.y + 10.0, "PAYMENTS RECEIVED", 7.2, true, MUTED, Align::Left);
        l.y += 16.0;
        for p in &o.payments {
            l.need(14.0);
            let when = ist(&p.created_at, "%d %b %Y, %I:%M %p");
            let reference = if p.reference.trim().is_empty() { String::new() } else { format!("  ({})", p.reference.trim()) };
            let y = l.y + 10.0;
            l.text(left + 4.0, y, format!("{when}  ·  {}{reference}", p.method), 8.8, false, INK, Align::Left);
            l.text(right - 4.0, y, rupees(p.amount_paise), 8.8, false, INK, Align::Right);
            l.y += 13.0;
        }
        l.y += 6.0;
    }
    if !o.bill_note.trim().is_empty() {
        for line in l.m.wrap(&format!("Note: {}", o.bill_note.trim()), cw - 8.0, 8.8, false) {
            l.need(14.0);
            let y = l.y + 11.0;
            l.text(left + 4.0, y, line, 8.8, false, MUTED, Align::Left);
            l.y += 12.0;
        }
    }
    l.need(40.0);
    let y = l.y + 26.0;
    l.text(mid, y, "Thank you. This bill is not a medical report.", 8.5, false, MUTED, Align::Center);

    let total = l.pages.len();
    let fy = PH - bottom_margin - 4.0;
    let foot = format!("{} · {}", name_or(&o.patient), o.patient.patient_code);
    for (i, page) in l.pages.iter_mut().enumerate() {
        page.push(Op::Line { x1: left, y1: fy - 10.0, x2: right, y2: fy - 10.0, w: 0.4, color: (200, 196, 188) });
        page.push(Op::Text { x: left, y: fy, s: foot.clone(), size: 7.5, bold: false, color: MUTED, align: Align::Left });
        page.push(Op::Text { x: right, y: fy, s: format!("Page {} of {}", i + 1, total), size: 7.5, bold: false, color: MUTED, align: Align::Right });
    }
    Ok(l.pages)
}

/// One bill for everything a patient has had done: every visit's tests, all payments, and what is still due.
/// `orders` are that patient's (non-deleted) visits; `input.order` is only used for the patient's details.
fn build_statement(input: &ReportInput, orders: &[Order]) -> Result<Vec<Vec<Op>>> {
    let o = input.order;
    let (mut l, bottom_margin) = make_layout(input);
    let (left, right, cw) = (l.left, l.right, l.cw());
    let mid = (left + right) / 2.0;
    l.text(mid, l.y + 14.0, "COMBINED BILL", 13.0, true, INK, Align::Center);
    l.y += 28.0;
    let box_h = 74.0;
    let by = l.y;
    l.ops().push(Op::Box { x: left, y: by, w: cw, h: box_h, line: 0.9, color: INK });
    let name = format!("{} {} {}", o.patient.designation, o.patient.first_name, o.patient.last_name).split_whitespace().collect::<Vec<_>>().join(" ");
    let age = format!("{} {}{} / {}", o.patient.age, o.patient.age_type, if o.patient.age == 1.0 { "" } else { "s" }, o.patient.gender);
    let first = orders.iter().map(|x| x.created_at.as_str()).min().unwrap_or("");
    let last = orders.iter().map(|x| x.created_at.as_str()).max().unwrap_or("");
    let col2 = left + cw * 0.58;
    let cells: [(f32, f32, &str, String, bool); 4] = [
        (left + 12.0, by + 27.0, "PATIENT NAME", name, true),
        (left + 12.0, by + 58.0, "AGE / GENDER", age, false),
        (col2, by + 27.0, "PATIENT ID", o.patient.patient_code.clone(), false),
        (col2, by + 58.0, "VISITS", format!("{}  ·  {} to {}", orders.len(), ist(first, "%d %b %Y"), ist(last, "%d %b %Y")), false),
    ];
    for (x, y, label, value, strong) in cells {
        l.text(x, y - 8.5, label, 6.8, true, MUTED, Align::Left);
        let maxw = if x < col2 { col2 - x - 8.0 } else { right - x - 8.0 };
        let mut v = value;
        while l.m.width(&v, 10.0, strong) > maxw && v.chars().count() > 2 { v.pop(); }
        l.text(x, y + 3.5, v, if strong { 11.0 } else { 10.0 }, strong, INK, Align::Left);
    }
    l.y = by + box_h + 14.0;

    let c_price = right - 150.0;
    let head = |l: &mut Layout| {
        let (y1, y2) = (l.y, l.y + 18.0);
        l.line(left, y1, right, y1, 0.6, INK);
        l.text(left + 4.0, y1 + 12.0, "TEST", 7.2, true, MUTED, Align::Left);
        l.text(c_price, y1 + 12.0, "PRICE", 7.2, true, MUTED, Align::Right);
        l.text(right - 70.0, y1 + 12.0, "DISCOUNT", 7.2, true, MUTED, Align::Right);
        l.text(right - 4.0, y1 + 12.0, "AMOUNT", 7.2, true, MUTED, Align::Right);
        l.line(left, y2, right, y2, 0.6, INK);
        l.y = y2 + 3.0;
    };
    let mut ordered: Vec<&Order> = orders.iter().collect();
    ordered.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    head(&mut l);
    for v in &ordered {
        l.need(50.0);
        let y = l.y + 13.0;
        l.text(left + 4.0, y, format!("Bill B{:05}  ·  {}", v.id, ist(&v.created_at, "%d %b %Y, %I:%M %p")), 9.0, true, INK, Align::Left);
        l.y += 18.0;
        for t in &v.tests {
            let lines = l.m.wrap(&t.name, c_price - left - 40.0, 9.5, false);
            let h = lines.len() as f32 * 12.0 + 6.0;
            if l.need(h) { head(&mut l); }
            let y0 = l.y + 11.0;
            for (k, s) in lines.iter().enumerate() { l.text(left + 14.0, y0 + k as f32 * 12.0, s.clone(), 9.5, false, INK, Align::Left); }
            l.text(c_price, y0, rupees(t.price_paise), 9.5, false, INK, Align::Right);
            l.text(right - 70.0, y0, if t.discount_paise > 0 { rupees(t.discount_paise) } else { "-".into() }, 9.5, false, MUTED, Align::Right);
            l.text(right - 4.0, y0, rupees(t.price_paise - t.discount_paise), 9.5, false, INK, Align::Right);
            l.y += h;
        }
        l.need(18.0);
        let y = l.y + 11.0;
        l.text(c_price - 40.0, y, "Bill total", 8.8, false, MUTED, Align::Right);
        l.text(right - 4.0, y, rupees(v.total_paise), 9.0, true, INK, Align::Right);
        l.y += 12.0;
        l.line(left, l.y + 2.0, right, l.y + 2.0, 0.25, (225, 222, 215));
        l.y += 8.0;
    }

    let subtotal: i64 = orders.iter().map(|x| x.subtotal_paise).sum();
    let discount: i64 = orders.iter().map(|x| x.discount_paise).sum();
    let total: i64 = orders.iter().map(|x| x.total_paise).sum();
    let paid: i64 = orders.iter().map(|x| x.paid_paise()).sum();
    l.need(110.0);
    l.y += 4.0;
    let lab = right - 150.0;
    let row = |l: &mut Layout, label: &str, val: String, bold: bool, color: Color| {
        let y = l.y + 11.0;
        l.text(lab, y, label, if bold { 10.5 } else { 9.5 }, bold, color, Align::Left);
        l.text(right - 4.0, y, val, if bold { 10.5 } else { 9.5 }, bold, color, Align::Right);
        l.y += 16.0;
    };
    row(&mut l, "Subtotal", rupees(subtotal), false, INK);
    if discount > 0 { row(&mut l, "Discount", format!("-{}", rupees(discount)), false, MUTED); }
    l.line(lab, l.y, right, l.y, 0.6, INK);
    l.y += 3.0;
    row(&mut l, "Total of all bills", rupees(total), true, INK);
    row(&mut l, "Paid so far", rupees(paid), false, INK);
    let due = total - paid;
    row(&mut l, if due > 0 { "Balance due" } else { "Balance" }, rupees(due), true, if due > 0 { RED } else { INK });
    l.y += 8.0;

    let mut pays: Vec<(&Order, &Payment)> = ordered.iter().flat_map(|v| v.payments.iter().map(move |p| (*v, p))).collect();
    pays.sort_by(|a, b| a.1.created_at.cmp(&b.1.created_at));
    if !pays.is_empty() {
        l.need(30.0 + pays.len().min(3) as f32 * 14.0);
        l.text(left + 4.0, l.y + 10.0, "PAYMENTS RECEIVED", 7.2, true, MUTED, Align::Left);
        l.y += 16.0;
        for (v, p) in pays {
            l.need(14.0);
            let reference = if p.reference.trim().is_empty() { String::new() } else { format!("  ({})", p.reference.trim()) };
            let y = l.y + 10.0;
            l.text(left + 4.0, y, format!("{}  ·  {}{}  ·  towards B{:05}", ist(&p.created_at, "%d %b %Y, %I:%M %p"), p.method, reference, v.id), 8.8, false, INK, Align::Left);
            l.text(right - 4.0, y, rupees(p.amount_paise), 8.8, false, INK, Align::Right);
            l.y += 13.0;
        }
    }
    l.need(40.0);
    let y = l.y + 26.0;
    l.text(mid, y, "Thank you. This bill is not a medical report.", 8.5, false, MUTED, Align::Center);

    let total_pages = l.pages.len();
    let fy = PH - bottom_margin - 4.0;
    let foot = format!("{} · {}", name_or(&o.patient), o.patient.patient_code);
    for (i, page) in l.pages.iter_mut().enumerate() {
        page.push(Op::Line { x1: left, y1: fy - 10.0, x2: right, y2: fy - 10.0, w: 0.4, color: (200, 196, 188) });
        page.push(Op::Text { x: left, y: fy, s: foot.clone(), size: 7.5, bold: false, color: MUTED, align: Align::Left });
        page.push(Op::Text { x: right, y: fy, s: format!("Page {} of {}", i + 1, total_pages), size: 7.5, bold: false, color: MUTED, align: Align::Right });
    }
    Ok(l.pages)
}

/// One combined bill for all of a patient's visits.
pub fn render_statement(input: &ReportInput, orders: &[Order]) -> Result<Rendered> {
    let pages = build_statement(input, orders)?;
    render_pages(input, pages, String::new(), format!("Combined bill {}", input.order.patient.patient_code))
}

/// One analyzer graph the way the analyzer draws it: curve, marker lines with their numbers, and a scale underneath.
fn draw_histogram(l: &mut Layout, h: &Histogram, x: f32, y: f32, w: f32, hgt: f32) {
    let n = h.data.len().max(2);
    let base = y + hgt;
    let (axis_max, step) = crate::machine::axis(&h.name);
    let vis = crate::machine::visible_fraction(&h.name);
    let xpos = |channel: f32| x + channel / 256.0 / vis * w;
    l.text(x - 2.0, y + 6.0, h.name.clone(), 6.5, true, MUTED, Align::Right);
    // the curve, filled lightly, outline dark (only the part the analyzer draws)
    let mut pts = vec![(x, base)];
    let mut last = x;
    for (i, v) in h.data.iter().enumerate() {
        let px = x + i as f32 / n as f32 / vis * w;
        if px > x + w { break; }
        pts.push((px, base - *v as f32 / 255.0 * (hgt - 6.0)));
        last = px;
    }
    pts.push((last, base));
    l.ops().push(Op::Curve { pts, fill: Some((236, 233, 226)), color: INK, w: 0.6 });
    l.line(x, y, x, base, 0.5, RULE);
    l.line(x, base, x + w, base, 0.5, RULE);
    // marker lines with the number the analyzer writes next to them
    for c in &h.lines {
        let mx = xpos(*c as f32);
        if mx > x + w + 1.0 { continue; }
        l.line(mx, y, mx, base, 0.5, INK);
        l.text(mx, y - 2.0, crate::machine::line_label(&h.name, *c).to_string(), 5.5, false, MUTED, Align::Center);
    }
    // scale
    let mut v = 0.0;
    while v <= axis_max {
        let tx = xpos(crate::machine::channel_of(&h.name, v).max(0.0) as f32);
        if tx <= x + w + 1.0 {
            l.line(tx, base, tx, base + 2.5, 0.4, RULE);
            l.text(tx, base + 9.0, format!("{}", v as i32), 5.5, false, MUTED, Align::Center);
        }
        v += step;
    }
}
fn name_or(p: &Patient) -> String { format!("{} {}", p.first_name, p.last_name).trim().to_string() }

fn fill(c: Color) -> Fill { Fill { paint: rgb::Color::new(c.0, c.1, c.2).into(), opacity: NormalizedF32::ONE, ..Default::default() } }
fn stroke(c: Color, w: f32) -> Stroke { Stroke { paint: rgb::Color::new(c.0, c.1, c.2).into(), width: w, ..Default::default() } }
fn pdf_err(e: impl std::fmt::Debug) -> Error { Error::Rule(format!("Could not create the PDF: {e:?}")) }
fn load_image(bytes: &[u8]) -> Result<Image> {
    let data: Data = bytes.to_vec().into();
    match image::guess_format(bytes) {
        Ok(image::ImageFormat::Png) => Image::from_png(data, true),
        Ok(image::ImageFormat::Jpeg) => Image::from_jpeg(data, true),
        _ => Err("unsupported image".to_string()),
    }.map_err(|e| Error::Rule(format!("That image could not be used in the PDF ({e}). Try saving it again as a normal PNG or JPG.")))
}

pub fn render(input: &ReportInput) -> Result<Rendered> {
    let (pages, code) = build(input)?;
    render_pages(input, pages, code, format!("Report {}", input.order.patient.patient_code))
}

/// The patient's bill (receipt): tests, prices, discount, payments and what is still due.
pub fn render_bill(input: &ReportInput) -> Result<Rendered> {
    let pages = build_bill(input)?;
    render_pages(input, pages, String::new(), format!("Bill {}", input.order.patient.patient_code))
}

/// The bill for a roll printer: one long, narrow page (58 mm or 80 mm wide). The lab's name and address are printed
/// at the top only when `use_letterhead` is true; the A4 letterhead image is never used on a roll.
pub fn render_bill_thermal(input: &ReportInput, width_mm: f32) -> Result<Rendered> {
    let width = width_mm.clamp(40.0, 110.0) * MM;
    let plain = ReportInput { order: input.order, profile: input.profile, letterhead: None, signature: None, use_letterhead: input.use_letterhead };
    let (ops, height) = build_thermal(&plain, width);
    render_pages_sized(&plain, vec![ops], String::new(), format!("Bill {}", input.order.patient.patient_code), (width, height))
}

fn build_thermal(input: &ReportInput, width: f32) -> (Vec<Op>, f32) {
    let (o, p) = (input.order, input.profile);
    let edge = 8.0;
    let (left, right) = (edge, width - edge);
    let cw = right - left;
    let c = width / 2.0;
    let mut l = Layout { m: Metrics::new(), pages: vec![Vec::new()], y: edge, left, right, top: edge, bottom: f32::MAX, header: Header::None, p };
    let rule = |l: &mut Layout| { l.y += 3.0; let y = l.y; l.line(left, y, right, y, 0.5, INK); l.y += 6.0; };

    if input.use_letterhead && !p.name.trim().is_empty() {
        for s in l.m.wrap(p.name.trim(), cw, 11.0, true) { l.y += 12.0; let y = l.y; l.text(c, y, s, 11.0, true, INK, Align::Center); }
        for s in l.m.wrap(p.address.trim(), cw, 7.2, false) { if s.is_empty() { continue; } l.y += 9.0; let y = l.y; l.text(c, y, s, 7.2, false, MUTED, Align::Center); }
        let contact = [p.phone.as_str(), p.email.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join("  ");
        for s in l.m.wrap(&contact, cw, 7.2, false) { if s.is_empty() { continue; } l.y += 9.0; let y = l.y; l.text(c, y, s, 7.2, false, MUTED, Align::Center); }
        rule(&mut l);
    }
    l.y += 10.0; let y = l.y; l.text(c, y, "BILL / RECEIPT", 9.0, true, INK, Align::Center);
    rule(&mut l);

    let label_w = (cw * 0.34).max(34.0);
    let name = format!("{} {} {}", o.patient.designation, o.patient.first_name, o.patient.last_name).split_whitespace().collect::<Vec<_>>().join(" ");
    let age = format!("{} {}{} / {}", o.patient.age, o.patient.age_type, if o.patient.age == 1.0 { "" } else { "s" }, o.patient.gender);
    let referred = if o.referral_name.trim().is_empty() { "Self".to_string() } else { o.referral_name.clone() };
    let rows: [(&str, String, bool); 6] = [
        ("Bill no.", format!("B{:05}", o.id), false),
        ("Date", ist(&o.created_at, "%d %b %Y, %I:%M %p"), false),
        ("Patient", name, true),
        ("Patient ID", o.patient.patient_code.clone(), false),
        ("Age / Sex", age, false),
        ("Referred by", referred, false),
    ];
    for (label, value, strong) in rows {
        let lines = l.m.wrap(&value, cw - label_w, 7.8, strong);
        l.y += 9.5;
        let y = l.y;
        l.text(left, y, label, 7.0, false, MUTED, Align::Left);
        for (k, s) in lines.iter().enumerate() {
            if k > 0 { l.y += 9.5; }
            let y = l.y;
            l.text(left + label_w, y, s.clone(), 7.8, strong, INK, Align::Left);
        }
    }
    rule(&mut l);

    l.y += 8.0; let y = l.y;
    l.text(left, y, "TEST", 6.8, true, MUTED, Align::Left);
    l.text(right, y, "AMOUNT", 6.8, true, MUTED, Align::Right);
    l.y += 3.0; let y = l.y; l.line(left, y, right, y, 0.3, RULE);
    let amount_w = 52.0;
    for t in &o.tests {
        let lines = l.m.wrap(&t.name, cw - amount_w, 8.0, false);
        l.y += 10.5;
        let y = l.y;
        l.text(right, y, rupees(t.price_paise), 8.0, false, INK, Align::Right);
        for (k, s) in lines.iter().enumerate() {
            if k > 0 { l.y += 10.0; }
            let y = l.y;
            l.text(left, y, s.clone(), 8.0, false, INK, Align::Left);
        }
        if t.discount_paise > 0 {
            l.y += 9.0; let y = l.y;
            l.text(left + 6.0, y, "discount", 6.8, false, MUTED, Align::Left);
            l.text(right, y, format!("-{}", rupees(t.discount_paise)), 6.8, false, MUTED, Align::Right);
        }
    }
    rule(&mut l);

    let row = |l: &mut Layout, label: &str, val: String, bold: bool| {
        l.y += 11.0;
        let y = l.y;
        l.text(left, y, label, if bold { 9.0 } else { 8.0 }, bold, INK, Align::Left);
        l.text(right, y, val, if bold { 9.0 } else { 8.0 }, bold, INK, Align::Right);
    };
    row(&mut l, "Subtotal", rupees(o.subtotal_paise), false);
    if o.discount_paise > 0 { row(&mut l, "Discount", format!("-{}", rupees(o.discount_paise)), false); }
    row(&mut l, "Total", rupees(o.total_paise), true);
    row(&mut l, "Paid", rupees(o.paid_paise()), false);
    let due = o.due_paise();
    row(&mut l, if due > 0 { "Balance due" } else { "Balance" }, rupees(due), true);

    if !o.payments.is_empty() {
        rule(&mut l);
        l.y += 8.0; let y = l.y;
        l.text(left, y, "PAYMENTS", 6.8, true, MUTED, Align::Left);
        for pay in &o.payments {
            l.y += 10.0; let y = l.y;
            l.text(left, y, format!("{}  {}", ist(&pay.created_at, "%d %b %y"), pay.method), 7.4, false, INK, Align::Left);
            l.text(right, y, rupees(pay.amount_paise), 7.4, false, INK, Align::Right);
        }
    }
    if !o.bill_note.trim().is_empty() {
        l.y += 4.0;
        for s in l.m.wrap(&format!("Note: {}", o.bill_note.trim()), cw, 7.2, false) { l.y += 9.0; let y = l.y; l.text(left, y, s, 7.2, false, MUTED, Align::Left); }
    }
    rule(&mut l);
    l.y += 8.0; let y = l.y; l.text(c, y, "Thank you.", 7.6, false, INK, Align::Center);
    l.y += 9.0; let y = l.y; l.text(c, y, "This bill is not a medical report.", 6.8, false, MUTED, Align::Center);
    let height = l.y + edge + 6.0;
    (l.pages.pop().unwrap_or_default(), height.max(120.0))
}

fn render_pages(input: &ReportInput, pages: Vec<Vec<Op>>, code: String, title: String) -> Result<Rendered> {
    render_pages_sized(input, pages, code, title, (PW, PH))
}

fn render_pages_sized(input: &ReportInput, pages: Vec<Vec<Op>>, code: String, title: String, size: (f32, f32)) -> Result<Rendered> {
    let reg = Font::new(Data::from(REGULAR), 0).ok_or_else(|| pdf_err("font"))?;
    let bold = Font::new(Data::from(BOLD), 0).ok_or_else(|| pdf_err("font"))?;
    let metrics = Metrics::new();
    let letterhead = match (input.use_letterhead, input.letterhead) { (true, Some(b)) => Some((load_image(b)?, image::ImageReader::new(std::io::Cursor::new(b)).with_guessed_format().ok().and_then(|r| r.into_dimensions().ok()).unwrap_or((1, 1)))), _ => None };
    let signature = match input.signature { Some(b) => Some(load_image(b)?), None => None };

    let mut doc = Document::new();
    doc.set_metadata(Metadata::new().title(title).creator("LabDesk".to_string()));
    for ops in &pages {
        let mut page = doc.start_page_with(PageSettings::from_wh(size.0, size.1).ok_or_else(|| pdf_err("page size"))?);
        let mut s = page.surface();
        if let Some((img, (iw, ih))) = &letterhead { draw_cover(&mut s, img.clone(), *iw as f32, *ih as f32)?; }
        for op in ops {
            match op {
                Op::Sig { x, y, w, h } => {
                    if let (Some(img), Some(sz)) = (&signature, Size::from_wh(*w, *h)) {
                        s.push_transform(&Transform::from_translate(*x, *y));
                        s.draw_image(img.clone(), sz);
                        s.pop();
                    }
                }
                Op::Text { x, y, s: txt, size, bold: b, color, align } => {
                    if txt.is_empty() { continue; }
                    let w = metrics.width(txt, *size, *b);
                    let x0 = match align { Align::Left => *x, Align::Center => x - w / 2.0, Align::Right => x - w };
                    s.set_fill(Some(fill(*color)));
                    s.set_stroke(None);
                    s.draw_text(Point::from_xy(x0, *y), if *b { bold.clone() } else { reg.clone() }, *size, txt, false, TextDirection::Auto);
                }
                Op::Line { x1, y1, x2, y2, w, color } => {
                    let mut pb = PathBuilder::new();
                    pb.move_to(*x1, *y1); pb.line_to(*x2, *y2);
                    if let Some(path) = pb.finish() { s.set_fill(None); s.set_stroke(Some(stroke(*color, *w))); s.draw_path(&path); }
                }
                Op::Box { x, y, w, h, line, color } => {
                    if let Some(r) = Rect::from_xywh(*x, *y, *w, *h) {
                        let mut pb = PathBuilder::new(); pb.push_rect(r);
                        if let Some(path) = pb.finish() { s.set_fill(Some(fill((255, 255, 255)))); s.set_stroke(Some(stroke(*color, *line))); s.draw_path(&path); }
                    }
                }
                Op::Curve { pts, fill: fc, color, w } => {
                    let mut pb = PathBuilder::new();
                    for (i, (px, py)) in pts.iter().enumerate() { if i == 0 { pb.move_to(*px, *py); } else { pb.line_to(*px, *py); } }
                    pb.close();
                    if let Some(path) = pb.finish() {
                        s.set_fill(fc.map(fill)); s.set_stroke(Some(stroke(*color, *w))); s.draw_path(&path);
                    }
                }
                Op::Qr { x, y, size, payload } => draw_qr(&mut s, *x, *y, *size, payload)?,
            }
        }
        s.finish();
        page.finish();
    }
    let n = pages.len();
    Ok(Rendered { pdf: doc.finish().map_err(pdf_err)?, pages: n, verification: code })
}

/// Scale the letterhead to cover the whole page (centred, cropped), like a CSS `background-size: cover`.
fn draw_cover(s: &mut Surface, img: Image, iw: f32, ih: f32) -> Result<()> {
    let k = (PW / iw).max(PH / ih);
    let (dw, dh) = (iw * k, ih * k);
    let page = Rect::from_xywh(0.0, 0.0, PW, PH).ok_or_else(|| pdf_err("page"))?;
    let mut pb = PathBuilder::new(); pb.push_rect(page);
    let clip = pb.finish().ok_or_else(|| pdf_err("clip"))?;
    s.push_clip_path(&clip, &krilla::paint::FillRule::NonZero);
    s.push_transform(&Transform::from_translate((PW - dw) / 2.0, (PH - dh) / 2.0));
    s.draw_image(img, Size::from_wh(dw, dh).ok_or_else(|| pdf_err("image size"))?);
    s.pop();
    s.pop();
    Ok(())
}

fn draw_qr(s: &mut Surface, x: f32, y: f32, size: f32, payload: &str) -> Result<()> {
    let code = qrcode::QrCode::with_error_correction_level(payload.as_bytes(), qrcode::EcLevel::L).map_err(pdf_err)?;
    let n = code.width();
    let m = size / n as f32;
    let colors = code.to_colors();
    let mut pb = PathBuilder::new();
    for row in 0..n {
        let mut col = 0;
        while col < n {
            if colors[row * n + col] == qrcode::Color::Dark {
                let start = col;
                while col < n && colors[row * n + col] == qrcode::Color::Dark { col += 1; }
                if let Some(r) = Rect::from_xywh(x + start as f32 * m, y + row as f32 * m, (col - start) as f32 * m + 0.02, m + 0.02) { pb.push_rect(r); }
            } else { col += 1; }
        }
    }
    if let Some(path) = pb.finish() { s.set_fill(Some(fill(INK))); s.set_stroke(None); s.draw_path(&path); }
    Ok(())
}

/// Made-up order used for the "print test page" in Settings.
pub fn sample_order() -> Order {
    let param = |id: &str, name: &str, unit: &str, low: Option<f64>, high: Option<f64>| Parameter { id: id.into(), name: name.into(), unit: unit.into(), low, high, ..Default::default() };
    let res = |id: &str, v: &str, flag: &str| ResultRow { parameter_id: id.into(), value: v.into(), flag: flag.into(), range_text: None, comment: String::new(), manual: false };
    let cbc = TestConfig { parameters: vec![
        param("hb", "Haemoglobin", "g/dL", Some(13.0), Some(17.0)), param("rbc", "RBC count", "million/µL", Some(4.5), Some(5.9)),
        param("wbc", "Total WBC count", "/µL", Some(4000.0), Some(11000.0)), param("plt", "Platelet count", "/µL", Some(150000.0), Some(450000.0))], ..Default::default() };
    let lip = TestConfig { parameters: vec![param("tc", "Total cholesterol", "mg/dL", None, Some(200.0)), param("hdl", "HDL cholesterol", "mg/dL", Some(40.0), None)], ..Default::default() };
    let at = Some("2026-10-13T09:30:00.000Z".to_string());
    let ot = |id, name: &str, dept: &str, config, results| OrderTest { id, test_id: id, name: name.into(), department: dept.into(), sample: String::new(), status: "Completed".into(), config, results, approved_at: at.clone(), approved_by_name: Some("Dr. Sample Pathologist".into()), price_paise: 0, discount_paise: 0, machine: None };
    Order {
        id: 1,
        patient: Patient { id: 1, patient_code: "261013001".into(), designation: "MR.".into(), first_name: "Sample".into(), last_name: "Patient".into(), age: 42.0, age_type: "Year".into(), gender: "Male".into(), phone: "".into(), email: "".into(), address: "".into() },
        referral_name: "Dr. A. Referrer".into(), created_at: "2026-10-13T05:10:00.000Z".into(), status: "Completed".into(),
        subtotal_paise: 0, discount_paise: 0, total_paise: 0, version: 1, bill_note: String::new(), deleted_at: None, notes: "This is a test page. It shows where your letterhead, the patient box and the results will sit.".into(), interpretation: String::new(),
        tests: vec![
            ot(1, "Complete Blood Count", "HAEMATOLOGY", cbc, vec![res("hb", "12.1", "L"), res("rbc", "4.9", ""), res("wbc", "12400", "H"), res("plt", "14000", "Critical")]),
            ot(2, "Lipid Profile", "BIOCHEMISTRY", lip, vec![res("tc", "214", "H"), res("hdl", "46", "")]),
        ],
        payments: vec![],
    }
}

/// A sample report for one test, with made-up values, so a lab can see how the test will print
/// before using it. Numbers are placed inside the normal range, and one is pushed out of it when
/// the test has a range, to show how abnormal results look.
pub fn preview_order(def: &TestDef) -> Order {
    let mut o = sample_order();
    let mut results = Vec::new();
    for (i, p) in def.config.parameters.iter().enumerate() {
        if !p.formula.is_empty() { continue; } // worked out from the other values below
        let r = crate::calc::range_for(p, &o.patient);
        let value = match p.field.as_str() {
            "Title" | "Rich Text" => continue,
            "Text" => "Sample result".to_string(),
            "Dropdown" => p.options.first().cloned().unwrap_or_default(),
            _ => match (r.low, r.high) {
                (Some(l), Some(h)) => { let v = if i == 1 { h + (h - l) * 0.2 } else { (l + h) / 2.0 }; format!("{}", (v * 100.0).round() / 100.0) }
                (Some(l), None) => format!("{}", l + 1.0),
                (None, Some(h)) => format!("{}", (h / 2.0 * 10.0).round() / 10.0),
                _ => "1".to_string(),
            },
        };
        let flag = crate::calc::flag_for(p, &value, &o.patient).to_string();
        results.push(ResultRow { parameter_id: p.id.clone(), value, flag, range_text: None, comment: String::new(), manual: false });
    }
    let calc = crate::calc::calculate_results(&def.config.parameters, &results);
    let mut t = o.tests.remove(0);
    t.name = def.name.clone(); t.department = def.department.clone(); t.config = def.config.clone(); t.results = calc;
    for r in t.results.iter_mut() {
        if let Some(p) = t.config.parameters.iter().find(|p| p.id == r.parameter_id) { r.flag = crate::calc::flag_for(p, &r.value, &o.patient).to_string(); }
    }
    o.tests = vec![t];
    o.notes = "PREVIEW: values on this page are made up to show the layout.".into();
    o
}

/// Plain text from the HTML kept in imported comments and interpretations.
pub fn strip_html(h: &str) -> String {
    let (mut out, mut tag) = (String::new(), String::new());
    let mut in_tag = false;
    for c in h.chars() {
        match c {
            '<' => { in_tag = true; tag.clear(); }
            '>' if in_tag => {
                in_tag = false;
                let t = tag.trim().trim_start_matches('/').to_lowercase();
                let name = t.split(|c: char| !c.is_alphanumeric()).next().unwrap_or("");
                match name { "br" | "p" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "table" => { if !out.ends_with('\n') && !out.is_empty() { out.push('\n'); } } "td" | "th" => { if !out.ends_with('\n') && !out.is_empty() && !out.ends_with(" | ") { out.push_str(" | "); } } _ => {} }
            }
            _ if in_tag => tag.push(c),
            _ => out.push(c),
        }
    }
    let s = out.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'");
    s.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ").trim_end_matches('|').trim().to_string()).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
}
