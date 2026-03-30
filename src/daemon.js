'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const readline = require('readline');

const brain = require('./brain');
const compiler = require('./compiler');
const conflict = require('./conflict');
const db = require('./db');

// ─── Constants ─────────────────────────────────────────────────────────────

const PORT = 7437;
const MAX_BODY = 10 * 1024; // 10KB
const CORTEX_DIR = db.CORTEX_DIR;
const PID_PATH = path.join(CORTEX_DIR, 'cortex.pid');
const TOKEN_PATH = path.join(CORTEX_DIR, 'cortex.token');
const LOG_PATH = path.join(CORTEX_DIR, 'cortex.log');

// ─── State ─────────────────────────────────────────────────────────────────

let authToken = null;
let httpServer = null;
let mcpCalls = 0;
let shuttingDown = false;
let keepAliveInterval = null;

// Conductor state (in-memory for MVP)
const locks = new Map(); // path -> { id, agent, lockedAt, expiresAt }
const activities = []; // { id, agent, description, files, timestamp }
const messages = []; // { id, from, to, message, timestamp }
const sessions = new Map(); // agent -> { sessionId, agent, project, files, description, startedAt, lastHeartbeat, expiresAt }
const tasks = new Map(); // taskId -> TaskObject
const feed = []; // { id, agent, kind, summary, content, files, taskId, traceId, priority, timestamp, tokens }
const feedAcks = new Map(); // agent -> lastSeenId
const sseClients = new Set(); // active SSE connections

// ─── Predictive Context Cache ─────────────────────────────────────────────
// Tracks recall patterns per agent. When an agent recalls X, predict what
// they'll recall next and pre-cache it. Eliminates cold-start on common flows.
const recallHistory = new Map(); // agent -> [{ query, timestamp }]
const preCache = new Map(); // agent -> { query, results, expires }
const PRECACHE_TTL = 5 * 60 * 1000; // 5 min
const MAX_RECALL_HISTORY = 50;

function recordRecallPattern(agent, query) {
  if (!recallHistory.has(agent)) recallHistory.set(agent, []);
  const history = recallHistory.get(agent);
  history.push({ query, timestamp: Date.now() });
  if (history.length > MAX_RECALL_HISTORY) history.shift();

  // Predict next query: if this agent has done A→B before, pre-cache B
  predictAndCache(agent, query).catch(() => {});
}

async function predictAndCache(agent, currentQuery) {
  const history = recallHistory.get(agent) || [];
  if (history.length < 3) return;

  // Find what query typically follows the current one
  const followers = new Map(); // query -> count
  for (let i = 0; i < history.length - 1; i++) {
    if (history[i].query === currentQuery) {
      const next = history[i + 1].query;
      followers.set(next, (followers.get(next) || 0) + 1);
    }
  }

  if (followers.size === 0) return;

  // Get the most common follower
  const [predicted] = [...followers.entries()].sort((a, b) => b[1] - a[1])[0];
  if (!predicted || predicted === currentQuery) return;

  // Pre-cache the predicted recall
  try {
    const results = await brain.budgetRecall(predicted, 200, 5);
    preCache.set(agent, {
      query: predicted,
      results,
      expires: Date.now() + PRECACHE_TTL,
    });
  } catch { /* non-critical */ }
}

// ─── Context Dedup (Bloom-like) ───────────────────────────────────────────
// Track what content has been served to each agent this session.
// Prevents re-serving the same info in boot + recall.
const servedContent = new Map(); // agent -> Set<hash>

function hashContent(text) {
  // Simple FNV-1a 32-bit hash — fast, good enough for dedup
  let hash = 2166136261;
  for (let i = 0; i < Math.min(text.length, 100); i++) {
    hash ^= text.charCodeAt(i);
    hash = (hash * 16777619) >>> 0;
  }
  return hash;
}

function markServed(agent, text) {
  if (!text) return;
  if (!servedContent.has(agent)) servedContent.set(agent, new Set());
  servedContent.get(agent).add(hashContent(text));
}

function wasServed(agent, text) {
  if (!text) return false;
  const set = servedContent.get(agent);
  if (!set) return false;
  return set.has(hashContent(text));
}

function clearServedOnBoot(agent) {
  // Reset dedup tracking on boot — new session starts fresh
  servedContent.delete(agent);
}

function getPreCached(agent, query) {
  const cached = preCache.get(agent);
  if (!cached) return null;
  if (cached.query !== query) return null;
  if (Date.now() > cached.expires) {
    preCache.delete(agent);
    return null;
  }
  return cached.results;
}
const MAX_ACTIVITIES = 1000;
const MAX_MESSAGES_PER_AGENT = 100;
const MAX_TASKS = 500;
const MAX_FEED = 200;
const FEED_TTL_MS = 4 * 60 * 60 * 1000; // 4 hours
const SESSION_TTL = 120; // seconds (2 minutes default)
const PRIORITY_RANK = { critical: 4, high: 3, medium: 2, low: 1 };

// Helper to generate UUID
function generateId() {
  return crypto.randomUUID();
}

// Helper to parse duration string (e.g., "5m", "1h", "1d")
function parseDuration(durationStr) {
  const match = durationStr.match(/^(\d+)([mhd])$/);
  if (!match) return 60 * 60 * 1000; // Default 1 hour

  const value = parseInt(match[1], 10);
  const unit = match[2];
  const multipliers = { m: 60 * 1000, h: 60 * 60 * 1000, d: 24 * 60 * 60 * 1000 };
  return value * multipliers[unit];
}

// Helper to clean expired locks
function cleanExpiredLocks() {
  const now = new Date();
  for (const [path, lock] of locks) {
    try {
      if (!lock.expiresAt) continue;
      const expiresDate = new Date(lock.expiresAt);
      // Check if the date is valid (not Invalid Date)
      if (isNaN(expiresDate.getTime())) continue;
      if (expiresDate < now) {
        locks.delete(path);
        log('info', `Lock expired: ${path}`);
      }
    } catch (err) {
      // Silently skip malformed lock entries, don't break cleanup
      log('warn', `Skipping malformed lock entry for ${path}: ${err.message}`);
    }
  }
}

// Helper to clean old activities (FIFO)
function cleanOldActivities() {
  if (activities.length > MAX_ACTIVITIES) {
    const removeCount = activities.length - MAX_ACTIVITIES;
    activities.splice(0, removeCount);
  }
}

// Helper to clean expired sessions
function cleanExpiredSessions() {
  const now = new Date();
  for (const [agent, session] of sessions) {
    if (new Date(session.expiresAt) < now) {
      sessions.delete(agent);
      log('info', `Session expired: ${agent}`);
    }
  }
}

// Helper to evict old completed tasks (FIFO)
function cleanOldTasks() {
  if (tasks.size > MAX_TASKS) {
    const completed = Array.from(tasks.values())
      .filter(t => t.status === 'completed')
      .sort((a, b) => new Date(a.completedAt) - new Date(b.completedAt));
    const removeCount = tasks.size - MAX_TASKS;
    for (let i = 0; i < Math.min(removeCount, completed.length); i++) {
      tasks.delete(completed[i].taskId);
    }
  }
}

// Helper to clean old feed entries (TTL + FIFO)
function cleanOldFeed() {
  const cutoff = Date.now() - FEED_TTL_MS;
  while (feed.length > 0 && new Date(feed[0].timestamp).getTime() < cutoff) {
    feed.shift();
  }
  while (feed.length > MAX_FEED) {
    feed.shift();
  }
}

// Helper to redact secrets from text
function redactSecrets(text) {
  if (!text) return text;
  return text
    .replace(/Bearer\s+[a-f0-9]{32,}/gi, 'Bearer [REDACTED]')
    .replace(/[a-f0-9]{40,}/gi, '[HASH_REDACTED]')
    .replace(/(?:token|key|secret|password)\s*[:=]\s*\S+/gi, '[CREDENTIAL_REDACTED]');
}

// Helper to get unread feed entries for an agent
// SSE: broadcast event to all connected clients
function emitEvent(type, data) {
  const payload = JSON.stringify({ type, ...data, timestamp: new Date().toISOString() });
  const msg = `event: ${type}\ndata: ${payload}\n\n`;
  for (const res of sseClients) {
    try { res.write(msg); } catch { sseClients.delete(res); }
  }
}

function getUnreadFeed(forAgent) {
  const lastSeenId = feedAcks.get(forAgent);
  if (!lastSeenId) return feed.filter(e => e.agent !== forAgent);

  let pastAck = false;
  const unread = [];
  for (const entry of feed) {
    if (entry.id === lastSeenId) { pastAck = true; continue; }
    if (pastAck && entry.agent !== forAgent) unread.push(entry);
  }
  return unread;
}

