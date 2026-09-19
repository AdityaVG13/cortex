use super::{
    InventoryStatus, MAX_BOOTSTRAP_BYTES, MAX_BOOTSTRAP_SOURCES, SourceInventory, cas_inventory,
    classify, ensure, file_source_path, inventory_cursor_in_range, mark, registered_file_keys,
    snapshot,
};
use crate::runtime::CortexRuntime;
use asupersync::Cx;
use std::path::PathBuf;

impl CortexRuntime {
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
        let principal = self.observation_principal()?;
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
                crate::indexer::file_capture_limit(&conn, &principal, &entry.source_key)
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
                        let mut conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
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
            cas_inventory(
                &conn,
                &principal,
                revision,
                &body,
                &previous,
                &inventory.restore_epoch,
            )?;
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
        let principal = self.observation_principal()?;
        let mut inventory = self.read_inventory(cx, revision).await?;
        inventory_cursor_in_range(&inventory)?;
        let previous = serde_json::to_string(&inventory).map_err(|e| e.to_string())?;
        let conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        ensure(&conn)?;
        let current = registered_file_keys(&conn, &principal, None)?;
        let known: std::collections::BTreeSet<_> = inventory
            .entries
            .iter()
            .map(|entry| entry.source_key.clone())
            .collect();
        inventory.untracked_registrations =
            current.iter().filter(|key| !known.contains(*key)).count();
        for entry in &mut inventory.entries {
            if !current.iter().any(|key| key == &entry.source_key) {
                mark(
                    entry,
                    InventoryStatus::PermissionRequired,
                    "registration_missing",
                );
                continue;
            }
            match crate::indexer::file_capture_limit(&conn, &principal, &entry.source_key).and_then(
                |limit| {
                    snapshot(&entry.source_key)
                        .map(|(bytes, revision)| (limit as u64, bytes, revision))
                },
            ) {
                Err(error) => mark(entry, classify(&error), error),
                Ok((limit, bytes, current_revision)) => {
                    entry.bytes = Some(bytes);
                    if entry.source_revision.as_ref() != Some(&current_revision) {
                        mark(
                            entry,
                            InventoryStatus::SourceChanged,
                            "source_changed: reconcile",
                        );
                    } else if bytes > limit {
                        mark(entry, InventoryStatus::QuotaBlocked, "capture_byte_limit");
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
        cas_inventory(
            &conn,
            &principal,
            revision,
            &body,
            &previous,
            &inventory.restore_epoch,
        )?;
        Ok(inventory)
    }
}
