#!/usr/bin/env python3
"""Drift detector -- checks MEMORY.md, CLAUDE.md, and rules for stale references.

Inspired by mex (323 stars). Validates that instructions, memory files, and
rules reference things that still exist in the codebase.

Usage:
    python drift_detector.py              # Full scan
    python drift_detector.py --memory     # Memory files only
    python drift_detector.py --rules      # Rules + CLAUDE.md only
"""

import json
import re
import subprocess
import sys
from pathlib import Path

HOME = Path.home()
CLAUDE_DIR = HOME / ".claude"
MEMORY_DIR = HOME / ".claude" / "projects" / "C--Users-aditya" / "memory"
RULES_DIR = HOME / ".claude" / "rules"
SKILLS_DIR = HOME / ".claude" / "skills"


def check_path_exists(path_str: str) -> bool:
    """Check if a referenced path exists, handling ~ and env vars."""
    expanded = path_str.replace("~", str(HOME))
    expanded = expanded.replace("$HOME", str(HOME))
    expanded = expanded.replace("%USERPROFILE%", str(HOME))
    return Path(expanded).exists()


def extract_paths(text: str) -> list[str]:
    """Extract file paths from text."""
    patterns = [
        r"~/[\w./-]+",
        r"~\\[\w.\\-]+",
        r"C:[/\\]Users[/\\]\w+[/\\][\w./\\-]+",
        r"/c/Users/\w+/[\w./-]+",
        r"\.claude/[\w./-]+",
    ]
    paths = []
    for pattern in patterns:
        for match in re.finditer(pattern, text):
            p = match.group()
            if len(p) > 5 and not p.endswith("/") and "." in p.split("/")[-1]:
                paths.append(p)
    return list(set(paths))


def extract_commands(text: str) -> list[str]:
    """Extract CLI commands referenced in backticks."""
    commands = []
    for match in re.finditer(r"`(\w[\w.-]+)`", text):
        cmd = match.group(1)
        if cmd in ("cortex_recall", "cortex_store", "cortex_peek", "cortex_unfold",
                    "cortex_boot", "cortex_health", "cortex_diary", "cortex_forget",
                    "cortex_focus_start", "cortex_focus_end", "cortex_resolve"):
            commands.append(cmd)
    return list(set(commands))


def check_memory_links(memory_index: Path) -> list[dict]:
    """Check MEMORY.md for broken links to memory files."""
    issues = []
    if not memory_index.exists():
        return [{"type": "missing", "file": str(memory_index), "message": "MEMORY.md not found"}]

    text = memory_index.read_text(encoding="utf-8")
    for match in re.finditer(r"\[([^\]]+)\]\(([^)]+)\)", text):
        title, link = match.groups()
        target = memory_index.parent / link
        if not target.exists():
            issues.append({
                "type": "broken_link",
                "file": "MEMORY.md",
                "link": link,
                "title": title,
                "message": f"Link target missing: {link}",
            })
    return issues


def check_memory_files(memory_dir: Path) -> list[dict]:
    """Check individual memory files for stale content."""
    issues = []
    if not memory_dir.exists():
        return []

    for md_file in memory_dir.glob("*.md"):
        if md_file.name == "MEMORY.md":
            continue

        text = md_file.read_text(encoding="utf-8")

        # Check for referenced paths that don't exist
        for path in extract_paths(text):
            if not check_path_exists(path):
                issues.append({
                    "type": "stale_path",
                    "file": md_file.name,
                    "path": path,
                    "message": f"Referenced path doesn't exist: {path}",
                })

        # Check for orphaned memory files (not linked from MEMORY.md)
        memory_index = memory_dir / "MEMORY.md"
        if memory_index.exists():
            index_text = memory_index.read_text(encoding="utf-8")
            if md_file.name not in index_text:
                issues.append({
                    "type": "orphan",
                    "file": md_file.name,
                    "message": f"Not linked from MEMORY.md",
                })

    return issues


def check_skills_exist(text: str, source: str) -> list[dict]:
    """Check if referenced skills still exist."""
    issues = []
    for match in re.finditer(r"skills/(\w[\w-]+)", text):
        skill_name = match.group(1)
        skill_dir = SKILLS_DIR / skill_name
        if not skill_dir.exists():
            issues.append({
                "type": "missing_skill",
                "file": source,
                "skill": skill_name,
                "message": f"Referenced skill doesn't exist: {skill_name}",
            })
    return issues


def check_rules(rules_dir: Path) -> list[dict]:
    """Check rules files for stale references."""
    issues = []
    if not rules_dir.exists():
        return []

    for rule_file in rules_dir.glob("*.md"):
        text = rule_file.read_text(encoding="utf-8")
        for path in extract_paths(text):
            if not check_path_exists(path):
                issues.append({
                    "type": "stale_path",
                    "file": f"rules/{rule_file.name}",
                    "path": path,
                    "message": f"Referenced path doesn't exist: {path}",
                })
    return issues


def check_claude_md() -> list[dict]:
    """Check CLAUDE.md for stale references."""
    issues = []
    claude_md = CLAUDE_DIR / "CLAUDE.md"
    if not claude_md.exists():
        return []

    text = claude_md.read_text(encoding="utf-8")

    for path in extract_paths(text):
        if not check_path_exists(path):
            issues.append({
                "type": "stale_path",
                "file": "CLAUDE.md",
                "path": path,
                "message": f"Referenced path doesn't exist: {path}",
            })

    issues.extend(check_skills_exist(text, "CLAUDE.md"))
    return issues


def run_full_scan() -> list[dict]:
    """Run all drift checks."""
    all_issues = []
    all_issues.extend(check_memory_links(MEMORY_DIR / "MEMORY.md"))
    all_issues.extend(check_memory_files(MEMORY_DIR))
    all_issues.extend(check_rules(RULES_DIR))
    all_issues.extend(check_claude_md())
    return all_issues


if __name__ == "__main__":
    args = set(sys.argv[1:])

    if "--memory" in args:
        issues = check_memory_links(MEMORY_DIR / "MEMORY.md") + check_memory_files(MEMORY_DIR)
    elif "--rules" in args:
        issues = check_rules(RULES_DIR) + check_claude_md()
    else:
        issues = run_full_scan()

    if not issues:
        print("No drift detected. All references valid.")
        sys.exit(0)

    # Group by type
    by_type: dict[str, list] = {}
    for issue in issues:
        t = issue["type"]
        by_type.setdefault(t, []).append(issue)

    print(f"=== Drift Report: {len(issues)} issues ===\n")

    for issue_type, items in sorted(by_type.items()):
        print(f"## {issue_type.replace('_', ' ').title()} ({len(items)})")
        for item in items:
            print(f"  {item['file']}: {item['message']}")
        print()

    sys.exit(1)
