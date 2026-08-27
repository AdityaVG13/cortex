# Dependency Upgrade Log

**Date:** 2026-05-22
**Project:** cortex
**Language:** Rust, Node.js, Python
**Manifest:** daemon-rs/Cargo.toml; desktop/cortex-control-center/src-tauri/Cargo.toml; desktop/cortex-control-center/package.json; sdks/typescript/package.json; sdks/python/pyproject.toml

---

## Summary

| Metric | Count |
|--------|-------|
| **Total dependencies reviewed** | 55 |
| **Updated** | 52 |
| **Skipped** | 3 |
| **Failed (rolled back)** | 0 |
| **Requires attention** | 0 |

---

## Successfully Updated

### typescript: 5.9.3 -> 6.0.3

**Changelog:** https://www.typescriptlang.org/docs/handbook/release-notes/typescript-6-0.html

**Breaking changes:**
- TypeScript 6 defaults no longer expose all ambient types implicitly.

**Migration applied:**
- Added `"types": ["node"]` to `sdks/typescript/tsconfig.json`.

**Files modified:** 3
- `sdks/typescript/package.json`
- `sdks/typescript/package-lock.json`
- `sdks/typescript/tsconfig.json`

**Tests:** Passed after config fix with `npm test`.

---

### @types/node: 20.19.39 -> 25.8.0

**Changelog:** https://github.com/DefinitelyTyped/DefinitelyTyped/tree/master/types/node

**Breaking changes:** None found in this SDK surface; package is type-only.

**Notable constraints:**
- npm reported 25.9.1 as latest, but local minimum-release-age policy rejected it. Updated to newest accepted stable version, 25.8.0.

**Files modified:** 2
- `sdks/typescript/package.json`
- `sdks/typescript/package-lock.json`

**Tests:** Passed with `npm test`.

---

### vite: 7.3.2 -> 8.0.13

**Changelog:** https://vite.dev/blog/announcing-vite8
**Migration guide:** https://vite.dev/guide/migration

**Breaking changes reviewed:**
- Vite 8 moves to Rolldown/Oxc internals and changes the default browser target.

**Migration applied:** No code changes needed for this simple Vite config.

**Notable constraints:**
- npm reported 8.0.14 as latest, but local minimum-release-age policy rejected it. Updated to newest accepted stable version, 8.0.13.

**Files modified:** 2
- `desktop/cortex-control-center/package.json`
- `desktop/cortex-control-center/package-lock.json`

**Tests:** Passed with `npm test` and `npm run web:build`.

**Notes:** Vite build reports an existing chunk-size warning for the large visualizer bundle; build exits successfully.

---

### @vitejs/plugin-react: 5.2.0 -> 6.0.2

**Changelog:** https://github.com/vitejs/vite-plugin-react/releases

**Breaking changes reviewed:**
- Version 6 peers on Vite 8 and uses the current Vite plugin stack.

**Migration applied:** No code changes needed.

**Files modified:** 2
- `desktop/cortex-control-center/package.json`
- `desktop/cortex-control-center/package-lock.json`

**Tests:** Passed with `npm test` and `npm run web:build`.

**Notes:** Vite build still reports the large visualizer chunk-size warning; build exits successfully.

---

### httpx: >=0.24 -> >=0.28.1

**Changelog:** https://github.com/encode/httpx/blob/master/CHANGELOG.md

**Breaking changes reviewed:**
- HTTPX 0.28 removed previously deprecated client arguments; this SDK uses `httpx.Client(timeout=...)` and standard request methods, so no code migration was needed.

**Files modified:** 2
- `sdks/python/pyproject.toml`
- `sdks/python/uv.lock`

**Tests:** Passed with `uv run --extra dev pytest`.

---

### pytest: unbounded -> >=8.4.2

**Changelog:** https://docs.pytest.org/en/stable/announce/release-8.4.2.html

**Breaking changes:** None for the selected Python 3.9-compatible bug-fix floor.

**Files modified:** 2
- `sdks/python/pyproject.toml`
- `sdks/python/uv.lock`

**Tests:** Passed with `uv run --extra dev pytest`.

---

### pytest-httpx: unbounded -> >=0.35.0

**Changelog:** https://pypi.org/project/pytest-httpx/0.35.0/

**Breaking changes:** None found in the SDK test surface.

**Files modified:** 2
- `sdks/python/pyproject.toml`
- `sdks/python/uv.lock`

