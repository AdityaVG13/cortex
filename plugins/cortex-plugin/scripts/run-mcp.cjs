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
const { resolveCortexBinary } = require('./resolve-binary.cjs');

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

function resolveRoute(config, env = process.env) {
  const explicitUrl = normalizeOption(config.cortexUrl);
  const appUrl = normalizeOption(env.CORTEX_APP_URL);

  if (explicitUrl) {
    return { mode: 'remote', url: explicitUrl, reason: 'explicit plugin URL' };
  }

  if (appUrl) {
    return { mode: 'remote', url: appUrl, reason: 'app route' };
  }

  return { mode: 'local', url: '', reason: 'local service-first' };
}

function buildMcpArgs(route, pluginAgent) {
  const args = ['plugin', 'mcp', '--agent', pluginAgent];
  if (route.mode === 'remote') {
    args.push('--url', route.url);
  }
  return args;
}

function resolveOwnerMode(route) {
  if (route.mode === 'local') {
    return 'solo-service';
  }
  return route.reason === 'explicit plugin URL' ? 'team' : 'app';
}

function buildChildEnv(baseEnv, route, pluginAgent, ownerMode, parentPid, cortexApiKey) {
  const childEnv = {
    ...baseEnv,
    CORTEX_DAEMON_OWNER_KIND: 'plugin',
    CORTEX_DAEMON_OWNER_SOURCE: 'claude-plugin',
    CORTEX_DAEMON_OWNER_AGENT: pluginAgent,
    CORTEX_DAEMON_OWNER_MODE: ownerMode,
    CORTEX_DAEMON_OWNER_LOCAL_SPAWN: '0',
    CORTEX_DAEMON_OWNER_PARENT_PID: String(parentPid)
  };
  delete childEnv.CORTEX_API_KEY;
  if (route.mode === 'local') {
    const userHome = baseEnv.USERPROFILE || baseEnv.HOME || '';
    if (userHome) {
      childEnv.CORTEX_HOME = path.join(userHome, '.cortex');
    }
    delete childEnv.CORTEX_DB;
  } else {
    const normalizedApiKey = normalizeOption(cortexApiKey);
    if (normalizedApiKey.length > 0) {
      childEnv.CORTEX_API_KEY = normalizedApiKey;
    }
  }
  return childEnv;
}

function resolveBinary(route, binaryName, env, resolveCortexBinaryImpl, ensureBundled) {
  const allowBundledBinary = isTruthy(env.CORTEX_PLUGIN_ALLOW_BUNDLED_BINARY);
  const requireAppBinary = isTruthy(env.CORTEX_PLUGIN_REQUIRE_APP_BINARY);
  return resolveCortexBinaryImpl({
    pluginData: env.CLAUDE_PLUGIN_DATA || PLUGIN_DATA,
    binaryName,
    ensureBundled,
    allowBundled: route.mode !== 'local' || (!requireAppBinary || allowBundledBinary),
    rejectTempCandidates: route.mode === 'local'
  });
}

function runMcpBridge(options = {}) {
  const env = options.env || process.env;
  const platform = options.platform || process.platform;
  const processRef = options.processRef || process;
  const spawnImpl = options.spawnImpl || spawn;
  const resolveCortexBinaryImpl = options.resolveCortexBinaryImpl || resolveCortexBinary;
  const ensureBundled = options.ensureBundled || (() => require('./prepare-runtime.cjs'));
  const log = options.log || console.error;
  const crashLogger = options.crashLogger || crashLog;
  const exit = options.exit || ((code) => process.exit(code));

  const binaryName = platform === 'win32' ? 'cortex.exe' : 'cortex';
  const cortexUrl = env.CLAUDE_PLUGIN_OPTION_CORTEX_URL || '';
  const cortexApiKey = env.CLAUDE_PLUGIN_OPTION_CORTEX_API_KEY || '';
  const pluginAgent = normalizeOption(env.CORTEX_PLUGIN_AGENT) || 'claude-code';
  const dryRun = isTruthy(env.CORTEX_PLUGIN_DRY_RUN);
  const route = resolveRoute({ cortexUrl }, env);

  let binaryPath = '';
  let binarySource = '';
  try {
    const resolved = resolveBinary(
      route,
      binaryName,
      env,
      resolveCortexBinaryImpl,
      ensureBundled
    );
    binaryPath = resolved.binaryPath;
    binarySource = resolved.source;
  } catch (error) {
    crashLogger(`BINARY RESOLUTION FAILED: ${error && error.message ? error.message : error}`);
    if (route.mode === 'local') {
      log(
        '[cortex-plugin] Local mode could not resolve a safe Cortex binary. Start Control Center or set CORTEX_APP_BINARY. ' +
          'Plugin-bundled fallback is allowed by default, but temporary runtime locations are blocked.'
      );
    }
    if (options.exitOnFailure !== false) {
      exit(1);
    }
    return { ok: false, route, pluginAgent };
  }

  const args = buildMcpArgs(route, pluginAgent);
  const ownerMode = resolveOwnerMode(route);
  const childEnv = buildChildEnv(env, route, pluginAgent, ownerMode, processRef.pid, cortexApiKey);

  log(`[cortex-plugin] MCP route: ${route.mode} (${route.reason})`);
  log(`[cortex-plugin] Cortex binary: ${binaryPath} (${binarySource})`);
  if (route.mode === 'local') {
    log(
      `[cortex-plugin] Local attach-only mode active for ${pluginAgent}. ` +
        'If the daemon is offline you will receive APP_INIT_REQUIRED. Open Cortex Control Center, initialize once, then retry.'
    );
  }

  if (dryRun) {
    log(
      `[cortex-plugin] Dry run complete. agent=${pluginAgent} mode=${route.mode} url=${route.url || '(none)'}`
    );
    if (options.exitOnDryRun !== false) {
      exit(0);
    }
    return { ok: true, dryRun: true, route, pluginAgent, binaryPath, binarySource, args, childEnv };
  }

  const child = spawnImpl(binaryPath, args, { stdio: 'inherit', env: childEnv });

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

  child.on('error', (err) => {
    crashLogger(`SPAWN FAILED: ${err.message} (binary: ${binaryPath})`);
    exit(1);
  });

  child.on('exit', (code, signal) => {
    if (signal) {
      crashLogger(`MCP server killed by signal ${signal}`);
      exit(1);
      return;
    }
    if (code !== 0) {
      crashLogger(`MCP server exited with code ${code}`);
    }
    exit(code || 0);
  });

  return { ok: true, route, pluginAgent, binaryPath, binarySource, args, childEnv, child };
}

if (require.main === module) {
  runMcpBridge();
}

module.exports = {
  normalizeOption,
  isTruthy,
  resolveRoute,
  buildMcpArgs,
  resolveOwnerMode,
  buildChildEnv,
  resolveBinary,
  runMcpBridge
};
