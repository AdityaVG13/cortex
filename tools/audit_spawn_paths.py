#!/usr/bin/env python3
"""Audit daemon spawn ownership paths.

Outputs a compact spawn-path map and can fail on unauthorized runtime callsites.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Iterable

REPO_ROOT = Path(__file__).resolve().parents[1]

AUTHORIZED_RUNTIME_SPAWN_CALLERS = {
    "daemon-rs/src/daemon_lifecycle.rs",
    "daemon-rs/src/main.rs",
}

AUTHORIZED_RESPAWN_CALLERS = {
    "daemon-rs/src/daemon_lifecycle.rs",
    "daemon-rs/src/mcp_proxy.rs",
}


@dataclass(frozen=True)
class Finding:
    kind: str
    path: str
    line: int
    snippet: str


def normalize(path: Path) -> str:
    return path.relative_to(REPO_ROOT).as_posix()


def read_lines(path: Path) -> list[str]:
    return path.read_text(encoding="utf-8", errors="replace").splitlines()


def scan(
    files: Iterable[Path],
    *,
    kind: str,
    pattern: re.Pattern[str],
    skip_when: re.Pattern[str] | None = None,
) -> list[Finding]:
    out: list[Finding] = []
    for path in files:
        rel = normalize(path)
        lines = read_lines(path)
        for index, line in enumerate(lines, start=1):
            if not pattern.search(line):
                continue
            if skip_when and skip_when.search(line):
                continue
            out.append(
                Finding(
                    kind=kind,
                    path=rel,
                    line=index,
                    snippet=line.strip(),
                )
            )
    return out


def gather_findings() -> dict[str, list[Finding]]:
    rust_runtime = sorted((REPO_ROOT / "daemon-rs" / "src").rglob("*.rs"))
    rust_tests = sorted((REPO_ROOT / "daemon-rs" / "tests").rglob("*.rs"))
    plugin_scripts = sorted((REPO_ROOT / "plugins" / "cortex-plugin" / "scripts").glob("*.cjs"))
    control_center_main = [
        REPO_ROOT / "desktop" / "cortex-control-center" / "src-tauri" / "src" / "main.rs"
    ]
    control_center_sidecar_file = (
        REPO_ROOT / "desktop" / "cortex-control-center" / "src-tauri" / "src" / "sidecar.rs"
    )

    spawn_def = scan(
        rust_runtime,
        kind="spawn_definition",
        pattern=re.compile(r"\bfn\s+spawn_daemon\s*\("),
    )
    spawn_call = scan(
        rust_runtime + rust_tests,
        kind="spawn_callsite",
        pattern=re.compile(r"\bspawn_daemon\s*\("),
        skip_when=re.compile(r"\bfn\s+spawn_daemon\s*\("),
    )
    respawn_call = scan(
        rust_runtime + rust_tests,
        kind="respawn_callsite",
        pattern=re.compile(r"\btry_respawn\s*\("),
        skip_when=re.compile(r"\bfn\s+try_respawn\s*\("),
    )
    ensure_call = scan(
        rust_runtime,
        kind="ensure_daemon_callsite",
        pattern=re.compile(r"\bensure_daemon\s*\("),
        skip_when=re.compile(r"\bfn\s+ensure_daemon\s*\("),
    )
    allow_spawn = scan(
        rust_runtime,
        kind="allow_spawn_flag",
        pattern=re.compile(r"\ballow_spawn\b"),
    )
    plugin_spawn = scan(
        plugin_scripts,
        kind="plugin_spawn_primitive",
        pattern=re.compile(r"\bspawn\s*\("),
    )
    owner_token_env = scan(
        rust_runtime + plugin_scripts,
        kind="owner_token_env_reference",
        pattern=re.compile(r"CORTEX_DAEMON_OWNER_TOKEN|DAEMON_OWNER_TOKEN_ENV"),
    )
    owner_token_validation = scan(
        rust_runtime,
        kind="owner_token_validation_callsite",
        pattern=re.compile(r"\bvalidate_spawned_owner_claim\s*\("),
        skip_when=re.compile(r"\bfn\s+validate_spawned_owner_claim\s*\("),
    )
    forbidden_control_center_sidecar_spawn = scan(
        control_center_main,
        kind="forbidden_control_center_sidecar_spawn",
        pattern=re.compile(r"\bstate\.start\s*\("),
    )
    forbidden_control_center_sidecar_env = scan(
        control_center_main,
        kind="forbidden_control_center_sidecar_env",
        pattern=re.compile(r"CORTEX_ALLOW_SIDECAR_FALLBACK"),
    )
    forbidden_plugin_legacy_app_url = scan(
        plugin_scripts,
        kind="forbidden_plugin_legacy_app_url",
        pattern=re.compile(r"CORTEX_DEV_APP_URL"),
    )
    forbidden_control_center_sidecar_module: list[Finding] = []
    if control_center_sidecar_file.exists():
        forbidden_control_center_sidecar_module.append(
            Finding(
                kind="forbidden_control_center_sidecar_module",
                path=normalize(control_center_sidecar_file),
                line=1,
                snippet="legacy sidecar module file exists",
            )
        )

    return {
        "spawn_definition": spawn_def,
        "spawn_callsite": spawn_call,
        "respawn_callsite": respawn_call,
        "ensure_daemon_callsite": ensure_call,
        "allow_spawn_flag": allow_spawn,
        "plugin_spawn_primitive": plugin_spawn,
        "owner_token_env_reference": owner_token_env,
        "owner_token_validation_callsite": owner_token_validation,
        "forbidden_control_center_sidecar_spawn": forbidden_control_center_sidecar_spawn,
        "forbidden_control_center_sidecar_env": forbidden_control_center_sidecar_env,
        "forbidden_plugin_legacy_app_url": forbidden_plugin_legacy_app_url,
        "forbidden_control_center_sidecar_module": forbidden_control_center_sidecar_module,
    }


def unauthorized_paths(
    findings: list[Finding], allowed_paths: set[str], *, allow_tests: bool
) -> set[str]:
    offenders: set[str] = set()
    for finding in findings:
        if finding.path in allowed_paths:
            continue
        if allow_tests and finding.path.startswith("daemon-rs/tests/"):
            continue
        offenders.add(finding.path)
    return offenders


def print_markdown_report(findings: dict[str, list[Finding]]) -> None:
    print("# Spawn Path Audit")
    print()
    print(f"- Repo root: `{REPO_ROOT.as_posix()}`")
    print("- Canonical spawn API: `daemon-rs/src/daemon_lifecycle.rs::spawn_daemon`")
    print()
    for section in (
        "spawn_definition",
        "spawn_callsite",
        "respawn_callsite",
        "ensure_daemon_callsite",
        "allow_spawn_flag",
        "plugin_spawn_primitive",
        "owner_token_env_reference",
        "owner_token_validation_callsite",
        "forbidden_control_center_sidecar_spawn",
        "forbidden_control_center_sidecar_env",
        "forbidden_plugin_legacy_app_url",
        "forbidden_control_center_sidecar_module",
    ):
        rows = findings[section]
        print(f"## {section}")
        if not rows:
            print("- none")
            print()
            continue
        print("| file | line | snippet |")
        print("|---|---:|---|")
        for row in rows:
            snippet = row.snippet.replace("|", "\\|")
            print(f"| `{row.path}` | {row.line} | `{snippet}` |")
        print()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--json",
        action="store_true",
        help="Emit JSON instead of markdown.",
    )
    parser.add_argument(
        "--strict",
        action="store_true",
        help="Fail if unauthorized runtime spawn/respawn callsites are detected.",
    )
    args = parser.parse_args()

    findings = gather_findings()
    if args.json:
        serializable = {key: [asdict(value) for value in values] for key, values in findings.items()}
        print(json.dumps(serializable, indent=2))
    else:
        print_markdown_report(findings)

    if not args.strict:
        return 0

    bad_spawn = unauthorized_paths(
        findings["spawn_callsite"], AUTHORIZED_RUNTIME_SPAWN_CALLERS, allow_tests=True
    )
    bad_respawn = unauthorized_paths(
        findings["respawn_callsite"], AUTHORIZED_RESPAWN_CALLERS, allow_tests=True
    )
    forbidden_hits = {
        key: findings[key]
        for key in (
            "forbidden_control_center_sidecar_spawn",
            "forbidden_control_center_sidecar_env",
            "forbidden_plugin_legacy_app_url",
            "forbidden_control_center_sidecar_module",
        )
        if findings[key]
    }
    if bad_spawn or bad_respawn or forbidden_hits:
        if bad_spawn:
            print(
                "Unauthorized spawn_daemon callsites: "
                + ", ".join(sorted(bad_spawn)),
                file=sys.stderr,
            )
        if bad_respawn:
            print(
                "Unauthorized try_respawn callsites: "
                + ", ".join(sorted(bad_respawn)),
                file=sys.stderr,
            )
        for key, rows in forbidden_hits.items():
            rendered = ", ".join(sorted({f"{row.path}:{row.line}" for row in rows}))
            print(f"Forbidden lifecycle pattern `{key}` found at: {rendered}", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
