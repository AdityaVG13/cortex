//! Registered-only, revision-bound, restartable base population.
use cortex_kernel::runtime::{CortexRuntime, inventory::InventoryStatus, observation::SourceSpec};
use cortex_tests::support::run_with_cx;

#[test]
fn inventory_is_registered_only_and_resumes_after_reopen() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let db = home.path().join("brain.db");
        let runtime = CortexRuntime::open_db(&db).unwrap();
        std::fs::write(home.path().join("ungranted.md"), "secret").unwrap();
        assert!(
            runtime
                .inventory_sources(&cx)
                .await
                .unwrap()
                .entries
                .is_empty()
        );
        for name in ["a.md", "b.md"] {
            let path = home.path().join(name);
            std::fs::write(&path, format!("exact {name}\nrare tail\n")).unwrap();
            let key = format!("file:{}", path.canonicalize().unwrap().display());
            runtime
                .register_source(&cx, SourceSpec::document(key, "project"))
                .await
                .unwrap();
        }
        let inventory = runtime.inventory_sources(&cx).await.unwrap();
        assert_eq!(inventory.entries.len(), 2);
        assert_eq!(inventory.status, InventoryStatus::PartiallyIndexed);
        let first = runtime
            .bootstrap_inventory(&cx, &inventory.revision, 1, 65536)
            .await
            .unwrap();
        assert_eq!(first.next_index, 1);
        assert_eq!(first.status, InventoryStatus::PartiallyIndexed);
        let receipt = first.entries[0].receipt.as_ref().unwrap();
        assert!(
            runtime
                .read_observation(&cx, &receipt.source_id)
                .await
                .unwrap()
                .text
                .ends_with("rare tail\n")
        );
        drop(runtime);
        let runtime = CortexRuntime::open_db(&db).unwrap();
        let done = runtime
            .bootstrap_inventory(&cx, &inventory.revision, 1, 65536)
            .await
            .unwrap();
        assert_eq!(done.next_index, 2);
        assert_eq!(done.status, InventoryStatus::Ready);
        let retry = runtime
            .bootstrap_inventory(&cx, &inventory.revision, 1, 65536)
            .await
            .unwrap();
        assert_eq!(retry.next_index, 2);
        let newer = runtime.inventory_sources(&cx).await.unwrap();
        assert_ne!(newer.revision, inventory.revision);
    });
}

#[test]
fn denied_changed_missing_binary_and_quota_do_not_advance() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        let path = home.path().join("source.md");
        std::fs::write(&path, "original").unwrap();
        let key = format!("file:{}", path.canonicalize().unwrap().display());
        runtime
            .register_source(&cx, SourceSpec::document(&key, "project"))
            .await
            .unwrap();
        let inventory = runtime.inventory_sources(&cx).await.unwrap();
        let blocked = runtime
            .bootstrap_inventory(&cx, &inventory.revision, 1, 1)
            .await
            .unwrap();
        assert_eq!(blocked.status, InventoryStatus::QuotaBlocked);
        assert_eq!(blocked.next_index, 0);
        runtime.set_source_enabled(&cx, &key, false).await.unwrap();
        let blocked = runtime
            .bootstrap_inventory(&cx, &inventory.revision, 1, 65536)
            .await
            .unwrap();
        assert_eq!(blocked.status, InventoryStatus::PermissionRequired);
        assert_eq!(blocked.next_index, 0);
        runtime.set_source_enabled(&cx, &key, true).await.unwrap();
        std::fs::write(&path, "changed length").unwrap();
        let changed = runtime
            .bootstrap_inventory(&cx, &inventory.revision, 1, 65536)
            .await
            .unwrap();
        assert_eq!(changed.status, InventoryStatus::SourceChanged);
        assert_eq!(changed.next_index, 0);
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        let binary = runtime.inventory_sources(&cx).await.unwrap();
        let blocked = runtime
            .bootstrap_inventory(&cx, &binary.revision, 1, 65536)
            .await
            .unwrap();
        assert_eq!(blocked.status, InventoryStatus::UnknownFormat);
        assert_eq!(blocked.next_index, 0);
        std::fs::remove_file(&path).unwrap();
        let missing = runtime.inventory_sources(&cx).await.unwrap();
        assert_eq!(
            missing.entries[0].status,
            InventoryStatus::SourceUnavailable
        );
    });
}

