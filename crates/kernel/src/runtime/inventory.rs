//! Explicit registered-file inventory and bounded, restartable exact base capture.
//!
//! A revision is a metadata snapshot, NOT a global filesystem snapshot. Completion
//! describes exact capture only, never projection, delivery, or invocation presence.
//! Blocked entries retain the cursor. A changed source requires a new inventory.
//! Capture commits before progress: a crash between them safely retries observe_file.
use super::{
    CortexRuntime,
    observation::{self, ObservationReceipt},
};
use asupersync::Cx;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

fn principal(runtime: &CortexRuntime) -> Result<String, String> {
    match (runtime.state().team_mode, runtime.state().default_owner_id) {
        (true, Some(id)) => Ok(format!("user:{id}")),
        (true, None) => Err("local_owner_required".into()),
        (false, _) => Ok("local".into()),
    }
}

fn ensure(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS observation_inventories (
        principal TEXT NOT NULL, revision TEXT NOT NULL,
        inventory_json TEXT NOT NULL CHECK(json_valid(inventory_json)),
        PRIMARY KEY(principal,revision)
    );",
    )
    .map_err(|e| e.to_string())
}

fn classify(error: &str) -> InventoryStatus {
    if error.contains("source_changed") {
        InventoryStatus::SourceChanged
    } else if error.contains("capture_byte_limit") || error.contains("quota") {
        InventoryStatus::QuotaBlocked
    } else if error.contains("source_not_utf8") {
        InventoryStatus::UnknownFormat
    } else if error.contains("source_not_authorized")
        || error.contains("source_disabled")
        || error.contains("capture_disabled")
        || error.contains("permission")
    {
        InventoryStatus::PermissionRequired
    } else {
        InventoryStatus::SourceUnavailable
    }
}

fn io_error(error: std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::PermissionDenied {
        format!("permission_required: {error}")
    } else {
        format!("source_unavailable: {error}")
    }
}

fn file_source_path(key: &str) -> Result<&str, String> {
    key.strip_prefix("file:").ok_or_else(|| "source_not_file".into())
}

fn inventory_cursor_in_range(inventory: &SourceInventory) -> Result<(), String> {
    if inventory.next_index > inventory.entries.len() {
        return Err("inventory_cursor_invalid".into());
    }
    Ok(())
}

