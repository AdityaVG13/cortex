use super::{ErasureRecord, MAX_ERASURE_LEDGER_BYTES, ledger_path};
use std::io::{Read, Write};
use std::path::Path;

pub(super) fn ledger_bytes(home: &Path) -> Result<Option<String>, String> {
    let file = match crate::auth::open_nofollow(&ledger_path(home)) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.to_string()),
    };
    let mut raw = String::new();
    file.take(MAX_ERASURE_LEDGER_BYTES + 1)
        .read_to_string(&mut raw)
        .map_err(|e| e.to_string())?;
    if raw.len() as u64 > MAX_ERASURE_LEDGER_BYTES {
        return Err("erasure_ledger_byte_limit".into());
    }
    Ok(Some(raw))
}

fn parse_ledger(raw: &str) -> Result<Vec<ErasureRecord>, String> {
    let mut out = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let rec = serde_json::from_str(line)
            .map_err(|e| format!("erasure_ledger_line_{}: {e}", idx + 1))?;
        out.push(rec);
    }
    Ok(out)
}

pub fn read_ledger(home: &Path) -> Vec<ErasureRecord> {
    match ledger_bytes(home) {
        Ok(Some(raw)) => parse_ledger(&raw).unwrap_or_default(),
        Ok(None) | Err(_) => Vec::new(),
    }
}

pub(super) fn append_ledger(home: &Path, record: &ErasureRecord) -> Result<(), String> {
    let mut f = crate::auth::open_append_nofollow(&ledger_path(home)).map_err(|e| e.to_string())?;
    writeln!(
        f,
        "{}",
        serde_json::to_string(record).map_err(|e| e.to_string())?
    )
    .map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())
}

pub(super) fn parse_or_fail(home: &Path) -> Result<Vec<ErasureRecord>, String> {
    match ledger_bytes(home) {
        Ok(Some(raw)) => parse_ledger(&raw),
        Ok(None) => Ok(Vec::new()),
        Err(err) => Err(err),
    }
}
