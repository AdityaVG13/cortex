use super::read_pool::{open_query_only_connection,read_pool_size_from_env,ReadConnectionPool,ReadConnectionProvider};use super::
runtime::{current_unix_secs,RuntimeState};use super::types::{BrainFiringEvent,DaemonEvent,SqliteVecCanaryConfig,SqliteVecRouteMode
};use crate::auth::CortexPaths;use rusqlite::Connection;use std::collections::HashMap;use std::sync::atomic::{AtomicBool,AtomicU64
};use std::sync::Arc;use tokio::sync::{broadcast,oneshot,Mutex};pub fn initialize(paths:&CortexPaths,allow_token_rotation:bool)->
Result<(RuntimeState,oneshot::Receiver<()>),String>{let db_path=&paths.db;let conn=crate::db::open(db_path).map_err(|e|format!(
"Failed to open database at {}: {e}",db_path.display()))?;crate::db::configure(&conn).map_err(|e|format!(
"Failed to configure database: {e}"))?;crate::db::initialize_schema(&conn).map_err(|e|format!("Failed to initialise schema: {e}"))
?;if crate::db::quick_check(&conn){eprintln!("[cortex] DB quick_check: OK");}else{eprintln!(
"[cortex] WARNING: PRAGMA quick_check FAILED on {} -- running full integrity_check",db_path.display());let integrity_ok=crate::db
::verify_integrity(&conn).unwrap_or(false);if!integrity_ok{eprintln!(
"[cortex] WARNING: PRAGMA integrity_check FAILED on {} -- attempting auto-repair",db_path.display());drop(conn);let timestamp=
chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();match crate::db::auto_repair(db_path,&timestamp){Ok(result)=>{eprintln!(
"[cortex] Auto-repair succeeded: {} memories, {} decisions recovered. \
                         Corrupted DB preserved at {}",
result.memories_recovered,result.decisions_recovered,result.corrupt_db_path.display());let conn=crate::db::open(db_path).map_err(|
e|format!("Failed to open repaired DB: {e}"))?;crate::db::configure(&conn).map_err(|e|format!(
"Failed to configure repaired DB: {e}"))?;return initialize_with_conn(conn,paths,allow_token_rotation);}Err(e)=>{eprintln!(
"[cortex] Auto-repair failed ({e:?}). \
                         Starting in degraded mode -- reads may return incomplete data. \
                         DB path: {}"
,db_path.display());let conn=crate::db::open(db_path).map_err(|open_err|format!(
"Database corrupt and could not be reopened after failed repair: {open_err}"))?;crate::db::configure(&conn).ok();crate::db::
initialize_schema(&conn).ok();let(state,rx)=initialize_with_conn(conn,paths,allow_token_rotation)?;state.db_corrupted.store(true,
std::sync::atomic::Ordering::SeqCst);return Ok((state,rx));}}}else{eprintln!(
"[cortex] DB integrity: OK (after quick_check failure)");}}initialize_with_conn(conn,paths,allow_token_rotation)}fn
initialize_with_conn(conn:Connection,paths:&CortexPaths,allow_token_rotation:bool)->Result<(RuntimeState,oneshot::Receiver<()>),
String>{match crate::db::rebuild_fts_if_needed(&conn){Ok(true)=>eprintln!("[cortex] FTS baseline rebuilt"),Ok(false)=>{}Err(e)=>
eprintln!("[cortex] WARNING: FTS rebuild check failed: {e}"),}let read_pool_size=read_pool_size_from_env();let mut
read_connections=Vec::with_capacity(read_pool_size);for _ in 0..read_pool_size{read_connections.push(open_query_only_connection(&
paths.db)?);}let db_read:Arc<dyn ReadConnectionProvider>=Arc::new(ReadConnectionPool::new(read_connections));eprintln!(
"[cortex] Read pool opened with {} query-only connection{} (WAL concurrent reads enabled)",db_read.pool_size(),if db_read.
pool_size()==1{""}else{"s"});let mode=crate::db::current_mode(&conn);let team_mode=mode=="team";let default_owner_id=if team_mode{
let from_config=conn.query_row("SELECT value FROM config WHERE key = 'owner_user_id' LIMIT 1",[],|row|row.get::<_,String>(0)).ok()
.and_then(|v|v.parse::<i64>().ok());from_config.or_else(||{conn.query_row(
"SELECT id FROM users ORDER BY CASE role WHEN 'owner' THEN 0 ELSE 1 END, id ASC LIMIT 1",[],|row|row.get::<_,i64>(0)).ok()})}else{
None};let team_api_key_hashes=if team_mode{let mut hashes:Vec<(i64,String)>=Vec::new();if let Ok(mut stmt)=conn.prepare(
"SELECT id, api_key_hash FROM users"){if let Ok(rows)=stmt.query_map([],|row|Ok((row.get::<_,i64>(0)?,row.get::<_,String>(1)?))){
for row in rows.flatten(){hashes.push(row);}}}Arc::new(std::sync::RwLock::new(hashes))}else{Arc::new(std::sync::RwLock::new(Vec::
new()))};let token=if team_mode{crate::auth::read_token_from(paths).unwrap_or_else(crate::auth::generate_ephemeral_token)}else if
allow_token_rotation{crate::auth::try_generate_token_for(paths).map_err(|e|format!("Failed to generate shared auth token: {e}"))?}
else{crate::auth::read_token_from(paths).unwrap_or_else(crate::auth::generate_ephemeral_token)};let(events_tx,_)=broadcast::
channel::<DaemonEvent>(256);let(brain_firing_tx,_)=broadcast::channel::<BrainFiringEvent>(256);let(shutdown_tx,shutdown_rx)=
oneshot::channel::<()>();let home=paths.home.clone();let models_dir=paths.models.clone();let embedding_engine=crate::embeddings::
EmbeddingEngine::load(&models_dir).map(Arc::new);if let Some(engine)=embedding_engine.as_ref(){eprintln!(
"[cortex] Embedding engine loaded (model={}, {}-dim, in-process ONNX)",engine.model_key(),engine.dimension());}else{eprintln!(
"[cortex] Embedding engine not available -- keyword search only until model downloaded");}let write_buffer_path=paths.write_buffer
.clone();let sqlite_vec_canary=SqliteVecCanaryConfig::from_env();if sqlite_vec_canary.force_off{eprintln!(
"[cortex] sqlite-vec routing force-off (configured mode={}, effective mode=baseline)",sqlite_vec_canary.route_mode.as_str());}else
{match sqlite_vec_canary.route_mode{SqliteVecRouteMode::Baseline=>{eprintln!(
"[cortex] sqlite-vec routing mode=baseline (shadow diagnostics only)");}SqliteVecRouteMode::Trial=>{if sqlite_vec_canary.
trial_percent>0{eprintln!("[cortex] sqlite-vec routing mode=trial ({}% sampled)",sqlite_vec_canary.trial_percent);}else{eprintln!(
"[cortex] sqlite-vec routing mode=trial but trial percent is 0 (baseline-only)");}}SqliteVecRouteMode::Primary=>{eprintln!(
"[cortex] sqlite-vec routing mode=primary (guarded vec0 routing enabled)");}}}let rerank_config=crate::rerank::RerankConfig::
from_env();let reranker=if rerank_config.is_active(){match crate::rerank::MiniLmReranker::load(&models_dir){Some(engine)=>{
eprintln!("[cortex] Reranker loaded (model={}, mode={}, top_n={}, alpha={:.2})",crate::rerank::Reranker::name(&engine),
rerank_config.mode.as_str(),rerank_config.top_n,rerank_config.fusion_alpha);Some(Arc::new(engine)as Arc<dyn crate::rerank::
Reranker>)}None=>{eprintln!("[cortex] Reranker unavailable (mode={} requested, missing or invalid assets)",rerank_config.mode.
as_str());None}}}else{None};let budget_config_status=crate::budgets::BudgetConfigStatus::load_from_home(&paths.home);if let Some(
error)=budget_config_status.error.as_ref(){eprintln!(
"[cortex] WARNING: invalid budget config at {} ({}): {}; budget enforcement disabled",budget_config_status.source.display(),error.
code,error.message);}else if budget_config_status.enabled(){eprintln!("[cortex] Budget governance enabled from {}",
budget_config_status.source.display());}let state=RuntimeState{db:Arc::new(Mutex::new(conn)),db_read,token:Arc::new(token),events:
events_tx,brain_firing:brain_firing_tx,mcp_calls:Arc::new(AtomicU64::new(0)),mcp_sessions:Arc::new(Mutex::new(HashMap::new())),
recall_history:Arc::new(Mutex::new(HashMap::new())),pre_cache:Arc::new(Mutex::new(HashMap::new())),served_content:Arc::new(Mutex::
new(HashMap::<String,HashMap<u32,i64>>::new())),shutdown_tx:Arc::new(Mutex::new(Some(shutdown_tx))),home,db_path:paths.db.clone(),
token_path:paths.token.clone(),pid_path:paths.pid.clone(),port:paths.port,embedding_engine,rate_limiter:crate::rate_limit::
RateLimiter::new_with_budget_status(budget_config_status),team_mode,default_owner_id,team_api_key_hashes,degraded_mode:Arc::new(
AtomicBool::new(false)),db_corrupted:Arc::new(AtomicBool::new(false)),readiness:Arc::new(AtomicBool::new(false)),
last_activity_unix_secs:Arc::new(AtomicU64::new(current_unix_secs())),write_buffer_path,sqlite_vec_canary,rerank_config,reranker,}
;Ok((state,shutdown_rx))}