**Tests:** Passed with `uv run --extra dev pytest`.

**Notes:** `uv` resolves `pytest==8.4.2` and `pytest-httpx==0.35.0` for Python 3.9, and `pytest==9.0.3` and `pytest-httpx==0.36.2` for Python 3.10+.

---

### three: 0.183.2 -> 0.184.0

**Changelog:** https://github.com/mrdoob/three.js/releases

**Breaking changes:** None found in tested visual modules.

**Files modified:** 2
- `desktop/cortex-control-center/package.json`
- `desktop/cortex-control-center/package-lock.json`

**Tests:** Passed with `npm test`.

---

### vitest: 4.1.2 -> 4.1.6

**Changelog:** https://github.com/vitest-dev/vitest/releases

**Breaking changes:** None found for this patch update.

**Notable constraints:**
- npm reported 4.1.7 as latest, but local minimum-release-age policy rejected it. Updated to newest accepted stable version, 4.1.6.

**Files modified:** 2
- `desktop/cortex-control-center/package.json`
- `desktop/cortex-control-center/package-lock.json`

**Tests:** Passed with `npm test`.

---

### react-dom: 19.2.4 -> 19.2.6

**Changelog:** https://github.com/facebook/react/releases

**Breaking changes:** None found for this patch update.

**Files modified:** 2
- `desktop/cortex-control-center/package.json`
- `desktop/cortex-control-center/package-lock.json`

**Tests:** Passed with `npm test`.

---

### accessibility-checker-engine: 4.0.16 -> 4.0.17

**Changelog:** https://www.npmjs.com/package/accessibility-checker-engine

**Breaking changes:** None found for this patch update.

**Notable constraints:**
- npm reported 4.0.18 as latest, but local minimum-release-age policy rejected it. Updated to newest accepted stable version, 4.0.17.

**Files modified:** 2
- `desktop/cortex-control-center/package.json`
- `desktop/cortex-control-center/package-lock.json`

**Tests:** Passed with `npm test` on rerun. First run hit an unrelated randomized `Tiers.test.js` budget assertion and passed immediately on retry.

---

### react: 19.2.4 -> 19.2.6

**Changelog:** https://github.com/facebook/react/releases

**Breaking changes:** None found for this patch update.

**Files modified:** 2
- `desktop/cortex-control-center/package.json`
- `desktop/cortex-control-center/package-lock.json`

**Tests:** Passed with `npm test`.

---

### @tauri-apps/cli: 2.11.0 -> 2.11.1

**Changelog:** https://github.com/tauri-apps/tauri/releases

**Breaking changes:** None found for this patch update.

**Notable constraints:**
- npm reported 2.11.2 as latest, but local minimum-release-age policy rejected it. Updated to newest accepted stable version, 2.11.1.

**Files modified:** 2
- `desktop/cortex-control-center/package.json`
- `desktop/cortex-control-center/package-lock.json`

**Tests:** Passed with `npm test`.

---

### daemon-rs Rust direct dependencies

**Changelog sources:** crates.io metadata and upstream release/advisory pages for breaking-change checks.

**Updated direct dependencies:**

| Package | From | To |
|---------|------|----|
| axum | 0.8.8 | 0.8.9 |
| chrono | 0.4.42 | 0.4.44 |
| futures-util | 0.3.31 | 0.3.32 |
| regex | 1.12.2 | 1.12.3 |
| rusqlite | 0.37.0 | 0.39.0 |
| serde | 1 | 1.0.228 |
| serde_json | 1.0.149 | 1.0.150 |
| tempfile | 3 | 3.27.0 |
| toml | 0.8.23 | 1.1.2 |
| tokio | 1.50.0 | 1.52.3 |
| tokio-stream | 0.1.17 | 0.1.18 |
| uuid | 1.23.0 | 1.23.1 |
| argon2 | 0.5 | 0.5.3 |
| fs2 | 0.4 | 0.4.3 |
| hmac | 0.12.1 | 0.13.0 |
| sha2 | 0.10.9 | 0.11.0 |
| ndarray | 0.16.1 | 0.17.2 |
| tokenizers | 0.22.2 | 0.23.1 |
| reqwest | 0.12.28 | 0.13.3 |
| dirs | 6 | 6.0.0 |
| sysinfo | 0.33.1 | 0.38.4 |
| tower-http | 0.6.8 | 0.6.11 |
| rustls | 0.23.37 | 0.23.40 |
| tokio-rustls | 0.26 | 0.26.4 |
| hyper-util | 0.1 | 0.1.20 |
| tower | 0.5 | 0.5.3 |
| windows-service | 0.7.0 | 0.8.1 |
| windows-sys | 0.59.0 | 0.61.2 |
| libc | 0.2.183 | 0.2.186 |

