//! Explicit registered-file inventory and bounded, restartable exact base capture.
//!
//! A revision is a metadata snapshot, NOT a global filesystem snapshot. Completion
//! describes exact capture only, never projection, delivery, or invocation presence.
//! Blocked entries retain the cursor. A changed source requires a new inventory.
//! Capture commits before progress: a crash between them safely retries observe_file.
use super::{CortexRuntime, observation::ObservationReceipt};
use asupersync::Cx;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;

mod progress;

pub const MAX_INVENTORY_SOURCES: usize = 4096;
pub const MAX_BOOTSTRAP_SOURCES: usize = 128;
pub const MAX_BOOTSTRAP_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryStatus {
    Ready,
    PartiallyIndexed,
    SourceUnavailable,
    SourceChanged,
    PermissionRequired,
    QuotaBlocked,
    UnknownFormat,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InventoryEntry {
    pub source_key: String,
    /// Extension hint only; no semantic parser is claimed. Intake is exact UTF-8.
    pub format: String,
    pub bytes: Option<u64>,
    pub source_revision: Option<String>,
    pub status: InventoryStatus,
    pub detail: Option<String>,
    pub receipt: Option<ObservationReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInventory {
    pub revision: String,
    pub restore_epoch: String,
    /// Contiguous captured prefix within this immutable enumeration only.
    pub next_index: usize,
    pub status: InventoryStatus,
    pub entries: Vec<InventoryEntry>,
    /// Registrations that appeared after this revision was sealed.
    #[serde(default)]
    pub untracked_registrations: usize,
}

pub(super) fn ensure(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS observation_inventories (principal TEXT NOT NULL, revision TEXT NOT NULL, inventory_json TEXT NOT NULL CHECK(json_valid(inventory_json)), PRIMARY KEY(principal,revision));").map_err(|e| e.to_string())
}

const CLASSIFY_RULES: &[(&[&str], InventoryStatus)] = &[
    (&["source_changed"], InventoryStatus::SourceChanged),
    (
        &["capture_byte_limit", "quota"],
        InventoryStatus::QuotaBlocked,
    ),
    (&["source_not_utf8"], InventoryStatus::UnknownFormat),
    (
        &[
            "source_not_authorized",
            "source_disabled",
            "capture_disabled",
            "permission",
        ],
        InventoryStatus::PermissionRequired,
    ),
];

pub(super) fn classify(error: &str) -> InventoryStatus {
    CLASSIFY_RULES
        .iter()
        .find(|(needles, _)| needles.iter().any(|needle| error.contains(*needle)))
        .map(|(_, status)| *status)
        .unwrap_or(InventoryStatus::SourceUnavailable)
}

fn io_error(error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        format!("permission_required: {error}")
    } else {
        format!("source_unavailable: {error}")
    }
}

pub(super) fn file_source_path(key: &str) -> Result<&str, String> {
    key.strip_prefix("file:")
        .ok_or_else(|| "source_not_file".into())
}

pub(super) fn inventory_cursor_in_range(inventory: &SourceInventory) -> Result<(), String> {
    (inventory.next_index <= inventory.entries.len())
        .then_some(())
        .ok_or_else(|| "inventory_cursor_invalid".into())
}

pub(super) fn snapshot(key: &str) -> Result<(u64, String), String> {
    let path = Path::new(file_source_path(key)?);
    // Relative names and aliases cannot widen an explicit canonical registration.
    if !path.is_absolute() {
        return Err("source_not_authorized".into());
    }
    let file = crate::auth::open_nofollow(path).map_err(io_error)?;
    let canonical = path.canonicalize().map_err(io_error)?;
    if canonical != path {
        return Err("source_changed: canonical_path".into());
    }
    let meta = file.metadata().map_err(io_error)?;
    if !meta.is_file() {
        return Err("source_unavailable: not_regular_file".into());
    }
    let modified = meta.modified().map_err(io_error)?;
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!(
            "{}:{}:{}:{}",
            meta.dev(),
            meta.ino(),
            meta.ctime(),
            meta.ctime_nsec()
        )
    };
    #[cfg(not(unix))]
    let identity = format!("{:?}", meta.created().map_err(io_error)?);
    Ok((
        meta.len(),
        format!("file-metadata/1:{identity}:{modified:?}:{}", meta.len()),
    ))
}

pub(super) fn mark(entry: &mut InventoryEntry, status: InventoryStatus, detail: impl Into<String>) {
    entry.status = status;
    entry.detail = Some(detail.into());
}

