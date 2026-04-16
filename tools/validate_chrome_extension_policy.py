#!/usr/bin/env python3
"""Static guardrails for the Cortex Chrome extension Web Store package."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ALLOWED_PERMISSIONS = {"storage", "contextMenus"}
ALLOWED_LOOPBACK_HOSTS = {"http://127.0.0.1/*", "http://localhost/*"}
DISALLOWED_JS_PATTERNS = (
    (re.compile(r"\beval\s*\("), "use of eval() is disallowed"),
    (re.compile(r"\bnew\s+Function\s*\("), "use of new Function() is disallowed"),
    (
        re.compile(r"importScripts\s*\(\s*['\"]https?://", re.IGNORECASE),
        "remote importScripts() URL is disallowed",
    ),
)
DISALLOWED_HTML_SCRIPT_SRC = re.compile(
    r"<script[^>]+src=['\"]https?://", re.IGNORECASE
)


def _load_manifest(path: Path) -> dict:
    with path.open("r", encoding="utf-8") as handle:
        return json.load(handle)


def _validate_manifest(manifest: dict, errors: list[str]) -> None:
    if manifest.get("manifest_version") != 3:
        errors.append("manifest_version must be 3.")

    permissions = manifest.get("permissions", [])
    if not isinstance(permissions, list):
        errors.append("permissions must be an array.")
    else:
        unknown = sorted(set(str(item) for item in permissions) - ALLOWED_PERMISSIONS)
        if unknown:
            errors.append(
                f"permissions contains non-approved entries for Web Store build: {unknown}"
            )

    host_permissions = manifest.get("host_permissions", [])
    if not isinstance(host_permissions, list):
        errors.append("host_permissions must be an array.")
    else:
        host_set = set(str(item) for item in host_permissions)
        if host_set != ALLOWED_LOOPBACK_HOSTS:
            errors.append(
                "host_permissions must be exactly loopback-only "
                "['http://127.0.0.1/*', 'http://localhost/*']."
            )

    optional_hosts = manifest.get("optional_host_permissions", [])
    if optional_hosts:
        errors.append("optional_host_permissions must be omitted/empty in Web Store build.")

    background = manifest.get("background", {})
    if not isinstance(background, dict) or not background.get("service_worker"):
        errors.append("background.service_worker is required.")


def _scan_html_for_remote_script(html_paths: list[Path], errors: list[str]) -> None:
    for html_path in html_paths:
        text = html_path.read_text(encoding="utf-8")
        if DISALLOWED_HTML_SCRIPT_SRC.search(text):
            errors.append(f"{html_path}: remote <script src> is disallowed.")


def _scan_js_for_dynamic_code(js_paths: list[Path], errors: list[str]) -> None:
    for js_path in js_paths:
        text = js_path.read_text(encoding="utf-8")
        for pattern, message in DISALLOWED_JS_PATTERNS:
            if pattern.search(text):
                errors.append(f"{js_path}: {message}.")


def _validate_required_policy_docs(extension_dir: Path, errors: list[str]) -> None:
    required = (
        extension_dir / "POLICY_COMPLIANCE.md",
        extension_dir / "PRIVACY_POLICY.md",
        extension_dir / "README.md",
    )
    for doc_path in required:
        if not doc_path.exists():
            errors.append(f"missing required policy/readme doc: {doc_path}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--extension-dir",
        default="extensions/cortex-chrome-extension",
        help="Path to extension root (default: extensions/cortex-chrome-extension)",
    )
    args = parser.parse_args()

    extension_dir = Path(args.extension_dir).resolve()
    manifest_path = extension_dir / "manifest.json"

    if not extension_dir.exists():
        print(f"error: extension dir does not exist: {extension_dir}", file=sys.stderr)
        return 2
    if not manifest_path.exists():
        print(f"error: missing manifest: {manifest_path}", file=sys.stderr)
        return 2

    errors: list[str] = []
    manifest = _load_manifest(manifest_path)
    _validate_manifest(manifest, errors)
    _validate_required_policy_docs(extension_dir, errors)

    html_paths = sorted(extension_dir.rglob("*.html"))
    js_paths = sorted(extension_dir.rglob("*.js"))
    _scan_html_for_remote_script(html_paths, errors)
    _scan_js_for_dynamic_code(js_paths, errors)

    if errors:
        print("Chrome extension policy validation failed:")
        for item in errors:
            print(f"- {item}")
        return 1

    print("Chrome extension policy validation passed.")
    print(f"Checked extension: {extension_dir}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