**Dependency replacement:**
- Replaced direct `rustls-pemfile` usage with `rustls-pki-types` `PemObject` APIs per RUSTSEC-2025-0134.

**Migration applied:**
- `reqwest@0.13`: replaced `rustls-tls` feature with `rustls` and added the explicit `query` feature.
- `hmac@0.13`: imported `KeyInit` for `new_from_slice`.
- `rusqlite@0.39`: adjusted tests to read SQLite PRAGMA integer values as `i64`.
- `rustls-pemfile`: migrated certificate and private-key PEM loading to `rustls-pki-types`.

**Files modified:** 5
- `daemon-rs/Cargo.toml`
- `daemon-rs/Cargo.lock`
- `daemon-rs/src/daemon_lifecycle.rs`
- `daemon-rs/src/db.rs`
- `daemon-rs/src/tls.rs`

**Tests:** Passed with `cargo check --all-features`, `cargo test --all-features` (545 tests), and `cargo fmt --check`.

---

### desktop src-tauri Rust direct dependencies

**Updated direct dependencies:**

| Package | From | To |
|---------|------|----|
| tauri-build | 2 | 2.6.2 |
| dirs | 5.0.1 | 6.0.0 |
| fs2 | 0.4 | 0.4.3 |
| rusqlite | 0.37.0 | 0.39.0 |
| serde | 1 | 1.0.228 |
| serde_json | 1.0.149 | 1.0.150 |
| tauri | 2 | 2.11.2 |
| tauri-plugin-updater | 2 | 2.10.1 |
| toml | 0.8.2 | 1.1.2 |

**Migration applied:**
- `rusqlite@0.39`: adjusted the shutdown flush test to read PRAGMA integer values as `i64`.

**Files modified:** 3
- `desktop/cortex-control-center/src-tauri/Cargo.toml`
- `desktop/cortex-control-center/src-tauri/Cargo.lock`
- `desktop/cortex-control-center/src-tauri/src/main.rs`

**Tests:** Passed with `cargo check --all-features`, `cargo test --all-features` (29 tests), and `cargo fmt --check`.

---

## Skipped

### ort: 2.0.0-rc.12
**Reason:** Current dependency is intentionally on a release-candidate line; preserved per version rules.

### sqlite-vec: 0.1.9
**Reason:** crates.io latest is `0.1.10-alpha.4`; preserved current stable version.

### sysinfo: 0.39.2
**Reason:** Latest stable requires Rust 1.95; this environment is Rust 1.94.1 and project docs still advertise Rust 1.78+. Updated to latest compatible stable version, 0.38.4.

---

## Failed Updates (Rolled Back)

_None._

---

## Requires Attention

_None._

---

## Security Notes

**Vulnerabilities resolved:**
- Removed direct `rustls-pemfile` usage flagged by RUSTSEC-2025-0134.

**New advisories:** None detected.

**Audit command:**
- `cargo audit --file daemon-rs/Cargo.lock`
- `cargo audit --file desktop/cortex-control-center/src-tauri/Cargo.lock`
- `npm audit --json`

**Audit notes:**
- Daemon audit still reports `paste` as unmaintained through latest `tokenizers`; no direct update is available.
- Desktop audit reports unmaintained transitive crates through the latest Tauri stack; no direct update is available.
- `pip-audit` is not installed; Python verification used `uv` resolution and tests.

---

## Post-Upgrade Checklist

- [x] All tests passing
- [x] No deprecation warnings introduced
- [x] Documentation updated (not needed)

---

## Commands Used

```bash
cargo update --manifest-path daemon-rs/Cargo.toml --dry-run --verbose
cargo update --manifest-path desktop/cortex-control-center/src-tauri/Cargo.toml --dry-run --verbose
npm outdated --json
uv pip compile pyproject.toml --upgrade --extra dev
uv pip compile pyproject.toml --upgrade --extra dev --python-version 3.9
npm test
npm run web:build
uv run --extra dev pytest
cargo check --all-features
cargo test --all-features
cargo fmt --check
cargo audit --file daemon-rs/Cargo.lock
cargo audit --file desktop/cortex-control-center/src-tauri/Cargo.lock
npm audit --json
```

