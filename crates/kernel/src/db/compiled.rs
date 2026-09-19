//! Compiled reads: cached recipe results guarded by positive dependencies
//! (exact revisions), negative predicate/range dependencies (guard epochs
//! per (scope, relation)), brain/policy epochs and an environment
//! fingerprint. Guard epochs are advanced inside the authoritative write
//! transaction; a coarse per-scope epoch is bumped alongside every relation
//! epoch so an unknown range can always fall back to over-invalidation.

use super::records::{DEFAULT_SCOPE, brain_epochs};
use crate::recipe::{Fact, Limits, RecipeResult, Snapshot, Step, evaluate};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

/// Advance the guard epoch of one (scope, relation) domain plus the coarse
/// scope-wide epoch. Called by every mutation that can change a searched
/// range: over-invalidation is safe, under-invalidation is not.
pub fn bump_guard(conn: &Connection, scope: &str, relation: &str) -> rusqlite::Result<()> {
    for key in [relation, "*"] {
        conn.execute("INSERT INTO guard_epochs (scope_id, guard_key, generation) VALUES (?1, ?2, 1) ON CONFLICT(scope_id, guard_key) DO UPDATE SET generation = generation + 1", params![scope, key])?;
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
    let mut stmt = conn.prepare("SELECT r.record_id, r.kind, h.revision_id, v.body_json, v.recorded_sequence, v.valid_from, v.valid_until, v.epistemic_status FROM records r JOIN record_heads h ON h.record_id = r.record_id JOIN revisions v ON v.revision_id = h.revision_id WHERE r.scope_id = ?1 ORDER BY r.record_id, h.revision_id")?;
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
        // Corrupt body is omitted, not rewritten as {}. Sibling facts stay loadable.
        let Ok(body) = serde_json::from_str::<Value>(&body) else {
            continue;
        };
        let mut fields: BTreeMap<String, Value> = body
            .as_object()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        fields.insert("record".into(), json!(record));
        fields.insert("revision".into(), json!(revision));
        fields.insert("epistemic".into(), json!(epistemic));
        // Recipe Filter/Gt can only see `fields`. Without this, `what_changed`
        // cannot name records recorded after `since_seq` (TemporalSlice is
        // as-of / `known_seq <=`, never a delta).
        fields.insert("known_seq".into(), json!(seq));
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

fn plan_fingerprint(steps: &[Step], outputs: &[String]) -> String {
    let payload = json!({"outputs": outputs, "steps": steps}).to_string();
    cortex_logic::traces::content_hash(&payload)
}

fn compiled_id(
    recipe_id: &str,
    principal: &str,
    params_json: &str,
    environment: &str,
    plan: &str,
) -> String {
    let payload = format!(
        "{recipe_id}|{principal}|{params_json}|{environment}|{}|{plan}",
        cortex_logic::recipe::OPERATOR_VERSIONS
    );
    format!("compiled:{}", cortex_logic::traces::content_hash(&payload))
}

const COMPILED_GUARD_SQL: &str = "INSERT OR REPLACE INTO compiled_guards (compiled_id, scope_id, guard_key, expected_generation, kind) VALUES (?1, ?2, ?3, ?4, ?5)";

fn json_array_len(value: Option<&Value>, key: &str) -> usize {
    value
        .and_then(|v| v.get(key))
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn recipe_result_from_cached(cached: &Value, snap: &Snapshot) -> Option<RecipeResult> {
    let values = cached
        .get("values")?
        .as_object()?
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let work = cached.get("work")?.as_u64()? as usize;
    let positive: BTreeSet<String> = cached
        .get("positive")?
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let mut guards = BTreeMap::new();
    for guard in cached.get("guards")?.as_array()? {
        let scope = guard.get("scope")?.as_str()?.to_string();
        let relation = guard.get("relation")?.as_str()?.to_string();
        let epoch = guard.get("epoch")?.as_i64()?;
        guards.insert((scope, relation), epoch);
    }
    Some(RecipeResult {
        values,
        guards,
        positive,
        brain_epoch: snap.brain_epoch.clone(),
        policy_epoch: snap.policy_epoch.clone(),
        environment: snap.environment.clone(),
        scope: DEFAULT_SCOPE.into(),
        work,
        operator_versions: cortex_logic::recipe::OPERATOR_VERSIONS,
    })
}

fn cached_result(
    conn: &Connection,
    id: &str,
    raw: &str,
    snap: &Snapshot,
) -> rusqlite::Result<Option<(Value, RecipeResult)>> {
    let value = serde_json::from_str::<Value>(raw).ok();
    let mut stmt = conn.prepare("SELECT scope_id, guard_key, expected_generation, kind FROM compiled_guards WHERE compiled_id = ?1")?;
    let guards = stmt
        .query_map(params![id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<(String, String, i64, String)>>>();
    let Ok(guards) = guards else {
        return Ok(None);
    };
    let Some(value) = value else {
        return Ok(None);
    };
    // A crashed replacement can leave a cached result without all guard rows.
    // Counts must match before checking epochs, including empty result sets.
    let positive = guards
        .iter()
        .filter(|(_, _, _, kind)| kind == "positive")
        .count();
    if positive != json_array_len(Some(&value), "positive")
        || guards.len() - positive != json_array_len(Some(&value), "guards")
    {
        return Ok(None);
    }
    let valid = guards.into_iter().all(|(scope, key, expected, kind)| {
        let now = if kind == "positive" {
            if snap.facts.contains_key(&key) {
                expected
            } else {
                -1
            }
        } else {
            snap.epochs.get(&(scope, key)).copied().unwrap_or(0)
        };
        now == expected
    });
    Ok(valid
        .then(|| recipe_result_from_cached(&value, snap))
        .flatten()
        .map(|result| (value, result)))
}

fn reuse_compiled(
    conn: &Connection,
    id: &str,
    principal: &str,
    snap: &Snapshot,
) -> rusqlite::Result<Option<(Value, RecipeResult)>> {
    let cached: Option<(String, String, String, String, i64, String)> = conn.query_row("SELECT result_json, brain_epoch, policy_epoch, environment_ref, through_sequence, operator_versions FROM compiled_reads WHERE compiled_id = ?1 AND principal_id = ?2", params![id, principal], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))).optional()?;
    let Some((raw, brain, policy, environment, _, operators)) = cached else {
        return Ok(None);
    };
    if brain == snap.brain_epoch
        && policy == snap.policy_epoch
        && environment == snap.environment
        && operators == cortex_logic::recipe::OPERATOR_VERSIONS
    {
        if let Some(result) = cached_result(conn, id, &raw, snap)? {
            return Ok(Some(result));
        }
    }
    for table in ["compiled_guards", "compiled_reads"] {
        conn.execute(
            &format!("DELETE FROM {table} WHERE compiled_id = ?1"),
            params![id],
        )?;
    }
    Ok(None)
}

/// Evaluate (or reuse) a recipe. Identity = recipe + operator versions +
/// plan (steps/outputs) + parameters + principal + brain epoch + policy
/// epoch + environment; a cached result for one scope/params/plan can
/// never answer another.
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
    let plan = plan_fingerprint(steps, outputs);
    let id = compiled_id(recipe_id, principal, &params_json, environment, &plan);
    if let Some((value, result)) =
        reuse_compiled(conn, &id, principal, &snap).map_err(|e| e.to_string())?
    {
        return Ok((value, true, result));
    }
    let result =
        evaluate(&snap, DEFAULT_SCOPE, steps, outputs, limits).map_err(|e| e.to_string())?;
    let value = json!({"values": result.values, "work": result.work, "operator_versions": result.operator_versions, "positive": result.positive, "guards": result.guards.iter().map(|((s, k), g)| json!({"scope": s, "relation": k, "epoch": g})).collect::<Vec<_>>()});
    let sp =
        crate::db::SqliteSavepoint::enter(conn, "compiled_write").map_err(|e| e.to_string())?;
    let through: i64 = conn
        .query_row("SELECT COALESCE(MAX(sequence),0) FROM commits", [], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())?;
    conn.execute("INSERT OR REPLACE INTO compiled_reads (compiled_id, recipe_id, operator_versions, scope_id, principal_id, brain_epoch, policy_epoch, parameters_json, environment_ref, through_sequence, result_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)", params![id, recipe_id, result.operator_versions, DEFAULT_SCOPE, principal, result.brain_epoch, result.policy_epoch, params_json, environment, through, value.to_string()]).map_err(|e| e.to_string())?;
    for ((scope, key), generation) in &result.guards {
        conn.execute("INSERT OR IGNORE INTO guard_epochs (scope_id, guard_key, generation) VALUES (?1, ?2, ?3)", params![scope, key, generation]).map_err(|e| e.to_string())?;
        conn.execute(
            COMPILED_GUARD_SQL,
            params![id, scope, key, generation, "negative"],
        )
        .map_err(|e| e.to_string())?;
    }
    for revision in &result.positive {
        conn.execute(
            COMPILED_GUARD_SQL,
            params![id, DEFAULT_SCOPE, revision, 1, "positive"],
        )
        .map_err(|e| e.to_string())?;
    }
    sp.release().map_err(|e| e.to_string())?;
    Ok((value, false, result))
}
