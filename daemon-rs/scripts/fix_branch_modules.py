#!/usr/bin/env python3
"""Fix compile issues from parallel branch module splits."""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_in(path: str, old: str, new: str) -> None:
    p = ROOT / path
    if not p.exists():
        return
    text = p.read_text()
    if old in text:
        p.write_text(text.replace(old, new))


def main() -> None:
    replace_in(
        "src/db/connection.rs",
        "    pub(crate) fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {",
        "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {",
    )
    replace_in(
        "src/handlers/mutate/types.rs",
        "use rusqlite::{params, Connection};\nuse serde_json::{json, Value};",
        "use rusqlite::{params, Connection};\nuse serde::Deserialize;\nuse serde_json::{json, Value};",
    )
    replace_in(
        "src/handlers/mutate/types.rs",
        "    pub(crate) fn default() -> Self {",
        "    fn default() -> Self {",
    )
    replace_in(
        "src/handlers/store/types.rs",
        "    pub(crate) fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {",
        "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {",
    )
    replace_in(
        "src/handlers/store/types.rs",
        "    pub(crate) fn from(value: String) -> Self {",
        "    fn from(value: String) -> Self {",
    )


if __name__ == "__main__":
    main()
