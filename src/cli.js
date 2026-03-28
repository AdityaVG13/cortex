#!/usr/bin/env node
'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');
const { spawn } = require('child_process');

// ─── Path Constants ────────────────────────────────────────────────────────

const CORTEX_DIR = path.join(process.env.USERPROFILE || process.env.HOME, '.cortex');
const DAEMON_SCRIPT = path.join(__dirname, 'daemon.js');
const PID_FILE = path.join(CORTEX_DIR, 'cortex.pid');
const TOKEN_FILE = path.join(CORTEX_DIR, 'cortex.token');
const LOG_FILE = path.join(CORTEX_DIR, 'cortex.log');

const DAEMON_HOST = '127.0.0.1';
const DAEMON_PORT = 7437;
const REQUEST_TIMEOUT_MS = 10_000;
const STARTUP_TIMEOUT_MS = 5_000;
const STARTUP_POLL_MS = 500;

// ─── Arg Parsing ───────────────────────────────────────────────────────────

function parseArgs(argv) {
  const command = argv[2] || 'help';
  const positional = [];
  const flags = {};

  for (let i = 3; i < argv.length; i++) {
    const arg = argv[i];
    if (arg.startsWith('--')) {
      const key = arg.slice(2);
      const next = argv[i + 1];
      if (next && !next.startsWith('--')) {
        flags[key] = next;
        i++;
      } else {
        flags[key] = true;
      }
    } else {
      positional.push(arg);
    }
  }

  return { command, positional, flags };
}

// ─── Auth Token ────────────────────────────────────────────────────────────

function readToken() {
  try {
    if (fs.existsSync(TOKEN_FILE)) {
      return fs.readFileSync(TOKEN_FILE, 'utf-8').trim();
    }
  } catch {
    // Token is optional — proceed without it
  }
  return null;
}

// ─── HTTP Client ───────────────────────────────────────────────────────────

/**
 * Send an HTTP request to the Cortex daemon.
 * @param {'GET'|'POST'} method
 * @param {string} urlPath  - e.g. '/health'
 * @param {object|null} body - JSON body for POST requests
 * @param {string} sourceAgent - value for X-Source-Agent header
 * @returns {Promise<object>} parsed JSON response
 */
function request(method, urlPath, body = null, sourceAgent = 'cli-manual') {
  return new Promise((resolve, reject) => {
    const headers = {
      'X-Source-Agent': sourceAgent,
    };

    let payload = null;
    if (method === 'POST' && body !== null) {
      payload = JSON.stringify(body);
      headers['Content-Type'] = 'application/json';
      headers['Content-Length'] = Buffer.byteLength(payload);

      const token = readToken();
      if (token) {
        headers['Authorization'] = `Bearer ${token}`;
      }
    }

    const req = http.request(
      {
        hostname: DAEMON_HOST,
        port: DAEMON_PORT,
        path: urlPath,
        method,
        headers,
        timeout: REQUEST_TIMEOUT_MS,
      },
      (res) => {
        let data = '';
        res.on('data', (chunk) => { data += chunk; });
        res.on('end', () => {
          try {
            resolve(JSON.parse(data));
          } catch {
            reject(new Error(`Invalid JSON from daemon: ${data.slice(0, 200)}`));
          }
        });
      }
    );

    req.on('timeout', () => {
      req.destroy();
      reject(new Error('Request timed out after 10s'));
    });

    req.on('error', (err) => {
      reject(err);
    });

    if (payload) {
      req.write(payload);
    }
    req.end();
  });
}

// ─── Daemon Management ─────────────────────────────────────────────────────

/**
 * Check if the daemon is alive by hitting GET /health.
 * Returns true if we get a successful response.
 */
async function isDaemonAlive() {
  try {
    const res = await request('GET', '/health');
    return res && res.status === 'ok';
  } catch {
    return false;
  }
}

/**
 * Read PID from the PID file. Returns null if missing or unreadable.
 */
function readPid() {
  try {
    if (fs.existsSync(PID_FILE)) {
      const pid = parseInt(fs.readFileSync(PID_FILE, 'utf-8').trim(), 10);
      return Number.isFinite(pid) ? pid : null;
    }
  } catch {
    // PID file may be stale or corrupt
  }
  return null;
}

/**
 * Spawn the daemon as a detached child process.
 * Pipes stdout/stderr to cortex.log in append mode.
 */
