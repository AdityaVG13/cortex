// SPDX-License-Identifier: MIT
use std::convert::Infallible;
use std::time::Duration as StdDuration;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::stream::{self, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_stream::wrappers::BroadcastStream;

use super::now_iso;
use crate::state::{BrainFiringEvent, RuntimeState};

fn scrub_event_payload(event_type: &str) -> Value {
    json!({
        "type": event_type,
        "timestamp": now_iso()
    })
}

// ─── GET /events/stream ─────────────────────────────────────────────────────

pub async fn handle_events_stream(State(state): State<RuntimeState>) -> Response {
    let initial = stream::once(async move {
        Ok::<Event, Infallible>(
            Event::default()
                .event("connected")
                .data(scrub_event_payload("connected").to_string()),
        )
    });

    let updates = BroadcastStream::new(state.events.subscribe()).filter_map(|msg| async move {
        match msg {
            Ok(event) => {
                let payload = scrub_event_payload(&event.event_type);
                Some(Ok::<Event, Infallible>(
                    Event::default()
                        .event(&event.event_type)
                        .data(payload.to_string()),
                ))
            }
            Err(_) => None,
        }
    });

    let stream = initial.chain(updates);
    let sse = Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(StdDuration::from_secs(30))
            .text("keepalive"),
    );
    // CORS handled by tower-http CorsLayer in server.rs -- no manual override
    sse.into_response()
}

// ─── GET /brain/firing ──────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct BrainFiringQuery {
    pub token: Option<String>,
}

fn brain_event_to_json(event: &BrainFiringEvent) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("type".to_string(), Value::String(event.kind.as_str().to_string()));
    obj.insert("ts".to_string(), Value::String(now_iso()));
    if let Some(payload_obj) = event.payload.as_object() {
        for (k, v) in payload_obj {
            obj.insert(k.clone(), v.clone());
        }
    }
    if let Some(owner) = event.owner_id {
        obj.insert("owner_id".to_string(), Value::from(owner));
    }
    Value::Object(obj)
}

pub async fn handle_brain_firing_stream(
    State(state): State<RuntimeState>,
    Query(query): Query<BrainFiringQuery>,
) -> Response {
    // Auth: token must match runtime token. Browser EventSource cannot send
    // custom headers, so the token rides in the query string.
    let provided = query.token.as_deref().unwrap_or("");
    if provided.is_empty() || provided != state.token.as_str() {
        return (StatusCode::UNAUTHORIZED, "missing or invalid token").into_response();
    }

    // Owner scoping: in single-user mode, the caller is the default owner.
    // Team mode resolution is out of scope for v1.
    let caller_owner_id = state.default_owner_id;

    let connected = stream::once(async move {
        Ok::<Event, Infallible>(
            Event::default()
                .event("connected")
                .data(json!({"type":"connected","timestamp":now_iso()}).to_string()),
        )
    });

    // Coalesce: collect events into a 50ms window then emit as a single
    // brain_batch SSE message whose data is a JSON array.
    let receiver = state.brain_firing.subscribe();
    let event_stream = BroadcastStream::new(receiver);

    let batch_window = StdDuration::from_millis(50);
    let buffered = futures_util::stream::unfold(
        (event_stream, Vec::<BrainFiringEvent>::new(), caller_owner_id),
        move |(mut events, mut buf, owner)| async move {
            // Wait for first event, then collect all that arrive within the window.
            let first = match events.next().await {
                Some(Ok(ev)) => ev,
                Some(Err(_)) => return None,
                None => return None,
            };

            // Owner filter — fail-closed if caller has no resolved owner_id.
            if owner.is_some() && first.owner_id == owner {
                buf.push(first);
            } else if owner.is_none() {
                // No owner resolution available; drop everything to fail-closed.
            } else if first.owner_id.is_none() {
                // Event has no owner — never leak.
            }

            let deadline = tokio::time::sleep(batch_window);
            tokio::pin!(deadline);

            loop {
                tokio::select! {
                    _ = &mut deadline => break,
                    next = events.next() => {
                        match next {
                            Some(Ok(ev)) => {
                                if owner.is_some() && ev.owner_id == owner {
                                    buf.push(ev);
                                }
                            }
                            Some(Err(_)) | None => break,
                        }
                    }
                }
            }

            if buf.is_empty() {
                // Continue without emitting (no owner-matching events in window).
                Some((None, (events, Vec::new(), owner)))
            } else {
                let array: Vec<Value> = buf.iter().map(brain_event_to_json).collect();
                buf.clear();
                Some((Some(Value::Array(array)), (events, buf, owner)))
            }
        },
    )
    .filter_map(|item: Option<Value>| async move {
        item.map(|payload| {
            Ok::<Event, Infallible>(
                Event::default()
                    .event("brain_batch")
                    .data(payload.to_string()),
            )
        })
    });

    let stream = connected.chain(buffered);
    let sse = Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(StdDuration::from_secs(30))
            .text("keepalive"),
    );
    sse.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{BrainFiringEvent, BrainKind};

    #[test]
    fn scrub_event_payload_only_exposes_type_and_timestamp() {
        let payload = scrub_event_payload("task");
        let object = payload.as_object().expect("payload object");

        assert_eq!(
            object.get("type").and_then(|value| value.as_str()),
            Some("task")
        );
        assert!(object
            .get("timestamp")
            .and_then(|value| value.as_str())
            .is_some());
        assert_eq!(object.len(), 2);
    }

    #[test]
    fn brain_event_to_json_includes_kind_and_payload_fields() {
        let event = BrainFiringEvent {
            kind: BrainKind::ClusterFinalized,
            payload: json!({"cluster_id": 42, "member_count": 7}),
            owner_id: Some(1),
        };
        let v = brain_event_to_json(&event);
        let obj = v.as_object().expect("object");
        assert_eq!(obj.get("type").and_then(|v| v.as_str()), Some("cluster_finalized"));
        assert_eq!(obj.get("cluster_id").and_then(|v| v.as_i64()), Some(42));
        assert_eq!(obj.get("member_count").and_then(|v| v.as_i64()), Some(7));
        assert_eq!(obj.get("owner_id").and_then(|v| v.as_i64()), Some(1));
        assert!(obj.get("ts").and_then(|v| v.as_str()).is_some());
    }
}