pub(super) fn registered_file_keys(
    conn: &Connection,
    principal: &str,
    limit: Option<i64>,
) -> Result<Vec<String>, String> {
    if !crate::db::table_exists(conn, "observation_sources") {
        return Ok(Vec::new());
    }
    const FILE_KEYS_SQL: &str = "SELECT source_key FROM observation_sources WHERE principal=?1 AND substr(source_key,1,5)='file:' ORDER BY source_key";
    let limited = limit.map(|_| format!("{FILE_KEYS_SQL} LIMIT ?2"));
    let sql = limited.as_deref().unwrap_or(FILE_KEYS_SQL);
    let mut statement = conn.prepare(sql).map_err(|e| e.to_string())?;
    fn key(row: &rusqlite::Row<'_>) -> rusqlite::Result<String> {
        row.get(0)
    }
    match limit {
        Some(n) => statement.query_map(params![principal, n], key),
        None => statement.query_map(params![principal], key),
    }
    .map_err(|e| e.to_string())?
    .collect::<Result<_, _>>()
    .map_err(|e| e.to_string())
}

pub(super) fn inventory_format(key: &str) -> Result<String, String> {
    Ok(Path::new(file_source_path(key)?)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| {
            matches!(
                s.as_str(),
                "md" | "txt"
                    | "json"
                    | "jsonl"
                    | "toml"
                    | "yaml"
                    | "yml"
                    | "rs"
                    | "js"
                    | "ts"
                    | "py"
            )
        })
        .unwrap_or_else(|| "unknown".into()))
}

pub(super) fn cas_inventory(
    conn: &Connection,
    principal: &str,
    revision: &str,
    body: &str,
    previous: &str,
    restore_epoch: &str,
) -> Result<(), String> {
    let updated = conn.execute("UPDATE observation_inventories SET inventory_json=?3 WHERE principal=?1 AND revision=?2 AND inventory_json=?4 AND EXISTS(SELECT 1 FROM brain_meta WHERE singleton=1 AND restore_epoch=?5)", params![principal, revision, body, previous, restore_epoch]).map_err(|e| e.to_string())?;
    if updated != 1 {
        return Err("inventory_resume_conflict".into());
    }
    Ok(())
}

impl CortexRuntime {
    /// Enumerate only this principal's explicitly registered `file:` sources.
    /// Non-file adapters have their own frontiers; no directories are traversed.
    /// New registrations never alter an existing revision's denominator.
    pub async fn inventory_sources(&self, cx: &Cx) -> Result<SourceInventory, String> {
        self.with_locked_db(cx, |conn, principal| {
            ensure(conn)?;
        let keys: Vec<String> = registered_file_keys(&conn, &principal, Some(MAX_INVENTORY_SOURCES as i64 + 1))?;
        if keys.len() > MAX_INVENTORY_SOURCES { return Err("inventory_source_limit".into()); }
        let mut entries = Vec::with_capacity(keys.len());
        for key in keys {
            let mut entry = InventoryEntry { format: inventory_format(&key)?, source_key: key, bytes: None, source_revision: None, status: InventoryStatus::PartiallyIndexed, detail: None, receipt: None };
            // Check grant before any filesystem access, including metadata.
            match crate::indexer::file_capture_limit(&conn, &principal, &entry.source_key) {
                Err(error) => mark(&mut entry, classify(&error), error),
                Ok(limit) => match snapshot(&entry.source_key) {
                    Ok((bytes, revision)) => {
                        entry.bytes = Some(bytes);
                        entry.source_revision = Some(revision);
                        if bytes > limit as u64 { mark(&mut entry, InventoryStatus::QuotaBlocked, "capture_byte_limit"); }
                    }
                    Err(error) => mark(&mut entry, classify(&error), error),
                },
            }
            entries.push(entry);
        }
        let (revision, restore_epoch): (String, String) = conn.query_row("SELECT lower(hex(randomblob(16))),restore_epoch FROM brain_meta WHERE singleton=1", [], |r| Ok((r.get(0)?, r.get(1)?))).map_err(|e| e.to_string())?;
        let inventory = SourceInventory { revision, restore_epoch, next_index: 0, status: if entries.is_empty() { InventoryStatus::Ready } else { InventoryStatus::PartiallyIndexed }, entries, untracked_registrations: 0 };
        conn.execute("INSERT INTO observation_inventories VALUES(?1,?2,?3)", params![principal, inventory.revision, serde_json::to_string(&inventory).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        Ok(inventory)
        }).await
    }

    /// Inspect persisted progress without accessing any source path.
    pub async fn read_inventory(&self, cx: &Cx, revision: &str) -> Result<SourceInventory, String> {
        self.with_locked_db(cx, |conn, principal| {
            ensure(conn)?;
            let body: String = conn.query_row("SELECT inventory_json FROM observation_inventories WHERE principal=?1 AND revision=?2", params![principal, revision], |r| r.get(0))
            .optional().map_err(|e| e.to_string())?.ok_or("inventory_not_found")?;
            let inventory: SourceInventory = serde_json::from_str(&body).map_err(|e| e.to_string())?;
            let epoch: String = conn.query_row("SELECT restore_epoch FROM brain_meta WHERE singleton=1", [], |r| r.get(0)).map_err(|e| e.to_string())?;
            if inventory.restore_epoch != epoch { return Err("inventory_restore_epoch_changed".into()); }
            Ok(inventory)
        }).await
    }
}
