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

test('trusted V5 invocation sidecar selects the automatic capture and prepare path', () => {
  const calls = [];
  const sidecar = { grant: 'approved', host_version: 'fixture-v1', session: 's', generation: 'g', event_key: 'u1', origins: { u1: 'external' }, context: 'c1' };
  assert.equal(bridge({ status: 0, stdout: '' }, { CORTEX_V5_CAPTURE: JSON.stringify(sidecar) }, calls), '');
  assert.deepEqual(calls[0].slice(0, 2), ['capture', 'host-cycle']);
  assert.equal(calls[0][calls[0].indexOf('--origins') + 1], '{"u1":"external"}');
  assert.ok(calls[0].includes('--payload'));
});

test('opted-in native user hooks derive stable invocation metadata without model commands', () => {
  const calls = [];
  const session = '225407e0-7c32-4812-b130-aa0d54039690';
  const prompt = 'ea0dd413-83bc-43a2-8913-506964f73808';
  const config = { grant: 'native-user', host_version: '2.1.260', native_user_prompts: true };
  const input = JSON.stringify({ session_id: session, prompt_id: prompt, hook_event_name: 'UserPromptSubmit', prompt: 'offline fixture' });
  const env = { CORTEX_TEST_EVENT: 'UserPromptSubmit', CORTEX_V5_CAPTURE: JSON.stringify(config) };
  assert.equal(bridge({ status: 0, stdout: '' }, env, calls, input), '');
  const args = calls[0];
  assert.deepEqual(args.slice(0, 2), ['capture', 'host-cycle']);
  assert.equal(args[args.indexOf('--session') + 1], session);
  assert.equal(args[args.indexOf('--event-key') + 1], prompt);
  assert.deepEqual(JSON.parse(args[args.indexOf('--origins') + 1]), { [prompt]: 'external' });
  assert.equal(args[args.indexOf('--context') + 1], `claude-user:${session}:${prompt}`);
  const rejected = [];
  const malformed = input.replace(prompt, 'not-a-native-uuid');
  assert.equal(JSON.parse(bridge({ status: 0, stdout: '' }, env, rejected, malformed)).cortex.decision, 'UNAVAILABLE');
  assert.equal(rejected.length, 0);
});

test('opted-in native Bash hooks use tool-use identity and leave other tools on the legacy path', () => {
  const calls = [];
  const session = '225407e0-7c32-4812-b130-aa0d54039690';
  const config = { grant: 'native-tools', host_version: '2.1.260', native_bash_results: true };
  const env = { CORTEX_V5_CAPTURE: JSON.stringify(config) };
  const payload = { session_id: session, tool_use_id: 'toolu_fixture_1', hook_event_name: 'PostToolUse', tool_name: 'Bash', tool_response: { stdout: 'fixture', stderr: '', interrupted: false, isImage: false, noOutputExpected: false } };
  assert.equal(bridge({ status: 0, stdout: '' }, env, calls, JSON.stringify(payload)), '');
  assert.deepEqual(calls[0].slice(0, 2), ['capture', 'host-cycle']);
  assert.equal(calls[0][calls[0].indexOf('--context') + 1], `claude-tool:${session}:toolu_fixture_1`);
  assert.deepEqual(JSON.parse(calls[0][calls[0].indexOf('--origins') + 1]), { toolu_fixture_1: 'external' });
  const other = [];
  bridge({ status: 0, stdout: '' }, env, other, JSON.stringify({ ...payload, tool_name: 'Read' }));
  assert.deepEqual(other[0].slice(0, 2), ['hook-event', 'PostToolUse']);
});



