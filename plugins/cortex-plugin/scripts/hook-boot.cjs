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

function isTruthy(value) {
  const normalized = normalizeOption(value).toLowerCase();
  return normalized === '1' || normalized === 'true' || normalized === 'yes' || normalized === 'on';
}

/**
 * Resolve route mode for the SessionStart status hook. Mirrors the semantic
 * priorities in `run-mcp.cjs` but uses the UI-facing mode names
 * (`team` / `app` / `solo` / `solo-standby`) so the status line stays
 * intelligible to operators.
 *
 *   - explicit plugin URL  → team
 *   - dev prefer app (+URL) → app
 *   - dev prefer app, no URL → solo-standby (no daemon will come up)
 *   - CORTEX_APP_URL       → app
 *   - local-disabled, no URL → solo-standby
 *   - otherwise             → solo
 */
function resolveRoute(config, env = process.env) {
  const explicitUrl = normalizeOption(config.cortexUrl);
  const devPreferApp = isTruthy(env.CORTEX_DEV_PREFER_APP);
  const devAppUrl = normalizeOption(env.CORTEX_DEV_APP_URL);
  const appUrl = normalizeOption(env.CORTEX_APP_URL);
  const disableLocalSpawn = isTruthy(env.CORTEX_DEV_DISABLE_LOCAL_SPAWN);
  const allowLocalSpawn = isTruthy(env.CORTEX_PLUGIN_ALLOW_LOCAL_SPAWN);

  if (explicitUrl) {
    return { mode: 'team', url: explicitUrl, reason: 'explicit plugin URL' };
  }

  if (devPreferApp) {
    const chosenUrl = devAppUrl || appUrl;
    if (chosenUrl) {
      return {
        mode: 'app',
        url: chosenUrl,
        reason: devAppUrl ? 'dev prefer app (CORTEX_DEV_APP_URL)' : 'dev prefer app (CORTEX_APP_URL)'
      };
    }
    return {
      mode: 'solo-standby',
      url: '',
      reason:
        'CORTEX_DEV_PREFER_APP=1 but neither CORTEX_DEV_APP_URL nor CORTEX_APP_URL is set — plugin will not spawn local daemon'
    };
  }

  if (appUrl) {
    return { mode: 'app', url: appUrl, reason: 'app route' };
  }

  if (disableLocalSpawn && !allowLocalSpawn) {
    return {
      mode: 'solo-standby',
      url: '',
      reason:
        'CORTEX_DEV_DISABLE_LOCAL_SPAWN=1 and no remote URL is set; set CORTEX_PLUGIN_ALLOW_LOCAL_SPAWN=1 to override'
    };
  }

  return { mode: 'solo', url: '', reason: 'local attach-only' };
}

const PLATFORM = process.platform;
const binaryName = PLATFORM === 'win32' ? 'cortex.exe' : 'cortex';
const DEFAULT_DAEMON_PORT = 7437;

const cortexUrl = process.env.CLAUDE_PLUGIN_OPTION_CORTEX_URL || '';
const pluginAgent = normalizeOption(process.env.CORTEX_PLUGIN_AGENT) || 'claude-code';
const route = resolveRoute({ cortexUrl }, process.env);
console.error(`[cortex-plugin] SessionStart route: ${route.mode} (${route.reason})`);
const allowBundledBinary = normalizeOption(process.env.CORTEX_PLUGIN_ALLOW_BUNDLED_BINARY)
  .toLowerCase();
const allowBundled =
  allowBundledBinary === '1' ||
  allowBundledBinary === 'true' ||
  allowBundledBinary === 'yes' ||
  allowBundledBinary === 'on';

let binaryPath = '';
let binarySource = '';
let binaryResolutionError = '';
try {
  const resolved = resolveCortexBinary({
    pluginData: PLUGIN_DATA,
    binaryName,
    ensureBundled: () => require('./prepare-runtime.cjs'),
    allowBundled: route.mode !== 'solo' || allowBundled,
    rejectTempCandidates: route.mode === 'solo' && !allowBundled
  });
  binaryPath = resolved.binaryPath;
  binarySource = resolved.source;
  console.error(`[cortex-plugin] SessionStart binary: ${binaryPath} (${binarySource})`);
} catch (error) {
  binaryResolutionError = error && error.message ? error.message : String(error);
  console.error(`[cortex-plugin] SessionStart binary resolution blocked: ${binaryResolutionError}`);
}

