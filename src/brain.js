'use strict';

const fs = require('fs');
const path = require('path');
const db = require('./db');
const { getEmbedding, cosineSim, vectorToBlob, blobToVector, buildEmbeddings } = require('./embeddings');
const { detectConflict, markDisputed } = require('./conflict');

const HOME = process.env.USERPROFILE || process.env.HOME;
const CLAUDE_DIR = path.join(HOME, '.claude');
const MEMORY_DIR = path.join(CLAUDE_DIR, 'projects', 'C--Users-aditya', 'memory');
const SIE_DIR = path.join(HOME, 'self-improvement-engine');
const STATE_PATH = path.join(CLAUDE_DIR, 'state.md');

// Sections we extract from state.md
const STATE_SECTIONS = ['## What Was Done', '## Next Session', '## Pending', '## Known Issues'];

// ─── Initialization ────────────────────────────────────────────────────────

/**
 * Initialize the brain: open DB, index all knowledge sources, build embeddings.
 */
async function init() {
  await db.getDb();
  await indexAll();
  await buildEmbeddings();
  logEvent('brain_init', { timestamp: new Date().toISOString() });
}

// ─── Knowledge Indexing ────────────────────────────────────────────────────

/**
 * Read all 6 knowledge sources and upsert into the memories table.
 */
async function indexAll() {
  let indexed = 0;

  indexed += indexStateFile();
  indexed += indexMemoryFiles();
  indexed += indexLessons();
  indexed += indexGoals();
  indexed += indexSkillTracker();
  indexed += indexGorci();

  if (indexed > 0) db.persist();
  logEvent('index_all', { indexed });

  return indexed;
}

/**
 * Upsert a memory by source path.
 * If a memory with the same source exists, update text + updated_at.
 * Otherwise, insert new.
 */
function upsertMemory(text, source, type = 'memory', sourceAgent = 'indexer') {
  if (!text || !text.trim()) return false;

  const existing = db.get(
    'SELECT id FROM memories WHERE source = ? AND status = ?',
    [source, 'active']
  );

  if (existing) {
    db.run(
      "UPDATE memories SET text = ?, updated_at = datetime('now') WHERE id = ?",
      [text.trim(), existing.id]
    );
    // Invalidate stale embedding so buildEmbeddings re-computes it
    db.run(
      'DELETE FROM embeddings WHERE target_type = ? AND target_id = ?',
      ['memory', existing.id]
    );
  } else {
    db.insert(
      'INSERT INTO memories (text, source, type, source_agent) VALUES (?, ?, ?, ?)',
      [text.trim(), source, type, sourceAgent]
    );
  }

  return true;
}

/**
 * Source 1: state.md — extract actionable sections.
 */
function indexStateFile() {
  if (!fs.existsSync(STATE_PATH)) return 0;

  let count = 0;

  try {
    const content = fs.readFileSync(STATE_PATH, 'utf-8');

    for (const section of STATE_SECTIONS) {
      const extracted = extractSection(content, section);
      if (extracted) {
        const source = `state.md::${section.replace('## ', '')}`;
        if (upsertMemory(extracted, source, 'state')) count++;
      }
    }
  } catch (err) {
    console.error(`[brain] Failed to index state.md: ${err.message}`);
  }

  return count;
}

/**
 * Extract a markdown section (from header to next ## header or EOF).
 */
function extractSection(markdown, header) {
  const idx = markdown.indexOf(header);
  if (idx === -1) return null;

  const start = idx + header.length;
  // Find next ## header or end of string
  const nextHeader = markdown.indexOf('\n## ', start);
  const end = nextHeader === -1 ? markdown.length : nextHeader;
  const text = markdown.slice(start, end).trim();

  return text || null;
}

/**
 * Source 2: Memory files at ~/.claude/projects/C--Users-aditya/memory/*.md
 * Parse YAML frontmatter for name, type, description.
 */
