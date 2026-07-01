#!/usr/bin/env python3
"""Post-split fixes for remaining module refactor."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def strip_duplicate_import_block(path: Path, marker: str = "use super::*;\n") -> None:
    text = path.read_text(encoding="utf-8")
    idx = text.find(marker)
    if idx == -1:
        return
    after = text[idx + len(marker) :]
    if not after.startswith("use "):
        return
    # Drop duplicated import block until blank line before const/fn
    lines = after.splitlines(keepends=True)
    cut = 0
    for i, line in enumerate(lines):
        if line.startswith("use ") or line.strip() == "":
            cut = i + 1
            continue
        break
    path.write_text(text[: idx + len(marker)] + "".join(lines[cut:]), encoding="utf-8")


def strip_trailing_orphan(path: Path) -> None:
    text = path.read_text(encoding="utf-8").rstrip() + "\n"
    while True:
        stripped = text.rstrip("\n")
        if stripped.endswith("#[cfg(test)]"):
            text = stripped[: -len("#[cfg(test)]")].rstrip() + "\n"
            continue
        if stripped.endswith("///  5. Return prompt with compilation metadata and savings"):
            # orphan doc lines from mis-split compile docs
            lines = stripped.splitlines()
            while lines and (lines[-1].startswith("///") or lines[-1].strip() == ""):
                lines.pop()
            text = "\n".join(lines) + "\n"
            continue
        break
    path.write_text(text, encoding="utf-8")


def fix_cli_daemon_imports(path: Path) -> None:
    text = path.read_text(encoding="utf-8")
    text = text.replace("use super::boot::", "use crate::cli::boot::")
    text = text.replace("use super::cleanup::", "use crate::cli::cleanup::")
    text = text.replace("use super::common::", "use crate::cli::common::")
    path.write_text(text, encoding="utf-8")


def main() -> None:
    for name in ("session.rs", "run.rs"):
        strip_duplicate_import_block(ROOT / "src/mcp_proxy" / name)

    for name in ("packing.rs", "compile.rs"):
        strip_trailing_orphan(ROOT / "src/compiler" / name)

    strip_trailing_orphan(ROOT / "src/mcp_proxy/run.rs")
    strip_trailing_orphan(ROOT / "src/server/runtime.rs")

    compile = ROOT / "src/compiler/compile.rs"
    doc = (
        "/// Compile the boot prompt for an agent within a token budget.\n"
        "///\n"
        "/// Prompt Compiler Pipeline (v3 -- score-adaptive context packing):\n"
        "///  1. Gather all context items with priority scores\n"
        "///  2. Sort by utility (priority / token_cost) -- best bang-per-token first\n"
        "///  3. Pack within budget using score-adaptive truncation when score variance exists\n"
        "///  4. Record admitted vs rejected for observability\n"
        "///  5. Return prompt with compilation metadata and savings\n"
    )
    text = compile.read_text(encoding="utf-8")
    if doc.strip() not in text:
        text = text.replace("use super::*;\n", f"use super::*;\n{doc}", 1)
        compile.write_text(text, encoding="utf-8")

    (ROOT / "src/cli/daemon/mod.rs").write_text(
        """// SPDX-License-Identifier: MIT
mod startup;
mod run;
mod backfill;

pub(crate) use startup::*;
pub(crate) use run::*;
pub(crate) use backfill::*;
""",
        encoding="utf-8",
    )

    (ROOT / "src/mcp_proxy/mod.rs").write_text(
        """// SPDX-License-Identifier: MIT
mod session;
mod run;

#[cfg(test)]
mod tests;

pub(crate) use session::*;
pub(crate) use run::*;

pub use run::run;
""",
        encoding="utf-8",
    )

    for name in ("startup.rs", "run.rs", "backfill.rs"):
        fix_cli_daemon_imports(ROOT / "src/cli/daemon" / name)

    print("post-split fixes applied")


if __name__ == "__main__":
    main()
