# Golden Test Provenance

These goldens freeze Cortex CLI and adapter contract output that is intended for humans and coding agents.

## Regeneration

From the repository root:

```powershell
$env:UPDATE_GOLDENS='1'
cargo test -p cortex-tests --test cli_goldens
cargo test -p cortex-tests --test adapter_conformance cli_http_and_mcp_golden_summary_matches
Remove-Item Env:\UPDATE_GOLDENS
cargo test -p cortex-tests --test cli_goldens
cargo test -p cortex-tests --test adapter_conformance cli_http_and_mcp_golden_summary_matches
```

Review every changed file under `tests/golden/` before committing.

## Stability Matrix

| Artifact | Deterministic | Platform-dependent | Volatility | Strategy |
| --- | --- | --- | --- | --- |
| `cli/help.golden` | Yes | Low | 2 | canonicalized exact text |
| `cli/capabilities_json.golden` | Yes | Low | 3 | canonicalized exact JSON text |
| `cli/status_json_unavailable.golden` | Yes | Medium | 3 | scrubbed/canonicalized exact JSON text |
| `cli/robot_docs_guide.golden` | Yes | Low | 2 | canonicalized exact text |
| `cli/unknown_command_capability_stderr.golden` | Yes | Low | 2 | canonicalized exact text |
| `adapter/http_mcp_status_contract_summary.golden` | Yes | Medium | 4 | Tier 3 logical summary for live CLI `/status`, HTTP `/readiness`, `/health`, `/store`, GET/POST `/recall`, `/peek`, `/boot`, `/export`, `/import`, import recall, MCP `tools/list`, auth failures, and malformed MCP envelopes |

The test harness normalizes CRLF to LF, path separators to `/`, trims trailing whitespace, scrubs the `status --json` scratch `CORTEX_HOME` path to `[CORTEX_HOME]`, replaces live adapter ports with `[PORT]`, and writes transient `.actual` files on mismatch.