// Helper to auto-post feed entry (used by task complete)
function autoPostFeed(entry) {
  cleanOldFeed();
  const feedEntry = {
    id: generateId(),
    agent: entry.agent,
    kind: entry.kind,
    summary: redactSecrets(entry.summary),
    content: entry.content ? redactSecrets(entry.content) : null,
    files: entry.files || [],
    taskId: entry.taskId || null,
    traceId: entry.traceId || null,
    priority: entry.priority || 'normal',
    timestamp: new Date().toISOString(),
    tokens: entry.summary ? Math.ceil(entry.summary.length / 4) : 0
  };
  feed.push(feedEntry);
  emitEvent('feed', { feedId: feedEntry.id, agent: feedEntry.agent, kind: feedEntry.kind, summary: feedEntry.summary });
  return feedEntry;
}

// Helper to clean old messages (FIFO per agent)
function cleanOldMessages(agent) {
  const agentMessages = messages.filter(m => m.to === agent);
  if (agentMessages.length > MAX_MESSAGES_PER_AGENT) {
    const removeCount = agentMessages.length - MAX_MESSAGES_PER_AGENT;
    const toRemove = agentMessages.slice(0, removeCount);
    for (const msg of toRemove) {
      const idx = messages.indexOf(msg);
      if (idx !== -1) messages.splice(idx, 1);
    }
  }
}

// ─── Logging ───────────────────────────────────────────────────────────────

let logStream = null;

function openLogStream() {
  db.ensureCortexDir();
  logStream = fs.createWriteStream(LOG_PATH, { flags: 'a' });
}

function log(level, msg, data) {
  const ts = new Date().toISOString();
  const entry = data
    ? `[${ts}] [${level}] ${msg} ${JSON.stringify(data)}`
    : `[${ts}] [${level}] ${msg}`;

  const canWriteToFile = logStream && !logStream.destroyed && !logStream.writableEnded;

  if (canWriteToFile) {
    try {
      logStream.write(entry + '\n');
      return;
    } catch {
      // Fall through to stderr fallback
    }
  }

  // Fallback before log stream is open or after it is closed
  if (process.stderr?.writable) {
    try {
      process.stderr.write(entry + '\n');
    } catch {
      // Best-effort logging only; never crash on logger fallback
    }
  }
}

// ─── Auth ──────────────────────────────────────────────────────────────────

function generateToken() {
  db.ensureCortexDir();
  authToken = crypto.randomBytes(32).toString('hex');
  fs.writeFileSync(TOKEN_PATH, authToken, 'utf-8');
  log('info', 'Auth token generated');
}

function validateAuth(req) {
  const header = req.headers['authorization'] || '';
  const match = header.match(/^Bearer\s+(.+)$/i);
  return match && match[1] === authToken;
}

function validateHost(req) {
  const host = (req.headers['host'] || '').replace(/:\d+$/, '');
  return host === 'localhost' || host === '127.0.0.1' || host === '[::1]';
}

// ─── PID file ──────────────────────────────────────────────────────────────

function writePid() {
  db.ensureCortexDir();
  fs.writeFileSync(PID_PATH, String(process.pid), 'utf-8');
  log('info', `PID file written: ${process.pid}`);
}

function removePid() {
  try {
    if (fs.existsSync(PID_PATH)) fs.unlinkSync(PID_PATH);
  } catch {
    // Best effort
  }
}

/**
 * Kill any stale daemon process before starting a new one.
 * Handles Windows (taskkill) and Unix (process.kill) gracefully.
 */
function killStaleDaemon() {
  let stalePid;
  try {
    stalePid = parseInt(fs.readFileSync(PID_PATH, 'utf-8').trim(), 10);
  } catch {
    return; // No PID file — nothing stale
  }

  if (!stalePid || stalePid === process.pid) return;

  // Check if the process is actually running
  try {
    process.kill(stalePid, 0); // Signal 0 = existence check
  } catch {
    // Process doesn't exist — just clean up stale files
    log('info', `Stale PID file found (${stalePid}), process already dead — cleaning up`);
    removePid();
    return;
  }

  // Process exists — kill it
  log('info', `Killing stale daemon (PID ${stalePid})`);
  try {
    if (process.platform === 'win32') {
      require('child_process').execSync(`taskkill /PID ${stalePid} /F`, { stdio: 'ignore' });
    } else {
      process.kill(stalePid, 'SIGTERM');
    }
  } catch (err) {
    log('warn', `Could not kill stale daemon (PID ${stalePid}): ${err.message}`);
  }

  // Wait briefly for port release
  removePid();
}

// ─── HTTP helpers ──────────────────────────────────────────────────────────

function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;
    let destroyed = false;

    req.on('data', (chunk) => {
      size += chunk.length;
      if (size > MAX_BODY) {
        req.destroy();
        destroyed = true;
        reject(new Error('Request body too large'));
        return;
      }
      chunks.push(chunk);
    });

    req.on('end', () => {
      if (destroyed) return; // Don't resolve if we already rejected
      resolve(Buffer.concat(chunks).toString('utf-8'));
    });
    req.on('error', reject);
  });
}

function sendJson(res, statusCode, data) {
  const body = JSON.stringify(data);
  res.writeHead(statusCode, {
    'Content-Type': 'application/json',
    'Content-Length': Buffer.byteLength(body),
    'Cache-Control': 'no-store',
    'Access-Control-Allow-Origin': '*',
    'Access-Control-Allow-Headers': 'Authorization, Content-Type',
  });
  res.end(body);
}

function sendError(res, statusCode, message) {
  sendJson(res, statusCode, { error: message });
}

function parseQuery(url) {
  const idx = url.indexOf('?');
  if (idx === -1) return {};
  const params = {};
  const search = url.slice(idx + 1);
  for (const pair of search.split('&')) {
    const eqIdx = pair.indexOf('=');
    if (eqIdx === -1) {
      params[decodeURIComponent(pair)] = '';
    } else {
      params[decodeURIComponent(pair.slice(0, eqIdx))] = decodeURIComponent(pair.slice(eqIdx + 1));
    }
  }
  return params;
}

function getPathname(url) {
  const idx = url.indexOf('?');
  return idx === -1 ? url : url.slice(0, idx);
}

// ─── HTTP route handlers ───────────────────────────────────────────────────

async function handleBoot(req, res) {
  const params = parseQuery(req.url);
  const profile = params.profile || 'full';
  const agent = params.agent || req.headers['x-source-agent'] || 'unknown';

  try {
    // Prepare conductor state for boot injection
    cleanExpiredLocks();
    cleanExpiredSessions();
    cleanOldFeed();
    const unreadFeed = getUnreadFeed(agent);
    const conductorState = {
      locks: Array.from(locks.values()),
      messages: messages.filter(m => m.to === agent),
      sessions: Array.from(sessions.values()),
      tasks: Array.from(tasks.values()),
      feed: unreadFeed.slice(-10) // cap at 10 most recent
    };

    // Auto-ack feed on boot
    if (unreadFeed.length > 0) {
      feedAcks.set(agent, unreadFeed[unreadFeed.length - 1].id);
    }

    // Use capsule compiler when agent is identified, legacy otherwise
    const result = (agent && agent !== 'unknown')
      ? compiler.compileCapsules(agent, parseInt(params.budget, 10) || 600, conductorState)
      : compiler.compile(profile);

    sendJson(res, 200, {
      bootPrompt: result.bootPrompt,
      tokenEstimate: result.tokenEstimate,
      profile: result.profile,
      capsules: result.capsules || null,
      savings: result.savings || null,
    });
  } catch (err) {
    log('error', 'boot failed', { error: err.message });
    sendError(res, 500, `Boot failed: ${err.message}`);
  }
}

async function handleRecall(req, res) {
  const params = parseQuery(req.url);
  const q = params.q || '';
  const budget = params.budget !== undefined ? parseInt(params.budget, 10) : 200;
  const agent = req.headers['x-source-agent'] || 'http';

  if (!q) {
    sendError(res, 400, 'Missing query parameter: q');
    return;
  }

  try {
    // Check predictive cache first
    const cached = getPreCached(agent, q);
    if (cached) {
      sendJson(res, 200, { results: cached, budget, spent: 0, cached: true });
      return;
    }

    if (budget === 0) {
      // Headlines only mode
      const raw = await brain.recall(q, 10);
      recordRecallPattern(agent, q);
      const headlines = raw.map(r => ({ source: r.source, relevance: r.relevance, method: r.method }));
      sendJson(res, 200, { count: headlines.length, results: headlines, budget: 0, mode: 'headlines' });
      return;
    }

    const results = await brain.budgetRecall(q, budget, 10);
    recordRecallPattern(agent, q);
    for (const r of results) markServed(agent, r.excerpt);

    const spent = results.reduce((sum, r) => sum + (r.tokens || 0), 0);
    sendJson(res, 200, { results, budget, spent, saved: budget - spent, mode: budget >= 500 ? 'full' : 'balanced' });
  } catch (err) {
    log('error', 'recall failed', { error: err.message });
    sendError(res, 500, `Recall failed: ${err.message}`);
  }
}

