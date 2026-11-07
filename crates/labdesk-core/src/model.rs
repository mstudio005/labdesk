// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! Data model. Test definitions keep unknown JSON fields (`extra`) so catalogues round-trip losslessly.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RangeOverlay {
    #[serde(default, skip_serializing_if = "Option::is_none")] pub gender: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub age_min_days: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub age_max_days: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub low: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub high: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub critical_low: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub critical_high: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub low_exclusive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub high_exclusive: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")] pub range_text: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Parameter {
    pub id: String,
    pub name: String,
    #[serde(default = "numeric")] pub field: String,
    #[serde(default)] pub unit: String,
    #[serde(default)] pub method: String,
    #[serde(default)] pub range_text: String,
    #[serde(default)] pub low: Option<f64>,
    #[serde(default)] pub high: Option<f64>,
    #[serde(default)] pub critical_low: Option<f64>,
    #[serde(default)] pub critical_high: Option<f64>,
    #[serde(default)] pub low_exclusive: bool,
    #[serde(default)] pub high_exclusive: bool,
    #[serde(default)] pub formula: String,
    #[serde(default)] pub options: Vec<String>,
    #[serde(default)] pub required: bool,
    #[serde(default)] pub hidden: bool,
    /// Leave this line out of the differential (DLC) total.
    #[serde(default)] pub omit_dlc: bool,
    /// The lab's own short code for this line.
    #[serde(default)] pub code: String,
    #[serde(default)] pub ranges: Vec<RangeOverlay>,
    #[serde(default)] pub gender_ranges: BTreeMap<String, RangeOverlay>,
    #[serde(flatten)] pub extra: Map<String, Value>,
}
fn numeric() -> String { "Numeric".into() }

