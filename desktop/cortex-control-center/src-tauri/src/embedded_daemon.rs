use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, options, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use futures_util::stream::{self, StreamExt};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::env;
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;
use std::time::Duration as StdDuration;
use tokio::sync::{broadcast, Mutex};
use tokio_stream::wrappers::BroadcastStream;
use uuid::Uuid;

const PORT: u16 = 7437;
const SESSION_TTL_SECONDS: i64 = 120;
const MAX_ACTIVITIES: i64 = 1000;
const MAX_MESSAGES_PER_AGENT: i64 = 100;
const MAX_TASKS: i64 = 500;
const MAX_FEED: i64 = 200;
const FEED_TTL_SECONDS: i64 = 4 * 60 * 60;
const PRECACHE_TTL_MS: i64 = 5 * 60 * 1000;
const MAX_RECALL_HISTORY: usize = 50;

#[derive(Clone)]
struct FeedEntry {
  id: String,
  agent: String,
  kind: String,
  summary: String,
  content: Option<String>,
  files: Value,
  task_id: Option<String>,
  trace_id: Option<String>,
  priority: String,
  timestamp: String,
  tokens: i64,
}

#[derive(Clone)]
struct RecallItem {
  source: String,
  relevance: f64,
  excerpt: String,
  method: String,
  tokens: Option<usize>,
}

#[derive(Clone)]
struct RecallHistoryEntry {
  query: String,
  timestamp: i64,
}

#[derive(Clone)]
struct PreCacheEntry {
  query: String,
  results: Vec<RecallItem>,
  expires_at: i64,
}

#[derive(Clone)]
struct SearchCandidate {
  source: String,
  excerpt: String,
  relevance: f64,
  matched_keywords: i64,
  score: f64,
  ts: i64,
}

#[derive(Clone, Debug)]
struct DaemonEvent {
  event_type: String,
  data: Value,
}

#[derive(Clone)]
struct RuntimeState {
  db: Arc<Mutex<Connection>>,
  token: Arc<String>,
  events: broadcast::Sender<DaemonEvent>,
  mcp_calls: Arc<AtomicU64>,
  mcp_sessions: Arc<Mutex<HashMap<String, i64>>>,
  recall_history: Arc<Mutex<HashMap<String, Vec<RecallHistoryEntry>>>>,
  pre_cache: Arc<Mutex<HashMap<String, PreCacheEntry>>>,
  served_content: Arc<Mutex<HashMap<String, HashSet<u32>>>>,
}

impl RuntimeState {
  fn emit(&self, event_type: &str, data: Value) {
    let _ = self.events.send(DaemonEvent {
      event_type: event_type.to_string(),
      data,
    });
  }

  fn next_mcp_call(&self) -> u64 {
    self.mcp_calls.fetch_add(1, Ordering::SeqCst) + 1
  }
}

struct DaemonControl {
  shutdown_tx: mpsc::Sender<()>,
  handle: thread::JoinHandle<()>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EmbeddedDaemonStatus {
  pub running: bool,
  pub pid: Option<u32>,
}

#[derive(Default)]
pub struct EmbeddedDaemon {
  running: bool,
  control: Option<DaemonControl>,
}

impl EmbeddedDaemon {
  pub fn start(&mut self) -> Result<EmbeddedDaemonStatus, String> {
    if self.running {
      return Ok(self.status());
    }

    if is_reachable() {
      return Ok(self.status());
    }

    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    let (started_tx, started_rx) = mpsc::channel::<Result<(), String>>();

    let handle = thread::spawn(move || {
      run_daemon(shutdown_rx, started_tx);
    });

    match started_rx.recv_timeout(StdDuration::from_secs(5)) {
      Ok(Ok(())) => {
        self.running = true;
        self.control = Some(DaemonControl { shutdown_tx, handle });
        Ok(self.status())
      }
      Ok(Err(err)) => {
        let _ = handle.join();
        Err(err)
      }
      Err(_) => {
        let _ = handle.join();
        Err("Timed out while starting embedded daemon".to_string())
      }
    }
  }

  pub fn stop(&mut self) -> Result<EmbeddedDaemonStatus, String> {
    if let Some(control) = self.control.take() {
      let _ = control.shutdown_tx.send(());
      let _ = control.handle.join();
    }
    self.running = false;
    Ok(self.status())
  }

  pub fn status(&self) -> EmbeddedDaemonStatus {
    EmbeddedDaemonStatus {
      running: self.running,
      pid: self.running.then(std::process::id),
    }
  }
}

fn run_daemon(shutdown_rx: mpsc::Receiver<()>, started_tx: mpsc::Sender<Result<(), String>>) {
  let runtime = match tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
  {
    Ok(rt) => rt,
    Err(err) => {
      let _ = started_tx.send(Err(format!("Failed to create tokio runtime: {err}")));
      return;
    }
  };

  runtime.block_on(async move {
    let state = match initialize_state() {
      Ok(state) => state,
      Err(err) => {
        let _ = started_tx.send(Err(err));
        return;
      }
    };

    let router = build_router(state.clone());
    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", PORT)).await {
      Ok(listener) => listener,
      Err(err) => {
        let _ = started_tx.send(Err(format!("Failed to bind 127.0.0.1:{PORT}: {err}")));
        return;
      }
    };

    let _ = started_tx.send(Ok(()));

    let (graceful_tx, graceful_rx) = tokio::sync::oneshot::channel::<()>();
    thread::spawn(move || {
      let _ = shutdown_rx.recv();
      let _ = graceful_tx.send(());
    });

    let server = axum::serve(listener, router).with_graceful_shutdown(async {
      let _ = graceful_rx.await;
    });
    let _ = server.await;

    {
      let conn = state.db.lock().await;
      checkpoint_wal_best_effort(&conn);
    }
  });
}

fn build_router(state: RuntimeState) -> Router {
  Router::new()
    .route("/boot", get(handle_boot))
    .route("/recall", get(handle_recall))
    .route("/peek", get(handle_peek))
    .route("/store", post(handle_store))
    .route("/health", get(handle_health))
    .route("/digest", get(handle_digest))
    .route("/savings", get(handle_savings))
    .route("/dump", get(handle_dump))
    .route("/forget", post(handle_forget))
    .route("/resolve", post(handle_resolve))
    .route("/conflicts", get(handle_conflicts))
    .route("/mcp", post(handle_mcp_post).get(handle_mcp_get))
    .route("/events/stream", get(handle_events_stream))
    .route("/lock", post(handle_lock))
    .route("/unlock", post(handle_unlock))
    .route("/locks", get(handle_locks))
    .route("/activity", post(handle_post_activity).get(handle_get_activity))
    .route("/message", post(handle_post_message))
    .route("/messages", post(handle_post_message).get(handle_get_messages))
    .route("/session/start", post(handle_session_start))
    .route("/session/heartbeat", post(handle_session_heartbeat))
    .route("/session/end", post(handle_session_end))
    .route("/sessions", get(handle_sessions))
    .route("/tasks", post(handle_create_task).get(handle_get_tasks))
    .route("/tasks/next", get(handle_next_task))
    .route("/tasks/claim", post(handle_claim_task))
    .route("/tasks/complete", post(handle_complete_task))
    .route("/tasks/abandon", post(handle_abandon_task))
    .route("/feed", post(handle_post_feed).get(handle_get_feed))
    .route("/feed/ack", post(handle_feed_ack))
    .route("/feed/{id}", get(handle_get_feed_by_id))
    .route("/{*path}", options(handle_options))
    .fallback(handle_not_found)
    .with_state(state)
}

fn initialize_state() -> Result<RuntimeState, String> {
  let home = cortex_home()?;
  let db_path = home.join("cortex").join("cortex.db");
  let token_path = home.join(".cortex").join("cortex.token");

  if let Some(parent) = db_path.parent() {
    fs::create_dir_all(parent)
      .map_err(|err| format!("Failed to create DB dir {}: {err}", parent.display()))?;
  }
  if let Some(parent) = token_path.parent() {
    fs::create_dir_all(parent)
      .map_err(|err| format!("Failed to create token dir {}: {err}", parent.display()))?;
  }

  let conn = Connection::open(&db_path)
    .map_err(|err| format!("Failed to open DB {}: {err}", db_path.display()))?;
  configure_sqlite(&conn).map_err(|err| format!("Failed to configure SQLite pragmas: {err}"))?;
  initialize_schema(&conn).map_err(|err| format!("Failed to initialize schema: {err}"))?;

  let token = if let Ok(existing) = fs::read_to_string(&token_path) {
    let trimmed = existing.trim().to_string();
    if trimmed.is_empty() {
      let generated = Uuid::new_v4().simple().to_string();
      fs::write(&token_path, &generated)
        .map_err(|err| format!("Failed to write token {}: {err}", token_path.display()))?;
      generated
    } else {
      trimmed
    }
  } else {
    let generated = Uuid::new_v4().simple().to_string();
    fs::write(&token_path, &generated)
      .map_err(|err| format!("Failed to write token {}: {err}", token_path.display()))?;
    generated
  };

  let (events, _) = broadcast::channel(256);
  Ok(RuntimeState {
    db: Arc::new(Mutex::new(conn)),
    token: Arc::new(token),
    events,
    mcp_calls: Arc::new(AtomicU64::new(0)),
    mcp_sessions: Arc::new(Mutex::new(HashMap::new())),
    recall_history: Arc::new(Mutex::new(HashMap::new())),
    pre_cache: Arc::new(Mutex::new(HashMap::new())),
    served_content: Arc::new(Mutex::new(HashMap::new())),
  })
}

fn configure_sqlite(conn: &Connection) -> rusqlite::Result<()> {
  conn.execute_batch(
    r#"
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = FULL;
    PRAGMA foreign_keys = ON;
    "#,
  )?;
  Ok(())
}

fn checkpoint_wal(conn: &Connection) -> rusqlite::Result<()> {
  conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
  Ok(())
}

fn checkpoint_wal_best_effort(conn: &Connection) {
  let _ = checkpoint_wal(conn);
}