async function handlePeek(req, res) {
  const params = parseQuery(req.url);
  const q = params.q || '';
  const k = parseInt(params.k, 10) || 10;

  if (!q) {
    sendError(res, 400, 'Missing query parameter: q');
    return;
  }

  try {
    const results = await brain.recall(q, k);
    // Strip excerpts — return only source, relevance, method (~80% token savings)
    const peek = results.map(r => ({
      source: r.source,
      relevance: r.relevance,
      method: r.method,
    }));
    sendJson(res, 200, { count: peek.length, matches: peek });
  } catch (err) {
    log('error', 'peek failed', { error: err.message });
    sendError(res, 500, `Peek failed: ${err.message}`);
  }
}

async function handleBudgetRecall(req, res) {
  const params = parseQuery(req.url);
  const q = params.q || '';
  const budget = parseInt(params.budget, 10) || 300;
  const k = parseInt(params.k, 10) || 10;

  if (!q) {
    sendError(res, 400, 'Missing query parameter: q');
    return;
  }

  try {
    const results = await brain.budgetRecall(q, budget, k);
    const totalTokens = results.reduce((sum, r) => sum + (r.tokens || 0), 0);
    sendJson(res, 200, { results, budget, spent: totalTokens, saved: budget - totalTokens });
  } catch (err) {
    sendError(res, 500, `Budget recall failed: ${err.message}`);
  }
}

async function handleStore(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.decision) {
      sendError(res, 400, 'Missing field: decision');
      return;
    }

    const sourceAgent = req.headers['x-source-agent'] || body.source_agent || 'http';
    const result = await brain.store(body.decision, {
      context: body.context || null,
      type: body.type || 'decision',
      source_agent: sourceAgent,
      confidence: body.confidence ?? 0.8,
    });

    sendJson(res, 200, { stored: result.stored, entry: result });
  } catch (err) {
    if (err.message === 'Request body too large') {
      sendError(res, 413, 'Request body too large (max 10KB)');
      return;
    }
    log('error', 'store failed', { error: err.message });
    sendError(res, 500, `Store failed: ${err.message}`);
  }
}

async function handleDiary(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    const result = brain.writeDiary({
      accomplished: body.accomplished || null,
      nextSteps: body.nextSteps || null,
      decisions: body.decisions || null,
      pending: body.pending || null,
      knownIssues: body.knownIssues || null,
    });

    sendJson(res, 200, { written: result.written });
  } catch (err) {
    if (err.message === 'Request body too large') {
      sendError(res, 413, 'Request body too large (max 10KB)');
      return;
    }
    log('error', 'diary failed', { error: err.message });
    sendError(res, 500, `Diary failed: ${err.message}`);
  }
}

async function handleHealth(req, res) {
  try {
    const stats = await brain.getStats();
    sendJson(res, 200, { status: 'ok', stats });
  } catch (err) {
    log('error', 'health failed', { error: err.message });
    sendError(res, 500, `Health check failed: ${err.message}`);
  }
}

async function handleDigest(req, res) {
  try {
    const digest = brain.getDigest();
    sendJson(res, 200, digest);
  } catch (err) {
    log('error', 'digest failed', { error: err.message });
    sendError(res, 500, `Digest failed: ${err.message}`);
  }
}

async function handleForget(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.keyword) {
      sendError(res, 400, 'Missing field: keyword');
      return;
    }

    const result = brain.forget(body.keyword);
    sendJson(res, 200, { affected: result.affected });
  } catch (err) {
    if (err.message === 'Request body too large') {
      sendError(res, 413, 'Request body too large (max 10KB)');
      return;
    }
    log('error', 'forget failed', { error: err.message });
    sendError(res, 500, `Forget failed: ${err.message}`);
  }
}

async function handleResolve(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.keepId || !body.action) {
      sendError(res, 400, 'Missing fields: keepId, action');
      return;
    }

    conflict.resolve(body.keepId, body.action, body.supersededId);
    sendJson(res, 200, { resolved: true });
  } catch (err) {
    if (err.message === 'Request body too large') {
      sendError(res, 413, 'Request body too large (max 10KB)');
      return;
    }
    log('error', 'resolve failed', { error: err.message });
    sendError(res, 500, `Resolve failed: ${err.message}`);
  }
}

async function handleShutdown(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  sendJson(res, 200, { shutdown: true });
  log('info', 'Shutdown requested via HTTP');
  gracefulShutdown();
}

async function handleDump(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    const data = await db.dumpActive();
    sendJson(res, 200, data);
  } catch (err) {
    log('error', 'dump failed', { error: err.message });
    sendError(res, 500, `Dump failed: ${err.message}`);
  }
}

async function handleArchive(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.ids || !Array.isArray(body.ids) || (body.type !== 'memories' && body.type !== 'decisions')) {
      sendError(res, 400, 'Invalid request body');
      return;
    }

    const result = await db.archiveEntries(body.type, body.ids);
    sendJson(res, 200, { affected: result.affected });
  } catch (err) {
    if (err.message === 'Request body too large') {
      sendError(res, 413, 'Request body too large (max 10KB)');
      return;
    }
    log('error', 'archive failed', { error: err.message });
    sendError(res, 500, `Archive failed: ${err.message}`);
  }
}

// ─── Phase 0: Conductor route handlers ──────────────────────────────────────

async function handleLock(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.path || !body.agent) {
      sendError(res, 400, 'Missing required fields: path, agent');
      return;
    }

    cleanExpiredLocks();

    const ttl = body.ttl ?? 300;
    const now = new Date();
    const existingLock = locks.get(body.path);

    if (existingLock) {
      // Same agent can renew
      if (existingLock.agent === body.agent) {
        const newExpiresAt = new Date(now.getTime() + ttl * 1000).toISOString();
        existingLock.expiresAt = newExpiresAt;
        log('info', `Lock renewed: ${body.path} by ${body.agent}`);
        sendJson(res, 200, {
          locked: true,
          lockId: existingLock.id,
          expiresAt: newExpiresAt
        });
        return;
      }

      // Different agent gets 409
      const minutesLeft = Math.ceil((new Date(existingLock.expiresAt) - now) / 60000);
      sendJson(res, 409, {
        error: 'file_already_locked',
        holder: existingLock.agent,
        expiresAt: existingLock.expiresAt,
        minutesLeft
      });
      return;
    }

    // Create new lock
    const lock = {
      id: generateId(),
      path: body.path,
      agent: body.agent,
      lockedAt: now.toISOString(),
      expiresAt: new Date(now.getTime() + ttl * 1000).toISOString()
    };

    locks.set(body.path, lock);
    log('info', `Lock acquired: ${body.path} by ${body.agent}`);
    emitEvent('lock', { action: 'acquired', path: body.path, agent: body.agent });
    sendJson(res, 200, {
      locked: true,
      lockId: lock.id,
      expiresAt: lock.expiresAt
    });
  } catch (err) {
    if (err.message === 'Request body too large') {
      sendError(res, 413, 'Request body too large (max 10KB)');
      return;
    }
    log('error', 'lock failed', { error: err.message });
    sendError(res, 500, `Lock failed: ${err.message}`);
  }
}

async function handleUnlock(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.path || !body.agent) {
      sendError(res, 400, 'Missing required fields: path, agent');
      return;
    }

    cleanExpiredLocks();
    const lock = locks.get(body.path);

    if (!lock) {
      sendJson(res, 404, { error: 'no_lock_found' });
      return;
    }

    if (lock.agent !== body.agent) {
      sendJson(res, 403, { error: 'not_lock_holder', holder: lock.agent });
      return;
    }

    locks.delete(body.path);
    log('info', `Lock released: ${body.path} by ${body.agent}`);
    emitEvent('lock', { action: 'released', path: body.path, agent: body.agent });
    sendJson(res, 200, { unlocked: true });
  } catch (err) {
    if (err.message === 'Request body too large') {
      sendError(res, 413, 'Request body too large (max 10KB)');
      return;
    }
    log('error', 'unlock failed', { error: err.message });
    sendError(res, 500, `Unlock failed: ${err.message}`);
  }
}

async function handleLocks(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    cleanExpiredLocks();
    const activeLocks = Array.from(locks.values());

    sendJson(res, 200, { locks: activeLocks });
  } catch (err) {
    log('error', 'get locks failed', { error: err.message });
    sendError(res, 500, `Get locks failed: ${err.message}`);
  }
}

