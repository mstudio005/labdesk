// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Syed Mutaib

use thiserror::Error;
#[derive(Debug, Error)]
pub enum Error {
    /// A rule was broken (bad input, locked report, stale version...). The message is safe to show to staff.
    #[error("{0}")]
    Rule(String),
    #[error("{0}")]
    NotFound(String),
    #[error("not allowed: {0}")]
    Forbidden(String),
    /// Too many wrong passwords in a row. The number is how many seconds are left to wait.
    #[error("Too many wrong passwords. Please wait {0} seconds and try again.")]
    Locked(u64),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("data error: {0}")]
    Json(#[from] serde_json::Error),
}
pub type Result<T> = std::result::Result<T, Error>;
/// `ensure!`-style helper: `check(cond, "msg")?`
pub fn check(cond: bool, msg: impl Into<String>) -> Result<()> {
    if cond { Ok(()) } else { Err(Error::Rule(msg.into())) }
}
