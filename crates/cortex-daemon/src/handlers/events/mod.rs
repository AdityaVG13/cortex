use super::{ensure_events_stream_auth, json_response, now_iso, runtime_token_matches};
use crate::state::{BrainFiringEvent, RuntimeState};
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use futures_util::stream::{self, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::time::Duration as StdDuration;
use tokio_stream::wrappers::BroadcastStream;
fn scrub_event_payload(event_type: &str) -> Value {
    json!({"type":event_type,"timestamp":now_iso()})
}
#[derive(Deserialize)]
pub struct EventsStreamQuery {
    pub token: Option<String>,
}
pub async fn handle_events_stream(State(state): State<RuntimeState>, headers: HeaderMap, Query(query): Query<EventsStreamQuery>) -> Response {
    if let Err(resp) = ensure_events_stream_auth(&headers, query.token.as_deref(), &state).await {
        return resp;
    }
    let initial = stream::once(async move { Ok::<Event, Infallible>(Event::default().event("connected").data(scrub_event_payload("connected").to_string())) });
    let updates = BroadcastStream::new(state.events.subscribe()).filter_map(|msg| async move {
        match msg {
            Ok(event) => {
                let payload = scrub_event_payload(&event.event_type);
                Some(Ok::<Event, Infallible>(Event::default().event(&event.event_type).data(payload.to_string())))
            }
            Err(_) => None,
        }
    });
    let stream = initial.chain(updates);
    let sse = Sse::new(stream).keep_alive(KeepAlive::new().interval(StdDuration::from_secs(30)).text("keepalive"));
    sse.into_response()
}
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
pub async fn handle_brain_firing_stream(State(state): State<RuntimeState>, Query(query): Query<BrainFiringQuery>) -> Response {
    let provided = query.token.as_deref().unwrap_or("");
    if provided.is_empty() || !runtime_token_matches(provided, &state) {
        return json_response(StatusCode::UNAUTHORIZED, json!({"error":"Unauthorized"})).into_response();
    }
    let caller_owner_id = state.default_owner_id;
    let connected = stream::once(async move {
        Ok::<Event, Infallible>(Event::default().event("connected").data(json!({"type":"connected","timestamp":now_iso()}).to_string()))
    });
    let receiver = state.brain_firing.subscribe();
    let event_stream = BroadcastStream::new(receiver);
    let batch_window = StdDuration::from_millis(50);
    let buffered =
        futures_util::stream::unfold((event_stream, Vec::<BrainFiringEvent>::new(), caller_owner_id), move |(mut events, mut buf, owner)| async move {
            let first = match events.next().await {
                Some(Ok(ev)) => ev,
                Some(Err(_)) => return None,
                None => return None,
            };
            if owner.is_some() && first.owner_id == owner {
                buf.push(first);
            } else if owner.is_none() {
            } else if first.owner_id.is_none() {
            }
            let deadline = tokio::time::sleep(batch_window);
            tokio::pin!(deadline);
            loop {
                tokio::select! {_=&mut deadline=>break
                ,next=events.next()=>{match next{Some(Ok(ev))=>{if owner.is_some()&&ev.owner_id==owner{buf.push(ev);}}Some(Err(_))|None=>break,}}}
            }
            if buf.is_empty() {
                Some((None, (events, Vec::new(), owner)))
            } else {
                let array: Vec<Value> = buf.iter().map(brain_event_to_json).collect();
                buf.clear();
                Some((Some(Value::Array(array)), (events, buf, owner)))
            }
        })
        .filter_map(|item: Option<Value>| async move {
            item.map(|payload| Ok::<Event, Infallible>(Event::default().event("brain_batch").data(payload.to_string())))
        });
    let stream = connected.chain(buffered);
    let sse = Sse::new(stream).keep_alive(KeepAlive::new().interval(StdDuration::from_secs(30)).text("keepalive"));
    sse.into_response()
}
#[cfg(test)]
mod tests;