function isCortexReadinessResponse(data) {
  if (!data || typeof data !== 'object') return false;
  if (typeof data.ready !== 'boolean') return false;
  if (data.status !== 'ready' && data.status !== 'starting') return false;
  if (!data.stats || typeof data.stats !== 'object') return false;
  // Backward-compat: accept `status + stats` without `runtime` (older daemons
  // pre-v0.5.0 that predate the runtime-identity block). `validateHealthIdentity`
  // still fails closed when the caller requested an identity match.
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

  // Backward-compat: older daemons (pre-v0.5.0) omitted the `runtime` block.
  // When no runtime identity fields exist to compare against, skip identity
  // enforcement rather than hard-failing local health — the port + stats
  // check in the outer readiness flow still guards against obvious mismatches.
  const hasRuntimeIdentity = runtime && typeof runtime === 'object';
  const expectsPathFields =
    typeof expectedIdentity.db === 'string' && expectedIdentity.db.trim().length > 0 ||
    typeof expectedIdentity.token === 'string' && expectedIdentity.token.trim().length > 0 ||
    typeof expectedIdentity.pid === 'string' && expectedIdentity.pid.trim().length > 0;

  if (!hasRuntimeIdentity) {
    if (!expectsPathFields && !Number.isFinite(expectedIdentity.port)) {
      return true;
    }
    // Caller wants identity enforcement but the daemon did not emit runtime.
    // Fall through to soft-accept home match only; port/path checks skipped.
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

function isCortexHealthResponse(data) {
  if (!data || typeof data !== 'object') return false;
  const ok = data.ok;
  const status = typeof data.status === 'string' ? data.status.toLowerCase() : '';
  return ok === true || status === 'ok' || status === 'healthy' || status === 'ready';
}

function healthCheck(url, timeoutMs = 5000, expectedIdentity = null, apiKey = '') {
  return new Promise((resolve) => {
    let baseTarget;
    try {
      baseTarget = new URL(url.replace(/\/+$/, ''));
    } catch (e) {
      resolve({ ok: false, error: `Invalid health URL: ${e.message}` });
      return;
    }

    const transport = baseTarget.protocol === 'https:' ? https : http;
    const normalizedApiKey = normalizeOption(apiKey);
    const baseHeaders = { 'X-Cortex-Request': 'true' };
    if (normalizedApiKey) {
      baseHeaders.Authorization = `Bearer ${normalizedApiKey}`;
    }

    function requestEndpoint(path, onSuccess) {
      return new Promise((innerResolve) => {
        const target = new URL(path, baseTarget);
        const request = transport.request(
          target,
          { method: 'GET', headers: baseHeaders },
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
                innerResolve({ ok: false, error: `Invalid ${path} response: ${e.message}` });
                return;
              }
              onSuccess(response.statusCode || 0, data, body, innerResolve);
            });
          }
        );
        request.setTimeout(timeoutMs, () => {
          request.destroy(new Error(`Health check timed out after ${timeoutMs}ms`));
        });
        request.on('error', (err) => {
          innerResolve({ ok: false, error: err.message });
        });
        request.end();
      });
    }

    requestEndpoint('/readiness', (statusCode, data, body, done) => {
      if (
        statusCode >= 200 &&
        statusCode < 300 &&
        isCortexReadinessResponse(data) &&
        validateHealthIdentity(data, expectedIdentity)
      ) {
        if (data.ready === true) {
          const stats = data.stats || {};
          done({
            ok: true,
            status: 'ready',
            memories: stats.memories,
            decisions: stats.decisions
          });
          return;
        }
        done({ ok: false, error: body.trim() || `HTTP ${statusCode}` });
        return;
      }

      requestEndpoint('/health', (healthStatus, healthData, healthBody, healthDone) => {
        if (
          healthStatus >= 200 &&
          healthStatus < 300 &&
          isCortexHealthResponse(healthData) &&
          (!expectedIdentity || validateHealthIdentity(healthData, expectedIdentity))
        ) {
          const stats = healthData?.stats || {};
          healthDone({
            ok: true,
            status: 'healthy',
            memories: stats.memories,
            decisions: stats.decisions
          });
          return;
        }
        healthDone({ ok: false, error: healthBody.trim() || `HTTP ${healthStatus}` });
      }).then(resolve);
    }).then(resolve);
  });
}

async function getLocalHealth() {
  if (!binaryPath) {
    return {
      ok: false,
      error:
        binaryResolutionError ||
        'No app-managed Cortex binary available for local attach mode.'
    };
  }

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

function buildStatusLine(health, routeState, agentName) {
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
    parts.push(`| Team @ ${normalizeOption(cortexUrl).split('/')[2] || 'custom'}`);
  } else if (routeState === 'app') {
    parts.push('| App route');
  } else if (routeState === 'solo-standby') {
    parts.push('| Solo standby (no URL set; local spawn blocked by policy)');
    parts.push(`| Set CORTEX_DEV_APP_URL / CORTEX_APP_URL or unset CORTEX_DEV_DISABLE_LOCAL_SPAWN for ${agentName}`);
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
  if (route.mode === 'team' || route.mode === 'app') {
    const health = await healthCheck(
      route.url,
      5000,
      null,
      process.env.CLAUDE_PLUGIN_OPTION_CORTEX_API_KEY || ''
    );
    emitStatus(buildStatusLine(health, route.mode, pluginAgent));
    return;
  }

  if (route.mode === 'solo-standby') {
    // Standby by policy — no URL, local spawn blocked. Emit status without probing.
    emitStatus(buildStatusLine({ ok: false }, 'solo-standby', pluginAgent));
    return;
  }

  const health = await getLocalHealth();
  emitStatus(buildStatusLine(health, 'solo', pluginAgent));
})().catch((err) => {
  console.error(`[cortex-plugin] SessionStart hook failed: ${err && err.stack ? err.stack : err}`);
  emitStatus('Brain: UNAVAILABLE | Cortex');
  process.exit(1);
});
