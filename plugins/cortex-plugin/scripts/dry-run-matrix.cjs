#!/usr/bin/env node
/**
 * Dry-run matrix for `run-mcp.cjs` routing resolver.
 *
 * Invokes `resolveRoute` with 5 representative environment shapes and asserts
 * the resulting (mode, spawnAllowed) pair matches the documented behavior
 * matrix in `docs/internal/v060/plugin-routing.md`.
 *
 * Exit code 0 = all 5 pass. Exit code 1 = at least one mismatch.
 *
 * Run: `node plugins/cortex-plugin/scripts/dry-run-matrix.cjs`
 */

const assert = require('node:assert/strict');
const { resolveRoute } = require('./run-mcp.cjs');

const cases = [
  {
    name: '1. Explicit URL set — remote route, no local spawn',
    config: { cortexUrl: 'https://cortex.myteam.com:7437' },
    env: {},
    expected: { mode: 'remote', spawnAllowed: false, hasUrl: true, reasonMatch: /explicit plugin URL/ }
  },
  {
    name: '2. Dev prefer app + URL set — remote route, no local spawn',
    config: {},
    env: {
      CORTEX_DEV_PREFER_APP: '1',
      CORTEX_DEV_APP_URL: 'http://127.0.0.1:7437'
    },
    expected: { mode: 'remote', spawnAllowed: false, hasUrl: true, reasonMatch: /dev prefer app/ }
  },
  {
    name: '3. Dev prefer app + NO URL — explicit failure',
    config: {},
    env: { CORTEX_DEV_PREFER_APP: '1' },
    expected: { mode: 'fail', spawnAllowed: false, hasUrl: false, reasonMatch: /CORTEX_DEV_PREFER_APP=1/ }
  },
  {
    name: '4. No URL, local spawn allowed — local route',
    config: {},
    env: {},
    expected: { mode: 'local', spawnAllowed: true, hasUrl: false, reasonMatch: /local service-first/ }
  },
  {
    name: '5. No URL, local spawn disabled — explicit failure',
    config: {},
    env: { CORTEX_DEV_DISABLE_LOCAL_SPAWN: '1' },
    expected: {
      mode: 'fail',
      spawnAllowed: false,
      hasUrl: false,
      reasonMatch: /CORTEX_DEV_DISABLE_LOCAL_SPAWN=1/
    }
  },
  // Extra bonus cases to lock corner behavior
  {
    name: '6. Explicit URL beats dev prefer app',
    config: { cortexUrl: 'https://explicit.example' },
    env: {
      CORTEX_DEV_PREFER_APP: '1',
      CORTEX_DEV_APP_URL: 'http://should-be-ignored'
    },
    expected: { mode: 'remote', spawnAllowed: false, hasUrl: true, reasonMatch: /explicit plugin URL/ }
  },
  {
    name: '7. Local disable overridden by CORTEX_PLUGIN_ALLOW_LOCAL_SPAWN',
    config: {},
    env: {
      CORTEX_DEV_DISABLE_LOCAL_SPAWN: '1',
      CORTEX_PLUGIN_ALLOW_LOCAL_SPAWN: '1'
    },
    expected: { mode: 'local', spawnAllowed: true, hasUrl: false, reasonMatch: /local service-first/ }
  },
  {
    name: '8. CORTEX_APP_URL alone → remote app route',
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
    if (tc.expected.reasonMatch) {
      assert.match(route.reason, tc.expected.reasonMatch, 'reason mismatch');
    }
    console.log(`PASS  ${tc.name}`);
    console.log(`      → mode=${route.mode} spawnAllowed=${route.spawnAllowed} reason="${route.reason}"`);
    pass++;
  } catch (err) {
    console.error(`FAIL  ${tc.name}`);
    console.error(`      ${err.message}`);
    fail++;
  }
}

console.log(`\n${pass}/${cases.length} passed. ${fail} failed.`);
process.exit(fail === 0 ? 0 : 1);