async function handleActivity(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.agent || !body.description) {
      sendError(res, 400, 'Missing required fields: agent, description');
      return;
    }

    cleanOldActivities();

    const activity = {
      id: generateId(),
      agent: body.agent,
      description: body.description,
      files: body.files || [],
      timestamp: new Date().toISOString()
    };

    activities.push(activity);
    log('info', `Activity recorded: ${body.agent} - ${body.description}`);
    sendJson(res, 200, { recorded: true, activityId: activity.id });
  } catch (err) {
    if (err.message === 'Request body too large') {
      sendError(res, 413, 'Request body too large (max 10KB)');
      return;
    }
    log('error', 'post activity failed', { error: err.message });
    sendError(res, 500, `Post activity failed: ${err.message}`);
  }
}

async function handleGetActivity(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    const params = parseQuery(req.url);
    const sinceStr = params.since || '1h';
    const sinceMs = parseDuration(sinceStr);
    const cutoff = new Date(Date.now() - sinceMs);

    const recentActivities = activities.filter(a => new Date(a.timestamp) >= cutoff);

    sendJson(res, 200, { activities: recentActivities });
  } catch (err) {
    log('error', 'get activity failed', { error: err.message });
    sendError(res, 500, `Get activity failed: ${err.message}`);
  }
}

async function handleMessage(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.from || !body.to || !body.message) {
      sendError(res, 400, 'Missing required fields: from, to, message');
      return;
    }

    const message = {
      id: generateId(),
      from: body.from,
      to: body.to,
      message: body.message,
      timestamp: new Date().toISOString()
    };

    cleanOldMessages(body.to);
    messages.push(message);
    log('info', `Message sent: ${body.from} -> ${body.to}: "${body.message.slice(0, 50)}..."`);
    sendJson(res, 200, { sent: true, messageId: message.id });
  } catch (err) {
    if (err.message === 'Request body too large') {
      sendError(res, 413, 'Request body too large (max 10KB)');
      return;
    }
    log('error', 'post message failed', { error: err.message });
    sendError(res, 500, `Post message failed: ${err.message}`);
  }
}

async function handleGetMessages(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    const params = parseQuery(req.url);
    const agent = params.agent;

    if (!agent) {
      sendError(res, 400, 'Missing parameter: agent');
      return;
    }

    const agentMessages = messages.filter(m => m.to === agent);

    sendJson(res, 200, { messages: agentMessages });
  } catch (err) {
    log('error', 'get messages failed', { error: err.message });
    sendError(res, 500, `Get messages failed: ${err.message}`);
  }
}

// ─── Session Bus handlers ─────────────────────────────────────────────���────

async function handleSessionStart(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.agent) {
      sendError(res, 400, 'Missing required field: agent');
      return;
    }

    const ttl = body.ttl ?? SESSION_TTL;
    const now = new Date();
    const session = {
      sessionId: generateId(),
      agent: body.agent,
      project: body.project || null,
      files: body.files || [],
      description: body.description || null,
      startedAt: now.toISOString(),
      lastHeartbeat: now.toISOString(),
      expiresAt: new Date(now.getTime() + ttl * 1000).toISOString()
    };

    sessions.set(body.agent, session);
    log('info', `Session started: ${body.agent} on ${body.project || 'unknown'}`);
    emitEvent('session', { action: 'started', agent: body.agent, project: body.project });
    sendJson(res, 200, {
      sessionId: session.sessionId,
      heartbeatInterval: 60
    });
  } catch (err) {
    if (err.message === 'Request body too large') {
      sendError(res, 413, 'Request body too large (max 10KB)');
      return;
    }
    log('error', 'session start failed', { error: err.message });
    sendError(res, 500, `Session start failed: ${err.message}`);
  }
}

async function handleSessionHeartbeat(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.agent || typeof body.agent !== 'string' || body.agent.trim().length === 0) {
      sendError(res, 400, 'Missing or invalid required field: agent');
      return;
    }

    // Basic agent format validation (reject clearly malicious patterns)
    const agent = body.agent.trim();
    if (agent.length > 100) {
      sendError(res, 400, 'Invalid agent: name too long (max 100 chars)');
      return;
    }
    if (!/^[a-zA-Z0-9_-]+$/.test(agent)) {
      sendError(res, 400, 'Invalid agent: name contains invalid characters (use alphanumeric, underscore, hyphen only)');
      return;
    }

    cleanExpiredSessions();
    const session = sessions.get(agent);

    if (!session) {
      sendJson(res, 404, { error: 'no_active_session' });
      return;
    }

    // Update session using validated agent string
    const now = new Date();
    session.lastHeartbeat = now.toISOString();
    session.expiresAt = new Date(now.getTime() + SESSION_TTL * 1000).toISOString();

    if (body.files !== undefined) session.files = body.files;
    if (body.description !== undefined) session.description = body.description;

    sendJson(res, 200, {
      renewed: true,
      expiresAt: session.expiresAt
    });
  } catch (err) {
    if (err.message === 'Request body too large') {
      sendError(res, 413, 'Request body too large (max 10KB)');
      return;
    }
    log('error', 'session heartbeat failed', { error: err.message });
    sendError(res, 500, `Session heartbeat failed: ${err.message}`);
  }
}

async function handleSessionEnd(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.agent) {
      sendError(res, 400, 'Missing required field: agent');
      return;
    }

    const existed = sessions.delete(body.agent);
    log('info', `Session ended: ${body.agent}`);
    emitEvent('session', { action: 'ended', agent: body.agent });
    sendJson(res, 200, { ended: true });
  } catch (err) {
    if (err.message === 'Request body too large') {
      sendError(res, 413, 'Request body too large (max 10KB)');
      return;
    }
    log('error', 'session end failed', { error: err.message });
    sendError(res, 500, `Session end failed: ${err.message}`);
  }
}

async function handleSessions(req, res) {
  if (!validateAuth(req)) {
    sendError(res, 401, 'Unauthorized');
    return;
  }

  try {
    cleanExpiredSessions();
    const activeSessions = Array.from(sessions.values());
    sendJson(res, 200, { sessions: activeSessions });
  } catch (err) {
    log('error', 'get sessions failed', { error: err.message });
    sendError(res, 500, `Get sessions failed: ${err.message}`);
  }
}

// ─── Task Board handlers ──────────────────────────────────────────────────

async function handleCreateTask(req, res) {
  if (!validateAuth(req)) { sendError(res, 401, 'Unauthorized'); return; }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.title) {
      sendError(res, 400, 'Missing required field: title');
      return;
    }

    cleanOldTasks();

    const task = {
      taskId: generateId(),
      title: body.title,
      description: body.description || null,
      project: body.project || null,
      files: body.files || [],
      priority: body.priority || 'medium',
      requiredCapability: body.requiredCapability || 'any',
      status: 'pending',
      claimedBy: null,
      createdAt: new Date().toISOString(),
      claimedAt: null,
      completedAt: null,
      summary: null
    };

    tasks.set(task.taskId, task);
    log('info', `Task created: [${task.priority}] ${task.title}`);
    emitEvent('task', { action: 'created', taskId: task.taskId, title: task.title, priority: task.priority });
    sendJson(res, 201, { taskId: task.taskId, status: 'pending' });
  } catch (err) {
    if (err.message === 'Request body too large') { sendError(res, 413, 'Request body too large (max 10KB)'); return; }
    log('error', 'create task failed', { error: err.message });
    sendError(res, 500, `Create task failed: ${err.message}`);
  }
}

async function handleGetTasks(req, res) {
  if (!validateAuth(req)) { sendError(res, 401, 'Unauthorized'); return; }

  try {
    const params = parseQuery(req.url);
    const statusFilter = params.status || 'pending';
    const projectFilter = params.project || null;

    let result = Array.from(tasks.values());

    if (statusFilter !== 'all') {
      result = result.filter(t => t.status === statusFilter);
    }
    if (projectFilter) {
      result = result.filter(t => t.project === projectFilter);
    }

    sendJson(res, 200, { tasks: result });
  } catch (err) {
    log('error', 'get tasks failed', { error: err.message });
    sendError(res, 500, `Get tasks failed: ${err.message}`);
  }
}