impl Parameter {
    /// Fields that hold a typed-in result (everything except section titles / rich text).
    pub fn is_result_field(&self) -> bool { !matches!(self.field.as_str(), "Title" | "Rich Text") }
    pub fn is_numeric(&self) -> bool { matches!(self.field.as_str(), "Numeric" | "Multiple ranges") }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TestConfig {
    #[serde(default)] pub parameters: Vec<Parameter>,
    #[serde(default)] pub comment: String,
    #[serde(default)] pub interpretation: String,
    #[serde(default = "both")] pub gender: String,
    #[serde(default)] pub only_billing: bool,
    #[serde(default)] pub start_next_page: bool,
    #[serde(default)] pub test_type: String,
    #[serde(default)] pub sample_color: String,
    #[serde(default)] pub barcode_suffix: String,
    #[serde(default)] pub lmp_required: bool,
    #[serde(default)] pub id_required: bool,
    #[serde(default)] pub consent_form: bool,
    #[serde(default)] pub species: bool,
    /// Text shown to the patient before the sample is taken (for example "Fasting for 10 hours").
    #[serde(default)] pub pre_collection: String,
    #[serde(default)] pub labdesk_id: String,
    #[serde(flatten)] pub extra: Map<String, Value>,
}
fn both() -> String { "Both".into() }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TestDef {
    pub id: i64,
    pub code: String,
    pub name: String,
    pub department: String,
    pub sample: String,
    pub price_paise: i64,
    pub disabled: bool,
    pub config: TestConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Patient {
    pub id: i64,
    pub patient_code: String,
    pub designation: String,
    pub first_name: String,
    pub last_name: String,
    pub age: f64,
    pub age_type: String,
    pub gender: String,
    pub phone: String,
    pub email: String,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResultRow {
    pub parameter_id: String,
    pub value: String,
    #[serde(default)] pub flag: String,
    #[serde(default)] pub range_text: Option<String>,
    #[serde(default)] pub comment: String,
    #[serde(default)] pub manual: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderTest {
    pub id: i64,
    pub test_id: i64,
    pub name: String,
    pub department: String,
    pub sample: String,
    pub status: String,
    pub config: TestConfig,
    pub results: Vec<ResultRow>,
    pub approved_at: Option<String>,
    pub approved_by_name: Option<String>,
    /// Price charged for this test on the bill, and any discount given on that line (paise).
    #[serde(default)] pub price_paise: i64,
    #[serde(default)] pub discount_paise: i64,
    /// Values and graphs sent by a blood cell counter for this test (if a machine result was attached).
    #[serde(default, skip_serializing_if = "Option::is_none")] pub machine: Option<MachineData>,
}
impl OrderTest { pub fn is_approved(&self) -> bool { self.approved_at.is_some() } }

/// One value as the analyzer sent it.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MachineItem { pub code: String, pub value: String, pub unit: String, pub range: String, pub flag: String }
/// A graph from the analyzer: heights 0-255, plus the marker lines (as positions 0-255 across the graph).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Histogram { pub name: String, pub data: Vec<u8>, pub lines: Vec<u16> }
/// Everything one analyzer message carried.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MachineData {
    pub sample_id: String, pub patient_name: String, pub sex: String, pub age_text: String, pub test_time: String,
    pub items: Vec<MachineItem>, pub histograms: Vec<Histogram>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payment { pub id: i64, pub amount_paise: i64, pub method: String, pub created_at: String, pub reference: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: i64,
    pub patient: Patient,
    pub referral_name: String,
    pub created_at: String,
    pub status: String,
    pub subtotal_paise: i64,
    pub discount_paise: i64,
    pub total_paise: i64,
    pub version: i64,
    pub notes: String,
    pub interpretation: String,
    #[serde(default)] pub bill_note: String,
    /// Set when the report was deleted (it can be restored by an Admin).
    #[serde(default)] pub deleted_at: Option<String>,
    pub tests: Vec<OrderTest>,
    pub payments: Vec<Payment>,
}
impl Order {
    pub fn paid_paise(&self) -> i64 { self.payments.iter().map(|p| p.amount_paise).sum() }
    pub fn due_paise(&self) -> i64 { self.total_paise - self.paid_paise() }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderSummary {
    pub id: i64,
    pub created_at: String,
    pub status: String,
    pub total_paise: i64,
    pub paid_paise: i64,
    pub patient_code: String,
    pub patient_name: String,
    pub age_gender: String,
    pub test_names: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role { Admin, Technician, Pathologist, Reception }
impl Role {
    pub fn as_str(self) -> &'static str {
        match self { Role::Admin => "admin", Role::Technician => "technician", Role::Pathologist => "pathologist", Role::Reception => "reception" }
    }
}
impl Role {
    pub fn parse(s: &str) -> Option<Role> {
        match s { "admin" => Some(Role::Admin), "technician" => Some(Role::Technician), "pathologist" => Some(Role::Pathologist), "reception" => Some(Role::Reception), _ => None }
    }
    /// Plain-language name for screens.
    pub fn label(self) -> &'static str {
        match self { Role::Admin => "Admin", Role::Technician => "Technician", Role::Pathologist => "Doctor", Role::Reception => "Front desk" }
    }
}
/// A person who can sign in to LabDesk.
#[derive(Debug, Clone, PartialEq)]
pub struct User { pub id: i64, pub username: String, pub name: String, pub role: Role, pub active: bool }
impl User { pub fn actor(&self) -> Actor { Actor::new(&self.name, self.role) } }
#[derive(Debug, Clone)]
pub struct Actor { pub name: String, pub role: Role }
impl Actor { pub fn new(name: &str, role: Role) -> Self { Self { name: name.into(), role } } }

#[derive(Debug, Clone, Default)]
pub struct NewPatient {
    pub designation: String, pub first_name: String, pub last_name: String,
    pub age: f64, pub age_type: String, pub gender: String,
    pub phone: String, pub email: String, pub address: String,
}
#[derive(Debug, Clone)]
pub struct OrderItem { pub test_id: i64, pub price_paise: Option<i64>, pub discount_paise: i64 }
#[derive(Debug, Clone, Default)]
pub struct NewOrder {
    pub patient: NewPatient,
    pub items: Vec<OrderItem>,
    pub referral_id: Option<i64>,
    /// A returning patient: register this visit under their existing patient ID (their name and gender are kept).
    pub existing_patient_id: Option<i64>,
    /// Name of the referring doctor. Empty means the patient came by themselves ("Self").
    /// A name not seen before is remembered and offered next time.
    pub referral_doctor: String,
    pub discount_paise: i64,
    pub discount_reason: String,
    pub bill_note: String,
    pub payments: Vec<(i64, String, String)>, // (amount_paise, method, reference)
}

/// Who sent the patient.
#[derive(Debug, Clone, PartialEq)]
pub enum Referral { SelfReferred, Doctor(String) }

/// The lab's own details, printed on reports. Margins are in millimetres and describe the blank
/// space to leave for the lab's letterhead (top/bottom) on every page.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Profile {
    pub name: String,
    pub address: String,
    pub phone: String,
    pub email: String,
    pub signatory: String,
    pub qualification: String,
    pub ending_line: String,
    pub top_mm: f32,
    pub bottom_mm: f32,
    pub left_mm: f32,
    pub right_mm: f32,
    /// Which of the eight patient-box designs reports use (0-7).
    pub header_design: u8,
    /// Size of the printed result text, in percent (70-120). 100 is the normal size.
    pub text_size: u8,
}
impl Default for Profile {
    fn default() -> Self {
        Self { name: String::new(), address: String::new(), phone: String::new(), email: String::new(), signatory: String::new(),
            qualification: String::new(), ending_line: "~~ End of report ~~".into(), top_mm: 42.0, bottom_mm: 28.0, left_mm: 15.0, right_mm: 15.0, header_design: 0, text_size: 100 }
    }
}

/// Numbers for the home screen.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    pub patients_today: i64,
    pub reports_pending: i64,
    pub ready_to_deliver: i64,
    pub due_paise: i64,
    pub collected_today_paise: i64,
}
