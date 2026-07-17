use crate::handlers::{ensure_auth_rated, json_response};
use crate::state::RuntimeState;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
pub async fn handle_dump(State(state): State<RuntimeState>, headers: HeaderMap) -> Response {
    if let Err(resp) = ensure_auth_rated(&headers, &state).await {
        return resp;
    }
    let conn = state.db_read.lock().await;
    let memories:Vec<Value>=conn.prepare(
"SELECT id, text, source, type, tags, source_agent, confidence, status, score, \
             retrievals, last_accessed, pinned, disputes_id, supersedes_id, confirmed_by, \
             created_at, updated_at \
             FROM memories WHERE status = 'active' ORDER BY score DESC"
,).and_then(|mut stmt|{stmt.query_map([],|row|{Ok(json!({"id":row.get::<_,i64>(0)?,"text":row.get::<_,String>(1).unwrap_or_default
(),"source":row.get::<_,Option<String>>(2).unwrap_or(None),"type":row.get::<_,String>(3).unwrap_or_default(),"tags":row.get::<_,
Option<String>>(4).unwrap_or(None),"source_agent":row.get::<_,Option<String>>(5).unwrap_or(None),"confidence":row.get::<_,Option<
f64>>(6).unwrap_or(Some(0.8)),"status":row.get::<_,Option<String>>(7).unwrap_or(Some("active".to_string())),"score":row.get::<_,
Option<f64>>(8).unwrap_or(Some(1.0)),"retrievals":row.get::<_,Option<i64>>(9).unwrap_or(Some(0)),"last_accessed":row.get::<_,
Option<String>>(10).unwrap_or(None),"pinned":row.get::<_,Option<i64>>(11).unwrap_or(Some(0)),"disputes_id":row.get::<_,Option<i64
>>(12).unwrap_or(None),"supersedes_id":row.get::<_,Option<i64>>(13).unwrap_or(None),"confirmed_by":row.get::<_,Option<String>>(14)
.unwrap_or(None),"created_at":row.get::<_,Option<String>>(15).unwrap_or(None),"updated_at":row.get::<_,Option<String>>(16).
unwrap_or(None),}))}).map(|rows|rows.filter_map(|r|r.ok()).collect())}).unwrap_or_default();
    let decisions:Vec<Value>=conn.prepare(
"SELECT id, decision, context, type, source_agent, confidence, surprise, status, \
             score, retrievals, last_accessed, pinned, parent_id, disputes_id, supersedes_id, \
             confirmed_by, created_at, updated_at \
             FROM decisions WHERE status = 'active' ORDER BY score DESC"
,).and_then(|mut stmt|{stmt.query_map([],|row|{Ok(json!({"id":row.get::<_,i64>(0)?,"decision":row.get::<_,String>(1).
unwrap_or_default(),"context":row.get::<_,Option<String>>(2).unwrap_or(None),"type":row.get::<_,Option<String>>(3).unwrap_or(Some(
"decision".to_string())),"source_agent":row.get::<_,Option<String>>(4).unwrap_or(None),"confidence":row.get::<_,Option<f64>>(5).
unwrap_or(Some(0.8)),"surprise":row.get::<_,Option<f64>>(6).unwrap_or(Some(1.0)),"status":row.get::<_,Option<String>>(7).unwrap_or
(Some("active".to_string())),"score":row.get::<_,Option<f64>>(8).unwrap_or(Some(1.0)),"retrievals":row.get::<_,Option<i64>>(9).
unwrap_or(Some(0)),"last_accessed":row.get::<_,Option<String>>(10).unwrap_or(None),"pinned":row.get::<_,Option<i64>>(11).unwrap_or
(Some(0)),"parent_id":row.get::<_,Option<i64>>(12).unwrap_or(None),"disputes_id":row.get::<_,Option<i64>>(13).unwrap_or(None),
"supersedes_id":row.get::<_,Option<i64>>(14).unwrap_or(None),"confirmed_by":row.get::<_,Option<String>>(15).unwrap_or(None),
"created_at":row.get::<_,Option<String>>(16).unwrap_or(None),"updated_at":row.get::<_,Option<String>>(17).unwrap_or(None),}))}).
map(|rows|rows.filter_map(|r|r.ok()).collect())}).unwrap_or_default();
    let mut source_nodes: BTreeMap<String, String> = BTreeMap::new();
    for memory in &memories {
        let Some(id) = memory.get("id").and_then(|value| value.as_i64()) else {
            continue;
        };
        let Some(source) = memory
            .get("source")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        source_nodes
            .entry(source.to_string())
            .or_insert_with(|| format!("mem-{id}"));
    }
    for decision in &decisions {
        let Some(id) = decision.get("id").and_then(|value| value.as_i64()) else {
            continue;
        };
        let Some(source) = decision
            .get("context")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        source_nodes
            .entry(source.to_string())
            .or_insert_with(|| format!("dec-{id}"));
    }
    let mut seen_links: HashSet<String> = HashSet::new();
    let mut graph_links: Vec<Value> = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT source_a, source_b, count, last_seen
         FROM co_occurrence
         ORDER BY count DESC, last_seen DESC
         LIMIT 240",
    ) {
        if let Ok(rows) = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        }) {
            for row in rows.flatten() {
                let (source_a, source_b, count, last_seen) = row;
                let Some(node_a) = source_nodes.get(&source_a) else {
                    continue;
                };
                let Some(node_b) = source_nodes.get(&source_b) else {
                    continue;
                };
                if node_a == node_b {
                    continue;
                }
                let (left, right) = if node_a <= node_b {
                    (node_a.clone(), node_b.clone())
                } else {
                    (node_b.clone(), node_a.clone())
                };
                let key = format!("{left}|{right}|co_occurrence");
                if !seen_links.insert(key) {
                    continue;
                }
                graph_links.push(json!({"source":left,"target":right,"type":
"co_occurrence","weight":count,"lastSeen":last_seen,}));
            }
        }
    }
    for decision in &decisions {
        let Some(id) = decision.get("id").and_then(|value| value.as_i64()) else {
            continue;
        };
        let Some(disputes_id) = decision.get("disputes_id").and_then(|value| value.as_i64()) else {
            continue;
        };
        let left = format!("dec-{id}");
        let right = format!("dec-{disputes_id}");
        let (source, target) = if left <= right {
            (left, right)
        } else {
            (right, left)
        };
        let key = format!("{source}|{target}|conflict");
        if !seen_links.insert(key) {
            continue;
        }
        graph_links.push(json!({"source":
source,"target":target,"type":"conflict","weight":1,}));
    }
    json_response(
        StatusCode::OK,
        json!({"memories":memories,"decisions":
decisions,"graph":{"links":graph_links,"nodeCount":memories.len()+decisions.len(),}}),
    )
}
