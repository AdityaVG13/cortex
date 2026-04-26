// SPDX-License-Identifier: MIT
const assert = require('assert/strict');
const test = require('node:test');
const { Writable } = require('node:stream');

const {
  DEFAULT_LOCAL_BASE_URL,
  resolveRoute,
  buildMcpArgs,
  resolveOwnerMode,
  buildChildEnv,
  buildAuthHeader,
  healthCheck,
  forwardMcpMessage,
  runMcpBridge
} = require('./run-mcp.cjs');

function memoryWriter() {
  const chunks = [];
  const stream = new Writable({
    write(chunk, _encoding, callback) {
      chunks.push(chunk.toString('utf8'));
      callback();
    }
  });
  stream.lines = () => chunks.join('').trim().split(/\n/).filter(Boolean);
  return stream;
}

test('resolveRoute prefers explicit plugin URL over app URL', () => {
  const route = resolveRoute(
    { cortexUrl: 'https://team.cortex.example' },
    { CORTEX_APP_URL: 'http://127.0.0.1:7437' }
  );
  assert.deepEqual(route, {
    mode: 'remote',
    url: 'https://team.cortex.example',
    reason: 'explicit plugin URL',
    spawnAllowed: false
  });
});

test('resolveRoute falls back to app URL when explicit URL is absent', () => {
  const route = resolveRoute({ cortexUrl: '' }, { CORTEX_APP_URL: 'http://127.0.0.1:7437' });
  assert.deepEqual(route, {
    mode: 'remote',
    url: 'http://127.0.0.1:7437',
    reason: 'app route',
    spawnAllowed: false
  });
});

test('resolveRoute defaults to local HTTP attach-only with no URLs', () => {
  const route = resolveRoute({ cortexUrl: '' }, {});
  assert.deepEqual(route, {
    mode: 'local',
    url: DEFAULT_LOCAL_BASE_URL,
    reason: 'local HTTP attach-only',
    spawnAllowed: false
  });
});

test('resolveRoute fails dev prefer app when CORTEX_APP_URL is absent', () => {
  const route = resolveRoute({ cortexUrl: '' }, { CORTEX_DEV_PREFER_APP: '1' });
  assert.equal(route.mode, 'fail');
  assert.equal(route.spawnAllowed, false);
  assert.match(route.reason, /CORTEX_APP_URL is not set/);
});

test('buildMcpArgs describes the node HTTP proxy contract', () => {
  const args = buildMcpArgs(
    { mode: 'remote', url: 'https://team.cortex.example' },
    'claude-code'
  );
  assert.deepEqual(args, [
    'http-proxy',
    '--agent',
    'claude-code',
    '--url',
    'https://team.cortex.example'
  ]);
});

test('resolveOwnerMode maps local/team/app routes correctly', () => {
  assert.equal(resolveOwnerMode({ mode: 'local', reason: 'local HTTP attach-only' }), 'solo-service');
  assert.equal(resolveOwnerMode({ mode: 'remote', reason: 'explicit plugin URL' }), 'team');
  assert.equal(resolveOwnerMode({ mode: 'remote', reason: 'app route' }), 'app');
});

test('buildChildEnv preserves no-local-spawn ownership contract for proxy context', () => {
  const proxyEnv = buildChildEnv(
    {
      USERPROFILE: 'C:\\Users\\qa',
      CORTEX_DB: 'C:\\temp\\db.sqlite'
    },
    { mode: 'local', reason: 'local HTTP attach-only', url: DEFAULT_LOCAL_BASE_URL },
    'claude-code',
    'solo-service',
    4242,
    ''
  );
  assert.equal(proxyEnv.CORTEX_DAEMON_OWNER_KIND, 'plugin');
  assert.equal(proxyEnv.CORTEX_DAEMON_OWNER_SOURCE, 'claude-plugin');
  assert.equal(proxyEnv.CORTEX_DAEMON_OWNER_AGENT, 'claude-code');
  assert.equal(proxyEnv.CORTEX_DAEMON_OWNER_MODE, 'solo-service');
  assert.equal(proxyEnv.CORTEX_DAEMON_OWNER_LOCAL_SPAWN, '0');
  assert.equal(proxyEnv.CORTEX_DAEMON_OWNER_PARENT_PID, '4242');
  assert.equal(proxyEnv.CORTEX_API_KEY, undefined);
});

