//! Coherent backup and restore.
//!
//! Backup uses the SQLite online backup API (WAL state included) and writes a
//! manifest beside the file. Restore refuses a live daemon, takes a coherent
//! pre-restore copy, copies the backup in through the backup API, mints a new
//! restore epoch (old cursors/aliases become `resnapshot_required`), drops
//! disposable presentation state, rebuilds disposable projections, verifies
//! integrity plus sample exact reads, and records a verification report the
//! health surface exposes as `last_verified_restore`.

use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

pub const MANIFEST_SUFFIX: &str = ".manifest.json";
pub const LAST_VERIFIED_RESTORE: &str = ".last_verified_restore.json";
/// Health report is a small JSON object. A replaced huge file must not OOM health.
pub const MAX_RESTORE_REPORT_BYTES: u64 = 64 * 1024;

fn open_configured(path: &Path) -> Result<Connection, String> {
    let conn = super::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    super::configure(&conn).map_err(|e| format!("configure {}: {e}", path.display()))?;
    Ok(conn)
}

pub fn copy_database(source: &Connection, destination: &Path) -> Result<(), String> {
    let mut dest = Connection::open(destination)
        .map_err(|e| format!("open destination {}: {e}", destination.display()))?;
    let backup = rusqlite::backup::Backup::new(source, &mut dest)
        .map_err(|e| format!("backup init: {e}"))?;
    backup
        .run_to_completion(128, std::time::Duration::from_millis(5), None)
        .map_err(|e| format!("backup run: {e}"))
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |r| r.get(0)).unwrap_or(0)
}

pub fn manifest_for(conn: &Connection, file: &Path) -> Value {
    let (brain_id, restore_epoch, policy_epoch) = super::records::brain_epochs(conn);
    json!({
        "file": file.file_name().map(|n| n.to_string_lossy().to_string()),
        "created_at": chrono::Utc::now().to_rfc3339(),
        "sqlite_version": super::sqlite_version(),
        "durability_profile": super::DurabilityProfile::from_env().as_str(),
        "brain_id": brain_id,
        "restore_epoch": restore_epoch,
        "policy_epoch": policy_epoch,
        "schema_user_version": super::current_schema_user_version(conn).unwrap_or(0),
        "counts": {
            "memories": count(conn, "SELECT COUNT(*) FROM memories"),
            "decisions": count(conn, "SELECT COUNT(*) FROM decisions"),
            "records": count(conn, "SELECT COUNT(*) FROM records"),
            "revisions": count(conn, "SELECT COUNT(*) FROM revisions"),
            "versions": count(conn, "SELECT COUNT(*) FROM versions"),
        },
        "bytes": std::fs::metadata(file).map(|m| m.len()).unwrap_or(0),
        "integrity_ok": super::quick_check(conn),
        "external_payloads": [],
        "note": "coherent online-backup snapshot; external payload manifest is empty because all sources are inline"
    })
}

/// Write a coherent backup and its manifest. Returns (db path, manifest path).
pub fn backup_to(db_path: &Path, destination: &Path) -> Result<(PathBuf, PathBuf), String> {
    let source = open_configured(db_path)?;
    if destination.exists() {
        std::fs::remove_file(destination)
            .map_err(|e| format!("replace stale backup {}: {e}", destination.display()))?;
    }
    copy_database(&source, destination)?;
    let verify = open_configured(destination)?;
    if !super::quick_check(&verify) {
        return Err(format!(
            "backup {} failed quick_check",
            destination.display()
        ));
    }
    let manifest = manifest_for(&verify, destination);
    let manifest_path = PathBuf::from(format!("{}{MANIFEST_SUFFIX}", destination.display()));
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap_or_default(),
    )
    .map_err(|e| format!("write manifest: {e}"))?;
    Ok((destination.to_path_buf(), manifest_path))
}

#[derive(Debug)]
pub struct RestoreReport {
    pub new_restore_epoch: String,
    pub previous_restore_epoch: String,
    pub integrity_ok: bool,
    pub sample_reads_ok: bool,
    pub records: i64,
    pub decisions: i64,
    pub memories: i64,
    pub aliases_expired: i64,
    pub projections_rebuilt: usize,
    pub erasure_ledger_present: bool,
    pub erasures_reapplied: usize,
    pub quarantined: bool,
    pub report_path: PathBuf,
}

impl RestoreReport {
    pub fn to_json(&self) -> Value {
        json!({
            "verified_at": chrono::Utc::now().to_rfc3339(),
            "new_restore_epoch": self.new_restore_epoch,
            "previous_restore_epoch": self.previous_restore_epoch,
            "integrity_ok": self.integrity_ok,
            "sample_reads_ok": self.sample_reads_ok,
            "counts": {"records": self.records, "decisions": self.decisions, "memories": self.memories},
            "aliases_expired": self.aliases_expired,
            "projections_rebuilt": self.projections_rebuilt,
            "erasure_ledger_present": self.erasure_ledger_present,
            "erasures_reapplied": self.erasures_reapplied,
            "quarantined": self.quarantined,
            "verified": self.integrity_ok && self.sample_reads_ok && !self.quarantined,
        })
    }
}

