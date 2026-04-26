#!/usr/bin/env node
/**
 * Dry-run matrix for `run-mcp.cjs` routing resolver.
 *
 * The plugin MCP entry point is HTTP attach-only. It never local-spawns a
 * cortex daemon process; missing local daemon readiness is reported at runtime.
 */

const assert = require('node:assert/strict');
const { DEFAULT_LOCAL_BASE_URL, resolveRoute } = require('./run-mcp.cjs');

const cases = [
  {
    name: '1. Explicit URL set - remote HTTP route, no local spawn',
    config: { cortexUrl: 'https://cortex.myteam.com:7437' },
    env: {},
    expected: { mode: 'remote', spawnAllowed: false, hasUrl: true, reasonMatch: /explicit plugin URL/ }
  },
  {
    name: '2. Dev prefer app + CORTEX_APP_URL set - remote HTTP route, no local spawn',
    config: {},
    env: {
      CORTEX_DEV_PREFER_APP: '1',
      CORTEX_APP_URL: 'http://127.0.0.1:7437'
    },
    expected: { mode: 'remote', spawnAllowed: false, hasUrl: true, reasonMatch: /dev prefer app/ }
  },
  {
    name: '3. Dev prefer app + NO CORTEX_APP_URL - explicit failure',
    config: {},
    env: { CORTEX_DEV_PREFER_APP: '1' },
    expected: { mode: 'fail', spawnAllowed: false, hasUrl: false, reasonMatch: /CORTEX_APP_URL/ }
  },
  {
    name: '4. No URL - local HTTP attach-only route, no local spawn',
    config: {},
    env: {},
    expected: { mode: 'local', spawnAllowed: false, hasUrl: true, reasonMatch: /local HTTP attach-only/ }
  },
  {
    name: '5. Local-spawn disable flag is redundant - still local HTTP attach-only',
    config: {},
    env: { CORTEX_DEV_DISABLE_LOCAL_SPAWN: '1' },
    expected: { mode: 'local', spawnAllowed: false, hasUrl: true, reasonMatch: /local HTTP attach-only/ }
  },
  {
    name: '6. Explicit URL beats dev prefer app',
    config: { cortexUrl: 'https://explicit.example' },
    env: {
      CORTEX_DEV_PREFER_APP: '1',
      CORTEX_APP_URL: 'http://should-be-ignored'
    },
    expected: { mode: 'remote', spawnAllowed: false, hasUrl: true, reasonMatch: /explicit plugin URL/ }
  },
  {
    name: '7. Legacy local-spawn allow flag is ignored - no spawn path exists',
    config: {},
    env: {
      CORTEX_DEV_DISABLE_LOCAL_SPAWN: '1',
      CORTEX_PLUGIN_ALLOW_LOCAL_SPAWN: '1'
    },
    expected: { mode: 'local', spawnAllowed: false, hasUrl: true, reasonMatch: /local HTTP attach-only/ }
  },
  {
    name: '8. CORTEX_APP_URL alone - remote app route',
    config: {},
    env: { CORTEX_APP_URL: 'http://127.0.0.1:7437' },
    expected: { mode: 'remote', spawnAllowed: false, hasUrl: true, reasonMatch: /app route/ }
  }
];

let pass = 0;
let fail = 0;
for (const tc of cases) {
  try {
    const route = resolveRoute(tc.config, tc.env);
    assert.equal(route.mode, tc.expected.mode, 'mode mismatch');
    assert.equal(route.spawnAllowed, tc.expected.spawnAllowed, 'spawnAllowed mismatch');
    if (tc.expected.hasUrl) {
      assert.ok(route.url && route.url.length > 0, `expected url, got: ${JSON.stringify(route.url)}`);
    } else {
      assert.equal(route.url, '', 'expected empty url');
    }
    if (route.mode === 'local') {
      assert.equal(route.url, DEFAULT_LOCAL_BASE_URL);
    }
    if (tc.expected.reasonMatch) {
      assert.match(route.reason, tc.expected.reasonMatch, 'reason mismatch');
    }
    console.log(`PASS  ${tc.name}`);
    console.log(`      -> mode=${route.mode} spawnAllowed=${route.spawnAllowed} reason="${route.reason}"`);
    pass++;
  } catch (err) {
    console.error(`FAIL  ${tc.name}`);
    console.error(`      ${err.message}`);
    fail++;
  }
}

console.log(`\n${pass}/${cases.length} passed. ${fail} failed.`);
process.exit(fail === 0 ? 0 : 1);