fn initialize_schema(conn: &Connection) -> rusqlite::Result<()> {
  conn.execute_batch(
    r#"
    PRAGMA journal_mode = WAL;
    PRAGMA foreign_keys = ON;

    CREATE TABLE IF NOT EXISTS memories (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      text TEXT NOT NULL,
      source TEXT,
      type TEXT DEFAULT 'memory',
      tags TEXT,
      source_agent TEXT DEFAULT 'unknown',
      confidence REAL DEFAULT 0.8,
      status TEXT DEFAULT 'active',
      score REAL DEFAULT 1.0,
      retrievals INTEGER DEFAULT 0,
      last_accessed TEXT,
      pinned INTEGER DEFAULT 0,
      disputes_id INTEGER,
      supersedes_id INTEGER,
      confirmed_by TEXT,
      created_at TEXT DEFAULT (datetime('now')),
      updated_at TEXT DEFAULT (datetime('now'))
    );

    CREATE TABLE IF NOT EXISTS decisions (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      decision TEXT NOT NULL,
      context TEXT,
      type TEXT DEFAULT 'decision',
      source_agent TEXT DEFAULT 'unknown',
      confidence REAL DEFAULT 0.8,
      surprise REAL DEFAULT 1.0,
      status TEXT DEFAULT 'active',
      score REAL DEFAULT 1.0,
      retrievals INTEGER DEFAULT 0,
      last_accessed TEXT,
      pinned INTEGER DEFAULT 0,
      parent_id INTEGER,
      disputes_id INTEGER,
      supersedes_id INTEGER,
      confirmed_by TEXT,
      created_at TEXT DEFAULT (datetime('now')),
      updated_at TEXT DEFAULT (datetime('now'))
    );

    CREATE TABLE IF NOT EXISTS embeddings (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      target_type TEXT NOT NULL,
      target_id INTEGER NOT NULL,
      vector BLOB NOT NULL,
      model TEXT DEFAULT 'nomic-embed-text',
      created_at TEXT DEFAULT (datetime('now'))
    );

    CREATE TABLE IF NOT EXISTS events (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      type TEXT NOT NULL,
      data TEXT,
      source_agent TEXT,
      created_at TEXT DEFAULT (datetime('now'))
    );

    CREATE TABLE IF NOT EXISTS co_occurrence (
      source_a TEXT NOT NULL,
      source_b TEXT NOT NULL,
      count INTEGER DEFAULT 1,
      last_seen TEXT DEFAULT (datetime('now')),
      PRIMARY KEY (source_a, source_b)
    );

    CREATE TABLE IF NOT EXISTS locks (
      id TEXT PRIMARY KEY,
      path TEXT NOT NULL UNIQUE,
      agent TEXT NOT NULL,
      locked_at TEXT NOT NULL,
      expires_at TEXT
    );

    CREATE TABLE IF NOT EXISTS activities (
      id TEXT PRIMARY KEY,
      agent TEXT NOT NULL,
      description TEXT NOT NULL,
      files_json TEXT NOT NULL DEFAULT '[]',
      timestamp TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS messages (
      id TEXT PRIMARY KEY,
      sender TEXT NOT NULL,
      recipient TEXT NOT NULL,
      message TEXT NOT NULL,
      timestamp TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS sessions (
      agent TEXT PRIMARY KEY,
      session_id TEXT NOT NULL,
      project TEXT,
      files_json TEXT NOT NULL DEFAULT '[]',
      description TEXT,
      started_at TEXT NOT NULL,
      last_heartbeat TEXT NOT NULL,
      expires_at TEXT NOT NULL
    );

    CREATE TABLE IF NOT EXISTS tasks (
      task_id TEXT PRIMARY KEY,
      title TEXT NOT NULL,
      description TEXT,
      project TEXT,
      files_json TEXT NOT NULL DEFAULT '[]',
      priority TEXT NOT NULL DEFAULT 'medium',
      required_capability TEXT NOT NULL DEFAULT 'any',
      status TEXT NOT NULL DEFAULT 'pending',
      claimed_by TEXT,
      created_at TEXT NOT NULL,
      claimed_at TEXT,
      completed_at TEXT,
      summary TEXT
    );

    CREATE TABLE IF NOT EXISTS feed (
      id TEXT PRIMARY KEY,
      agent TEXT NOT NULL,
      kind TEXT NOT NULL,
      summary TEXT NOT NULL,
      content TEXT,
      files_json TEXT NOT NULL DEFAULT '[]',
      task_id TEXT,
      trace_id TEXT,
      priority TEXT NOT NULL DEFAULT 'normal',
      timestamp TEXT NOT NULL,
      tokens INTEGER NOT NULL DEFAULT 0
    );

    CREATE TABLE IF NOT EXISTS feed_acks (
      agent TEXT PRIMARY KEY,
      last_seen_id TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE INDEX IF NOT EXISTS idx_cooccur_a ON co_occurrence(source_a);
    CREATE INDEX IF NOT EXISTS idx_cooccur_b ON co_occurrence(source_b);
    "#,
  )?;
  Ok(())
}

fn cortex_home() -> Result<PathBuf, String> {
  env::var_os("USERPROFILE")
    .map(PathBuf::from)
    .or_else(|| env::var_os("HOME").map(PathBuf::from))
    .ok_or_else(|| "Could not resolve USERPROFILE/HOME".to_string())
}

fn is_reachable() -> bool {
  let addr = SocketAddr::from(([127, 0, 0, 1], PORT));
  TcpStream::connect_timeout(&addr, StdDuration::from_millis(300)).is_ok()
}

#[derive(Deserialize, Default)]
struct BootQuery {
  profile: Option<String>,
  agent: Option<String>,
  budget: Option<usize>,
}

#[derive(Deserialize, Default)]
struct RecallQuery {
  q: Option<String>,
  k: Option<usize>,
  budget: Option<usize>,
  agent: Option<String>,
}

#[derive(Deserialize, Default)]
struct StoreRequest {
  decision: Option<String>,
  context: Option<String>,
  #[serde(rename = "type")]
  entry_type: Option<String>,
  source_agent: Option<String>,
  confidence: Option<f64>,
}

#[derive(Deserialize, Default)]
struct ForgetRequest {
  keyword: Option<String>,
  source: Option<String>,
}

#[derive(Deserialize, Default)]
struct ResolveRequest {
  #[serde(rename = "keepId")]
  keep_id: Option<i64>,
  action: Option<String>,
  #[serde(rename = "supersededId")]
  superseded_id: Option<i64>,
}

#[derive(Deserialize, Default)]
struct LockRequest {
  path: Option<String>,
  agent: Option<String>,
  ttl: Option<i64>,
}

#[derive(Deserialize, Default)]
struct ActivityRequest {
  agent: Option<String>,
  description: Option<String>,
  files: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
struct SinceQuery {
  since: Option<String>,
}

#[derive(Deserialize, Default)]
struct MessageRequest {
  from: Option<String>,
  to: Option<String>,
  message: Option<String>,
}

#[derive(Deserialize, Default)]
struct MessagesQuery {
  agent: Option<String>,
}

#[derive(Deserialize, Default)]
struct SessionStartRequest {
  agent: Option<String>,
  project: Option<String>,
  files: Option<Vec<String>>,
  description: Option<String>,
  ttl: Option<i64>,
}

#[derive(Deserialize, Default)]
struct SessionHeartbeatRequest {
  agent: Option<String>,
  files: Option<Vec<String>>,
  description: Option<String>,
}

#[derive(Deserialize, Default)]
struct SessionEndRequest {
  agent: Option<String>,
}

#[derive(Deserialize, Default)]
struct TaskCreateRequest {
  title: Option<String>,
  description: Option<String>,
  project: Option<String>,
  files: Option<Vec<String>>,
  priority: Option<String>,
  #[serde(rename = "requiredCapability")]
  required_capability: Option<String>,
}

#[derive(Deserialize, Default)]
struct TaskQuery {
  status: Option<String>,
  project: Option<String>,
}

#[derive(Deserialize, Default)]
struct TaskClaimRequest {
  #[serde(rename = "taskId")]
  task_id: Option<String>,
  agent: Option<String>,
}

#[derive(Deserialize, Default)]
struct TaskCompleteRequest {
  #[serde(rename = "taskId")]
  task_id: Option<String>,
  agent: Option<String>,
  summary: Option<String>,
}

#[derive(Deserialize, Default)]
struct TaskAbandonRequest {
  #[serde(rename = "taskId")]
  task_id: Option<String>,
  agent: Option<String>,
}

#[derive(Deserialize, Default)]
struct NextTaskQuery {
  agent: Option<String>,
  capability: Option<String>,
}

#[derive(Deserialize, Default)]
struct FeedRequest {
  agent: Option<String>,
  kind: Option<String>,
  summary: Option<String>,
  content: Option<String>,
  files: Option<Vec<String>>,
  #[serde(rename = "taskId")]
  task_id: Option<String>,
  #[serde(rename = "traceId")]
  trace_id: Option<String>,
  priority: Option<String>,
}

#[derive(Deserialize, Default)]
struct FeedQuery {
  since: Option<String>,
  kind: Option<String>,
  agent: Option<String>,
  unread: Option<bool>,
}

#[derive(Deserialize, Default)]
struct FeedAckRequest {
  agent: Option<String>,
  #[serde(rename = "lastSeenId")]
  last_seen_id: Option<String>,
}

fn now_iso() -> String {
  Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn default_boot_prompt() -> String {
  "## Identity\nUser: Aditya. Platform: Windows 10.\n\n## Delta\nRust daemon core online with persistent conductor state.".to_string()
}

fn parse_duration_to_seconds(raw: &str) -> i64 {
  if raw.is_empty() {
    return 60 * 60;
  }

  let mut chars = raw.chars();
  let unit = chars.next_back().unwrap_or('h');
  let digits = chars.as_str();
  if digits.is_empty() {
    return 60 * 60;
  }

  let value = digits.parse::<i64>().unwrap_or(1).max(1);
  match unit {
    'm' => value * 60,
    'h' => value * 60 * 60,
    'd' => value * 24 * 60 * 60,
    _ => 60 * 60,
  }
}

fn redact_secrets(text: &str) -> String {
  let bearer = Regex::new(r"Bearer\s+[a-f0-9]{32,}")
    .map(|re| re.replace_all(text, "Bearer [REDACTED]").to_string())
    .unwrap_or_else(|_| text.to_string());
  let hashes = Regex::new(r"[a-f0-9]{40,}")
    .map(|re| re.replace_all(&bearer, "[HASH_REDACTED]").to_string())
    .unwrap_or(bearer);
  Regex::new(r"(?i)(?:token|key|secret|password)\s*[:=]\s*\S+")
    .map(|re| re.replace_all(&hashes, "[CREDENTIAL_REDACTED]").to_string())
    .unwrap_or(hashes)
}

fn json_response(status: StatusCode, body: Value) -> Response {
  let mut response = (status, Json(body)).into_response();
  apply_json_headers(response.headers_mut());
  response
}

fn apply_json_headers(headers: &mut HeaderMap) {
  headers.insert("Cache-Control", HeaderValue::from_static("no-store"));
  headers.insert("Access-Control-Allow-Origin", HeaderValue::from_static("*"));
  headers.insert(
    "Access-Control-Allow-Headers",
    HeaderValue::from_static("Authorization, Content-Type"),
  );
  headers.insert(
    "Access-Control-Allow-Methods",
    HeaderValue::from_static("GET, POST, OPTIONS"),
  );
}

fn ensure_auth(headers: &HeaderMap, state: &RuntimeState) -> Result<(), Response> {
  let header = headers
    .get("authorization")
    .and_then(|h| h.to_str().ok())
    .unwrap_or("");

  let token = header
    .strip_prefix("Bearer ")
    .or_else(|| header.strip_prefix("bearer "));

  match token {
    Some(candidate) if candidate == state.token.as_str() => Ok(()),
    _ => Err(json_response(
      StatusCode::UNAUTHORIZED,
      json!({ "error": "Unauthorized" }),
    )),
  }
}

async fn handle_options() -> Response {
  let mut response = StatusCode::NO_CONTENT.into_response();
  apply_json_headers(response.headers_mut());
  response
}

async fn handle_not_found() -> Response {
  json_response(StatusCode::NOT_FOUND, json!({ "error": "Not found" }))
}

async fn handle_peek(
  State(state): State<RuntimeState>,
  Query(query): Query<RecallQuery>,
) -> Response {
  let q = match &query.q {
    Some(q) if !q.is_empty() => q.clone(),
    _ => return json_response(StatusCode::BAD_REQUEST, json!({"error": "Missing query parameter: q"})),
  };
  let k = query.k.unwrap_or(10) as usize;
  let mut conn = state.db.lock().await;
  match run_recall(&mut conn, &q, k) {
    Ok(results) => {
      let matches: Vec<Value> = results.iter().map(|r| json!({
        "source": r.source,
        "relevance": r.relevance,
        "method": r.method,
      })).collect();
      json_response(StatusCode::OK, json!({"count": matches.len(), "matches": matches}))
    }
    Err(e) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e})),
  }
}

fn estimate_tokens(text: &str) -> usize {
  (text.len() as f64 / 3.8).ceil() as usize
}

fn read_state_md() -> String {
  let home = std::env::var("USERPROFILE")
    .or_else(|_| std::env::var("HOME"))
    .unwrap_or_default();
  let path = std::path::Path::new(&home).join(".claude").join("state.md");
  std::fs::read_to_string(path).unwrap_or_default()
}

fn extract_section(content: &str, heading: &str, max_lines: usize) -> String {
  let mut capturing = false;
  let mut captured = Vec::new();
  for line in content.lines() {
    if capturing {
      if line.starts_with("## ") { break; }
      captured.push(line);
      if captured.len() >= max_lines { break; }
    } else if line.trim_start_matches("## ").trim() == heading {
      capturing = true;
    }
  }
  captured.join("\n").trim().to_string()
}

fn build_identity_capsule(conn: &Connection) -> (String, usize) {
  let mut parts = vec![
    "User: Aditya. Platform: Windows 10. Shell: bash. Python: uv only. Git: conventional commits.".to_string(),
  ];

  // Hard constraints (never/always/must rules)
  if let Ok(mut stmt) = conn.prepare(
    "SELECT text FROM memories WHERE type = 'feedback' AND status = 'active' ORDER BY score DESC LIMIT 20"
  ) {
    let constraint_re = Regex::new(r"(?i)\b(never|always|must|do not|don't|required|mandatory)\b").unwrap();
    if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
      let constraints: Vec<String> = rows
        .filter_map(|r| r.ok())
        .filter(|t| constraint_re.is_match(t))
        .take(5)
        .map(|t| { let s = &t[..t.len().min(120)]; s.to_string() })
        .collect();
      if !constraints.is_empty() {
        parts.push(format!("Rules: {}", constraints.join(" | ")));
      }
    }
  }

  // Platform sharp edges
  if let Ok(mut stmt) = conn.prepare(
    "SELECT text FROM memories WHERE type = 'feedback' AND status = 'active' ORDER BY score DESC LIMIT 20"
  ) {
    let edge_re = Regex::new(r"(?i)\b(windows|win32|encoding|cp1252|bash\.exe|CRLF)\b").unwrap();
    if let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) {
      let edges: Vec<String> = rows
        .filter_map(|r| r.ok())
        .filter(|t| edge_re.is_match(t))
        .take(3)
        .map(|t| { let s = &t[..t.len().min(100)]; s.to_string() })
        .collect();
      if !edges.is_empty() {
        parts.push(format!("Sharp edges: {}", edges.join(" | ")));
      }
    }
  }

  let text = parts.join("\n");
  let tokens = estimate_tokens(&text);
  (text, tokens)
}

fn get_last_boot_time(conn: &Connection, agent: &str) -> Option<String> {
  conn.query_row(
    "SELECT data FROM events WHERE type = 'agent_boot' AND source_agent = ?1 ORDER BY created_at DESC LIMIT 1",
    params![agent],
    |r| r.get::<_, String>(0),
  ).ok().and_then(|data| {
    serde_json::from_str::<Value>(&data).ok()?.get("timestamp")?.as_str().map(|s| s.to_string())
  })
}

fn build_delta_capsule(conn: &Connection, agent: &str) -> (String, usize, String) {
  let last_boot = get_last_boot_time(conn, agent);
  let mut parts: Vec<String> = Vec::new();

  // Pending messages
  let messages = fetch_messages_for_agent(conn, agent).unwrap_or_default();
  if !messages.is_empty() {
    let lines: Vec<String> = messages.iter().map(|m| {
      let from = m.get("from").and_then(|v| v.as_str()).unwrap_or("?");
      let msg = m.get("message").and_then(|v| v.as_str()).unwrap_or("");
      format!("- From {}: \"{}\"", from, &msg[..msg.len().min(200)])
    }).collect();
    parts.push(format!("## Pending Messages\n{}", lines.join("\n")));
  }

  // Active agents
  let sessions = fetch_sessions(conn).unwrap_or_default();
  let other_sessions: Vec<&Value> = sessions.iter()
    .filter(|s| s.get("agent").and_then(|v| v.as_str()) != Some(agent))
    .collect();
  if !other_sessions.is_empty() {
    let lines: Vec<String> = other_sessions.iter().map(|s| {
      let ag = s.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
      let proj = s.get("project").and_then(|v| v.as_str()).unwrap_or("unknown");
      let desc = s.get("description").and_then(|v| v.as_str()).unwrap_or("no description");
      format!("- {} working on {}: \"{}\"", ag, proj, desc)
    }).collect();
    parts.push(format!("## Active Agents\n{}", lines.join("\n")));
  }

  // Active locks
  let locks = fetch_locks(conn).unwrap_or_default();
  if !locks.is_empty() {
    let lines: Vec<String> = locks.iter().map(|l| {
      let path = l.get("path").and_then(|v| v.as_str()).unwrap_or("?");
      let ag = l.get("agent").and_then(|v| v.as_str()).unwrap_or("?");
      format!("- {} locked by {}", path, ag)
    }).collect();
    parts.push(format!("## Active Locks\n{}", lines.join("\n")));
  }

  // Unread feed
  let mut feed = get_unread_feed(conn, agent).unwrap_or_default();
  if feed.len() > 10 { feed = feed.split_off(feed.len() - 10); }
  if !feed.is_empty() {
    // Mark as read
    if let Some(last) = feed.last() {
      let _ = conn.execute(
        "INSERT INTO feed_acks (agent, last_seen_id, updated_at) VALUES (?1, ?2, ?3)
         ON CONFLICT(agent) DO UPDATE SET last_seen_id = excluded.last_seen_id, updated_at = excluded.updated_at",
        params![agent, last.id, now_iso()],
      );
    }
    let lines: Vec<String> = feed.iter().map(|e| {
      format!("- [{}] {}: {}", e.kind, e.agent, e.summary)
    }).collect();
    parts.push(format!("## Feed\n{}", lines.join("\n")));
  }

  // Tasks
  let tasks = fetch_tasks(conn, "all", None).unwrap_or_default();
  let pending: Vec<&Value> = tasks.iter().filter(|t| t.get("status").and_then(|v| v.as_str()) == Some("pending")).collect();
  if !pending.is_empty() {
    let lines: Vec<String> = pending.iter().take(5).map(|t| {
      let pri = t.get("priority").and_then(|v| v.as_str()).unwrap_or("?");
      let title = t.get("title").and_then(|v| v.as_str()).unwrap_or("?");
      format!("- [{}] {}", pri, title)
    }).collect();
    parts.push(format!("## Pending Tasks\n{}", lines.join("\n")));
  }

  // State.md sections
  let state = read_state_md();
  if !state.is_empty() {
    let next = extract_section(&state, "Next Session", 5);
    if !next.is_empty() { parts.push(format!("Next: {}", next.replace('\n', " | "))); }
    let pending_s = extract_section(&state, "Pending", 3);
    if !pending_s.is_empty() { parts.push(format!("Pending: {}", pending_s.replace('\n', " | "))); }
    let issues = extract_section(&state, "Known Issues", 3);
    if !issues.is_empty() { parts.push(format!("Issues: {}", issues.replace('\n', " | "))); }
  }

  // New decisions since last boot
  if let Some(ref lb) = last_boot {
    if let Ok(mut stmt) = conn.prepare(
      "SELECT decision, context, source_agent FROM decisions WHERE status = 'active' AND created_at >= ?1 ORDER BY created_at DESC LIMIT 5"
    ) {
      if let Ok(rows) = stmt.query_map(params![lb], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?))
      }) {
        let lines: Vec<String> = rows.filter_map(|r| r.ok()).map(|(dec, ctx, ag)| {
          let c = ctx.map(|c| format!(" ({})", c)).unwrap_or_default();
          format!("- [{}] {}{}", ag, dec, c)
        }).collect();
        if !lines.is_empty() { parts.push(format!("New decisions:\n{}", lines.join("\n"))); }
      }
    }

    // New memories since last boot
    if let Ok(mut stmt) = conn.prepare(
      "SELECT text, type FROM memories WHERE status = 'active' AND updated_at >= ?1 AND type != 'state' ORDER BY updated_at DESC LIMIT 3"
    ) {
      if let Ok(rows) = stmt.query_map(params![lb], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
      }) {
        let lines: Vec<String> = rows.filter_map(|r| r.ok()).map(|(text, mtype)| {
          format!("- [{}] {}", mtype, &text[..text.len().min(100)])
        }).collect();
        if !lines.is_empty() { parts.push(format!("New knowledge:\n{}", lines.join("\n"))); }
      }
    }
  } else {
    // First boot — show recent decisions for orientation
    if let Ok(mut stmt) = conn.prepare(
      "SELECT decision, context FROM decisions WHERE status = 'active' ORDER BY created_at DESC LIMIT 5"
    ) {
      if let Ok(rows) = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
      }) {
        let lines: Vec<String> = rows.filter_map(|r| r.ok()).map(|(dec, ctx)| {
          let c = ctx.map(|c| format!(" — {}", c)).unwrap_or_default();
          format!("- {}{}", dec, c)
        }).collect();
        if !lines.is_empty() { parts.push(format!("Recent decisions:\n{}", lines.join("\n"))); }
      }
    }
  }

  let text = parts.join("\n\n");
  let tokens = estimate_tokens(&text);
  let freshness = last_boot.as_ref()
    .map(|lb| format!("since {}", &lb[..lb.len().min(16)]))
    .unwrap_or_else(|| "first boot".to_string());
  (text, tokens, freshness)
}

