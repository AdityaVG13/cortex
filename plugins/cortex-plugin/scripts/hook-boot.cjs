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
const http = require('http');
const https = require('https');
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
function isCortexHealthResponse(data) {
  return Boolean(
    data &&
    typeof data === 'object' &&
    (data.status === 'ok' || data.status === 'degraded') &&
    data.runtime &&
    typeof data.runtime === 'object' &&
    data.stats &&
    typeof data.stats === 'object'
  );
}

function healthCheck(url, timeoutMs = 5000) {
  return new Promise((resolve) => {
    let target;
    try {
      target = new URL(`${url.replace(/\/+$/, '')}/health`);
    } catch (e) {
      resolve({ ok: false, error: `Invalid health URL: ${e.message}` });
      return;
    }

    const transport = target.protocol === 'https:' ? https : http;
    let settled = false;
    const finish = (result) => {
      if (!settled) {
        settled = true;
        resolve(result);
      }
    };

    const request = transport.request(
      target,
      {
        method: 'GET',
        headers: {
          'X-Cortex-Request': 'true'
        }
      },
      (response) => {
        let body = '';
        response.setEncoding('utf8');
        response.on('data', (chunk) => {
          body += chunk;
        });
        response.on('end', () => {
          if (response.statusCode >= 200 && response.statusCode < 300 && body) {
            try {
              const data = JSON.parse(body);
              if (!isCortexHealthResponse(data)) {
                finish({ ok: false, error: 'Invalid Cortex health response' });
                return;
              }
              const stats = data.stats || {};
              finish({
                ok: true,
                status: data.status,
                memories: stats.memories,
                decisions: stats.decisions
              });
            } catch (e) {
              finish({ ok: false, error: `Invalid health response: ${e.message}` });
            }
            return;
          }

          finish({
            ok: false,
            error: body.trim() || `HTTP ${response.statusCode || 'unknown'}`
          });
        });
      }
    );

    request.setTimeout(timeoutMs, () => {
      request.destroy(new Error(`Health check timed out after ${timeoutMs}ms`));
    });
    request.on('error', (err) => {
      finish({ ok: false, error: err.message });
    });
    request.end();
  });
}

/**
 * Get health status for local daemon
 */
async function getLocalHealth() {
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
async function ensureLocalDaemon() {
  // Check if already healthy
  const health = await getLocalHealth();
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
        health: await getLocalHealth()
      };
    }

    if (result.status !== 0) {
      return {
        started: false,
        error: result.stderr || 'ensure-daemon failed',
        health: await getLocalHealth()
      };
    }

    // Check health again
    const newHealth = await getLocalHealth();
    return { started: true, health: newHealth };
  } catch (e) {
    return { started: false, error: e.message, health: await getLocalHealth() };
  }
}

/**
 * Team mode: just check remote server
 */
async function checkTeamServer() {
  return healthCheck(cortexUrl.trim());
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

(async () => {
  let result;

  if (isTeamMode) {
    console.error(`[cortex-plugin] Team mode: connecting to ${cortexUrl}`);
    const health = await checkTeamServer();
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
    const { started, health, error } = await ensureLocalDaemon();
    result = { mode: 'solo', started, health, error };
    const status = buildStatusLine(health, 'solo');

    process.stdout.write(JSON.stringify({
      hookSpecificOutput: {
        hookEventName: 'SessionStart',
        additionalContext: status
      }
    }) + '\n');
  }

  return result;
})().catch((err) => {
  console.error(`[cortex-plugin] SessionStart hook failed: ${err && err.stack ? err.stack : err}`);
  process.stdout.write(JSON.stringify({
    hookSpecificOutput: {
      hookEventName: 'SessionStart',
      additionalContext: 'Brain: UNAVAILABLE | Cortex'
    }
  }) + '\n');
  process.exit(1);
});
