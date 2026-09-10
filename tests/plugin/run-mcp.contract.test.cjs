// SPDX-License-Identifier: MIT
const assert = require('assert/strict');
const test = require('node:test');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { EventEmitter } = require('node:events');
const { PassThrough } = require('node:stream');

const {
  resolveAgent,
  buildMcpArgs,
  resolveBridgePlan,
  runMcpBridge
} = require('../../plugins/cortex-plugin/scripts/run-mcp.cjs');

// Binary fixtures must live outside os.tmpdir() because the production
// resolver rejects temporary candidate paths.
const FIXTURE_ROOT = path.join(process.cwd(), 'target', 'plugin-bridge-fixtures');

function makeBinary(name = 'cortex') {
  fs.mkdirSync(FIXTURE_ROOT, { recursive: true });
  const binaryPath = path.join(FIXTURE_ROOT, `${name}-${process.pid}-${Date.now()}-${Math.random().toString(16).slice(2)}`);
  fs.writeFileSync(binaryPath, '#!/bin/sh\nexit 0\n', { mode: 0o755 });
  return binaryPath;
}

function fakeChild() {
  const child = new EventEmitter();
  child.stdin = new PassThrough();
  child.stdout = new PassThrough();
  child.stderr = new PassThrough();
  child.kill = () => {};
  return child;
}

function emptyHomeEnv(extra = {}) {
  const missingHome = path.join(FIXTURE_ROOT, `home-missing-${process.pid}`);
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

test('resolveAgent defaults to claude-code and honors CORTEX_PLUGIN_AGENT', () => {
  assert.equal(resolveAgent({}), 'claude-code');
  assert.equal(resolveAgent({ CORTEX_PLUGIN_AGENT: 'codex' }), 'codex');
});

test('buildMcpArgs is local stdio, not an HTTP proxy', () => {
  assert.deepEqual(buildMcpArgs('claude-code'), ['mcp', '--agent', 'claude-code']);
});

test('resolveBridgePlan uses env binary override', () => {
  const binaryPath = makeBinary();
  const plan = resolveBridgePlan(
    emptyHomeEnv({ CORTEX_APP_BINARY: binaryPath, CORTEX_PLUGIN_AGENT: 'codex' })
  );
  assert.equal(plan.binaryPath, binaryPath);
  assert.equal(plan.agent, 'codex');
  assert.deepEqual(plan.args, ['mcp', '--agent', 'codex']);
  assert.equal(plan.source, 'env:CORTEX_APP_BINARY');
});

test('runMcpBridge dry run reports local binary and does not spawn', async () => {
  const binaryPath = makeBinary();
  const exits = [];
  let spawned = false;
  const result = await runMcpBridge({
    env: emptyHomeEnv({
      CORTEX_PLUGIN_DRY_RUN: '1',
      CORTEX_PLUGIN_AGENT: 'codex',
      CORTEX_APP_BINARY: binaryPath
    }),
    processRef: { pid: 9, on: () => {} },
    log: () => {},
    crashLogger: () => {},
    exit: (code) => exits.push(code),
    spawnImpl: () => {
      spawned = true;
      return fakeChild();
    },
    exitOnDryRun: false
  });

  assert.equal(result.ok, true);
  assert.equal(result.dryRun, true);
  assert.equal(result.binaryPath, binaryPath);
  assert.deepEqual(result.args, ['mcp', '--agent', 'codex']);
  assert.equal(spawned, false);
  assert.deepEqual(exits, []);
});

test('runMcpBridge spawns cortex mcp and forwards stdio', async () => {
  const binaryPath = makeBinary();
  const child = fakeChild();
  const spawnCalls = [];
  const stdin = new PassThrough();
  const result = runMcpBridge({
    env: emptyHomeEnv({ CORTEX_APP_BINARY: binaryPath, CORTEX_PLUGIN_AGENT: 'claude-code' }),
    processRef: new EventEmitter(),
    log: () => {},
    crashLogger: () => {},
    exit: () => {},
    spawnImpl: (binary, args, opts) => {
      spawnCalls.push({ binary, args, opts });
      return child;
    },
    stdin,
    stdout: new PassThrough(),
    stderr: new PassThrough(),
    registerProcessHandlers: false,
    exitOnChildExit: false,
    exitOnChildSignal: false,
    exitOnFailure: false
  });

  assert.equal(result.ok, true);
  assert.equal(result.binaryPath, binaryPath);
  assert.equal(spawnCalls.length, 1);
  assert.equal(spawnCalls[0].binary, binaryPath);
  assert.deepEqual(spawnCalls[0].args, ['mcp', '--agent', 'claude-code']);
});

test('runMcpBridge fails closed when no binary can be resolved', async () => {
  const exits = [];
  const logs = [];
  const result = await runMcpBridge({
    env: emptyHomeEnv({ CORTEX_PLUGIN_AGENT: 'claude-code' }),
    processRef: { pid: 1, on: () => {} },
    log: () => {},
    crashLogger: (msg) => logs.push(msg),
    exit: (code) => exits.push(code),
    spawnImpl: () => {
      throw new Error('should not spawn');
    },
    exitOnFailure: false
  });

  assert.equal(result.ok, false);
  assert.match(result.error, /No local cortex binary/);
  assert.equal(exits.length, 0);
  assert.ok(logs.some((line) => /No local cortex binary/.test(line)));
});
