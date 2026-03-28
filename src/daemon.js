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

  if (logStream) {
    logStream.write(entry + '\n');
  } else {
    // Fallback before log stream is open (serve mode only)
    process.stderr.write(entry + '\n');
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

// ─── HTTP helpers ──────────────────────────────────────────────────────────

function readBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let size = 0;

    req.on('data', (chunk) => {
      size += chunk.length;
      if (size > MAX_BODY) {
        req.destroy();
        reject(new Error('Request body too large'));
        return;
      }
      chunks.push(chunk);
    });

    req.on('end', () => resolve(Buffer.concat(chunks).toString('utf-8')));
    req.on('error', reject);
  });
}

function sendJson(res, statusCode, data) {
  const body = JSON.stringify(data);
  res.writeHead(statusCode, {
    'Content-Type': 'application/json',
    'Content-Length': Buffer.byteLength(body),
    'Cache-Control': 'no-store',
    // No CORS headers — deny all cross-origin
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
    // Use capsule compiler when agent is identified, legacy otherwise
    const result = (agent && agent !== 'unknown')
      ? compiler.compileCapsules(agent, parseInt(params.budget, 10) || 600)
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
  const k = parseInt(params.k, 10) || 7;

  if (!q) {
    sendError(res, 400, 'Missing query parameter: q');
    return;
  }

  try {
    const results = await brain.recall(q, k);
    sendJson(res, 200, { results });
  } catch (err) {
    log('error', 'recall failed', { error: err.message });
    sendError(res, 500, `Recall failed: ${err.message}`);
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

// ─── HTTP router ───────────────────────────────────────────────────────────

const ROUTES = {
  'GET /boot': handleBoot,
  'GET /recall': handleRecall,
  'POST /store': handleStore,
  'POST /diary': handleDiary,
  'GET /health': handleHealth,
  'GET /digest': handleDigest,
  'POST /forget': handleForget,
  'POST /resolve': handleResolve,
  'POST /shutdown': handleShutdown,
  'GET /dump': handleDump, // New endpoint
  'POST /archive': handleArchive, // New endpoint
};

async function handleRequest(req, res) {
  // Host validation on ALL requests
  if (!validateHost(req)) {
    sendError(res, 403, 'Forbidden: invalid host');
    return;
  }

  // Handle CORS preflight — deny all
  if (req.method === 'OPTIONS') {
    res.writeHead(204);
    res.end();
    return;
  }

  const pathname = getPathname(req.url);
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
    description: 'Hybrid semantic+keyword search across all Cortex memories and decisions.',
    inputSchema: {
      type: 'object',
      properties: {
        query: { type: 'string', description: 'Search query text' },
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

    case 'cortex_recall': {
      if (!args.query) throw new Error('Missing required argument: query');
      const results = await brain.recall(args.query);
      return { results };
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
        capabilities: { tools: {} },
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

// ─── MCP stdio transport ──────────────────────────────────────────────────

function startMcpTransport(stdinStream, stdoutStream) {
  const rl = readline.createInterface({
    input: stdinStream,
    terminal: false,
  });

  function send(obj) {
    if (obj === null) return;
    stdoutStream.write(JSON.stringify(obj) + '\n');
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
    // Capture original stdin/stdout BEFORE redirecting
    const origStdin = process.stdin;
    const origStdout = process.stdout;

    // Redirect stdout and stderr to log file
    openLogStream();

    // Replace process.stdout.write and process.stderr.write to go to log
    // This prevents brain.js console.error from corrupting the JSON-RPC stream
    const origStdoutWrite = process.stdout.write.bind(process.stdout);
    const origStderrWrite = process.stderr.write.bind(process.stderr);

    process.stdout.write = (chunk, encoding, callback) => {
      if (logStream) {
        return logStream.write(chunk, encoding, callback);
      }
      return origStdoutWrite(chunk, encoding, callback);
    };

    process.stderr.write = (chunk, encoding, callback) => {
      if (logStream) {
        return logStream.write(chunk, encoding, callback);
      }
      return origStderrWrite(chunk, encoding, callback);
    };

    log('info', '=== Cortex v2.0.0 starting (MCP mode) ===');

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

    // Start HTTP server
    try {
      await startHttpServer();
    } catch (err) {
      log('error', 'HTTP server failed to start', { error: err.message });
      process.exit(1);
    }

    // Start MCP transport on original stdin/stdout
    startMcpTransport(origStdin, origStdout);

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
    } catch (err) {
      process.stderr.write(`[cortex] FATAL: ${err.message}\n`);
      process.exit(1);
    }
  }
}

main().catch((err) => {
  const msg = `[cortex] Fatal startup error: ${err.message}\n`;
  if (logStream) {
    logStream.write(msg);
  }
  process.stderr.write(msg);
  process.exit(1);
});