function indexMemoryFiles() {
  if (!fs.existsSync(MEMORY_DIR)) return 0;

  let count = 0;

  try {
    const files = fs.readdirSync(MEMORY_DIR).filter(f => f.endsWith('.md'));

    for (const file of files) {
      try {
        const filePath = path.join(MEMORY_DIR, file);
        const raw = fs.readFileSync(filePath, 'utf-8');
        const { frontmatter, body } = parseFrontmatter(raw);

        const name = frontmatter.name || path.basename(file, '.md');
        const type = frontmatter.type || 'memory';
        const description = frontmatter.description || '';

        // Compose a compact text representation
        const text = description
          ? `[${name}] (${type}) ${description}\n${body.slice(0, 500)}`
          : `[${name}] (${type})\n${body.slice(0, 500)}`;

        const source = `memory::${file}`;
        if (upsertMemory(text, source, type)) count++;
      } catch (err) {
        console.error(`[brain] Failed to index memory file ${file}: ${err.message}`);
      }
    }
  } catch (err) {
    console.error(`[brain] Failed to read memory dir: ${err.message}`);
  }

  return count;
}

/**
 * Minimal YAML frontmatter parser.
 * Extracts key: value pairs between --- delimiters.
 */
function parseFrontmatter(raw) {
  const frontmatter = {};
  let body = raw;

  if (raw.startsWith('---')) {
    const endIdx = raw.indexOf('---', 3);
    if (endIdx !== -1) {
      const yamlBlock = raw.slice(3, endIdx).trim();
      body = raw.slice(endIdx + 3).trim();

      for (const line of yamlBlock.split('\n')) {
        const colonIdx = line.indexOf(':');
        if (colonIdx > 0) {
          const key = line.slice(0, colonIdx).trim();
          const val = line.slice(colonIdx + 1).trim();
          frontmatter[key] = val;
        }
      }
    }
  }

  return { frontmatter, body };
}

/**
 * Source 3: Lessons at ~/self-improvement-engine/lessons/lessons.jsonl
 */
function indexLessons() {
  const filePath = path.join(SIE_DIR, 'lessons', 'lessons.jsonl');
  if (!fs.existsSync(filePath)) return 0;

  let count = 0;

  try {
    const lines = fs.readFileSync(filePath, 'utf-8').split('\n').filter(Boolean);

    for (const line of lines) {
      try {
        const entry = JSON.parse(line);
        const text = `[${entry.type || 'lesson'}] ${entry.lesson || ''}${entry.evidence ? ` — Evidence: ${entry.evidence}` : ''}`;
        const source = `lessons::${entry.skill || 'general'}::${entry.timestamp || 'unknown'}`;
        if (upsertMemory(text, source, 'lesson')) count++;
      } catch {
        // Skip malformed lines
      }
    }
  } catch (err) {
    console.error(`[brain] Failed to index lessons: ${err.message}`);
  }

  return count;
}

/**
 * Source 4: Goals at ~/self-improvement-engine/tools/goal-setter/current-goals.json
 */
function indexGoals() {
  const filePath = path.join(SIE_DIR, 'tools', 'goal-setter', 'current-goals.json');
  if (!fs.existsSync(filePath)) return 0;

  let count = 0;

  try {
    const data = JSON.parse(fs.readFileSync(filePath, 'utf-8'));
    const goals = data.goals || [];

    for (const goal of goals) {
      const text = `[Goal #${goal.rank}] ${goal.goal} (category: ${goal.category || 'unknown'}, priority: ${goal.priority?.toFixed(2) || '?'}, effort: ${goal.effort || '?'})`;
      const source = `goals::rank${goal.rank}`;
      if (upsertMemory(text, source, 'goal')) count++;
    }
  } catch (err) {
    console.error(`[brain] Failed to index goals: ${err.message}`);
  }

  return count;
}

/**
 * Source 5: Skill tracker at ~/self-improvement-engine/tools/skill-tracker/invocations.jsonl
 * Aggregate by skill to produce per-skill summaries instead of one row per invocation.
 */
