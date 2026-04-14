#!/usr/bin/env node
/**
 * Cortex Plugin - MCP Server Bridge
 *
 * Spawns the cortex MCP proxy, handing over stdin/stdout entirely.
 * The child process runs the actual MCP protocol against the daemon.
 *
 * CRITICAL: stdio: 'inherit' is required - do NOT pipe and relay.
 */

const path = require('path');
const fs = require('fs');
const { spawn } = require('child_process');
const { resolveCortexBinary } = require('./resolve-binary.cjs');

const PLUGIN_DATA = process.env.CLAUDE_PLUGIN_DATA;

function crashLog(msg) {
  const home = process.env.USERPROFILE || process.env.HOME || '.';
  const cortexDir = path.join(home, '.cortex');
  const logPath = path.join(cortexDir, 'mcp-crash.log');
  const line = `[${new Date().toISOString()}] ${msg}\n`;
  try {
    fs.mkdirSync(cortexDir, { recursive: true });
    fs.appendFileSync(logPath, line);
  } catch (_) {}
  console.error(`[cortex-plugin] ${msg}`);
}

function normalizeOption(value) {
  if (typeof value !== 'string') return '';
  return value.trim();
}

function isTruthy(value) {
  const normalized = normalizeOption(value).toLowerCase();
  return normalized === '1' || normalized === 'true' || normalized === 'yes' || normalized === 'on';
}

function resolveRoute(config) {
  const explicitUrl = normalizeOption(config.cortexUrl);
  const appUrl = normalizeOption(process.env.CORTEX_APP_URL);

  if (explicitUrl) {
    return { mode: 'remote', url: explicitUrl, reason: 'explicit plugin URL' };
  }

  if (appUrl) {
    return { mode: 'remote', url: appUrl, reason: 'app route' };
  }

  return { mode: 'local', url: '', reason: 'local attach-only' };
}

const PLATFORM = process.platform;
const binaryName = PLATFORM === 'win32' ? 'cortex.exe' : 'cortex';
// User config from Claude Code
const cortexUrl = process.env.CLAUDE_PLUGIN_OPTION_CORTEX_URL || '';
const cortexApiKey = process.env.CLAUDE_PLUGIN_OPTION_CORTEX_API_KEY || '';
const pluginAgent = (process.env.CORTEX_PLUGIN_AGENT || 'claude-code').trim() || 'claude-code';
const dryRun = isTruthy(process.env.CORTEX_PLUGIN_DRY_RUN);
const allowBundledBinary = isTruthy(process.env.CORTEX_PLUGIN_ALLOW_BUNDLED_BINARY);

const route = resolveRoute({ cortexUrl });

let binaryPath = '';
let binarySource = '';
try {
  const resolved = resolveCortexBinary({
    pluginData: PLUGIN_DATA,
    binaryName,
    ensureBundled: () => require('./prepare-runtime.cjs'),
    allowBundled: route.mode !== 'local' || allowBundledBinary,
    rejectTempCandidates: route.mode === 'local' && !allowBundledBinary
  });
  binaryPath = resolved.binaryPath;
  binarySource = resolved.source;
} catch (error) {
  crashLog(
    `BINARY RESOLUTION FAILED: ${error && error.message ? error.message : error}`
  );
  if (route.mode === 'local') {
    console.error(
      '[cortex-plugin] Local attach mode requires an app-managed binary. Start Control Center or set CORTEX_APP_BINARY. ' +
      'Set CORTEX_PLUGIN_ALLOW_BUNDLED_BINARY=1 only if you explicitly accept bundled fallback.'
    );
  }
  process.exit(1);
}

const args = ['plugin', 'mcp', '--agent', pluginAgent];
if (route.mode === 'remote') {
  args.push('--url', route.url);
  if (normalizeOption(cortexApiKey).length > 0) {
    args.push('--api-key', normalizeOption(cortexApiKey));
  }
}

const ownerMode = route.mode === 'local'
  ? 'solo-attach'
  : route.reason === 'explicit plugin URL'
    ? 'team'
    : 'app';

console.error(`[cortex-plugin] MCP route: ${route.mode} (${route.reason})`);
console.error(`[cortex-plugin] Cortex binary: ${binaryPath} (${binarySource})`);

if (dryRun) {
  console.error(
    `[cortex-plugin] Dry run complete. agent=${pluginAgent} mode=${route.mode} url=${route.url || '(none)'}`
  );
  process.exit(0);
}

const childEnv = {
  ...process.env,
  CORTEX_DAEMON_OWNER_KIND: 'plugin',
  CORTEX_DAEMON_OWNER_SOURCE: 'claude-plugin',
  CORTEX_DAEMON_OWNER_AGENT: pluginAgent,
  CORTEX_DAEMON_OWNER_MODE: ownerMode,
  CORTEX_DAEMON_OWNER_LOCAL_SPAWN: '0',
  CORTEX_DAEMON_OWNER_PARENT_PID: String(process.pid)
};

if (route.mode === 'local') {
  const userHome = process.env.USERPROFILE || process.env.HOME || '';
  if (userHome) {
    childEnv.CORTEX_HOME = path.join(userHome, '.cortex');
  }
  delete childEnv.CORTEX_DB;
}

const child = spawn(binaryPath, args, {
  stdio: 'inherit',
  env: childEnv
});

process.on('uncaughtException', (err) => {
  crashLog(`BRIDGE CRASH: ${err && err.stack ? err.stack : err}`);
  process.exit(1);
});

process.on('unhandledRejection', (reason) => {
  crashLog(`BRIDGE REJECTION: ${reason && reason.stack ? reason.stack : reason}`);
  process.exit(1);
});

child.on('error', (err) => {
  crashLog(`SPAWN FAILED: ${err.message} (binary: ${binaryPath})`);
  process.exit(1);
});

child.on('exit', (code, signal) => {
  if (signal) {
    crashLog(`MCP server killed by signal ${signal}`);
    process.exit(1);
  }
  if (code !== 0) {
    crashLog(`MCP server exited with code ${code}`);
  }
  process.exit(code || 0);
});
