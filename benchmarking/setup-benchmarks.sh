#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$ROOT"
LOCK="$ROOT/benchmarks.lock.json"
TOOLS="$ROOT/tools"

if [[ ! -f "$LOCK" ]]; then
  echo "Missing lock file: $LOCK" >&2
  exit 1
fi

mkdir -p "$TOOLS" "$ROOT/runs"

read -r name url commit < <(python - <<PY
import json
lock = json.loads(open("$LOCK").read())
tool = lock["tools"][0]
print(tool["name"], tool["url"], tool["commit"])
PY
)

target="$TOOLS/$name"
if [[ -d "$target/.git" ]]; then
  echo "Updating $name..."
  git -C "$target" fetch --all --tags --prune
else
  echo "Cloning $name..."
  git clone "$url" "$target"
fi
git -C "$target" checkout "$commit"
echo "Benchmark harness ready at $target"