function indexSkillTracker() {
  const filePath = path.join(SIE_DIR, 'tools', 'skill-tracker', 'invocations.jsonl');
  if (!fs.existsSync(filePath)) return 0;

  let count = 0;

  try {
    const lines = fs.readFileSync(filePath, 'utf-8').split('\n').filter(Boolean);
    const bySkill = {};

    for (const line of lines) {
      try {
        const entry = JSON.parse(line);
        const skill = entry.skill || 'unknown';
        if (!bySkill[skill]) {
          bySkill[skill] = { total: 0, success: 0, correction: 0, retry: 0, last: entry.timestamp };
        }
        bySkill[skill].total++;
        if (entry.outcome === 'success') bySkill[skill].success++;
        else if (entry.outcome === 'correction') bySkill[skill].correction++;
        else if (entry.outcome === 'retry') bySkill[skill].retry++;
        if (entry.timestamp > bySkill[skill].last) bySkill[skill].last = entry.timestamp;
      } catch {
        // Skip malformed lines
      }
    }

    for (const [skill, stats] of Object.entries(bySkill)) {
      const rate = stats.total > 0 ? ((stats.success / stats.total) * 100).toFixed(0) : '0';
      const text = `[Skill: ${skill}] ${stats.total} invocations, ${rate}% success (${stats.correction} corrections, ${stats.retry} retries). Last: ${stats.last}`;
      const source = `skills::${skill}`;
      if (upsertMemory(text, source, 'skill_stats')) count++;
    }
  } catch (err) {
    console.error(`[brain] Failed to index skill tracker: ${err.message}`);
  }

  return count;
}

/**
 * Source 6: GORCI status at ~/self-improvement-engine/tools/gorci/last-run.json
 */
function indexGorci() {
  const filePath = path.join(SIE_DIR, 'tools', 'gorci', 'last-run.json');
  if (!fs.existsSync(filePath)) return 0;

  let count = 0;

  try {
    const data = JSON.parse(fs.readFileSync(filePath, 'utf-8'));
    const text = `[GORCI] Skill: ${data.skill || 'unknown'}, Mode: ${data.mode || '?'}, Tier: ${data.tier || '?'}, Cases: ${data.cases || 0}, Pass: ${data.pass ?? '?'}, Score: ${data.overallScore ?? '?'}. Run: ${data.timestamp || 'unknown'}`;
    const source = `gorci::last-run`;
    if (upsertMemory(text, source, 'gorci')) count++;
  } catch (err) {
    console.error(`[brain] Failed to index gorci: ${err.message}`);
  }

  return count;
}

// ─── Recall (Hybrid Search) ───────────────────────────────────────────────

/**
 * Hybrid search: semantic + keyword. Returns top k results with
 * { source, relevance, excerpt, method }.
 */
async function recall(queryText, k = 7) {
  if (!queryText || typeof queryText !== 'string') return [];

  const results = new Map(); // source → result object

  // 1. Semantic search via embeddings
  const queryVec = await getEmbedding(queryText);

  if (queryVec) {
    const queryBlob = vectorToBlob(queryVec);
    const allEmbeddings = db.query('SELECT target_type, target_id, vector FROM embeddings');

    for (const row of allEmbeddings) {
      const sim = cosineSim(queryBlob, row.vector);
      if (sim < 0.3) continue;

      const { text, source } = loadTarget(row.target_type, row.target_id);
      if (!text || !source) continue;

      const key = source;
      const existing = results.get(key);
      if (!existing || sim > existing.relevance) {
        results.set(key, {
          source,
          relevance: parseFloat(sim.toFixed(4)),
          excerpt: text.slice(0, 200),
          method: 'semantic',
        });
      }
    }
  }

  // 2. Keyword search
  const keywords = extractKeywords(queryText);
  const keywordQuery = keywords.length > 0 ? keywords.join(' ') : queryText;

  const memResults = db.searchMemories(keywordQuery);
  for (const row of memResults) {
    const key = row.source || `memory::${row.id}`;
    const existing = results.get(key);
    // Keyword results get a base relevance of 0.5, scaled by score
    const relevance = parseFloat((0.5 * (row.score || 1.0)).toFixed(4));
    if (!existing || relevance > existing.relevance) {
      results.set(key, {
        source: key,
        relevance,
        excerpt: (row.text || '').slice(0, 200),
        method: existing?.method === 'semantic' ? 'hybrid' : 'keyword',
      });
    }
  }

  const decResults = db.searchDecisions(keywordQuery);
  for (const row of decResults) {
    const key = row.context || `decision::${row.id}`;
    const existing = results.get(key);
    const relevance = parseFloat((0.5 * (row.score || 1.0)).toFixed(4));
    if (!existing || relevance > existing.relevance) {
      results.set(key, {
        source: key,
        relevance,
        excerpt: (row.decision || '').slice(0, 200),
        method: existing?.method === 'semantic' ? 'hybrid' : 'keyword',
      });
    }
  }

  // 3. Sort by relevance descending, return top k
  const sorted = [...results.values()].sort((a, b) => b.relevance - a.relevance);

  // Bump retrieval counters for returned results (fire-and-forget)
  for (const r of sorted.slice(0, k)) {
    bumpRetrieval(r.source);
  }

  return sorted.slice(0, k);
}