function spawnDaemon() {
  if (!fs.existsSync(CORTEX_DIR)) {
    fs.mkdirSync(CORTEX_DIR, { recursive: true });
  }

  if (!fs.existsSync(DAEMON_SCRIPT)) {
    throw new Error(`Daemon script not found: ${DAEMON_SCRIPT}`);
  }

  const logFd = fs.openSync(LOG_FILE, 'a');

  const child = spawn(process.execPath, [DAEMON_SCRIPT, 'serve'], {
    detached: true,
    stdio: ['ignore', logFd, logFd],
    cwd: path.dirname(DAEMON_SCRIPT),
  });

  child.unref();
  fs.closeSync(logFd);

  return child.pid;
}

/**
 * Sleep for a given number of milliseconds.
 */
function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

/**
 * Ensure the daemon is running. If not, spawn it and wait for /health.
 * Returns true if daemon is ready, false on failure.
 */
async function ensureDaemon() {
  // Fast path: daemon already responding
  if (await isDaemonAlive()) return true;

  // Attempt to spawn
  process.stderr.write('Starting Cortex daemon...');

  let spawnedPid;
  try {
    spawnedPid = spawnDaemon();
  } catch (err) {
    process.stderr.write(` failed\n`);
    process.stderr.write(`Error: ${err.message}\n`);
    return false;
  }

  // Poll /health until ready or timeout
  const deadline = Date.now() + STARTUP_TIMEOUT_MS;
  while (Date.now() < deadline) {
    await sleep(STARTUP_POLL_MS);
    if (await isDaemonAlive()) {
      process.stderr.write(` ready (PID ${spawnedPid})\n`);
      return true;
    }
    process.stderr.write('.');
  }

  process.stderr.write(` timeout\n`);
  process.stderr.write(`Daemon did not respond within ${STARTUP_TIMEOUT_MS / 1000}s.\n`);
  process.stderr.write(`Check logs: ${LOG_FILE}\n`);
  return false;
}

// ─── Command Handlers ──────────────────────────────────────────────────────

async function cmdBoot(flags) {
  const profile = encodeURIComponent(flags.profile || 'full');
  const res = await request('GET', `/boot?profile=${profile}`);

  if (res.error) {
    process.stderr.write(`Error: ${res.error}\n`);
    return 1;
  }

  // Print raw markdown boot prompt — no JSON wrapper
  process.stdout.write(res.bootPrompt || res.prompt || '');
  if (!process.stdout.isTTY) return 0;
  process.stdout.write('\n');
  return 0;
}

async function cmdRecall(positional, flags) {
  const query = positional.join(' ');
  if (!query) {
    process.stderr.write('Usage: cortex recall <query>\n');
    return 1;
  }

  const k = flags.k || '7';
  const q = encodeURIComponent(query);
  const res = await request('GET', `/recall?q=${q}&k=${k}`);

  if (res.error) {
    process.stderr.write(`Error: ${res.error}\n`);
    return 1;
  }

  const results = res.results || [];
  if (results.length === 0) {
    process.stdout.write('No results found.\n');
    return 0;
  }

  for (const r of results) {
    const relevance = typeof r.relevance === 'number' ? r.relevance.toFixed(3) : '?';
    const source = r.source || 'unknown';
    const excerpt = (r.excerpt || '').replace(/\n/g, ' ').slice(0, 120);
    process.stdout.write(`- [${relevance}] ${source}: ${excerpt}\n`);
  }
  return 0;
}

async function cmdStore(positional, flags) {
  const text = positional.join(' ');
  if (!text) {
    process.stderr.write('Usage: cortex store <text> [--agent <name>] [--type <type>]\n');
    return 1;
  }

  const sourceAgent = flags.agent || 'cli-manual';
  const type = flags.type || 'decision';

  const res = await request('POST', '/store', {
    decision: text,
    context: null,
    type,
    source_agent: sourceAgent,
  }, sourceAgent);

  if (res.error) {
    process.stderr.write(`Error: ${res.error}\n`);
    return 1;
  }

  const entry = res.entry || {};
  if (res.stored) {
    const preview = text.length > 80 ? text.slice(0, 80) + '...' : text;
    process.stdout.write(`Stored: ${preview}\n`);
    if (entry.status === 'disputed') {
      process.stdout.write(`  Warning: conflicts with decision #${entry.conflictWith}\n`);
    }
    if (entry.surprise !== undefined) {
      process.stdout.write(`  Surprise: ${entry.surprise.toFixed(3)}\n`);
    }
  } else {
    const reason = entry.reason || 'unknown';
    process.stderr.write(`Not stored: ${reason}\n`);
    if (reason === 'duplicate') {
      process.stderr.write(`  Similarity too high (surprise: ${(entry.surprise || 0).toFixed(3)})\n`);
    }
    return 1;
  }
  return 0;
}

