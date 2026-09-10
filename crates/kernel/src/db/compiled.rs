//! Compiled reads: cached recipe results guarded by positive dependencies
//! (exact revisions), negative predicate/range dependencies (guard epochs
//! per (scope, relation)), brain/policy epochs and an environment
//! fingerprint. Guard epochs are advanced inside the authoritative write
//! transaction; a coarse per-scope epoch is bumped alongside every relation
//! epoch so an unknown range can always fall back to over-invalidation.

use super::records::{brain_epochs, DEFAULT_SCOPE};
use crate::recipe::{evaluate, Fact, Limits, RecipeResult, Snapshot, Step};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Advance the guard epoch of one (scope, relation) domain plus the coarse
/// scope-wide epoch. Called by every mutation that can change a searched
/// range: over-invalidation is safe, under-invalidation is not.
pub fn bump_guard(conn: &Connection, scope: &str, relation: &str) -> rusqlite::Result<()> {
    for key in [relation, "*"] {
        conn.execute(
            "INSERT INTO guard_epochs (scope_id, guard_key, generation) VALUES (?1, ?2, 1) ON CONFLICT(scope_id, guard_key) DO UPDATE SET generation = generation + 1",
            params![scope, key],
        )?;
    }
    Ok(())
}

pub fn guard_epoch(conn: &Connection, scope: &str, relation: &str) -> i64 {
    conn.query_row(
        "SELECT generation FROM guard_epochs WHERE scope_id = ?1 AND guard_key = ?2",
        params![scope, relation],
        |r| r.get(0),
    )
    .optional()
    .ok()
    .flatten()
    .unwrap_or(0)
}

/// Build a recipe snapshot from the authoritative tables: every current
/// head revision is a fact whose relation is its record kind, with the
/// revision body's fields plus `text`, `status`, `record`, `revision`.
pub fn snapshot(conn: &Connection, environment: &str) -> rusqlite::Result<Snapshot> {
    let (_, restore_epoch, policy_epoch) = brain_epochs(conn);
    let mut snap = Snapshot {
        brain_epoch: restore_epoch,
        policy_epoch,
        environment: environment.to_string(),
        ..Snapshot::default()
    };
    let mut stmt = conn.prepare(
        "SELECT r.record_id, r.kind, h.revision_id, v.body_json, v.recorded_sequence, v.valid_from, v.valid_until, v.epistemic_status FROM records r JOIN record_heads h ON h.record_id = r.record_id JOIN revisions v ON v.revision_id = h.revision_id WHERE r.scope_id = ?1 ORDER BY r.record_id, h.revision_id",
    )?;
    let rows = stmt.query_map(params![DEFAULT_SCOPE], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, Option<i64>>(5)?,
            r.get::<_, Option<i64>>(6)?,
            r.get::<_, String>(7)?,
        ))
    })?;
    for row in rows {
        let (record, kind, revision, body, seq, valid_from, valid_until, epistemic) = row?;
        let body: Value = serde_json::from_str(&body).unwrap_or(json!({}));
        let mut fields: BTreeMap<String, Value> = body
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        fields.insert("record".into(), json!(record));
        fields.insert("revision".into(), json!(revision));
        fields.insert("epistemic".into(), json!(epistemic));
        snap.facts.insert(
            revision.clone(),
            Fact {
                id: revision,
                scope: DEFAULT_SCOPE.into(),
                relation: kind,
                fields,
                valid_from,
                valid_until,
                known_seq: seq,
            },
        );
    }
    let mut stmt = conn.prepare("SELECT scope_id, guard_key, generation FROM guard_epochs")?;
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })? {
        let (scope, key, generation) = row?;
        snap.epochs.insert((scope, key), generation);
    }
    Ok(snap)
}

fn compiled_id(recipe_id: &str, principal: &str, params_json: &str, environment: &str) -> String {
    let payload = format!("{recipe_id}|{principal}|{params_json}|{environment}");
    format!("compiled:{}", cortex_logic::traces::content_hash(&payload))
}

