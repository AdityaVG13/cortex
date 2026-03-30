const initSqlJs = require('sql.js');
const path = require('path');
const fs = require('fs');

const DB_PATH = path.join(__dirname, '..', 'cortex.db');
const CORTEX_DIR = path.join(process.env.USERPROFILE || process.env.HOME, '.cortex');

let _db = null;
let _dirty = false;
let _saveTimer = null;
const DAY_MS = 24 * 60 * 60 * 1000;

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
      last_accessed TEXT,
      pinned INTEGER DEFAULT 0,
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
      last_accessed TEXT,
      pinned INTEGER DEFAULT 0,
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

  db.exec(`
    CREATE TABLE IF NOT EXISTS co_occurrence (
      source_a TEXT NOT NULL,
      source_b TEXT NOT NULL,
      count INTEGER DEFAULT 1,
      last_seen TEXT DEFAULT (datetime('now')),
      PRIMARY KEY (source_a, source_b)
    )
  `);

  // Create indexes for common queries
  db.run('CREATE INDEX IF NOT EXISTS idx_cooccur_a ON co_occurrence(source_a)');
  db.run('CREATE INDEX IF NOT EXISTS idx_cooccur_b ON co_occurrence(source_b)');
  db.run('CREATE INDEX IF NOT EXISTS idx_memories_status ON memories(status)');
  db.run('CREATE INDEX IF NOT EXISTS idx_memories_type ON memories(type)');
  db.run('CREATE INDEX IF NOT EXISTS idx_memories_source_agent ON memories(source_agent)');
  db.run('CREATE INDEX IF NOT EXISTS idx_decisions_status ON decisions(status)');
  db.run('CREATE INDEX IF NOT EXISTS idx_decisions_source_agent ON decisions(source_agent)');
  db.run('CREATE INDEX IF NOT EXISTS idx_embeddings_target ON embeddings(target_type, target_id)');
  db.run('CREATE INDEX IF NOT EXISTS idx_events_type ON events(type)');

  ensureColumn(db, 'memories', 'last_accessed', 'TEXT');
  ensureColumn(db, 'memories', 'pinned', 'INTEGER DEFAULT 0');
  ensureColumn(db, 'decisions', 'last_accessed', 'TEXT');
  ensureColumn(db, 'decisions', 'pinned', 'INTEGER DEFAULT 0');
}

function ensureColumn(db, tableName, columnName, columnSql) {
  const info = db.exec(`PRAGMA table_info(${tableName})`);
  const columns = info[0]?.values?.map((row) => row[1]) || [];
  if (!columns.includes(columnName)) {
    db.run(`ALTER TABLE ${tableName} ADD COLUMN ${columnName} ${columnSql}`);
  }
}

// Persist database to disk
function persist() {
  if (!_db) return;
  const data = _db.export();
  const buffer = Buffer.from(data);
  try {
    fs.writeFileSync(DB_PATH, buffer);
    _dirty = false;
  } catch (err) {
    if (err?.code === 'ENOENT') {
      // Tests can remove temp DB dirs during teardown; ignore stale flushes.
      _dirty = false;
      return;
    }
    throw err;
  }
}

// Debounced persist for high-frequency reads that touch updated_at etc.
function markDirty() {
  _dirty = true;
  if (_saveTimer) clearTimeout(_saveTimer);
  _saveTimer = setTimeout(() => {
    if (_dirty) persist();
  }, 500);
  // DO NOT unref() — timer must keep process alive until flush completes.
  // Previous unref() caused data loss on process exit during debounce window.
}

// Immediate persist for critical writes (store, forget, resolve).
// Never debounce data the user expects to survive a crash.
function persistNow() {
  _dirty = true;
  if (_saveTimer) clearTimeout(_saveTimer);
  _saveTimer = null;
  persist();
}

// Emergency flush — called from process exit hooks
function flushSync() {
  if (_db && _dirty) {
    try { persist(); } catch { /* best effort on exit */ }
  }
}

// Install process exit hooks to prevent data loss
process.on('SIGINT', () => { flushSync(); process.exit(0); });
process.on('SIGTERM', () => { flushSync(); process.exit(0); });
process.on('beforeExit', () => { flushSync(); });
process.on('exit', () => {
  // exit handler is synchronous-only — persist() uses writeFileSync so it works here
  if (_db && _dirty) {
    try { persist(); } catch { /* last resort */ }
  }
});

// Execute a write query and auto-persist (debounced)
function run(sql, params = []) {
  const db = _db;
  if (!db) throw new Error('Database not initialized. Call getDb() first.');
  db.run(sql, params);
  markDirty();
}

