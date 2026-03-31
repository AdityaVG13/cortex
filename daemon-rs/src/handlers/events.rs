use std::convert::Infallible;
use std::time::Duration as StdDuration;

use axum::extract::State;
use axum::http::HeaderValue;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::stream::{self, StreamExt};
use serde_json::{json, Value};
use tokio_stream::wrappers::BroadcastStream;

use crate::state::RuntimeState;
use super::now_iso;

// ─── GET /events/stream ─────────────────────────────────────────────────────

pub async fn handle_events_stream(State(state): State<RuntimeState>) -> Response {
    let initial = stream::once(async move {
        Ok::<Event, Infallible>(
            Event::default()
                .event("connected")
                .data(
                    json!({ "timestamp": now_iso(), "clients": 1 }).to_string(),
                ),
        )
    });

    let updates =
        BroadcastStream::new(state.events.subscribe()).filter_map(|msg| async move {
            match msg {
                Ok(event) => {
                    let payload = match event.data {
                        Value::Object(mut map) => {
                            map.insert(
                                "type".to_string(),
                                Value::String(event.event_type.clone()),
                            );
                            map.insert(
                                "timestamp".to_string(),
                                Value::String(now_iso()),
                            );
                            Value::Object(map)
                        }
                        other => json!({
                            "type": event.event_type,
                            "data": other,
                            "timestamp": now_iso()
                        }),
                    };
                    Some(Ok::<Event, Infallible>(
                        Event::default()
                            .event(
                                payload
                                    .get("type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("event"),
                            )
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