/// Evaluate (or reuse) a recipe. Identity = recipe + operator versions +
/// parameters + principal + brain epoch + policy epoch + environment; a
/// cached result for one scope/params can never answer another.
pub fn run_compiled(
    conn: &Connection,
    principal: &str,
    recipe_id: &str,
    steps: &[Step],
    outputs: &[String],
    params_value: &Value,
    environment: &str,
    limits: Limits,
) -> Result<(Value, bool, RecipeResult), String> {
    super::records::ensure_authoritative_schema(conn).map_err(|e| e.to_string())?;
    let snap = snapshot(conn, environment).map_err(|e| e.to_string())?;
    let params_json = params_value.to_string();
    let id = compiled_id(recipe_id, principal, &params_json, environment);
    // Reuse path: stored guards must all match the current epochs.
    let cached: Option<(String, String, String, String, i64)> = conn
        .query_row(
            "SELECT result_json, brain_epoch, policy_epoch, environment_ref, through_sequence FROM compiled_reads WHERE compiled_id = ?1 AND principal_id = ?2",
            params![id, principal],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some((result_json, brain, policy, env, _)) = cached {
        let mut valid =
            brain == snap.brain_epoch && policy == snap.policy_epoch && env == snap.environment;
        if valid {
            let mut stmt = conn
                .prepare("SELECT scope_id, guard_key, expected_generation, kind FROM compiled_guards WHERE compiled_id = ?1")
                .map_err(|e| e.to_string())?;
            let guards: Vec<(String, String, i64, String)> = stmt
                .query_map(params![id], |r| {
                    Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                })
                .map_err(|e| e.to_string())?
                .flatten()
                .collect();
            for (scope, key, expected, kind) in guards {
                let now = if kind == "positive" {
                    if snap.facts.contains_key(&key) {
                        expected
                    } else {
                        -1
                    }
                } else {
                    snap.epochs
                        .get(&(scope.clone(), key.clone()))
                        .copied()
                        .unwrap_or(0)
                };
                if now != expected {
                    valid = false;
                    break;
                }
            }
        }
        if valid {
            let cached_value: Value = serde_json::from_str(&result_json).unwrap_or(Value::Null);
            let result = evaluate(&snap, DEFAULT_SCOPE, steps, outputs, limits)
                .map_err(|e| e.to_string())?;
            return Ok((cached_value, true, result));
        }
        conn.execute(
            "DELETE FROM compiled_guards WHERE compiled_id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM compiled_reads WHERE compiled_id = ?1",
            params![id],
        )
        .map_err(|e| e.to_string())?;
    }
    let result =
        evaluate(&snap, DEFAULT_SCOPE, steps, outputs, limits).map_err(|e| e.to_string())?;
    let value = json!({"values": result.values, "work": result.work, "operator_versions": result.operator_versions, "positive": result.positive, "guards": result.guards.iter().map(|((s, k), g)| json!({"scope": s, "relation": k, "epoch": g})).collect::<Vec<_>>()});
    let through: i64 = conn
        .query_row("SELECT COALESCE(MAX(sequence),0) FROM commits", [], |r| {
            r.get(0)
        })
        .unwrap_or(0);
    conn.execute(
        "INSERT OR REPLACE INTO compiled_reads (compiled_id, recipe_id, operator_versions, scope_id, principal_id, brain_epoch, policy_epoch, parameters_json, environment_ref, through_sequence, result_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![id, recipe_id, result.operator_versions, DEFAULT_SCOPE, principal, result.brain_epoch, result.policy_epoch, params_json, environment, through, value.to_string()],
    )
    .map_err(|e| e.to_string())?;
    for ((scope, key), generation) in &result.guards {
        conn.execute("INSERT OR IGNORE INTO guard_epochs (scope_id, guard_key, generation) VALUES (?1, ?2, ?3)", params![scope, key, generation])
            .map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO compiled_guards (compiled_id, scope_id, guard_key, expected_generation, kind) VALUES (?1, ?2, ?3, ?4, 'negative')",
            params![id, scope, key, generation],
        )
        .map_err(|e| e.to_string())?;
    }
    for revision in &result.positive {
        conn.execute(
            "INSERT OR REPLACE INTO compiled_guards (compiled_id, scope_id, guard_key, expected_generation, kind) VALUES (?1, ?2, ?3, 1, 'positive')",
            params![id, DEFAULT_SCOPE, revision],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok((value, false, result))
}
