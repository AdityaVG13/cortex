#!/usr/bin/env node
/**
 * Cortex Plugin - SessionStart Hook
 *
 * Reports daemon status before MCP server connects. The hook is status-only:
 * it never starts, stops, or shells into a Cortex binary.
 */

const http = require('http');
const https = require('https');
const fs = require('fs');
const path = require('path');

const DEFAULT_LOCAL_BASE_URL = 'http://127.0.0.1:7437';

function normalizeOption(value) {
  if (typeof value !== 'string') return '';
  return value.trim();
}

function isTruthy(value) {
  const normalized = normalizeOption(value).toLowerCase();
  return normalized === '1' || normalized === 'true' || normalized === 'yes' || normalized === 'on';
}

/**
 * Resolve route mode for the SessionStart status hook. Mirrors `run-mcp.cjs`
 * but uses UI-facing mode names (`team` / `app` / `solo` / `solo-standby`).
 */
function resolveRoute(config, env = process.env) {
  const explicitUrl = normalizeOption(config.cortexUrl);
  const devPreferApp = isTruthy(env.CORTEX_DEV_PREFER_APP);
  const appUrl = normalizeOption(env.CORTEX_APP_URL);

  if (explicitUrl) {
    return { mode: 'team', url: explicitUrl, reason: 'explicit plugin URL' };
  }

  if (devPreferApp) {
    if (appUrl) {
      return {
        mode: 'app',
        url: appUrl,
        reason: 'dev prefer app (CORTEX_APP_URL)'
      };
    }
    return {
      mode: 'solo-standby',
      url: '',
      reason: 'CORTEX_DEV_PREFER_APP=1 but CORTEX_APP_URL is not set'
    };
  }

  if (appUrl) {
    return { mode: 'app', url: appUrl, reason: 'app route' };
  }

  return { mode: 'solo', url: DEFAULT_LOCAL_BASE_URL, reason: 'local HTTP attach-only' };
}

function normalizeBaseUrl(baseUrl) {
  const normalized = normalizeOption(baseUrl).replace(/\/+$/, '');
  if (!normalized) return '';
  const parsed = new URL(normalized);
  if (parsed.username || parsed.password) {
    throw new Error('Cortex target URL must not include embedded credentials');
  }
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
    throw new Error(`Unsupported Cortex target URL scheme '${parsed.protocol}'`);
  }
  parsed.search = '';
  parsed.hash = '';
  return parsed.toString().replace(/\/+$/, '');
}

function isLocalBaseUrl(baseUrl) {
  try {
    const parsed = new URL(baseUrl);
    const host = (parsed.hostname || '').toLowerCase();
    return host === '127.0.0.1' || host === 'localhost' || host === '::1';
  } catch (_) {
    return false;
  }
}

function resolveCortexHome(env = process.env) {
  const explicit = normalizeOption(env.CORTEX_HOME);
  if (explicit) return explicit;
  const home = env.USERPROFILE || env.HOME || '';
  return home ? path.join(home, '.cortex') : '';
}

