#!/usr/bin/env node
// Host event bridge: forwards the Claude Code hook payload (stdin) to the
// in-process `cortex hook-event <kind>` entry and relays its envelope. Never
// fabricates a decision: when no binary is available the output says
// UNAVAILABLE and tells the agent not to assume an empty brain.
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
  let args = ['hook-event', kind, '--agent', agent];
    if (process.env.CORTEX_V5_CAPTURE) {
      try {
        args = v5Arguments(process.env.CORTEX_V5_CAPTURE, kind, payloadRaw) || args;
      } catch (err) {
        process.stdout.write(JSON.stringify(unavailable(hostEvent, `invalid V5 invocation sidecar: ${err.message}`)) + '\n');
        return;
      }
    }
    const result = spawnSync(binaryPath, args, { input: payloadRaw, encoding: 'utf8', timeout: 8000 });
  if (result.error || result.status !== 0) {
    const reason = result.error ? result.error.message : `cortex hook-event exited ${result.status}: ${(result.stderr || '').trim().slice(0, 200)}`;
    process.stdout.write(JSON.stringify(unavailable(hostEvent, reason)) + '\n');
    return;
  }
  if (result.stdout && result.stdout.trim()) process.stdout.write(result.stdout.trim() + '\n');
}

if (require.main === module) main();
module.exports = { unavailable };
function v5Arguments(configRaw, kind, payloadRaw) {
    let sidecar = JSON.parse(configRaw);
      if (sidecar.native_user_prompts === true || sidecar.native_bash_results === true) {
        const user = kind === 'UserPromptSubmit' && sidecar.native_user_prompts === true;
        const tool = kind === 'PostToolUse' && sidecar.native_bash_results === true;
        if (!user && !tool) return null;
        if (sidecar.host_version !== '2.1.260') throw new Error('Unsupported native hook version');
        const payload = JSON.parse(payloadRaw);
        if (tool && payload.tool_name !== 'Bash') return null;
        const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
        const eventKey = user ? payload.prompt_id : payload.tool_use_id;
        if (payload.hook_event_name !== kind || typeof payload.session_id !== 'string' || !uuid.test(payload.session_id) ||
            typeof eventKey !== 'string' || (user ? !uuid.test(eventKey) : !/^[a-z0-9_-]{1,128}$/i.test(eventKey)) ||
            (user && typeof payload.prompt !== 'string')) {
          throw new Error('Missing native hook identity');
        }
        sidecar = {
          ...sidecar,
          session: payload.session_id,
          generation: sidecar.generation === undefined ? 'claude-code-2.1.260-v1' : sidecar.generation,
          event_key: eventKey,
          origins: { [eventKey]: 'external' },
          context: `claude-${user ? 'user' : 'tool'}:${payload.session_id}:${eventKey}`
        };
      }
      const args = ['capture', 'host-cycle', '--payload'];
      for (const [field, flag] of [['grant', '--grant'], ['host_version', '--host-version'], ['session', '--session'], ['generation', '--generation'], ['context', '--context']]) {
        if (typeof sidecar[field] !== 'string' || !sidecar[field].trim()) throw new Error(`Missing trusted ${field}`);
        args.push(flag, sidecar[field]);
      }
      if (!sidecar.origins || typeof sidecar.origins !== 'object' || Array.isArray(sidecar.origins)) throw new Error('Missing trusted origins');
      args.push('--origins', JSON.stringify(sidecar.origins));
      for (const [field, flag] of [['event_key', '--event-key'], ['present', '--present']]) {
        if (sidecar[field] !== undefined) {
          if (typeof sidecar[field] !== 'string') throw new Error(`Invalid trusted ${field}`);
          args.push(flag, sidecar[field]);
        }
      }
      return args;
}
