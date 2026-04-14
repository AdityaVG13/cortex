#!/usr/bin/env node
/**
 * Cortex Plugin - SessionStart Hook
 *
 * Reports daemon status before MCP server connects.
 * - Solo mode: checks local daemon health without starting it
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

function normalizeRuntimePath(value) {
  if (typeof value !== 'string') return '';
  let normalized = value.trim().replace(/\\/g, '/');
  while (normalized.length > 1 && normalized.endsWith('/')) {
    normalized = normalized.slice(0, -1);
  }
  if (process.platform === 'win32') {
    normalized = normalized.toLowerCase();
  }
  return normalized;
}

function pathFieldMatches(actualValue, expectedPath) {
  if (typeof expectedPath !== 'string' || expectedPath.trim().length === 0) {
    return true;
  }
  if (typeof actualValue !== 'string' || actualValue.trim().length === 0) {
    return false;
  }
  return normalizeRuntimePath(actualValue) === normalizeRuntimePath(expectedPath);
}

function validateHealthIdentity(data, expectedIdentity) {
  if (!expectedIdentity || typeof expectedIdentity !== 'object') {
    return true;
  }

  const runtime = (data && data.runtime) || {};
  const stats = (data && data.stats) || {};

  if (Number.isFinite(expectedIdentity.port) && runtime.port !== expectedIdentity.port) {
    return false;
  }
  if (!pathFieldMatches(stats.home, expectedIdentity.home)) return false;
  if (!pathFieldMatches(runtime.db_path, expectedIdentity.db)) return false;
  if (!pathFieldMatches(runtime.token_path, expectedIdentity.token)) return false;
  if (!pathFieldMatches(runtime.pid_path, expectedIdentity.pid)) return false;

  return true;
}

function resolveHealthHost(bind) {
  const trimmed = typeof bind === 'string' ? bind.trim() : '';
  if (!trimmed || trimmed === '0.0.0.0' || trimmed === '::' || trimmed === '[::]') {
    return '127.0.0.1';
  }
  return trimmed.replace(/^\[/, '').replace(/\]$/, '');
}

function formatHostForUrl(host) {
  return host.includes(':') && !host.startsWith('[') ? `[${host}]` : host;
}

function healthCheck(url, timeoutMs = 5000, expectedIdentity = null) {
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
              if (!validateHealthIdentity(data, expectedIdentity)) {
                finish({ ok: false, error: 'Invalid Cortex health response (local identity mismatch)' });
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
  let bind = '127.0.0.1';
  const expectedIdentity = {};
  try {
    const result = spawnSync(binaryPath, ['paths', '--json'], { encoding: 'utf8', timeout: 5000 });
    if (result.error) {
      console.error(`[cortex-plugin] paths --json failed: ${result.error.message}`);
    }
    if (result.status === 0 && result.stdout) {
      const paths = JSON.parse(result.stdout);
      if (Number.isFinite(paths.port)) {
        port = paths.port;
      }
      if (typeof paths.bind === 'string' && paths.bind.trim().length > 0) {
        bind = paths.bind.trim();
      }
      if (typeof paths.home === 'string') expectedIdentity.home = paths.home;
      if (typeof paths.db === 'string') expectedIdentity.db = paths.db;
      if (typeof paths.token === 'string') expectedIdentity.token = paths.token;
      if (typeof paths.pid === 'string') expectedIdentity.pid = paths.pid;
    } else if (result.status !== 0 && result.stderr) {
      console.error(`[cortex-plugin] paths --json stderr: ${result.stderr.trim()}`);
    }
  } catch (e) {
    console.error(`[cortex-plugin] Failed to resolve daemon port: ${e.message}`);
  }
  expectedIdentity.port = port;
  const host = formatHostForUrl(resolveHealthHost(bind));
  return healthCheck(`http://${host}:${port}`, 5000, expectedIdentity);
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
  } else if (mode === 'solo') {
    parts.push('STANDBY');
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
    console.error('[cortex-plugin] Solo mode: checking local daemon');
    const health = await getLocalHealth();
    result = { mode: 'solo', health };
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
