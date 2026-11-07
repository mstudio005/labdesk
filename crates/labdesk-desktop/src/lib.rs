// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

//! LabDesk UI glue. All lab rules live in `labdesk-core`; this crate only shows and collects data.
slint::include_modules!();
pub mod app;
pub mod bridge;
pub mod diag;
pub mod fmt;
pub mod mailer;
pub mod printing;
pub mod updater;
