// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Result calculations, reference ranges and flags.
//! Ported from the web app's `calculations.mjs`; behaviour is kept identical (including
//! JS-style rounding and `-2^2 == 4`) so results match during the parallel-run period.
use crate::model::{Parameter, Patient, RangeOverlay, ResultRow};
use std::collections::HashMap;

#[derive(Debug, PartialEq)]
pub struct CalcError(pub &'static str);

fn js_round(x: f64) -> f64 { (x + 0.5).floor() } // Math.round: ties go toward +infinity

#[derive(Debug, Clone, PartialEq)]
enum Tok { Num(f64), Ident(String), Op(char) }

fn tokenize(s: &str) -> Result<Vec<Tok>, CalcError> {
    let b: Vec<char> = s.chars().collect();
    let (mut i, mut out) = (0, Vec::new());
    while i < b.len() {
        let c = b[i];
        if c.is_whitespace() { i += 1; continue; }
        if c.is_ascii_digit() || (c == '.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let st = i;
            while i < b.len() && b[i].is_ascii_digit() { i += 1; }
            if i < b.len() && b[i] == '.' { i += 1; while i < b.len() && b[i].is_ascii_digit() { i += 1; } }
            if i < b.len() && (b[i] == 'e' || b[i] == 'E') {
                let mut j = i + 1;
                if j < b.len() && (b[j] == '+' || b[j] == '-') { j += 1; }
                if j < b.len() && b[j].is_ascii_digit() { while j < b.len() && b[j].is_ascii_digit() { j += 1; } i = j; }
            }
            let t: String = b[st..i].iter().collect();
            out.push(Tok::Num(t.parse().map_err(|_| CalcError("Unsupported formula"))?));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let st = i;
            while i < b.len() && (b[i].is_ascii_alphabetic() || b[i] == '_') { i += 1; }
            out.push(Tok::Ident(b[st..i].iter().collect()));
        } else if "()+-*/^%,".contains(c) {
            out.push(Tok::Op(c)); i += 1;
        } else { return Err(CalcError("Unsupported formula")); }
        if out.len() > 500 { return Err(CalcError("Unsupported formula")); }
    }
    Ok(out)
}

struct Parser { t: Vec<Tok>, i: usize }
impl Parser {
    fn next(&mut self) -> Option<Tok> { let t = self.t.get(self.i).cloned(); self.i += 1; t }
    fn peek_op(&self) -> Option<char> { if let Some(Tok::Op(c)) = self.t.get(self.i) { Some(*c) } else { None } }
    fn primary(&mut self) -> Result<f64, CalcError> {
        match self.next() {
            Some(Tok::Op('+')) => self.primary(),
            Some(Tok::Op('-')) => Ok(-self.primary()?),
            Some(Tok::Op('(')) => { let n = self.sum()?; if self.next() != Some(Tok::Op(')')) { return Err(CalcError("Unclosed formula")); } Ok(n) }
            Some(Tok::Num(n)) => Ok(n),
            Some(Tok::Ident(name)) => {
                if self.next() != Some(Tok::Op('(')) { return Err(CalcError("Unsupported formula function")); }
                let mut args = vec![self.sum()?];
                while self.peek_op() == Some(',') { self.i += 1; args.push(self.sum()?); }
                if self.next() != Some(Tok::Op(')')) { return Err(CalcError("Unclosed formula")); }
                let a = args[0];
                Ok(match name.to_ascii_lowercase().as_str() {
                    "abs" => a.abs(), "sqrt" => a.sqrt(), "log" => a.log10(), "ln" => a.ln(), "exp" => a.exp(),
                    "pow" => a.powf(args.get(1).copied().unwrap_or(f64::NAN)),
                    "min" => args.iter().copied().fold(f64::INFINITY, f64::min),
                    "max" => args.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    "round" => { let d = args.get(1).copied().unwrap_or(0.0); js_round(a * 10f64.powf(d)) / 10f64.powf(d) }
                    _ => return Err(CalcError("Unsupported formula function")),
                })
            }
            _ => Err(CalcError("Unsupported formula")),
        }
    }
    fn power(&mut self) -> Result<f64, CalcError> {
        let n = self.primary()?;
        if self.peek_op() == Some('^') { self.i += 1; return Ok(n.powf(self.power()?)); }
        Ok(n)
    }
    fn product(&mut self) -> Result<f64, CalcError> {
        let mut n = self.power()?;
        while let Some(op @ ('*' | '/' | '%')) = self.peek_op() {
            self.i += 1; let r = self.power()?;
            n = match op { '*' => n * r, '/' => n / r, _ => n % r };
        }
        Ok(n)
    }
    fn sum(&mut self) -> Result<f64, CalcError> {
        let mut n = self.product()?;
        while let Some(op @ ('+' | '-')) = self.peek_op() {
            self.i += 1; let r = self.product()?;
            n = if op == '+' { n + r } else { n - r };
        }
        Ok(n)
    }
}

/// Evaluate `<Name>`-style formulas such as `<Hb> / <RBC> * 10`. Arithmetic only — never executes code.
pub fn calculate(expr: &str, values: &HashMap<String, String>) -> Result<String, CalcError> {
    let mut input = String::new();
    let mut rest = expr;
    while let Some(s) = rest.find('<') {
        input.push_str(&rest[..s]);
        let Some(e) = rest[s + 1..].find('>') else { input.push_str(&rest[s..]); rest = ""; break };
        let name = rest[s + 1..s + 1 + e].trim();
        let v = values.get(name).or_else(|| values.get(&name.replace('_', " ")));
        let n = v.and_then(|v| v.trim().parse::<f64>().ok()).filter(|n| n.is_finite())
            .ok_or(CalcError("Missing formula input"))?;
        input.push_str(&format!("({n})"));
        rest = &rest[s + 1 + e + 1..];
    }
    input.push_str(rest);
    let mut p = Parser { t: tokenize(&input)?, i: 0 };
    let r = p.sum()?;
    if p.i != p.t.len() || !r.is_finite() { return Err(CalcError("Formula has no finite result")); }
    Ok(format!("{}", js_round((r + f64::EPSILON) * 100.0) / 100.0))
}

fn age_days(p: &Patient) -> f64 {
    p.age * match p.age_type.as_str() { "Year" => 365.25, "Month" => 30.4375, _ => 1.0 }
}

/// The parameter with the patient's matching age/sex range applied on top.
pub fn range_for(p: &Parameter, patient: &Patient) -> Parameter {
    let days = age_days(patient);
    let mut out = p.clone();
    let apply = |out: &mut Parameter, o: &RangeOverlay| {
        if let Some(v) = o.low { out.low = Some(v) }
        if let Some(v) = o.high { out.high = Some(v) }
        if let Some(v) = o.critical_low { out.critical_low = Some(v) }
        if let Some(v) = o.critical_high { out.critical_high = Some(v) }
        if let Some(v) = o.low_exclusive { out.low_exclusive = v }
        if let Some(v) = o.high_exclusive { out.high_exclusive = v }
        if let Some(v) = &o.range_text { out.range_text = v.clone() }
    };
    let sex = match patient.gender.as_str() { "Male" | "Female" => Some(patient.gender.as_str()), _ if days < 6574.5 => Some("Child"), _ => None };
    if let Some(o) = sex.and_then(|s| p.gender_ranges.get(s)) { apply(&mut out, o); }
    let hit = p.ranges.iter().find(|r| {
        r.gender.as_deref().is_none_or(|g| g.is_empty() || g == "Both" || g == patient.gender)
            && days >= r.age_min_days.unwrap_or(0.0) && days <= r.age_max_days.unwrap_or(f64::INFINITY)
    });
    if let Some(r) = hit { apply(&mut out, r); }
    out
}

/// "" (normal / not applicable), "L", "H" or "Critical".
pub fn flag_for(p: &Parameter, value: &str, patient: &Patient) -> &'static str {
    let p = range_for(p, patient);
    if value.is_empty() || !p.is_numeric() { return ""; }
    let Ok(n) = value.trim().parse::<f64>() else { return "" };
    if !n.is_finite() { return ""; }
    if p.critical_low.is_some_and(|c| n < c) || p.critical_high.is_some_and(|c| n > c) { return "Critical"; }
    if p.low.is_some_and(|l| n < l || (p.low_exclusive && n == l)) { return "L"; }
    if p.high.is_some_and(|h| n > h || (p.high_exclusive && n == h)) { return "H"; }
    ""
}

