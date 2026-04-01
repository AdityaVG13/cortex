'use strict';

const fs = require('fs');
const path = require('path');

const PROFILES_PATH = path.join(__dirname, '..', 'cortex-profiles.json');

/**
 * Hardcoded fallback profiles used when cortex-profiles.json is missing or corrupt.
 */
const FALLBACK_PROFILES = {
  full: {
    maxTokens: 700,
    sections: ['identity', 'openConflicts', 'nextSession', 'recentDecisions', 'keyRules', 'pending', 'knownIssues', 'activeLessons', 'underperformers'],
    description: 'Full context for Claude Code sessions',
  },
  operational: {
    maxTokens: 500,
    sections: ['identity', 'openConflicts', 'constraints', 'recentDecisions', 'sharpEdges'],
    description: 'Execution-focused for Codex CLI',
  },
  subagent: {
    maxTokens: 200,
    sections: ['identity', 'constraints'],
    description: 'Minimal context for sub-agents',
  },
  index: {
    maxTokens: 300,
    sections: ['identity', 'topicIndex'],
    description: 'Topic list only for Gemini CLI',
  },
};

const FALLBACK_AGENT_DEFAULTS = {
  'claude-opus': 'full',
  'claude-sonnet': 'subagent',
  gemini: 'index',
  codex: 'operational',
  cursor: 'operational',
  qwen: 'subagent',
  'cli-manual': 'full',
};

/**
 * Load the profiles registry from disk. Returns parsed JSON or null on failure.
 */
function loadRegistry() {
  try {
    if (!fs.existsSync(PROFILES_PATH)) return null;
    const raw = fs.readFileSync(PROFILES_PATH, 'utf-8');
    const data = JSON.parse(raw);
    if (!data.profiles || typeof data.profiles !== 'object') return null;
    return data;
  } catch {
    return null;
  }
}

/**
 * Get a profile by name.
 * Loads from cortex-profiles.json first; falls back to hardcoded defaults.
 *
 * @param {string} name - Profile name (e.g. 'full', 'operational')
 * @returns {{ maxTokens: number, sections: string[], description: string }}
 */
function getProfile(name) {
  const registry = loadRegistry();
  const profiles = registry?.profiles ?? FALLBACK_PROFILES;
  const profile = profiles[name];
  if (!profile) {
    // Unknown profile — return 'full' as the safest default
    return profiles.full ?? FALLBACK_PROFILES.full;
  }
  return {
    maxTokens: profile.maxTokens,
    sections: Array.isArray(profile.sections) ? profile.sections : [],
    description: profile.description || '',
  };
}

/**
 * Get the default profile name for a given agent.
 *
 * @param {string} agentName - Agent identifier (e.g. 'claude-opus', 'gemini')
 * @returns {string} Profile name
 */
function getDefaultProfile(agentName) {
  const registry = loadRegistry();
  const defaults = registry?.agentDefaults ?? FALLBACK_AGENT_DEFAULTS;
  return defaults[agentName] || 'full';
}

/**
 * List all available profiles with their descriptions.
 *
 * @returns {{ name: string, description: string, maxTokens: number }[]}
 */
function listProfiles() {
  const registry = loadRegistry();
  const profiles = registry?.profiles ?? FALLBACK_PROFILES;
  return Object.entries(profiles).map(([name, profile]) => ({
    name,
    description: profile.description || '',
    maxTokens: profile.maxTokens,
  }));
}

module.exports = { getProfile, getDefaultProfile, listProfiles };