/**
 * Load the text + source for a given target_type + target_id.
 */
function loadTarget(targetType, targetId) {
  if (targetType === 'memory') {
    const row = db.get('SELECT text, source FROM memories WHERE id = ? AND status = ?', [targetId, 'active']);
    return row ? { text: row.text, source: row.source || `memory::${targetId}` } : { text: null, source: null };
  }
  if (targetType === 'decision') {
    const row = db.get('SELECT decision AS text, context AS source FROM decisions WHERE id = ? AND status = ?', [targetId, 'active']);
    return row ? { text: row.text, source: row.source || `decision::${targetId}` } : { text: null, source: null };
  }
  return { text: null, source: null };
}

/**
 * Extract meaningful keywords from a query (drop stop words, short tokens).
 */
function extractKeywords(text) {
  const stopWords = new Set([
    'the', 'a', 'an', 'is', 'are', 'was', 'were', 'be', 'been', 'being',
    'have', 'has', 'had', 'do', 'does', 'did', 'will', 'would', 'could',
    'should', 'may', 'might', 'shall', 'can', 'to', 'of', 'in', 'for',
    'on', 'with', 'at', 'by', 'from', 'as', 'into', 'about', 'that',
    'this', 'it', 'its', 'not', 'but', 'and', 'or', 'if', 'then',
    'so', 'what', 'which', 'who', 'how', 'when', 'where', 'why',
    'all', 'each', 'every', 'both', 'few', 'more', 'most', 'some',
    'any', 'no', 'my', 'your', 'his', 'her', 'our', 'their', 'i', 'me',
  ]);

  return text
    .toLowerCase()
    .replace(/[^a-z0-9\s-]/g, ' ')
    .split(/\s+/)
    .filter(w => w.length > 2 && !stopWords.has(w));
}

/**
 * Increment the retrieval counter for a source.
 */
function bumpRetrieval(source) {
  try {
    db.run(
      'UPDATE memories SET retrievals = retrievals + 1 WHERE source = ?',
      [source]
    );
    db.run(
      'UPDATE decisions SET retrievals = retrievals + 1 WHERE context = ?',
      [source]
    );
  } catch {
    // Non-critical — don't let counter bumps break recall
  }
}

// ─── Store (Decision Ingestion) ───────────────────────────────────────────

/**
 * Store a new decision with conflict detection, dedup, and surprise scoring.
 * opts: { context, type, source_agent, confidence }
 */
