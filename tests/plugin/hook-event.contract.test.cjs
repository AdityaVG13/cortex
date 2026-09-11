const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const bridgePath = path.resolve(__dirname, '../../plugins/cortex-plugin/scripts/hook-event.cjs');
function bridge(result, env = {}, calls = [], inputRaw = '{}') {
  const module = { exports: {} };
  let output = '';
  const localRequire = (name) => {
    if (name === 'path') return path;
    if (name === 'fs') return { readFileSync: () => inputRaw };
    if (name === 'child_process') return { spawnSync: (_binary, args) => { calls.push(Array.from(args)); return result; } };
    if (name.endsWith('resolve-binary.cjs')) return { resolveCortexBinary: () => ({ binaryPath: '/approved/cortex' }) };
    throw new Error(`Unexpected require: ${name}`);
  };
  localRequire.main = module;
  vm.runInNewContext(fs.readFileSync(bridgePath, 'utf8'), {
    require: localRequire, module, __dirname: path.dirname(bridgePath),
    process: { argv: ['node', bridgePath, env.CORTEX_TEST_EVENT || 'PostToolUse'], env, platform: 'darwin', stdout: { write: (text) => { output += text; } } },
  });
  return output;
}
test('successful silent capture does not become model-visible UNAVAILABLE', () => {
  assert.equal(bridge({ status: 0, stdout: '', stderr: '' }), '');
});
test('real bridge failures remain visible while delivery is forwarded', () => {
  assert.equal(JSON.parse(bridge({ status: 1, stdout: '', stderr: 'denied' })).cortex.decision, 'UNAVAILABLE');
  assert.equal(bridge({ status: 0, stdout: '{"delivery":"evidence"}', stderr: '' }), '{"delivery":"evidence"}\n');
});

test('every host event uses cortex hook, including when a capture sidecar is present', () => {
  const withSidecar = [];
  const sidecar = { grant: 'approved', host_version: 'fixture-v1', session: 's', generation: 'g', event_key: 'u1', origins: { u1: 'external' }, context: 'c1' };
  assert.equal(bridge({ status: 0, stdout: '' }, { CORTEX_CAPTURE: JSON.stringify(sidecar) }, withSidecar), '');
  assert.deepEqual(withSidecar[0].slice(0, 2), ['hook', 'PostToolUse']);
  const without = [];
  assert.equal(bridge({ status: 0, stdout: '' }, {}, without), '');
  assert.deepEqual(without[0].slice(0, 2), ['hook', 'PostToolUse']);
});

test('file and Bash events share the same hook command', () => {
  const fileCalls = [];
  const bashCalls = [];
  const session = '225407e0-7c32-4812-b130-aa0d54039690';
  const env = { CORTEX_CAPTURE: JSON.stringify({ grant: 'native-files', host_version: 'fixture-v1', native_file_results: true }) };
  const payload = { session_id: session, tool_use_id: 'toolu_read_1', hook_event_name: 'PostToolUse', tool_name: 'Read', tool_response: { filePath: '/tmp/a.rs', content: 'fn main() {}' } };
  assert.equal(bridge({ status: 0, stdout: '' }, env, fileCalls, JSON.stringify(payload)), '');
  assert.deepEqual(fileCalls[0].slice(0, 2), ['hook', 'PostToolUse']);
  bridge({ status: 0, stdout: '' }, env, bashCalls, JSON.stringify({ ...payload, tool_name: 'Bash', tool_response: { stdout: 'x', stderr: '', interrupted: false, isImage: false, noOutputExpected: false } }));
  assert.deepEqual(bashCalls[0].slice(0, 2), ['hook', 'PostToolUse']);
});
