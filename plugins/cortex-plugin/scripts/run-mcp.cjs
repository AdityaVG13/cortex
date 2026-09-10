#!/usr/bin/env node
// Local MCP stdio bridge: resolve the cortex binary and exec
// `cortex mcp --agent <name>` as a child process. No HTTP listener, token
// file, or remote URL is required. Lifetime is bound to this host connection.
const path = require('path');
const { spawn } = require('child_process');
const { resolveCortexBinary } = require(path.join(__dirname, 'resolve-binary.cjs'));

function normalizeOption(value) {
  return typeof value === 'string' ? value.trim() : '';
}

function isTruthy(value) {
  const normalized = normalizeOption(value).toLowerCase();
  return normalized === '1' || normalized === 'true' || normalized === 'yes' || normalized === 'on';
}

function crashLog(msg) {
  const home = process.env.USERPROFILE || process.env.HOME || '.';
  const cortexDir = path.join(home, '.cortex');
  const logPath = path.join(cortexDir, 'mcp-crash.log');
  const line = `[${new Date().toISOString()}] ${msg}\n`;
  try {
    require('fs').mkdirSync(cortexDir, { recursive: true });
    require('fs').appendFileSync(logPath, line);
  } catch (_) {}
  console.error(`[cortex-plugin] ${msg}`);
}

function resolveAgent(env = process.env) {
  return normalizeOption(env.CORTEX_PLUGIN_AGENT) || 'claude-code';
}

function buildMcpArgs(agent) {
  return ['mcp', '--agent', agent];
}

function resolveBinary(env = process.env) {
  const pluginData = normalizeOption(env.CLAUDE_PLUGIN_DATA);
  const binaryName = process.platform === 'win32' ? 'cortex.exe' : 'cortex';
  const resolved = resolveCortexBinary({
    pluginData,
    binaryName,
    allowBundled: true,
    rejectTempCandidates: true,
    env
  });
  if (!resolved.binaryPath || !require('fs').existsSync(resolved.binaryPath)) {
    throw new Error(`resolved binary is missing: ${resolved.binaryPath || '(empty)'}`);
  }
  return resolved;
}

function resolveBridgePlan(env = process.env) {
  const agent = resolveAgent(env);
  const { binaryPath, source } = resolveBinary(env);
  return {
    agent,
    binaryPath,
    source,
    args: buildMcpArgs(agent)
  };
}

function runMcpBridge(options = {}) {
  const env = options.env || process.env;
  const processRef = options.processRef || process;
  const log = options.log || console.error;
  const crashLogger = options.crashLogger || crashLog;
  const exit = options.exit || ((code) => process.exit(code));
  const spawnImpl = options.spawnImpl || spawn;
  const stdin = options.stdin || process.stdin;
  const stdout = options.stdout || process.stdout;
  const stderr = options.stderr || process.stderr;

  let plan;
  try {
    plan = resolveBridgePlan(env);
  } catch (err) {
    const message = `No local cortex binary: ${err && err.message ? err.message : err}`;
    crashLogger(message);
    if (options.exitOnFailure !== false) exit(1);
    return { ok: false, error: message };
  }

  log(
    `[cortex-plugin] MCP route: local stdio binary=${plan.binaryPath} source=${plan.source} args=${JSON.stringify(plan.args)}`
  );

  if (isTruthy(env.CORTEX_PLUGIN_DRY_RUN)) {
    log(
      `[cortex-plugin] Dry run complete. agent=${plan.agent} binary=${plan.binaryPath} source=${plan.source}`
    );
    if (options.exitOnDryRun !== false) exit(0);
    return { ok: true, dryRun: true, ...plan };
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

  const child = spawnImpl(plan.binaryPath, plan.args, {
    env,
    stdio: ['pipe', 'pipe', 'pipe']
  });

  stdin.pipe(child.stdin);
  child.stdout.pipe(stdout);
  child.stderr.pipe(stderr);

  child.on('error', (err) => {
    crashLogger(`MCP child failed to start: ${err && err.message ? err.message : err}`);
    if (options.exitOnFailure !== false) exit(1);
  });

  child.on('exit', (code, signal) => {
    if (signal) {
      try {
        processRef.kill(processRef.pid, signal);
      } catch (_) {}
      if (options.exitOnChildSignal !== false) exit(0);
      return;
    }
    if (options.exitOnChildExit !== false) exit(code == null ? 0 : code);
  });

  const forwardSignal = (signal) => {
    try {
      child.kill(signal);
    } catch (_) {}
  };
  for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
    try {
      processRef.on(signal, () => forwardSignal(signal));
    } catch (_) {}
  }

  return { ok: true, ...plan, child };
}

if (require.main === module) {
  runMcpBridge().catch((err) => {
    crashLog(`BRIDGE CRASH: ${err && err.stack ? err.stack : err}`);
    process.exit(1);
  });
}

module.exports = {
  normalizeOption,
  isTruthy,
  resolveAgent,
  buildMcpArgs,
  resolveBinary,
  resolveBridgePlan,
  runMcpBridge
};
