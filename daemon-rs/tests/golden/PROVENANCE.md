# Golden Test Provenance

These goldens freeze Cortex CLI contract output that is intended for humans and coding agents.

## Regeneration

From `daemon-rs/`:

```powershell
$env:UPDATE_GOLDENS='1'
cargo test --test cli_goldens
Remove-Item Env:\UPDATE_GOLDENS
cargo test --test cli_goldens
```

Review every changed file under `tests/golden/` before committing.

## Stability Matrix

| Artifact | Deterministic | Platform-dependent | Volatility | Strategy |
| --- | --- | --- | --- | --- |
| `cli/help.golden` | Yes | Low | 2 | canonicalized exact text |
| `cli/capabilities_json.golden` | Yes | Low | 3 | canonicalized exact JSON text |
| `cli/robot_docs_guide.golden` | Yes | Low | 2 | canonicalized exact text |
| `cli/unknown_command_capability_stderr.golden` | Yes | Low | 2 | canonicalized exact text |

The test harness normalizes CRLF to LF, path separators to `/`, trims trailing whitespace, and writes transient `.actual` files on mismatch.
