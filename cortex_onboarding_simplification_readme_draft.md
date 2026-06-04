# README Approval Record

Aditya approved applying the README onboarding simplification on June 4, 2026.
The applied patch was text-only: capsule/header image lines, asset references, and GIF/image files were not changed.

Applied changes:

- Inserted a `Quick Start` section before the first `Before / After` visual section.
- Added `cortex status --json` as the readiness check and `scripts\first-run-smoke.ps1` as the product smoke.
- Clarified that `cortex-http-pure`, provider keys, and LongMemEval are benchmark-only concerns, not normal runtime setup.
- Reworded the Claude/plugin onboarding copy so the plugin is attach-only and reports `APP_INIT_REQUIRED` when the runtime is not ready.
- Replaced manual release-verification curl ceremony with the status command and smoke script.

Verification:

- `git diff -- README.md` showed no changed `capsule-render`, `assets/`, GIF, PNG, JPG, JPEG, or WebP lines.
- Targeted tests and `git diff --check` passed after applying the README text.
