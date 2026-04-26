#!/usr/bin/env node
/**
 * Cortex Plugin - MCP Server Bridge
 *
 * Bridges MCP stdio directly to the already-running Cortex daemon over HTTP.
 * This entry point must not spawn cortex.exe. The Control Center/service owns
 * daemon lifecycle; the plugin only attaches.
 */

const path = require('path');
const fs = require('fs');
const http = require('http');
const https = require('https');
const readline = require('readline');

const DEFAULT_LOCAL_BASE_URL = 'http://127.0.0.1:7437';
const HEALTH_TIMEOUT_MS = 5000;
const REQUEST_TIMEOUT_MS = 10000;
const REQUEST_ATTEMPTS = 3;
const SESSION_HEARTBEAT_MS = 15000;
const SESSION_TTL_SECS = 7200;

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

function isLoopbackHost(hostname) {
  const normalized = normalizeOption(hostname).toLowerCase().replace(/^\[/, '').replace(/\]$/, '');
  return (
    normalized === 'localhost' ||
    normalized === '127.0.0.1' ||
    normalized === '::1' ||
    normalized === '0:0:0:0:0:0:0:1'
  );
}

function isLocalBaseUrl(baseUrl) {
  try {
    const parsed = new URL(baseUrl);
    return isLoopbackHost(parsed.hostname);
  } catch (_) {
    return false;
  }
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
  parsed.pathname = parsed.pathname.replace(/\/+$/, '');
  parsed.search = '';
  parsed.hash = '';
  return parsed.toString().replace(/\/+$/, '');
}

/**
 * Resolve the MCP route.
 *
 * Priority order:
 *   1. `CLAUDE_PLUGIN_OPTION_CORTEX_URL` (explicit user/team endpoint).
 *   2. `CORTEX_DEV_PREFER_APP=1` requires `CORTEX_APP_URL`.
 *   3. `CORTEX_APP_URL` emitted by an app-managed daemon.
 *   4. Local HTTP attach to `http://127.0.0.1:7437`.
 *
 * No route permits local daemon spawn from the plugin.
 */
function resolveRoute(config, env = process.env) {
  const explicitUrl = normalizeOption(config.cortexUrl);
  const devPreferApp = isTruthy(env.CORTEX_DEV_PREFER_APP);
  const appUrl = normalizeOption(env.CORTEX_APP_URL);

  if (explicitUrl) {
    return {
      mode: 'remote',
      url: explicitUrl,
      reason: 'explicit plugin URL',
      spawnAllowed: false
    };
  }

  if (devPreferApp && !appUrl) {
    return {
      mode: 'fail',
      url: '',
      reason: 'CORTEX_DEV_PREFER_APP=1 but CORTEX_APP_URL is not set',
      spawnAllowed: false,
      error: true
    };
  }

  if (appUrl) {
    return {
      mode: 'remote',
      url: appUrl,
      reason: devPreferApp ? 'dev prefer app (CORTEX_APP_URL)' : 'app route',
      spawnAllowed: false
    };
  }

  return {
    mode: 'local',
    url: DEFAULT_LOCAL_BASE_URL,
    reason: 'local HTTP attach-only',
    spawnAllowed: false
  };
}

function resolveOwnerMode(route) {
  if (route.mode === 'local') return 'solo-service';
  if (route.mode === 'fail') return 'unresolved';
  return route.reason === 'explicit plugin URL' ? 'team' : 'app';
}

function buildMcpArgs(route, pluginAgent) {
  return [
    'http-proxy',
    '--agent',
    pluginAgent,
    '--url',
    route.url || DEFAULT_LOCAL_BASE_URL
  ];
}

function buildChildEnv(baseEnv, route, pluginAgent, ownerMode, parentPid, cortexApiKey) {
  const proxyEnv = {
    ...baseEnv,
    CORTEX_DAEMON_OWNER_KIND: 'plugin',
    CORTEX_DAEMON_OWNER_SOURCE: 'claude-plugin',
    CORTEX_DAEMON_OWNER_AGENT: pluginAgent,
    CORTEX_DAEMON_OWNER_MODE: ownerMode,
    CORTEX_DAEMON_OWNER_LOCAL_SPAWN: '0',
    CORTEX_DAEMON_OWNER_PARENT_PID: String(parentPid)
  };
  delete proxyEnv.CORTEX_API_KEY;
  const normalizedApiKey = normalizeOption(cortexApiKey);
  if (normalizedApiKey.length > 0) {
    proxyEnv.CORTEX_API_KEY = normalizedApiKey;
  }
  return proxyEnv;
}

