#!/usr/bin/env node
/**
 * Shared binary resolution for plugin entrypoints.
 *
 * Resolution order:
 * 1. Explicit env overrides
 * 2. Canonical app-managed install (~/.cortex/bin)
 * 3. Common local workspace build outputs
 * 4. Bundled plugin runtime binary
 */

const fs = require('fs');
const path = require('path');

function normalizeOption(value) {
  if (typeof value !== 'string') return '';
  return value.trim();
}

function fileExists(filePath) {
  return !!filePath && fs.existsSync(filePath);
}

function resolveCanonicalUserHome() {
  const home = process.env.USERPROFILE || process.env.HOME || '';
  return normalizeOption(home);
}

function workspaceBinaryCandidates(workspaceRoot, binaryName) {
  if (!workspaceRoot) return [];
  return [
    path.join(workspaceRoot, 'daemon-rs', 'target-control-center-dev', 'debug', binaryName),
    path.join(workspaceRoot, 'daemon-rs', 'target', 'debug', binaryName),
    path.join(workspaceRoot, 'daemon-rs', 'target-control-center-release', 'release', binaryName),
    path.join(workspaceRoot, 'daemon-rs', 'target', 'release', binaryName)
  ];
}

function resolveCortexBinary({ pluginData, binaryName, ensureBundled }) {
  const envOverrides = [
    ['CORTEX_APP_BINARY', normalizeOption(process.env.CORTEX_APP_BINARY)],
    ['CORTEX_DAEMON_BINARY', normalizeOption(process.env.CORTEX_DAEMON_BINARY)],
    ['CORTEX_PLUGIN_CORTEX_BINARY', normalizeOption(process.env.CORTEX_PLUGIN_CORTEX_BINARY)]
  ];

  for (const [name, candidate] of envOverrides) {
    if (fileExists(candidate)) {
      return { binaryPath: candidate, source: `env:${name}` };
    }
  }

  const userHome = resolveCanonicalUserHome();
  if (userHome) {
    const canonicalInstall = path.join(userHome, '.cortex', 'bin', binaryName);
    if (fileExists(canonicalInstall)) {
      return { binaryPath: canonicalInstall, source: 'canonical-install' };
    }

    const workspaceRoot =
      normalizeOption(process.env.CORTEX_WORKSPACE_ROOT) || path.join(userHome, 'cortex');
    for (const candidate of workspaceBinaryCandidates(workspaceRoot, binaryName)) {
      if (fileExists(candidate)) {
        return { binaryPath: candidate, source: 'workspace-build' };
      }
    }
  }

  const bundled = path.join(pluginData, 'bin', binaryName);
  if (!fileExists(bundled) && typeof ensureBundled === 'function') {
    ensureBundled();
  }
  return { binaryPath: bundled, source: 'plugin-bundled' };
}

module.exports = {
  normalizeOption,
  resolveCortexBinary
};
