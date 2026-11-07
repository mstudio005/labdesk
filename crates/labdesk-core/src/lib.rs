// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! LabDesk core: data model, calculations and the SQLite-backed store. No UI code here.
pub mod analytics;
pub mod backup;
pub mod calc;
pub mod capture;
pub mod email;
pub mod error;
pub mod machine;
pub mod model;
pub mod patients;
pub mod report;
pub mod store;
pub mod summary;
pub mod update;
pub mod users;
pub use email::{preset as email_preset, valid_address, EmailSettings};
pub use analytics::{Analytics, Bucket, DeptLine, Totals};
pub use backup::{backup_is_locked, is_encrypted_backup, list_backups, BackupInfo, BackupSettings};
pub use error::{Error, Result};
pub use machine::{MachineCandidate, MachineEntry};
pub use model::*;
pub use patients::{Trend, TrendPoint, PatientHistory, PaymentEntry, PatientSummary, TestCount, Verification, Visit, VisitTest};
pub use summary::{ReferralLine, Summary};
pub use store::{check_email, DeletedOrder, ResultInput, Store};
pub use store::{BillPrefs, TERMS_VERSION};
