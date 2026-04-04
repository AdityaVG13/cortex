#!/usr/bin/env bash
set -euo pipefail

AGENT="${1:-gemini-cli}"
TOKEN_FILE="${HOME}/.cortex/cortex.token"
OUT_FILE="${2:-./gemini.boot.json}"

if [[ ! -f "${TOKEN_FILE}" ]]; then
  echo "missing token file: ${TOKEN_FILE}" >&2
  exit 1
fi

TOKEN="$(tr -d '\r\n' < "${TOKEN_FILE}")"

curl -sS \
  -H "Authorization: Bearer ${TOKEN}" \
  -H "X-Cortex-Request: true" \
  "http://127.0.0.1:7437/boot?agent=${AGENT}&budget=600" \
  > "${OUT_FILE}"

echo "wrote ${OUT_FILE}"