// Execute a write query and persist IMMEDIATELY (for store/forget/resolve)
function runCritical(sql, params = []) {
  const db = _db;
  if (!db) throw new Error('Database not initialized. Call getDb() first.');
  db.run(sql, params);
  persistNow();
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

// Insert with immediate persist — for decisions, memories, critical data
function insertCritical(sql, params = []) {
  runCritical(sql, params);
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
  return searchRows({
    tableName: 'memories',
    textFields: ['text', 'source', 'tags'],
    queryText,
  });
}

function searchDecisions(queryText) {
  return searchRows({
    tableName: 'decisions',
    textFields: ['decision', 'context'],
    queryText,
  });
}

function searchRows({ tableName, textFields, queryText, limit = 20 }) {
  const keywords = extractSearchKeywords(queryText);
  const rows = query(`SELECT * FROM ${tableName} WHERE status = ?`, ['active']);

  if (keywords.length === 0) {
    return rows
      .sort((a, b) => getTimestampMs(b.created_at) - getTimestampMs(a.created_at))
      .slice(0, limit);
  }

  const ranked = [];

  for (const row of rows) {
    const haystacks = textFields
      .map((field) => String(row[field] || '').toLowerCase())
      .filter(Boolean);

    let matchedKeywords = 0;
    for (const keyword of keywords) {
      if (haystacks.some((text) => text.includes(keyword))) {
        matchedKeywords++;
      }
    }

    if (matchedKeywords === 0) continue;

    const scoreWeight = Math.max(0, Number(row.score) || 0);
    const recencyDays = getRecencyDays(row.last_accessed || row.created_at);
    const recencyWeight = 1 / (1 + recencyDays / 7);
    const keywordWeight = matchedKeywords / keywords.length;
    const retrievals = Math.max(0, Number(row.retrievals) || 0);
    const retrievalWeight = Math.min(retrievals, 20) / 20; // cap at 20 retrievals
    const ranking = (keywordWeight * 0.5) + (recencyWeight * 0.2) + (retrievalWeight * 0.15) + (Math.min(scoreWeight, 5) / 5 * 0.15);

    ranked.push({
      ...row,
      _matched_keywords: matchedKeywords,
      _recency_days: recencyDays,
      _keyword_score: parseFloat(ranking.toFixed(4)),
    });
  }

  return ranked
    .sort((a, b) =>
      b._keyword_score - a._keyword_score ||
      b._matched_keywords - a._matched_keywords ||
      (Number(b.score) || 0) - (Number(a.score) || 0) ||
      getTimestampMs(b.last_accessed || b.created_at) - getTimestampMs(a.last_accessed || a.created_at)
    )
    .slice(0, limit);
}

function extractSearchKeywords(queryText) {
  return String(queryText || '')
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, ' ')
    .split(/\s+/)
    .filter((token) => token.length > 1);
}

function getTimestampMs(value) {
  if (!value) return 0;
  const normalized = String(value).replace(' ', 'T') + (String(value).includes('T') ? '' : 'Z');
  const parsed = Date.parse(normalized);
  return Number.isFinite(parsed) ? parsed : 0;
}

function getRecencyDays(value) {
  const ts = getTimestampMs(value);
  if (ts === 0) return 3650;
  return Math.max(0, Math.floor((Date.now() - ts) / DAY_MS));
}

function decayPass(referenceTime = Date.now()) {
  let affected = 0;

  affected += decayTable('memories', referenceTime);
  affected += decayTable('decisions', referenceTime);

  if (affected > 0) {
    persist();
  }

  return { affected };
}

function decayTable(tableName, referenceTime) {
  const rows = query(
    `SELECT id, score, pinned, created_at, updated_at, last_accessed FROM ${tableName} WHERE status = ?`,
    ['active']
  );

  let affected = 0;

  for (const row of rows) {
    if (Number(row.pinned) === 1) continue;

    const baselineMs = Math.max(
      getTimestampMs(row.last_accessed || row.created_at),
      getTimestampMs(row.updated_at || row.created_at)
    );
    const daysSince = Math.max(0, Math.floor((referenceTime - baselineMs) / DAY_MS));
    if (daysSince <= 0) continue;

    const currentScore = Math.max(0, Number(row.score) || 0);
    const nextScore = Math.max(0.1, currentScore * Math.pow(0.95, daysSince));
    if (Math.abs(nextScore - currentScore) < 0.0001) continue;

    run(
      `UPDATE ${tableName} SET score = ?, updated_at = datetime('now') WHERE id = ?`,
      [parseFloat(nextScore.toFixed(4)), row.id]
    );
    affected++;
  }

  return affected;
}

// ─── Dump & Archive (for dreaming worker) ─────────────────────────────────

function dumpActive() {
  const memories = query("SELECT * FROM memories WHERE status = 'active' ORDER BY score DESC");
  const decisions = query("SELECT * FROM decisions WHERE status = 'active' ORDER BY score DESC");
  return { memories, decisions };
}

function archiveEntries(type, ids) {
  if (!ids || !ids.length) return { affected: 0 };
  const table = type === 'memories' ? 'memories' : 'decisions';
  let affected = 0;
  for (const id of ids) {
    run(`UPDATE ${table} SET status = 'archived', updated_at = datetime('now') WHERE id = ? AND status = 'active'`, [id]);
    affected++;
  }
  if (affected > 0) persist();
  return { affected };
}

module.exports = {
  getDb, close, getStats, ensureCortexDir,
  run, runCritical, query, get, insert, insertCritical, persist, persistNow, markDirty,
  searchMemories, searchDecisions, decayPass,
  dumpActive, archiveEntries,
  DB_PATH, CORTEX_DIR
};