async fn handle_boot(
  State(state): State<RuntimeState>,
  Query(query): Query<BootQuery>,
  headers: HeaderMap,
) -> Response {
  let agent = query
    .agent
    .or_else(|| {
      headers
        .get("x-source-agent")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
    })
    .unwrap_or_else(|| "unknown".to_string());
  let profile = query.profile.unwrap_or_else(|| "full".to_string());
  let max_tokens = query.budget.unwrap_or(600) as usize;

  clear_served_on_boot(&state, &agent).await;

  let conn = state.db.lock().await;
  let _ = clean_expired_locks(&conn);
  let _ = clean_expired_sessions(&conn);
  let _ = clean_old_feed(&conn);

  // Build capsules (matches Node.js compiler.js logic)
  let (identity_text, identity_tokens) = build_identity_capsule(&conn);
  let (delta_text, delta_tokens, delta_freshness) = build_delta_capsule(&conn, &agent);

  // Record boot
  let boot_ts = now_iso();
  let _ = conn.execute(
    "INSERT INTO events (type, data, source_agent) VALUES (?1, ?2, ?3)",
    params![
      "agent_boot",
      serde_json::to_string(&json!({"timestamp": boot_ts, "agent": agent.clone()})).unwrap_or_default(),
      agent.clone()
    ],
  );

  // Assemble with token budget
  let identity_section = format!("## Identity\n{}", identity_text);
  let delta_section = format!("## Delta\n{}", delta_text);
  let mut assembled = identity_section.clone();
  let mut capsules = vec![json!({
    "name": "identity", "tokens": identity_tokens, "freshness": "stable", "truncated": false
  })];

  let combined = format!("{}\n\n{}", assembled, delta_section);
  if estimate_tokens(&combined) <= max_tokens {
    assembled = combined;
    capsules.push(json!({
      "name": "delta", "tokens": delta_tokens, "freshness": delta_freshness, "truncated": false
    }));
  } else {
    // Truncate delta to fit budget
    let remaining = max_tokens.saturating_sub(estimate_tokens(&assembled)).saturating_sub(10);
    if remaining > 50 && !delta_text.is_empty() {
      let trunc_chars = (remaining as f64 * 3.8) as usize;
      let trunc = &delta_text[..delta_text.len().min(trunc_chars)];
      assembled = format!("{}\n\n## Delta\n{}...", assembled, trunc);
      capsules.push(json!({
        "name": "delta", "tokens": estimate_tokens(trunc), "freshness": delta_freshness, "truncated": true
      }));
    }
  }

  let token_estimate = estimate_tokens(&assembled);

  // Estimate raw baseline for savings calculation
  let raw_baseline = {
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    let state_size = std::fs::metadata(std::path::Path::new(&home).join(".claude").join("state.md"))
      .map(|m| m.len() as usize).unwrap_or(0);
    let mem_dir = std::path::Path::new(&home).join(".claude").join("projects").join("C--Users-aditya").join("memory");
    let mem_size: usize = std::fs::read_dir(&mem_dir).ok().map(|entries| {
      entries.filter_map(|e| e.ok()).filter(|e| e.path().extension().map(|x| x == "md").unwrap_or(false))
        .filter_map(|e| e.metadata().ok()).map(|m| m.len() as usize).sum()
    }).unwrap_or(0);
    estimate_tokens(&"x".repeat(state_size + mem_size))
  };
  let saved = raw_baseline.saturating_sub(token_estimate);
  let percent = if raw_baseline > 0 { (saved * 100) / raw_baseline } else { 0 };

  let _ = conn.execute(
    "INSERT INTO events (type, data, source_agent) VALUES (?1, ?2, ?3)",
    params![
      "boot_savings",
      serde_json::to_string(&json!({
        "agent": agent.clone(),
        "served": token_estimate,
        "baseline": raw_baseline,
        "saved": saved,
        "percent": percent
      }))
      .unwrap_or_default(),
      "rust-daemon"
    ],
  );
  checkpoint_wal_best_effort(&conn);

  state.emit("agent_boot", json!({"agent": agent, "profile": profile}));

  json_response(
    StatusCode::OK,
    json!({
      "bootPrompt": assembled,
      "tokenEstimate": token_estimate,
      "profile": if profile == "full" { "capsules" } else { &profile },
      "capsules": capsules,
      "savings": { "rawBaseline": raw_baseline, "served": token_estimate, "saved": saved, "percent": percent }
    }),
  )
}

async fn handle_recall(
  State(state): State<RuntimeState>,
  Query(query): Query<RecallQuery>,
  headers: HeaderMap,
) -> Response {
  let q = query.q.unwrap_or_default();
  let k = query.k.unwrap_or(10);
  let budget = query.budget.unwrap_or(200);
  let agent = query
    .agent
    .or_else(|| {
      headers
        .get("x-source-agent")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string())
    })
    .unwrap_or_else(|| "http".to_string());

  if q.trim().is_empty() {
    return json_response(
      StatusCode::BAD_REQUEST,
      json!({ "error": "Missing query parameter: q" }),
    );
  }

  match execute_unified_recall(&state, q.trim(), budget, k, &agent).await {
    Ok(payload) => json_response(StatusCode::OK, payload),
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Recall failed: {err}") }),
    ),
  }
}

async fn handle_store(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Json(body): Json<StoreRequest>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let decision = body.decision.unwrap_or_default();
  if decision.trim().is_empty() {
    return json_response(
      StatusCode::BAD_REQUEST,
      json!({ "error": "Missing field: decision" }),
    );
  }

  let source_agent = headers
    .get("x-source-agent")
    .and_then(|v| v.to_str().ok())
    .map(|s| s.to_string())
    .or(body.source_agent)
    .unwrap_or_else(|| "http".to_string());

  let mut conn = state.db.lock().await;
  match store_decision(
    &mut conn,
    decision.trim(),
    body.context,
    body.entry_type,
    source_agent,
    body.confidence,
  ) {
    Ok(entry) => json_response(StatusCode::OK, json!({ "stored": true, "entry": entry })),
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Store failed: {err}") }),
    ),
  }
}

async fn handle_health(State(state): State<RuntimeState>) -> Response {
  let conn = state.db.lock().await;
  let memories: i64 = conn
    .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
    .unwrap_or(0);
  let decisions: i64 = conn
    .query_row("SELECT COUNT(*) FROM decisions", [], |row| row.get(0))
    .unwrap_or(0);
  let embeddings: i64 = conn
    .query_row("SELECT COUNT(*) FROM embeddings", [], |row| row.get(0))
    .unwrap_or(0);
  let events: i64 = conn
    .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
    .unwrap_or(0);

  let home = env::var("USERPROFILE")
    .or_else(|_| env::var("HOME"))
    .unwrap_or_default();

  json_response(
    StatusCode::OK,
    json!({
      "status": "ok",
      "stats": {
        "memories": memories,
        "decisions": decisions,
        "embeddings": embeddings,
        "events": events,
        "ollama": "offline",
        "home": home
      }
    }),
  )
}

async fn handle_digest(State(state): State<RuntimeState>) -> Response {
  let conn = state.db.lock().await;
  match build_digest(&conn) {
    Ok(payload) => json_response(StatusCode::OK, payload),
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Digest failed: {err}") }),
    ),
  }
}

async fn handle_dump(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let conn = state.db.lock().await;
  let memories: Vec<Value> = conn.prepare(
    "SELECT id, text, source, type, tags, source_agent, confidence, status, score, retrievals, last_accessed, pinned, disputes_id, supersedes_id, confirmed_by, created_at, updated_at
     FROM memories
     WHERE status = 'active'
     ORDER BY score DESC"
  ).and_then(|mut stmt| {
    stmt.query_map([], |row| Ok(json!({
      "id": row.get::<_, i64>(0)?,
      "text": row.get::<_, String>(1).unwrap_or_default(),
      "source": row.get::<_, Option<String>>(2).unwrap_or(None),
      "type": row.get::<_, String>(3).unwrap_or_default(),
      "tags": row.get::<_, Option<String>>(4).unwrap_or(None),
      "source_agent": row.get::<_, Option<String>>(5).unwrap_or(None),
      "confidence": row.get::<_, Option<f64>>(6).unwrap_or(Some(0.8)),
      "status": row.get::<_, Option<String>>(7).unwrap_or(Some("active".to_string())),
      "score": row.get::<_, Option<f64>>(8).unwrap_or(Some(1.0)),
      "retrievals": row.get::<_, Option<i64>>(9).unwrap_or(Some(0)),
      "last_accessed": row.get::<_, Option<String>>(10).unwrap_or(None),
      "pinned": row.get::<_, Option<i64>>(11).unwrap_or(Some(0)),
      "disputes_id": row.get::<_, Option<i64>>(12).unwrap_or(None),
      "supersedes_id": row.get::<_, Option<i64>>(13).unwrap_or(None),
      "confirmed_by": row.get::<_, Option<String>>(14).unwrap_or(None),
      "created_at": row.get::<_, Option<String>>(15).unwrap_or(None),
      "updated_at": row.get::<_, Option<String>>(16).unwrap_or(None),
    }))).map(|rows| rows.filter_map(|r| r.ok()).collect())
  }).unwrap_or_default();

  let decisions: Vec<Value> = conn.prepare(
    "SELECT id, decision, context, type, source_agent, confidence, surprise, status, score, retrievals, last_accessed, pinned, parent_id, disputes_id, supersedes_id, confirmed_by, created_at, updated_at
     FROM decisions
     WHERE status = 'active'
     ORDER BY score DESC"
  ).and_then(|mut stmt| {
    stmt.query_map([], |row| Ok(json!({
      "id": row.get::<_, i64>(0)?,
      "decision": row.get::<_, String>(1).unwrap_or_default(),
      "context": row.get::<_, Option<String>>(2).unwrap_or(None),
      "type": row.get::<_, Option<String>>(3).unwrap_or(Some("decision".to_string())),
      "source_agent": row.get::<_, Option<String>>(4).unwrap_or(None),
      "confidence": row.get::<_, Option<f64>>(5).unwrap_or(Some(0.8)),
      "surprise": row.get::<_, Option<f64>>(6).unwrap_or(Some(1.0)),
      "status": row.get::<_, Option<String>>(7).unwrap_or(Some("active".to_string())),
      "score": row.get::<_, Option<f64>>(8).unwrap_or(Some(1.0)),
      "retrievals": row.get::<_, Option<i64>>(9).unwrap_or(Some(0)),
      "last_accessed": row.get::<_, Option<String>>(10).unwrap_or(None),
      "pinned": row.get::<_, Option<i64>>(11).unwrap_or(Some(0)),
      "parent_id": row.get::<_, Option<i64>>(12).unwrap_or(None),
      "disputes_id": row.get::<_, Option<i64>>(13).unwrap_or(None),
      "supersedes_id": row.get::<_, Option<i64>>(14).unwrap_or(None),
      "confirmed_by": row.get::<_, Option<String>>(15).unwrap_or(None),
      "created_at": row.get::<_, Option<String>>(16).unwrap_or(None),
      "updated_at": row.get::<_, Option<String>>(17).unwrap_or(None),
    }))).map(|rows| rows.filter_map(|r| r.ok()).collect())
  }).unwrap_or_default();

  json_response(StatusCode::OK, json!({
    "memories": memories,
    "decisions": decisions,
  }))
}

async fn handle_savings(State(state): State<RuntimeState>) -> Response {
  let conn = state.db.lock().await;
  let mut stmt = match conn.prepare(
    "SELECT data, created_at FROM events WHERE type = 'boot_savings' ORDER BY created_at ASC"
  ) {
    Ok(s) => s,
    Err(e) => return json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": e.to_string()})),
  };

  let rows: Vec<(String, String)> = stmt.query_map([], |row| {
    let data_str: String = row.get(0)?;
    let created: String = row.get(1)?;
    Ok((data_str, created))
  }).map(|iter| iter.filter_map(|r| r.ok()).collect()).unwrap_or_default();

  let points: Vec<Value> = rows.into_iter()
    .map(|(data_str, created)| {
      let d: Value = serde_json::from_str(&data_str).unwrap_or(json!({}));
      json!({
        "timestamp": created,
        "agent": d.get("agent").and_then(|v| v.as_str()).unwrap_or("unknown"),
        "served": d.get("served").and_then(|v| v.as_i64()).unwrap_or(0),
        "baseline": d.get("baseline").and_then(|v| v.as_i64()).unwrap_or(0),
        "saved": d.get("saved").and_then(|v| v.as_i64()).unwrap_or(0),
        "percent": d.get("percent").and_then(|v| v.as_i64()).unwrap_or(0),
      })
    }).collect();

  let total_saved: i64 = points.iter().map(|p| p["saved"].as_i64().unwrap_or(0)).sum();
  let total_served: i64 = points.iter().map(|p| p["served"].as_i64().unwrap_or(0)).sum();
  let total_baseline: i64 = points.iter().map(|p| p["baseline"].as_i64().unwrap_or(0)).sum();
  let avg_percent = if !points.is_empty() {
    points.iter().map(|p| p["percent"].as_i64().unwrap_or(0)).sum::<i64>() / points.len() as i64
  } else { 0 };

  // Daily aggregation
  let mut daily: std::collections::BTreeMap<String, (i64, i64, i64)> = std::collections::BTreeMap::new();
  for p in &points {
    let ts = p["timestamp"].as_str().unwrap_or("");
    let day = &ts[..ts.len().min(10)];
    if day.is_empty() { continue; }
    let e = daily.entry(day.to_string()).or_insert((0, 0, 0));
    e.0 += p["saved"].as_i64().unwrap_or(0);
    e.1 += p["served"].as_i64().unwrap_or(0);
    e.2 += 1;
  }
  let daily_arr: Vec<Value> = daily.into_iter().map(|(date, (saved, served, boots))| {
    json!({"date": date, "saved": saved, "served": served, "boots": boots})
  }).collect();

  let recent: Vec<&Value> = points.iter().rev().take(20).collect();

  json_response(StatusCode::OK, json!({
    "summary": { "totalSaved": total_saved, "totalServed": total_served, "totalBaseline": total_baseline, "avgPercent": avg_percent, "totalBoots": points.len() },
    "daily": daily_arr,
    "recent": recent,
  }))
}

async fn handle_forget(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Json(body): Json<ForgetRequest>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let keyword = body.keyword.or(body.source).unwrap_or_default();
  if keyword.trim().is_empty() {
    return json_response(
      StatusCode::BAD_REQUEST,
      json!({ "error": "Missing field: keyword" }),
    );
  }

  let mut conn = state.db.lock().await;
  match forget_keyword(&mut conn, keyword.trim()) {
    Ok(affected) => json_response(StatusCode::OK, json!({ "affected": affected })),
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Forget failed: {err}") }),
    ),
  }
}

async fn handle_resolve(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Json(body): Json<ResolveRequest>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let keep_id = match body.keep_id {
    Some(v) => v,
    None => {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({ "error": "Missing fields: keepId, action" }),
      )
    }
  };
  let action = match body.action {
    Some(v) if !v.trim().is_empty() => v,
    _ => {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({ "error": "Missing fields: keepId, action" }),
      )
    }
  };

  let mut conn = state.db.lock().await;
  match resolve_decision(&mut conn, keep_id, &action, body.superseded_id) {
    Ok(()) => json_response(StatusCode::OK, json!({ "resolved": true })),
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Resolve failed: {err}") }),
    ),
  }
}

async fn handle_conflicts(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let conn = state.db.lock().await;
  let mut stmt = match conn.prepare(
    "SELECT d1.id, d1.decision, d1.context, d1.source_agent, d1.confidence, d1.created_at,
            d2.id, d2.decision, d2.context, d2.source_agent, d2.confidence, d2.created_at
     FROM decisions d1
     JOIN decisions d2 ON d1.disputes_id = d2.id
     WHERE d1.status = 'disputed' AND d1.id > d2.id
     ORDER BY d1.created_at DESC",
  ) {
    Ok(s) => s,
    Err(e) => {
      return json_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({ "error": format!("Query failed: {e}") }),
      );
    }
  };

  let pairs: Vec<serde_json::Value> = match stmt.query_map([], |row| {
    Ok(json!({
      "left": {
        "id": row.get::<_, i64>(0)?,
        "decision": row.get::<_, String>(1)?,
        "context": row.get::<_, Option<String>>(2)?,
        "source_agent": row.get::<_, Option<String>>(3)?,
        "confidence": row.get::<_, Option<f64>>(4)?,
        "created_at": row.get::<_, Option<String>>(5)?,
      },
      "right": {
        "id": row.get::<_, i64>(6)?,
        "decision": row.get::<_, String>(7)?,
        "context": row.get::<_, Option<String>>(8)?,
        "source_agent": row.get::<_, Option<String>>(9)?,
        "confidence": row.get::<_, Option<f64>>(10)?,
        "created_at": row.get::<_, Option<String>>(11)?,
      },
    }))
  }) {
    Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
    Err(_) => vec![],
  };

  let count = pairs.len();
  json_response(StatusCode::OK, json!({ "pairs": pairs, "count": count }))
}

