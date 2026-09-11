#!/usr/bin/env node
// SessionStart: spawn local `cortex hook-boot` with the host stdin payload
// and relay its hook JSON. Never probes HTTP. A missing binary is
// UNAVAILABLE, not an empty brain.
const path = require('path');
const { spawnSync } = require('child_process');
const { resolveCortexBinary } = require(path.join(__dirname, 'resolve-binary.cjs'));

function normalizeOption(value) {
  return typeof value === 'string' ? value.trim() : '';
}

function unavailable(reason) {
  return {
    hookSpecificOutput: {
      hookEventName: 'SessionStart',
      additionalContext:
        'Cortex: memory unavailable for SessionStart. Do not assume the brain is empty.'
    },
    cortex: {
      event: null,
      decision: 'UNAVAILABLE',
      reason,
      overflow: false,
      presence: 'unknown',
      automatic_capture: false,
      counted: false
    }
  };
}

function resolveAgent(env = process.env) {
  return normalizeOption(env.CORTEX_PLUGIN_AGENT) || 'claude-code';
}

function buildBootArgs(agent) {
  return ['hook-boot', '--agent', agent];
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

function readStdin() {
  try {
    return require('fs').readFileSync(0, 'utf8');
  } catch (_) {
    return '';
  }
}

function runHookBoot(options = {}) {
  const env = options.env || process.env;
  const spawnSyncImpl = options.spawnSyncImpl || spawnSync;
  const stdoutWrite = options.stdoutWrite || ((text) => process.stdout.write(text));
  const stdin = Object.prototype.hasOwnProperty.call(options, 'stdin') ? options.stdin : '';
  const agent = resolveAgent(env);
  let binaryPath;
  try {
    ({ binaryPath } = resolveBinary(env));
  } catch (err) {
    stdoutWrite(
      `${JSON.stringify(unavailable(`no cortex binary: ${err && err.message ? err.message : err}`))}\n`
    );
    return { ok: false, decision: 'UNAVAILABLE', agent };
  }

  const args = buildBootArgs(agent);
  const result = spawnSyncImpl(binaryPath, args, {
    env,
    input: stdin,
    encoding: 'utf8',
    timeout: 8000
  });

  if (result.error || result.status !== 0) {
    const reason = result.error
      ? result.error.message
      : `cortex hook-boot exited ${result.status}: ${(result.stderr || '').trim().slice(0, 200)}`;
    stdoutWrite(`${JSON.stringify(unavailable(reason))}\n`);
    return { ok: false, decision: 'UNAVAILABLE', agent, binaryPath, args };
  }

  if (result.stdout && result.stdout.trim()) {
    stdoutWrite(`${result.stdout.trim()}\n`);
  }
  return { ok: true, agent, binaryPath, args };
}

if (require.main === module) runHookBoot({ stdin: readStdin() });

module.exports = {
  unavailable,
  resolveAgent,
  buildBootArgs,
  resolveBinary,
  runHookBoot
};
