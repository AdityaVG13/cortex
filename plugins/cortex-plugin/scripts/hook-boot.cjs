#!/usr/bin/env node
/**
 * Cortex Plugin - SessionStart Hook
 *
 * Reports daemon status before MCP server connects.
 * Never starts or stops a daemon from this hook.
 *
 * Output: JSON on stdout for Claude Code to consume.
 */

const path = require('path');
const http = require('http');
const https = require('https');
const { spawnSync } = require('child_process');
const { resolveCortexBinary } = require('./resolve-binary.cjs');

const PLUGIN_DATA = process.env.CLAUDE_PLUGIN_DATA;

function normalizeOption(value) {
  if (typeof value !== 'string') return '';
  return value.trim();
}

function resolveRoute(config) {
  const explicitUrl = normalizeOption(config.cortexUrl);
  const appUrl =
    normalizeOption(process.env.CORTEX_APP_URL) ||
    normalizeOption(process.env.CORTEX_DEV_APP_URL);

  if (explicitUrl) {
    return { mode: 'team', url: explicitUrl, reason: 'explicit plugin URL' };
  }

  if (appUrl) {
    return { mode: 'app', url: appUrl, reason: 'app route' };
  }

  return { mode: 'solo', url: '', reason: 'local attach-only' };
}

const PLATFORM = process.platform;
const binaryName = PLATFORM === 'win32' ? 'cortex.exe' : 'cortex';
const { binaryPath, source: binarySource } = resolveCortexBinary({
  pluginData: PLUGIN_DATA,
  binaryName,
  ensureBundled: () => require('./prepare-runtime.cjs')
});
const DEFAULT_DAEMON_PORT = 7437;

const cortexUrl = process.env.CLAUDE_PLUGIN_OPTION_CORTEX_URL || '';
const route = resolveRoute({ cortexUrl });
console.error(`[cortex-plugin] SessionStart binary: ${binaryPath} (${binarySource})`);

function isCortexHealthResponse(data) {
  if (!data || typeof data !== 'object') return false;
  if (data.status !== 'ok' && data.status !== 'degraded') return false;
  if (!data.stats || typeof data.stats !== 'object') return false;
  // runtime is optional for backward compatibility with older daemons
  if (data.runtime !== undefined && typeof data.runtime !== 'object') return false;
  return true;
}

function isCortexReadinessResponse(data) {
  if (!data || typeof data !== 'object') return false;
  if (typeof data.ready !== 'boolean') return false;
  if (data.status !== 'ready' && data.status !== 'starting') return false;
  if (!data.stats || typeof data.stats !== 'object') return false;
  if (!data.runtime || typeof data.runtime !== 'object') return false;
  return true;
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
  if (!expectedIdentity || typeof expectedIdentity !== 'object') return true;
  if (!data || typeof data !== 'object') return false;
  const runtime = data.runtime;
  const stats = data.stats || {};

  // Backward-compatible: if runtime block is missing, accept legacy payload.
  if (!runtime || typeof runtime !== 'object') return true;

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
    const requestProbe = (pathSuffix, mode, fallback) => {
      let target;
      try {
        target = new URL(`${url.replace(/\/+$/, '')}${pathSuffix}`);
      } catch (e) {
        resolve({ ok: false, error: `Invalid health URL: ${e.message}` });
        return;
      }

      const transport = target.protocol === 'https:' ? https : http;
      const request = transport.request(
        target,
        {
          method: 'GET',
          headers: { 'X-Cortex-Request': 'true' }
        },
        (response) => {
          let body = '';
          response.setEncoding('utf8');
          response.on('data', (chunk) => {
            body += chunk;
          });
          response.on('end', () => {
            let data;
            try {
              data = body ? JSON.parse(body) : null;
            } catch (e) {
              if (fallback) {
                fallback();
                return;
              }
              resolve({ ok: false, error: `Invalid health response: ${e.message}` });
              return;
            }

            if (mode === 'readiness') {
              if (!isCortexReadinessResponse(data)) {
                if (fallback) {
                  fallback();
                  return;
                }
                resolve({ ok: false, error: 'Invalid Cortex readiness response' });
                return;
              }
              if (!validateHealthIdentity(data, expectedIdentity)) {
                resolve({ ok: false, error: 'Invalid Cortex readiness response (identity mismatch)' });
                return;
              }
              if (data.ready === true) {
                const stats = data.stats || {};
                resolve({
                  ok: true,
                  status: 'ready',
                  memories: stats.memories,
                  decisions: stats.decisions
                });
                return;
              }
              resolve({ ok: false, error: body.trim() || `HTTP ${response.statusCode || 'unknown'}` });
              return;
            }

            if (!isCortexHealthResponse(data)) {
              resolve({ ok: false, error: 'Invalid Cortex health response' });
              return;
            }
            if (!validateHealthIdentity(data, expectedIdentity)) {
              resolve({ ok: false, error: 'Invalid Cortex health response (identity mismatch)' });
              return;
            }
            const stats = data.stats || {};
            resolve({
              ok: true,
              status: data.status,
              memories: stats.memories,
              decisions: stats.decisions
            });
          });
        }
      );

      request.setTimeout(timeoutMs, () => {
        request.destroy(new Error(`Health check timed out after ${timeoutMs}ms`));
      });
      request.on('error', (err) => {
        if (fallback) {
          fallback();
          return;
        }
        resolve({ ok: false, error: err.message });
      });
      request.end();
    };

    requestProbe('/readiness', 'readiness', () => requestProbe('/health', 'health'));
  });
}