#[test]
fn inventory_revisions_are_principal_scoped_and_ownerless_team_fails() {
    run_with_cx(|cx| async move {
        let alice = cortex_tests::support::team_state(1);
        let mut bob = alice.clone();
        bob.default_owner_id = Some(2);
        let mut ownerless = alice.clone();
        ownerless.default_owner_id = None;
        let a = CortexRuntime::from_state(alice);
        let revision = a.inventory_sources(&cx).await.unwrap().revision;
        let b = CortexRuntime::from_state(bob);
        assert!(
            b.bootstrap_inventory(&cx, &revision, 1, 65536)
                .await
                .is_err()
        );
        assert!(
            CortexRuntime::from_state(ownerless)
                .inventory_sources(&cx)
                .await
                .is_err()
        );
    });
}

#[test]
fn capture_commit_before_progress_checkpoint_is_replayable() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        let path = home.path().join("source.md");
        std::fs::write(&path, "exact checkpoint evidence").unwrap();
        let key = format!("file:{}", path.canonicalize().unwrap().display());
        runtime
            .register_source(&cx, SourceSpec::document(&key, "project"))
            .await
            .unwrap();
        let inventory = runtime.inventory_sources(&cx).await.unwrap();
        {
            let conn = runtime.state().db.lock(&cx).await.unwrap();
            conn.execute_batch("CREATE TRIGGER fail_inventory_checkpoint BEFORE UPDATE ON observation_inventories BEGIN SELECT RAISE(ABORT,'checkpoint_failed'); END;").unwrap();
        }
        assert!(
            runtime
                .bootstrap_inventory(&cx, &inventory.revision, 1, 65536)
                .await
                .unwrap_err()
                .contains("checkpoint_failed")
        );
        assert_eq!(
            runtime
                .read_inventory(&cx, &inventory.revision)
                .await
                .unwrap()
                .next_index,
            0
        );
        {
            let conn = runtime.state().db.lock(&cx).await.unwrap();
            assert_eq!(
                conn.query_row("SELECT count(*) FROM observation_events", [], |r| r
                    .get::<_, i64>(0))
                    .unwrap(),
                1
            );
            conn.execute_batch("DROP TRIGGER fail_inventory_checkpoint;")
                .unwrap();
        }
        let done = runtime
            .bootstrap_inventory(&cx, &inventory.revision, 1, 65536)
            .await
            .unwrap();
        assert_eq!(done.status, InventoryStatus::Ready);
        assert!(done.entries[0].receipt.as_ref().unwrap().duplicate);
    });
}

#[test]
fn reconcile_marks_changed_files_and_counts_later_registrations() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        let first = home.path().join("first.md");
        std::fs::write(&first, "original").unwrap();
        let key = format!("file:{}", first.canonicalize().unwrap().display());
        runtime
            .register_source(&cx, SourceSpec::document(&key, "project"))
            .await
            .unwrap();
        let inventory = runtime.inventory_sources(&cx).await.unwrap();
        runtime
            .bootstrap_inventory(&cx, &inventory.revision, 1, 65536)
            .await
            .unwrap();
        std::fs::write(&first, "changed after inventory").unwrap();
        let second = home.path().join("second.md");
        std::fs::write(&second, "later").unwrap();
        runtime
            .register_source(
                &cx,
                SourceSpec::document(
                    format!("file:{}", second.canonicalize().unwrap().display()),
                    "project",
                ),
            )
            .await
            .unwrap();
        let reconciled = runtime
            .reconcile_inventory(&cx, &inventory.revision)
            .await
            .unwrap();
        assert_eq!(reconciled.entries.len(), 1);
        assert_eq!(reconciled.entries[0].status, InventoryStatus::SourceChanged);
        assert_eq!(reconciled.untracked_registrations, 1);
        assert_eq!(reconciled.next_index, 1);
    });
}

#[test]
fn inventory_snapshot_applies_indexer_file_ceiling() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let runtime = CortexRuntime::open_db(&home.path().join("brain.db")).unwrap();
        let path = home.path().join("big.md");
        let oversized = cortex_kernel::indexer::INDEXER_MAX_FILE_BYTES as usize + 1;
        std::fs::write(&path, vec![b'a'; oversized]).unwrap();
        let key = format!("file:{}", path.canonicalize().unwrap().display());
        let mut spec = SourceSpec::document(&key, "project");
        spec.max_bytes = cortex_kernel::runtime::observation::MAX_CAPTURE_BYTES;
        runtime.register_source(&cx, spec).await.unwrap();
        let inventory = runtime.inventory_sources(&cx).await.unwrap();
        assert_eq!(inventory.entries[0].status, InventoryStatus::QuotaBlocked);
        assert_eq!(
            inventory.entries[0].detail.as_deref(),
            Some("capture_byte_limit")
        );
        let blocked = runtime
            .bootstrap_inventory(&cx, &inventory.revision, 1, 16 * 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(blocked.status, InventoryStatus::QuotaBlocked);
        assert_eq!(blocked.next_index, 0);
    });
}
