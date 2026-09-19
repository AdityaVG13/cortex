use super::super::CortexRuntime;
use super::*;
use crate::db::records;
use asupersync::Cx;
use cortex_logic::assembly::{FactorCodec, expand_records, factor_records};
use cortex_logic::presence::CurrentEpochs;
use cortex_logic::protocol::ContextPresence;
use rusqlite::{TransactionBehavior, params};
use serde_json::Value;

mod learning;

impl CortexRuntime {
    pub async fn put_assembly(
        &self,
        cx: &Cx,
        spec: AssemblySpec,
    ) -> Result<StoredAssembly, String> {
        let mut spec = spec;
        spec.scope = canonical_scope(&spec.scope)?;
        check_id(&spec.id)?;
        check_id(&spec.kind)?;
        if spec.members.is_empty() || spec.members.len() > 32 {
            return Err("invalid_assembly_members".into());
        }
        if spec.guards.len() > 32 {
            return Err("invalid_assembly_guards".into());
        }
        self.with_db_tx(cx, TransactionBehavior::Immediate, ensure, |tx, principal| {
            let bodies = load_member_bodies(tx, &spec.members)?;
            let envelope = factor_records(&bodies);
            let codec = envelope.get("codec").and_then(Value::as_str).ok_or("unknown_factor_codec")?;
            FactorCodec::parse(codec)?;
            let sequence = records::append_ack_commit(tx, principal)?;
            tx.execute("INSERT INTO assemblies(assembly_id,principal,scope_label,kind,created_sequence) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(principal,assembly_id) DO UPDATE SET kind=excluded.kind, scope_label=excluded.scope_label", params![spec.id, principal, spec.scope, spec.kind, sequence]).map_err(|err| err.to_string())?;
            // Route edges are keyed by (principal, scope_label, cue, assembly_id).
            // Relocating an assembly across scopes must not leave stale edges under
            // the previous label, or ranking would credit the new row in the old scope.
            tx.execute("DELETE FROM assembly_route_edges WHERE principal=?1 AND assembly_id=?2 AND scope_label!=?3", params![principal, spec.id, spec.scope]).map_err(|err| err.to_string())?;
            let revision_id = format!("{}@{}", spec.id, sequence);
            tx.execute("INSERT INTO assembly_revisions VALUES(?1,?2,?3,?4,?5,?6)", params![revision_id, principal, spec.id, codec, envelope.to_string(), sequence]).map_err(|err| err.to_string())?;
            for (ordinal, member) in spec.members.iter().enumerate() {
                tx.execute("INSERT INTO assembly_members VALUES(?1,?2,?3,?4)", params![revision_id, member.revision_id, ordinal as i64, member.role.as_str()]).map_err(|err| err.to_string())?;
            }
            for guard in &spec.guards {
                check_id(&guard.kind)?;
                check_id(&guard.key)?;
                check_id(&guard.epoch)?;
                tx.execute("INSERT INTO assembly_guards VALUES(?1,?2,?3,?4)", params![revision_id, guard.kind, guard.key, guard.epoch]).map_err(|err| err.to_string())?;
            }
            Ok(StoredAssembly { id: spec.id, revision_id, principal: principal.to_string(), scope: spec.scope, kind: spec.kind, codec: codec.into(), members: spec.members, guards: spec.guards, envelope })
        }).await
    }

    pub async fn get_assembly(&self, cx: &Cx, id: &str) -> Result<StoredAssembly, String> {
        check_id(id)?;
        self.with_locked_db(cx, |conn, principal| {
            ensure(conn)?;
            let revision = current_revision(conn, principal, id)?.ok_or("assembly_missing")?;
            stored_from_revision(conn, principal, &revision)
        })
        .await
    }

    pub async fn expand_assembly(&self, cx: &Cx, id: &str) -> Result<Vec<Value>, String> {
        let stored = self.get_assembly(cx, id).await?;
        let expanded = expand_records(&stored.envelope)?;
        if expanded.len() != stored.members.len() {
            return Err("assembly_expand_mismatch".into());
        }
        Ok(expanded)
    }

