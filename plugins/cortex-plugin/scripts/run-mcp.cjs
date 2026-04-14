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

function resolveCanonicalCortexHome() {
  const userHome = process.env.USERPROFILE || process.env.HOME || '';
  if (!userHome) return '';
  return path.join(userHome, '.cortex');
}

function resolveRoute(config) {
  const explicitUrl = normalizeOption(config.cortexUrl);
  const devAppUrl =
    normalizeOption(process.env.CORTEX_DEV_APP_URL) ||
    normalizeOption(process.env.CORTEX_APP_URL);
  const preferApp = isTruthy(process.env.CORTEX_DEV_PREFER_APP);
  const disableLocalSpawn = isTruthy(process.env.CORTEX_DEV_DISABLE_LOCAL_SPAWN);
  const allowLocalSpawnRaw = process.env.CORTEX_PLUGIN_ALLOW_LOCAL_SPAWN;
  const allowLocalSpawn =
    allowLocalSpawnRaw === undefined ? false : isTruthy(allowLocalSpawnRaw);

  if (explicitUrl) {
    return { mode: 'remote', url: explicitUrl, reason: 'explicit plugin URL', allowLocalSpawn: false };
  }

  if (preferApp) {
    if (!devAppUrl) {
      return {
        error:
          'CORTEX_DEV_PREFER_APP=1 is set but no app URL was provided. Set CORTEX_DEV_APP_URL (or CORTEX_APP_URL) or configure Cortex Server URL in plugin settings.'
      };
    }
    return { mode: 'remote', url: devAppUrl, reason: 'dev app preference', allowLocalSpawn: false };
  }

  if (disableLocalSpawn || !allowLocalSpawn) {
    return { mode: 'local', url: '', reason: 'local attach-only', allowLocalSpawn: false };
  }

  return { mode: 'local', url: '', reason: 'local fallback', allowLocalSpawn: true };
}

// Load prepare-runtime first (ensures binary is extracted)
require('./prepare-runtime.cjs');

const PLATFORM = process.platform;
const binaryName = PLATFORM === 'win32' ? 'cortex.exe' : 'cortex';
const binaryPath = path.join(PLUGIN_DATA, 'bin', binaryName);

// User config from Claude Code
const cortexUrl = process.env.CLAUDE_PLUGIN_OPTION_CORTEX_URL || '';
const cortexApiKey = process.env.CLAUDE_PLUGIN_OPTION_CORTEX_API_KEY || '';
const pluginAgent = (process.env.CORTEX_PLUGIN_AGENT || 'claude-code').trim() || 'claude-code';
const isClaudeAgent = /^claude(?:-|$)/i.test(pluginAgent);
const dryRun = isTruthy(process.env.CORTEX_PLUGIN_DRY_RUN);

const route = resolveRoute({ cortexUrl });
if (route.error) {
  crashLog(route.error);
  process.exit(1);
}

if (route.mode === 'local' && !isClaudeAgent) {
  crashLog(`Refusing local daemon spawn for non-Claude agent "${pluginAgent}"`);
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
  ? 'solo'
  : route.reason === 'explicit plugin URL'
    ? 'team'
    : 'app';

console.error(`[cortex-plugin] MCP route: ${route.mode} (${route.reason})`);

if (dryRun) {
  console.error(
    `[cortex-plugin] Dry run complete. agent=${pluginAgent} local_spawn=${route.allowLocalSpawn ? 'on' : 'off'} url=${route.url || '(none)'}`
  );
  process.exit(0);
}

const childEnv = {
  ...process.env,
  CORTEX_DAEMON_OWNER_KIND: 'plugin',
  CORTEX_DAEMON_OWNER_SOURCE: 'claude-plugin',
  CORTEX_DAEMON_OWNER_AGENT: pluginAgent,
  CORTEX_DAEMON_OWNER_MODE: ownerMode,
  CORTEX_DAEMON_OWNER_CLAUDE_ONLY: '1',
  CORTEX_DAEMON_OWNER_LOCAL_SPAWN: route.allowLocalSpawn ? '1' : '0',
  CORTEX_DAEMON_OWNER_PARENT_PID: String(process.pid)
};

if (route.mode === 'local') {
  const canonicalHome = resolveCanonicalCortexHome();
  if (canonicalHome) {
    childEnv.CORTEX_HOME = canonicalHome;
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
