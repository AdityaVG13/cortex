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

const PLUGIN_ROOT = process.env.CLAUDE_PLUGIN_ROOT;
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

// Load prepare-runtime first (ensures binary is extracted)
require('./prepare-runtime.cjs');

const PLATFORM = process.platform;
const binaryName = PLATFORM === 'win32' ? 'cortex.exe' : 'cortex';
const binaryPath = path.join(PLUGIN_DATA, 'bin', binaryName);

// User config from Claude Code
const cortexUrl = process.env.CLAUDE_PLUGIN_OPTION_CORTEX_URL || '';
const cortexApiKey = process.env.CLAUDE_PLUGIN_OPTION_CORTEX_API_KEY || '';

const isTeamMode = cortexUrl && cortexUrl.trim().length > 0;
const pluginAgent = (process.env.CORTEX_PLUGIN_AGENT || 'claude-code').trim() || 'claude-code';
const isClaudeAgent = /^claude(?:-|$)/i.test(pluginAgent);

// Build args for cortex plugin mcp
const args = ['plugin', 'mcp', '--agent', pluginAgent];

if (isTeamMode) {
  args.push('--url', cortexUrl.trim());
  if (cortexApiKey && cortexApiKey.trim().length > 0) {
    args.push('--api-key', cortexApiKey.trim());
  }
} else {
  if (!isClaudeAgent) {
    crashLog(`Refusing local daemon spawn for non-Claude agent "${pluginAgent}"`);
    process.exit(1);
  }
  // Solo mode: use default localhost URL (daemon resolve its own port)
  // cortex plugin mcp will use resolved port from CortexPaths
}

const childEnv = {
  ...process.env,
  CORTEX_DAEMON_OWNER_KIND: 'plugin',
  CORTEX_DAEMON_OWNER_SOURCE: 'claude-plugin',
  CORTEX_DAEMON_OWNER_AGENT: pluginAgent,
  CORTEX_DAEMON_OWNER_MODE: isTeamMode ? 'team' : 'solo',
  CORTEX_DAEMON_OWNER_CLAUDE_ONLY: '1',
  CORTEX_DAEMON_OWNER_LOCAL_SPAWN: isTeamMode ? '0' : '1',
  CORTEX_DAEMON_OWNER_PARENT_PID: String(process.pid)
};

// Spawn the MCP proxy with stdio: 'inherit'
// The child process takes over stdin/stdout entirely
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