async function store(decision, opts = {}) {
  const {
    context = null,
    type = 'decision',
    source_agent = 'unknown',
    confidence = 0.8,
  } = opts;

  if (!decision || typeof decision !== 'string') {
    return { stored: false, reason: 'empty_decision' };
  }

  // 1. Conflict detection via conflict.js (async, uses embeddings or Jaccard fallback)
  const conflict = await detectConflict(decision, source_agent);

  if (conflict.isConflict) {
    // Different agent conflict: insert new as disputed, then mark both via markDisputed
    const newId = db.insert(
      'INSERT INTO decisions (decision, context, type, source_agent, confidence, status, disputes_id) VALUES (?, ?, ?, ?, ?, ?, ?)',
      [decision, context, type, source_agent, confidence, 'disputed', conflict.matchedId]
    );

    markDisputed(newId, conflict.matchedId);
    logEvent('decision_conflict', { newId, existingId: conflict.matchedId, source_agent, matchedAgent: conflict.matchedAgent });

    // Async embed (fire-and-forget)
    embedDecisionAsync(newId, decision);

    return { stored: true, id: newId, status: 'disputed', conflictWith: conflict.matchedId };
  }

  // 2. Same-agent update: conflict.js detected high similarity from the same agent
  if (conflict.isUpdate && conflict.matchedId) {
    db.run(
      "UPDATE decisions SET status = 'superseded', updated_at = datetime('now') WHERE id = ?",
      [conflict.matchedId]
    );

    const newId = db.insert(
      'INSERT INTO decisions (decision, context, type, source_agent, confidence, supersedes_id) VALUES (?, ?, ?, ?, ?, ?)',
      [decision, context, type, source_agent, confidence, conflict.matchedId]
    );

    logEvent('decision_supersede', { newId, supersededId: conflict.matchedId, source_agent });
    embedDecisionAsync(newId, decision);

    return { stored: true, id: newId, status: 'superseded_old', supersedes: conflict.matchedId };
  }

  // 3. Jaccard surprise against all existing — reject duplicates
  const existingDecisions = db.query(
    'SELECT * FROM decisions WHERE status = ? ORDER BY created_at DESC LIMIT 50',
    ['active']
  );

  let maxSimilarity = 0;
  for (const existing of existingDecisions) {
    const sim = jaccardSimilarity(decision, existing.decision);
    if (sim > maxSimilarity) maxSimilarity = sim;
  }

  const surprise = 1 - maxSimilarity;
  if (surprise < 0.25) {
    logEvent('decision_rejected_duplicate', { decision: decision.slice(0, 100), surprise, source_agent });
    return { stored: false, reason: 'duplicate', surprise };
  }

  // 4. Insert new decision
  const id = db.insert(
    'INSERT INTO decisions (decision, context, type, source_agent, confidence, surprise) VALUES (?, ?, ?, ?, ?, ?)',
    [decision, context, type, source_agent, confidence, parseFloat(surprise.toFixed(4))]
  );

  logEvent('decision_stored', { id, source_agent, surprise });

  // Async embed (fire-and-forget)
  embedDecisionAsync(id, decision);

  return { stored: true, id, status: 'active', surprise };
}

/**
 * Jaccard similarity between two text strings (word-level).
 */
function jaccardSimilarity(a, b) {
  const wordsA = new Set(a.toLowerCase().split(/\s+/).filter(w => w.length > 1));
  const wordsB = new Set(b.toLowerCase().split(/\s+/).filter(w => w.length > 1));

  if (wordsA.size === 0 && wordsB.size === 0) return 1;
  if (wordsA.size === 0 || wordsB.size === 0) return 0;

  let intersection = 0;
  for (const w of wordsA) {
    if (wordsB.has(w)) intersection++;
  }

  const union = wordsA.size + wordsB.size - intersection;
  return union === 0 ? 0 : intersection / union;
}

/**
 * Embed a decision asynchronously (fire-and-forget).
 */
function embedDecisionAsync(id, text) {
  getEmbedding(text)
    .then(vec => {
      if (vec) {
        db.insert(
          'INSERT INTO embeddings (target_type, target_id, vector, model) VALUES (?, ?, ?, ?)',
          ['decision', id, vectorToBlob(vec), 'nomic-embed-text']
        );
        db.persist();
      }
    })
    .catch(() => {
      // Non-critical — embedding will be picked up by next buildEmbeddings() run
    });
}

// ─── Forget (Decay) ──────────────────────────────────────────────────────

/**
 * Find matching decisions/memories by keyword and multiply their score by 0.3.
 */
function forget(keyword) {
  if (!keyword || typeof keyword !== 'string') return { affected: 0 };

  const pattern = `%${keyword}%`;
  let affected = 0;

  const memories = db.query(
    'SELECT id FROM memories WHERE status = ? AND (text LIKE ? OR source LIKE ?)',
    ['active', pattern, pattern]
  );
  for (const row of memories) {
    db.run('UPDATE memories SET score = score * 0.3 WHERE id = ?', [row.id]);
    affected++;
  }

  const decisions = db.query(
    'SELECT id FROM decisions WHERE status = ? AND (decision LIKE ? OR context LIKE ?)',
    ['active', pattern, pattern]
  );
  for (const row of decisions) {
    db.run('UPDATE decisions SET score = score * 0.3 WHERE id = ?', [row.id]);
    affected++;
  }

  if (affected > 0) {
    db.persist();
    logEvent('forget', { keyword, affected });
  }

  return { affected };
}