async fn handle_lock(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Json(body): Json<LockRequest>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let path = match body.path {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({ "error": "Missing required fields: path, agent" }),
      )
    }
  };
  let agent = match body.agent {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({ "error": "Missing required fields: path, agent" }),
      )
    }
  };

  let ttl = body.ttl.unwrap_or(300).max(1);
  let mut conn = state.db.lock().await;
  let _ = clean_expired_locks(&conn);

  let existing = conn
    .query_row(
      "SELECT id, agent, expires_at FROM locks WHERE path = ?1",
      params![path.clone()],
      |row| {
        Ok((
          row.get::<_, String>(0)?,
          row.get::<_, String>(1)?,
          row.get::<_, String>(2)?,
        ))
      },
    )
    .optional()
    .ok()
    .flatten();

  let now = Utc::now();
  let expires_at = (now + Duration::seconds(ttl)).to_rfc3339();
  if let Some((lock_id, holder, holder_expires)) = existing {
    if holder == agent {
      let _ = conn.execute(
        "UPDATE locks SET expires_at = ?1 WHERE path = ?2",
        params![expires_at.clone(), path],
      );
      checkpoint_wal_best_effort(&conn);
      return json_response(
        StatusCode::OK,
        json!({ "locked": true, "lockId": lock_id, "expiresAt": expires_at }),
      );
    }

    let minutes_left = {
      let target = parse_timestamp_ms(&holder_expires);
      let now_ms = Utc::now().timestamp_millis();
      ((target - now_ms) as f64 / 60000.0).ceil().max(0.0) as i64
    };
    return json_response(
      StatusCode::CONFLICT,
      json!({
        "error": "file_already_locked",
        "holder": holder,
        "expiresAt": holder_expires,
        "minutesLeft": minutes_left
      }),
    );
  }

  let lock_id = Uuid::new_v4().to_string();
  match conn.execute(
    "INSERT INTO locks (id, path, agent, locked_at, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)",
    params![lock_id.clone(), path.clone(), agent.clone(), now_iso(), expires_at.clone()],
  ) {
    Ok(_) => {
      checkpoint_wal_best_effort(&conn);
      state.emit(
        "lock",
        json!({ "action": "acquired", "path": path, "agent": agent }),
      );
      json_response(
        StatusCode::OK,
        json!({ "locked": true, "lockId": lock_id, "expiresAt": expires_at }),
      )
    }
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Lock failed: {err}") }),
    ),
  }
}

async fn handle_unlock(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Json(body): Json<LockRequest>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let path = match body.path {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({ "error": "Missing required fields: path, agent" }),
      )
    }
  };
  let agent = match body.agent {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({ "error": "Missing required fields: path, agent" }),
      )
    }
  };

  let mut conn = state.db.lock().await;
  let _ = clean_expired_locks(&conn);
  let holder = conn
    .query_row(
      "SELECT agent FROM locks WHERE path = ?1",
      params![path.clone()],
      |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten();

  let holder = match holder {
    Some(v) => v,
    None => return json_response(StatusCode::NOT_FOUND, json!({ "error": "no_lock_found" })),
  };

  if holder != agent {
    return json_response(
      StatusCode::FORBIDDEN,
      json!({ "error": "not_lock_holder", "holder": holder }),
    );
  }

  let _ = conn.execute("DELETE FROM locks WHERE path = ?1", params![path.clone()]);
  checkpoint_wal_best_effort(&conn);
  state.emit(
    "lock",
    json!({ "action": "released", "path": path, "agent": agent }),
  );
  json_response(StatusCode::OK, json!({ "unlocked": true }))
}

async fn handle_locks(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let mut conn = state.db.lock().await;
  let _ = clean_expired_locks(&conn);
  match fetch_locks(&conn) {
    Ok(locks) => json_response(StatusCode::OK, json!({ "locks": locks })),
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Get locks failed: {err}") }),
    ),
  }
}

async fn handle_post_activity(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Json(body): Json<ActivityRequest>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let agent = match body.agent {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({ "error": "Missing required fields: agent, description" }),
      )
    }
  };
  let description = match body.description {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({ "error": "Missing required fields: agent, description" }),
      )
    }
  };

  let files = body.files.unwrap_or_default();
  let id = Uuid::new_v4().to_string();
  let mut conn = state.db.lock().await;
  let _ = clean_old_activities(&conn);
  match conn.execute(
    "INSERT INTO activities (id, agent, description, files_json, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
    params![
      id.clone(),
      agent,
      description,
      serde_json::to_string(&files).unwrap_or_else(|_| "[]".to_string()),
      now_iso()
    ],
  ) {
    Ok(_) => {
      checkpoint_wal_best_effort(&conn);
      json_response(StatusCode::OK, json!({ "recorded": true, "activityId": id }))
    }
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Post activity failed: {err}") }),
    ),
  }
}

async fn handle_get_activity(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Query(query): Query<SinceQuery>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let since_secs = parse_duration_to_seconds(query.since.as_deref().unwrap_or("1h"));
  let cutoff = (Utc::now() - Duration::seconds(since_secs)).to_rfc3339();
  let conn = state.db.lock().await;

  let mut stmt = match conn.prepare(
    "SELECT id, agent, description, files_json, timestamp FROM activities WHERE timestamp >= ?1 ORDER BY timestamp ASC",
  ) {
    Ok(stmt) => stmt,
    Err(err) => {
      return json_response(
        StatusCode::INTERNAL_SERVER_ERROR,
        json!({ "error": format!("Get activity failed: {err}") }),
      )
    }
  };

  let rows = stmt.query_map(params![cutoff], |row| {
    let files: String = row.get(3)?;
    Ok(json!({
      "id": row.get::<_, String>(0)?,
      "agent": row.get::<_, String>(1)?,
      "description": row.get::<_, String>(2)?,
      "files": parse_json_array(&files),
      "timestamp": row.get::<_, String>(4)?
    }))
  });

  match rows {
    Ok(iter) => {
      let mut activities = Vec::new();
      for row in iter.flatten() {
        activities.push(row);
      }
      json_response(StatusCode::OK, json!({ "activities": activities }))
    }
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Get activity failed: {err}") }),
    ),
  }
}

async fn handle_post_message(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Json(body): Json<MessageRequest>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let from = match body.from {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({ "error": "Missing required fields: from, to, message" }),
      )
    }
  };
  let to = match body.to {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({ "error": "Missing required fields: from, to, message" }),
      )
    }
  };
  let message = match body.message {
    Some(v) if !v.trim().is_empty() => v,
    _ => {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({ "error": "Missing required fields: from, to, message" }),
      )
    }
  };

  let id = Uuid::new_v4().to_string();
  let mut conn = state.db.lock().await;
  let _ = clean_old_messages(&conn, &to);
  match conn.execute(
    "INSERT INTO messages (id, sender, recipient, message, timestamp) VALUES (?1, ?2, ?3, ?4, ?5)",
    params![id.clone(), from, to, message, now_iso()],
  ) {
    Ok(_) => {
      checkpoint_wal_best_effort(&conn);
      json_response(StatusCode::OK, json!({ "sent": true, "messageId": id }))
    }
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Post message failed: {err}") }),
    ),
  }
}

async fn handle_get_messages(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Query(query): Query<MessagesQuery>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let agent = match query.agent {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({ "error": "Missing parameter: agent" }),
      )
    }
  };

  let conn = state.db.lock().await;
  match fetch_messages_for_agent(&conn, &agent) {
    Ok(messages) => json_response(StatusCode::OK, json!({ "messages": messages })),
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Get messages failed: {err}") }),
    ),
  }
}

async fn handle_session_start(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Json(body): Json<SessionStartRequest>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let agent = match body.agent {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({ "error": "Missing required field: agent" }),
      )
    }
  };

  let ttl = body.ttl.unwrap_or(SESSION_TTL_SECONDS).max(1);
  let now = Utc::now();
  let session_id = Uuid::new_v4().to_string();
  let started_at = now.to_rfc3339();
  let expires_at = (now + Duration::seconds(ttl)).to_rfc3339();
  let files_json = serde_json::to_string(&body.files.unwrap_or_default()).unwrap_or_else(|_| "[]".to_string());

  let mut conn = state.db.lock().await;
  match conn.execute(
    "INSERT INTO sessions (agent, session_id, project, files_json, description, started_at, last_heartbeat, expires_at)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)
     ON CONFLICT(agent) DO UPDATE SET
       session_id = excluded.session_id,
       project = excluded.project,
       files_json = excluded.files_json,
       description = excluded.description,
       started_at = excluded.started_at,
       last_heartbeat = excluded.last_heartbeat,
       expires_at = excluded.expires_at",
    params![agent.clone(), session_id.clone(), body.project.clone(), files_json, body.description.clone(), started_at, expires_at],
  ) {
    Ok(_) => {
      checkpoint_wal_best_effort(&conn);
      state.emit("session", json!({ "action": "started", "agent": agent, "project": body.project }));
      json_response(
        StatusCode::OK,
        json!({ "sessionId": session_id, "heartbeatInterval": 60 }),
      )
    }
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Session start failed: {err}") }),
    ),
  }
}

async fn handle_session_heartbeat(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Json(body): Json<SessionHeartbeatRequest>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let agent = body.agent.unwrap_or_default().trim().to_string();
  if agent.is_empty() {
    return json_response(
      StatusCode::BAD_REQUEST,
      json!({ "error": "Missing or invalid required field: agent" }),
    );
  }
  if agent.len() > 100 {
    return json_response(
      StatusCode::BAD_REQUEST,
      json!({ "error": "Invalid agent: name too long (max 100 chars)" }),
    );
  }
  let agent_re = Regex::new(r"^[a-zA-Z0-9_-]+$").unwrap();
  if !agent_re.is_match(&agent) {
    return json_response(
      StatusCode::BAD_REQUEST,
      json!({ "error": "Invalid agent: name contains invalid characters (use alphanumeric, underscore, hyphen only)" }),
    );
  }

  let mut conn = state.db.lock().await;
  let _ = clean_expired_sessions(&conn);
  let exists = conn
    .query_row(
      "SELECT session_id FROM sessions WHERE agent = ?1",
      params![agent.clone()],
      |row| row.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten();
  if exists.is_none() {
    return json_response(StatusCode::NOT_FOUND, json!({ "error": "no_active_session" }));
  }

  let now = Utc::now();
  let expires_at = (now + Duration::seconds(SESSION_TTL_SECONDS)).to_rfc3339();
  let files_json = body
    .files
    .as_ref()
    .map(|f| serde_json::to_string(f).unwrap_or_else(|_| "[]".to_string()));
  match conn.execute(
    "UPDATE sessions SET
       last_heartbeat = ?1,
       expires_at = ?2,
       files_json = CASE WHEN ?3 IS NULL THEN files_json ELSE ?3 END,
       description = CASE WHEN ?4 IS NULL THEN description ELSE ?4 END
     WHERE agent = ?5",
    params![now.to_rfc3339(), expires_at.clone(), files_json, body.description, agent],
  ) {
    Ok(_) => {
      checkpoint_wal_best_effort(&conn);
      json_response(
        StatusCode::OK,
        json!({ "renewed": true, "expiresAt": expires_at }),
      )
    }
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Session heartbeat failed: {err}") }),
    ),
  }
}

async fn handle_session_end(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Json(body): Json<SessionEndRequest>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let agent = match body.agent {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({ "error": "Missing required field: agent" }),
      )
    }
  };

  let mut conn = state.db.lock().await;
  match conn.execute("DELETE FROM sessions WHERE agent = ?1", params![agent.clone()]) {
    Ok(_) => {
      checkpoint_wal_best_effort(&conn);
      state.emit("session", json!({ "action": "ended", "agent": agent }));
      json_response(StatusCode::OK, json!({ "ended": true }))
    }
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Session end failed: {err}") }),
    ),
  }
}

async fn handle_sessions(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let mut conn = state.db.lock().await;
  let _ = clean_expired_sessions(&conn);
  match fetch_sessions(&conn) {
    Ok(sessions) => json_response(StatusCode::OK, json!({ "sessions": sessions })),
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Get sessions failed: {err}") }),
    ),
  }
}

async fn handle_create_task(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Json(body): Json<TaskCreateRequest>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let title = match body.title {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({ "error": "Missing required field: title" }),
      )
    }
  };

  let task_id = Uuid::new_v4().to_string();
  let mut conn = state.db.lock().await;
  let _ = clean_old_tasks(&conn);
  let files_json = serde_json::to_string(&body.files.unwrap_or_default()).unwrap_or_else(|_| "[]".to_string());
  match conn.execute(
    "INSERT INTO tasks (task_id, title, description, project, files_json, priority, required_capability, status, claimed_by, created_at, claimed_at, completed_at, summary)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', NULL, ?8, NULL, NULL, NULL)",
    params![
      task_id.clone(),
      title.clone(),
      body.description,
      body.project,
      files_json,
      body.priority.unwrap_or_else(|| "medium".to_string()),
      body.required_capability.unwrap_or_else(|| "any".to_string()),
      now_iso()
    ],
  ) {
    Ok(_) => {
      checkpoint_wal_best_effort(&conn);
      state.emit("task", json!({ "action": "created", "taskId": task_id, "title": title }));
      json_response(StatusCode::CREATED, json!({ "taskId": task_id, "status": "pending" }))
    }
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Create task failed: {err}") }),
    ),
  }
}

async fn handle_get_tasks(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Query(query): Query<TaskQuery>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }
  let status_filter = query.status.unwrap_or_else(|| "pending".to_string());
  let project_filter = query.project;
  let conn = state.db.lock().await;
  match fetch_tasks(&conn, &status_filter, project_filter.as_deref()) {
    Ok(tasks) => json_response(StatusCode::OK, json!({ "tasks": tasks })),
    Err(err) => json_response(
      StatusCode::INTERNAL_SERVER_ERROR,
      json!({ "error": format!("Get tasks failed: {err}") }),
    ),
  }
}

async fn handle_claim_task(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Json(body): Json<TaskClaimRequest>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }
  let task_id = match body.task_id {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing required fields: taskId, agent" })),
  };
  let agent = match body.agent {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing required fields: taskId, agent" })),
  };

  let mut conn = state.db.lock().await;
  let row = conn
    .query_row(
      "SELECT status, claimed_by, title FROM tasks WHERE task_id = ?1",
      params![task_id.clone()],
      |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?)),
    )
    .optional()
    .ok()
    .flatten();
  let (status, claimed_by, title) = match row {
    Some(v) => v,
    None => return json_response(StatusCode::NOT_FOUND, json!({ "error": "task_not_found" })),
  };
  if status == "claimed" {
    return json_response(StatusCode::CONFLICT, json!({ "error": "task_already_claimed", "claimedBy": claimed_by }));
  }
  if status == "completed" {
    return json_response(StatusCode::CONFLICT, json!({ "error": "task_already_completed" }));
  }

  match conn.execute(
    "UPDATE tasks SET status = 'claimed', claimed_by = ?1, claimed_at = ?2 WHERE task_id = ?3",
    params![agent.clone(), now_iso(), task_id.clone()],
  ) {
    Ok(_) => {
      checkpoint_wal_best_effort(&conn);
      state.emit("task", json!({ "action": "claimed", "taskId": task_id, "title": title, "agent": agent }));
      json_response(StatusCode::OK, json!({ "claimed": true, "taskId": task_id }))
    }
    Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Claim task failed: {err}") })),
  }
}

async fn handle_complete_task(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Json(body): Json<TaskCompleteRequest>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }
  let task_id = match body.task_id {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing required fields: taskId, agent" })),
  };
  let agent = match body.agent {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing required fields: taskId, agent" })),
  };

  let mut conn = state.db.lock().await;
  let row = conn
    .query_row(
      "SELECT claimed_by, title, files_json FROM tasks WHERE task_id = ?1",
      params![task_id.clone()],
      |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)),
    )
    .optional()
    .ok()
    .flatten();
  let (claimed_by, title, files_json) = match row {
    Some(v) => v,
    None => return json_response(StatusCode::NOT_FOUND, json!({ "error": "task_not_found" })),
  };
  if claimed_by.as_deref() != Some(agent.as_str()) {
    return json_response(StatusCode::FORBIDDEN, json!({ "error": "not_task_holder", "claimedBy": claimed_by }));
  }

  match conn.execute(
    "UPDATE tasks SET status = 'completed', completed_at = ?1, summary = ?2 WHERE task_id = ?3",
    params![now_iso(), body.summary.clone(), task_id.clone()],
  ) {
    Ok(_) => {
      state.emit("task", json!({ "action": "completed", "taskId": task_id, "title": title, "agent": agent }));

      let posted: i64 = conn
        .query_row(
          "SELECT COUNT(*) FROM feed WHERE task_id = ?1 AND kind = 'task_complete'",
          params![task_id.clone()],
          |r| r.get(0),
        )
        .unwrap_or(0);
      if posted == 0 {
        let entry = FeedEntry {
          id: Uuid::new_v4().to_string(),
          agent: agent.clone(),
          kind: "task_complete".to_string(),
          summary: redact_secrets(&format!("Completed: {title}")),
          content: body.summary.map(|s| redact_secrets(&s)),
          files: parse_json_array(&files_json),
          task_id: Some(task_id.clone()),
          trace_id: None,
          priority: "normal".to_string(),
          timestamp: now_iso(),
          tokens: ((title.len() as f64) / 4.0).ceil() as i64,
        };
        let _ = insert_feed_entry(&conn, &entry);
        state.emit(
          "feed",
          json!({ "feedId": entry.id, "agent": entry.agent, "kind": entry.kind, "summary": entry.summary }),
        );
      }
      checkpoint_wal_best_effort(&conn);
      json_response(StatusCode::OK, json!({ "completed": true, "taskId": task_id }))
    }
    Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Complete task failed: {err}") })),
  }
}