async function handleClaimTask(req, res) {
  if (!validateAuth(req)) { sendError(res, 401, 'Unauthorized'); return; }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.taskId || !body.agent) {
      sendError(res, 400, 'Missing required fields: taskId, agent');
      return;
    }

    const task = tasks.get(body.taskId);
    if (!task) {
      sendJson(res, 404, { error: 'task_not_found' });
      return;
    }

    if (task.status === 'claimed') {
      sendJson(res, 409, { error: 'task_already_claimed', claimedBy: task.claimedBy });
      return;
    }

    if (task.status === 'completed') {
      sendJson(res, 409, { error: 'task_already_completed' });
      return;
    }

    task.status = 'claimed';
    task.claimedBy = body.agent;
    task.claimedAt = new Date().toISOString();

    log('info', `Task claimed: "${task.title}" by ${body.agent}`);
    emitEvent('task', { action: 'claimed', taskId: task.taskId, title: task.title, agent: body.agent });
    sendJson(res, 200, { claimed: true, taskId: task.taskId });
  } catch (err) {
    if (err.message === 'Request body too large') { sendError(res, 413, 'Request body too large (max 10KB)'); return; }
    log('error', 'claim task failed', { error: err.message });
    sendError(res, 500, `Claim task failed: ${err.message}`);
  }
}

async function handleCompleteTask(req, res) {
  if (!validateAuth(req)) { sendError(res, 401, 'Unauthorized'); return; }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.taskId || !body.agent) {
      sendError(res, 400, 'Missing required fields: taskId, agent');
      return;
    }

    const task = tasks.get(body.taskId);
    if (!task) {
      sendJson(res, 404, { error: 'task_not_found' });
      return;
    }

    if (task.claimedBy !== body.agent) {
      sendJson(res, 403, { error: 'not_task_holder', claimedBy: task.claimedBy });
      return;
    }

    task.status = 'completed';
    task.completedAt = new Date().toISOString();
    task.summary = body.summary || null;

    log('info', `Task completed: "${task.title}" by ${body.agent}`);
    emitEvent('task', { action: 'completed', taskId: task.taskId, title: task.title, agent: body.agent });

    // Auto-post to feed (dedupe by taskId)
    const alreadyPosted = feed.some(e => e.taskId === task.taskId && e.kind === 'task_complete');
    if (!alreadyPosted) {
      autoPostFeed({
        agent: body.agent,
        kind: 'task_complete',
        summary: `Completed: ${task.title}`,
        content: task.summary || null,
        taskId: task.taskId,
        files: task.files || []
      });
    }

    sendJson(res, 200, { completed: true, taskId: task.taskId });
  } catch (err) {
    if (err.message === 'Request body too large') { sendError(res, 413, 'Request body too large (max 10KB)'); return; }
    log('error', 'complete task failed', { error: err.message });
    sendError(res, 500, `Complete task failed: ${err.message}`);
  }
}

async function handleAbandonTask(req, res) {
  if (!validateAuth(req)) { sendError(res, 401, 'Unauthorized'); return; }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.taskId || !body.agent) {
      sendError(res, 400, 'Missing required fields: taskId, agent');
      return;
    }

    const task = tasks.get(body.taskId);
    if (!task) {
      sendJson(res, 404, { error: 'task_not_found' });
      return;
    }

    if (task.claimedBy !== body.agent) {
      sendJson(res, 403, { error: 'not_task_holder', claimedBy: task.claimedBy });
      return;
    }

    task.status = 'pending';
    task.claimedBy = null;
    task.claimedAt = null;

    log('info', `Task abandoned: "${task.title}" by ${body.agent}`);
    emitEvent('task', { action: 'abandoned', taskId: task.taskId, title: task.title, agent: body.agent });
    sendJson(res, 200, { abandoned: true, taskId: task.taskId, status: 'pending' });
  } catch (err) {
    if (err.message === 'Request body too large') { sendError(res, 413, 'Request body too large (max 10KB)'); return; }
    log('error', 'abandon task failed', { error: err.message });
    sendError(res, 500, `Abandon task failed: ${err.message}`);
  }
}

async function handleNextTask(req, res) {
  if (!validateAuth(req)) { sendError(res, 401, 'Unauthorized'); return; }

  try {
    const params = parseQuery(req.url);
    const agent = params.agent;
    const capability = params.capability || 'any';

    if (!agent) {
      sendError(res, 400, 'Missing parameter: agent');
      return;
    }

    const pending = Array.from(tasks.values())
      .filter(t => t.status === 'pending')
      .filter(t => capability === 'any' || t.requiredCapability === 'any' || t.requiredCapability === capability)
      .sort((a, b) => {
        const pa = PRIORITY_RANK[a.priority] || 0;
        const pb = PRIORITY_RANK[b.priority] || 0;
        if (pb !== pa) return pb - pa; // higher priority first
        return new Date(a.createdAt) - new Date(b.createdAt); // older first (FIFO)
      });

    sendJson(res, 200, { task: pending[0] || null });
  } catch (err) {
    log('error', 'get next task failed', { error: err.message });
    sendError(res, 500, `Get next task failed: ${err.message}`);
  }
}

// ─── SSE Event Stream handler ─────────────────────────────────────────────

async function handleEventStream(req, res) {
  res.writeHead(200, {
    'Content-Type': 'text/event-stream',
    'Cache-Control': 'no-cache',
    'Connection': 'keep-alive',
    'Access-Control-Allow-Origin': '*',
  });

  // Send initial connected event
  res.write(`event: connected\ndata: ${JSON.stringify({ timestamp: new Date().toISOString(), clients: sseClients.size + 1 })}\n\n`);

  sseClients.add(res);
  log('info', `SSE client connected (${sseClients.size} total)`);

  // Keep-alive ping every 30s
  const keepAlive = setInterval(() => {
    try { res.write(': keepalive\n\n'); } catch { clearInterval(keepAlive); }
  }, 30000);

  req.on('close', () => {
    sseClients.delete(res);
    clearInterval(keepAlive);
    log('info', `SSE client disconnected (${sseClients.size} remaining)`);
  });
}

// ─── Shared Feed handlers ─────────────────────────────────────────────────

async function handlePostFeed(req, res) {
  if (!validateAuth(req)) { sendError(res, 401, 'Unauthorized'); return; }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.agent || !body.kind || !body.summary) {
      sendError(res, 400, 'Missing required fields: agent, kind, summary');
      return;
    }

    const entry = autoPostFeed({
      agent: body.agent,
      kind: body.kind,
      summary: body.summary,
      content: body.content || null,
      files: body.files || [],
      taskId: body.taskId || null,
      traceId: body.traceId || null,
      priority: body.priority || 'normal'
    });

    log('info', `Feed: [${entry.kind}] ${entry.agent}: ${entry.summary.slice(0, 60)}`);
    sendJson(res, 201, { feedId: entry.id, recorded: true });
  } catch (err) {
    if (err.message === 'Request body too large') { sendError(res, 413, 'Request body too large (max 10KB)'); return; }
    log('error', 'post feed failed', { error: err.message });
    sendError(res, 500, `Post feed failed: ${err.message}`);
  }
}

async function handleGetFeed(req, res) {
  if (!validateAuth(req)) { sendError(res, 401, 'Unauthorized'); return; }

  try {
    cleanOldFeed();
    const params = parseQuery(req.url);
    const sinceStr = params.since || '1h';
    const sinceMs = parseDuration(sinceStr);
    const cutoff = new Date(Date.now() - sinceMs);
    const kindFilter = params.kind || null;
    const agentFilter = params.agent || null;
    const unread = params.unread === 'true';

    let entries;
    if (unread && agentFilter) {
      entries = getUnreadFeed(agentFilter);
    } else {
      entries = feed.filter(e => new Date(e.timestamp) >= cutoff);
    }

    if (kindFilter) entries = entries.filter(e => e.kind === kindFilter);
    if (agentFilter && !unread) entries = entries.filter(e => e.agent !== agentFilter || true);

    // Return summary only, strip content for list view
    const slim = entries.map(({ content, ...rest }) => rest);

    sendJson(res, 200, { entries: slim });
  } catch (err) {
    log('error', 'get feed failed', { error: err.message });
    sendError(res, 500, `Get feed failed: ${err.message}`);
  }
}

async function handleGetFeedById(req, res, feedId) {
  if (!validateAuth(req)) { sendError(res, 401, 'Unauthorized'); return; }

  const entry = feed.find(e => e.id === feedId);
  if (!entry) {
    sendJson(res, 404, { error: 'feed_entry_not_found' });
    return;
  }

  sendJson(res, 200, entry);
}

async function handleFeedAck(req, res) {
  if (!validateAuth(req)) { sendError(res, 401, 'Unauthorized'); return; }

  try {
    const raw = await readBody(req);
    const body = JSON.parse(raw);

    if (!body.agent || !body.lastSeenId) {
      sendError(res, 400, 'Missing required fields: agent, lastSeenId');
      return;
    }

    feedAcks.set(body.agent, body.lastSeenId);
    log('info', `Feed ack: ${body.agent} seen up to ${body.lastSeenId}`);
    sendJson(res, 200, { acked: true });
  } catch (err) {
    if (err.message === 'Request body too large') { sendError(res, 413, 'Request body too large (max 10KB)'); return; }
    log('error', 'feed ack failed', { error: err.message });
    sendError(res, 500, `Feed ack failed: ${err.message}`);
  }
}