/// Imported ranges often start with the group they are for ("Male: 50 - 500"). The report already says
/// the patient's gender, so the label is dropped: only the range is printed.
pub fn strip_gender_label(text: &str) -> String {
    let t = text.trim();
    for label in ["Female", "Male", "Children", "Child", "Both", "Other"] {
        if t.len() > label.len() && t.is_char_boundary(label.len()) && t[..label.len()].eq_ignore_ascii_case(label) {
            if let Some(rest) = t[label.len()..].trim_start().strip_prefix(':') { return rest.trim_start().to_string(); }
        }
    }
    t.to_string()
}

/// Printable reference range for this patient, e.g. "13 - 17", "≥ 5", "< 200".
pub fn printable_range(p: &Parameter, patient: &Patient) -> String {
    let s = range_for(p, patient);
    if !s.range_text.is_empty() { return strip_gender_label(&s.range_text); }
    match (s.low, s.high) {
        (Some(l), Some(h)) => format!("{l} - {h}"),
        (Some(l), None) => format!("{} {l}", if s.low_exclusive { ">" } else { "≥" }),
        (None, Some(h)) => format!("{} {h}", if s.high_exclusive { "<" } else { "≤" }),
        _ => String::new(),
    }
}

/// A saved per-result override wins only if non-blank.
pub fn result_range(p: &Parameter, saved: Option<&str>, patient: &Patient) -> String {
    match saved { Some(s) if !s.trim().is_empty() => s.to_string(), _ => printable_range(p, patient) }
}

