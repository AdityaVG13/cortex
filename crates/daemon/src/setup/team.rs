use super::helpers::{arg_value, persist_team_owner_token, restore_previous_token, rollback_team_setup};
use crate::auth;
use crate::db;
use std::fs;
pub async fn run_setup_team(args: &[String], dry_run: bool) {
    let db_path = auth::db_path();
    if let Some(parent) = db_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
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
        let bak_path = db_path.with_extension("db.bak");
        eprint!("  Backing up database to {}... ", bak_path.display());
        drop(conn);
        if let Err(e) = fs::copy(&db_path, &bak_path) {
            eprintln!("FAILED");
            eprintln!("  [FAIL] Backup failed: {e}  -- aborting migration.");
            return;
        }
        eprintln!("done");
        let wal = db_path.with_extension("db-wal");
        let shm = db_path.with_extension("db-shm");
        if wal.exists() {
            let _ = fs::copy(&wal, bak_path.with_extension("db.bak-wal"));
        }
        if shm.exists() {
            let _ = fs::copy(&shm, bak_path.with_extension("db.bak-shm"));
        }
    } else if !dry_run {
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
    if let Err(e) = conn.execute_batch("BEGIN IMMEDIATE") {
        eprintln!("  [FAIL] Cannot begin transaction: {e}");
        return;
    }
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
    let paths = auth::CortexPaths::resolve();
    let previous_token = fs::read(&paths.token).ok();
    if let Err(e) = persist_team_owner_token(&paths, &owner_key) {
        rollback_team_setup(&conn);
        eprintln!("  [FAIL] Team migration rolled back because owner token persistence failed: {e}");
        return;
    }
    if let Err(e) = conn.execute_batch("COMMIT") {
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
