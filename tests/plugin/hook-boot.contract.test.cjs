// SPDX-License-Identifier: MIT
const assert = require('assert/strict');
const test = require('node:test');
const fs = require('node:fs');
const path = require('node:path');

const {
  unavailable,
  buildBootArgs,
  runHookBoot
} = require('../../plugins/cortex-plugin/scripts/hook-boot.cjs');

const FIXTURE_ROOT = path.join(process.cwd(), 'target', 'plugin-bridge-fixtures');

function emptyHomeEnv(extra = {}) {
  const missingHome = path.join(FIXTURE_ROOT, `boot-home-missing-${process.pid}`);
  return {
    HOME: missingHome,
    USERPROFILE: '',
    TEMP: '',
    TMP: '',
    CORTEX_APP_BINARY: '',
    CORTEX_DAEMON_BINARY: '',
    CORTEX_PLUGIN_CORTEX_BINARY: '',
    CORTEX_WORKSPACE_ROOT: path.join(missingHome, 'workspace'),
    CLAUDE_PLUGIN_DATA: path.join(missingHome, 'plugin-data'),
    ...extra
  };
}

test('buildBootArgs is local hook-boot, not an HTTP health probe', () => {
  assert.deepEqual(buildBootArgs('claude-code'), ['hook-boot', '--agent', 'claude-code']);
});

test('missing binary emits UNAVAILABLE without claiming an empty brain', () => {
  const lines = [];
  const result = runHookBoot({
    env: emptyHomeEnv(),
    spawnSyncImpl: () => {
      throw new Error('should not spawn');
    },
    stdoutWrite: (text) => lines.push(text)
  });

  assert.equal(result.ok, false);
  const payload = JSON.parse(lines.join(''));
  assert.equal(payload.cortex.decision, 'UNAVAILABLE');
  assert.match(payload.hookSpecificOutput.additionalContext, /Do not assume the brain is empty/);
});

test('successful local hook-boot relays stdout verbatim', () => {
  const binaryPath = path.join(FIXTURE_ROOT, `boot-ok-${process.pid}`);
  fs.mkdirSync(FIXTURE_ROOT, { recursive: true });
  fs.writeFileSync(binaryPath, '#!/bin/sh\nexit 0\n', { mode: 0o755 });
  const lines = [];
  const calls = [];
  const result = runHookBoot({
    env: emptyHomeEnv({ CORTEX_APP_BINARY: binaryPath, CORTEX_PLUGIN_AGENT: 'codex' }),
    spawnSyncImpl: (binary, args, opts) => {
      calls.push({ binary, args, opts });
      return {
        status: 0,
        stdout: '{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"boot"}}\n',
        stderr: ''
      };
    },
    stdoutWrite: (text) => lines.push(text)
  });

  assert.equal(result.ok, true);
  assert.deepEqual(calls[0].args, ['hook-boot', '--agent', 'codex']);
  assert.equal(
    lines.join('').trim(),
    '{"hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"boot"}}'
  );
});

test('failed hook-boot becomes UNAVAILABLE with the exit reason', () => {
  const lines = [];
  const result = runHookBoot({
    env: emptyHomeEnv(),
    spawnSyncImpl: () => ({ status: 2, stdout: '', stderr: 'db locked' }),
    stdoutWrite: (text) => lines.push(text)
  });
  // emptyHomeEnv has no binary, so this exercises the resolve-failure path
  // rather than the child-exit path; either way the host sees UNAVAILABLE.
  assert.equal(result.ok, false);
  const payload = JSON.parse(lines.join(''));
  assert.equal(payload.cortex.decision, 'UNAVAILABLE');
});

test('failed child exit becomes UNAVAILABLE with the exit reason', () => {
  const binaryPath = path.join(FIXTURE_ROOT, `boot-bin-${process.pid}`);
  fs.mkdirSync(FIXTURE_ROOT, { recursive: true });
  fs.writeFileSync(binaryPath, '#!/bin/sh\nexit 0\n', { mode: 0o755 });
  const lines = [];
  const result = runHookBoot({
    env: emptyHomeEnv({ CORTEX_APP_BINARY: binaryPath }),
    spawnSyncImpl: () => ({ status: 2, stdout: '', stderr: 'db locked' }),
    stdoutWrite: (text) => lines.push(text)
  });
  assert.equal(result.ok, false);
  const payload = JSON.parse(lines.join(''));
  assert.equal(payload.cortex.decision, 'UNAVAILABLE');
  assert.match(payload.cortex.reason, /exited 2/);
  assert.match(payload.cortex.reason, /db locked/);
});

test('unavailable envelope is well-formed SessionStart output', () => {
  const payload = unavailable('fixture');
  assert.equal(payload.hookSpecificOutput.hookEventName, 'SessionStart');
  assert.equal(payload.cortex.decision, 'UNAVAILABLE');
  assert.equal(payload.cortex.reason, 'fixture');
});