test('buildAuthHeader reads local token only for loopback targets', () => {
  const local = buildAuthHeader('http://127.0.0.1:7437', '', {
    USERPROFILE: 'Z:\\missing'
  });
  const remote = buildAuthHeader('https://team.cortex.example', '', {
    USERPROFILE: 'Z:\\missing'
  });
  assert.equal(local, '');
  assert.equal(remote, '');
  assert.equal(
    buildAuthHeader('https://team.cortex.example', 'ctx_remote', {}),
    'Bearer ctx_remote'
  );
});

test('healthCheck accepts readiness payload and does not require auth for local probe', async () => {
  const requests = [];
  const health = await healthCheck('http://127.0.0.1:7437', '', {
    env: { CORTEX_HOME: '/tmp/cortex-home' },
    requestImpl: async (request) => {
      requests.push(request);
      return {
        statusCode: 200,
        body: JSON.stringify({
          ready: true,
          status: 'ready',
          runtime: {
            port: 7437,
            db_path: '/tmp/cortex-home/cortex.db',
            token_path: '/tmp/cortex-home/cortex.token',
            pid_path: '/tmp/cortex-home/cortex.pid'
          },
          stats: { home: '/tmp/cortex-home', memories: 4, decisions: 1 }
        })
      };
    }
  });

  assert.equal(health.ok, true);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].url, 'http://127.0.0.1:7437/readiness');
  assert.equal(requests[0].headers['X-Cortex-Request'], 'true');
  assert.equal(requests[0].headers.Authorization, undefined);
});

test('forwardMcpMessage posts JSON-RPC to /mcp-rpc with Cortex headers', async () => {
  const stdout = memoryWriter();
  const requests = [];

  const result = await forwardMcpMessage(
    '{"jsonrpc":"2.0","id":7,"method":"ping"}',
    {
      baseUrl: 'http://127.0.0.1:7437',
      apiKey: 'local_token',
      agent: 'claude-code',
      model: 'test-model',
      env: {},
      stdout
    },
    {
      requestImpl: async (request) => {
        requests.push(request);
        return {
          statusCode: 200,
          body: '{"jsonrpc":"2.0","id":7,"result":{"ok":true}}'
        };
      }
    }
  );

  assert.equal(result.ok, true);
  assert.equal(requests.length, 1);
  assert.equal(requests[0].url, 'http://127.0.0.1:7437/mcp-rpc');
  assert.equal(requests[0].headers['X-Cortex-Request'], 'true');
  assert.equal(requests[0].headers.Authorization, 'Bearer local_token');
  assert.equal(requests[0].headers['X-Source-Agent'], 'claude-code');
  assert.equal(requests[0].headers['X-Source-Model'], 'test-model');
  assert.deepEqual(stdout.lines(), ['{"jsonrpc":"2.0","id":7,"result":{"ok":true}}']);
});

test('forwardMcpMessage returns JSON-RPC parse error locally', async () => {
  const stdout = memoryWriter();
  const result = await forwardMcpMessage('{bad json', {
    baseUrl: 'http://127.0.0.1:7437',
    apiKey: '',
    agent: 'claude-code',
    model: '',
    env: {},
    stdout
  });

  assert.equal(result.parseError, true);
  const [line] = stdout.lines();
  const payload = JSON.parse(line);
  assert.equal(payload.error.code, -32700);
  assert.equal(payload.error.message, 'Parse error');
});

test('runMcpBridge dry run returns HTTP proxy contract without binary resolution or spawn', async () => {
  const exits = [];
  const result = await runMcpBridge({
    env: {
      CORTEX_PLUGIN_DRY_RUN: '1',
      CORTEX_PLUGIN_AGENT: 'codex',
      CORTEX_APP_URL: 'http://127.0.0.1:7441',
      HOME: '/tmp/cortex-home'
    },
    processRef: { pid: 9, on: () => {} },
    log: () => {},
    crashLogger: () => {},
    exit: (code) => exits.push(code),
    exitOnDryRun: false
  });

  assert.equal(result.ok, true);
  assert.equal(result.dryRun, true);
  assert.equal(result.route.mode, 'remote');
  assert.equal(result.route.reason, 'app route');
  assert.equal(result.baseUrl, 'http://127.0.0.1:7441');
  assert.equal(result.proxyEnv.CORTEX_DAEMON_OWNER_MODE, 'app');
  assert.deepEqual(exits, []);
});
