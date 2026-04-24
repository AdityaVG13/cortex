#!/usr/bin/env node
/**
 * Lockstep version guard for the Cortex plugin bundle ↔ daemon release.
 *
 * Fails (exit 1) when:
 *   - `daemon-rs/Cargo.toml` package version and
 *     `plugins/cortex-plugin/.claude-plugin/plugin.json` version disagree.
 *   - Either file is missing or malformed.
 *
 * Warns (exit 0) on advisories:
 *   - `plugins/cortex-plugin/scripts/prepare-runtime.cjs` hard-coded fallback
 *     version string does not match the current Cargo version.
 *
 * Intended CI hook:
 *   `node scripts/check-plugin-lockstep.cjs`
 *   Add to `.github/workflows/ci.yml` as a pre-build step on the plugin job.
 *
 * Local usage: run before cutting any release candidate.
 */

'use strict';

const fs = require('fs');
const path = require('path');

const repoRoot = path.resolve(__dirname, '..');
const cargoTomlPath = path.join(repoRoot, 'daemon-rs', 'Cargo.toml');
const pluginJsonPath = path.join(
  repoRoot,
  'plugins',
  'cortex-plugin',
  '.claude-plugin',
  'plugin.json'
);
const prepareRuntimePath = path.join(
  repoRoot,
  'plugins',
  'cortex-plugin',
  'scripts',
  'prepare-runtime.cjs'
);

function die(msg) {
  console.error(`[lockstep] ERROR: ${msg}`);
  process.exit(1);
}

function warn(msg) {
  console.warn(`[lockstep] WARN : ${msg}`);
}

function readCargoVersion() {
  if (!fs.existsSync(cargoTomlPath)) {
    die(`Cargo.toml not found: ${cargoTomlPath}`);
  }
  const text = fs.readFileSync(cargoTomlPath, 'utf8');
  // Match only the [package] table's version (first version key before the
  // next [section]). Guards against matching a dependency's version= field.
  const pkgSection = text.split(/^\[/m)[1]; // everything between the first [ and the next [
  if (!pkgSection || !pkgSection.startsWith('package]')) {
    die('Cargo.toml does not appear to open with a [package] section');
  }
  const sectionBody = pkgSection.slice('package]'.length);
  const match = sectionBody.match(/^\s*version\s*=\s*"([^"]+)"\s*$/m);
  if (!match) {
    die(`Could not find version in [package] of ${cargoTomlPath}`);
  }
  return match[1];
}

function readPluginVersion() {
  if (!fs.existsSync(pluginJsonPath)) {
    die(`plugin.json not found: ${pluginJsonPath}`);
  }
  let obj;
  try {
    obj = JSON.parse(fs.readFileSync(pluginJsonPath, 'utf8'));
  } catch (e) {
    die(`plugin.json is not valid JSON: ${e.message}`);
  }
  if (!obj.version || typeof obj.version !== 'string') {
    die('plugin.json is missing a version field');
  }
  return obj.version;
}

function readPrepareRuntimeFallback() {
  if (!fs.existsSync(prepareRuntimePath)) {
    return null; // advisory only; do not fail
  }
  const text = fs.readFileSync(prepareRuntimePath, 'utf8');
  const match = text.match(/let\s+version\s*=\s*'([^']+)'/);
  return match ? match[1] : null;
}

function main() {
  const cargoVersion = readCargoVersion();
  const pluginVersion = readPluginVersion();

  console.error(`[lockstep] daemon-rs/Cargo.toml      : ${cargoVersion}`);
  console.error(`[lockstep] plugin.json               : ${pluginVersion}`);

  if (cargoVersion !== pluginVersion) {
    die(
      `version mismatch: daemon=${cargoVersion} plugin=${pluginVersion}. ` +
        'Bump both simultaneously before tagging a release.'
    );
  }

  const fallback = readPrepareRuntimeFallback();
  if (fallback && fallback !== cargoVersion) {
    warn(
      `prepare-runtime.cjs hard-coded fallback version ${fallback} ≠ ${cargoVersion}. ` +
        'Update the `let version = "..."` line when bumping.'
    );
  }

  console.error('[lockstep] PASS — daemon and plugin versions are lockstep.');
  process.exit(0);
}

main();