async function handleSavings(req, res) {
  try {
    const allSavings = db.query("SELECT data, created_at FROM events WHERE type = 'boot_savings' ORDER BY created_at ASC");
    const points = allSavings.map(row => {
      const d = JSON.parse(row.data);
      return {
        timestamp: row.created_at,
        agent: d.agent || 'unknown',
        served: d.served || 0,
        baseline: d.baseline || 0,
        saved: d.saved || 0,
        percent: d.percent || 0,
      };
    });

    // Aggregates
    const totalSaved = points.reduce((sum, p) => sum + p.saved, 0);
    const totalServed = points.reduce((sum, p) => sum + p.served, 0);
    const totalBaseline = points.reduce((sum, p) => sum + p.baseline, 0);
    const avgPercent = points.length > 0 ? Math.round(points.reduce((sum, p) => sum + p.percent, 0) / points.length) : 0;

    // Daily aggregation for charts
    const daily = {};
    for (const p of points) {
      const day = (p.timestamp || '').slice(0, 10);
      if (!day) continue;
      if (!daily[day]) daily[day] = { saved: 0, served: 0, boots: 0 };
      daily[day].saved += p.saved;
      daily[day].served += p.served;
      daily[day].boots += 1;
    }

    sendJson(res, 200, {
      summary: { totalSaved, totalServed, totalBaseline, avgPercent, totalBoots: points.length },
      daily: Object.entries(daily).map(([date, d]) => ({ date, ...d })),
      recent: points.slice(-20),
    });
  } catch (err) {
    sendError(res, 500, `Savings query failed: ${err.message}`);
  }
}

// ─── HTTP router ───────────────────────────────────────────────────────────

const ROUTES = {
  'GET /boot': handleBoot,
  'GET /recall': handleRecall,
  'GET /peek': handlePeek,
  'GET /recall/budget': handleBudgetRecall,
  'POST /store': handleStore,
  'POST /diary': handleDiary,
  'GET /health': handleHealth,
  'GET /digest': handleDigest,
  'GET /savings': handleSavings,
  'POST /forget': handleForget,
  'POST /resolve': handleResolve,
  'POST /shutdown': handleShutdown,
  'GET /dump': handleDump, // New endpoint
  'POST /archive': handleArchive, // New endpoint

  // MCP-over-HTTP transport (Streamable HTTP)
  'POST /mcp': handleMcpHttp,
  'GET /mcp': handleMcpSse,

  // Phase 0: Conductor endpoints
  'POST /lock': handleLock,
  'POST /unlock': handleUnlock,
  'GET /locks': handleLocks,
  'POST /activity': handleActivity,
  'GET /activity': handleGetActivity,
  'POST /message': handleMessage,
  'GET /messages': handleGetMessages,

  // Session Bus endpoints
  'POST /session/start': handleSessionStart,
  'POST /session/heartbeat': handleSessionHeartbeat,
  'POST /session/end': handleSessionEnd,
  'GET /sessions': handleSessions,

  // Task Board endpoints
  'POST /tasks': handleCreateTask,
  'GET /tasks': handleGetTasks,
  'POST /tasks/claim': handleClaimTask,
  'POST /tasks/complete': handleCompleteTask,
  'POST /tasks/abandon': handleAbandonTask,
  'GET /tasks/next': handleNextTask,

  // Shared Feed endpoints
  'POST /feed': handlePostFeed,
  'GET /feed': handleGetFeed,
  'POST /feed/ack': handleFeedAck,

  // SSE Event Stream
  'GET /events/stream': handleEventStream,
};

async function handleRequest(req, res) {
  // Host validation on ALL requests
  if (!validateHost(req)) {
    sendError(res, 403, 'Forbidden: invalid host');
    return;
  }

  // Handle CORS preflight — allow Tauri desktop app + localhost tools
  if (req.method === 'OPTIONS') {
    res.writeHead(204, {
      'Access-Control-Allow-Origin': '*',
      'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
      'Access-Control-Allow-Headers': 'Authorization, Content-Type',
      'Access-Control-Max-Age': '86400',
    });
    res.end();
    return;
  }

  const pathname = getPathname(req.url);

  // Dynamic route: GET /feed/:id
  const feedMatch = pathname.match(/^\/feed\/([a-f0-9-]+)$/);
  if (req.method === 'GET' && feedMatch) {
    try { await handleGetFeedById(req, res, feedMatch[1]); } catch (err) {
      log('error', 'Unhandled route error', { route: `GET /feed/:id`, error: err.message });
      if (!res.headersSent) sendError(res, 500, 'Internal server error');
    }
    return;
  }

  const routeKey = `${req.method} ${pathname}`;
  const handler = ROUTES[routeKey];

  if (!handler) {
    sendError(res, 404, `Not found: ${req.method} ${pathname}`);
    return;
  }

  try {
    await handler(req, res);
  } catch (err) {
    log('error', 'Unhandled route error', { route: routeKey, error: err.message });
    if (!res.headersSent) {
      sendError(res, 500, 'Internal server error');
    }
  }
}

// ─── HTTP server ───────────────────────────────────────────────────────────

function startHttpServer() {
  return new Promise((resolve, reject) => {
    httpServer = http.createServer(handleRequest);

    httpServer.on('error', (err) => {
      if (err.code === 'EADDRINUSE') {
        log('error', `Port ${PORT} already in use`);
        reject(new Error(`Port ${PORT} already in use — is another Cortex running?`));
      } else {
        log('error', 'HTTP server error', { error: err.message });
        reject(err);
      }
    });

    httpServer.listen(PORT, '127.0.0.1', () => {
      log('info', `HTTP server listening on 127.0.0.1:${PORT}`);
      resolve();
    });
  });
}

// ─── Graceful shutdown ─────────────────────────────────────────────────────

function gracefulShutdown() {
  if (shuttingDown) return;
  shuttingDown = true;

  log('info', 'Graceful shutdown initiated');

  // Clear keep-alive interval
  if (keepAliveInterval) {
    clearInterval(keepAliveInterval);
    keepAliveInterval = null;
  }

  // Close HTTP server (stop accepting new connections)
  if (httpServer) {
    httpServer.close(() => {
      log('info', 'HTTP server closed');
    });
  }

  // Flush brain state
  try {
    db.close();
    log('info', 'Database closed');
  } catch (err) {
    log('error', 'Error closing database', { error: err.message });
  }

  // Remove PID file
  removePid();

  // Close log stream
  if (logStream) {
    logStream.end(() => {
      process.exit(0);
    });
    // Force exit after 3s if log stream hangs
    setTimeout(() => process.exit(0), 3000).unref();
  } else {
    process.exit(0);
  }
}

// ─── Signal handlers ───────────────────────────────────────────────────────

process.on('SIGTERM', () => {
  log('info', 'Received SIGTERM');
  gracefulShutdown();
});

process.on('SIGINT', () => {
  log('info', 'Received SIGINT');
  gracefulShutdown();
});

process.on('uncaughtException', (err) => {
  log('error', 'Uncaught exception', { error: err.message, stack: err.stack });
  gracefulShutdown();
});

process.on('unhandledRejection', (reason) => {
  const msg = reason instanceof Error ? reason.message : String(reason);
  log('error', 'Unhandled rejection', { reason: msg });
});

// ═══════════════════════════════════════════════════════════════════════════
// MCP JSON-RPC 2.0 Transport
// ═══════════════════════════════════════════════════════════════════════════

// ─── MCP tool definitions ──────────────────────────────────────────────────