/// Recompute formula parameters (that were not manually overridden) from the entered values.
/// Dependent formulas resolve in repeated passes; unresolved ones end up blank.
pub fn calculate_results(params: &[Parameter], results: &[ResultRow]) -> Vec<ResultRow> {
    let mut next: Vec<ResultRow> = results.to_vec();
    let aliases = |p: &Parameter| [p.id.clone(), p.name.clone(), p.name.split_whitespace().collect::<Vec<_>>().join("_")];
    let mut values: HashMap<String, String> = HashMap::new();
    for p in params {
        let v = next.iter().find(|r| r.parameter_id == p.id).map(|r| r.value.clone()).unwrap_or_default();
        for k in aliases(p) { values.insert(k, v.clone()); }
    }
    let formulas: Vec<&Parameter> = params.iter()
        .filter(|p| !p.formula.is_empty() && !next.iter().any(|r| r.parameter_id == p.id && r.manual)).collect();
    for p in &formulas { for k in aliases(p) { values.remove(&k); } }
    let mut pending: Vec<&Parameter> = formulas.clone();
    for _ in 0..formulas.len() {
        let mut progress = false;
        pending.retain(|p| match calculate(&p.formula, &values) {
            Ok(v) => { for k in aliases(p) { values.insert(k, v.clone()); } progress = true; false }
            Err(_) => true,
        });
        if !progress { break; }
    }
    for p in formulas {
        let value = values.get(&p.id).cloned().unwrap_or_default();
        match next.iter_mut().find(|r| r.parameter_id == p.id) {
            Some(r) => { r.value = value; r.manual = false }
            None => next.push(ResultRow { parameter_id: p.id.clone(), value, flag: String::new(), range_text: None, comment: String::new(), manual: false }),
        }
    }
    next
}
