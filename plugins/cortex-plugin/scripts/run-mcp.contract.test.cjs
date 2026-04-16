// SPDX-License-Identifier: MIT
const assert = require('assert/strict');
const test = require('node:test');

const {
  resolveRoute,
  buildMcpArgs,
  resolveOwnerMode,
  buildChildEnv,
  runMcpBridge
} = require('./run-mcp.cjs');

test('resolveRoute prefers explicit plugin URL over app URL', () => {
  const route = resolveRoute(
    { cortexUrl: 'https://team.cortex.example' },
    { CORTEX_APP_URL: 'http://127.0.0.1:7437' }
  );
  assert.deepEqual(route, {
    mode: 'remote',
    url: 'https://team.cortex.example',
    reason: 'explicit plugin URL'
  });
});

test('resolveRoute falls back to app URL when explicit URL is absent', () => {
  const route = resolveRoute({ cortexUrl: '' }, { CORTEX_APP_URL: 'http://127.0.0.1:7437' });
  assert.deepEqual(route, {
    mode: 'remote',
    url: 'http://127.0.0.1:7437',
    reason: 'app route'
  });
});

test('resolveRoute defaults to local service-first mode with no URLs', () => {
  const route = resolveRoute({ cortexUrl: '' }, {});
  assert.deepEqual(route, { mode: 'local', url: '', reason: 'local service-first' });
});

test('buildMcpArgs includes remote URL without exposing API key in args', () => {
  const args = buildMcpArgs(
    { mode: 'remote', url: 'https://team.cortex.example' },
    'claude-code'
  );
  assert.deepEqual(args, [
    'plugin',
    'mcp',
    '--agent',
    'claude-code',
    '--url',
    'https://team.cortex.example'
  ]);
});

test('buildMcpArgs keeps local mode attach-only args', () => {
  const args = buildMcpArgs({ mode: 'local', url: '' }, 'codex');
  assert.deepEqual(args, ['plugin', 'mcp', '--agent', 'codex']);
});

test('resolveOwnerMode maps local/team/app routes correctly', () => {
  assert.equal(
    resolveOwnerMode({ mode: 'local', reason: 'local service-first' }),
    'solo-service'
  );
  assert.equal(
    resolveOwnerMode({ mode: 'remote', reason: 'explicit plugin URL' }),
    'team'
  );
  assert.equal(resolveOwnerMode({ mode: 'remote', reason: 'app route' }), 'app');
});

test('buildChildEnv sets attach-only ownership contract in local mode', () => {
  const childEnv = buildChildEnv(
    {
      USERPROFILE: 'C:\\Users\\qa',
      CORTEX_DB: 'C:\\temp\\db.sqlite'
    },
    { mode: 'local', reason: 'local service-first', url: '' },
    'claude-code',
    'solo-service',
    4242,
    ''
  );
  assert.equal(childEnv.CORTEX_DAEMON_OWNER_KIND, 'plugin');
  assert.equal(childEnv.CORTEX_DAEMON_OWNER_SOURCE, 'claude-plugin');
  assert.equal(childEnv.CORTEX_DAEMON_OWNER_AGENT, 'claude-code');
  assert.equal(childEnv.CORTEX_DAEMON_OWNER_MODE, 'solo-service');
  assert.equal(childEnv.CORTEX_DAEMON_OWNER_LOCAL_SPAWN, '0');
  assert.equal(childEnv.CORTEX_DAEMON_OWNER_PARENT_PID, '4242');
  assert.equal(childEnv.CORTEX_HOME, 'C:\\Users\\qa\\.cortex');
  assert.equal(childEnv.CORTEX_DB, undefined);
});

test('buildChildEnv forwards remote API key via env instead of args', () => {
  const childEnv = buildChildEnv(
    {},
    { mode: 'remote', reason: 'explicit plugin URL', url: 'https://team.cortex.example' },
    'claude-code',
    'team',
    99,
    'ctx_remote'
  );
  assert.equal(childEnv.CORTEX_API_KEY, 'ctx_remote');
});

test('runMcpBridge dry run returns computed contract without spawning', () => {
  let spawned = false;
  const exits = [];
  const result = runMcpBridge({
    env: {
      CORTEX_PLUGIN_DRY_RUN: '1',
      CORTEX_PLUGIN_AGENT: 'codex',
      CORTEX_APP_URL: 'http://127.0.0.1:7441',
      HOME: '/tmp/cortex-home'
    },
    platform: 'linux',
    processRef: { pid: 9, on: () => {} },
    resolveCortexBinaryImpl: () => ({
      binaryPath: '/tmp/cortex',
      source: 'test'
    }),
    spawnImpl: () => {
      spawned = true;
      return { on: () => {} };
    },
    log: () => {},
    crashLogger: () => {},
    exit: (code) => exits.push(code),
    exitOnDryRun: false
  });

  assert.equal(spawned, false);
  assert.equal(result.ok, true);
  assert.equal(result.dryRun, true);
  assert.equal(result.route.mode, 'remote');
  assert.equal(result.route.reason, 'app route');
  assert.deepEqual(result.args, [
    'plugin',
    'mcp',
    '--agent',
    'codex',
    '--url',
    'http://127.0.0.1:7441'
  ]);
  assert.equal(result.childEnv.CORTEX_DAEMON_OWNER_MODE, 'app');
  assert.deepEqual(exits, []);
});

test('runMcpBridge spawns with expected args and env in explicit remote mode', async () => {
  const exits = [];
  const spawned = [];
  const childHandlers = {};

  runMcpBridge({
    env: {
      CLAUDE_PLUGIN_OPTION_CORTEX_URL: 'https://team.cortex.example',
      CLAUDE_PLUGIN_OPTION_CORTEX_API_KEY: 'ctx_remote',
      CORTEX_PLUGIN_AGENT: 'claude-code',
      HOME: '/tmp/home'
    },
    platform: 'linux',
    processRef: { pid: 777, on: () => {} },
    resolveCortexBinaryImpl: () => ({ binaryPath: '/tmp/cortex', source: 'test' }),
    spawnImpl: (binaryPath, args, options) => {
      spawned.push({ binaryPath, args, options });
      return {
        on: (event, handler) => {
          childHandlers[event] = handler;
        }
      };
    },
    log: () => {},
    crashLogger: () => {},
    exit: (code) => exits.push(code),
    registerProcessHandlers: false
  });

  assert.equal(spawned.length, 1);
  const [call] = spawned;
  assert.equal(call.binaryPath, '/tmp/cortex');
  assert.deepEqual(call.args, [
    'plugin',
    'mcp',
    '--agent',
    'claude-code',
    '--url',
    'https://team.cortex.example'
  ]);
  assert.equal(call.options.env.CORTEX_API_KEY, 'ctx_remote');
  assert.equal(call.options.env.CORTEX_DAEMON_OWNER_MODE, 'team');
  assert.equal(call.options.env.CORTEX_DAEMON_OWNER_PARENT_PID, '777');
  assert.equal(call.options.env.CORTEX_DAEMON_OWNER_LOCAL_SPAWN, '0');

  childHandlers.exit(0, null);
  assert.deepEqual(exits, [0]);
});
