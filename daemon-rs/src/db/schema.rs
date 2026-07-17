use rusqlite::Connection;
pub fn initialize_schema(conn: &Connection) -> rusqlite::Result<()> {
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
          source_client TEXT DEFAULT 'unknown',
          source_model TEXT,
          confidence REAL DEFAULT 0.8,
          reasoning_depth TEXT DEFAULT 'single-shot',
          trust_score REAL DEFAULT 0.8,
          status TEXT DEFAULT 'active',
          score REAL DEFAULT 1.0,
          retrievals INTEGER DEFAULT 0,
          last_accessed TEXT,
          pinned INTEGER DEFAULT 0,
          disputes_id INTEGER,
          supersedes_id INTEGER,
          confirmed_by TEXT,
          merged_count INTEGER DEFAULT 0,
          quality INTEGER DEFAULT 50,
          retention_class TEXT NOT NULL DEFAULT 'operational'
            CHECK (retention_class IN ('durable', 'operational', 'audit', 'ephemeral')),
          expires_at TEXT,
          observed_at TEXT,
          valid_from TEXT,
          valid_until TEXT,
          created_at TEXT DEFAULT (datetime('now')),
          updated_at TEXT DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS decisions (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          decision TEXT NOT NULL,
          context TEXT,
          type TEXT DEFAULT 'decision',
          source_agent TEXT DEFAULT 'unknown',
          source_client TEXT DEFAULT 'unknown',
          source_model TEXT,
          confidence REAL DEFAULT 0.8,
          reasoning_depth TEXT DEFAULT 'single-shot',
          trust_score REAL DEFAULT 0.8,
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
          merged_count INTEGER DEFAULT 0,
          quality INTEGER DEFAULT 50,
          retention_class TEXT NOT NULL DEFAULT 'operational'
            CHECK (retention_class IN ('durable', 'operational', 'audit', 'ephemeral')),
          expires_at TEXT,
          observed_at TEXT,
          valid_from TEXT,
          valid_until TEXT,
          created_at TEXT DEFAULT (datetime('now')),
          updated_at TEXT DEFAULT (datetime('now'))
        );
        CREATE TABLE IF NOT EXISTS decision_conflicts (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          source_decision_id INTEGER REFERENCES decisions(id) ON DELETE SET NULL,
          target_decision_id INTEGER NOT NULL REFERENCES decisions(id) ON DELETE CASCADE,
          classification TEXT NOT NULL
            CHECK (classification IN ('AGREES', 'CONTRADICTS', 'REFINES', 'UNRELATED')),
          similarity_jaccard REAL,
          similarity_cosine REAL,
          status TEXT NOT NULL DEFAULT 'open'
            CHECK (status IN ('open', 'auto_resolved', 'user_resolved')),
          resolution_strategy TEXT,
          resolved_by TEXT,
          resolved_at TEXT,
          created_at TEXT NOT NULL DEFAULT (datetime('now'))
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
        CREATE TABLE IF NOT EXISTS client_permissions (
          owner_id INTEGER NOT NULL DEFAULT 0,
          client_id TEXT NOT NULL,
          permission TEXT NOT NULL,
          scope TEXT NOT NULL DEFAULT '*',
          granted_by TEXT NOT NULL DEFAULT 'system',
          granted_at TEXT NOT NULL DEFAULT (datetime('now')),
          PRIMARY KEY (owner_id, client_id, permission, scope)
        );
        CREATE INDEX IF NOT EXISTS idx_client_permissions_client
          ON client_permissions(owner_id, client_id);
        CREATE INDEX IF NOT EXISTS idx_cooccur_a ON co_occurrence(source_a);
        CREATE INDEX IF NOT EXISTS idx_cooccur_b ON co_occurrence(source_b);
        -- Performance indexes (added 2026-03-31)
        CREATE INDEX IF NOT EXISTS idx_memories_status ON memories(status);
        CREATE INDEX IF NOT EXISTS idx_memories_source_status ON memories(source, status);
        CREATE INDEX IF NOT EXISTS idx_memories_active_source_recent
          ON memories(source, COALESCE(last_accessed, created_at) DESC)
          WHERE status = 'active';
        CREATE INDEX IF NOT EXISTS idx_decisions_status ON decisions(status);
        CREATE INDEX IF NOT EXISTS idx_decisions_context_status ON decisions(context, status);
        CREATE INDEX IF NOT EXISTS idx_decisions_active_context_recent
          ON decisions(context, COALESCE(last_accessed, created_at) DESC)
          WHERE status = 'active';
        CREATE INDEX IF NOT EXISTS idx_decision_conflicts_source ON decision_conflicts(source_decision_id);
        CREATE INDEX IF NOT EXISTS idx_decision_conflicts_target ON decision_conflicts(target_decision_id);
        CREATE INDEX IF NOT EXISTS idx_decision_conflicts_status_created ON decision_conflicts(status, created_at);
        CREATE INDEX IF NOT EXISTS idx_embeddings_target ON embeddings(target_type, target_id);
        CREATE INDEX IF NOT EXISTS idx_embeddings_model_norm
          ON embeddings(LOWER(COALESCE(model, '')));
        CREATE INDEX IF NOT EXISTS idx_embeddings_target_model_norm
          ON embeddings(target_type, target_id, LOWER(COALESCE(model, '')));
        CREATE INDEX IF NOT EXISTS idx_embeddings_model_type_target_norm
          ON embeddings(LOWER(COALESCE(model, '')), target_type, target_id);
        CREATE INDEX IF NOT EXISTS idx_events_type_created ON events(type, created_at);
        CREATE INDEX IF NOT EXISTS idx_events_created ON events(created_at);
        CREATE INDEX IF NOT EXISTS idx_events_type_id ON events(type, id);
        CREATE INDEX IF NOT EXISTS idx_events_source_agent_created ON events(source_agent, created_at);
        CREATE INDEX IF NOT EXISTS idx_decisions_type_source_created
          ON decisions(type, source_agent, created_at);
        CREATE INDEX IF NOT EXISTS idx_messages_recipient ON messages(recipient);
        CREATE INDEX IF NOT EXISTS idx_messages_recipient_timestamp ON messages(recipient, timestamp);
        CREATE INDEX IF NOT EXISTS idx_sessions_heartbeat ON sessions(last_heartbeat);
        CREATE INDEX IF NOT EXISTS idx_activities_timestamp ON activities(timestamp);
        CREATE INDEX IF NOT EXISTS idx_feed_timestamp ON feed(timestamp);
        CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
        CREATE INDEX IF NOT EXISTS idx_tasks_status_created ON tasks(status, created_at);
        CREATE INDEX IF NOT EXISTS idx_locks_expires ON locks(expires_at);
        CREATE TABLE IF NOT EXISTS context_cache (
          cache_key TEXT PRIMARY KEY,
          content_hash TEXT NOT NULL,
          compressed TEXT NOT NULL,
          tokens INTEGER NOT NULL DEFAULT 0,
          created_at TEXT DEFAULT (datetime('now')),
          hits INTEGER DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS schema_migrations (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          version TEXT NOT NULL UNIQUE,
          name TEXT NOT NULL,
          applied_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        -- FTS5 full-text search indexes (porter+unicode61 for stemming + unicode tokenization)
        CREATE VIRTUAL TABLE IF NOT EXISTS memories_fts USING fts5(
          text, source, tags,
          content=memories,
          content_rowid=id,
          tokenize='porter unicode61'
        );
        CREATE VIRTUAL TABLE IF NOT EXISTS decisions_fts USING fts5(
          decision, context,
          content=decisions,
          content_rowid=id,
          tokenize='porter unicode61'
        );
        -- Relevance feedback: tracks which recalled results were actually useful
        CREATE TABLE IF NOT EXISTS recall_feedback (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          query_text TEXT NOT NULL,
          query_embedding BLOB,
          result_source TEXT NOT NULL,
          result_type TEXT NOT NULL DEFAULT 'unknown',
          result_id INTEGER,
          signal REAL NOT NULL DEFAULT 1.0,
          agent TEXT NOT NULL DEFAULT 'unknown',
          created_at TEXT DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_feedback_result ON recall_feedback(result_source);
        CREATE INDEX IF NOT EXISTS idx_feedback_created ON recall_feedback(created_at);
        CREATE TABLE IF NOT EXISTS event_savings_rollups (
          day TEXT NOT NULL,
          hour INTEGER NOT NULL,
          operation TEXT NOT NULL
            CHECK (operation IN ('recall', 'store', 'tool')),
          saved INTEGER NOT NULL DEFAULT 0,
          served INTEGER NOT NULL DEFAULT 0,
          baseline INTEGER NOT NULL DEFAULT 0,
          events INTEGER NOT NULL DEFAULT 0,
          hits INTEGER NOT NULL DEFAULT 0,
          misses INTEGER NOT NULL DEFAULT 0,
          updated_at TEXT NOT NULL DEFAULT (datetime('now')),
          PRIMARY KEY (day, hour, operation)
        );
        CREATE INDEX IF NOT EXISTS idx_event_savings_rollups_day
          ON event_savings_rollups(day);
        CREATE INDEX IF NOT EXISTS idx_event_savings_rollups_operation_day
          ON event_savings_rollups(operation, day);
        CREATE TABLE IF NOT EXISTS agent_feedback (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          owner_id INTEGER NOT NULL DEFAULT 0,
          agent TEXT NOT NULL,
          task_class TEXT NOT NULL DEFAULT 'general',
          outcome TEXT NOT NULL
            CHECK (outcome IN ('success', 'partial', 'failure')),
          outcome_score REAL NOT NULL,
          quality_score REAL NOT NULL DEFAULT 0.7,
          latency_ms INTEGER,
          retries INTEGER,
          tokens_used INTEGER,
          memory_sources_json TEXT NOT NULL DEFAULT '[]',
          notes TEXT,
          created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_agent_feedback_agent_created
          ON agent_feedback(owner_id, agent, created_at);
        CREATE INDEX IF NOT EXISTS idx_agent_feedback_task_created
          ON agent_feedback(owner_id, task_class, created_at);
        -- Triggers to keep FTS in sync with base tables
        CREATE TRIGGER IF NOT EXISTS memories_fts_ai AFTER INSERT ON memories BEGIN
          INSERT INTO memories_fts(rowid, text, source, tags) VALUES (new.id, new.text, new.source, new.tags);
        END;
        CREATE TRIGGER IF NOT EXISTS memories_fts_ad AFTER DELETE ON memories BEGIN
          INSERT INTO memories_fts(memories_fts, rowid, text, source, tags) VALUES('delete', old.id, old.text, old.source, old.tags);
        END;
        CREATE TRIGGER IF NOT EXISTS memories_fts_au AFTER UPDATE ON memories BEGIN
          INSERT INTO memories_fts(memories_fts, rowid, text, source, tags) VALUES('delete', old.id, old.text, old.source, old.tags);
          INSERT INTO memories_fts(rowid, text, source, tags) VALUES (new.id, new.text, new.source, new.tags);
        END;
        CREATE TRIGGER IF NOT EXISTS decisions_fts_ai AFTER INSERT ON decisions BEGIN
          INSERT INTO decisions_fts(rowid, decision, context) VALUES (new.id, new.decision, new.context);
        END;
        CREATE TRIGGER IF NOT EXISTS decisions_fts_ad AFTER DELETE ON decisions BEGIN
          INSERT INTO decisions_fts(decisions_fts, rowid, decision, context) VALUES('delete', old.id, old.decision, old.context);
        END;
        CREATE TRIGGER IF NOT EXISTS decisions_fts_au AFTER UPDATE ON decisions BEGIN
          INSERT INTO decisions_fts(decisions_fts, rowid, decision, context) VALUES('delete', old.id, old.decision, old.context);
          INSERT INTO decisions_fts(rowid, decision, context) VALUES (new.id, new.decision, new.context);
        END;
        "#,
    )?;
    Ok(())
}
