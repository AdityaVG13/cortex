use super::*;
use crate::aging;
use crate::auth;
use crate::cli::cleanup::{cleanup_backup_retention, cleanup_bridge_backups, cleanup_expired_rows, create_backup, rotate_startup_logs, should_backup};
use crate::cli::common::{parse_env_u64, parse_env_usize, parse_truthy_flag};
use crate::compaction;
use crate::crystallize;
use crate::daemon_lifecycle;
use crate::db;
use crate::embeddings;
use crate::indexer;
use crate::server;
use crate::state;
use daemon_lifecycle::daemon_healthy;
use std::time::Duration;
pub async fn run_daemon(paths: auth::CortexPaths, extra_shutdown: impl std::future::Future<Output = ()> + Send + 'static) {
    let _daemon_lock = match acquire_runtime_lock(&paths) {
        Ok(lock) => lock,
        Err(err) => {
            if daemon_healthy(&paths).await {
                eprintln!("[cortex] Daemon already healthy on port {}; exiting cleanly.", paths.port);
                return;
            }
            eprintln!("[cortex] FATAL: {err}");
            eprintln!("[cortex] Reuse the existing daemon instead of launching a second `cortex serve`.");
            std::process::exit(1);
        }
    };
    let db_path = paths.db.clone();
    eprintln!("[cortex] Starting Cortex v{} (Rust)...", env!("CARGO_PKG_VERSION"));
    eprintln!("[cortex] DB: {}", db_path.display());
    crate::install_daemon_panic_hook(&paths);
    let daemon_owner = daemon_owner_tag_from_env();
    let parent_pid = spawn_parent_pid_from_env();
    let parent_start_time = spawn_parent_start_time_from_env();
    let owner_token = daemon_owner_token_from_env();
    if let Err(reason) = validate_spawned_owner_runtime_claim(&paths, daemon_owner.as_deref(), parent_pid, parent_start_time, owner_token.as_deref()) {
        eprintln!("[cortex] FATAL: invalid spawned owner claim ({reason}); refusing startup");
        std::process::exit(1);
    }
    if let Err(reason) = startup_single_daemon_preflight(&paths).await {
        eprintln!("[cortex] FATAL: {reason}");
        std::process::exit(1);
    }
    let (state, shutdown_rx) = match state::initialize(&paths, true) {
        Ok(initialized) => initialized,
        Err(err) => {
            eprintln!("[cortex] FATAL: failed to initialize state: {err}");
            std::process::exit(1);
        }
    };
    if should_watch_spawn_parent(daemon_owner.as_deref()) {
        if let (Some(parent_pid), Some(parent_start_time)) = (parent_pid, parent_start_time) {
            let _watcher = spawn_parent_orphan_watch_task(
                state.shutdown_tx.clone(),
                parent_pid,
                parent_start_time,
                Duration::from_secs(ORPHAN_WATCH_INTERVAL_SECS),
                process_pid_identity_matches,
            );
        }
    }
    let startup_schedule = startup_schedule(daemon_owner.as_deref());
    let background_lock_wait = background_db_lock_max_wait();
    eprintln!(
        "[cortex] Startup scheduling: index={}s, aging={}s, embeddings={}s, crystallize={}s, storage_governor={}s",
        startup_schedule.index.as_secs(),
        startup_schedule.aging.as_secs(),
        startup_schedule.embed.as_secs(),
        startup_schedule.crystallize.as_secs(),
        startup_schedule.storage_governor_initial.as_secs()
    );
    if let Some(parent) = paths.pid.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&paths.pid, std::process::id().to_string()).ok();
    let token_path = paths.token.clone();
    let pid_path = paths.pid.clone();
    let backup_dir = paths.home.join("backups");
    eprintln!("[cortex] Auth token at {}", token_path.display());
    eprintln!("[cortex] PID {} written to {}", std::process::id(), pid_path.display());
    let cleaned_backups = cleanup_backup_retention(&backup_dir);
    eprintln!("[cortex] Cleaned {cleaned_backups} old backups, kept {}", crate::cli::cleanup::BACKUP_RETENTION_COUNT);
    let rotated_logs = rotate_startup_logs(&paths.home);
    if rotated_logs > 0 {
        eprintln!("[cortex] Rotated {rotated_logs} oversized log files");
    }
    {
        let conn = state.db.lock().await;
        if let Err(e) = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);") {
            eprintln!("[cortex] WAL recovery warning: {e}");
        } else {
            eprintln!("[cortex] WAL recovery complete");
        }
    }
    let schema_version = {
        let conn = state.db.lock().await;
        let applied = db::run_pending_migrations(&conn);
        if applied > 0 {
            eprintln!("[cortex] Applied {applied} schema migrations");
        }
        db::current_schema_user_version(&conn).unwrap_or(0)
    };
    let _ = cleanup_bridge_backups(&paths.home, schema_version);
    {
        let db_index = state.db.clone();
        let home = state.home.clone();
        let owner_id = state.default_owner_id;
        let startup_delay = startup_schedule.index;
        let lock_wait = background_lock_wait;
        tokio::spawn(async move {
            if startup_delay > Duration::from_secs(0) {
                tokio::time::sleep(startup_delay).await;
            }
            let started = std::time::Instant::now();
            if let Some(conn) = acquire_background_db_lock(&db_index, "startup indexing", lock_wait).await {
                let indexed = indexer::index_all(&conn, &home, owner_id);
                let decayed = indexer::decay_pass(&conn);
                eprintln!("[cortex] Startup indexing complete: indexed {indexed}, decayed {decayed} scores in {}ms", started.elapsed().as_millis());
            }
        });
    }
    if let Some(engine) = state.embedding_engine.clone() {
        let db = state.db.clone();
        let batch_size = parse_env_usize("CORTEX_EMBED_BACKFILL_BATCH_SIZE", DEFAULT_EMBED_BACKFILL_BATCH_SIZE).clamp(1, 10_000);
        let max_batches_per_pass = parse_env_usize("CORTEX_EMBED_BACKFILL_MAX_BATCHES_PER_PASS", DEFAULT_EMBED_BACKFILL_MAX_BATCHES_PER_PASS).clamp(1, 1000);
        let interval_secs = parse_env_u64("CORTEX_EMBED_BACKFILL_INTERVAL_SECS", DEFAULT_EMBED_BACKFILL_INTERVAL_SECS).clamp(5, 86_400);
        let drain_on_startup = std::env::var(EMBED_BACKFILL_DRAIN_ON_STARTUP_ENV).ok().map(|value| parse_truthy_flag(&value)).unwrap_or(false);
        let startup_drain_max_batches =
            parse_env_usize(EMBED_BACKFILL_STARTUP_DRAIN_MAX_BATCHES_ENV, DEFAULT_EMBED_BACKFILL_STARTUP_DRAIN_MAX_BATCHES).clamp(1, 10_000);
        let startup_delay = startup_schedule.embed;
        let lock_wait = background_lock_wait;
        let startup_max_batches_per_pass = if startup_delay > Duration::from_secs(0) { max_batches_per_pass.min(2) } else { max_batches_per_pass };
        tokio::spawn(async move {
            if startup_delay > Duration::from_secs(0) {
                tokio::time::sleep(startup_delay).await;
            }
            let startup_pass = build_embeddings_async(engine.clone(), &db, batch_size, startup_max_batches_per_pass, lock_wait).await;
            if startup_pass.queued_total > 0 && !startup_pass.exhausted {
                if drain_on_startup {
                    let drain_pass = build_embeddings_async(engine.clone(), &db, batch_size, startup_drain_max_batches, lock_wait).await;
                    if drain_pass.exhausted {
                        eprintln!("[embeddings] Startup drain completed backlog in {} batches", drain_pass.passes_ran);
                    } else if drain_pass.queued_total > 0 {
                        eprintln!(
                            "[embeddings] Startup drain reached cap with backlog still pending (passes={}, queued={})",
                            drain_pass.passes_ran, drain_pass.queued_total
                        );
                    }
                } else {
                    eprintln!("[embeddings] Startup pass left backlog pending; set {}=1 to run a one-time extended drain", EMBED_BACKFILL_DRAIN_ON_STARTUP_ENV);
                }
            }
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
            interval.tick().await;
            loop {
                interval.tick().await;
                build_embeddings_async(engine.clone(), &db, batch_size, max_batches_per_pass, lock_wait).await;
            }
        });
    } else {
        let models_dir = paths.models.clone();
        tokio::spawn(async move {
            if let Some(dir) = embeddings::ensure_model_downloaded_in(&models_dir).await {
                eprintln!("[embeddings] Model ready at {} -- restart to activate", dir.display());
            }
        });
    }
    {
        let db_wal = state.db.clone();
        let db_path = db_path.clone();
        let home_dir = paths.home.clone();
        let lock_wait = background_lock_wait.min(Duration::from_millis(750));
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
            interval.tick().await;
            loop {
                interval.tick().await;
                let Some(conn) = acquire_background_db_lock(&db_wal, "wal checkpoint", lock_wait).await else {
                    continue;
                };
                {
                    db::checkpoint_wal_best_effort(&conn);
                }
                let backup_dir = home_dir.join("backups");
                if should_backup(&backup_dir) {
                    if let Err(e) = create_backup(&db_path, &backup_dir) {
                        eprintln!("[cortex] Backup failed: {e}");
                    }
                }
            }
        });
    }
    {
        let db_qc = state.db_read.clone();
        let db_corrupted_flag = state.db_corrupted.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30 * 60));
            interval.tick().await;
            loop {
                interval.tick().await;
                let conn = db_qc.lock().await;
                if db::quick_check(&conn) {
                    db_corrupted_flag.store(false, std::sync::atomic::Ordering::SeqCst);
                } else {
                    eprintln!(
                        "[cortex] WARNING: runtime PRAGMA quick_check FAILED -- \
                         database may be corrupted. Restart the daemon to trigger auto-repair. \
                         /health endpoint now shows degraded=true, db_corrupted=true."
                    );
                    db_corrupted_flag.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
        });
    }
    {
        let db_aging = state.db.clone();
        let startup_delay = startup_schedule.aging;
        let lock_wait = background_lock_wait;
        tokio::spawn(async move {
            if startup_delay > Duration::from_secs(0) {
                tokio::time::sleep(startup_delay).await;
            }
            if let Some(conn) = acquire_background_db_lock(&db_aging, "initial aging pass", lock_wait).await {
                let (compressed, archived) = aging::run_aging_pass(&conn);
                if compressed > 0 || archived > 0 {
                    eprintln!("[cortex] Initial aging: {compressed} compressed, {archived} archived");
                }
                cleanup_expired_rows(&conn, "Initial expired cleanup");
            }
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(6 * 3600));
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Some(conn) = acquire_background_db_lock(&db_aging, "aging pass", lock_wait).await {
                    aging::run_aging_pass(&conn);
                    cleanup_expired_rows(&conn, "Expired cleanup");
                }
            }
        });
    }
    {
        let db_compaction = state.db.clone();
        let startup_delay = startup_schedule.storage_governor_initial;
        let lock_wait = background_lock_wait;
        tokio::spawn(async move {
            if startup_delay > Duration::from_secs(0) {
                tokio::time::sleep(startup_delay).await;
            }
            for pass in 0..STARTUP_STORAGE_GOVERNOR_CATCHUP_PASSES {
                if let Some(conn) = acquire_background_db_lock(&db_compaction, "startup storage governor", lock_wait).await {
                    let ran = compaction::run_compaction_governor_startup(&conn).is_some();
                    if !ran {
                        break;
                    }
                }
                if pass + 1 < STARTUP_STORAGE_GOVERNOR_CATCHUP_PASSES {
                    tokio::time::sleep(Duration::from_secs(STARTUP_STORAGE_GOVERNOR_CATCHUP_INTERVAL_SECS)).await;
                }
            }
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30 * 60));
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Some(conn) = acquire_background_db_lock(&db_compaction, "storage governor", lock_wait).await {
                    let _ = compaction::run_compaction_governor(&conn);
                }
            }
        });
    }
    {
        let db_crystal = state.db.clone();
        let engine_crystal = state.embedding_engine.clone();
        let crystal_owner_id = state.default_owner_id;
        let brain_crystal: crystallize::BrainFiringSender = Some(state.brain_firing.clone());
        let initial_delay = startup_schedule.crystallize;
        let lock_wait = background_lock_wait;
        tokio::spawn(async move {
            tokio::time::sleep(initial_delay).await;
            if let Some(conn) = acquire_background_db_lock(&db_crystal, "initial crystallization", lock_wait).await {
                let result = crystallize::run_crystallize_pass_with_brain(&conn, engine_crystal.as_deref(), crystal_owner_id, &brain_crystal);
                if result.crystals_created > 0 || result.crystals_updated > 0 {
                    eprintln!("[cortex] Initial crystallization: {} created, {} updated", result.crystals_created, result.crystals_updated);
                }
            }
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(2 * 3600));
            interval.tick().await;
            loop {
                interval.tick().await;
                if let Some(conn) = acquire_background_db_lock(&db_crystal, "crystallization pass", lock_wait).await {
                    crystallize::run_crystallize_pass_with_brain(&conn, engine_crystal.as_deref(), crystal_owner_id, &brain_crystal);
                }
            }
        });
    }
    {
        let rl = state.rate_limiter.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            interval.tick().await;
            loop {
                interval.tick().await;
                rl.cleanup().await;
            }
        });
    }
    let idle_shutdown_secs = std::env::var(IDLE_SHUTDOWN_SECS_ENV).ok().and_then(|raw| raw.trim().parse::<u64>().ok()).unwrap_or(0);
    let idle_min_uptime_secs = parse_env_u64(IDLE_SHUTDOWN_MIN_UPTIME_SECS_ENV, DEFAULT_IDLE_SHUTDOWN_MIN_UPTIME_SECS).clamp(1, 86_400);
    if idle_shutdown_secs > 0 {
        eprintln!("[cortex] Idle shutdown enabled (timeout={}s, min_uptime={}s)", idle_shutdown_secs, idle_min_uptime_secs);
    }
    let readiness_signal = state.readiness.clone();
    let db_for_shutdown = state.db.clone();
    let state_for_idle_shutdown = state.clone();
    let router = server::build_router(state, paths.port);
    let shutdown_future = async move {
        let idle_shutdown_future = async move {
            if idle_shutdown_secs == 0 {
                std::future::pending::<()>().await;
                return;
            }
            tokio::time::sleep(Duration::from_secs(idle_min_uptime_secs)).await;
            let mut interval = tokio::time::interval(Duration::from_secs(DEFAULT_IDLE_SHUTDOWN_CHECK_INTERVAL_SECS));
            interval.tick().await;
            loop {
                interval.tick().await;
                let idle_for = state_for_idle_shutdown.idle_for_secs();
                if idle_for >= idle_shutdown_secs {
                    eprintln!("[cortex] Idle shutdown threshold reached (idle={}s >= {}s)", idle_for, idle_shutdown_secs);
                    break;
                }
            }
        };
        tokio::select! {_=shutdown_rx=>{eprintln!("[cortex] Shutdown requested via HTTP");}_=extra_shutdown=>
        {}_=idle_shutdown_future=>{}}
    };
    server::run(router, &paths.bind, paths.port, paths.ipc_endpoint.clone(), &db_path, Some(readiness_signal), shutdown_future).await;
    eprintln!("[cortex] Flushing database...");
    {
        let conn = db_for_shutdown.lock().await;
        if let Err(e) = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);") {
            eprintln!("[cortex] Warning: WAL checkpoint failed: {e}");
        }
    }
    let _ = std::fs::remove_file(&pid_path);
    eprintln!("[cortex] Shutdown complete.");
}
