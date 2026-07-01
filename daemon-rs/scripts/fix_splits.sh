#!/bin/bash
# Post-split fixes for daemon-rs module refactor
set -e
cd "$(dirname "$0")/.."

# store mod.rs
cat > src/handlers/store/mod.rs <<'EOF'
// SPDX-License-Identifier: MIT
mod core; mod embedding; mod handler; mod insert; mod merge; mod policies; mod types;
#[cfg(test)] mod tests;
pub(crate) use types::*; pub(crate) use core::*; pub(crate) use policies::*;
pub(crate) use insert::*; pub(crate) use merge::*;
pub use handler::handle_store;
pub use core::{store_decision, store_decision_with_ttl};
pub use embedding::persist_decision_embedding;
pub(crate) use core::{store_decision_with_input_embedding, store_decision_with_input_embedding_and_provenance, store_decision_with_input_embedding_and_provenance_retention};
pub(crate) use types::{DecisionProvenance, validate_explicit_ttl_seconds};
EOF

# health mod.rs
cat > src/handlers/health/mod.rs <<'EOF'
// SPDX-License-Identifier: MIT
mod digest; mod dump; mod health; mod metrics; mod savings; mod savings_build; mod savings_stats; mod stats;
#[cfg(test)] mod tests;
pub use digest::{build_digest, handle_digest};
pub use dump::handle_dump;
pub use health::{build_health_payload, build_readiness_payload, handle_health, handle_readiness};
pub use savings::handle_savings;
pub use stats::handle_stats;
pub(crate) use metrics::*;
pub(crate) use savings_build::*;
pub(crate) use savings_stats::*;
pub(crate) use health::{include_private_runtime_details, redact_private_runtime_details};
EOF

# db fixes
sed -i 's/^static LAST_BEST_EFFORT/pub(crate) static LAST_BEST_EFFORT/g; s/^type MigrationDef/pub(crate) type MigrationDef/g' src/db/connection.rs 2>/dev/null || true

# store orphan attributes
for f in src/handlers/store/handler.rs src/handlers/store/core.rs src/handlers/store/policies.rs src/handlers/store/insert.rs; do
  python3 -c "
from pathlib import Path
p=Path('$f')
ls=p.read_text().splitlines()
while ls and ls[-1].strip().startswith('#['): ls.pop()
p.write_text('\n'.join(ls)+'\n')
" 2>/dev/null || true
done

# db orphan docs
for f in src/db/team.rs src/db/migrations.rs src/db/schema.rs src/compiler/core.rs; do
  python3 -c "
from pathlib import Path
p=Path('$f')
ls=p.read_text().splitlines()
while ls and (ls[-1].strip().startswith('///') or ls[-1].strip()==''):
  if ls[-1].strip().startswith('///'): ls.pop(); break
  ls.pop()
p.write_text('\n'.join(ls)+'\n')
" 2>/dev/null || true
done

# mcp imports in submodules
for f in src/handlers/mcp/dispatch.rs src/handlers/mcp/handler.rs src/handlers/mcp/session.rs src/handlers/mcp/rpc.rs src/handlers/mcp/permissions.rs; do
  [ -f "$f" ] || continue
done

# server router fix
if [ -f src/server/router.rs ] && tail -1 src/server/router.rs | grep -q '} else {'; then
  printf '        None\n    }\n}\n' >> src/server/router.rs
fi
sed -i 's/async pub(crate) fn /pub(crate) async fn /g' src/server/*.rs src/mcp_proxy/*.rs 2>/dev/null || true
sed -i '/use tower_http::trace::TraceLayer/d' src/server/*.rs 2>/dev/null || true

# compaction/db pub exports -> pub(crate) items need pub on exported types
for f in src/compaction/events.rs src/compaction/types.rs src/db/maintenance.rs; do
  sed -i 's/pub(crate) struct \(CompactionResult\|BenchmarkPurgeResult\|RepairResult\|ExpiredCleanupCounts\)/pub struct \1/g' "$f" 2>/dev/null || true
  sed -i 's/pub(crate) enum RepairError/pub enum RepairError/g' "$f" 2>/dev/null || true
  sed -i 's/pub(crate) fn \(run_compaction\|purge_benchmark_artifacts\|initialize_schema\)/pub fn \1/g' "$f" 2>/dev/null || true
done

# mutate exports
sed -i 's/pub(crate) fn parse_conflict_id/pub fn parse_conflict_id/g' src/handlers/mutate/conflicts.rs 2>/dev/null || true

# health/compiler struct fields
sed -i 's/    computed_at_unix_secs: i64/    pub(crate) computed_at_unix_secs: i64/g; s/    embedding_inventory: EmbeddingInventoryMetrics/    pub(crate) embedding_inventory: EmbeddingInventoryMetrics/g; s/    storage_bytes: u64/    pub(crate) storage_bytes: u64/g; s/    backup_count: usize/    pub(crate) backup_count: usize/g; s/    log_bytes: u64/    pub(crate) log_bytes: u64/g; s/    payload: Value/    pub(crate) payload: Value/g' src/handlers/health/metrics.rs 2>/dev/null || true

# mcp submodule cross-imports
python3 <<'PY'
from pathlib import Path
fixes = {
    'src/handlers/mcp/rpc.rs': 'use super::{mcp_tools, required_permission_for_tool, ClientPermission};\n',
    'src/handlers/mcp/permissions.rs': 'use super::arg_str;\n',
    'src/handlers/mcp/permissions.rs_impl': 'impl ClientPermission {\n    pub(crate) fn as_str',
    'src/handlers/mcp/dispatch.rs': '''use super::{arg_f64, arg_i64, arg_str, arg_usize, clear_served_scope_for_boot, enforce_client_permission, fetch_last_call, normalize_mcp_agent_label, normalize_permission_client_id, parse_client_permission, refresh_mcp_session_presence, source_agent_for_tool, source_client_for_permissions, source_model_for_tool, upsert_mcp_session, wrap_mcp_tool_result, wrap_mcp_tool_result_verbose, McpPresenceDisposition};\n''',
    'src/handlers/mcp/handler.rs': '''use super::{mcp_dispatch, mcp_error, mcp_error_with_data, mcp_resource_payload, mcp_resource_read_result, mcp_resources, mcp_resource_uris, mcp_success, mcp_tools, required_permission_for_tool, tool_name_suggestions, wrap_mcp_tool_result, wrap_mcp_tool_result_verbose};\n''',
    'src/handlers/mcp/session.rs': 'use super::{mcp_session_description, mcp_session_owner_id, normalize_mcp_agent_label};\n',
}
for path, ins in fixes.items():
    if path.endswith('_impl'): continue
    p = Path(path)
    if not p.exists(): continue
    t = p.read_text()
    if ins.strip() in t: continue
    marker = 'use super::*;\n'
    if marker in t:
        t = t.replace(marker, marker + ins, 1)
    elif 'use crate::{aging' in t:
        t = t.replace('use crate::{aging', ins + 'use crate::{aging', 1)
    p.write_text(t)
p = Path('src/handlers/mcp/permissions.rs')
if p.exists():
    t = p.read_text().replace('impl ClientPermission {\n    fn as_str', 'impl ClientPermission {\n    pub(crate) fn as_str')
    p.write_text(t)
PY

echo "fixes applied"