// ─── Stats ────────────────────────────────────────────────────────────────

/**
 * Return DB stats + Ollama status.
 */
async function getStats() {
  const dbStats = db.getStats();

  let ollamaStatus = 'unknown';
  try {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), 3000);
    const res = await fetch(`${require('./embeddings').OLLAMA_URL}/api/tags`, {
      signal: controller.signal,
    });
    clearTimeout(timer);
    ollamaStatus = res.ok ? 'connected' : `error_${res.status}`;
  } catch {
    ollamaStatus = 'offline';
  }

  return {
    ...dbStats,
    ollama: ollamaStatus,
    home: HOME,
  };
}

// ─── Diary (state.md writer) ──────────────────────────────────────────────

// Sections that are permanent and must never be overwritten by diary writes
const PERMANENT_SECTIONS = ['## DO NOT REMOVE'];

/**
 * Write state.md — update dynamic sections while preserving permanent ones.
 * data: { accomplished, nextSteps, pending, knownIssues }
 */
function writeDiary(data = {}) {
  const { accomplished, nextSteps, pending, knownIssues } = data;

  let existing = '';
  if (fs.existsSync(STATE_PATH)) {
    existing = fs.readFileSync(STATE_PATH, 'utf-8');
  }

  // Preserve permanent sections
  const preserved = [];
  for (const marker of PERMANENT_SECTIONS) {
    const section = extractSection(existing, marker);
    if (section) {
      preserved.push({ header: marker, content: section });
    }
  }

  const now = new Date().toISOString().slice(0, 10);
  const lines = [`# Session State — ${now}`, ''];

  // Write permanent sections first
  for (const p of preserved) {
    lines.push(p.header);
    lines.push(p.content);
    lines.push('');
  }

  if (accomplished) {
    lines.push('## What Was Done This Session');
    lines.push(accomplished);
    lines.push('');
  }

  if (nextSteps) {
    lines.push('## Next Session');
    lines.push(nextSteps);
    lines.push('');
  }

  if (pending) {
    lines.push('## Pending');
    lines.push(pending);
    lines.push('');
  }

  if (knownIssues) {
    lines.push('## Known Issues');
    lines.push(knownIssues);
    lines.push('');
  }

  // Preserve any existing Key Decisions section if not overwritten
  // Accept both 'decisions' (API field name) and 'keyDecisions' (legacy)
  const keyDecisions = data.decisions || data.keyDecisions;
  const existingDecisions = extractSection(existing, '## Key Decisions');
  if (existingDecisions && !keyDecisions) {
    lines.push('## Key Decisions');
    lines.push(existingDecisions);
    lines.push('');
  } else if (keyDecisions) {
    lines.push('## Key Decisions');
    lines.push(keyDecisions);
    lines.push('');
  }

  const dir = path.dirname(STATE_PATH);
  if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });

  fs.writeFileSync(STATE_PATH, lines.join('\n'), 'utf-8');

  logEvent('diary_write', { date: now });

  return { written: true, path: STATE_PATH };
}

// ─── Event Logging ────────────────────────────────────────────────────────

/**
 * Insert an event into the events table.
 */
function logEvent(type, data = {}, sourceAgent = 'brain') {
  try {
    db.insert(
      'INSERT INTO events (type, data, source_agent) VALUES (?, ?, ?)',
      [type, JSON.stringify(data), sourceAgent]
    );
  } catch {
    // Events are best-effort — never let logging break the caller
  }
}

// ─── Exports ──────────────────────────────────────────────────────────────

module.exports = {
  init,
  indexAll,
  recall,
  store,
  forget,
  getStats,
  writeDiary,
  logEvent,

  // Exposed for testing
  _internal: {
    upsertMemory,
    parseFrontmatter,
    extractSection,
    extractKeywords,
    jaccardSimilarity,
    loadTarget,
    bumpRetrieval,
    indexStateFile,
    indexMemoryFiles,
    indexLessons,
    indexGoals,
    indexSkillTracker,
    indexGorci,
  },
};