function resolveTokenPath(env = process.env) {
  const explicit = normalizeOption(env.CORTEX_TOKEN_PATH);
  if (explicit) return explicit;
  const cortexHome = normalizeOption(env.CORTEX_HOME);
  if (cortexHome) return path.join(cortexHome, 'cortex.token');
  const home = env.USERPROFILE || env.HOME || '';
  return home ? path.join(home, '.cortex', 'cortex.token') : '';
}

function resolveCortexHome(env = process.env) {
  const explicit = normalizeOption(env.CORTEX_HOME);
  if (explicit) return explicit;
  const home = env.USERPROFILE || env.HOME || '';
  return home ? path.join(home, '.cortex') : '';
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

function readLocalAuthToken(env = process.env) {
  const tokenPath = resolveTokenPath(env);
  if (!tokenPath) return '';
  try {
    return fs.readFileSync(tokenPath, 'utf8').trim();
  } catch (_) {
    return '';
  }
}

function buildAuthHeader(baseUrl, apiKey, env = process.env) {
  const normalizedApiKey = normalizeOption(apiKey);
  if (normalizedApiKey) return `Bearer ${normalizedApiKey}`;
  if (!isLocalBaseUrl(baseUrl)) return '';
  const localToken = readLocalAuthToken(env);
  return localToken ? `Bearer ${localToken}` : '';
}

function defaultHttpRequest(request) {
  return new Promise((resolve, reject) => {
    let target;
    try {
      target = new URL(request.url);
    } catch (err) {
      reject(err);
      return;
    }

    const transport = target.protocol === 'https:' ? https : http;
    const req = transport.request(
      target,
      {
        method: request.method || 'GET',
        headers: request.headers || {}
      },
      (response) => {
        let body = '';
        response.setEncoding('utf8');
        response.on('data', (chunk) => {
          body += chunk;
        });
        response.on('end', () => {
          resolve({ statusCode: response.statusCode || 0, body });
        });
      }
    );

    req.setTimeout(request.timeoutMs || REQUEST_TIMEOUT_MS, () => {
      req.destroy(new Error(`HTTP request timed out after ${request.timeoutMs || REQUEST_TIMEOUT_MS}ms`));
    });
    req.on('error', reject);
    if (request.body) req.write(request.body);
    req.end();
  });
}

async function httpRequest(request, options = {}) {
  const requestImpl = options.requestImpl || defaultHttpRequest;
  return requestImpl(request);
}

function isCortexReadinessResponse(data) {
  if (!data || typeof data !== 'object') return false;
  if (typeof data.ready !== 'boolean') return false;
  if (data.status !== 'ready' && data.status !== 'starting') return false;
  return !!data.stats && typeof data.stats === 'object';
}

function isCortexHealthResponse(data) {
  if (!data || typeof data !== 'object') return false;
  const status = typeof data.status === 'string' ? data.status.toLowerCase() : '';
  return data.ok === true || status === 'ok' || status === 'healthy' || status === 'ready';
}

function tryParseJson(body) {
  try {
    return body ? JSON.parse(body) : null;
  } catch (_) {
    return null;
  }
}

async function healthCheck(baseUrl, apiKey = '', options = {}) {
  const normalizedBase = normalizeBaseUrl(baseUrl);
  const expectedIdentity = expectedLocalIdentity(normalizedBase, options.env || process.env);
  const headers = { 'X-Cortex-Request': 'true' };
  const normalizedApiKey = normalizeOption(apiKey);
  if (normalizedApiKey) {
    headers.Authorization = `Bearer ${normalizedApiKey}`;
  }

  const readiness = await httpRequest(
    {
      method: 'GET',
      url: `${normalizedBase}/readiness`,
      headers,
      timeoutMs: options.timeoutMs || HEALTH_TIMEOUT_MS
    },
    options
  ).catch((err) => ({ statusCode: 0, body: '', error: err.message }));

  const readinessData = tryParseJson(readiness.body);
  if (
    readiness.statusCode >= 200 &&
    readiness.statusCode < 300 &&
    isCortexReadinessResponse(readinessData) &&
    validateHealthIdentity(readinessData, expectedIdentity)
  ) {
    if (readinessData.ready === true) {
      return {
        ok: true,
        status: 'ready',
        body: readinessData
      };
    }
    return {
      ok: false,
      status: 'starting',
      error: readiness.body || 'daemon reports starting'
    };
  }

  const health = await httpRequest(
    {
      method: 'GET',
      url: `${normalizedBase}/health`,
      headers,
      timeoutMs: options.timeoutMs || HEALTH_TIMEOUT_MS
    },
    options
  ).catch((err) => ({ statusCode: 0, body: '', error: err.message }));

  const healthData = tryParseJson(health.body);
  if (
    health.statusCode >= 200 &&
    health.statusCode < 300 &&
    isCortexHealthResponse(healthData) &&
    validateHealthIdentity(healthData, expectedIdentity)
  ) {
    return {
      ok: true,
      status: 'healthy',
      body: healthData
    };
  }

  return {
    ok: false,
    status: 'unavailable',
    error:
      readiness.error ||
      health.error ||
      health.body ||
      readiness.body ||
      `readiness HTTP ${readiness.statusCode}; health HTTP ${health.statusCode}`
  };
}

function buildProxyHeaders(baseUrl, apiKey, agent, model, env = process.env) {
  const headers = {
    'Content-Type': 'application/json',
    'X-Cortex-Request': 'true',
    'X-Source-Agent': agent
  };
  const authHeader = buildAuthHeader(baseUrl, apiKey, env);
  if (authHeader) headers.Authorization = authHeader;
  const normalizedModel = normalizeOption(model);
  if (normalizedModel) headers['X-Source-Model'] = normalizedModel;
  return headers;
}

async function postJson(baseUrl, pathName, payload, context, options = {}) {
  const normalizedBase = normalizeBaseUrl(baseUrl);
  return httpRequest(
    {
      method: 'POST',
      url: `${normalizedBase}${pathName}`,
      headers: buildProxyHeaders(
        normalizedBase,
        context.apiKey,
        context.agent,
        context.model,
        context.env
      ),
      body: payload,
      timeoutMs: options.timeoutMs || REQUEST_TIMEOUT_MS
    },
    options
  );
}

async function sessionStart(baseUrl, context, options = {}) {
  const payload = JSON.stringify({
    agent: context.agent,
    ttl: SESSION_TTL_SECS,
    description: context.model ? `MCP session - ${context.model}` : 'MCP session'
  });
  try {
    const response = await postJson(baseUrl, '/session/start', payload, context, options);
    return response.statusCode >= 200 && response.statusCode < 300;
  } catch (_) {
    return false;
  }
}

async function sessionHeartbeat(baseUrl, context, options = {}) {
  const payload = JSON.stringify({
    agent: context.agent,
    description: context.model ? `MCP session - ${context.model}` : 'MCP session'
  });
  try {
    const response = await postJson(baseUrl, '/session/heartbeat', payload, context, {
      ...options,
      timeoutMs: 8000
    });
    return response.statusCode >= 200 && response.statusCode < 300;
  } catch (_) {
    return false;
  }
}

async function sessionEnd(baseUrl, context, options = {}) {
  try {
    await postJson(baseUrl, '/session/end', JSON.stringify({ agent: context.agent }), context, {
      ...options,
      timeoutMs: 8000
    });
  } catch (_) {}
}

function makeJsonRpcError(id, code, message) {
  return {
    jsonrpc: '2.0',
    error: { code, message },
    id: id === undefined ? null : id
  };
}

async function writeLine(output, line) {
  if (!output || typeof output.write !== 'function') return;
  await new Promise((resolve, reject) => {
    output.write(`${line}\n`, (err) => {
      if (err) reject(err);
      else resolve();
    });
  });
}

async function forwardMcpMessage(line, context, options = {}) {
  const trimmed = normalizeOption(line);
  if (!trimmed) return { ok: true, skipped: true };

  let msg;
  try {
    msg = JSON.parse(trimmed);
  } catch (_) {
    await writeLine(
      context.stdout,
      JSON.stringify(makeJsonRpcError(null, -32700, 'Parse error'))
    );
    return { ok: false, parseError: true };
  }

  const hasId = Object.prototype.hasOwnProperty.call(msg, 'id');
  const id = hasId ? msg.id : null;
  let lastErr = '';

  for (let attempt = 1; attempt <= REQUEST_ATTEMPTS; attempt++) {
    try {
      const response = await postJson(
        context.baseUrl,
        '/mcp-rpc',
        trimmed,
        context,
        options
      );
      const responseBody = normalizeOption(response.body);

      if (response.statusCode >= 200 && response.statusCode < 300) {
        if (hasId) {
          if (!responseBody) {
            lastErr = 'daemon returned an empty response body';
            break;
          }
          if (!tryParseJson(responseBody)) {
            lastErr = 'daemon returned invalid JSON-RPC';
            break;
          }
          await writeLine(context.stdout, responseBody);
        }
        return { ok: true, statusCode: response.statusCode };
      }

      if (hasId && responseBody && tryParseJson(responseBody)) {
        await writeLine(context.stdout, responseBody);
        return { ok: false, statusCode: response.statusCode };
      }

      lastErr = responseBody || `HTTP ${response.statusCode}`;
    } catch (err) {
      lastErr = err && err.message ? err.message : String(err);
    }

    if (attempt < REQUEST_ATTEMPTS) {
      await new Promise((resolve) => setTimeout(resolve, 150 * attempt));
    }
  }

  if (hasId) {
    await writeLine(
      context.stdout,
      JSON.stringify(makeJsonRpcError(id, -32603, `Daemon unavailable: ${lastErr}`))
    );
  }
  return { ok: false, error: lastErr };
}

async function runProxyLoop(context, options = {}) {
  const readlineImpl = options.readlineImpl || readline;
  const input = options.stdin || process.stdin;
  const rl = readlineImpl.createInterface({ input, crlfDelay: Infinity });

  for await (const line of rl) {
    await forwardMcpMessage(line, context, options);
  }
}

async function runMcpBridge(options = {}) {
  const env = options.env || process.env;
  const processRef = options.processRef || process;
  const log = options.log || console.error;
  const crashLogger = options.crashLogger || crashLog;
  const exit = options.exit || ((code) => process.exit(code));
  const stdout = options.stdout || process.stdout;

  const cortexUrl = env.CLAUDE_PLUGIN_OPTION_CORTEX_URL || '';
  const cortexApiKey = env.CLAUDE_PLUGIN_OPTION_CORTEX_API_KEY || env.CORTEX_API_KEY || '';
  const pluginAgent = normalizeOption(env.CORTEX_PLUGIN_AGENT) || 'claude-code';
  const agentModel = normalizeOption(env.CORTEX_AGENT_MODEL || env.CORTEX_MODEL);
  const dryRun = isTruthy(env.CORTEX_PLUGIN_DRY_RUN);
  const route = resolveRoute({ cortexUrl }, env);
  const ownerMode = resolveOwnerMode(route);
  const args = buildMcpArgs(route, pluginAgent);
  const proxyEnv = buildChildEnv(env, route, pluginAgent, ownerMode, processRef.pid, cortexApiKey);

  if (route.mode === 'fail') {
    log(`[cortex-plugin] MCP route: FAIL (${route.reason})`);
    if (dryRun) {
      log(
        `[cortex-plugin] Dry run complete. agent=${pluginAgent} mode=fail url=(none) spawnAllowed=${route.spawnAllowed}`
      );
      if (options.exitOnDryRun !== false) exit(0);
      return { ok: false, dryRun: true, route, pluginAgent };
    }
    crashLogger(`ROUTE FAILURE: ${route.reason}`);
    if (options.exitOnFailure !== false) exit(1);
    return { ok: false, route, pluginAgent };
  }

  let baseUrl;
  try {
    baseUrl = normalizeBaseUrl(route.url || DEFAULT_LOCAL_BASE_URL);
  } catch (error) {
    crashLogger(`ROUTE FAILURE: ${error && error.message ? error.message : error}`);
    if (options.exitOnFailure !== false) exit(1);
    return { ok: false, route, pluginAgent };
  }

  if (!isLocalBaseUrl(baseUrl) && !normalizeOption(cortexApiKey)) {
    const message = `Remote Cortex target '${baseUrl}' requires CLAUDE_PLUGIN_OPTION_CORTEX_API_KEY`;
    if (dryRun) {
      log(`[cortex-plugin] MCP route: FAIL (${message})`);
      if (options.exitOnDryRun !== false) exit(0);
      return { ok: false, dryRun: true, route, pluginAgent, baseUrl };
    }
    crashLogger(message);
    if (options.exitOnFailure !== false) exit(1);
    return { ok: false, route, pluginAgent, baseUrl };
  }

  log(
    `[cortex-plugin] MCP route: ${route.mode} (${route.reason}) url=${baseUrl} spawnAllowed=false`
  );
  log(`[cortex-plugin] MCP bridge: node stdio -> HTTP /mcp-rpc (no cortex child process)`);

  if (dryRun) {
    log(
      `[cortex-plugin] Dry run complete. agent=${pluginAgent} mode=${route.mode} url=${baseUrl} spawnAllowed=false`
    );
    if (options.exitOnDryRun !== false) exit(0);
    return { ok: true, dryRun: true, route, pluginAgent, baseUrl, args, proxyEnv };
  }

  if (options.registerProcessHandlers !== false) {
    processRef.on('uncaughtException', (err) => {
      crashLogger(`BRIDGE CRASH: ${err && err.stack ? err.stack : err}`);
      exit(1);
    });
    processRef.on('unhandledRejection', (reason) => {
      crashLogger(`BRIDGE REJECTION: ${reason && reason.stack ? reason.stack : reason}`);
      exit(1);
    });
  }

  const health = await healthCheck(baseUrl, cortexApiKey, { ...options, env: proxyEnv });
  if (!health.ok) {
    const message =
      route.mode === 'local'
        ? `APP_INIT_REQUIRED: Cortex daemon is not ready at ${baseUrl}. Open Cortex Control Center, initialize once, then retry. ${health.error || ''}`.trim()
        : `Cortex daemon is not ready at ${baseUrl}: ${health.error || 'unavailable'}`;
    crashLogger(message);
    if (options.exitOnFailure !== false) exit(1);
    return { ok: false, route, pluginAgent, baseUrl, health };
  }

  const context = {
    baseUrl,
    apiKey: cortexApiKey,
    agent: pluginAgent,
    model: agentModel,
    env: proxyEnv,
    stdout
  };

  await sessionStart(baseUrl, context, options);
  const setIntervalImpl = options.setIntervalImpl || setInterval;
  const clearIntervalImpl = options.clearIntervalImpl || clearInterval;
  const heartbeat = setIntervalImpl(() => {
    sessionHeartbeat(baseUrl, context, options).catch(() => {});
  }, SESSION_HEARTBEAT_MS);

  try {
    await runProxyLoop(context, options);
  } finally {
    clearIntervalImpl(heartbeat);
    await sessionEnd(baseUrl, context, options);
  }

  return { ok: true, route, pluginAgent, baseUrl };
}

if (require.main === module) {
  runMcpBridge().catch((err) => {
    crashLog(`BRIDGE CRASH: ${err && err.stack ? err.stack : err}`);
    process.exit(1);
  });
}

module.exports = {
  DEFAULT_LOCAL_BASE_URL,
  normalizeOption,
  isTruthy,
  isLocalBaseUrl,
  resolveRoute,
  buildMcpArgs,
  resolveOwnerMode,
  buildChildEnv,
  resolveTokenPath,
  readLocalAuthToken,
  buildAuthHeader,
  healthCheck,
  forwardMcpMessage,
  runMcpBridge
};
