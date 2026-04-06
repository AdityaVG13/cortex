#!/usr/bin/env node
/**
 * Cortex Plugin - SessionStart Hook
 *
 * Ensures daemon is running before MCP server connects.
 * - Solo mode: starts local daemon
 * - Team mode: health-checks remote server
 *
 * Output: JSON on stdout for Claude Code to consume.
 */

const path = require('path');
const { spawnSync } = require('child_process');

const PLUGIN_ROOT = process.env.CLAUDE_PLUGIN_ROOT;
const PLUGIN_DATA = process.env.CLAUDE_PLUGIN_DATA;

// Load prepare-runtime first (ensures binary is extracted)
require('./prepare-runtime.cjs');

const PLATFORM = process.platform;
const binaryName = PLATFORM === 'win32' ? 'cortex.exe' : 'cortex';
const binaryPath = path.join(PLUGIN_DATA, 'bin', binaryName);
const DEFAULT_DAEMON_PORT = 7437;

// User config from Claude Code (CLAUDE_PLUGIN_OPTION_<KEY>)
const cortexUrl = process.env.CLAUDE_PLUGIN_OPTION_CORTEX_URL || '';
const cortexApiKey = process.env.CLAUDE_PLUGIN_OPTION_CORTEX_API_KEY || '';

const isTeamMode = cortexUrl && cortexUrl.trim().length > 0;

/**
 * Health check a daemon endpoint
 */
function healthCheck(url, timeoutMs = 5000) {
  const timeoutSecs = Math.max(1, Math.ceil(timeoutMs / 1000));
  try {
    const result = spawnSync('curl', ['-sf', '--connect-timeout', String(timeoutSecs), '--max-time', String(timeoutSecs), `${url}/health`], {
      encoding: 'utf8',
      timeout: timeoutMs
    });
    if (result.error) {
      return { ok: false, error: result.error.message };
    }
    if (result.status === 0 && result.stdout) {
      const data = JSON.parse(result.stdout);
      const stats = data.stats || {};
      return {
        ok: true,
        status: data.status,
        memories: stats.memories,
        decisions: stats.decisions
      };
    }
    return { ok: false, error: (result.stderr || '').trim() || `curl exited with ${result.status}` };
  } catch (e) {
    return { ok: false, error: e.message };
  }
}

/**
 * Get health status for local daemon
 */
function getLocalHealth() {
  // Resolve port from daemon (cortex paths --json)
  let port = DEFAULT_DAEMON_PORT;
  try {
    const result = spawnSync(binaryPath, ['paths', '--json'], { encoding: 'utf8', timeout: 5000 });
    if (result.error) {
      console.error(`[cortex-plugin] paths --json failed: ${result.error.message}`);
    }
    if (result.status === 0 && result.stdout) {
      const paths = JSON.parse(result.stdout);
      port = paths.port || port;
    } else if (result.status !== 0 && result.stderr) {
      console.error(`[cortex-plugin] paths --json stderr: ${result.stderr.trim()}`);
    }
  } catch (e) {
    console.error(`[cortex-plugin] Failed to resolve daemon port: ${e.message}`);
  }
  return healthCheck(`http://127.0.0.1:${port}`);
}

/**
 * Solo mode: ensure local daemon is running
 */
function ensureLocalDaemon() {
  // Check if already healthy
  const health = getLocalHealth();
  if (health.ok) {
    return { started: false, health };
  }

  // Not running, start it
  console.error('[cortex-plugin] Starting local daemon...');

  try {
    const result = spawnSync(binaryPath, ['plugin', 'ensure-daemon', '--agent', 'claude-code'], {
      encoding: 'utf8',
      timeout: 15000,
      env: { ...process.env }
    });

    if (result.error) {
      return {
        started: false,
        error: result.error.message,
        health: getLocalHealth()
      };
    }

    if (result.status !== 0) {
      return {
        started: false,
        error: result.stderr || 'ensure-daemon failed',
        health: getLocalHealth()
      };
    }

    // Check health again
    const newHealth = getLocalHealth();
    return { started: true, health: newHealth };
  } catch (e) {
    return { started: false, error: e.message, health: getLocalHealth() };
  }
}

/**
 * Team mode: just check remote server
 */
function checkTeamServer() {
  const health = healthCheck(cortexUrl.trim());
  return health;
}

/**
 * Build status line for Claude context
 */
function buildStatusLine(health, mode) {
  const parts = [];

  if (health.ok) {
    parts.push('READY');
    const counts = [];
    if (typeof health.memories === 'number') counts.push(`${health.memories} memories`);
    if (typeof health.decisions === 'number') counts.push(`${health.decisions} decisions`);
    if (counts.length > 0) {
      parts.push(`(${counts.join(', ')})`);
    }
  } else {
    parts.push('UNAVAILABLE');
  }

  if (mode === 'team') {
    parts.push(`| Team @ ${cortexUrl.split('/')[2]}`);
  } else {
    parts.push('| Solo mode');
  }

  return `Brain: ${parts.join(' ')} | Cortex`;
}

// Main
let result;

if (isTeamMode) {
  console.error(`[cortex-plugin] Team mode: connecting to ${cortexUrl}`);
  const health = checkTeamServer();
  result = { mode: 'team', health, url: cortexUrl };
  const status = buildStatusLine(health, 'team');

  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: 'SessionStart',
      additionalContext: status
    }
  }) + '\n');
} else {
  console.error('[cortex-plugin] Solo mode: ensuring local daemon');
  const { started, health, error } = ensureLocalDaemon();
  result = { mode: 'solo', started, health, error };
  const status = buildStatusLine(health, 'solo');

  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: 'SessionStart',
      additionalContext: status
    }
  }) + '\n');
}
