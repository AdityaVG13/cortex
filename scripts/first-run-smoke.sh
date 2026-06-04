#!/usr/bin/env bash
set -eu
if (set -o pipefail) >/dev/null 2>&1; then
  set -o pipefail
fi

CORTEX_BIN="${CORTEX_BIN:-cortex}"
AGENT="${CORTEX_AGENT:-cortex-onboarding-smoke}"

PYTHON_CMD=""
if command -v python3 >/dev/null 2>&1; then
  PYTHON_CMD="python3"
elif command -v python >/dev/null 2>&1; then
  PYTHON_CMD="python"
else
  echo "python3 or python is required for JSON parsing." >&2
  exit 1
fi

if ! command -v curl >/dev/null 2>&1; then
  echo "curl is required for the first-run smoke." >&2
  exit 1
fi

status_file="$(mktemp)"
stderr_file="$(mktemp)"
fields_file="$(mktemp)"
store_file="$(mktemp)"
recall_file="$(mktemp)"

cleanup() {
  rm -f "${status_file}" "${stderr_file}" "${fields_file}" "${store_file}" "${recall_file}"
}
trap cleanup EXIT

set +e
"${CORTEX_BIN}" status --json >"${status_file}" 2>"${stderr_file}"
status_code=$?
set -e

if ! "${PYTHON_CMD}" - "${status_file}" >"${fields_file}" <<'PY'; then
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    payload = json.load(handle)

def nested(*keys):
    current = payload
    for key in keys:
        if not isinstance(current, dict):
            return ""
        current = current.get(key)
    return "" if current is None else str(current)

for value in (
    nested("status"),
    nested("nextAction", "label"),
    nested("repair", "command"),
    nested("runtime", "tokenPath"),
    nested("runtime", "baseUrl"),
):
    print(value)
PY
  echo "cortex status --json did not return parseable JSON. Stdout:" >&2
  cat "${status_file}" >&2
  echo "Stderr:" >&2
  cat "${stderr_file}" >&2
  exit 1
fi

status="$(sed -n '1p' "${fields_file}")"
next_action="$(sed -n '2p' "${fields_file}")"
repair_command="$(sed -n '3p' "${fields_file}")"
token_path="$(sed -n '4p' "${fields_file}")"
base_url="$(sed -n '5p' "${fields_file}")"

if [ "${status}" != "ready" ]; then
  echo "Cortex smoke blocked: ${status:-unknown}"
  echo "Next action: ${next_action:-Run cortex status --json and inspect repair.}"
  if [ -n "${repair_command}" ]; then
    echo "Repair command: ${repair_command}"
  fi
  exit 1
fi

if [ "${status_code}" -ne 0 ]; then
  echo "cortex status --json reported ready but exited ${status_code}." >&2
  cat "${stderr_file}" >&2
  exit 1
fi

if [ -z "${token_path}" ] || [ ! -f "${token_path}" ]; then
  echo "Token path from status does not exist: ${token_path}" >&2
  exit 1
fi

token="$(tr -d '\r\n' < "${token_path}")"
if [ -z "${token}" ]; then
  echo "Token path is empty: ${token_path}" >&2
  exit 1
fi

base_url="${base_url%/}"
stamp="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
decision="Cortex first-run smoke stored at ${stamp}"
query="first-run smoke stored ${stamp}"

body="$("${PYTHON_CMD}" - "${decision}" "${AGENT}" <<'PY'
import json
import sys

print(json.dumps({
    "decision": sys.argv[1],
    "context": "Disposable onboarding smoke memory; safe to archive later.",
    "type": "memory",
    "source_agent": sys.argv[2],
}))
PY
)"

curl -fsS \
  -X POST "${base_url}/store" \
  -H "Authorization: Bearer ${token}" \
  -H "X-Cortex-Request: true" \
  -H "X-Source-Agent: ${AGENT}" \
  -H "Content-Type: application/json" \
  --data "${body}" \
  >"${store_file}"

query_encoded="$("${PYTHON_CMD}" - "${query}" <<'PY'
import sys
import urllib.parse

print(urllib.parse.quote(sys.argv[1]))
PY
)"

curl -fsS \
  -H "Authorization: Bearer ${token}" \
  -H "X-Cortex-Request: true" \
  -H "X-Source-Agent: ${AGENT}" \
  "${base_url}/recall?q=${query_encoded}&k=3&budget=200" \
  >"${recall_file}"

result_count="$("${PYTHON_CMD}" - "${recall_file}" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    payload = json.load(handle)

results = payload.get("results") or []
print(len(results))
PY
)"

if [ "${result_count}" -lt 1 ]; then
  echo "Recall returned no results for smoke query." >&2
  exit 1
fi

echo "Cortex first-run smoke passed."
echo "Stored: ${decision}"
echo "Recalled results: ${result_count}"