async function getLocalHealth() {
  let port = DEFAULT_DAEMON_PORT;
  let bind = '127.0.0.1';
  const expectedIdentity = {};

  try {
    const result = spawnSync(binaryPath, ['paths', '--json'], { encoding: 'utf8', timeout: 5000 });
    if (result.status === 0 && result.stdout) {
      const paths = JSON.parse(result.stdout);
      if (Number.isFinite(paths.port)) port = paths.port;
      if (typeof paths.bind === 'string' && paths.bind.trim().length > 0) bind = paths.bind.trim();
      if (typeof paths.home === 'string') expectedIdentity.home = paths.home;
      if (typeof paths.db === 'string') expectedIdentity.db = paths.db;
      if (typeof paths.token === 'string') expectedIdentity.token = paths.token;
      if (typeof paths.pid === 'string') expectedIdentity.pid = paths.pid;
    } else if (result.status !== 0 && result.stderr) {
      console.error(`[cortex-plugin] paths --json stderr: ${result.stderr.trim()}`);
    }
  } catch (e) {
    console.error(`[cortex-plugin] Failed to resolve local daemon paths: ${e.message}`);
  }

  expectedIdentity.port = port;
  const host = formatHostForUrl(resolveHealthHost(bind));
  return healthCheck(`http://${host}:${port}`, 5000, expectedIdentity);
}

function buildStatusLine(health, routeState) {
  const parts = [];

  if (health.ok) {
    parts.push('READY');
    const counts = [];
    if (typeof health.memories === 'number') counts.push(`${health.memories} memories`);
    if (typeof health.decisions === 'number') counts.push(`${health.decisions} decisions`);
    if (counts.length > 0) parts.push(`(${counts.join(', ')})`);
  } else if (routeState === 'solo') {
    parts.push('STANDBY');
  } else {
    parts.push('UNAVAILABLE');
  }

  if (routeState === 'team') {
    parts.push(`| Team @ ${normalizeOption(cortexUrl).split('/')[2] || 'custom'}`);
  } else if (routeState === 'app') {
    parts.push('| App route');
  } else {
    parts.push('| Solo mode');
  }
  return `Brain: ${parts.join(' ')} | Cortex`;
}

function emitStatus(additionalContext) {
  process.stdout.write(
    JSON.stringify({
      hookSpecificOutput: {
        hookEventName: 'SessionStart',
        additionalContext
      }
    }) + '\n'
  );
}

(async () => {
  if (route.mode === 'team' || route.mode === 'app') {
    const health = await healthCheck(route.url);
    emitStatus(buildStatusLine(health, route.mode));
    return;
  }

  const health = await getLocalHealth();
  emitStatus(buildStatusLine(health, 'solo'));
})().catch((err) => {
  console.error(`[cortex-plugin] SessionStart hook failed: ${err && err.stack ? err.stack : err}`);
  emitStatus('Brain: UNAVAILABLE | Cortex');
  process.exit(1);
});
