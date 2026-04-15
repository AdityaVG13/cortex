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
const os = require('os');
const path = require('path');

function normalizeOption(value) {
  if (typeof value !== 'string') return '';
  return value.trim();
}

function fileExists(filePath) {
  return !!filePath && fs.existsSync(filePath);
}

function normalizedPath(value) {
  if (typeof value !== 'string') return '';
  let normalized = path.resolve(value).replace(/\\/g, '/');
  if (process.platform === 'win32') normalized = normalized.toLowerCase();
  return normalized;
}

function isLikelyTempPath(candidatePath) {
  if (!candidatePath) return false;
  const tempRoots = [process.env.TEMP, process.env.TMP, os.tmpdir()]
    .map(normalizeOption)
    .filter(Boolean)
    .map(normalizedPath);
  if (tempRoots.length === 0) return false;
  const normalizedCandidate = normalizedPath(candidatePath);
  return tempRoots.some((root) => {
    if (!root) return false;
    return normalizedCandidate === root || normalizedCandidate.startsWith(`${root}/`);
  });
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

function ensureCanonicalInstallFromSource(sourcePath, binaryName) {
  const userHome = resolveCanonicalUserHome();
  if (!userHome) {
    throw new Error('Cannot resolve user home to install canonical Cortex binary.');
  }
  const canonicalDir = path.join(userHome, '.cortex', 'bin');
  const canonicalPath = path.join(canonicalDir, binaryName);
  fs.mkdirSync(canonicalDir, { recursive: true });
  fs.copyFileSync(sourcePath, canonicalPath);
  if (process.platform !== 'win32') {
    try {
      fs.chmodSync(canonicalPath, 0o755);
    } catch (_) {}
  }
  return canonicalPath;
}

function resolveCortexBinary({
  pluginData,
  binaryName,
  ensureBundled,
  allowBundled = true,
  rejectTempCandidates = false
}) {
  const envOverrides = [
    ['CORTEX_APP_BINARY', normalizeOption(process.env.CORTEX_APP_BINARY)],
    ['CORTEX_DAEMON_BINARY', normalizeOption(process.env.CORTEX_DAEMON_BINARY)],
    ['CORTEX_PLUGIN_CORTEX_BINARY', normalizeOption(process.env.CORTEX_PLUGIN_CORTEX_BINARY)]
  ];

  for (const [name, candidate] of envOverrides) {
    if (fileExists(candidate) && !(rejectTempCandidates && isLikelyTempPath(candidate))) {
      return { binaryPath: candidate, source: `env:${name}` };
    }
  }

  const userHome = resolveCanonicalUserHome();
  if (userHome) {
    const canonicalInstall = path.join(userHome, '.cortex', 'bin', binaryName);
    if (fileExists(canonicalInstall) && !(rejectTempCandidates && isLikelyTempPath(canonicalInstall))) {
      return { binaryPath: canonicalInstall, source: 'canonical-install' };
    }

    const workspaceRoot =
      normalizeOption(process.env.CORTEX_WORKSPACE_ROOT) || path.join(userHome, 'cortex');
    for (const candidate of workspaceBinaryCandidates(workspaceRoot, binaryName)) {
      if (fileExists(candidate) && !(rejectTempCandidates && isLikelyTempPath(candidate))) {
        return { binaryPath: candidate, source: 'workspace-build' };
      }
    }
  }

  if (!allowBundled) {
    throw new Error(
      'No app-managed Cortex binary found. Install/start Cortex Control Center or set CORTEX_APP_BINARY.'
    );
  }

  const bundled = path.join(pluginData, 'bin', binaryName);
  if (!fileExists(bundled) && typeof ensureBundled === 'function') {
    ensureBundled();
  }
  if (rejectTempCandidates && isLikelyTempPath(bundled)) {
    try {
      const canonicalInstalled = ensureCanonicalInstallFromSource(bundled, binaryName);
      if (!isLikelyTempPath(canonicalInstalled) && fileExists(canonicalInstalled)) {
        return { binaryPath: canonicalInstalled, source: 'canonical-install-promoted' };
      }
    } catch (_) {}
    throw new Error(
      'Refusing temporary bundled Cortex binary for local mode and failed canonical promotion. Install/start Cortex Control Center or set CORTEX_APP_BINARY.'
    );
  }
  return { binaryPath: bundled, source: 'plugin-bundled' };
}

module.exports = {
  normalizeOption,
  resolveCortexBinary
};
