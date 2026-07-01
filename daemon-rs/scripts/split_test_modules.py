#!/usr/bin/env python3
"""Split recall and cli test megfiles into submodules."""
from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPDX = "// SPDX-License-Identifier: MIT\n"


def strip_test_wrapper(content: str) -> str:
    content = re.sub(r"^#\[cfg\(test\)\]\n", "", content)
    content = re.sub(r"^mod tests \{\n", "", content)
    s = content.rstrip()
    if s.endswith("}"):
        content = s[:-1] + "\n"
    return content


def write_test_module(path: Path, body: str, extra_use: str = "") -> None:
    path.write_text(
        SPDX
        + extra_use
        + "#[cfg(test)]\nmod tests {\n    use super::support::*;\n    use super::super::*;\n"
        + body
        + "}\n",
        encoding="utf-8",
    )


def split_recall_tests() -> None:
    src = ROOT / "src/handlers/recall/tests.rs"
    lines = src.read_text(encoding="utf-8").splitlines(keepends=True)
    inner = strip_test_wrapper("".join(lines[3:]))  # skip header + cfg + mod tests {

    # Shared helpers: everything before first `#[test]` in is_visible section marker area
    # Keep lines until line 365 (before first #[test] at search_memories) - use line index in inner
    inner_lines = inner.splitlines(keepends=True)

    def find_line(prefix: str) -> int:
        for i, line in enumerate(inner_lines):
            if prefix in line:
                return i
        raise ValueError(prefix)

    support_end = find_line("fn search_memories_excludes_temporally_invalid_rows")
    support = "".join(inner_lines[:support_end])
    rest = inner_lines[support_end:]

    sections: list[tuple[str, str | None, str | None]] = [
        ("search", "// ── existing tests", "// ── RRF fusion tests"),
        ("rrf", "// ── RRF fusion tests", "// ── compound scoring tests"),
        ("scoring", "// ── compound scoring tests", "// ── query cache tests"),
        ("cache", "// ── query cache tests", None),
    ]

    dest = ROOT / "src/handlers/recall/tests"
    dest.mkdir(exist_ok=True)

    (dest / "support.rs").write_text(
        SPDX + "use super::*;\n" + support,
        encoding="utf-8",
    )

    chunk = rest
    for name, start_marker, end_marker in sections:
        start = 0
        for i, line in enumerate(chunk):
            if start_marker in line:
                start = i
                break
        end = len(chunk)
        if end_marker:
            for i, line in enumerate(chunk):
                if end_marker in line:
                    end = i
                    break
        write_test_module(dest / f"{name}.rs", "".join(chunk[start:end]))
        if end_marker:
            chunk = chunk[end:]

    (dest / "mod.rs").write_text(
        SPDX
        + "mod support;\n"
        + "".join(f"mod {name};\n" for name, _, _ in sections)
        + "\n",
        encoding="utf-8",
    )
    src.unlink()
    print(f"split recall tests -> {dest.name}/")


def split_cli_tests() -> None:
    src = ROOT / "src/cli/tests.rs"
    lines = src.read_text(encoding="utf-8").splitlines(keepends=True)
    inner = strip_test_wrapper("".join(lines[2:]))

    inner_lines = inner.splitlines(keepends=True)

    def idx(substr: str, default: int | None = None) -> int:
        for i, line in enumerate(inner_lines):
            if substr in line:
                return i
        if default is not None:
            return default
        raise ValueError(substr)

    support_end = idx("fn cli_usage_exposes_agent_entrypoints")
    support = "".join(inner_lines[:support_end])

    sections = [
        ("usage", support_end, idx("fn backfill_batch_may_have_more")),
        ("embeddings", idx("fn backfill_batch_may_have_more"), idx("fn rotate_backups_keeps_three")),
        ("cleanup", idx("fn rotate_backups_keeps_three"), idx("fn acquire_runtime_lock_rejects_duplicate")),
        ("daemon", idx("fn acquire_runtime_lock_rejects_duplicate"), idx("fn parse_flag_usize_validates_and_parses_values")),
        ("sync", idx("fn parse_flag_usize_validates_and_parses_values"), idx("fn resolve_boot_auth_header_prefers_api_key")),
        ("client", idx("fn resolve_boot_auth_header_prefers_api_key"), len(inner_lines)),
    ]

    dest = ROOT / "src/cli/tests"
    dest.mkdir(exist_ok=True)
    (dest / "support.rs").write_text(
        SPDX + "use crate::cli::*;\nuse crate::*;\n" + support,
        encoding="utf-8",
    )

    for name, start, end in sections:
        write_test_module(
            dest / f"{name}.rs",
            "".join(inner_lines[start:end]),
            extra_use="use crate::cli::*;\nuse crate::*;\n",
        )

    (dest / "mod.rs").write_text(
        SPDX
        + "#[cfg(test)]\nmod support;\n"
        + "".join(f"#[cfg(test)]\nmod {name};\n" for name, _, _ in sections)
        + "\n",
        encoding="utf-8",
    )
    src.unlink()
    print(f"split cli tests -> {dest.name}/")


if __name__ == "__main__":
    split_recall_tests()
    split_cli_tests()