const MCP_TOOLS = [
  {
    name: 'cortex_boot',
    description: 'Get compiled boot prompt with session context. Uses capsule system: identity (stable) + delta (what changed since your last boot). Call once at session start.',
    inputSchema: {
      type: 'object',
      properties: {
        profile: { type: 'string', description: 'Legacy profile name. Ignored when agent is set.' },
        agent: { type: 'string', description: 'Your agent ID (e.g. claude-opus, gemini, codex). Enables delta tracking.' },
        budget: { type: 'number', description: 'Max token budget for boot prompt (default: 600)' },
      },
    },
  },
  {
    name: 'cortex_recall',
    description: 'Search Cortex brain for memories and decisions. Adapts detail level to your token budget:\n- budget=0: headlines only (source + relevance, no excerpts) ~30 tokens\n- budget=200: top result detailed, rest compressed ~200 tokens\n- budget=500: all results with full excerpts ~500 tokens\n- Default: 200 tokens. ONE call gets everything you need. No need to call peek first.',
    inputSchema: {
      type: 'object',
      properties: {
        query: { type: 'string', description: 'Search query text' },
        budget: { type: 'number', description: 'Token budget. 0=headlines only, 200=balanced (default), 500+=full detail' },
      },
      required: ['query'],
    },
  },
  {
    name: 'cortex_store',
    description: 'Store a decision or insight with conflict detection and dedup.',
    inputSchema: {
      type: 'object',
      properties: {
        decision: { type: 'string', description: 'The decision or insight text' },
        context: { type: 'string', description: 'Optional context about where/why' },
        type: { type: 'string', description: 'Entry type (default: decision)' },
        source_agent: { type: 'string', description: 'Agent that produced this' },
        confidence: { type: 'number', description: 'Confidence score 0-1 (default: 0.8)' },
      },
      required: ['decision'],
    },
  },
  {
    name: 'cortex_diary',
    description: 'Write session state to state.md for cross-session continuity.',
    inputSchema: {
      type: 'object',
      properties: {
        accomplished: { type: 'string', description: 'What was done this session' },
        nextSteps: { type: 'string', description: 'What to do next session' },
        decisions: { type: 'string', description: 'Key decisions made' },
        pending: { type: 'string', description: 'Pending items' },
        knownIssues: { type: 'string', description: 'Known issues' },
      },
    },
  },
  {
    name: 'cortex_health',
    description: 'Check Cortex system health: DB stats, Ollama status, memory counts.',
    inputSchema: {
      type: 'object',
      properties: {},
    },
  },
  {
    name: 'cortex_digest',
    description: 'Daily health digest: memory counts, today\'s activity, top recalls, decay stats, agent boots. Use to check if the brain is compounding.',
    inputSchema: {
      type: 'object',
      properties: {},
    },
  },
  {
    name: 'cortex_forget',
    description: 'Decay matching memories/decisions by keyword (multiply score by 0.3).',
    inputSchema: {
      type: 'object',
      properties: {
        source: { type: 'string', description: 'Keyword to match for decay' },
      },
      required: ['source'],
    },
  },
  {
    name: 'cortex_resolve',
    description: 'Resolve a disputed decision pair.',
    inputSchema: {
      type: 'object',
      properties: {
        keepId: { type: 'number', description: 'ID of the decision to keep' },
        action: { type: 'string', enum: ['keep', 'merge'], description: 'Resolution action' },
        supersededId: { type: 'number', description: 'ID of the decision to supersede (for keep action)' },
      },
      required: ['keepId', 'action'],
    },
  },
];

// ─── MCP tool dispatch ─────────────────────────────────────────────────────

async function mcpDispatch(toolName, args) {
  switch (toolName) {
    case 'cortex_boot': {
      const agent = args.source_agent || args.agent || 'mcp';
      if (agent && agent !== 'unknown') {
        return compiler.compileCapsules(agent, parseInt(args.budget, 10) || 600);
      }
      return compiler.compile(args.profile || 'full');
    }

    case 'cortex_peek':
    case 'cortex_recall_budget':
    case 'cortex_recall': {
      if (!args.query) throw new Error('Missing required argument: query');
      const agent = args.source_agent || 'mcp';
      const budget = typeof args.budget === 'number' ? args.budget : 200;

      // Check predictive cache first
      const cached = getPreCached(agent, args.query);
      if (cached) {
        return { results: cached, budget, spent: 0, cached: true };
      }

      if (budget === 0) {
        // Headlines only — like peek
        const raw = await brain.recall(args.query, 10);
        recordRecallPattern(agent, args.query);
        const headlines = raw.map(r => ({ source: r.source, relevance: r.relevance, method: r.method }));
        return { count: headlines.length, results: headlines, budget: 0, spent: 0, mode: 'headlines' };
      }

      // Budget-aware recall
      const results = await brain.budgetRecall(args.query, budget, 10);
      recordRecallPattern(agent, args.query);

      // Mark served for dedup
      for (const r of results) markServed(agent, r.excerpt);

      const spent = results.reduce((sum, r) => sum + (r.tokens || 0), 0);
      return { results, budget, spent, saved: budget - spent, mode: budget >= 500 ? 'full' : 'balanced' };
    }

    case 'cortex_store': {
      if (!args.decision) throw new Error('Missing required argument: decision');
      const result = await brain.store(args.decision, {
        context: args.context || null,
        type: args.type || 'decision',
        source_agent: args.source_agent || 'mcp',
        confidence: args.confidence ?? 0.8,
      });
      return { stored: result.stored, entry: result };
    }

    case 'cortex_diary': {
      const result = brain.writeDiary({
        accomplished: args.accomplished || null,
        nextSteps: args.nextSteps || null,
        decisions: args.decisions || null,
        pending: args.pending || null,
        knownIssues: args.knownIssues || null,
      });
      return { written: result.written };
    }

    case 'cortex_health': {
      const stats = await brain.getStats();
      return { stats, overall: stats.ollama === 'connected' ? 'healthy' : 'degraded' };
    }

    case 'cortex_digest': {
      return brain.getDigest();
    }

    case 'cortex_forget': {
      if (!args.source) throw new Error('Missing required argument: source');
      const result = brain.forget(args.source);
      return { affected: result.affected };
    }

    case 'cortex_resolve': {
      if (args.keepId == null || !args.action) {
        throw new Error('Missing required arguments: keepId, action');
      }
      conflict.resolve(args.keepId, args.action, args.supersededId);
      return { resolved: true };
    }

    default:
      throw new Error(`Unknown tool: ${toolName}`);
  }
}

// ─── MCP envelope helpers ──────────────────────────────────────────────────

function mcpSuccess(id, result) {
  return {
    jsonrpc: '2.0',
    id,
    result,
  };
}

function mcpError(id, code, message) {
  return {
    jsonrpc: '2.0',
    id,
    error: { code, message },
  };
}

function wrapMcpToolResult(data) {
  mcpCalls++;
  return {
    content: [
      {
        type: 'text',
        text: JSON.stringify({
          ...data,
          _liveness: true,
          _ts: new Date().toISOString(),
          _calls: mcpCalls,
        }),
      },
    ],
  };
}

// ─── MCP message handler ──────────────────────────────────────────────────

async function handleMcpMessage(msg) {
  const { id, method, params } = msg;

  switch (method) {
    case 'initialize':
      return mcpSuccess(id, {
        protocolVersion: '2024-11-05',
        capabilities: { tools: { listChanged: true } },
        serverInfo: { name: 'cortex', version: '2.0.0' },
      });

    case 'notifications/initialized':
      // Client acknowledgment — no response needed
      return null;

    case 'tools/list':
      return mcpSuccess(id, { tools: MCP_TOOLS });

    case 'tools/call': {
      const toolName = params?.name;
      const toolArgs = params?.arguments || {};

      if (!toolName) {
        return mcpError(id, -32602, 'Missing tool name');
      }

      const knownTool = MCP_TOOLS.find((t) => t.name === toolName);
      if (!knownTool) {
        return mcpError(id, -32601, `Unknown tool: ${toolName}`);
      }

      try {
        const result = await mcpDispatch(toolName, toolArgs);
        return mcpSuccess(id, wrapMcpToolResult(result));
      } catch (err) {
        log('error', `MCP tool error: ${toolName}`, { error: err.message });
        return mcpSuccess(id, {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                error: err.message,
                _liveness: true,
                _ts: new Date().toISOString(),
                _calls: ++mcpCalls,
              }),
            },
          ],
          isError: true,
        });
      }
    }

    default:
      // Unknown methods get a method-not-found error if they have an id
      if (id != null) {
        return mcpError(id, -32601, `Method not found: ${method}`);
      }
      // Notifications (no id) are silently ignored per JSON-RPC spec
      return null;
  }
}

// ─── MCP-over-HTTP transport (Streamable HTTP) ─────────────────────────────

const mcpSessions = new Map(); // sessionId -> { createdAt, lastActivity }