async fn handle_abandon_task(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Json(body): Json<TaskAbandonRequest>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }
  let task_id = match body.task_id {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing required fields: taskId, agent" })),
  };
  let agent = match body.agent {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing required fields: taskId, agent" })),
  };

  let mut conn = state.db.lock().await;
  let row = conn
    .query_row(
      "SELECT claimed_by, title FROM tasks WHERE task_id = ?1",
      params![task_id.clone()],
      |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?)),
    )
    .optional()
    .ok()
    .flatten();
  let (claimed_by, title) = match row {
    Some(v) => v,
    None => return json_response(StatusCode::NOT_FOUND, json!({ "error": "task_not_found" })),
  };
  if claimed_by.as_deref() != Some(agent.as_str()) {
    return json_response(StatusCode::FORBIDDEN, json!({ "error": "not_task_holder", "claimedBy": claimed_by }));
  }

  match conn.execute(
    "UPDATE tasks SET status = 'pending', claimed_by = NULL, claimed_at = NULL WHERE task_id = ?1",
    params![task_id.clone()],
  ) {
    Ok(_) => {
      checkpoint_wal_best_effort(&conn);
      state.emit("task", json!({ "action": "abandoned", "taskId": task_id, "title": title, "agent": agent }));
      json_response(StatusCode::OK, json!({ "abandoned": true, "taskId": task_id, "status": "pending" }))
    }
    Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Abandon task failed: {err}") })),
  }
}

async fn handle_next_task(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Query(query): Query<NextTaskQuery>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let _agent = match query.agent {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing parameter: agent" })),
  };
  let capability = query.capability.unwrap_or_else(|| "any".to_string());
  let conn = state.db.lock().await;

  let mut stmt = match conn.prepare(
    "SELECT task_id, title, description, project, files_json, priority, required_capability, status, claimed_by, created_at, claimed_at, completed_at, summary
     FROM tasks
     WHERE status = 'pending'
       AND (?1 = 'any' OR required_capability = 'any' OR required_capability = ?1)
     ORDER BY
       CASE priority
         WHEN 'critical' THEN 4
         WHEN 'high' THEN 3
         WHEN 'medium' THEN 2
         WHEN 'low' THEN 1
         ELSE 0
       END DESC,
       created_at ASC
     LIMIT 1",
  ) {
    Ok(stmt) => stmt,
    Err(err) => return json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Get next task failed: {err}") })),
  };

  let task = stmt
    .query_row(params![capability], |row| {
      Ok(json!({
        "taskId": row.get::<_, String>(0)?,
        "title": row.get::<_, String>(1)?,
        "description": row.get::<_, Option<String>>(2)?,
        "project": row.get::<_, Option<String>>(3)?,
        "files": parse_json_array(&row.get::<_, String>(4)?),
        "priority": row.get::<_, String>(5)?,
        "requiredCapability": row.get::<_, String>(6)?,
        "status": row.get::<_, String>(7)?,
        "claimedBy": row.get::<_, Option<String>>(8)?,
        "createdAt": row.get::<_, String>(9)?,
        "claimedAt": row.get::<_, Option<String>>(10)?,
        "completedAt": row.get::<_, Option<String>>(11)?,
        "summary": row.get::<_, Option<String>>(12)?
      }))
    })
    .optional()
    .ok()
    .flatten();

  json_response(StatusCode::OK, json!({ "task": task }))
}

async fn handle_post_feed(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Json(body): Json<FeedRequest>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }
  let agent = match body.agent {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing required fields: agent, kind, summary" })),
  };
  let kind = match body.kind {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing required fields: agent, kind, summary" })),
  };
  let summary = match body.summary {
    Some(v) if !v.trim().is_empty() => v,
    _ => return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing required fields: agent, kind, summary" })),
  };

  let entry = FeedEntry {
    id: Uuid::new_v4().to_string(),
    agent: agent.clone(),
    kind: kind.clone(),
    summary: redact_secrets(&summary),
    content: body.content.map(|c| redact_secrets(&c)),
    files: serde_json::to_value(body.files.unwrap_or_default()).unwrap_or_else(|_| json!([])),
    task_id: body.task_id,
    trace_id: body.trace_id,
    priority: body.priority.unwrap_or_else(|| "normal".to_string()),
    timestamp: now_iso(),
    tokens: ((summary.len() as f64) / 4.0).ceil() as i64,
  };

  let mut conn = state.db.lock().await;
  let _ = clean_old_feed(&conn);
  match insert_feed_entry(&conn, &entry) {
    Ok(()) => {
      checkpoint_wal_best_effort(&conn);
      state.emit(
        "feed",
        json!({ "feedId": entry.id, "agent": agent, "kind": kind, "summary": entry.summary }),
      );
      json_response(StatusCode::CREATED, json!({ "feedId": entry.id, "recorded": true }))
    }
    Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Post feed failed: {err}") })),
  }
}

async fn handle_get_feed(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Query(query): Query<FeedQuery>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let mut conn = state.db.lock().await;
  let _ = clean_old_feed(&conn);
  let since = query.since.unwrap_or_else(|| "1h".to_string());
  let cutoff = (Utc::now() - Duration::seconds(parse_duration_to_seconds(&since))).to_rfc3339();

  let mut entries = if query.unread.unwrap_or(false) {
    if let Some(agent) = query.agent.as_deref() {
      get_unread_feed(&conn, agent).unwrap_or_default()
    } else {
      vec![]
    }
  } else {
    fetch_feed_since(&conn, &cutoff).unwrap_or_default()
  };

  if let Some(kind) = query.kind {
    entries.retain(|e| e.kind == kind);
  }

  let slim = entries
    .iter()
    .map(|e| feed_to_json(e, false))
    .collect::<Vec<_>>();
  json_response(StatusCode::OK, json!({ "entries": slim }))
}

async fn handle_get_feed_by_id(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Path(feed_id): Path<String>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }

  let conn = state.db.lock().await;
  let entry = conn
    .query_row(
      "SELECT id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens FROM feed WHERE id = ?1",
      params![feed_id],
      |row| {
        Ok(FeedEntry {
          id: row.get(0)?,
          agent: row.get(1)?,
          kind: row.get(2)?,
          summary: row.get(3)?,
          content: row.get(4)?,
          files: parse_json_array(&row.get::<_, String>(5)?),
          task_id: row.get(6)?,
          trace_id: row.get(7)?,
          priority: row.get(8)?,
          timestamp: row.get(9)?,
          tokens: row.get(10)?,
        })
      },
    )
    .optional()
    .ok()
    .flatten();

  match entry {
    Some(entry) => json_response(StatusCode::OK, feed_to_json(&entry, true)),
    None => json_response(StatusCode::NOT_FOUND, json!({ "error": "feed_entry_not_found" })),
  }
}

async fn handle_feed_ack(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  Json(body): Json<FeedAckRequest>,
) -> Response {
  if let Err(resp) = ensure_auth(&headers, &state) {
    return resp;
  }
  let agent = match body.agent {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing required fields: agent, lastSeenId" })),
  };
  let last_seen_id = match body.last_seen_id {
    Some(v) if !v.trim().is_empty() => v.trim().to_string(),
    _ => return json_response(StatusCode::BAD_REQUEST, json!({ "error": "Missing required fields: agent, lastSeenId" })),
  };

  let mut conn = state.db.lock().await;
  match conn.execute(
    "INSERT INTO feed_acks (agent, last_seen_id, updated_at) VALUES (?1, ?2, ?3)
     ON CONFLICT(agent) DO UPDATE SET last_seen_id = excluded.last_seen_id, updated_at = excluded.updated_at",
    params![agent, last_seen_id, now_iso()],
  ) {
    Ok(_) => {
      checkpoint_wal_best_effort(&conn);
      json_response(StatusCode::OK, json!({ "acked": true }))
    }
    Err(err) => json_response(StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": format!("Feed ack failed: {err}") })),
  }
}

async fn handle_events_stream(State(state): State<RuntimeState>) -> Response {
  let initial = stream::once(async move {
    Ok::<Event, Infallible>(
      Event::default()
        .event("connected")
        .data(json!({ "timestamp": now_iso(), "clients": 1 }).to_string()),
    )
  });

  let updates = BroadcastStream::new(state.events.subscribe()).filter_map(|msg| async move {
    match msg {
      Ok(event) => {
        let payload = match event.data {
          Value::Object(mut map) => {
            map.insert("type".to_string(), Value::String(event.event_type.clone()));
            map.insert("timestamp".to_string(), Value::String(now_iso()));
            Value::Object(map)
          }
          other => json!({
            "type": event.event_type,
            "data": other,
            "timestamp": now_iso()
          }),
        };
        Some(Ok::<Event, Infallible>(
          Event::default()
            .event(payload.get("type").and_then(|v| v.as_str()).unwrap_or("event"))
            .data(payload.to_string()),
        ))
      }
      Err(_) => None,
    }
  });

  let stream = initial.chain(updates);
  let sse = Sse::new(stream)
    .keep_alive(KeepAlive::new().interval(StdDuration::from_secs(30)).text("keepalive"));
  let mut response = sse.into_response();
  response
    .headers_mut()
    .insert("Access-Control-Allow-Origin", HeaderValue::from_static("*"));
  response
}

async fn handle_mcp_get() -> Response {
  json_response(
    StatusCode::METHOD_NOT_ALLOWED,
    json!({ "error": "SSE streaming not implemented. Use POST for requests." }),
  )
}

async fn handle_mcp_post(
  State(state): State<RuntimeState>,
  headers: HeaderMap,
  body: Bytes,
) -> Response {
  let origin = headers
    .get("origin")
    .and_then(|h| h.to_str().ok())
    .unwrap_or("");
  if !origin.is_empty() && origin != "http://localhost" && origin != "http://127.0.0.1" {
    return json_response(
      StatusCode::FORBIDDEN,
      json!({ "error": "Forbidden: invalid origin" }),
    );
  }

  let protocol_version = headers
    .get("mcp-protocol-version")
    .and_then(|h| h.to_str().ok())
    .unwrap_or("2024-11-05")
    .to_string();

  let msg: Value = match serde_json::from_slice(&body) {
    Ok(v) => v,
    Err(_) => {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({ "jsonrpc": "2.0", "error": { "code": -32700, "message": "Parse error" }, "id": Value::Null }),
      )
    }
  };

  if msg.get("jsonrpc") != Some(&Value::String("2.0".to_string())) {
    return json_response(
      StatusCode::BAD_REQUEST,
      json!({
        "jsonrpc": "2.0",
        "error": { "code": -32600, "message": "Invalid JSON-RPC version" },
        "id": msg.get("id").cloned().unwrap_or(Value::Null)
      }),
    );
  }

  let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or_default();
  let mut session_id = headers
    .get("mcp-session-id")
    .and_then(|h| h.to_str().ok())
    .map(|s| s.to_string());

  {
    let mut sessions = state.mcp_sessions.lock().await;
    if method == "initialize" {
      let id = Uuid::new_v4().simple().to_string();
      sessions.insert(id.clone(), Utc::now().timestamp_millis());
      session_id = Some(id);
    } else if session_id.is_none() {
      return json_response(
        StatusCode::BAD_REQUEST,
        json!({
          "jsonrpc": "2.0",
          "error": { "code": -32600, "message": "Missing Mcp-Session-Id header" },
          "id": msg.get("id").cloned().unwrap_or(Value::Null)
        }),
      );
    }
    if let Some(id) = session_id.as_ref() {
      sessions.insert(id.clone(), Utc::now().timestamp_millis());
    }
  }

  let response_payload = match handle_mcp_message(&state, &msg).await {
    Ok(value) => value,
    Err(err) => Some(json!({
      "jsonrpc": "2.0",
      "error": { "code": -32603, "message": format!("Internal error: {err}") },
      "id": msg.get("id").cloned().unwrap_or(Value::Null)
    })),
  };

  if msg.get("id").is_none() || response_payload.is_none() {
    let mut response = StatusCode::ACCEPTED.into_response();
    apply_json_headers(response.headers_mut());
    return response;
  }

  let mut response = (StatusCode::OK, Json(response_payload.unwrap())).into_response();
  apply_json_headers(response.headers_mut());
  response.headers_mut().insert(
    "MCP-Protocol-Version",
    HeaderValue::from_str(&protocol_version).unwrap_or_else(|_| HeaderValue::from_static("2024-11-05")),
  );
  if let Some(session_id) = session_id {
    if let Ok(value) = HeaderValue::from_str(&session_id) {
      response.headers_mut().insert("Mcp-Session-Id", value);
    }
  }
  response
}

async fn handle_mcp_message(state: &RuntimeState, msg: &Value) -> Result<Option<Value>, String> {
  let id = msg.get("id").cloned().unwrap_or(Value::Null);
  let method = msg
    .get("method")
    .and_then(|v| v.as_str())
    .unwrap_or_default();

  match method {
    "initialize" => Ok(Some(mcp_success(
      id,
      json!({
        "protocolVersion": "2024-11-05",
        "capabilities": { "tools": { "listChanged": true } },
        "serverInfo": { "name": "cortex", "version": "2.0.0" }
      }),
    ))),
    "notifications/initialized" => Ok(None),
    "tools/list" => Ok(Some(mcp_success(id, json!({ "tools": mcp_tools() })))),
    "tools/call" => {
      let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
      let tool_name = params
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
      if tool_name.is_empty() {
        return Ok(Some(mcp_error(id, -32602, "Missing tool name")));
      }

      let known = mcp_tools().iter().any(|tool| {
        tool
          .get("name")
          .and_then(|v| v.as_str())
          .map(|name| name == tool_name)
          .unwrap_or(false)
      });
      if !known {
        return Ok(Some(mcp_error(id, -32601, &format!("Unknown tool: {tool_name}"))));
      }

      let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

      match mcp_dispatch(state, tool_name, &args).await {
        Ok(result) => Ok(Some(mcp_success(id, wrap_mcp_tool_result(state, result)))),
        Err(err) => Ok(Some(mcp_success(
          id,
          json!({
            "content": [{
              "type": "text",
              "text": json!({
                "error": err,
                "_liveness": true,
                "_ts": now_iso(),
                "_calls": state.next_mcp_call()
              }).to_string()
            }],
            "isError": true
          }),
        ))),
      }
    }
    _ => {
      if msg.get("id").is_some() {
        Ok(Some(mcp_error(
          id,
          -32601,
          &format!("Method not found: {method}"),
        )))
      } else {
        Ok(None)
      }
    }
  }
}

async fn mcp_dispatch(state: &RuntimeState, tool_name: &str, args: &Value) -> Result<Value, String> {
  match tool_name {
    "cortex_boot" => {
      let profile = args.get("profile").and_then(|v| v.as_str()).map(str::to_string);
      let agent = args
        .get("source_agent")
        .and_then(|v| v.as_str())
        .or_else(|| args.get("agent").and_then(|v| v.as_str()))
        .map(str::to_string);
      let budget = args.get("budget").and_then(|v| v.as_u64()).map(|v| v as usize);
      let response = handle_boot(
        State(state.clone()),
        Query(BootQuery {
          profile,
          agent,
          budget,
        }),
        HeaderMap::new(),
      )
      .await;
      response_to_json(response).await
    }
    "cortex_peek" => {
      let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required argument: query".to_string())?;
      let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
      let mut conn = state.db.lock().await;
      let results = run_recall(&mut conn, query, limit)?;
      let matches: Vec<Value> = results.iter().map(|r| json!({
        "source": r.source, "relevance": r.relevance, "method": r.method,
      })).collect();
      Ok(json!({ "count": matches.len(), "matches": matches }))
    }
    "cortex_recall" => {
      let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required argument: query".to_string())?;
      let budget = args
        .get("budget")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(200);
      let agent = args
        .get("source_agent")
        .and_then(|v| v.as_str())
        .or_else(|| args.get("agent").and_then(|v| v.as_str()))
        .unwrap_or("mcp");
      execute_unified_recall(state, query, budget, 10, agent).await
    }
    "cortex_store" => {
      let decision = args
        .get("decision")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required argument: decision".to_string())?;
      let context = args.get("context").and_then(|v| v.as_str()).map(str::to_string);
      let entry_type = args.get("type").and_then(|v| v.as_str()).map(str::to_string);
      let source_agent = args
        .get("source_agent")
        .and_then(|v| v.as_str())
        .unwrap_or("mcp")
        .to_string();
      let confidence = args.get("confidence").and_then(|v| v.as_f64());
      let mut conn = state.db.lock().await;
      let entry = store_decision(&mut conn, decision, context, entry_type, source_agent, confidence)?;
      Ok(json!({ "stored": true, "entry": entry }))
    }
    "cortex_health" => {
      let response = handle_health(State(state.clone())).await;
      response_to_json(response).await
    }
    "cortex_digest" => {
      let conn = state.db.lock().await;
      build_digest(&conn)
    }
    "cortex_forget" => {
      let keyword = args
        .get("source")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required argument: source".to_string())?;
      let mut conn = state.db.lock().await;
      let affected = forget_keyword(&mut conn, keyword)?;
      Ok(json!({ "affected": affected }))
    }
    "cortex_resolve" => {
      let keep_id = args
        .get("keepId")
        .and_then(|v| v.as_i64())
        .ok_or_else(|| "Missing required arguments: keepId, action".to_string())?;
      let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing required arguments: keepId, action".to_string())?;
      let superseded_id = args.get("supersededId").and_then(|v| v.as_i64());
      let mut conn = state.db.lock().await;
      resolve_decision(&mut conn, keep_id, action, superseded_id)?;
      Ok(json!({ "resolved": true }))
    }
    _ => Err(format!("Unknown tool: {tool_name}")),
  }
}

