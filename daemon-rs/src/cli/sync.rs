use super::common::{open_cli_connection, parse_flag_usize, parse_flag_value, validate_cli_options, validate_cli_options_or_exit};
use crate::auth;
use crate::db;
use crate::export_data;
use fs2::FileExt;
use serde_json::Value;
use std::collections::HashSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;
pub(crate) fn run_sync_cli(paths: &auth::CortexPaths, args: &[String]) {
    let Some(command) = args.first().map(|value| value.as_str()) else {
        eprintln!("Usage: cortex sync <export|import|watch> [options]");
        std::process::exit(1);
    };
    validate_sync_cli_options_or_exit(command, &args[1..]);
    let _sync_lock = match acquire_sync_lock(paths) {
        Ok(lock) => lock,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    match command {
        "export" => run_sync_export_cli(paths, &args[1..]),
        "import" => run_sync_import_cli(paths, &args[1..]),
        "watch" => run_sync_watch_cli(paths, &args[1..]),
        _ => {
            eprintln!("Usage: cortex sync <export|import|watch> [options]");
            std::process::exit(1);
        }
    }
}
fn validate_sync_cli_options_or_exit(command: &str, args: &[String]) {
    let result = match command {
        "export" => validate_cli_options(args, &["--out", "--since", "--cursor-file"], &[]),
        "import" => validate_cli_options(args, &["--file", "--user", "--visibility"], &[]),
        "watch" => {
            validate_cli_options(args, &["--dir", "--interval-seconds", "--user", "--visibility", "--since", "--cursor-file"], &["--once"])
        }
        _ => Err("Usage: cortex sync <export|import|watch> [options]".to_string()),
    };
    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
pub(crate) fn run_export_cli(paths: &auth::CortexPaths, args: &[String]) {
    validate_cli_options_or_exit(args, &["--format", "--out"], &[]);
    let mut format = "json".to_string();
    let mut out_path: Option<String> = None;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                if let Some(v) = args.get(i + 1) {
                    format = v.to_string();
                    i += 1;
                }
            }
            "--out" => {
                if let Some(v) = args.get(i + 1) {
                    out_path = Some(v.to_string());
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let Some(export_format) = export_data::ExportFormat::parse(&format) else {
        eprintln!("Usage: cortex export --format json|sql [--out <path>]");
        std::process::exit(1);
    };
    let mut conn = match open_cli_connection(&paths.db) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    let output = match export_snapshot_text(&mut conn, export_format) {
        Ok(output) => output,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    if let Some(path) = out_path {
        if let Err(e) = write_atomic_text_file(Path::new(&path), &output) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        eprintln!("Exported to {path}");
    } else {
        println!("{output}");
    }
}
fn run_sync_export_cli(paths: &auth::CortexPaths, args: &[String]) {
    validate_cli_options_or_exit(args, &["--out", "--since", "--cursor-file"], &[]);
    let out_path = parse_flag_value(args, "--out");
    let since_override = parse_flag_value(args, "--since");
    if let Some(since) = since_override.as_deref() {
        if chrono::DateTime::parse_from_rfc3339(since).is_err() {
            eprintln!("Invalid --since value '{since}'. Use RFC3339 (for example 2026-04-19T00:00:00Z).");
            std::process::exit(1);
        }
    }
    let cursor_file = parse_flag_value(args, "--cursor-file").map(PathBuf::from);
    let since = resolve_sync_since(since_override.as_deref(), cursor_file.as_deref());
    let mut conn = match open_cli_connection(&paths.db) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    let value = match export_changeset_snapshot_value(&mut conn, since.as_deref()) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    let output = serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string());
    if let Some(path) = out_path {
        if let Err(e) = write_atomic_text_file(Path::new(&path), &output) {
            eprintln!("{e}");
            std::process::exit(1);
        }
        eprintln!("Sync export written to {path}");
    } else {
        println!("{output}");
    }
    if let Some(cursor_path) = cursor_file {
        if let Some(cursor) = value.get("cursor").and_then(serde_json::Value::as_str) {
            if let Err(err) = write_sync_cursor_file(&cursor_path, cursor) {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
    }
}
pub(crate) fn run_import_cli(paths: &auth::CortexPaths, args: &[String]) {
    let parsed = parse_import_cli_args(args, "Usage: cortex import --file <path> [--user <username>] [--visibility private|team|shared]");
    let parsed = match parsed {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    let counts = match import_payload_from_file(paths, &parsed, "import-cli", ImportPayloadExpectation::GeneralJson) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    println!("{{\"imported\":{{\"memories\":{},\"decisions\":{}}}}}", counts.memories, counts.decisions);
}
fn run_sync_import_cli(paths: &auth::CortexPaths, args: &[String]) {
    let parsed =
        parse_import_cli_args(args, "Usage: cortex sync import --file <path> [--user <username>] [--visibility private|team|shared]");
    let parsed = match parsed {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    let counts = match import_payload_from_file(paths, &parsed, "sync-import-cli", ImportPayloadExpectation::SyncChangeset) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    println!("{{\"imported\":{{\"memories\":{},\"decisions\":{}}}}}", counts.memories, counts.decisions);
}
fn run_sync_watch_cli(paths: &auth::CortexPaths, args: &[String]) {
    validate_cli_options_or_exit(args, &["--dir", "--interval-seconds", "--user", "--visibility", "--since", "--cursor-file"], &["--once"]);
    let Some(dir_raw) = parse_flag_value(args, "--dir") else {
        eprintln!(
"Usage: cortex sync watch --dir <path> [--interval-seconds <n>] [--once] [--user <username>] [--visibility private|team|shared] [--since <iso>] [--cursor-file <path>]"
);
        std::process::exit(1);
    };
    let watch_dir = PathBuf::from(dir_raw);
    if !watch_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&watch_dir) {
            eprintln!("Failed to create sync watch directory {}: {e}", watch_dir.display());
            std::process::exit(1);
        }
    }
    if !watch_dir.is_dir() {
        eprintln!("Sync watch path must be a directory: {}", watch_dir.display());
        std::process::exit(1);
    }
    let interval_seconds = match parse_flag_usize(args, "--interval-seconds") {
        Ok(Some(value)) => value as u64,
        Ok(None) => 15,
        Err(err) => {
            eprintln!("Invalid --interval-seconds: {err}");
            std::process::exit(1);
        }
    };
    let once = args.iter().any(|arg| arg == "--once");
    let username = parse_flag_value(args, "--user");
    let visibility = parse_flag_value(args, "--visibility").unwrap_or_else(|| "private".to_string());
    if !matches!(visibility.as_str(), "private" | "team" | "shared") {
        eprintln!("Invalid --visibility value '{visibility}'. Use private|team|shared.");
        std::process::exit(1);
    }
    let mut bootstrap_since = parse_flag_value(args, "--since");
    if let Some(since) = bootstrap_since.as_deref() {
        if chrono::DateTime::parse_from_rfc3339(since).is_err() {
            eprintln!("Invalid --since value '{since}'. Use RFC3339 (for example 2026-04-19T00:00:00Z).");
            std::process::exit(1);
        }
    }
    let state_id = sync_watch_state_id(&watch_dir);
    let state_root = paths.home.join("runtime").join("sync-watch");
    let seen_file = state_root.join(format!("{state_id}.seen"));
    let default_cursor = state_root.join(format!("{state_id}.cursor"));
    let cursor_file = parse_flag_value(args, "--cursor-file").map(PathBuf::from).unwrap_or(default_cursor);
    let local_site_id = match ensure_sync_site_id(paths) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    let mut seen = match load_sync_seen_set(&seen_file) {
        Ok(value) => value,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    loop {
        let candidates = match collect_sync_watch_import_candidates(&watch_dir, &local_site_id) {
            Ok(value) => value,
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        };
        let mut seen_dirty = false;
        for candidate in candidates {
            let Some(name) = candidate.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if seen.contains(name) {
                continue;
            }
            let import_options = ImportCliArgs {
                file_path: candidate.clone(),
                username: username.clone(),
                visibility: visibility.clone(),
            };
            match import_payload_from_file(paths, &import_options, "sync-watch-import", ImportPayloadExpectation::SyncChangeset) {
                Ok(counts) => {
                    eprintln!(
                        "[sync watch] imported {} (memories={}, decisions={})",
                        candidate.display(),
                        counts.memories,
                        counts.decisions
                    );
                    seen.insert(name.to_string());
                    seen_dirty = true;
                }
                Err(err) => {
                    eprintln!("[sync watch] import skipped for {}: {}", candidate.display(), err);
                }
            }
        }
        if seen_dirty {
            if let Err(err) = write_sync_seen_set(&seen_file, &seen) {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
        let since = read_sync_cursor_file(&cursor_file).or_else(|| bootstrap_since.take());
        let mut conn = match open_cli_connection(&paths.db) {
            Ok(conn) => conn,
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        };
        let changeset = match export_changeset_snapshot_value(&mut conn, since.as_deref()) {
            Ok(value) => value,
            Err(err) => {
                eprintln!("{err}");
                std::process::exit(1);
            }
        };
        let memories_count = changeset.get("memories_count").and_then(serde_json::Value::as_u64).unwrap_or(0);
        let decisions_count = changeset.get("decisions_count").and_then(serde_json::Value::as_u64).unwrap_or(0);
        let total = memories_count + decisions_count;
        if total > 0 {
            let filename = format!("changeset-{}-{}.json", local_site_id, chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ"));
            let out_path = watch_dir.join(filename);
            let output = serde_json::to_string_pretty(&changeset).unwrap_or_else(|_| "{}".to_string());
            if let Err(err) = write_atomic_text_file(&out_path, &output) {
                eprintln!("{err}");
                std::process::exit(1);
            }
            eprintln!("[sync watch] exported {} (memories={}, decisions={})", out_path.display(), memories_count, decisions_count);
        }
        if let Some(cursor) = changeset.get("cursor").and_then(serde_json::Value::as_str) {
            if let Err(err) = write_sync_cursor_file(&cursor_file, cursor) {
                eprintln!("{err}");
                std::process::exit(1);
            }
        }
        if once {
            break;
        }
        std::thread::sleep(Duration::from_secs(interval_seconds.max(1)));
    }
}
#[derive(Debug, Clone)]
struct ImportCliArgs {
    file_path: PathBuf,
    username: Option<String>,
    visibility: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportPayloadExpectation {
    GeneralJson,
    SyncChangeset,
}
fn parse_import_cli_args(args: &[String], usage: &str) -> Result<ImportCliArgs, String> {
    validate_cli_options(args, &["--file", "--user", "--visibility"], &[])?;
    let mut file_path: Option<String> = None;
    let mut username: Option<String> = None;
    let mut visibility = "private".to_string();
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--file" => {
                if let Some(v) = args.get(i + 1) {
                    file_path = Some(v.to_string());
                    i += 1;
                }
            }
            "--user" => {
                if let Some(v) = args.get(i + 1) {
                    username = Some(v.to_string());
                    i += 1;
                }
            }
            "--visibility" => {
                if let Some(v) = args.get(i + 1) {
                    visibility = v.to_string();
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    let Some(file_path) = file_path else {
        return Err(usage.to_string());
    };
    if !matches!(visibility.as_str(), "private" | "team" | "shared") {
        return Err(format!("Invalid --visibility value '{visibility}'. Use private|team|shared."));
    }
    Ok(ImportCliArgs { file_path: PathBuf::from(file_path), username, visibility })
}
fn import_payload_from_file(
    paths: &auth::CortexPaths, parsed: &ImportCliArgs, source_agent_fallback: &str, expectation: ImportPayloadExpectation,
) -> Result<export_data::ImportCounts, String> {
    let file_display = parsed.file_path.display().to_string();
    let raw = std::fs::read_to_string(&parsed.file_path).map_err(|e| format!("Cannot read import file {file_display}: {e}"))?;
    let raw_value: Value = serde_json::from_str(&raw).map_err(|e| format!("Import file is not valid JSON: {e}"))?;
    validate_import_payload_metadata(&raw_value, expectation)?;
    let payload: export_data::ImportPayload =
        serde_json::from_value(raw_value).map_err(|e| format!("Import file has unsupported record shape: {e}"))?;
    let mut conn = open_cli_connection(&paths.db)?;
    let team_mode = db::current_mode(&conn) == "team";
    if parsed.username.is_some() && !team_mode {
        return Err("--user import requires team mode. Run: cortex setup --team".to_string());
    }
    let owner_id = if team_mode {
        if let Some(user) = parsed.username.as_ref() {
            match conn.query_row("SELECT id FROM users WHERE username = ?1", rusqlite::params![user.clone()], |row| row.get::<_, i64>(0)) {
                Ok(id) => Some(id),
                Err(_) => {
                    return Err(format!("Unknown user '{user}'. Create the user before import."));
                }
            }
        } else {
            conn.query_row("SELECT value FROM config WHERE key = 'owner_user_id' LIMIT 1", [], |row| row.get::<_, String>(0))
                .ok()
                .and_then(|v| v.parse::<i64>().ok())
                .or_else(|| {
                    conn.query_row("SELECT id FROM users ORDER BY CASE role WHEN 'owner' THEN 0 ELSE 1 END, id ASC LIMIT 1", [], |row| {
                        row.get::<_, i64>(0)
                    })
                    .ok()
                })
        }
    } else {
        None
    };
    if team_mode && owner_id.is_none() {
        return Err("Team mode import requires a target owner. Run `cortex setup --team` first.".to_string());
    }
    let options = export_data::ImportOptions {
        owner_id,
        visibility: if team_mode { Some(parsed.visibility.clone()) } else { None },
        source_agent_fallback: source_agent_fallback.to_string(),
    };
    export_data::import_payload(&mut conn, &payload, &options)
}
fn validate_import_payload_metadata(value: &Value, expectation: ImportPayloadExpectation) -> Result<(), String> {
    let Some(obj) = value.as_object() else {
        return Err("Import file must be a JSON object.".to_string());
    };
    let mode = obj.get("mode").and_then(Value::as_str);
    match obj.get("version") {
        Some(version) if version.as_u64() == Some(1) => {}
        Some(version) => {
            return Err(format!("Import file has unsupported version marker {version}; expected 1."));
        }
        None if expectation == ImportPayloadExpectation::SyncChangeset || mode.is_some() => {
            return Err("Import file is missing required version marker.".to_string());
        }
        None => {}
    }
    match expectation {
        ImportPayloadExpectation::GeneralJson => match mode {
            Some("changeset") | None => {}
            Some("page") => {
                return Err("Import file is a paged export fragment; import a full export or sync changeset.".to_string());
            }
            Some(other) => return Err(format!("Import file has unsupported mode '{other}'.")),
        },
        ImportPayloadExpectation::SyncChangeset => {
            if mode != Some("changeset") {
                return Err("Sync import requires a changeset export with mode=\"changeset\".".to_string());
            }
            let Some(cursor) = obj.get("cursor").and_then(Value::as_str) else {
                return Err("Sync changeset is missing cursor version marker.".to_string());
            };
            validate_rfc3339_marker("cursor", cursor)?;
        }
    }
    if let Some(exported_at) = obj.get("exported_at").and_then(Value::as_str) {
        validate_rfc3339_marker("exported_at", exported_at)?;
    }
    let count_markers_required = expectation == ImportPayloadExpectation::SyncChangeset;
    validate_import_count_marker(value, "memories", "memories_count", count_markers_required)?;
    validate_import_count_marker(value, "decisions", "decisions_count", count_markers_required)?;
    Ok(())
}
fn validate_rfc3339_marker(label: &str, value: &str) -> Result<(), String> {
    if chrono::DateTime::parse_from_rfc3339(value).is_ok() {
        Ok(())
    } else {
        Err(format!("Import file has invalid {label} marker '{value}'; expected RFC3339."))
    }
}
fn validate_import_count_marker(value: &Value, rows_key: &str, count_key: &str, required: bool) -> Result<(), String> {
    let Some(expected_value) = value.get(count_key) else {
        if required {
            return Err(format!("Sync changeset is missing required {count_key} marker."));
        }
        return Ok(());
    };
    let Some(expected_u64) = expected_value.as_u64() else {
        return Err(format!("Import file has non-numeric {count_key} marker."));
    };
    let expected = usize::try_from(expected_u64).map_err(|_| format!("Import file has out-of-range {count_key} marker."))?;
    let actual = value.get(rows_key).and_then(Value::as_array).map(|rows| rows.len()).unwrap_or(0);
    if actual == expected {
        Ok(())
    } else {
        Err(format!("Import file {count_key} marker ({expected}) does not match {rows_key} rows ({actual})."))
    }
}
fn resolve_sync_since(override_since: Option<&str>, cursor_file: Option<&Path>) -> Option<String> {
    override_since.map(str::to_string).or_else(|| cursor_file.and_then(read_sync_cursor_file))
}
fn read_sync_cursor_file(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().and_then(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}
fn write_sync_cursor_file(path: &Path, cursor: &str) -> Result<(), String> {
    write_atomic_text_file(path, &format!("{cursor}\n")).map_err(|e| format!("Failed to write cursor file {}: {e}", path.display()))
}
fn ensure_sync_site_id(paths: &auth::CortexPaths) -> Result<String, String> {
    let site_id_path = paths.home.join("site_id");
    if let Ok(existing) = std::fs::read_to_string(&site_id_path) {
        let candidate = sanitize_sync_site_id(existing.trim());
        if !candidate.is_empty() {
            return Ok(candidate);
        }
    }
    if let Some(parent) = site_id_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create sync site-id directory {}: {e}", parent.display()))?;
    }
    let created = uuid::Uuid::new_v4().to_string();
    write_atomic_text_file(&site_id_path, &format!("{created}\n"))
        .map_err(|e| format!("Failed to persist sync site-id {}: {e}", site_id_path.display()))?;
    Ok(created)
}
fn sanitize_sync_site_id(raw: &str) -> String {
    raw.chars().filter(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_')).collect()
}
fn sync_watch_state_id(watch_dir: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    watch_dir.to_string_lossy().to_string().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}
fn is_sync_changeset_file_name(name: &str) -> bool {
    name.starts_with("changeset-") && name.ends_with(".json")
}
fn collect_sync_watch_import_candidates(watch_dir: &Path, local_site_id: &str) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let local_prefix = format!("changeset-{local_site_id}-");
    let entries = std::fs::read_dir(watch_dir).map_err(|e| format!("Failed to read sync watch directory {}: {e}", watch_dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !is_sync_changeset_file_name(name) {
            continue;
        }
        if name.starts_with(&local_prefix) {
            continue;
        }
        files.push(path);
    }
    files.sort();
    Ok(files)
}
fn load_sync_seen_set(path: &Path) -> Result<HashSet<String>, String> {
    let mut seen = HashSet::new();
    if !path.exists() {
        return Ok(seen);
    }
    let raw = std::fs::read_to_string(path).map_err(|e| format!("Failed to read sync watch state {}: {e}", path.display()))?;
    for line in raw.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            seen.insert(trimmed.to_string());
        }
    }
    Ok(seen)
}
fn write_sync_seen_set(path: &Path, seen: &HashSet<String>) -> Result<(), String> {
    let mut rows: Vec<&str> = seen.iter().map(String::as_str).collect();
    rows.sort_unstable();
    write_atomic_text_file(path, &rows.join("\n")).map_err(|e| format!("Failed to write sync watch state {}: {e}", path.display()))
}
fn acquire_sync_lock(paths: &auth::CortexPaths) -> Result<std::fs::File, String> {
    let lock_path = paths.home.join("sync.lock");
    std::fs::create_dir_all(&paths.home).map_err(|e| format!("Failed to create sync lock directory {}: {e}", paths.home.display()))?;
    let lock_file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|e| format!("Failed to open sync lock {}: {e}", lock_path.display()))?;
    lock_file
        .lock_exclusive()
        .map_err(|e| format!("Failed to acquire sync lock {}: {e}", lock_path.display()))?;
    Ok(lock_file)
}
fn export_snapshot_text(conn: &mut rusqlite::Connection, format: export_data::ExportFormat) -> Result<String, String> {
    let tx = conn.transaction().map_err(|e| format!("Failed to start export snapshot transaction: {e}"))?;
    let output = match format {
        export_data::ExportFormat::Json => {
            let value = export_data::export_json_value(&tx);
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
        }
        export_data::ExportFormat::Sql => export_data::export_sql_text(&tx),
    };
    tx.commit().map_err(|e| format!("Failed to finish export snapshot transaction: {e}"))?;
    Ok(output)
}
fn export_changeset_snapshot_value(conn: &mut rusqlite::Connection, since: Option<&str>) -> Result<Value, String> {
    let tx = conn.transaction().map_err(|e| format!("Failed to start sync export snapshot transaction: {e}"))?;
    let value = export_data::export_json_changeset_value(&tx, since);
    tx.commit().map_err(|e| format!("Failed to finish sync export snapshot transaction: {e}"))?;
    Ok(value)
}
fn write_atomic_text_file(path: &Path, contents: &str) -> Result<(), String> {
    let parent = writable_parent_dir(path)?;
    std::fs::create_dir_all(parent).map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
    let mut tmp =
        tempfile::NamedTempFile::new_in(parent).map_err(|e| format!("Failed to create temp file in {}: {e}", parent.display()))?;
    tmp.write_all(contents.as_bytes())
        .map_err(|e| format!("Failed to write temp file for {}: {e}", path.display()))?;
    tmp.as_file()
        .sync_all()
        .map_err(|e| format!("Failed to flush temp file for {}: {e}", path.display()))?;
    tmp.persist(path)
        .map_err(|e| format!("Failed to replace {} atomically: {}", path.display(), e.error))?;
    sync_parent_dir(parent).map_err(|e| format!("Failed to flush directory {}: {e}", parent.display()))?;
    Ok(())
}
fn writable_parent_dir(path: &Path) -> Result<&Path, String> {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Ok(parent),
        _ => Ok(Path::new(".")),
    }
}
#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> std::io::Result<()> {
    std::fs::File::open(parent)?.sync_all()
}
#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> std::io::Result<()> {
    Ok(())
}
