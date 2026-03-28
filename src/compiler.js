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

// --- Helpers ----------------------------------------------------------------

/**
 * Estimate token count from text. ~3.8 chars per token on average.
 */
function estimateTokens(text) {
  return Math.ceil(text.length / 3.8);
}

/**
 * Read state.md from disk. Returns empty string on failure.
 */
function readState() {
  try {
    if (!fs.existsSync(STATE_PATH)) return '';
    return fs.readFileSync(STATE_PATH, 'utf-8');
  } catch {
    return '';
  }
}

/**
 * Extract a markdown section from state.md by heading.
 * Returns lines under the heading up to the next same-level heading.
 *
 * @param {string} content - Full state.md content
 * @param {string} heading - Heading text without `##` prefix
 * @param {number} [maxLines=Infinity] - Maximum lines to return
 * @returns {string}
 */
function extractSection(content, heading, maxLines = Infinity) {
  const lines = content.split('\n');
  let capturing = false;
  const captured = [];

  for (const line of lines) {
    if (capturing) {
      // Stop at the next same-level (##) heading
      if (/^## /.test(line)) break;
      captured.push(line);
      if (captured.length >= maxLines) break;
    } else if (line.replace(/^## /, '').trim() === heading) {
      capturing = true;
    }
  }

  return captured.join('\n').trim();
}

// --- Section generators -----------------------------------------------------
// Each returns a markdown string (may be empty if data is unavailable).

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

    // Aggregate success rates per skill
    const stats = {};
    for (const row of rows) {
      try {
        const d = JSON.parse(row.data);
        const skill = d.skill || d.name;
        if (!skill) continue;
        if (!stats[skill]) stats[skill] = { total: 0, success: 0 };
        stats[skill].total++;
        if (d.success || d.result === 'pass') stats[skill].success++;
      } catch {
        // skip malformed event data
      }
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
      "SELECT decision, context FROM decisions WHERE status = 'disputed' ORDER BY created_at DESC LIMIT 3"
    );
    if (!rows.length) return '_No open conflicts._';
    return rows
      .map((r) => {
        const ctx = r.context ? `\n  Context: ${r.context}` : '';
        return `- **Disputed:** ${r.decision}${ctx}`;
      })
      .join('\n');
  } catch {
    return '_Conflicts unavailable._';
  }
}

function genTopicIndex() {
  try {
    // Unique memory types
    const types = db.query(
      "SELECT DISTINCT type FROM memories WHERE status = 'active' ORDER BY type"
    );
    const typeList = types.map((r) => r.type).join(', ') || 'none';

    // Top topics by frequency (from tags)
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

// --- Section registry -------------------------------------------------------

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

/**
 * Human-readable heading for each section key.
 */
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

// --- Compiler ---------------------------------------------------------------

/**
 * Compile a boot prompt for the given profile.
 *
 * Pipeline:
 *  1. Load profile (sections + maxTokens)
 *  2. Generate markdown for each section
 *  3. Assemble with ## headers
 *  4. Trim lowest-priority (bottom) sections if over budget
 *  5. Return { bootPrompt, tokenEstimate, profile }
 *
 * @param {string} profileName - Profile name to compile for
 * @returns {{ bootPrompt: string, tokenEstimate: number, profile: string }}
 */
function compile(profileName) {
  const profile = getProfile(profileName);

  // Generate each section's content
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

  // Assemble and enforce token budget by dropping from the bottom
  let assembled = '';
  let includedCount = 0;

  for (const block of sectionBlocks) {
    const candidate = assembled
      ? `${assembled}\n\n${block.text}`
      : block.text;
    const tokens = estimateTokens(candidate);

    if (tokens > profile.maxTokens && includedCount > 0) {
      // Budget exceeded — stop adding sections
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

module.exports = { compile };