fn mcp_tools() -> Vec<Value> {
  vec![
    json!({
      "name": "cortex_boot",
      "description": "Get compiled boot prompt with session context. Uses capsule system: identity (stable) + delta (what changed since your last boot). Call once at session start.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "profile": { "type": "string", "description": "Legacy profile name. Ignored when agent is set." },
          "agent": { "type": "string", "description": "Your agent ID (e.g. claude-opus, gemini, codex). Enables delta tracking." },
          "budget": { "type": "number", "description": "Max token budget for boot prompt (default: 600)" }
        }
      }
    }),
    json!({
      "name": "cortex_peek",
      "description": "Lightweight check: returns source names and relevance scores only (no excerpts). Use BEFORE cortex_recall to check if relevant memories exist. Saves ~80% tokens vs full recall.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "query": { "type": "string", "description": "Search query text" },
          "limit": { "type": "number", "description": "Max results (default 10)" }
        },
        "required": ["query"]
      }
    }),
    json!({
      "name": "cortex_recall",
      "description": "Search Cortex brain for memories and decisions. Adapts detail level to token budget: 0=headlines, 200=balanced, 500+=full.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "query": { "type": "string", "description": "Search query text" },
          "budget": { "type": "number", "description": "Token budget. 0=headlines only, 200=balanced, 500+=full detail" },
          "agent": { "type": "string", "description": "Optional agent id for dedup/predictive cache" }
        },
        "required": ["query"]
      }
    }),
    json!({
      "name": "cortex_store",
      "description": "Store a decision or insight with conflict detection and dedup.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "decision": { "type": "string", "description": "The decision or insight text" },
          "context": { "type": "string", "description": "Optional context about where/why" },
          "type": { "type": "string", "description": "Entry type (default: decision)" },
          "source_agent": { "type": "string", "description": "Agent that produced this" },
          "confidence": { "type": "number", "description": "Confidence score 0-1 (default: 0.8)" }
        },
        "required": ["decision"]
      }
    }),
    json!({
      "name": "cortex_health",
      "description": "Check Cortex system health: DB stats, Ollama status, memory counts.",
      "inputSchema": { "type": "object", "properties": {} }
    }),
    json!({
      "name": "cortex_digest",
      "description": "Daily health digest: memory counts, today's activity, top recalls, decay stats, agent boots. Use to check if the brain is compounding.",
      "inputSchema": { "type": "object", "properties": {} }
    }),
    json!({
      "name": "cortex_forget",
      "description": "Decay matching memories/decisions by keyword (multiply score by 0.3).",
      "inputSchema": {
        "type": "object",
        "properties": { "source": { "type": "string", "description": "Keyword to match for decay" } },
        "required": ["source"]
      }
    }),
    json!({
      "name": "cortex_resolve",
      "description": "Resolve a disputed decision pair.",
      "inputSchema": {
        "type": "object",
        "properties": {
          "keepId": { "type": "number", "description": "ID of the decision to keep" },
          "action": { "type": "string", "enum": ["keep", "merge"], "description": "Resolution action" },
          "supersededId": { "type": "number", "description": "ID of the decision to supersede (for keep action)" }
        },
        "required": ["keepId", "action"]
      }
    }),
  ]
}

fn mcp_success(id: Value, result: Value) -> Value {
  json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn mcp_error(id: Value, code: i64, message: &str) -> Value {
  json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn wrap_mcp_tool_result(state: &RuntimeState, data: Value) -> Value {
  let calls = state.next_mcp_call();
  let decorated = match data {
    Value::Object(mut map) => {
      map.insert("_liveness".to_string(), Value::Bool(true));
      map.insert("_ts".to_string(), Value::String(now_iso()));
      map.insert("_calls".to_string(), Value::Number(calls.into()));
      Value::Object(map)
    }
    other => json!({
      "value": other,
      "_liveness": true,
      "_ts": now_iso(),
      "_calls": calls
    }),
  };

  json!({
    "content": [{
      "type": "text",
      "text": decorated.to_string()
    }]
  })
}

async fn response_to_json(response: Response) -> Result<Value, String> {
  let status = response.status();
  let body = response.into_body();
  let bytes = axum::body::to_bytes(body, usize::MAX)
    .await
    .map_err(|err| format!("Failed to read response body: {err}"))?;
  let value: Value = serde_json::from_slice(&bytes).unwrap_or_else(|_| json!({}));
  if !status.is_success() {
    return Err(
      value
        .get("error")
        .and_then(|v| v.as_str())
        .unwrap_or("Request failed")
        .to_string(),
    );
  }
  Ok(value)
}

async fn execute_unified_recall(
  state: &RuntimeState,
  query_text: &str,
  budget: usize,
  k: usize,
  agent: &str,
) -> Result<Value, String> {
  if budget > 0 {
    if let Some(cached) = get_pre_cached(state, agent, query_text).await {
      let deduped_cached = dedup_and_mark_served(state, agent, cached).await;
      return Ok(json!({
        "results": deduped_cached.into_iter().map(recall_to_json).collect::<Vec<_>>(),
        "budget": budget,
        "spent": 0,
        "saved": budget as i64,
        "mode": if budget >= 500 { "full" } else { "balanced" },
        "cached": true
      }));
    }
  }

  let mut conn = state.db.lock().await;
  let mut results = if budget == 0 {
    run_recall(&mut conn, query_text, k)?
  } else {
    run_budget_recall(&mut conn, query_text, budget, k)?
  };

  let sources: Vec<String> = results.iter().map(|item| item.source.clone()).collect();
  let predictions = if sources.len() >= 2 {
    if record_co_occurrence(&conn, &sources).is_ok() {
      checkpoint_wal_best_effort(&conn);
    } else {
      let _ = reset_co_occurrence_table(&conn);
    }

    match predict_from_co_occurrence(&conn, &sources, 3) {
      Ok(predictions) => predictions,
      Err(_) => {
        let _ = reset_co_occurrence_table(&conn);
        vec![]
      }
    }
  } else {
    vec![]
  };
  drop(conn);

  record_recall_pattern(state, agent, query_text).await;
  let state_clone = state.clone();
  let agent_owned = agent.to_string();
  let query_owned = query_text.to_string();
  tokio::spawn(async move {
    let _ = predict_and_cache(state_clone, &agent_owned, &query_owned).await;
  });

  if budget == 0 {
    let headlines = results
      .iter()
      .map(|item| {
        json!({
          "source": item.source,
          "relevance": item.relevance,
          "method": item.method
        })
      })
      .collect::<Vec<_>>();
    return Ok(json!({
      "count": headlines.len(),
      "results": headlines,
      "budget": 0,
      "spent": 0,
      "mode": "headlines"
    }));
  }

  results = dedup_and_mark_served(state, agent, results).await;
  let spent: usize = results
    .iter()
    .map(|item| {
      item
        .tokens
        .unwrap_or_else(|| estimate_tokens(&format!("{}{}", item.source, item.excerpt)))
    })
    .sum();
  let saved = budget as i64 - spent as i64;

  let mut payload = json!({
    "results": results.into_iter().map(recall_to_json).collect::<Vec<_>>(),
    "budget": budget,
    "spent": spent,
    "saved": saved,
    "mode": if budget >= 500 { "full" } else { "balanced" }
  });

  if let Value::Object(ref mut map) = payload {
    if !predictions.is_empty() {
      map.insert("predictions".to_string(), Value::Array(predictions));
    }
  }

  Ok(payload)
}

fn run_budget_recall(
  conn: &mut Connection,
  query_text: &str,
  token_budget: usize,
  k: usize,
) -> Result<Vec<RecallItem>, String> {
  let raw = run_recall(conn, query_text, k)?;
  if raw.is_empty() {
    return Ok(vec![]);
  }

  let mut spent = 0usize;
  let mut budgeted = Vec::new();
  for (idx, item) in raw.into_iter().enumerate() {
    let remaining = token_budget.saturating_sub(spent);
    if remaining <= 10 {
      break;
    }

    let max_chars = if idx == 0 {
      ((remaining as f64 * 3.8) as usize).min(400)
    } else if idx <= 2 {
      ((remaining as f64 * 3.8) as usize).min(150)
    } else {
      ((remaining as f64 * 3.8) as usize).min(60)
    };

    let original = item.excerpt.clone();
    let mut excerpt = truncate_chars(&original, max_chars);
    if excerpt.chars().count() < original.chars().count() {
      excerpt.push_str("...");
    }
    let tokens = estimate_tokens(&format!("{}{}", item.source, excerpt));
    spent += tokens;

    budgeted.push(RecallItem {
      source: item.source,
      relevance: item.relevance,
      excerpt,
      method: item.method,
      tokens: Some(tokens),
    });
  }

  Ok(budgeted)
}

fn record_co_occurrence(conn: &Connection, sources: &[String]) -> Result<(), String> {
  if sources.len() < 2 {
    return Ok(());
  }

  let unique = sources
    .iter()
    .filter(|source| !source.trim().is_empty())
    .cloned()
    .collect::<HashSet<_>>()
    .into_iter()
    .take(10)
    .collect::<Vec<_>>();
  if unique.len() < 2 {
    return Ok(());
  }

  for i in 0..unique.len() {
    for j in (i + 1)..unique.len() {
      let (a, b) = if unique[i] <= unique[j] {
        (unique[i].clone(), unique[j].clone())
      } else {
        (unique[j].clone(), unique[i].clone())
      };
      conn
        .execute(
          "INSERT INTO co_occurrence (source_a, source_b, count, last_seen)
           VALUES (?1, ?2, 1, datetime('now'))
           ON CONFLICT(source_a, source_b) DO UPDATE SET
             count = count + 1,
             last_seen = datetime('now')",
          params![a, b],
        )
        .map_err(|e| e.to_string())?;
    }
  }

  Ok(())
}

fn predict_from_co_occurrence(
  conn: &Connection,
  recalled_sources: &[String],
  limit: usize,
) -> Result<Vec<Value>, String> {
  if recalled_sources.is_empty() {
    return Ok(vec![]);
  }

  let already_have = recalled_sources
    .iter()
    .filter(|source| !source.trim().is_empty())
    .cloned()
    .collect::<HashSet<_>>();
  let mut candidates: HashMap<String, i64> = HashMap::new();

  for source in &already_have {
    let mut stmt = conn
      .prepare(
        "SELECT
          CASE WHEN source_a = ?1 THEN source_b ELSE source_a END AS partner,
          count
         FROM co_occurrence
         WHERE source_a = ?1 OR source_b = ?1
         ORDER BY count DESC
         LIMIT 10",
      )
      .map_err(|e| e.to_string())?;

    let rows = stmt
      .query_map(params![source], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
      })
      .map_err(|e| e.to_string())?;

    for row in rows.flatten() {
      let (partner, count) = row;
      if already_have.contains(&partner) {
        continue;
      }
      let existing = candidates.get(&partner).copied().unwrap_or(0);
      candidates.insert(partner, existing + count);
    }
  }

  let mut ranked = candidates.into_iter().collect::<Vec<_>>();
  ranked.sort_by(|a, b| b.1.cmp(&a.1));
  ranked.truncate(limit);

  Ok(ranked
    .into_iter()
    .map(|(source, score)| json!({ "source": source, "coScore": score }))
    .collect())
}

fn reset_co_occurrence_table(conn: &Connection) -> Result<(), String> {
  conn
    .execute_batch(
      "DROP TABLE IF EXISTS co_occurrence;
       CREATE TABLE IF NOT EXISTS co_occurrence (
         source_a TEXT NOT NULL,
         source_b TEXT NOT NULL,
         count INTEGER DEFAULT 1,
         last_seen TEXT DEFAULT (datetime('now')),
         PRIMARY KEY (source_a, source_b)
       );",
    )
    .map_err(|e| e.to_string())
}

async fn record_recall_pattern(state: &RuntimeState, agent: &str, query: &str) {
  let mut history = state.recall_history.lock().await;
  let entries = history
    .entry(agent.to_string())
    .or_insert_with(Vec::<RecallHistoryEntry>::new);
  entries.push(RecallHistoryEntry {
    query: query.to_string(),
    timestamp: Utc::now().timestamp_millis(),
  });
  if entries.len() > MAX_RECALL_HISTORY {
    let overflow = entries.len() - MAX_RECALL_HISTORY;
    entries.drain(0..overflow);
  }
}

async fn get_pre_cached(
  state: &RuntimeState,
  agent: &str,
  query: &str,
) -> Option<Vec<RecallItem>> {
  let mut cache = state.pre_cache.lock().await;
  let now = Utc::now().timestamp_millis();
  if let Some(entry) = cache.get(agent) {
    if entry.query == query && entry.expires_at > now {
      return Some(entry.results.clone());
    }
  }

  let should_remove = cache
    .get(agent)
    .map(|entry| entry.expires_at <= now)
    .unwrap_or(false);
  if should_remove {
    cache.remove(agent);
  }
  None
}

async fn predict_and_cache(
  state: RuntimeState,
  agent: &str,
  current_query: &str,
) -> Result<(), String> {
  let predicted_query = {
    let history = state.recall_history.lock().await;
    let entries = match history.get(agent) {
      Some(entries) if entries.len() >= 3 => entries,
      _ => return Ok(()),
    };

    let mut followers: HashMap<String, (i64, i64)> = HashMap::new();
    for pair in entries.windows(2) {
      if pair[0].query == current_query {
        let next_query = pair[1].query.clone();
        let entry = followers.entry(next_query).or_insert((0, 0));
        entry.0 += 1;
        entry.1 = entry.1.max(pair[1].timestamp);
      }
    }

    followers
      .into_iter()
      .filter(|(query, _)| query != current_query)
      .max_by(|a, b| {
        a.1
          .0
          .cmp(&b.1.0)
          .then_with(|| a.1.1.cmp(&b.1.1))
          .then_with(|| b.0.cmp(&a.0))
      })
      .map(|(query, _)| query)
  };

  let predicted_query = match predicted_query {
    Some(query) if !query.trim().is_empty() => query,
    _ => return Ok(()),
  };

  let mut conn = state.db.lock().await;
  let results = run_budget_recall(&mut conn, &predicted_query, 200, 5)?;
  drop(conn);
  if results.is_empty() {
    return Ok(());
  }

  let mut cache = state.pre_cache.lock().await;
  cache.insert(
    agent.to_string(),
    PreCacheEntry {
      query: predicted_query,
      results,
      expires_at: Utc::now().timestamp_millis() + PRECACHE_TTL_MS,
    },
  );
  Ok(())
}

fn hash_content(content: &str) -> u32 {
  let mut hash: u32 = 2_166_136_261;
  for ch in content.chars().take(100) {
    hash ^= ch as u32;
    hash = hash.wrapping_mul(16_777_619);
  }
  hash
}

async fn dedup_and_mark_served(
  state: &RuntimeState,
  agent: &str,
  results: Vec<RecallItem>,
) -> Vec<RecallItem> {
  if results.is_empty() {
    return results;
  }

  let mut served = state.served_content.lock().await;
  let set = served
    .entry(agent.to_string())
    .or_insert_with(HashSet::<u32>::new);

  let mut filtered = Vec::new();
  for result in results {
    let hash = hash_content(&result.excerpt);
    if set.contains(&hash) {
      continue;
    }
    set.insert(hash);
    filtered.push(result);
  }

  filtered
}

async fn clear_served_on_boot(state: &RuntimeState, agent: &str) {
  let mut served = state.served_content.lock().await;
  served.remove(agent);
}

fn run_recall(conn: &mut Connection, query_text: &str, k: usize) -> Result<Vec<RecallItem>, String> {
  let extracted = extract_keywords(query_text);
  let keyword_query = if extracted.is_empty() {
    query_text.to_string()
  } else {
    extracted.join(" ")
  };

  let mut merged: HashMap<String, RecallItem> = HashMap::new();
  for row in search_memories(conn, &keyword_query, 20)? {
    let key = row.source.clone();
    let should_replace = merged
      .get(&key)
      .map(|existing| row.relevance > existing.relevance)
      .unwrap_or(true);
    if should_replace {
      merged.insert(
        key,
        RecallItem {
          source: row.source,
          relevance: row.relevance,
          excerpt: row.excerpt,
          method: "keyword".to_string(),
          tokens: None,
        },
      );
    }
  }

  for row in search_decisions(conn, &keyword_query, 20)? {
    let key = row.source.clone();
    let should_replace = merged
      .get(&key)
      .map(|existing| row.relevance > existing.relevance)
      .unwrap_or(true);
    if should_replace {
      merged.insert(
        key,
        RecallItem {
          source: row.source,
          relevance: row.relevance,
          excerpt: row.excerpt,
          method: "keyword".to_string(),
          tokens: None,
        },
      );
    }
  }

  let mut ranked = merged.into_values().collect::<Vec<_>>();
  ranked.sort_by(|a, b| {
    b.relevance
      .partial_cmp(&a.relevance)
      .unwrap_or(std::cmp::Ordering::Equal)
  });
  ranked.truncate(k);

  for row in &ranked {
    bump_retrieval(conn, &row.source);
  }
  Ok(ranked)
}

fn search_memories(
  conn: &Connection,
  query_text: &str,
  limit: usize,
) -> Result<Vec<SearchCandidate>, String> {
  let mut stmt = conn
    .prepare(
      "SELECT id, text, source, tags, score, retrievals, last_accessed, created_at
       FROM memories
       WHERE status = 'active'",
    )
    .map_err(|e| e.to_string())?;

  let rows = stmt
    .query_map([], |row| {
      Ok((
        row.get::<_, i64>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, Option<String>>(2)?,
        row.get::<_, Option<String>>(3)?,
        row.get::<_, Option<f64>>(4)?,
        row.get::<_, Option<i64>>(5)?,
        row.get::<_, Option<String>>(6)?,
        row.get::<_, Option<String>>(7)?,
      ))
    })
    .map_err(|e| e.to_string())?;

  let tokens = extract_search_keywords(query_text);
  let mut ranked = Vec::new();
  for row in rows.flatten() {
    let (id, text, source, tags, score, retrievals, last_accessed, created_at) = row;
    let source_key = source.clone().unwrap_or_else(|| format!("memory::{id}"));
    let score = score.unwrap_or(1.0).max(0.0);
    let ts_source = last_accessed.clone().or(created_at.clone()).unwrap_or_default();
    let ts = parse_timestamp_ms(&ts_source);

    if tokens.is_empty() {
      ranked.push(SearchCandidate {
        source: source_key,
        excerpt: truncate_chars(&text, 200),
        relevance: round4(0.5 * score),
        matched_keywords: 0,
        score,
        ts,
      });
      continue;
    }

    let haystacks = vec![
      text.to_lowercase(),
      source.unwrap_or_default().to_lowercase(),
      tags.unwrap_or_default().to_lowercase(),
    ];

    let mut matched = 0_i64;
    for token in &tokens {
      if haystacks.iter().any(|h| h.contains(token)) {
        matched += 1;
      }
    }
    if matched == 0 {
      continue;
    }

    let recency_days = recency_days(last_accessed.as_deref().or(created_at.as_deref()));
    let recency_weight = 1.0 / (1.0 + recency_days as f64 / 7.0);
    let keyword_weight = matched as f64 / tokens.len() as f64;
    let retrieval_weight = (retrievals.unwrap_or(0).max(0).min(20) as f64) / 20.0;
    let score_weight = score.min(5.0) / 5.0;
    let ranking = (keyword_weight * 0.5)
      + (recency_weight * 0.2)
      + (retrieval_weight * 0.15)
      + (score_weight * 0.15);

    ranked.push(SearchCandidate {
      source: source_key,
      excerpt: truncate_chars(&text, 200),
      relevance: round4(ranking),
      matched_keywords: matched,
      score,
      ts,
    });
  }

  if tokens.is_empty() {
    ranked.sort_by(|a, b| b.ts.cmp(&a.ts));
  } else {
    ranked.sort_by(|a, b| {
      b.relevance
        .partial_cmp(&a.relevance)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then(b.matched_keywords.cmp(&a.matched_keywords))
        .then(
          b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal),
        )
        .then(b.ts.cmp(&a.ts))
    });
  }
  ranked.truncate(limit);
  Ok(ranked)
}

