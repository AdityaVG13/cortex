// SPDX-License-Identifier: AGPL-3.0-only
// This file is part of Cortex.
//
// Cortex is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.
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

// ─── GET /events/stream ─────────────────────────────────────────────────────

pub async fn handle_events_stream(State(state): State<RuntimeState>) -> Response {
    let initial = stream::once(async move {
        Ok::<Event, Infallible>(
            Event::default()
                .event("connected")
                .data(json!({ "timestamp": now_iso(), "clients": 1 }).to_string()),
        )
    });

    let updates = BroadcastStream::new(state.events.subscribe()).filter_map(|msg| async move {
        match msg {
            Ok(event) => {
                let payload = match event.data {
                    Value::Object(mut map) => {
                        map.insert("type".to_string(), Value::String(event.event_type.clone()));
                        map.insert("timestamp".to_string(), Value::String(now_iso()));
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