fn snapshot(key: &str) -> Result<(u64, String), String> {
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

fn mark(entry: &mut InventoryEntry, status: InventoryStatus, detail: impl Into<String>) {
    entry.status = status;
    entry.detail = Some(detail.into());
}

impl CortexRuntime {
    /// Enumerate only this principal's explicitly registered `file:` sources.
    /// Non-file adapters have their own frontiers; no directories are traversed.
    /// New registrations never alter an existing revision's denominator.
    pub async fn inventory_sources(&self, cx: &Cx) -> Result<SourceInventory, String> {
        let principal = principal(self)?;
        let conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        ensure(&conn)?;
        let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='observation_sources')", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        let keys: Vec<String> = if exists {
            let mut statement = conn.prepare("SELECT source_key FROM observation_sources WHERE principal=?1 AND substr(source_key,1,5)='file:' ORDER BY source_key LIMIT ?2").map_err(|e| e.to_string())?;
            statement
                .query_map(params![principal, MAX_INVENTORY_SOURCES as i64 + 1], |r| {
                    r.get(0)
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };
        if keys.len() > MAX_INVENTORY_SOURCES {
            return Err("inventory_source_limit".into());
        }
        let mut entries = Vec::with_capacity(keys.len());
        for key in keys {
            let format = Path::new(file_source_path(&key)?)
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
                .unwrap_or_else(|| "unknown".into());
            let mut entry = InventoryEntry {
                source_key: key,
                format,
                bytes: None,
                source_revision: None,
                status: InventoryStatus::PartiallyIndexed,
                detail: None,
                receipt: None,
            };
            // Check grant before any filesystem access, including metadata.
            match observation::source_capture_limit(&conn, &principal, &entry.source_key) {
                Err(error) => mark(&mut entry, classify(&error), error),
                Ok(limit) => match snapshot(&entry.source_key) {
                    Ok((bytes, revision)) => {
                        entry.bytes = Some(bytes);
                        entry.source_revision = Some(revision);
                        if bytes > limit as u64 {
                            mark(
                                &mut entry,
                                InventoryStatus::QuotaBlocked,
                                "capture_byte_limit",
                            );
                        }
                    }
                    Err(error) => mark(&mut entry, classify(&error), error),
                },
            }
            entries.push(entry);
        }
        let (revision, restore_epoch): (String, String) = conn
            .query_row(
                "SELECT lower(hex(randomblob(16))),restore_epoch FROM brain_meta WHERE singleton=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        let inventory = SourceInventory {
            revision,
            restore_epoch,
            next_index: 0,
            status: if entries.is_empty() {
                InventoryStatus::Ready
            } else {
                InventoryStatus::PartiallyIndexed
            },
            entries,
            untracked_registrations: 0,
        };
        conn.execute(
            "INSERT INTO observation_inventories VALUES(?1,?2,?3)",
            params![
                principal,
                inventory.revision,
                serde_json::to_string(&inventory).map_err(|e| e.to_string())?
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(inventory)
    }

    /// Inspect persisted progress without accessing any source path.
    pub async fn read_inventory(&self, cx: &Cx, revision: &str) -> Result<SourceInventory, String> {
        let principal = principal(self)?;
        let conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        ensure(&conn)?;
        let body: String = conn.query_row("SELECT inventory_json FROM observation_inventories WHERE principal=?1 AND revision=?2", params![principal, revision], |r| r.get(0))
            .optional().map_err(|e| e.to_string())?.ok_or("inventory_not_found")?;
        let inventory: SourceInventory = serde_json::from_str(&body).map_err(|e| e.to_string())?;
        let epoch: String = conn
            .query_row(
                "SELECT restore_epoch FROM brain_meta WHERE singleton=1",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if inventory.restore_epoch != epoch {
            return Err("inventory_restore_epoch_changed".into());
        }
        Ok(inventory)
    }

    /// Resume the durable contiguous cursor, bounded by source count and bytes.
    /// A blocked source is retried, never skipped. Capture uses observe_file exactly;
    /// idempotency handles crashes before the separate progress checkpoint. Concurrent
    /// resumptions use compare-and-swap and return inventory_resume_conflict to retry.
    pub async fn bootstrap_inventory(
        &self,
        cx: &Cx,
        revision: &str,
        max_sources: usize,
        max_bytes: u64,
    ) -> Result<SourceInventory, String> {
        if max_sources == 0
            || max_sources > MAX_BOOTSTRAP_SOURCES
            || max_bytes == 0
            || max_bytes > MAX_BOOTSTRAP_BYTES
        {
            return Err("invalid_bootstrap_budget".into());
        }
        let principal = principal(self)?;
        let mut inventory = self.read_inventory(cx, revision).await?;
        inventory_cursor_in_range(&inventory)?;
        let mut budget = max_bytes;
        for _ in 0..max_sources {
            if inventory.next_index == inventory.entries.len() {
                break;
            }
            let previous = serde_json::to_string(&inventory).map_err(|e| e.to_string())?;
            let entry = &mut inventory.entries[inventory.next_index];
            let limit = {
                let conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
                observation::source_capture_limit(&conn, &principal, &entry.source_key)
            };
            let probe = limit.and_then(|limit| {
                let (bytes, current) = snapshot(&entry.source_key)?;
                if entry.source_revision.as_ref() != Some(&current) {
                    return Err("source_changed: create_new_inventory".into());
                }
                // Reserve the grant ceiling, not observed length: a concurrent
                // writer may grow the file between this probe and exact intake.
                if bytes > limit as u64 || limit as u64 > budget {
                    return Err("quota_blocked: batch_or_source_byte_limit".into());
                }
                Ok(limit as u64)
            });
            match probe {
                Err(error) => mark(entry, classify(&error), error),
                Ok(bytes) => {
                    budget -= bytes;
                    let owner = if self.state().team_mode {
                        self.state().default_owner_id
                    } else {
                        None
                    };
                    let path = PathBuf::from(file_source_path(&entry.source_key)?);
                    match {
                        let mut conn = self
                            .state()
                            .db
                            .lock(cx)
                            .await
                            .map_err(|e| e.to_string())?;
                        crate::indexer::index_file_nofollow(&mut conn, &path, owner)
                    } {
                        Err(error) => mark(entry, classify(&error), error),
                        Ok(receipt) => {
                            entry.receipt = Some(receipt);
                            match snapshot(&entry.source_key) {
                                Ok((_, current))
                                    if entry.source_revision.as_ref() == Some(&current) =>
                                {
                                    entry.status = InventoryStatus::Ready;
                                    entry.detail = None;
                                    inventory.next_index += 1;
                                }
                                _ => mark(
                                    entry,
                                    InventoryStatus::SourceChanged,
                                    "source_changed: after_capture",
                                ),
                            }
                        }
                    }
                }
            }
            let blocked = inventory.next_index < inventory.entries.len()
                && inventory.entries[inventory.next_index].status
                    != InventoryStatus::PartiallyIndexed;
            inventory.status = if inventory.next_index == inventory.entries.len() {
                InventoryStatus::Ready
            } else if blocked {
                inventory.entries[inventory.next_index].status
            } else {
                InventoryStatus::PartiallyIndexed
            };
            let body = serde_json::to_string(&inventory).map_err(|e| e.to_string())?;
            let conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
            let updated = conn.execute("UPDATE observation_inventories SET inventory_json=?3 WHERE principal=?1 AND revision=?2 AND inventory_json=?4 AND EXISTS(SELECT 1 FROM brain_meta WHERE singleton=1 AND restore_epoch=?5)",
                params![principal, revision, body, previous, inventory.restore_epoch]).map_err(|e| e.to_string())?;
            if updated != 1 {
                return Err("inventory_resume_conflict".into());
            }
            if blocked {
                break;
            }
        }
        Ok(inventory)
    }

    /// Re-check this sealed revision against current grants and file metadata.
    /// New registrations are counted, not added. The cursor never advances here.
    pub async fn reconcile_inventory(
        &self,
        cx: &Cx,
        revision: &str,
    ) -> Result<SourceInventory, String> {
        let principal = principal(self)?;
        let mut inventory = self.read_inventory(cx, revision).await?;
        inventory_cursor_in_range(&inventory)?;
        let previous = serde_json::to_string(&inventory).map_err(|e| e.to_string())?;
        let conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        ensure(&conn)?;
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='observation_sources')",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        let current: Vec<String> = if exists {
            let mut statement = conn
                .prepare("SELECT source_key FROM observation_sources WHERE principal=?1 AND substr(source_key,1,5)='file:' ORDER BY source_key")
                .map_err(|e| e.to_string())?;
            statement
                .query_map(params![principal], |r| r.get(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };
        let known: std::collections::BTreeSet<_> = inventory
            .entries
            .iter()
            .map(|entry| entry.source_key.clone())
            .collect();
        inventory.untracked_registrations = current.iter().filter(|key| !known.contains(*key)).count();
        for entry in &mut inventory.entries {
            if !current.iter().any(|key| key == &entry.source_key) {
                mark(
                    entry,
                    InventoryStatus::PermissionRequired,
                    "registration_missing",
                );
                continue;
            }
            match observation::source_capture_limit(&conn, &principal, &entry.source_key)
                .and_then(|_| snapshot(&entry.source_key))
            {
                Err(error) => mark(entry, classify(&error), error),
                Ok((bytes, current_revision)) => {
                    entry.bytes = Some(bytes);
                    if entry.source_revision.as_ref() != Some(&current_revision) {
                        mark(
                            entry,
                            InventoryStatus::SourceChanged,
                            "source_changed: reconcile",
                        );
                    }
                }
            }
        }
        inventory.status = if inventory.next_index == inventory.entries.len()
            && inventory
                .entries
                .iter()
                .all(|entry| entry.status == InventoryStatus::Ready)
        {
            InventoryStatus::Ready
        } else if inventory.next_index < inventory.entries.len()
            && inventory.entries[inventory.next_index].status != InventoryStatus::PartiallyIndexed
        {
            inventory.entries[inventory.next_index].status
        } else {
            InventoryStatus::PartiallyIndexed
        };
        let body = serde_json::to_string(&inventory).map_err(|e| e.to_string())?;
        let updated = conn
            .execute(
                "UPDATE observation_inventories SET inventory_json=?3 WHERE principal=?1 AND revision=?2 AND inventory_json=?4 AND EXISTS(SELECT 1 FROM brain_meta WHERE singleton=1 AND restore_epoch=?5)",
                params![
                    principal,
                    revision,
                    body,
                    previous,
                    inventory.restore_epoch
                ],
            )
            .map_err(|e| e.to_string())?;
        if updated != 1 {
            return Err("inventory_resume_conflict".into());
        }
        Ok(inventory)
    }
}