async function cmdHealth() {
  const res = await request('GET', '/health');

  if (res.error) {
    process.stderr.write(`Error: ${res.error}\n`);
    return 1;
  }

  const s = res.stats || {};
  process.stdout.write(`Cortex Health\n`);
  process.stdout.write(`─────────────\n`);
  process.stdout.write(`Status:     ${res.status || 'unknown'}\n`);
  process.stdout.write(`Ollama:     ${s.ollama || 'unknown'}\n`);
  process.stdout.write(`Memories:   ${s.memories ?? '?'}\n`);
  process.stdout.write(`Decisions:  ${s.decisions ?? '?'}\n`);
  process.stdout.write(`Embeddings: ${s.embeddings ?? '?'}\n`);
  process.stdout.write(`Events:     ${s.events ?? '?'}\n`);
  return 0;
}

async function cmdForget(positional) {
  const keyword = positional.join(' ');
  if (!keyword) {
    process.stderr.write('Usage: cortex forget <keyword>\n');
    return 1;
  }

  const res = await request('POST', '/forget', { keyword });

  if (res.error) {
    process.stderr.write(`Error: ${res.error}\n`);
    return 1;
  }

  process.stdout.write(`Decayed ${res.affected || 0} entries matching "${keyword}"\n`);
  return 0;
}

async function cmdResolve(positional, flags) {
  const id = parseInt(positional[0], 10);
  if (!Number.isFinite(id)) {
    process.stderr.write('Usage: cortex resolve <id> --keep <other_id> | --supersede <other_id>\n');
    return 1;
  }

  let action, otherId;

  if (flags.keep) {
    action = 'keep';
    otherId = parseInt(flags.keep, 10);
  } else if (flags.supersede) {
    action = 'keep';
    // --supersede means the other one wins; swap IDs so the "other" is kept
    otherId = id;
    // The id we received becomes the superseded one, and flags.supersede is the keeper
    const keeperId = parseInt(flags.supersede, 10);
    if (!Number.isFinite(keeperId)) {
      process.stderr.write('Error: --supersede requires a valid decision ID\n');
      return 1;
    }
    return resolveRequest(keeperId, action, otherId);
  } else {
    process.stderr.write('Usage: cortex resolve <id> --keep <other_id> | --supersede <other_id>\n');
    return 1;
  }

  if (!Number.isFinite(otherId)) {
    process.stderr.write('Error: --keep requires a valid decision ID\n');
    return 1;
  }

  return resolveRequest(id, action, otherId);
}

async function resolveRequest(keepId, action, supersededId) {
  const res = await request('POST', '/resolve', {
    keepId,
    action,
    supersededId,
  });

  if (res.error) {
    process.stderr.write(`Error: ${res.error}\n`);
    return 1;
  }

  process.stdout.write(`Resolved: kept #${keepId}, superseded #${supersededId}\n`);
  return 0;
}

async function cmdStop() {
  try {
    const res = await request('POST', '/shutdown', {});
    process.stdout.write(`Daemon shutting down: ${res.message || 'ok'}\n`);
  } catch (err) {
    // Connection reset is expected when daemon exits immediately
    if (err.code === 'ECONNRESET' || err.code === 'ECONNREFUSED') {
      process.stdout.write('Daemon stopped.\n');
    } else {
      process.stderr.write(`Error stopping daemon: ${err.message}\n`);
      return 1;
    }
  }
  return 0;
}

async function cmdStatus() {
  const pid = readPid();
  const alive = await isDaemonAlive();

  if (!alive) {
    process.stdout.write('Cortex daemon is not running.\n');
    if (pid) {
      process.stdout.write(`  Stale PID file: ${pid}\n`);
    }
    return 1;
  }

  // Fetch full health for stats
  const res = await request('GET', '/health');
  const s = res.stats || {};

  process.stdout.write(`Cortex Status\n`);
  process.stdout.write(`─────────────\n`);
  process.stdout.write(`PID:        ${pid || 'unknown'}\n`);
  process.stdout.write(`Memories:   ${s.memories ?? '?'}\n`);
  process.stdout.write(`Decisions:  ${s.decisions ?? '?'}\n`);
  process.stdout.write(`Embeddings: ${s.embeddings ?? '?'}\n`);
  return 0;
}

