#!/usr/bin/env bash
# Syncs docs/SECURITY-RULES.md content into CLAUDE.md, AGENTS.md, GEMINI.md
# between <!-- SECURITY-RULES:START --> and <!-- SECURITY-RULES:END --> markers.
# Run: bash tools/sync-security-rules.sh (or via pre-commit hook)

set -euo pipefail
cd "$(git -C "$(dirname "$0")/.." rev-parse --show-toplevel)"

SOURCE="docs/SECURITY-RULES.md"
TARGETS=("CLAUDE.md" "AGENTS.md" "GEMINI.md")
START_MARKER="<!-- SECURITY-RULES:START"
END_MARKER="<!-- SECURITY-RULES:END -->"

if [ ! -f "$SOURCE" ]; then
  echo "ERROR: $SOURCE not found" >&2
  exit 1
fi

# Build the replacement block (skip frontmatter/title, keep rules)
RULES=$(sed -n '/^## /,$ p' "$SOURCE")
BLOCK="$START_MARKER (auto-synced from $SOURCE -- do not edit here) -->
$RULES
$END_MARKER"

changed=0
for target in "${TARGETS[@]}"; do
  if [ ! -f "$target" ]; then
    echo "SKIP: $target not found"
    continue
  fi
  if ! grep -q "$START_MARKER" "$target"; then
    echo "SKIP: $target has no sync markers"
    continue
  fi

  # Replace content between markers
  awk -v block="$BLOCK" -v start="$START_MARKER" -v end="$END_MARKER" '
    $0 ~ start { print block; skip=1; next }
    $0 ~ end { skip=0; next }
    !skip { print }
  ' "$target" > "${target}.tmp"

  if ! diff -q "$target" "${target}.tmp" > /dev/null 2>&1; then
    mv "${target}.tmp" "$target"
    echo "SYNCED: $target"
    changed=1
  else
    rm "${target}.tmp"
  fi
done

if [ "$changed" -eq 1 ]; then
  echo "Security rules synced. Stage the updated files."
fi
