use super::helpers::{arg_value, persist_team_owner_token, restore_previous_token, rollback_team_setup};
use crate::auth;
use crate::db;
use std::fs;

/// Coherent copy of the target database (WAL included via the online backup
/// API) in a temporary directory. Returns the directory guard and the copy path.
fn dry_run_copy(real_db_path: &std::path::Path) -> Result<(tempfile::TempDir, std::path::PathBuf), String> {
    let dir = tempfile::Builder::new().prefix("cortex-migrate-dry-run-").tempdir().map_err(|e| e.to_string())?;
    let copy = dir.path().join("cortex.db");
    if real_db_path.exists() {
        let source = db::open(real_db_path).map_err(|e| e.to_string())?;
        let mut dest = rusqlite::Connection::open(&copy).map_err(|e| e.to_string())?;
        let backup = rusqlite::backup::Backup::new(&source, &mut dest).map_err(|e| e.to_string())?;
        backup.run_to_completion(64, std::time::Duration::from_millis(5), None).map_err(|e| e.to_string())?;
    }
    Ok((dir, copy))
}
pub async fn run_setup_team(args: &[String], dry_run: bool) {
    // Resolve from the CLI args (falling back to env/defaults) so the global
    // `--home`/`--db` flags the validator accepts actually select the target.
    // The previous env-only resolution silently migrated the default home
    // while `--home` pointed elsewhere.
    let paths = auth::CortexPaths::resolve_from_args(args);
    let real_db_path = paths.db.clone();
    if let Some(parent) = real_db_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    // A dry run never touches the real database: schema initialization and
    // migrations below are real writes, so the preview runs against a coherent
    // online-backup copy in a scratch directory that is removed afterwards.
    let scratch = if dry_run {
        match dry_run_copy(&real_db_path) {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("  [FAIL] Cannot prepare dry-run copy: {e}");
                return;
            }
        }
    } else {
        None
    };
    let db_path = scratch.as_ref().map(|(_, path)| path.clone()).unwrap_or_else(|| real_db_path.clone());
    let conn = match db::open(&db_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  [FAIL] Cannot open {}: {e}", db_path.display());
            return;
        }
    };
    if let Err(e) = db::configure(&conn) {
        eprintln!("  [FAIL] Cannot configure DB: {e}");
        return;
    }
    if let Err(e) = db::initialize_schema(&conn) {
        eprintln!("  [FAIL] Cannot initialize schema: {e}");
        return;
    }
    db::migrate_focus_table(&conn);
    crate::crystallize::migrate_crystal_tables(&conn);
    if db::is_team_mode(&conn) {
        eprintln!();
        eprintln!("  Already in team mode. No changes needed.");
        eprintln!();
        return;
    }
    let default_owner = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_else(|_| "owner".to_string());
    let owner = if let Some(v) = arg_value(args, "--owner") {
        v
    } else {
        eprint!("  Enter owner username [default: {default_owner}]: ");
        let mut input = String::new();
        if std::io::stdin().read_line(&mut input).is_ok() && !input.trim().is_empty() {
            input.trim().to_string()
        } else {
            default_owner.clone()
        }
    };
    let display_name = arg_value(args, "--display-name").unwrap_or_else(|| owner.clone());
    eprintln!();
    if dry_run {
        eprintln!("  [DRY RUN] Cortex Team Migration Preview");
        eprintln!("  ========================================");
    } else {
        eprintln!("  Cortex Team Setup");
        eprintln!("  =================");
    }
    eprintln!();
    eprintln!("  Owner username: {owner}");
    eprintln!();
    if !dry_run && db_path.exists() {
        // Coherent pre-migration backup through the online backup API; a raw
        // file copy can miss WAL state.
        let bak_path = db_path.with_extension("db.bak");
        eprint!("  Backing up database to {}... ", bak_path.display());
        let backup_result = rusqlite::Connection::open(&bak_path).map_err(|e| e.to_string()).and_then(|mut dest| {
            let backup = rusqlite::backup::Backup::new(&conn, &mut dest).map_err(|e| e.to_string())?;
            backup.run_to_completion(64, std::time::Duration::from_millis(5), None).map_err(|e| e.to_string())
        });
        drop(conn);
        if let Err(e) = backup_result {
            eprintln!("FAILED");
            eprintln!("  [FAIL] Backup failed: {e}  -- aborting migration.");
            return;
        }
        eprintln!("done");
    }
    let conn = match db::open(&db_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  [FAIL] Cannot reopen {}: {e}", db_path.display());
            return;
        }
    };
    if let Err(e) = db::configure(&conn) {
        eprintln!("  [FAIL] Cannot configure DB: {e}");
        return;
    }
    if let Err(e) = db::initialize_schema(&conn) {
        eprintln!("  [FAIL] Cannot initialize schema: {e}");
        return;
    }
    db::migrate_focus_table(&conn);
    crate::crystallize::migrate_crystal_tables(&conn);
    let tx = match db::ImmediateWrite::begin(&conn) {
        Ok(tx) => tx,
        Err(e) => {
            eprintln!("  [FAIL] Cannot begin transaction: {e}");
            return;
        }
    };
    eprintln!("  Migrating to team mode...");
    eprintln!();
    let owner_key = auth::generate_ctx_api_key();
    let owner_hash = match auth::hash_api_key_argon2id(&owner_key) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  [FAIL] Failed to hash owner API key: {e}");
            rollback_team_setup(&conn);
            return;
        }
    };
    eprint!("  Creating team tables... ");
    if let Err(e) = db::create_team_mode_tables(&conn) {
        eprintln!("FAILED");
        eprintln!("  [FAIL] {e}");
        rollback_team_setup(&conn);
        return;
    }
    eprintln!("done");
    let owner_id = match db::upsert_owner_user(&conn, &owner, Some(&display_name), &owner_hash) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  [FAIL] Failed to create owner user: {e}");
            rollback_team_setup(&conn);
            return;
        }
    };
    eprint!("  Adding ownership columns... ");
    if let Err(e) = db::migrate_to_team_mode(&conn, owner_id) {
        eprintln!("FAILED");
        eprintln!("  [FAIL] {e}");
        rollback_team_setup(&conn);
        return;
    }
    eprintln!("done");
    eprintln!();
    let counts = db::migration_counts(&conn);
    let total: i64 = counts.iter().map(|(_, n)| n).sum();
    if dry_run {
        eprintln!("  [DRY RUN] Would migrate to team mode:");
    } else {
        eprintln!("  Assigned ownership:");
    }
    let label_width = 22;
    for (table, count) in &counts {
        if dry_run {
            eprintln!("    {:<width$} {:>6} rows would be assigned", format!("{table}:"), count, width = label_width,);
        } else {
            eprintln!("    {:<width$} {:>6} rows", format!("{table}:"), count, width = label_width,);
        }
    }
    if dry_run {
        eprintln!("    {:<width$} {:>6} rows", "Total:", total, width = label_width);
        eprintln!();
        rollback_team_setup(&conn);
        eprintln!("  No changes made.");
        eprintln!();
        return;
    }
    let default_team_id = match db::ensure_default_team_membership(&conn, owner_id) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("  [FAIL] Failed to create default team membership: {e}");
            rollback_team_setup(&conn);
            return;
        }
    };
    let previous_token = fs::read(&paths.token).ok();
    if let Err(e) = persist_team_owner_token(&paths, &owner_key) {
        rollback_team_setup(&conn);
        eprintln!("  [FAIL] Team migration rolled back because owner token persistence failed: {e}");
        return;
    }
    if let Err(e) = tx.commit() {
        rollback_team_setup(&conn);
        restore_previous_token(&paths, previous_token);
        eprintln!("  [FAIL] Failed to commit team migration: {e}");
        return;
    }
    let key_preview: String = owner_key.chars().take(8).collect();
    eprintln!("    ────────────────────────────");
    eprintln!("    {:<width$} {:>6} rows -> owner \"{owner}\" (id: {owner_id})", "Total:", total, width = label_width,);
    eprintln!();
    eprintln!("  All rows set to visibility: private");
    eprintln!();
    eprintln!("  Generated API key: {key_preview}...");
    eprintln!("  Save this key -- it will not be shown again.");
    eprintln!("  (Full key written to {})", paths.token.display());
    eprintln!();
    eprintln!("  Default team id: {default_team_id}");
    eprintln!();
    eprintln!("  Migration complete. Restart daemon: cortex serve");
    eprintln!();
}
