// SPDX-License-Identifier: MIT
use std::convert::Infallible;
use std::time::Duration as StdDuration;

use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::stream::{self, StreamExt};
use serde_json::{json, Value};
use tokio_stream::wrappers::BroadcastStream;

use super::now_iso;
use crate::state::RuntimeState;

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

#[cfg(test)]
mod tests {
    use super::scrub_event_payload;

    #[test]
    fn scrub_event_payload_only_exposes_type_and_timestamp() {
        let payload = scrub_event_payload("task");
        let object = payload.as_object().expect("payload object");

        assert_eq!(object.get("type").and_then(|value| value.as_str()), Some("task"));
        assert!(object.get("timestamp").and_then(|value| value.as_str()).is_some());
        assert_eq!(object.len(), 2);
    }
}