async function handleMcpHttp(req, res) {
  // Security: validate Origin header to prevent DNS rebinding
  const origin = req.headers['origin'] || '';
  if (origin && !['http://localhost', 'http://127.0.0.1', ''].includes(origin)) {
    sendError(res, 403, 'Forbidden: invalid origin');
    return;
  }

  // Check for MCP-Protocol-Version header
  const protocolVersion = req.headers['mcp-protocol-version'] || '2024-11-05';

  try {
    const raw = await readBody(req);
    let msg;
    try {
      msg = JSON.parse(raw);
    } catch {
      res.writeHead(400, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ jsonrpc: '2.0', error: { code: -32700, message: 'Parse error' }, id: null }));
      return;
    }

    // Validate JSON-RPC
    if (!msg.jsonrpc || msg.jsonrpc !== '2.0') {
      res.writeHead(400, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ jsonrpc: '2.0', error: { code: -32600, message: 'Invalid JSON-RPC version' }, id: msg.id || null }));
      return;
    }

    // Handle session management
    let sessionId = req.headers['mcp-session-id'];
    if (msg.method === 'initialize') {
      // Create new session on initialize
      sessionId = crypto.randomBytes(16).toString('hex');
      mcpSessions.set(sessionId, { createdAt: Date.now(), lastActivity: Date.now() });
    } else if (!sessionId) {
      // Require session for non-initialize requests
      res.writeHead(400, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ jsonrpc: '2.0', error: { code: -32600, message: 'Missing Mcp-Session-Id header' }, id: msg.id || null }));
      return;
    }

    // Update session activity
    if (sessionId && mcpSessions.has(sessionId)) {
      const session = mcpSessions.get(sessionId);
      session.lastActivity = Date.now();
    }

    // Process the MCP message
    const response = await handleMcpMessage(msg);

    // For notifications (no id), return 202 Accepted
    if (msg.id == null || response === null) {
      res.writeHead(202);
      res.end();
      return;
    }

    // For requests, return the response with session header
    const headers = {
      'Content-Type': 'application/json',
      'MCP-Protocol-Version': protocolVersion,
    };
    if (sessionId) {
      headers['Mcp-Session-Id'] = sessionId;
    }

    res.writeHead(200, headers);
    res.end(JSON.stringify(response));
  } catch (err) {
    log('error', 'MCP HTTP error', { error: err.message });
    res.writeHead(500, { 'Content-Type': 'application/json' });
    res.end(JSON.stringify({ jsonrpc: '2.0', error: { code: -32603, message: `Internal error: ${err.message}` }, id: null }));
  }
}

async function handleMcpSse(req, res) {
  // GET /mcp opens SSE stream for server-to-client messages
  // For now, we return 405 as we don't have push notifications
  // This is valid per MCP spec
  res.writeHead(405, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify({ error: 'SSE streaming not implemented. Use POST for requests.' }));
}

// ─── MCP stdio transport ──────────────────────────────────────────────────

function startMcpTransport(stdinStream, stdoutWrite) {
  // On Windows, ensure stdin is readable
  if (process.platform === 'win32') {
    // Force stdin to be readable if it's not already
    if (!stdinStream.readable) {
      stdinStream.resume();
    }
  }
  
  const rl = readline.createInterface({
    input: stdinStream,
    terminal: false,
  });

  function send(obj) {
    if (obj === null) return;
    // Use the provided write function directly (bypasses log redirection)
    stdoutWrite(JSON.stringify(obj) + '\n');
  }

  rl.on('line', async (line) => {
    if (!line.trim()) return;

    let msg;
    try {
      msg = JSON.parse(line);
    } catch {
      send(mcpError(null, -32700, 'Parse error'));
      return;
    }

    if (!msg.jsonrpc || msg.jsonrpc !== '2.0') {
      send(mcpError(msg.id || null, -32600, 'Invalid JSON-RPC version'));
      return;
    }

    try {
      const response = await handleMcpMessage(msg);
      send(response);
    } catch (err) {
      log('error', 'MCP handler error', { error: err.message });
      send(mcpError(msg.id || null, -32603, `Internal error: ${err.message}`));
    }
  });

  rl.on('close', () => {
    log('info', 'MCP stdin closed');
    gracefulShutdown();
  });

  log('info', 'MCP stdio transport started');
}

// ═══════════════════════════════════════════════════════════════════════════
// Entry point
// ═══════════════════════════════════════════════════════════════════════════

async function main() {
  const mode = process.argv[2];

  if (!mode || (mode !== 'serve' && mode !== 'mcp')) {
    process.stderr.write(
      'Usage: node src/daemon.js <serve|mcp>\n' +
      '  serve — HTTP daemon only (standalone)\n' +
      '  mcp   — MCP stdio + HTTP daemon (for Claude Code)\n'
    );
    process.exit(1);
  }

  if (mode === 'mcp') {
    // ── MCP mode ─────────────────────────────────────────────────────
    // Capture original stdin/stdout write functions BEFORE redirecting
    const origStdin = process.stdin;
    const origStdout = process.stdout;
    const origStdoutWrite = process.stdout.write.bind(process.stdout);
    const origStderrWrite = process.stderr.write.bind(process.stderr);

    // Redirect stdout and stderr to log file
    openLogStream();

    // Replace process.stdout.write and process.stderr.write to go to log
    // This prevents brain.js console.error from corrupting the JSON-RPC stream
    process.stdout.write = (chunk, encoding, callback) => {
      if (logStream) {
        try {
          const result = logStream.write(chunk, encoding, callback);
          return result;
        } catch (streamErr) {
          // Fallback to original stdout if log stream fails
          return origStdoutWrite(chunk, encoding, callback);
        }
      }
      return origStdoutWrite(chunk, encoding, callback);
    };

    process.stderr.write = (chunk, encoding, callback) => {
      if (logStream) {
        try {
          const result = logStream.write(chunk, encoding, callback);
          return result;
        } catch (streamErr) {
          // Fallback to original stderr if log stream fails
          return origStderrWrite(chunk, encoding, callback);
        }
      }
      return origStderrWrite(chunk, encoding, callback);
    };

    log('info', '=== Cortex v2.0.0 starting (MCP mode) ===');

    // Kill stale daemon if one exists
    killStaleDaemon();

    // Generate auth token + PID
    generateToken();
    writePid();

    // Initialize brain (async — DB, indexing, embeddings)
    try {
      await brain.init();
      log('info', 'Brain initialized');
    } catch (err) {
      log('error', 'Brain init failed', { error: err.message, stack: err.stack });
      process.exit(1);
    }

    // Start HTTP server (skip if port already in use — serve mode may be running)
    try {
      await startHttpServer();
    } catch (err) {
      if (err.code === 'EADDRINUSE') {
        log('info', `Port ${PORT} already in use — HTTP serve mode likely running. MCP will use stdio only.`);
      } else {
        log('error', 'HTTP server failed to start', { error: err.message });
        process.exit(1);
      }
    }

    // Start MCP transport with original write function (not patched stdout)
    startMcpTransport(origStdin, origStdoutWrite);

  } else {
    // ── Serve mode ───────────────────────────────────────────────────
    openLogStream();

    // In serve mode, also log to stderr for visibility
    const origStderrWrite = process.stderr.write.bind(process.stderr);
    const patchedStderr = (chunk, encoding, callback) => {
      if (logStream) logStream.write(chunk, encoding);
      return origStderrWrite(chunk, encoding, callback);
    };
    process.stderr.write = patchedStderr;

    log('info', '=== Cortex v2.0.0 starting (serve mode) ===');
    process.stderr.write(`[cortex] Starting Cortex v2.0.0 on port ${PORT}...\n`);

    // Kill stale daemon if one exists
    killStaleDaemon();

    // Generate auth token + PID
    generateToken();
    writePid();

    // Initialize brain
    try {
      await brain.init();
      log('info', 'Brain initialized');
    } catch (err) {
      log('error', 'Brain init failed', { error: err.message, stack: err.stack });
      process.exit(1);
    }

    // Start HTTP server
    try {
      await startHttpServer();
      process.stderr.write(`[cortex] Listening on http://127.0.0.1:${PORT}\n`);
      process.stderr.write(`[cortex] Auth token at ${TOKEN_PATH}\n`);
      process.stderr.write(`[cortex] PID ${process.pid} written to ${PID_PATH}\n`);

      // Keep process alive - HTTP server should do this but ensure it
      keepAliveInterval = setInterval(() => {}, 24 * 60 * 60 * 1000); // 24hr interval (minimal CPU)
    } catch (err) {
      process.stderr.write(`[cortex] FATAL: ${err.message}\n`);
      process.exit(1);
    }
  }
}

// Expose Phase 0 state for compiler (boot injection)
module.exports.activeLocks = locks;
module.exports.activities = activities;
module.exports.messages = messages;
module.exports.sessions = sessions;
module.exports.tasks = tasks;
module.exports.feed = feed;
module.exports.feedAcks = feedAcks;
module.exports.cleanExpiredLocks = cleanExpiredLocks;
module.exports.cleanExpiredSessions = cleanExpiredSessions;

main().catch((err) => {
  const msg = `[cortex] Fatal startup error: ${err.message}\n`;
  if (logStream) {
    logStream.write(msg);
  }
  process.stderr.write(msg);
  process.exit(1);
});
