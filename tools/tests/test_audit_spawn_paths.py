# SPDX-License-Identifier: MIT

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path


SCRIPT_PATH = Path(__file__).resolve().parents[1] / "audit_spawn_paths.py"


def _write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def _build_fixture_repo(root: Path, *, include_unauthorized_spawn: bool = False) -> None:
    _write(root / "daemon-rs" / "src" / "main.rs", "fn allowed() { spawn_daemon(); }\n")
    _write(root / "daemon-rs" / "src" / "daemon_lifecycle.rs", "fn also_allowed() { try_respawn(); }\n")
    if include_unauthorized_spawn:
        _write(root / "daemon-rs" / "src" / "handlers" / "rogue.rs", "fn bad() { spawn_daemon(); }\n")
    _write(root / "plugins" / "cortex-plugin" / "scripts" / "run-mcp.cjs", "const child = spawnImpl();\n")
    _write(root / "desktop" / "cortex-control-center" / "src-tauri" / "src" / "main.rs", "// noop\n")


def _run_audit(repo_root: Path, *args: str) -> subprocess.CompletedProcess[str]:
    env = os.environ.copy()
    env["CORTEX_AUDIT_REPO_ROOT"] = str(repo_root)
    return subprocess.run(
        [sys.executable, str(SCRIPT_PATH), *args],
        cwd=repo_root,
        env=env,
        capture_output=True,
        text=True,
        check=False,
    )


def test_strict_passes_for_authorized_spawn_paths(tmp_path: Path) -> None:
    _build_fixture_repo(tmp_path, include_unauthorized_spawn=False)
    result = _run_audit(tmp_path, "--strict")
    assert result.returncode == 0, result.stderr


def test_strict_fails_for_unauthorized_spawn_paths(tmp_path: Path) -> None:
    _build_fixture_repo(tmp_path, include_unauthorized_spawn=True)
    result = _run_audit(tmp_path, "--strict")
    assert result.returncode == 1
    assert "Unauthorized spawn_daemon callsites" in result.stderr
    assert "daemon-rs/src/handlers/rogue.rs" in result.stderr


def test_json_reports_plugin_spawn_impl_primitive(tmp_path: Path) -> None:
    _build_fixture_repo(tmp_path, include_unauthorized_spawn=False)
    result = _run_audit(tmp_path, "--json")
    assert result.returncode == 0, result.stderr
    payload = json.loads(result.stdout)
    plugin_hits = payload["plugin_spawn_primitive"]
    assert plugin_hits
    assert plugin_hits[0]["path"] == "plugins/cortex-plugin/scripts/run-mcp.cjs"
