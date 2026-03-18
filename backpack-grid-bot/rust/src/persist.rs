use std::{fs::{self, OpenOptions}, io::Write, path::Path};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;

use crate::model::{CycleReport, StateFile};

pub fn save_state(path: &Path, report: CycleReport) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(&StateFile { last_report: report })?;
    fs::write(path, body).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

pub fn append_jsonl<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(value)?)?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct EventLine<'a> {
    pub ts: String,
    pub level: &'a str,
    pub message: &'a str,
}

pub fn info(path: &Path, message: &str) -> Result<()> {
    append_jsonl(path, &EventLine { ts: Utc::now().to_rfc3339(), level: "info", message })
}