function resolveTokenPath(env = process.env) {
  const cortexHome = resolveCortexHome(env);
  if (cortexHome) return path.join(cortexHome, 'cortex.token');
  const home = env.USERPROFILE || env.HOME || '';
  return home ? path.join(home, '.cortex', 'cortex.token') : '';
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

function expectedLocalIdentity(baseUrl, env = process.env) {
  if (!isLocalBaseUrl(baseUrl)) return null;
  const cortexHome = resolveCortexHome(env);
  const identity = {
    home: cortexHome,
    token: resolveTokenPath(env),
    db: cortexHome ? path.join(cortexHome, 'cortex.db') : '',
    pid: cortexHome ? path.join(cortexHome, 'cortex.pid') : ''
  };
  try {
    const parsed = new URL(baseUrl);
    const port = parsed.port ? Number.parseInt(parsed.port, 10) : parsed.protocol === 'https:' ? 443 : 80;
    if (Number.isFinite(port)) identity.port = port;
  } catch (_) {}
  return identity;
}

function validateHealthIdentity(data, expectedIdentity) {
  if (!expectedIdentity || typeof expectedIdentity !== 'object') return true;
  if (!data || typeof data !== 'object') return false;
  const runtime = data.runtime;
  const stats = data.stats || {};

  if (!runtime || typeof runtime !== 'object') {
    return pathFieldMatches(stats.home, expectedIdentity.home);
  }

  if (Number.isFinite(expectedIdentity.port) && runtime.port !== expectedIdentity.port) {
    return false;
  }
  if (!pathFieldMatches(stats.home, expectedIdentity.home)) return false;
  if (!pathFieldMatches(runtime.db_path, expectedIdentity.db)) return false;
  if (!pathFieldMatches(runtime.token_path, expectedIdentity.token)) return false;
  if (!pathFieldMatches(runtime.pid_path, expectedIdentity.pid)) return false;
  return true;
}

function isCortexReadinessResponse(data) {
  if (!data || typeof data !== 'object') return false;
  if (typeof data.ready !== 'boolean') return false;
  if (data.status !== 'ready' && data.status !== 'starting') return false;
  return !!data.stats && typeof data.stats === 'object';
}

function isCortexHealthResponse(data) {
  if (!data || typeof data !== 'object') return false;
  const ok = data.ok;
  const status = typeof data.status === 'string' ? data.status.toLowerCase() : '';
  return ok === true || status === 'ok' || status === 'healthy' || status === 'ready';
}

function requestJson(url, headers, timeoutMs = 5000) {
  return new Promise((resolve) => {
    let target;
    try {
      target = new URL(url);
    } catch (e) {
      resolve({ ok: false, error: `Invalid URL: ${e.message}` });
      return;
    }

    const transport = target.protocol === 'https:' ? https : http;
    const request = transport.request(
      target,
      { method: 'GET', headers },
      (response) => {
        let body = '';
        response.setEncoding('utf8');
        response.on('data', (chunk) => {
          body += chunk;
        });
        response.on('end', () => {
          let data = null;
          try {
            data = body ? JSON.parse(body) : null;
          } catch (e) {
            resolve({ ok: false, statusCode: response.statusCode || 0, error: e.message, body });
            return;
          }
          resolve({ ok: true, statusCode: response.statusCode || 0, data, body });
        });
      }
    );
    request.setTimeout(timeoutMs, () => {
      request.destroy(new Error(`Health check timed out after ${timeoutMs}ms`));
    });
    request.on('error', (err) => {
      resolve({ ok: false, error: err.message });
    });
    request.end();
  });
}

async function healthCheck(baseUrl, timeoutMs = 5000, apiKey = '') {
  const normalizedBase = normalizeBaseUrl(baseUrl);
  const expectedIdentity = expectedLocalIdentity(normalizedBase);
  const headers = { 'X-Cortex-Request': 'true' };
  const normalizedApiKey = normalizeOption(apiKey);
  if (normalizedApiKey) headers.Authorization = `Bearer ${normalizedApiKey}`;

  const readiness = await requestJson(`${normalizedBase}/readiness`, headers, timeoutMs);
  if (
    readiness.ok &&
    readiness.statusCode >= 200 &&
    readiness.statusCode < 300 &&
    isCortexReadinessResponse(readiness.data)
  ) {
    if (!validateHealthIdentity(readiness.data, expectedIdentity)) {
      return {
        ok: false,
        status: 'identity_mismatch',
        error: 'daemon identity does not match local Cortex home paths'
      };
    }
    if (readiness.data.ready === true) {
      const stats = readiness.data.stats || {};
      return {
        ok: true,
        status: 'ready',
        memories: stats.memories,
        decisions: stats.decisions
      };
    }
    return { ok: false, status: 'starting', error: readiness.body || 'daemon reports starting' };
  }

  const health = await requestJson(`${normalizedBase}/health`, headers, timeoutMs);
  if (
    health.ok &&
    health.statusCode >= 200 &&
    health.statusCode < 300 &&
    isCortexHealthResponse(health.data)
  ) {
    if (!validateHealthIdentity(health.data, expectedIdentity)) {
      return {
        ok: false,
        status: 'identity_mismatch',
        error: 'daemon identity does not match local Cortex home paths'
      };
    }
    const stats = health.data.stats || {};
    return {
      ok: true,
      status: 'healthy',
      memories: stats.memories,
      decisions: stats.decisions
    };
  }

  return {
    ok: false,
    status: 'unavailable',
    error: health.error || readiness.error || health.body || readiness.body || 'unavailable'
  };
}

function buildStatusLine(health, routeState, agentName, routeUrl) {
  const parts = [];

  if (health.ok) {
    parts.push('READY');
    const counts = [];
    if (typeof health.memories === 'number') counts.push(`${health.memories} memories`);
    if (typeof health.decisions === 'number') counts.push(`${health.decisions} decisions`);
    if (counts.length > 0) parts.push(`(${counts.join(', ')})`);
  } else if (routeState === 'solo' || routeState === 'solo-standby') {
    parts.push('STANDBY');
  } else {
    parts.push('UNAVAILABLE');
  }

  if (routeState === 'team') {
    parts.push(`| Team @ ${normalizeOption(routeUrl).split('/')[2] || 'custom'}`);
  } else if (routeState === 'app') {
    parts.push('| App route');
  } else if (routeState === 'solo-standby') {
    parts.push('| Solo standby (CORTEX_APP_URL missing; no local daemon start is allowed)');
  } else {
    parts.push('| Solo mode');
    if (!health.ok) {
      parts.push(`| Open Cortex Control Center to initialize daemon for ${agentName}`);
    }
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
  const cortexUrl = process.env.CLAUDE_PLUGIN_OPTION_CORTEX_URL || '';
  const pluginAgent = normalizeOption(process.env.CORTEX_PLUGIN_AGENT) || 'claude-code';
  const route = resolveRoute({ cortexUrl }, process.env);
  console.error(`[cortex-plugin] SessionStart route: ${route.mode} (${route.reason})`);

  if (route.mode === 'solo-standby') {
    emitStatus(buildStatusLine({ ok: false }, route.mode, pluginAgent, route.url));
    return;
  }

  const health = await healthCheck(
    route.url || DEFAULT_LOCAL_BASE_URL,
    5000,
    process.env.CLAUDE_PLUGIN_OPTION_CORTEX_API_KEY || process.env.CORTEX_API_KEY || ''
  );
  emitStatus(buildStatusLine(health, route.mode, pluginAgent, route.url));
})().catch((err) => {
  console.error(`[cortex-plugin] SessionStart hook failed: ${err && err.stack ? err.stack : err}`);
  emitStatus('Brain: UNAVAILABLE | Cortex');
  process.exit(1);
});

module.exports = {
  resolveRoute,
  healthCheck,
  buildStatusLine
};