/// Restore `backup_file` over `db_path` (daemon must be stopped; the caller
/// checks). `home` receives the verification report.
pub fn restore_from(
    backup_file: &Path,
    db_path: &Path,
    home: &Path,
) -> Result<RestoreReport, String> {
    let candidate = open_configured(backup_file)?;
    if !super::quick_check(&candidate) {
        return Err(format!(
            "backup {} fails quick_check; refusing to restore",
            backup_file.display()
        ));
    }
    drop(candidate);
    // Pre-restore coherent copy of whatever is there now.
    if db_path.exists() {
        let current = open_configured(db_path)?;
        let pre = home.join(format!(
            "cortex.pre-restore.{}.db",
            chrono::Local::now().format("%Y%m%dT%H%M%S")
        ));
        copy_database(&current, &pre)?;
    }
    let previous_epoch = db_path
        .exists()
        .then(|| {
            open_configured(db_path)
                .ok()
                .map(|c| super::records::brain_epochs(&c).1)
        })
        .flatten()
        .unwrap_or_else(|| "0".into());
    // Copy in through the backup API so WAL sidecars of the old file never
    // replay over the restored pages.
    let source = open_configured(backup_file)?;
    let target = open_configured(db_path)?;
    {
        let mut target_mut = Connection::open(db_path).map_err(|e| e.to_string())?;
        let backup = rusqlite::backup::Backup::new(&source, &mut target_mut)
            .map_err(|e| format!("restore init: {e}"))?;
        backup
            .run_to_completion(128, std::time::Duration::from_millis(5), None)
            .map_err(|e| format!("restore run: {e}"))?;
    }
    drop(target);
    let mut conn = open_configured(db_path)?;
    super::initialize_schema(&conn).map_err(|e| e.to_string())?;
    super::run_pending_migrations_quiet(&mut conn);
    super::records::ensure_authoritative_schema(&conn).map_err(|e| e.to_string())?;
    // New restore epoch: every cursor/alias minted before is now foreign.
    let backup_epoch = super::records::brain_epochs(&conn).1;
    let new_epoch = format!("restore-{}", chrono::Utc::now().format("%Y%m%dT%H%M%SZ"));
    conn.execute(
        "UPDATE brain_meta SET restore_epoch = ?1 WHERE singleton = 1",
        params![new_epoch],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "INSERT OR REPLACE INTO config (key, value) VALUES ('restore_epoch', ?1)",
        params![new_epoch],
    )
    .ok();
    let aliases_expired = conn.execute("DELETE FROM view_aliases", []).unwrap_or(0) as i64
        + conn.execute("DELETE FROM view_receipts", []).unwrap_or(0) as i64;
    // Erasure floor first: the home ledger re-applies every erasure the
    // backup predates, so a restore can never resurrect an erased record. A
    // backup without the ledger while the home has one is quarantined.
    let reconciliation = super::erasure::reconcile_after_restore(&conn, home)?;
    let erasure_ledger_present =
        count(&conn, "SELECT COUNT(*) FROM erasures") > 0 || reconciliation.ledger_entries == 0;
    // Disposable projections are rebuilt, never trusted from the backup.
    let mut projections_rebuilt = 0usize;
    if super::rebuild_fts(&conn).is_ok() {
        projections_rebuilt += 1;
    }
    if crate::clockwork::rebuild_clock_projections(&conn, 256).is_ok() {
        projections_rebuilt += 1;
    }
    let integrity_ok = super::verify_integrity(&conn).unwrap_or(false);
    let records = count(&conn, "SELECT COUNT(*) FROM records");
    let decisions = count(&conn, "SELECT COUNT(*) FROM decisions");
    let memories = count(&conn, "SELECT COUNT(*) FROM memories");
    // Sample exact reads: every legacy address must resolve to a head with a body.
    let sample_reads_ok = conn
        .prepare("SELECT a.record_id FROM addresses a WHERE a.scheme='legacy' ORDER BY a.record_id LIMIT 25")
        .and_then(|mut stmt| {
            let ids: Vec<String> = stmt.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<_, _>>()?;
            for id in ids {
                let heads = super::records::heads(&conn, &id)?;
                if heads.is_empty() || super::records::revision_body(&conn, &heads[0])?.is_none() {
                    return Ok(false);
                }
            }
            Ok(true)
        })
        .unwrap_or(false);
    let report = RestoreReport {
        new_restore_epoch: new_epoch,
        previous_restore_epoch: if backup_epoch != previous_epoch {
            format!("{previous_epoch} (live) / {backup_epoch} (backup)")
        } else {
            previous_epoch
        },
        integrity_ok,
        sample_reads_ok,
        records,
        decisions,
        memories,
        aliases_expired,
        projections_rebuilt,
        erasure_ledger_present,
        erasures_reapplied: reconciliation.reapplied,
        quarantined: reconciliation.quarantined,
        report_path: home.join(LAST_VERIFIED_RESTORE),
    };
    std::fs::write(
        &report.report_path,
        serde_json::to_string_pretty(&report.to_json()).unwrap_or_default(),
    )
    .map_err(|e| format!("write report: {e}"))?;
    Ok(report)
}

pub fn last_verified_restore(home: &Path) -> Option<Value> {
    use std::io::Read;
    let file = crate::auth::open_nofollow(&home.join(LAST_VERIFIED_RESTORE)).ok()?;
    let mut raw = String::new();
    file.take(MAX_RESTORE_REPORT_BYTES + 1)
        .read_to_string(&mut raw)
        .ok()?;
    if raw.len() as u64 > MAX_RESTORE_REPORT_BYTES {
        return None;
    }
    serde_json::from_str(&raw).ok()
}