fn search_decisions(
  conn: &Connection,
  query_text: &str,
  limit: usize,
) -> Result<Vec<SearchCandidate>, String> {
  let mut stmt = conn
    .prepare(
      "SELECT id, decision, context, score, retrievals, last_accessed, created_at
       FROM decisions
       WHERE status = 'active'",
    )
    .map_err(|e| e.to_string())?;

  let rows = stmt
    .query_map([], |row| {
      Ok((
        row.get::<_, i64>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, Option<String>>(2)?,
        row.get::<_, Option<f64>>(3)?,
        row.get::<_, Option<i64>>(4)?,
        row.get::<_, Option<String>>(5)?,
        row.get::<_, Option<String>>(6)?,
      ))
    })
    .map_err(|e| e.to_string())?;

  let tokens = extract_search_keywords(query_text);
  let mut ranked = Vec::new();
  for row in rows.flatten() {
    let (id, decision, context, score, retrievals, last_accessed, created_at) = row;
    let source_key = context.clone().unwrap_or_else(|| format!("decision::{id}"));
    let score = score.unwrap_or(1.0).max(0.0);
    let ts_source = last_accessed.clone().or(created_at.clone()).unwrap_or_default();
    let ts = parse_timestamp_ms(&ts_source);

    if tokens.is_empty() {
      ranked.push(SearchCandidate {
        source: source_key,
        excerpt: truncate_chars(&decision, 200),
        relevance: round4(0.5 * score),
        matched_keywords: 0,
        score,
        ts,
      });
      continue;
    }

    let haystacks = vec![decision.to_lowercase(), context.unwrap_or_default().to_lowercase()];
    let mut matched = 0_i64;
    for token in &tokens {
      if haystacks.iter().any(|h| h.contains(token)) {
        matched += 1;
      }
    }
    if matched == 0 {
      continue;
    }

    let recency_days = recency_days(last_accessed.as_deref().or(created_at.as_deref()));
    let recency_weight = 1.0 / (1.0 + recency_days as f64 / 7.0);
    let keyword_weight = matched as f64 / tokens.len() as f64;
    let retrieval_weight = (retrievals.unwrap_or(0).max(0).min(20) as f64) / 20.0;
    let score_weight = score.min(5.0) / 5.0;
    let ranking = (keyword_weight * 0.5)
      + (recency_weight * 0.2)
      + (retrieval_weight * 0.15)
      + (score_weight * 0.15);
    ranked.push(SearchCandidate {
      source: source_key,
      excerpt: truncate_chars(&decision, 200),
      relevance: round4(ranking),
      matched_keywords: matched,
      score,
      ts,
    });
  }

  if tokens.is_empty() {
    ranked.sort_by(|a, b| b.ts.cmp(&a.ts));
  } else {
    ranked.sort_by(|a, b| {
      b.relevance
        .partial_cmp(&a.relevance)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then(b.matched_keywords.cmp(&a.matched_keywords))
        .then(
          b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal),
        )
        .then(b.ts.cmp(&a.ts))
    });
  }
  ranked.truncate(limit);
  Ok(ranked)
}

fn store_decision(
  conn: &mut Connection,
  decision: &str,
  context: Option<String>,
  entry_type: Option<String>,
  source_agent: String,
  confidence: Option<f64>,
) -> Result<Value, String> {
  conn
    .execute(
      "INSERT INTO decisions (decision, context, type, source_agent, confidence, surprise, status, created_at, updated_at)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?7)",
      params![
        decision,
        context,
        entry_type.unwrap_or_else(|| "decision".to_string()),
        source_agent.clone(),
        confidence.unwrap_or(0.8),
        1.0_f64,
        now_iso()
      ],
    )
    .map_err(|e| e.to_string())?;
  let id = conn.last_insert_rowid();
  let _ = log_event(
    conn,
    "decision_stored",
    json!({ "id": id, "source_agent": source_agent }),
    "rust-daemon",
  );
  checkpoint_wal_best_effort(conn);
  Ok(json!({ "stored": true, "id": id, "status": "active", "surprise": 1.0 }))
}

fn forget_keyword(conn: &mut Connection, keyword: &str) -> Result<usize, String> {
  let pattern = format!("%{}%", keyword.to_lowercase());
  let now = now_iso();
  let memories = conn
    .execute(
      "UPDATE memories SET score = score * 0.3, updated_at = ?2
       WHERE status = 'active' AND (lower(text) LIKE ?1 OR lower(source) LIKE ?1)",
      params![pattern.clone(), now.clone()],
    )
    .map_err(|e| e.to_string())?;
  let decisions = conn
    .execute(
      "UPDATE decisions SET score = score * 0.3, updated_at = ?2
       WHERE status = 'active' AND (lower(decision) LIKE ?1 OR lower(context) LIKE ?1)",
      params![pattern, now],
    )
    .map_err(|e| e.to_string())?;
  let affected = memories + decisions;
  if affected > 0 {
    let _ = log_event(
      conn,
      "forget",
      json!({ "keyword": keyword, "affected": affected }),
      "rust-daemon",
    );
    checkpoint_wal_best_effort(conn);
  }
  Ok(affected)
}

fn resolve_decision(
  conn: &mut Connection,
  keep_id: i64,
  action: &str,
  superseded_id: Option<i64>,
) -> Result<(), String> {
  match action {
    "keep" => {
      conn
        .execute(
          "UPDATE decisions SET status = 'active', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
          params![keep_id, now_iso()],
        )
        .map_err(|e| e.to_string())?;
      if let Some(other) = superseded_id {
        conn
          .execute(
            "UPDATE decisions SET status = 'superseded', supersedes_id = ?1, disputes_id = NULL, updated_at = ?3 WHERE id = ?2",
            params![keep_id, other, now_iso()],
          )
          .map_err(|e| e.to_string())?;
      }
    }
    "merge" => {
      conn
        .execute(
          "UPDATE decisions SET status = 'active', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
          params![keep_id, now_iso()],
        )
        .map_err(|e| e.to_string())?;
      if let Some(other) = superseded_id {
        conn
          .execute(
            "UPDATE decisions SET status = 'active', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
            params![other, now_iso()],
          )
          .map_err(|e| e.to_string())?;
      }
    }
    "archive" => {
      let ts = now_iso();
      conn
        .execute(
          "UPDATE decisions SET status = 'archived', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
          params![keep_id, ts],
        )
        .map_err(|e| e.to_string())?;
      if let Some(other) = superseded_id {
        conn
          .execute(
            "UPDATE decisions SET status = 'archived', disputes_id = NULL, updated_at = ?2 WHERE id = ?1",
            params![other, ts],
          )
          .map_err(|e| e.to_string())?;
      }
    }
    _ => return Err("Invalid action. Expected keep, merge, or archive.".to_string()),
  }
  let _ = log_event(
    conn,
    "decision_resolve",
    json!({ "keepId": keep_id, "action": action, "supersededId": superseded_id }),
    "rust-daemon",
  );
  checkpoint_wal_best_effort(conn);
  Ok(())
}

fn build_digest(conn: &Connection) -> Result<Value, String> {
  let today = Utc::now().format("%Y-%m-%d").to_string();
  let today_like = format!("{today}%");
  let total_memories: i64 = conn
    .query_row("SELECT COUNT(*) FROM memories WHERE status = 'active'", [], |r| r.get(0))
    .unwrap_or(0);
  let total_decisions: i64 = conn
    .query_row("SELECT COUNT(*) FROM decisions WHERE status = 'active'", [], |r| r.get(0))
    .unwrap_or(0);
  let total_conflicts: i64 = conn
    .query_row("SELECT COUNT(*) FROM decisions WHERE status = 'disputed'", [], |r| r.get(0))
    .unwrap_or(0);

  let new_memories: i64 = conn
    .query_row(
      "SELECT COUNT(*) FROM memories WHERE created_at LIKE ?1",
      params![today_like.clone()],
      |r| r.get(0),
    )
    .unwrap_or(0);
  let new_decisions: i64 = conn
    .query_row(
      "SELECT COUNT(*) FROM decisions WHERE created_at LIKE ?1",
      params![today_like.clone()],
      |r| r.get(0),
    )
    .unwrap_or(0);
  let stores_today: i64 = conn
    .query_row(
      "SELECT COUNT(*) FROM events WHERE type = 'decision_stored' AND created_at LIKE ?1",
      params![today_like.clone()],
      |r| r.get(0),
    )
    .unwrap_or(0);
  let conflicts_today: i64 = conn
    .query_row(
      "SELECT COUNT(*) FROM events WHERE type = 'decision_conflict' AND created_at LIKE ?1",
      params![today_like.clone()],
      |r| r.get(0),
    )
    .unwrap_or(0);

  let decayed_memories: i64 = conn
    .query_row(
      "SELECT COUNT(*) FROM memories WHERE status = 'active' AND score < 0.5 AND pinned = 0",
      [],
      |r| r.get(0),
    )
    .unwrap_or(0);
  let decayed_decisions: i64 = conn
    .query_row(
      "SELECT COUNT(*) FROM decisions WHERE status = 'active' AND score < 0.5 AND pinned = 0",
      [],
      |r| r.get(0),
    )
    .unwrap_or(0);

  let mut top_stmt = conn
    .prepare("SELECT source, text, retrievals FROM memories WHERE status = 'active' AND retrievals > 0 ORDER BY retrievals DESC LIMIT 5")
    .map_err(|e| e.to_string())?;
  let top_rows = top_stmt
    .query_map([], |row| {
      Ok(json!({
        "source": row.get::<_, Option<String>>(0)?.unwrap_or_else(|| "unknown".to_string()),
        "text": truncate_chars(&row.get::<_, String>(1)?, 80),
        "retrievals": row.get::<_, i64>(2)?
      }))
    })
    .map_err(|e| e.to_string())?;
  let mut top_recalled = Vec::new();
  for row in top_rows.flatten() {
    top_recalled.push(row);
  }

  let mut boots_stmt = conn
    .prepare(
      "SELECT source_agent, COUNT(*) as cnt FROM events WHERE type = 'agent_boot' AND created_at LIKE ?1 GROUP BY source_agent",
    )
    .map_err(|e| e.to_string())?;
  let boots_rows = boots_stmt
    .query_map(params![today_like.clone()], |row| {
      Ok(json!({
        "source_agent": row.get::<_, Option<String>>(0)?.unwrap_or_else(|| "unknown".to_string()),
        "cnt": row.get::<_, i64>(1)?
      }))
    })
    .map_err(|e| e.to_string())?;
  let mut agent_boots = Vec::new();
  for row in boots_rows.flatten() {
    agent_boots.push(row);
  }

  let mut total_saved = 0_i64;
  let mut total_served = 0_i64;
  let mut boot_count = 0_i64;
  let mut today_saved = 0_i64;
  let mut today_served = 0_i64;
  let mut today_boots = 0_i64;
  let mut savings_stmt = conn
    .prepare("SELECT data, created_at FROM events WHERE type = 'boot_savings'")
    .map_err(|e| e.to_string())?;
  let savings_rows = savings_stmt
    .query_map([], |row| {
      Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?))
    })
    .map_err(|e| e.to_string())?;
  for row in savings_rows.flatten() {
    if let (Some(data), created_at) = row {
      if let Ok(parsed) = serde_json::from_str::<Value>(&data) {
        total_saved += parsed.get("saved").and_then(|v| v.as_i64()).unwrap_or(0);
        total_served += parsed.get("served").and_then(|v| v.as_i64()).unwrap_or(0);
        boot_count += 1;
        if created_at
          .as_deref()
          .map(|v| v.starts_with(&today))
          .unwrap_or(false)
        {
          today_saved += parsed.get("saved").and_then(|v| v.as_i64()).unwrap_or(0);
          today_served += parsed.get("served").and_then(|v| v.as_i64()).unwrap_or(0);
          today_boots += 1;
        }
      }
    }
  }

  let agent_str = if agent_boots.is_empty() {
    "none".to_string()
  } else {
    agent_boots
      .iter()
      .map(|row| {
        format!(
          "{} ({})",
          row.get("source_agent")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown"),
          row.get("cnt").and_then(|v| v.as_i64()).unwrap_or(0)
        )
      })
      .collect::<Vec<_>>()
      .join(", ")
  };

  let savings_str = if total_saved > 0 {
    format!(" | Saved: {} tokens ({} boots)", total_saved, boot_count)
  } else {
    String::new()
  };
  let oneliner = format!(
    "Cortex Daily — {today} | Mem: {total_memories} (+{new_memories}) | Dec: {total_decisions} (+{new_decisions}) | Conflicts: {total_conflicts} | Decaying: {} | Agents: {}{}",
    decayed_memories + decayed_decisions,
    agent_str,
    savings_str
  );

  Ok(json!({
    "date": today,
    "totals": { "memories": total_memories, "decisions": total_decisions, "conflicts": total_conflicts },
    "today": { "newMemories": new_memories, "newDecisions": new_decisions, "stores": stores_today, "conflictsDetected": conflicts_today },
    "tokenSavings": {
      "allTime": { "saved": total_saved, "served": total_served, "boots": boot_count },
      "today": { "saved": today_saved, "served": today_served, "boots": today_boots }
    },
    "topRecalled": top_recalled,
    "decay": { "memories": decayed_memories, "decisions": decayed_decisions },
    "agentBoots": agent_boots,
    "oneliner": oneliner
  }))
}