    pub async fn rebuild_assembly_routes(&self, cx: &Cx, scope: &str) -> Result<usize, String> {
        let scope = canonical_scope(scope)?;
        self.with_db_tx(
            cx,
            TransactionBehavior::Immediate,
            ensure,
            |tx, principal| {
                crate::runtime::upsert_scope_enabled(
                    tx,
                    "assembly_route_state",
                    principal,
                    &scope,
                    1,
                )?;
                refresh_routes(tx, principal, &scope, chrono::Utc::now().timestamp())
            },
        )
        .await
    }

    pub async fn reset_assembly_routes(&self, cx: &Cx, scope: &str) -> Result<(), String> {
        let scope = canonical_scope(scope)?;
        self.with_db_tx(
            cx,
            TransactionBehavior::Immediate,
            ensure,
            |tx, principal| {
                crate::runtime::upsert_scope_enabled(
                    tx,
                    "assembly_route_state",
                    principal,
                    &scope,
                    0,
                )?;
                tx.execute(
                    "DELETE FROM assembly_route_edges WHERE principal=?1 AND scope_label=?2",
                    params![principal, scope],
                )
                .map_err(|err| err.to_string())?;
                Ok(())
            },
        )
        .await
    }

    pub async fn explain_assembly_routes(
        &self,
        cx: &Cx,
        scope: &str,
        cues: &[String],
        limit: usize,
    ) -> Result<Vec<RouteExplanation>, String> {
        let scope = canonical_scope(scope)?;
        self.with_locked_db(cx, |conn, principal| {
            ensure(conn)?;
            if !routes_enabled(conn, principal, &scope)? {
                return Ok(Vec::new());
            }
            let edges = load_route_edges(conn, principal, &scope)?;
            let allowed = load_allowed_assemblies(conn, principal, &scope)?;
            let events = live_events(conn, principal, &scope)?;
            Ok(ranked_route_explanations(
                &edges, cues, &allowed, &events, limit, false,
            ))
        })
        .await
    }

    pub async fn compile_assemblies(
        &self,
        cx: &Cx,
        scope: &str,
        cues: &[String],
        limit: usize,
        presence: Option<&ContextPresence>,
        attested_brain: Option<&str>,
        attested_policy: Option<&str>,
        context_epoch: &str,
    ) -> Result<AssemblyCompilation, String> {
        let scope = canonical_scope(scope)?;
        self.compile_assemblies_for_paths(
            cx,
            &[],
            Some(&scope),
            cues,
            limit,
            presence,
            attested_brain,
            attested_policy,
            context_epoch,
        )
        .await
    }

    /// Evidence-closed bundles for caller project roots. Path-scoped
    /// assemblies stay in their repository; the extra label (default
    /// `project`) remains the unscoped bucket when roots are named.
    pub async fn compile_assemblies_for_paths(
        &self,
        cx: &Cx,
        paths: &[String],
        extra_scope: Option<&str>,
        cues: &[String],
        limit: usize,
        presence: Option<&ContextPresence>,
        attested_brain: Option<&str>,
        attested_policy: Option<&str>,
        context_epoch: &str,
    ) -> Result<AssemblyCompilation, String> {
        let display = extra_scope
            .map(crate::runtime::observation::normalize_scope)
            .filter(|scope| !scope.is_empty())
            .unwrap_or_else(|| "project".into());
        self.with_locked_db(cx, |conn, principal| {
            ensure(conn)?;
            let (_, restore, policy) = records::brain_epochs(conn);
            let current = CurrentEpochs {
                brain_epoch: restore,
                policy_epoch: policy,
            };
            let scopes = resolve_compile_scopes(conn, principal, paths, extra_scope)?;
            let mut parts = Vec::new();
            for scope in scopes {
                check_id(&scope)?;
                parts.push(compile_for_cues(
                    conn,
                    principal,
                    &scope,
                    cues,
                    limit,
                    presence,
                    attested_brain,
                    attested_policy,
                    &current,
                    context_epoch,
                )?);
            }
            Ok(merge_compilations(parts, &display, limit))
        })
        .await
    }
}
