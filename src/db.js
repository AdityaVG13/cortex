const initSqlJs = require('sql.js');
const path = require('path');
const fs = require('fs');

const DB_PATH = path.join(__dirname, '..', 'cortex.db');
const CORTEX_DIR = path.join(process.env.USERPROFILE || process.env.HOME, '.cortex');

let _db = null;
let _dirty = false;
let _saveTimer = null;

function ensureCortexDir() {
  if (!fs.existsSync(CORTEX_DIR)) {
    fs.mkdirSync(CORTEX_DIR, { recursive: true });
  }
}

async function getDb() {
  if (_db) return _db;
  ensureCortexDir();

  const SQL = await initSqlJs();

  // Load existing DB or create new
  if (fs.existsSync(DB_PATH)) {
    const buffer = fs.readFileSync(DB_PATH);
    _db = new SQL.Database(buffer);
  } else {
    _db = new SQL.Database();
  }

  _db.run('PRAGMA journal_mode = WAL');
  _db.run('PRAGMA foreign_keys = ON');
  initSchema(_db);
  persist(); // Save initial schema
  return _db;
}

function initSchema(db) {
  db.run(`
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
      disputes_id INTEGER,
      supersedes_id INTEGER,
      confirmed_by TEXT,
      created_at TEXT DEFAULT (datetime('now')),
      updated_at TEXT DEFAULT (datetime('now'))
    )
  `);

  db.run(`
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
      parent_id INTEGER,
      disputes_id INTEGER,
      supersedes_id INTEGER,
      confirmed_by TEXT,
      created_at TEXT DEFAULT (datetime('now')),
      updated_at TEXT DEFAULT (datetime('now'))
    )
  `);

  db.run(`
    CREATE TABLE IF NOT EXISTS embeddings (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      target_type TEXT NOT NULL,
      target_id INTEGER NOT NULL,
      vector BLOB NOT NULL,
      model TEXT DEFAULT 'nomic-embed-text',
      created_at TEXT DEFAULT (datetime('now'))
    )
  `);

  db.run(`
    CREATE TABLE IF NOT EXISTS events (
      id INTEGER PRIMARY KEY AUTOINCREMENT,
      type TEXT NOT NULL,
      data TEXT,
      source_agent TEXT,
      created_at TEXT DEFAULT (datetime('now'))
    )
  `);

  // Create indexes for common queries
  db.run('CREATE INDEX IF NOT EXISTS idx_memories_status ON memories(status)');
  db.run('CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(type)');
  db.run('CREATE INDEX IF NOT EXISTS idx_memories_source_agent ON memories(source_agent)');
  db.run('CREATE INDEX IF NOT EXISTS idx_decisions_status ON decisions(status)');
  db.run('CREATE INDEX IF NOT EXISTS idx_decisions_source_agent ON decisions(source_agent)');
  db.run('CREATE INDEX IF NOT EXISTS idx_embeddings_target ON embeddings(target_type, target_id)');
  db.run('CREATE INDEX IF NOT EXISTS idx_events_type ON events(type)');
}

// Persist database to disk (debounced)
function persist() {
  if (!_db) return;
  const data = _db.export();
  const buffer = Buffer.from(data);
  fs.writeFileSync(DB_PATH, buffer);
  _dirty = false;
}

function markDirty() {
  _dirty = true;
  if (_saveTimer) clearTimeout(_saveTimer);
  _saveTimer = setTimeout(() => {
    if (_dirty) persist();
  }, 1000); // Auto-persist after 1s of inactivity
}

// Execute a write query and auto-persist
function run(sql, params = []) {
  const db = _db;
  if (!db) throw new Error('Database not initialized. Call getDb() first.');
  db.run(sql, params);
  markDirty();
}

// Execute a read query
function query(sql, params = []) {
  const db = _db;
  if (!db) throw new Error('Database not initialized. Call getDb() first.');
  const stmt = db.prepare(sql);
  if (params.length) stmt.bind(params);
  const results = [];
  while (stmt.step()) {
    results.push(stmt.getAsObject());
  }
  stmt.free();
  return results;
}

// Get single row
function get(sql, params = []) {
  const results = query(sql, params);
  return results.length > 0 ? results[0] : null;
}

// Insert and return last insert ID
function insert(sql, params = []) {
  run(sql, params);
  const row = get('SELECT last_insert_rowid() as id');
  return row ? row.id : null;
}

function close() {
  if (_saveTimer) {
    clearTimeout(_saveTimer);
    _saveTimer = null;
  }
  if (_db) {
    if (_dirty) persist();
    _db.close();
    _db = null;
  }
}

function getStats() {
  return {
    memories: get('SELECT COUNT(*) as count FROM memories')?.count || 0,
    decisions: get('SELECT COUNT(*) as count FROM decisions')?.count || 0,
    embeddings: get('SELECT COUNT(*) as count FROM embeddings')?.count || 0,
    events: get('SELECT COUNT(*) as count FROM events')?.count || 0,
  };
}

// Keyword search using LIKE (sql.js doesn't have FTS5 in WASM build)
function searchMemories(queryText) {
  const pattern = `%${queryText}%`;
  return query(
    'SELECT * FROM memories WHERE status = ? AND (text LIKE ? OR source LIKE ?) ORDER BY score DESC, created_at DESC LIMIT 20',
    ['active', pattern, pattern]
  );
}

function searchDecisions(queryText) {
  const pattern = `%${queryText}%`;
  return query(
    'SELECT * FROM decisions WHERE status = ? AND (decision LIKE ? OR context LIKE ?) ORDER BY score DESC, created_at DESC LIMIT 20',
    ['active', pattern, pattern]
  );
}

module.exports = {
  getDb, close, getStats, ensureCortexDir,
  run, query, get, insert, persist, markDirty,
  searchMemories, searchDecisions,
  DB_PATH, CORTEX_DIR
};
