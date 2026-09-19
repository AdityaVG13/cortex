use super::super::CortexRuntime;
use super::observation;
use super::*;
use asupersync::Cx;
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

impl CortexRuntime {
    async fn observations_for_paths(
        &self,
        cx: &Cx,
        paths: &[String],
        extra_scope: Option<&str>,
        max_results: usize,
        max_bytes: usize,
        mut each: impl FnMut(&Transaction<'_>, &str, &str) -> Result<PreparedView, String>,
    ) -> Result<PreparedView, String> {
        self.with_db_tx(
            cx,
            TransactionBehavior::Deferred,
            ensure,
            |tx, principal| {
                let scopes = observation::resolve_query_scopes(tx, principal, paths, extra_scope)?;
                let mut views = Vec::new();
                for scope in scopes {
                    views.push(each(tx, principal, &scope)?);
                }
                Ok(merge_prepared(views, max_results, max_bytes))
            },
        )
        .await
    }

    pub async fn query_observations(
        &self,
        cx: &Cx,
        scope: &str,
        query: &str,
        max_results: usize,
        max_bytes: usize,
        learned: bool,
    ) -> Result<PreparedView, String> {
        let spec = pull_spec(scope, query, max_results, max_bytes, learned)?;
        self.with_db_tx(
            cx,
            TransactionBehavior::Deferred,
            ensure,
            |tx, principal| {
                project(tx, principal, &spec.scope, 32)?;
                materialize(tx, principal, &spec)
            },
        )
        .await
    }

    /// Cue-filtered pull over caller project roots. Path-scoped sources stay
    /// in their repository; the extra label (default `project`) remains the
    /// unscoped bucket when roots are named.
    pub async fn query_observations_for_paths(
        &self,
        cx: &Cx,
        query: &str,
        paths: &[String],
        extra_scope: Option<&str>,
        max_results: usize,
        max_bytes: usize,
        learned: bool,
    ) -> Result<PreparedView, String> {
        self.observations_for_paths(
            cx,
            paths,
            extra_scope,
            max_results,
            max_bytes,
            |tx, principal, scope| {
                let spec = pull_spec(scope, query, max_results, max_bytes, learned)?;
                project(tx, principal, &spec.scope, 32)?;
                materialize(tx, principal, &spec)
            },
        )
        .await
    }

    /// Latest attributed observations in the caller roots, without a cue join.
    /// Used when orient names a project and the task is only that path.
    pub async fn recent_observations_for_paths(
        &self,
        cx: &Cx,
        paths: &[String],
        extra_scope: Option<&str>,
        max_results: usize,
        max_bytes: usize,
    ) -> Result<PreparedView, String> {
        self.observations_for_paths(
            cx,
            paths,
            extra_scope,
            max_results,
            max_bytes,
            |tx, principal, scope| {
                let spec = NeedSpec {
                    id: format!("recent:{}", uuid::Uuid::new_v4()),
                    scope: observation::normalize_scope(scope),
                    cues: vec!["_".into()],
                    exclude_cues: Vec::new(),
                    max_results,
                    max_bytes,
                    ttl_seconds: 1,
                    learned: false,
                };
                materialize_recent(tx, principal, &spec)
            },
        )
        .await
    }

    pub async fn rebuild_observation_projection(
        &self,
        cx: &Cx,
        scope: &str,
    ) -> Result<usize, String> {
        self.with_locked_db(cx, |conn, principal| {
            ensure(conn)?;
            let scope = observation::normalize_scope(scope);
            if scope.is_empty() || scope.len() > 1024 { return Err("invalid_need_bounds".into()); }
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
            let projection = format!("DELETE FROM observation_projection WHERE source_id IN (SELECT e.source_id FROM {} WHERE e.principal=?1 AND g.scope_label=?2)", observation::EVENT_GRANT_JOIN);
            for sql in [projection.as_str(), "DELETE FROM observation_postings WHERE principal=?1 AND scope=?2", "DELETE FROM observation_matches WHERE principal=?1 AND need_id IN (SELECT need_id FROM observation_needs WHERE principal=?1 AND scope=?2)"] {
                tx.execute(sql, params![principal, scope]).map_err(|e| e.to_string())?;
            }
            let count = project(&tx, principal, &scope, 32)?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(count)
        }).await
    }
    pub async fn subscribe_observations(
        &self,
        cx: &Cx,
        spec: NeedSpec,
    ) -> Result<PreparedView, String> {
        if spec.id.starts_with("pull:") {
            return Err("reserved_need_identity".into());
        }
        self.with_db_tx(
            cx,
            TransactionBehavior::Immediate,
            ensure,
            |tx, principal| {
                register(tx, principal, &spec)?;
                project(tx, principal, &spec.scope, 32)?;
                materialize(tx, principal, &spec)
            },
        )
        .await
    }

    pub async fn prepare_observations(
        &self,
        cx: &Cx,
        id: &str,
        context: &str,
        present_delivery: Option<&str>,
    ) -> Result<PreparedView, String> {
        if context.is_empty() || context.len() > 256 {
            return Err("invalid_context_identity".into());
        }
        self.with_db_tx(cx, TransactionBehavior::Immediate, ensure, |tx, principal| {
            let now = chrono::Utc::now().timestamp();
            tx.execute("DELETE FROM observation_deliveries WHERE principal=?1 AND expires<=?2", params![principal, now]).map_err(|e| e.to_string())?;
            let raw: String = tx.query_row("SELECT spec_json FROM observation_needs WHERE principal=?1 AND need_id=?2 AND expires>?3", params![principal, id, now], |r| r.get(0)).optional().map_err(|e| e.to_string())?.ok_or("need_missing_or_expired")?;
            let spec: NeedSpec = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
            project(tx, principal, &spec.scope, 32)?;
            let mut view = materialize(tx, principal, &spec)?;
            apply_observation_delivery(tx, principal, id, context, present_delivery, &mut view, now)?;
            Ok(view)
        }).await
    }

    /// Retraction excludes a source from active Views without deleting its exact bytes.
    pub async fn retract_observation(
        &self,
        cx: &Cx,
        source_id: &str,
        reason: &str,
    ) -> Result<(), String> {
        if reason.is_empty() || reason.len() > 1024 {
            return Err("invalid_retraction_reason".into());
        }
        self.with_db_tx(cx, TransactionBehavior::Immediate, ensure, |tx, principal| {
            let key: String = tx.query_row("SELECT source_key FROM observation_events WHERE principal=?1 AND source_id=?2", params![principal, source_id], |r| r.get(0)).optional().map_err(|e| e.to_string())?.ok_or("source_not_authorized")?;
            observation::granted(tx, principal, &key, true)?;
            tx.execute("INSERT INTO observation_retractions VALUES(?1,?2,?3) ON CONFLICT(source_id) DO UPDATE SET reason=excluded.reason",params![source_id,principal,reason]).map_err(|e|e.to_string())?;
            Ok(())
        }).await
    }

    /// Bind a required child to a parent. Delivery of the parent is incomplete
    /// unless that child is still authorized, current, and unretracted.
    pub async fn require_observation(
        &self,
        cx: &Cx,
        parent_id: &str,
        child_id: &str,
    ) -> Result<(), String> {
        if parent_id == child_id {
            return Err("invalid_requirement".into());
        }
        self.with_db_tx(
            cx,
            TransactionBehavior::Immediate,
            ensure,
            |tx, principal| {
                let parent_scope = event_scope(tx, principal, parent_id)?;
                let child_scope = event_scope(tx, principal, child_id)?;
                if parent_scope != child_scope {
                    return Err("requirement_scope_mismatch".into());
                }
                tx.execute(
                    "INSERT OR IGNORE INTO observation_requirements VALUES(?1,?2,?3)",
                    params![principal, parent_id, child_id],
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            },
        )
        .await
    }
}

fn apply_observation_delivery(
    tx: &Transaction<'_>,
    principal: &str,
    id: &str,
    context: &str,
    present_delivery: Option<&str>,
    view: &mut PreparedView,
    now: i64,
) -> Result<(), String> {
    let present = if let Some(delivery) = present_delivery {
        tx.query_row("SELECT 1 FROM observation_deliveries WHERE delivery_id=?1 AND principal=?2 AND need_id=?3 AND context=?4 AND fingerprint=?5 AND restore_epoch=?6 AND expires>?7",params![delivery,principal,id,context,view.fingerprint,view.restore_epoch,now],|r|r.get::<_,i64>(0)).optional().map_err(|e|e.to_string())?.is_some()
    } else {
        false
    };
    if present {
        view.payload.clear();
        view.payload_bytes = 0;
        view.delivery_id = present_delivery.map(str::to_string);
    } else if !view.payload.is_empty() {
        let delivery = format!("delivery:{}", uuid::Uuid::new_v4());
        tx.execute(
            "INSERT INTO observation_deliveries VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![
                delivery,
                principal,
                id,
                context,
                view.fingerprint,
                view.restore_epoch,
                now + 300
            ],
        )
        .map_err(|e| e.to_string())?;
        view.delivery_id = Some(delivery);
    }
    Ok(())
}
