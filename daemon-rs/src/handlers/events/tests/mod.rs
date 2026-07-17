// SPDX-License-Identifier: MIT
use super::*;

use super::*;
use crate::state::{BrainFiringEvent, BrainKind};
#[test]
fn scrub_event_payload_only_exposes_type_and_timestamp() {
    let payload = scrub_event_payload("task");
    let object = payload.as_object().expect("payload object");
    assert_eq!(object.get("type").and_then(|value| value.as_str()), Some("task"));
    assert!(object.get("timestamp").and_then(|value| value.as_str()).is_some());
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
