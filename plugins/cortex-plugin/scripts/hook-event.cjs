#!/usr/bin/env node
// Host event bridge: forwards the host hook payload (stdin) to
// `cortex hook <kind>` and relays its envelope. Never fabricates a decision:
// when no binary is available the output says UNAVAILABLE and tells the agent
// not to assume an empty brain. That command is observation-only; CQR boot
// stays on hook-boot.cjs. This script does not choose a second command.
const path = require('path');
const { spawnSync } = require('child_process');
const { resolveCortexBinary } = require(path.join(__dirname, 'resolve-binary.cjs'));

function unavailable(hostEvent, reason) {
  return {
    hookSpecificOutput: { hookEventName: hostEvent, additionalContext: 'Cortex: memory unavailable for this event. Do not assume the brain is empty.' },
    cortex: { event: null, decision: 'UNAVAILABLE', reason, overflow: false, presence: 'unknown', automatic_capture: false, counted: false }
  };
}

function readStdin() {
  try {
    return require('fs').readFileSync(0, 'utf8');
  } catch (_) {
    return '';
  }
}

function main() {
  const kind = process.argv[2] || 'SessionStart';
  const payloadRaw = readStdin();
  let hostEvent = kind;
  try {
    const parsed = JSON.parse(payloadRaw || '{}');
    if (parsed && typeof parsed.hook_event_name === 'string') hostEvent = parsed.hook_event_name;
  } catch (_) {}
  let binaryPath;
  try {
    ({ binaryPath } = resolveCortexBinary({
      pluginData: process.env.CLAUDE_PLUGIN_DATA || '',
      binaryName: process.platform === 'win32' ? 'cortex.exe' : 'cortex',
      allowBundled: true,
      rejectTempCandidates: true,
      env: process.env
    }));
  } catch (err) {
    process.stdout.write(JSON.stringify(unavailable(hostEvent, `no cortex binary: ${err.message}`)) + '\n');
    return;
  }
  const agent = (process.env.CORTEX_PLUGIN_AGENT || 'claude-code').trim() || 'claude-code';
  const result = spawnSync(binaryPath, ['hook', kind, '--agent', agent], {
    input: payloadRaw,
    encoding: 'utf8',
    timeout: 8000
  });
  if (result.error || result.status !== 0) {
    const reason = result.error ? result.error.message : `cortex hook exited ${result.status}: ${(result.stderr || '').trim().slice(0, 200)}`;
    process.stdout.write(JSON.stringify(unavailable(hostEvent, reason)) + '\n');
    return;
  }
  if (result.stdout && result.stdout.trim()) process.stdout.write(result.stdout.trim() + '\n');
}

if (require.main === module) main();
module.exports = { unavailable };
