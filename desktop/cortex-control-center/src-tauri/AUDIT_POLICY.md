# Desktop Rust Audit Policy

Last reviewed: 2026-05-18

Owner: Cortex desktop maintainer.

## Current State

`cargo update` has been run for this crate so the desktop lockfile uses the current compatible Tauri 2.x stack (`tauri` 2.11.2, `tauri-plugin-updater` 2.10.1, `wry` 0.55.1). That update removed the prior `rand` and `fxhash` audit warnings.

`cargo audit --no-fetch` reports no vulnerability advisories. The remaining advisories are warnings from target-universal lockfile dependencies:

| Advisory | Packages | Source | Disposition |
| --- | --- | --- | --- |
| RUSTSEC-2024-0411 through RUSTSEC-2024-0420 | GTK3 bindings (`atk`, `gdk`, `gtk`, related `-sys` crates, `gtk3-macros`) | Linux Tauri/Wry WebKit and tray stack | Temporarily allowed until Tauri/Wry move off GTK3. Review before any Linux release and monthly with dependency updates. |
| RUSTSEC-2024-0429 | `glib` 0.18.5 | Same Linux GTK3 stack | Temporarily allowed; this app does not call the affected `glib::VariantStrIter` API directly. Remove when the Tauri/Wry dependency graph upgrades `glib`. |
| RUSTSEC-2024-0370 | `proc-macro-error` 1.0.4 | `glib-macros` and `gtk3-macros` build-time dependency | Temporarily allowed with the GTK3 stack; remove with the GTK migration. |
| RUSTSEC-2025-0075, RUSTSEC-2025-0080, RUSTSEC-2025-0081, RUSTSEC-2025-0098, RUSTSEC-2025-0100 | `unic-*` crates | `urlpattern` via `tauri-utils` | Temporarily allowed until Tauri updates or replaces this parser dependency. |

## Verification

Run the reviewed audit gate from `desktop/cortex-control-center/src-tauri`:

```powershell
.\audit-reviewed.ps1
```

The script runs `cargo audit --deny warnings` with only the reviewed advisories ignored. Any new vulnerability or warning fails the audit and must be fixed or added here with owner, source, and removal criteria.