fn fetch_locks(conn: &Connection) -> Result<Vec<Value>, String> {
  let mut stmt = conn
    .prepare("SELECT id, path, agent, locked_at, expires_at FROM locks ORDER BY locked_at ASC")
    .map_err(|e| e.to_string())?;
  let rows = stmt
    .query_map([], |row| {
      Ok(json!({
        "id": row.get::<_, String>(0)?,
        "path": row.get::<_, String>(1)?,
        "agent": row.get::<_, String>(2)?,
        "lockedAt": row.get::<_, String>(3)?,
        "expiresAt": row.get::<_, String>(4)?
      }))
    })
    .map_err(|e| e.to_string())?;
  let mut out = Vec::new();
  for row in rows.flatten() {
    out.push(row);
  }
  Ok(out)
}

fn fetch_messages_for_agent(conn: &Connection, agent: &str) -> Result<Vec<Value>, String> {
  let mut stmt = conn
    .prepare(
      "SELECT id, sender, recipient, message, timestamp FROM messages WHERE recipient = ?1 ORDER BY timestamp ASC",
    )
    .map_err(|e| e.to_string())?;
  let rows = stmt
    .query_map(params![agent], |row| {
      Ok(json!({
        "id": row.get::<_, String>(0)?,
        "from": row.get::<_, String>(1)?,
        "to": row.get::<_, String>(2)?,
        "message": row.get::<_, String>(3)?,
        "timestamp": row.get::<_, String>(4)?
      }))
    })
    .map_err(|e| e.to_string())?;
  let mut out = Vec::new();
  for row in rows.flatten() {
    out.push(row);
  }
  Ok(out)
}

fn fetch_sessions(conn: &Connection) -> Result<Vec<Value>, String> {
  let mut stmt = conn
    .prepare(
      "SELECT session_id, agent, project, files_json, description, started_at, last_heartbeat, expires_at
       FROM sessions ORDER BY started_at ASC",
    )
    .map_err(|e| e.to_string())?;
  let rows = stmt
    .query_map([], |row| {
      Ok(json!({
        "sessionId": row.get::<_, String>(0)?,
        "agent": row.get::<_, String>(1)?,
        "project": row.get::<_, Option<String>>(2)?,
        "files": parse_json_array(&row.get::<_, String>(3)?),
        "description": row.get::<_, Option<String>>(4)?,
        "startedAt": row.get::<_, String>(5)?,
        "lastHeartbeat": row.get::<_, String>(6)?,
        "expiresAt": row.get::<_, String>(7)?
      }))
    })
    .map_err(|e| e.to_string())?;
  let mut out = Vec::new();
  for row in rows.flatten() {
    out.push(row);
  }
  Ok(out)
}

fn fetch_tasks(conn: &Connection, status_filter: &str, project: Option<&str>) -> Result<Vec<Value>, String> {
  let mut sql = "SELECT task_id, title, description, project, files_json, priority, required_capability, status, claimed_by, created_at, claimed_at, completed_at, summary FROM tasks".to_string();
  let mut clauses = Vec::new();
  if status_filter != "all" {
    clauses.push(format!("status = '{}'", status_filter.replace('\'', "''")));
  }
  if let Some(project) = project {
    clauses.push(format!("project = '{}'", project.replace('\'', "''")));
  }
  if !clauses.is_empty() {
    sql.push_str(" WHERE ");
    sql.push_str(&clauses.join(" AND "));
  }
  sql.push_str(" ORDER BY created_at ASC");

  let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
  let rows = stmt
    .query_map([], |row| {
      Ok(json!({
        "taskId": row.get::<_, String>(0)?,
        "title": row.get::<_, String>(1)?,
        "description": row.get::<_, Option<String>>(2)?,
        "project": row.get::<_, Option<String>>(3)?,
        "files": parse_json_array(&row.get::<_, String>(4)?),
        "priority": row.get::<_, String>(5)?,
        "requiredCapability": row.get::<_, String>(6)?,
        "status": row.get::<_, String>(7)?,
        "claimedBy": row.get::<_, Option<String>>(8)?,
        "createdAt": row.get::<_, String>(9)?,
        "claimedAt": row.get::<_, Option<String>>(10)?,
        "completedAt": row.get::<_, Option<String>>(11)?,
        "summary": row.get::<_, Option<String>>(12)?
      }))
    })
    .map_err(|e| e.to_string())?;
  let mut out = Vec::new();
  for row in rows.flatten() {
    out.push(row);
  }
  Ok(out)
}

fn fetch_feed_since(conn: &Connection, cutoff: &str) -> Result<Vec<FeedEntry>, String> {
  let mut stmt = conn
    .prepare(
      "SELECT id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens
       FROM feed WHERE timestamp >= ?1 ORDER BY timestamp ASC",
    )
    .map_err(|e| e.to_string())?;
  let rows = stmt
    .query_map(params![cutoff], |row| {
      Ok(FeedEntry {
        id: row.get(0)?,
        agent: row.get(1)?,
        kind: row.get(2)?,
        summary: row.get(3)?,
        content: row.get(4)?,
        files: parse_json_array(&row.get::<_, String>(5)?),
        task_id: row.get(6)?,
        trace_id: row.get(7)?,
        priority: row.get(8)?,
        timestamp: row.get(9)?,
        tokens: row.get(10)?,
      })
    })
    .map_err(|e| e.to_string())?;
  let mut out = Vec::new();
  for row in rows.flatten() {
    out.push(row);
  }
  Ok(out)
}

fn get_unread_feed(conn: &Connection, for_agent: &str) -> Result<Vec<FeedEntry>, String> {
  let ack = conn
    .query_row(
      "SELECT last_seen_id FROM feed_acks WHERE agent = ?1",
      params![for_agent],
      |row| row.get::<_, String>(0),
    )
    .optional()
    .map_err(|e| e.to_string())?;

  let mut stmt = conn
    .prepare(
      "SELECT id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens
       FROM feed ORDER BY timestamp ASC",
    )
    .map_err(|e| e.to_string())?;
  let rows = stmt
    .query_map([], |row| {
      Ok(FeedEntry {
        id: row.get(0)?,
        agent: row.get(1)?,
        kind: row.get(2)?,
        summary: row.get(3)?,
        content: row.get(4)?,
        files: parse_json_array(&row.get::<_, String>(5)?),
        task_id: row.get(6)?,
        trace_id: row.get(7)?,
        priority: row.get(8)?,
        timestamp: row.get(9)?,
        tokens: row.get(10)?,
      })
    })
    .map_err(|e| e.to_string())?;
  let mut all = Vec::new();
  for row in rows.flatten() {
    all.push(row);
  }

  if ack.is_none() {
    return Ok(all
      .into_iter()
      .filter(|entry| entry.agent != for_agent)
      .collect::<Vec<_>>());
  }

  let ack_id = ack.unwrap();
  let mut past_ack = false;
  let mut unread = Vec::new();
  for entry in all {
    if entry.id == ack_id {
      past_ack = true;
      continue;
    }
    if past_ack && entry.agent != for_agent {
      unread.push(entry);
    }
  }
  Ok(unread)
}

fn insert_feed_entry(conn: &Connection, entry: &FeedEntry) -> Result<(), String> {
  conn
    .execute(
      "INSERT INTO feed (id, agent, kind, summary, content, files_json, task_id, trace_id, priority, timestamp, tokens)
       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
      params![
        entry.id,
        entry.agent,
        entry.kind,
        entry.summary,
        entry.content,
        entry.files.to_string(),
        entry.task_id,
        entry.trace_id,
        entry.priority,
        entry.timestamp,
        entry.tokens
      ],
    )
    .map_err(|e| e.to_string())?;
  Ok(())
}

fn feed_to_json(entry: &FeedEntry, include_content: bool) -> Value {
  if include_content {
    json!({
      "id": entry.id,
      "agent": entry.agent,
      "kind": entry.kind,
      "summary": entry.summary,
      "content": entry.content,
      "files": entry.files,
      "taskId": entry.task_id,
      "traceId": entry.trace_id,
      "priority": entry.priority,
      "timestamp": entry.timestamp,
      "tokens": entry.tokens
    })
  } else {
    json!({
      "id": entry.id,
      "agent": entry.agent,
      "kind": entry.kind,
      "summary": entry.summary,
      "files": entry.files,
      "taskId": entry.task_id,
      "traceId": entry.trace_id,
      "priority": entry.priority,
      "timestamp": entry.timestamp,
      "tokens": entry.tokens
    })
  }
}

fn clean_expired_locks(conn: &Connection) -> rusqlite::Result<()> {
  conn.execute("DELETE FROM locks WHERE expires_at < ?1", params![now_iso()])?;
  Ok(())
}

fn clean_old_activities(conn: &Connection) -> rusqlite::Result<()> {
  let count: i64 = conn.query_row("SELECT COUNT(*) FROM activities", [], |r| r.get(0))?;
  if count > MAX_ACTIVITIES {
    conn.execute(
      "DELETE FROM activities WHERE id IN (SELECT id FROM activities ORDER BY timestamp ASC LIMIT ?1)",
      params![count - MAX_ACTIVITIES],
    )?;
  }
  Ok(())
}

fn clean_old_messages(conn: &Connection, recipient: &str) -> rusqlite::Result<()> {
  let count: i64 = conn.query_row(
    "SELECT COUNT(*) FROM messages WHERE recipient = ?1",
    params![recipient],
    |r| r.get(0),
  )?;
  if count > MAX_MESSAGES_PER_AGENT {
    conn.execute(
      "DELETE FROM messages WHERE id IN (SELECT id FROM messages WHERE recipient = ?1 ORDER BY timestamp ASC LIMIT ?2)",
      params![recipient, count - MAX_MESSAGES_PER_AGENT],
    )?;
  }
  Ok(())
}

fn clean_expired_sessions(conn: &Connection) -> rusqlite::Result<()> {
  conn.execute("DELETE FROM sessions WHERE expires_at < ?1", params![now_iso()])?;
  Ok(())
}

fn clean_old_tasks(conn: &Connection) -> rusqlite::Result<()> {
  let count: i64 = conn.query_row("SELECT COUNT(*) FROM tasks", [], |r| r.get(0))?;
  if count > MAX_TASKS {
    conn.execute(
      "DELETE FROM tasks WHERE task_id IN (SELECT task_id FROM tasks WHERE status = 'completed' ORDER BY completed_at ASC LIMIT ?1)",
      params![count - MAX_TASKS],
    )?;
  }
  Ok(())
}

fn clean_old_feed(conn: &Connection) -> rusqlite::Result<()> {
  let cutoff = (Utc::now() - Duration::seconds(FEED_TTL_SECONDS)).to_rfc3339();
  conn.execute("DELETE FROM feed WHERE timestamp < ?1", params![cutoff])?;
  let count: i64 = conn.query_row("SELECT COUNT(*) FROM feed", [], |r| r.get(0))?;
  if count > MAX_FEED {
    conn.execute(
      "DELETE FROM feed WHERE id IN (SELECT id FROM feed ORDER BY timestamp ASC LIMIT ?1)",
      params![count - MAX_FEED],
    )?;
  }
  Ok(())
}

fn log_event(conn: &Connection, kind: &str, data: Value, source_agent: &str) -> rusqlite::Result<()> {
  conn.execute(
    "INSERT INTO events (type, data, source_agent) VALUES (?1, ?2, ?3)",
    params![kind, data.to_string(), source_agent],
  )?;
  Ok(())
}

fn parse_json_array(raw: &str) -> Value {
  serde_json::from_str(raw).unwrap_or_else(|_| json!([]))
}

fn normalize_text(input: &str) -> String {
  input
    .chars()
    .map(|ch| {
      if ch.is_ascii_alphanumeric() || ch == '-' || ch.is_ascii_whitespace() {
        ch.to_ascii_lowercase()
      } else {
        ' '
      }
    })
    .collect()
}

fn extract_keywords(text: &str) -> Vec<String> {
  let stop_words: HashSet<&'static str> = [
    "the", "a", "an", "is", "are", "was", "were", "be", "been", "being", "have", "has", "had",
    "do", "does", "did", "will", "would", "could", "should", "may", "might", "shall", "can",
    "to", "of", "in", "for", "on", "with", "at", "by", "from", "as", "into", "about", "that",
    "this", "it", "its", "not", "but", "and", "or", "if", "then", "so", "what", "which", "who",
    "how", "when", "where", "why", "all", "each", "every", "both", "few", "more", "most", "some",
    "any", "no", "my", "your", "his", "her", "our", "their", "i", "me",
  ]
  .into_iter()
  .collect();

  normalize_text(text)
    .split_whitespace()
    .filter(|word| word.len() > 2 && !stop_words.contains(*word))
    .map(str::to_string)
    .collect()
}

fn extract_search_keywords(text: &str) -> Vec<String> {
  normalize_text(text)
    .split_whitespace()
    .filter(|word| word.len() > 1)
    .map(str::to_string)
    .collect()
}

fn parse_timestamp_ms(value: &str) -> i64 {
  if value.trim().is_empty() {
    return 0;
  }
  if let Ok(dt) = DateTime::parse_from_rfc3339(value) {
    return dt.timestamp_millis();
  }
  if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
    return Utc.from_utc_datetime(&naive).timestamp_millis();
  }
  if let Ok(naive) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.f") {
    return Utc.from_utc_datetime(&naive).timestamp_millis();
  }
  0
}

fn recency_days(value: Option<&str>) -> i64 {
  let ts = value.map(parse_timestamp_ms).unwrap_or(0);
  if ts == 0 {
    return 3650;
  }
  ((Utc::now().timestamp_millis() - ts).max(0) / (24 * 60 * 60 * 1000)) as i64
}

fn round4(value: f64) -> f64 {
  (value * 10000.0).round() / 10000.0
}

fn truncate_chars(input: &str, max: usize) -> String {
  input.chars().take(max).collect::<String>()
}

fn bump_retrieval(conn: &Connection, source: &str) {
  let now = now_iso();
  let _ = conn.execute(
    "UPDATE memories SET retrievals = retrievals + 1, last_accessed = ?1 WHERE source = ?2",
    params![now.clone(), source],
  );
  if let Some(id_text) = source.strip_prefix("decision::") {
    if let Ok(id) = id_text.parse::<i64>() {
      let _ = conn.execute(
        "UPDATE decisions SET retrievals = retrievals + 1, last_accessed = ?1 WHERE id = ?2",
        params![now, id],
      );
    }
  } else {
    let _ = conn.execute(
      "UPDATE decisions SET retrievals = retrievals + 1, last_accessed = ?1 WHERE context = ?2",
      params![now, source],
    );
  }
}

fn recall_to_json(item: RecallItem) -> Value {
  let mut payload = json!({
    "source": item.source,
    "relevance": item.relevance,
    "excerpt": item.excerpt,
    "method": item.method
  });
  if let Some(tokens) = item.tokens {
    if let Value::Object(ref mut map) = payload {
      map.insert("tokens".to_string(), Value::Number((tokens as u64).into()));
    }
  }
  payload
}
