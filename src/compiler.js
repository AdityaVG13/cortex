'use strict';

const fs = require('fs');
const path = require('path');
const { getProfile } = require('./profiles');
const db = require('./db');

const STATE_PATH = path.join(
  process.env.USERPROFILE || process.env.HOME,
  '.claude',
  'state.md'
);

// ═══════════════════════════════════════════════════════════════════════════
// Helpers
// ═══════════════════════════════════════════════════════════════════════════

function estimateTokens(text) {
  return Math.ceil(text.length / 3.8);
}

function readState() {
  try {
    if (!fs.existsSync(STATE_PATH)) return '';
    return fs.readFileSync(STATE_PATH, 'utf-8');
  } catch {
    return '';
  }
}

function extractSection(content, heading, maxLines = Infinity) {
  const lines = content.split('\n');
  let capturing = false;
  const captured = [];

  for (const line of lines) {
    if (capturing) {
      if (/^## /.test(line)) break;
      captured.push(line);
      if (captured.length >= maxLines) break;
    } else if (line.replace(/^## /, '').trim() === heading) {
      capturing = true;
    }
  }

  return captured.join('\n').trim();
}

// ═══════════════════════════════════════════════════════════════════════════
// Capsule System (new)
// ═══════════════════════════════════════════════════════════════════════════

// ─── Boot tracking ────────────────────────────────────────────────────────

/**
 * Get the timestamp of this agent's previous boot.
 * Returns ISO string or null if first boot.
 */
function getLastBootTime(agentId) {
  try {
    const row = db.get(
      "SELECT data FROM events WHERE type = 'agent_boot' AND source_agent = ? ORDER BY created_at DESC LIMIT 1",
      [agentId]
    );
    if (!row) return null;
    const data = JSON.parse(row.data);
    const ts = data.timestamp || null;
    if (!ts) return null;
    // Normalize to SQLite format and truncate milliseconds to match datetime('now')
    return ts.replace('T', ' ').replace('Z', '').replace(/\.\d+$/, '');
  } catch {
    return null;
  }
}

/**
 * Record this boot so the next session knows when we last connected.
 */
function recordBoot(agentId) {
  // Use SQLite-compatible format: space separator, no Z, no milliseconds
  const now = new Date().toISOString().replace('T', ' ').replace('Z', '').replace(/\.\d+$/, '');
  try {
    db.insert(
      'INSERT INTO events (type, data, source_agent) VALUES (?, ?, ?)',
      ['agent_boot', JSON.stringify({ timestamp: now, agent: agentId }), agentId]
    );
  } catch {
    // Non-critical
  }
  return now;
}

// ─── Identity Capsule ─────────────────────────────────────────────────────
// Stable across sessions. Changes only when user prefs or rules change.
// Target: ~200 tokens.

function buildIdentityCapsule() {
  const parts = [];

  // Core identity
  parts.push('User: Aditya. Platform: Windows 10. Shell: bash. Python: uv only. Git: conventional commits.');

  // Hard constraints (never/always/must rules)
  try {
    const rows = db.query(
      "SELECT text FROM memories WHERE type = 'feedback' AND status = 'active' ORDER BY score DESC LIMIT 20"
    );
    const constraintPattern = /\b(never|always|must|do not|don't|required|mandatory)\b/i;
    const constraints = rows.filter(r => constraintPattern.test(r.text)).slice(0, 5);
    if (constraints.length) {
      parts.push('Rules: ' + constraints.map(r => r.text.slice(0, 120)).join(' | '));
    }
  } catch { /* non-critical */ }

  // Platform sharp edges (Windows-specific gotchas)
  try {
    const rows = db.query(
      "SELECT text FROM memories WHERE type = 'feedback' AND status = 'active' ORDER BY score DESC LIMIT 20"
    );
    const edgePattern = /\b(windows|win32|encoding|cp1252|bash\.exe|CRLF)\b/i;
    const edges = rows.filter(r => edgePattern.test(r.text)).slice(0, 3);
    if (edges.length) {
      parts.push('Sharp edges: ' + edges.map(r => r.text.slice(0, 100)).join(' | '));
    }
  } catch { /* non-critical */ }

  const text = parts.join('\n');
  return {
    name: 'identity',
    text,
    tokens: estimateTokens(text),
    freshness: 'stable',
  };
}

// ─── Delta Capsule ────────────────────────────────────────────────────────
// What changed since this agent last connected.
// High relevance, changes every session. Target: ~300 tokens.

function buildDeltaCapsule(agentId, conductorState = null) {
  const lastBoot = getLastBootTime(agentId);
  const parts = [];

  // 0. Pending messages (highest priority, goes first)
  if (conductorState && conductorState.messages && conductorState.messages.length > 0) {
    const myMessages = conductorState.messages.filter(m => m.to === agentId);
    if (myMessages.length > 0) {
      const lines = [];
      for (const msg of myMessages) {
        lines.push(`- From ${msg.from}: "${msg.message}"`);
      }
      parts.push('## Pending Messages\n' + lines.join('\n'));
    }
  }

  // 0b. Active agents (session bus — who else is online)
  if (conductorState && conductorState.sessions && conductorState.sessions.length > 0) {
    const otherSessions = conductorState.sessions.filter(s => s.agent !== agentId);
    if (otherSessions.length > 0) {
      const sessionLines = [];
      for (const s of otherSessions) {
        const filePart = s.files && s.files.length > 0 ? ` (files: ${s.files.join(', ')})` : '';
        sessionLines.push(`- ${s.agent} working on ${s.project || 'unknown'}: "${s.description || 'no description'}"${filePart}`);
      }
      parts.push('## Active Agents\n' + sessionLines.join('\n'));
    }
  }

  // 0c. Active locks (high priority)
  if (conductorState && conductorState.locks && conductorState.locks.length > 0) {
    const lockLines = [];
    for (const lock of conductorState.locks) {
      const minutesLeft = Math.ceil((new Date(lock.expiresAt) - new Date()) / 60000);
      lockLines.push(`${lock.path} locked by ${lock.agent} (${minutesLeft}m remaining)`);
    }
    parts.push('## Active Locks\n' + lockLines.map(l => `- ${l}`).join('\n'));
  }

  // 0d. Task board (pending tasks + agent's claimed tasks)
  if (conductorState && conductorState.tasks && conductorState.tasks.length > 0) {
    const pendingTasks = conductorState.tasks.filter(t => t.status === 'pending');
    const myTasks = conductorState.tasks.filter(t => t.status === 'claimed' && t.claimedBy === agentId);

    if (pendingTasks.length > 0) {
      const lines = pendingTasks.slice(0, 5).map(t => {
        const filePart = t.files && t.files.length > 0 ? ` (files: ${t.files.join(', ')})` : '';
        return `- [${t.priority}] ${t.title}${t.project ? ' (' + t.project + ')' : ''}${filePart}`;
      });
      parts.push('## Pending Tasks\n' + lines.join('\n'));
    }

    if (myTasks.length > 0) {
      const lines = myTasks.map(t => {
        const ago = Math.ceil((new Date() - new Date(t.claimedAt)) / 60000);
        return `- [${t.priority}] ${t.title} (claimed ${ago}m ago)`;
      });
      parts.push('## Your Active Tasks\n' + lines.join('\n'));
    }
  }

  // 1. Open conflicts (always include — highest priority)
  try {
    const disputes = db.query(
      "SELECT id, decision, source_agent, disputes_id FROM decisions WHERE status = 'disputed' ORDER BY created_at DESC LIMIT 6"
    );
    if (disputes.length) {
      const seen = new Set();
      const lines = [];
      for (const r of disputes) {
        if (seen.has(r.id)) continue;
        seen.add(r.id);
        if (r.disputes_id) seen.add(r.disputes_id);
        const partner = r.disputes_id
          ? db.get("SELECT decision, source_agent FROM decisions WHERE id = ?", [r.disputes_id])
          : null;
        let line = `#${r.id} (${r.source_agent}): ${r.decision}`;
        if (partner) line += ` vs #${r.disputes_id} (${partner.source_agent}): ${partner.decision}`;
        lines.push(line);
      }
      parts.push('CONFLICTS:\n' + lines.map(l => `- ${l}`).join('\n'));
    }
  } catch { /* non-critical */ }

  // 2. State.md: next session + pending (always fresh)
  const state = readState();
  if (state) {
    const next = extractSection(state, 'Next Session', 5);
    if (next) parts.push('Next: ' + next.replace(/\n/g, ' | '));

    const pending = extractSection(state, 'Pending', 3);
    if (pending) parts.push('Pending: ' + pending.replace(/\n/g, ' | '));

    const issues = extractSection(state, 'Known Issues', 3);
    if (issues) parts.push('Issues: ' + issues.replace(/\n/g, ' | '));
  }

  // 3. New decisions since last boot
  if (lastBoot) {
    try {
      const newDecisions = db.query(
        "SELECT decision, context, source_agent FROM decisions WHERE status = 'active' AND created_at >= ? ORDER BY created_at DESC LIMIT 5",
        [lastBoot]
      );
      if (newDecisions.length) {
        const lines = newDecisions.map(r => {
          const ctx = r.context ? ` (${r.context})` : '';
          return `- [${r.source_agent}] ${r.decision}${ctx}`;
        });
        parts.push('New decisions:\n' + lines.join('\n'));
      }
    } catch { /* non-critical */ }

    // 4. New memories indexed since last boot
    try {
      const newMemories = db.query(
        "SELECT text, type FROM memories WHERE status = 'active' AND updated_at >= ? AND type != 'state' ORDER BY updated_at DESC LIMIT 3",
        [lastBoot]
      );
      if (newMemories.length) {
        const lines = newMemories.map(r => `- [${r.type}] ${r.text.slice(0, 100)}`);
        parts.push('New knowledge:\n' + lines.join('\n'));
      }
    } catch { /* non-critical */ }

    // 5. Events since last boot (summarized)
    try {
      const eventCounts = db.query(
        "SELECT type, COUNT(*) as cnt FROM events WHERE created_at > ? AND type NOT IN ('brain_init', 'index_all', 'agent_boot') GROUP BY type",
        [lastBoot]
      );
      if (eventCounts.length) {
        const summary = eventCounts.map(r => `${r.cnt} ${r.type.replace(/_/g, ' ')}`).join(', ');
        parts.push('Activity since last boot: ' + summary);
      }
    } catch { /* non-critical */ }
  } else {
    // First boot for this agent — include recent decisions as orientation
    try {
      const recent = db.query(
        "SELECT decision, context FROM decisions WHERE status = 'active' ORDER BY created_at DESC LIMIT 5"
      );
      if (recent.length) {
        const lines = recent.map(r => {
          const ctx = r.context ? ` — ${r.context}` : '';
          return `- ${r.decision}${ctx}`;
        });
        parts.push('Recent decisions:\n' + lines.join('\n'));
      }
    } catch { /* non-critical */ }
  }

  const text = parts.join('\n\n');
  return {
    name: 'delta',
    text,
    tokens: estimateTokens(text),
    freshness: lastBoot ? `since ${lastBoot.slice(0, 16)}` : 'first boot',
    lastBoot,
  };
}

// ─── Capsule Compiler ─────────────────────────────────────────────────────

/**
 * Compile boot prompt using the capsule system.
 *
 * Pipeline:
 *  1. Build identity capsule (stable, ~200 tokens)
 *  2. Build delta capsule (what's changed, ~300 tokens)
 *  3. Record this boot for next session's delta
 *  4. Return assembled prompt with capsule metadata
 *
 * @param {string} agentId - Agent identifier (e.g. 'claude-opus', 'gemini', 'codex')
 * @param {number} [maxTokens=600] - Token budget
 * @returns {{ bootPrompt, tokenEstimate, profile, capsules }}
 */
/**
 * Estimate what raw file reads would cost (the baseline Cortex replaces).
 * Counts chars in state.md + all memory files.
 */
function estimateRawBaseline() {
  const HOME = process.env.USERPROFILE || process.env.HOME;
  let totalChars = 0;

  // state.md
  try {
    if (fs.existsSync(STATE_PATH)) {
      totalChars += fs.readFileSync(STATE_PATH, 'utf-8').length;
    }
  } catch { /* non-critical */ }

  // Memory files
  const memDir = path.join(HOME, '.claude', 'projects', 'C--Users-aditya', 'memory');
  try {
    if (fs.existsSync(memDir)) {
      const files = fs.readdirSync(memDir).filter(f => f.endsWith('.md'));
      for (const file of files) {
        try {
          totalChars += fs.readFileSync(path.join(memDir, file), 'utf-8').length;
        } catch { /* skip */ }
      }
    }
  } catch { /* non-critical */ }

  return estimateTokens(String.fromCharCode(0).repeat(totalChars));  // chars/3.8
}

function compileCapsules(agentId, maxTokens = 600, conductorState = null) {
  const identity = buildIdentityCapsule();
  const delta = buildDeltaCapsule(agentId, conductorState);

  // Record this boot for next session
  recordBoot(agentId);

  // Assemble: identity first, then delta
  const capsules = [identity, delta].filter(c => c.text);
  let assembled = '';

  const included = [];
  for (const capsule of capsules) {
    const section = `## ${capsule.name === 'identity' ? 'Identity' : 'Delta'}\n${capsule.text}`;
    const candidate = assembled ? `${assembled}\n\n${section}` : section;
    const tokens = estimateTokens(candidate);

    if (tokens > maxTokens && included.length > 0) {
      // Over budget — delta gets trimmed
      // Try to fit a truncated delta
      const remaining = maxTokens - estimateTokens(assembled) - 10; // 10 for header overhead
      if (remaining > 50 && capsule.text) {
        const truncChars = Math.floor(remaining * 3.8);
        const truncText = capsule.text.slice(0, truncChars) + '...';
        assembled += `\n\n## Delta\n${truncText}`;
        included.push({ ...capsule, tokens: estimateTokens(truncText), truncated: true });
      }
      break;
    }

    assembled = candidate;
    included.push(capsule);
  }

  const bootPrompt = assembled;
  const tokenEstimate = estimateTokens(bootPrompt);
  const rawBaseline = estimateRawBaseline();
  const tokensSaved = Math.max(0, rawBaseline - tokenEstimate);
  const savingsPercent = rawBaseline > 0 ? Math.round((tokensSaved / rawBaseline) * 100) : 0;

  // Log boot event with savings data
  try {
    db.insert(
      'INSERT INTO events (type, data, source_agent) VALUES (?, ?, ?)',
      ['boot_savings', JSON.stringify({
        agent: agentId,
        served: tokenEstimate,
        baseline: rawBaseline,
        saved: tokensSaved,
        percent: savingsPercent,
      }), agentId]
    );
  } catch { /* non-critical */ }

  return {
    bootPrompt,
    tokenEstimate,
    profile: 'capsules',
    savings: {
      rawBaseline,
      served: tokenEstimate,
      saved: tokensSaved,
      percent: savingsPercent,
    },
    capsules: included.map(c => ({
      name: c.name,
      tokens: c.tokens,
      freshness: c.freshness,
      truncated: c.truncated || false,
    })),
  };
}

// ═══════════════════════════════════════════════════════════════════════════
// Legacy Section-Based Compiler (preserved as fallback)
// ═══════════════════════════════════════════════════════════════════════════

function genIdentity() {
  return 'User: Aditya. Platform: Windows 10. Shell: bash. Python: uv only. Git: conventional commits.';
}

function genNextSession() {
  const state = readState();
  if (!state) return '';
  return extractSection(state, 'Next Session', 5);
}

function genRecentDecisions() {
  try {
    const rows = db.query(
      "SELECT decision, context FROM decisions WHERE status = 'active' ORDER BY created_at DESC LIMIT 5"
    );
    if (!rows.length) return '_No recent decisions._';
    return rows
      .map((r) => {
        const ctx = r.context ? ` — ${r.context}` : '';
        return `- ${r.decision}${ctx}`;
      })
      .join('\n');
  } catch {
    return '_Decisions unavailable._';
  }
}

function genKeyRules() {
  try {
    const rows = db.query(
      "SELECT text FROM memories WHERE type = 'feedback' AND status = 'active' ORDER BY score DESC LIMIT 5"
    );
    if (!rows.length) return '_No key rules._';
    return rows.map((r) => `- ${r.text}`).join('\n');
  } catch {
    return '_Key rules unavailable._';
  }
}

function genConstraints() {
  try {
    const rows = db.query(
      "SELECT text FROM memories WHERE type = 'feedback' AND status = 'active' ORDER BY score DESC LIMIT 20"
    );
    const keywords = /\b(never|always|must|do not|don't|required|mandatory)\b/i;
    const filtered = rows.filter((r) => keywords.test(r.text)).slice(0, 5);
    if (!filtered.length) return '_No constraints found._';
    return filtered.map((r) => `- ${r.text}`).join('\n');
  } catch {
    return '_Constraints unavailable._';
  }
}

function genPending() {
  const state = readState();
  if (!state) return '';
  return extractSection(state, 'Pending');
}

function genKnownIssues() {
  const state = readState();
  if (!state) return '';
  return extractSection(state, 'Known Issues');
}

function genActiveLessons() {
  try {
    const rows = db.query(
      "SELECT text FROM memories WHERE type = 'lesson' AND confidence >= 0.6 AND status = 'active' ORDER BY score DESC LIMIT 3"
    );
    if (!rows.length) return '_No active lessons._';
    return rows.map((r) => `- ${r.text}`).join('\n');
  } catch {
    return '_Lessons unavailable._';
  }
}

function genUnderperformers() {
  try {
    const rows = db.query(
      "SELECT data FROM events WHERE type = 'skill-tracker' ORDER BY created_at DESC LIMIT 50"
    );
    if (!rows.length) return '_No skill-tracker data._';

    const stats = {};
    for (const row of rows) {
      try {
        const d = JSON.parse(row.data);
        const skill = d.skill || d.name;
        if (!skill) continue;
        if (!stats[skill]) stats[skill] = { total: 0, success: 0 };
        stats[skill].total++;
        if (d.success || d.result === 'pass') stats[skill].success++;
      } catch { /* skip */ }
    }

    const underperformers = Object.entries(stats)
      .map(([skill, s]) => ({ skill, rate: s.total > 0 ? (s.success / s.total) * 100 : 100 }))
      .filter((s) => s.rate < 60)
      .sort((a, b) => a.rate - b.rate)
      .slice(0, 5);

    if (!underperformers.length) return '_All skills above 60% threshold._';
    return underperformers
      .map((s) => `- **${s.skill}**: ${s.rate.toFixed(0)}% success rate`)
      .join('\n');
  } catch {
    return '_Skill tracker unavailable._';
  }
}

function genSharpEdges() {
  try {
    const rows = db.query(
      "SELECT text FROM memories WHERE type = 'feedback' AND status = 'active' ORDER BY score DESC LIMIT 20"
    );
    const windowsOrError = /\b(windows|win32|encoding|cp1252|path|bash\.exe|CRLF|error|crash|fail|quirk|workaround)\b/i;
    const filtered = rows.filter((r) => windowsOrError.test(r.text)).slice(0, 5);
    if (!filtered.length) return '_No sharp edges._';
    return filtered.map((r) => `- ${r.text}`).join('\n');
  } catch {
    return '_Sharp edges unavailable._';
  }
}

function genOpenConflicts() {
  try {
    const rows = db.query(
      "SELECT id, decision, context, source_agent, disputes_id FROM decisions WHERE status = 'disputed' ORDER BY created_at DESC LIMIT 6"
    );
    if (!rows.length) return '';

    const seen = new Set();
    const pairs = [];
    for (const r of rows) {
      if (seen.has(r.id)) continue;
      seen.add(r.id);
      if (r.disputes_id) seen.add(r.disputes_id);

      const partner = r.disputes_id
        ? db.get("SELECT decision, source_agent FROM decisions WHERE id = ?", [r.disputes_id])
        : null;

      let line = `- **#${r.id}** (${r.source_agent}): ${r.decision}`;
      if (partner) {
        line += `\n  **vs #${r.disputes_id}** (${partner.source_agent}): ${partner.decision}`;
      }
      pairs.push(line);
    }

    return pairs.join('\n');
  } catch {
    return '_Conflicts unavailable._';
  }
}

function genTopicIndex() {
  try {
    const types = db.query(
      "SELECT DISTINCT type FROM memories WHERE status = 'active' ORDER BY type"
    );
    const typeList = types.map((r) => r.type).join(', ') || 'none';

    const tagRows = db.query(
      "SELECT tags FROM memories WHERE status = 'active' AND tags IS NOT NULL AND tags != '' LIMIT 100"
    );
    const tagCounts = {};
    for (const row of tagRows) {
      const tags = String(row.tags).split(',').map((t) => t.trim()).filter(Boolean);
      for (const tag of tags) {
        tagCounts[tag] = (tagCounts[tag] || 0) + 1;
      }
    }
    const topTags = Object.entries(tagCounts)
      .sort((a, b) => b[1] - a[1])
      .slice(0, 5)
      .map(([tag]) => tag);

    let out = `**Types:** ${typeList}`;
    if (topTags.length) {
      out += `\n**Top topics:** ${topTags.join(', ')}`;
    }
    return out;
  } catch {
    return '_Topic index unavailable._';
  }
}

const SECTION_GENERATORS = {
  identity: genIdentity,
  nextSession: genNextSession,
  recentDecisions: genRecentDecisions,
  keyRules: genKeyRules,
  constraints: genConstraints,
  pending: genPending,
  knownIssues: genKnownIssues,
  activeLessons: genActiveLessons,
  underperformers: genUnderperformers,
  sharpEdges: genSharpEdges,
  openConflicts: genOpenConflicts,
  topicIndex: genTopicIndex,
};

const SECTION_HEADINGS = {
  identity: 'Identity',
  nextSession: 'Next Session',
  recentDecisions: 'Recent Decisions',
  keyRules: 'Key Rules',
  constraints: 'Constraints',
  pending: 'Pending',
  knownIssues: 'Known Issues',
  activeLessons: 'Active Lessons',
  underperformers: 'Underperformers',
  sharpEdges: 'Sharp Edges',
  openConflicts: 'Open Conflicts',
  topicIndex: 'Topic Index',
};

/**
 * Legacy compile — section-based, profile-driven.
 * Kept as fallback for profiles that don't use capsules.
 */
function compile(profileName) {
  // Route to capsule compiler when agent info is available
  if (profileName === 'capsules') {
    return compileCapsules('unknown');
  }

  const profile = getProfile(profileName);

  const sectionBlocks = [];
  for (const sectionKey of profile.sections) {
    const generator = SECTION_GENERATORS[sectionKey];
    if (!generator) continue;

    const content = generator();
    if (!content) continue;

    const heading = SECTION_HEADINGS[sectionKey] || sectionKey;
    sectionBlocks.push({
      key: sectionKey,
      text: `## ${heading}\n${content}`,
    });
  }

  let assembled = '';
  let includedCount = 0;

  for (const block of sectionBlocks) {
    const candidate = assembled
      ? `${assembled}\n\n${block.text}`
      : block.text;
    const tokens = estimateTokens(candidate);

    if (tokens > profile.maxTokens && includedCount > 0) {
      break;
    }

    assembled = candidate;
    includedCount++;
  }

  const bootPrompt = assembled;
  const tokenEstimate = estimateTokens(bootPrompt);

  return {
    bootPrompt,
    tokenEstimate,
    profile: profileName,
  };
}

module.exports = { compile, compileCapsules };