---

## Notes

- cargo-outdated is not installed, so discovery used Cargo dry-run updates plus direct crates.io version checks.
- The root package.json has no direct dependencies.
- npm latest versions newer than the local minimum-release-age policy were not forced.
- Vite build reports a chunk-size warning for the visualizer bundle but exits successfully.

---

## 2026-06-04 Continuation

**Mode:** continue existing `library-updater` artifacts from 2026-05-22.

### Summary

| Metric | Count |
|--------|-------|
| **Updated groups** | 8 |
| **Skipped or preserved** | 4 |
| **Failed and rolled back** | 1 |
| **Requires attention** | 0 |

### Successfully Updated

#### desktop/cortex-control-center Node dependencies

| Package | From | To |
|---------|------|----|
| @tauri-apps/cli | 2.11.1 | 2.11.2 |
| accessibility-checker-engine | 4.0.17 | 4.0.24 |
| vite | 8.0.13 | 8.0.14 |
| vitest | 4.1.6 | 4.1.7 |

**Research notes:**
- Tauri release notes list `@tauri-apps/cli@2.11.2` as a dependency-only CLI update.
- Vite 8.0.14 and Vitest 4.1.7 are patch releases; Vitest 4.1.7 lists a runner concurrency bug fix.
- `accessibility-checker-engine@4.0.26` exists, but npm rejected it under the repo's minimum-release-age policy; `4.0.24` was the newest accepted stable version.

**Files modified:** 2
- `desktop/cortex-control-center/package.json`
- `desktop/cortex-control-center/package-lock.json`

**Tests:** Passed after each update with `npm test` (23 files / 186 tests).

#### sdks/typescript Node dependencies

| Package | From | To |
|---------|------|----|
| @types/node | 25.8.0 | 25.9.1 |

**Breaking changes:** None expected; type-only patch update.

**Files modified:** 2
- `sdks/typescript/package.json`
- `sdks/typescript/package-lock.json`

**Tests:** Passed with `npm test` (build plus 10 node tests).

#### daemon-rs Rust direct dependencies and lockfile

| Package | From | To |
|---------|------|----|
| reqwest | 0.13.3 | 0.13.4 |
| uuid | 1.23.1 | 1.23.2 |

**Lockfile refresh:** Updated 30 additional Cargo.lock entries to latest Cargo-compatible versions.

**Research notes:**
- `reqwest@0.13.4` includes redirect-sensitive-header handling, TLS/client option fixes, and MSRV 1.85, compatible with local rustc 1.94.1.
- `uuid@1.23.2` improves ambiguous-format error messages.

**Files modified:** 2
- `daemon-rs/Cargo.toml`
- `daemon-rs/Cargo.lock`

**Tests:** Passed with `cargo check --manifest-path daemon-rs/Cargo.toml --all-features`.

#### desktop src-tauri Rust lockfile

**Lockfile refresh:** Updated 31 Cargo.lock entries to latest Cargo-compatible versions, including transitive `reqwest` 0.13.4, `uuid` 1.23.2, and `tao` 0.35.3.

**Files modified:** 1
- `desktop/cortex-control-center/src-tauri/Cargo.lock`

**Tests:** Passed with `cargo check --manifest-path desktop/cortex-control-center/src-tauri/Cargo.toml --all-features`.

#### sdks/python lockfile

| Package | From | To |
|---------|------|----|
| idna | 3.16 | 3.18 |

**Python 3.9 note:** `pytest>=8.4.2` and `pytest-httpx>=0.35.0` remain the latest Python 3.9-compatible lower bounds; Python 3.10+ still resolves newer versions through `uv.lock`.

**Files modified:** 1
- `sdks/python/uv.lock`

**Tests:** Passed with `uv run --extra dev pytest` (8 tests).

### Failed Updates (Rolled Back)

#### rusqlite: 0.39.0 -> 0.40.0

**Reason:** `libsqlite3-sys@0.38.0`, pulled by `rusqlite@0.40.0`, failed to compile on stable rustc 1.94.1 because its build script uses unstable `cfg_select!`.

**Action:** Rolled both Rust manifests back to `rusqlite@0.39.0` and restored daemon lockfile entries to `rusqlite@0.39.0` / `libsqlite3-sys@0.37.0`.

