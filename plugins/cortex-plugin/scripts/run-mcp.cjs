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
const { spawn } = require('child_process');

const PLUGIN_ROOT = process.env.CLAUDE_PLUGIN_ROOT;
const PLUGIN_DATA = process.env.CLAUDE_PLUGIN_DATA;

// Load prepare-runtime first (ensures binary is extracted)
require('./prepare-runtime.cjs');

const PLATFORM = process.platform;
const binaryName = PLATFORM === 'win32' ? 'cortex.exe' : 'cortex';
const binaryPath = path.join(PLUGIN_DATA, 'bin', binaryName);

// User config from Claude Code
const cortexUrl = process.env.CLAUDE_PLUGIN_OPTION_CORTEX_URL || '';
const cortexApiKey = process.env.CLAUDE_PLUGIN_OPTION_CORTEX_API_KEY || '';

const isTeamMode = cortexUrl && cortexUrl.trim().length > 0;

// Build args for cortex plugin mcp
const args = ['plugin', 'mcp'];

if (isTeamMode) {
  args.push('--url', cortexUrl.trim());
  if (cortexApiKey && cortexApiKey.trim().length > 0) {
    args.push('--api-key', cortexApiKey.trim());
  }
} else {
  // Solo mode: use default localhost URL (daemon resolve its own port)
  // cortex plugin mcp will use resolved port from CortexPaths
}

// Spawn the MCP proxy with stdio: 'inherit'
// The child process takes over stdin/stdout entirely
const child = spawn(binaryPath, args, {
  stdio: 'inherit',
  env: process.env
});

child.on('error', (err) => {
  console.error(`[cortex-plugin] Failed to spawn MCP server: ${err.message}`);
  process.exit(1);
});

child.on('exit', (code, signal) => {
  if (signal) {
    console.error(`[cortex-plugin] MCP server killed by signal ${signal}`);
    process.exit(1);
  }
  process.exit(code || 0);
});