async function cmdDigest() {
  const res = await request('GET', '/digest');

  if (res.error) {
    process.stderr.write(`Error: ${res.error}\n`);
    return 1;
  }

  const t = res.totals || {};
  const d = res.today || {};
  const boots = res.agentBoots || [];
  const top = res.topRecalled || [];
  const decay = res.decay || {};

  process.stdout.write(`Cortex Daily — ${res.date || 'unknown'}\n`);
  process.stdout.write(`${'─'.repeat(50)}\n`);
  process.stdout.write(`Memories:   ${t.memories ?? '?'} (+${d.newMemories ?? 0} today)\n`);
  process.stdout.write(`Decisions:  ${t.decisions ?? '?'} (+${d.newDecisions ?? 0} today)\n`);
  process.stdout.write(`Conflicts:  ${t.conflicts ?? 0}\n`);
  process.stdout.write(`Decaying:   ${(decay.memories || 0) + (decay.decisions || 0)} entries below 0.5 score\n`);

  if (boots.length) {
    const agentStr = boots.map(a => `${a.source_agent} (${a.cnt})`).join(', ');
    process.stdout.write(`Agents:     ${agentStr}\n`);
  } else {
    process.stdout.write(`Agents:     no boots today\n`);
  }

  if (top.length) {
    process.stdout.write(`\nTop Recalled:\n`);
    for (const r of top.slice(0, 5)) {
      const label = (r.source || r.text || '').slice(0, 60);
      process.stdout.write(`  ${r.retrievals}x  ${label}\n`);
    }
  }

  return 0;
}

function cmdHelp() {
  const help = `
Cortex v2 — Universal AI Memory CLI

Usage: cortex <command> [options]

Commands:
  boot [--profile <name>]                  Print compiled boot prompt to stdout
  recall <query>                           Search memories, print results
  store <text> [--agent <n>] [--type <t>]  Store a decision or memory
  health                                   Show system health status
  digest                                   Daily health digest (activity, trends)
  forget <keyword>                         Decay matching entries
  resolve <id> --keep|--supersede <other>  Resolve a conflict between decisions
  stop                                     Shutdown the daemon
  status                                   Show PID, uptime, and counts
  help                                     Show this help text

Options:
  --profile <name>   Boot profile (full, operational, subagent, index)
  --agent <name>     Source agent identifier (default: cli-manual)
  --type <type>      Decision type (default: decision)

Daemon:
  The daemon auto-starts on first command (localhost:${DAEMON_PORT}).
  Logs: ${LOG_FILE}
  PID:  ${PID_FILE}

Examples:
  cortex boot --profile operational
  cortex recall "how did we handle auth tokens"
  cortex store "Use uv for all Python installs" --agent claude-code
  cortex forget "deprecated-api"
  cortex resolve 42 --keep 37
`.trimStart();

  process.stdout.write(help);
  return 0;
}

// ─── Formatting Helpers ────────────────────────────────────────────────────

function formatUptime(seconds) {
  if (seconds === undefined || seconds === null) return 'unknown';

  const s = Math.floor(seconds);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;

  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (h < 24) return `${h}h ${m}m`;

  const d = Math.floor(h / 24);
  return `${d}d ${h % 24}h ${m}m`;
}

// ─── Main ──────────────────────────────────────────────────────────────────

async function main() {
  const { command, positional, flags } = parseArgs(process.argv);

  // Help does not require the daemon
  if (command === 'help' || flags.help) {
    process.exitCode = cmdHelp();
    return;
  }

  // All other commands require a running daemon
  if (!(await ensureDaemon())) {
    process.stderr.write('Failed to connect to Cortex daemon.\n');
    process.exitCode = 1;
    return;
  }

  let exitCode;

  try {
    switch (command) {
      case 'boot':
        exitCode = await cmdBoot(flags);
        break;
      case 'recall':
        exitCode = await cmdRecall(positional, flags);
        break;
      case 'store':
        exitCode = await cmdStore(positional, flags);
        break;
      case 'health':
        exitCode = await cmdHealth();
        break;
      case 'digest':
        exitCode = await cmdDigest();
        break;
      case 'forget':
        exitCode = await cmdForget(positional);
        break;
      case 'resolve':
        exitCode = await cmdResolve(positional, flags);
        break;
      case 'stop':
        exitCode = await cmdStop();
        break;
      case 'status':
        exitCode = await cmdStatus();
        break;
      default:
        process.stderr.write(`Unknown command: ${command}\n`);
        process.stderr.write('Run "cortex help" for usage.\n');
        exitCode = 1;
    }
  } catch (err) {
    if (err.code === 'ECONNREFUSED') {
      process.stderr.write('Error: Daemon is not responding (connection refused)\n');
    } else if (err.code === 'ECONNRESET') {
      process.stderr.write('Error: Daemon connection reset unexpectedly\n');
    } else {
      process.stderr.write(`Error: ${err.message}\n`);
    }
    exitCode = 1;
  }

  process.exitCode = exitCode;
}

main();