**Tests after rollback:** `cargo check --manifest-path daemon-rs/Cargo.toml --all-features` passed.

### Skipped / Preserved

#### sysinfo: 0.38.4 -> 0.39.3
**Reason:** crates.io metadata reports newer stable requires Rust 1.95; local toolchain is rustc 1.94.1.

#### sqlite-vec: 0.1.9 -> 0.1.10-alpha.4
**Reason:** Latest available version is alpha; stable 0.1.9 preserved.

#### ort: 2.0.0-rc.12
**Reason:** Current dependency is intentionally on a release-candidate line; preserved per version rules.

#### Python dev lower bounds
**Reason:** Newer `pytest` and `pytest-httpx` resolve for Python 3.10+, but raising manifest lower bounds would drop Python 3.9 compatibility.

### Commands Used

```bash
npm install --save-dev @tauri-apps/cli@2.11.2
npm install --save-dev accessibility-checker-engine@4.0.24
npm install --save-dev vite@8.0.14
npm install --save-dev vitest@4.1.7
npm install --save-dev @types/node@25.9.1
npm test
cargo update --manifest-path daemon-rs/Cargo.toml -p uuid --precise 1.23.2
cargo update --manifest-path daemon-rs/Cargo.toml -p reqwest --precise 0.13.4
cargo update --manifest-path daemon-rs/Cargo.toml
cargo update --manifest-path desktop/cortex-control-center/src-tauri/Cargo.toml
cargo check --manifest-path daemon-rs/Cargo.toml --all-features
cargo check --manifest-path desktop/cortex-control-center/src-tauri/Cargo.toml --all-features
uv lock --upgrade
uv run --extra dev pytest
```

### Final Verification

| Check | Result |
|-------|--------|
| `python -m json.tool claude-upgrade-progress.json` | Passed |
| `git diff --check` | Passed; Git reported existing CRLF normalization warnings for touched files |
| `cargo fmt --manifest-path daemon-rs/Cargo.toml -- --check` | Passed |
| `cargo fmt --manifest-path desktop/cortex-control-center/src-tauri/Cargo.toml -- --check` | Passed |
| `cargo test --manifest-path daemon-rs/Cargo.toml --all-features` | Passed, 548 unit tests plus integration suites |
| `cargo test --manifest-path desktop/cortex-control-center/src-tauri/Cargo.toml --all-features` | Passed, 29 tests |
| `npm test` in `desktop/cortex-control-center` | Passed, 23 files / 186 tests |
| `npm run web:build` in `desktop/cortex-control-center` | Passed; Vite still reports the known large visualizer chunk warning |
| `npm test` in `sdks/typescript` | Passed, build plus 10 tests |
| `uv run --extra dev pytest` in `sdks/python` | Passed, 8 tests |
| `uv lock --check` in `sdks/python` | Passed |

### Audit Notes

- `npm audit --json` passed with 0 vulnerabilities for `desktop/cortex-control-center`.
- `npm audit --json` passed with 0 vulnerabilities for `sdks/typescript`.
- `cargo audit --file daemon-rs/Cargo.lock` reported the known `paste` unmaintained warning through `tokenizers`; no direct update is available in this pass.
- `cargo audit --file desktop/cortex-control-center/src-tauri/Cargo.lock` reported known transitive Tauri/GTK ecosystem warnings; no direct update is available in this pass.
- `pip-audit` is not installed, so Python security audit was not run; Python verification used `uv lock --check`, `uv lock --upgrade`, and tests.

---

## 2026-06-04 Repair Attempt 1

**Trigger:** Queue-level verification failed after the continuation with `cargo test --manifest-path daemon-rs/Cargo.toml`.

**Root cause:** The plain daemon suite exposed a Windows full-suite scheduling race in the test-only response server used by startup preflight tests. The fixture could stop accepting after 3 seconds before `startup_single_daemon_preflight` reached its readiness probe, leaving the test with the expected bind denial but failed readiness and health probes.

**Repair:** Extended the test response server idle deadline from 3 seconds to 15 seconds in `daemon-rs/src/main.rs`.

**Verification:**
- `cargo test --manifest-path daemon-rs/Cargo.toml startup_preflight_rejects_canonical_ready_readiness_state --bin cortex -- --nocapture` passed.
- `cargo test --manifest-path daemon-rs/Cargo.toml` passed: 548 unit tests plus daemon integration suites.
