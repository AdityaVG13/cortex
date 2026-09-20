use super::super::super::CortexRuntime;
use super::super::*;
use crate::db::records;
use asupersync::Cx;
use cortex_logic::assembly::LearningEvent;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

fn upsert_learning_retraction(
    tx: &Transaction<'_>,
    origin: &str,
    origin_event_id: &str,
    sequence: i64,
    reason: &str,
) -> Result<(), String> {
    tx.execute("INSERT INTO learning_retractions VALUES(?1,?2,?3,?4) ON CONFLICT(origin,origin_event_id) DO UPDATE SET reason=excluded.reason", params![origin, origin_event_id, sequence, reason]).map_err(|err| err.to_string())?;
    Ok(())
}

/// Connection-level record shared by the async runtime API and the
/// feedback op's in-savepoint wire (Loop 3). Savepoint-nestable: rolls
/// back with an enclosing savepoint instead of committing through it.
pub(crate) fn record_learning_event_conn(
    conn: &Connection,
    principal: &str,
    event: &LearningEvent,
) -> Result<bool, String> {
    ensure(conn)?;
    let sp = crate::db::SqliteSavepoint::enter(conn, "learn_event")
        .map_err(|err| err.to_string())?;
    if crate::db::count_sql(conn, "SELECT COUNT(*) FROM assemblies WHERE principal=?1 AND assembly_id=?2 AND scope_label=?3", params![principal, event.target, event.scope])? == 0 { return Err("learning_target_missing".into()); }
    if crate::db::count_sql(conn, "SELECT COUNT(*) FROM learning_retractions WHERE origin=?1 AND origin_event_id=?2", params![event.origin, event.origin_event_id])? > 0 { return Ok(false); }
    for source in &event.sources {
        if crate::db::count_sql(conn, "SELECT COUNT(*) FROM learning_source_erasures WHERE source_id=?1", params![source])? > 0 { return Ok(false); }
    }
    let previous: Option<(i64, String)> = conn.query_row("SELECT reward,cues_json FROM learning_events WHERE origin=?1 AND origin_event_id=?2", params![event.origin, event.origin_event_id], |row| Ok((row.get(0)?, row.get(1)?))).optional().map_err(|err| err.to_string())?;
    if let Some((reward, cues_json)) = previous {
        let cues: Vec<String> = serde_json::from_str(&cues_json).map_err(|err| err.to_string())?;
        if reward != i64::from(event.reward) || cues != event.cues { return Err("feedback_identity_conflict".into()); }
        return Ok(false);
    }
    let sequence = records::append_ack_commit(conn, principal)?;
    conn.execute("INSERT INTO learning_events VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)", params![event.origin, event.origin_event_id, principal, event.scope, event.training_unit, event.target, event.kind.as_str(), event.reward, serde_json::to_string(&event.cues).map_err(|err| err.to_string())?, event.observed_at, event.receipt_ref, sequence]).map_err(|err| err.to_string())?;
    for source in &event.sources {
        conn.execute("INSERT INTO learning_dependencies VALUES(?1,?2,?3)", params![event.origin, event.origin_event_id, source]).map_err(|err| err.to_string())?;
    }
    sp.release().map_err(|err| err.to_string())?;
    Ok(true)
}

impl CortexRuntime {
    pub async fn record_learning_event(
        &self,
        cx: &Cx,
        event: LearningEvent,
    ) -> Result<bool, String> {
        event.validate()?;
        let mut event = event;
        event.scope = canonical_scope(&event.scope)?;
        let principal = self.observation_principal()?;
        if event.principal != principal {
            return Err("feedback_not_authorized".into());
        }
        self.with_locked_db(cx, |conn, principal| {
            record_learning_event_conn(conn, principal, &event)
        })
        .await
    }

    pub async fn retract_learning_event(
        &self,
        cx: &Cx,
        origin: &str,
        origin_event_id: &str,
        reason: &str,
    ) -> Result<(), String> {
        check_id(origin)?;
        check_id(origin_event_id)?;
        if reason.is_empty() || reason.len() > 1024 {
            return Err("invalid_retraction_reason".into());
        }
        self.with_db_tx(cx, TransactionBehavior::Immediate, ensure, |tx, principal| {
            // Retractions are a global (origin, event) tombstone. A missing row
            // would let any principal poison a key before the owner records it;
            // a row owned by someone else is not this caller's feedback.
            let owner: Option<String> = tx.query_row("SELECT principal FROM learning_events WHERE origin=?1 AND origin_event_id=?2", params![origin, origin_event_id], |row| row.get(0)).optional().map_err(|err| err.to_string())?;
            match owner {
                None => return Err("learning_event_missing".into()),
                Some(owner) if owner != principal => return Err("feedback_not_authorized".into()),
                Some(_) => {}
            }
            let sequence = records::append_ack_commit(tx, principal)?;
            upsert_learning_retraction(tx, origin, origin_event_id, sequence, reason)?;
            Ok(())
        }).await
    }

    pub async fn erase_learning_source(&self, cx: &Cx, source_id: &str) -> Result<usize, String> {
        check_id(source_id)?;
        self.with_db_tx(cx, TransactionBehavior::Immediate, ensure, |tx, principal| {
            let learned = crate::db::count_sql(tx, "SELECT COUNT(*) FROM learning_events e JOIN learning_dependencies d ON d.origin=e.origin AND d.origin_event_id=e.origin_event_id WHERE d.source_id=?1 AND e.principal=?2", params![source_id, principal])?;
            let observed: i64 = match tx.query_row("SELECT COUNT(*) FROM observation_events WHERE source_id=?1 AND principal=?2", params![source_id, principal], |row| row.get(0)) {
                Ok(count) => count,
                Err(err) if err.to_string().contains("no such table") => 0,
                Err(err) => return Err(err.to_string()),
            };
            if learned + observed == 0 { return Err("learning_source_not_owned".into()); }
            let sequence = records::append_ack_commit(tx, principal)?;
            let (_, restore, _) = records::brain_epochs(tx);
            tx.execute("INSERT INTO learning_source_erasures VALUES(?1,?2,?3) ON CONFLICT(source_id) DO UPDATE SET erasure_epoch=excluded.erasure_epoch", params![source_id, restore, sequence]).map_err(|err| err.to_string())?;
            let mut stmt = tx.prepare("SELECT e.origin,e.origin_event_id FROM learning_dependencies d JOIN learning_events e ON e.origin=d.origin AND e.origin_event_id=d.origin_event_id WHERE d.source_id=?1 AND e.principal=?2").map_err(|err| err.to_string())?;
            let keys = stmt.query_map(params![source_id, principal], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).map_err(|err| err.to_string())?.collect::<Result<Vec<_>, _>>().map_err(|err| err.to_string())?;
            drop(stmt);
            for (origin, origin_event_id) in &keys {
                upsert_learning_retraction(tx, origin, origin_event_id, sequence, "source_erased")?;
            }
            Ok(keys.len())
        }).await
    }
}
