//! Protocol vocabulary contracts: identities are distinct, statuses map to
//! transport codes deterministically, envelopes round-trip namespaced
//! extensions but fail explicitly on unknown operations/flags/fields, and a
//! Receipt is durable only with a local commit frontier.

use cortex_daemon::protocol::{Envelope, EnvelopeError, LogicalId, Receipt, ResponseStatus};
use std::fs;
use std::path::PathBuf;

fn envelopes_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../docs/contracts/protocol/envelopes")
}

fn read(name: &str) -> String {
    let path = envelopes_dir().join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn every_status_round_trips_and_derives_its_http_code() {
    let all = [
        (ResponseStatus::Ok, 200, true),
        (ResponseStatus::Partial, 200, true),
        (ResponseStatus::NoMatch, 200, true),
        (ResponseStatus::Ambiguous, 200, true),
        (ResponseStatus::NeedsMoreBudget, 413, false),
        (ResponseStatus::ProjectionPending, 202, false),
        (ResponseStatus::ResnapshotRequired, 409, false),
        (ResponseStatus::Unavailable, 503, false),
        (ResponseStatus::Denied, 403, false),
        (ResponseStatus::OutcomeUnknown, 202, false),
        (ResponseStatus::InvalidRequest, 400, false),
    ];
    for (status, code, is_answer) in all {
        assert_eq!(ResponseStatus::parse(status.as_str()), Some(status));
        assert_eq!(status.http_code(), code, "{status:?}");
        assert_eq!(status.is_answer(), is_answer, "{status:?}");
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, format!("\"{}\"", status.as_str()));
    }
    assert_eq!(ResponseStatus::parse("success"), None);
    assert_eq!(ResponseStatus::from_http_code(401), ResponseStatus::Denied);
    assert_eq!(ResponseStatus::from_http_code(404), ResponseStatus::NoMatch);
    assert_eq!(
        ResponseStatus::from_http_code(500),
        ResponseStatus::Unavailable
    );
    assert_eq!(
        ResponseStatus::from_http_code(422),
        ResponseStatus::InvalidRequest
    );
}

#[test]
fn valid_envelopes_parse_validate_and_round_trip_extensions() {
    for name in ["orient_request.json", "commit_request.json"] {
        let text = read(name);
        let env: Envelope = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        env.validate().unwrap_or_else(|e| panic!("{name}: {e}"));
        let back = serde_json::to_value(&env).unwrap();
        let original: serde_json::Value = serde_json::from_str(&text).unwrap();
        for (key, value) in original.as_object().unwrap() {
            if key.starts_with("x-") {
                assert_eq!(
                    &back[key], value,
                    "{name}: extension {key} must round-trip untouched"
                );
            }
        }
        assert_eq!(back["operation"], original["operation"]);
    }
}

#[test]
fn invalid_envelopes_fail_with_the_field_to_fix() {
    let cases = [
        ("invalid_unknown_operation.json", "summarize"),
        ("invalid_unknown_required_flag.json", "telepathy"),
        ("invalid_unknown_field.json", "owner_id"),
    ];
    for (name, needle) in cases {
        let env: Envelope =
            serde_json::from_str(&read(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        let err = env.validate().expect_err(name);
        assert!(err.to_string().contains(needle), "{name}: {err}");
        match (name, &err) {
            ("invalid_unknown_operation.json", EnvelopeError::UnknownOperation(_)) => {}
            ("invalid_unknown_required_flag.json", EnvelopeError::UnknownRequiredFlag(_)) => {}
            ("invalid_unknown_field.json", EnvelopeError::UnknownField(_)) => {}
            other => panic!("wrong error class: {other:?}"),
        }
    }
}

#[test]
fn receipt_durability_is_a_vector_not_a_boolean() {
    let committed: Receipt = serde_json::from_str(&read("commit_receipt.json")).unwrap();
    assert!(committed.durability.accepted);
    assert!(committed.is_locally_durable());
    assert_eq!(
        committed.entries["attempt"],
        LogicalId::new("attempt", "48")
    );
    assert_eq!(
        committed.durability.projected_through["lexical"].opaque,
        vec![0x00, 0x51]
    );
    let accepted_only: Receipt =
        serde_json::from_str(&read("accepted_not_committed_receipt.json")).unwrap();
    assert!(accepted_only.durability.accepted);
    assert!(
        !accepted_only.is_locally_durable(),
        "accepted is not committed"
    );
}

#[test]
fn legacy_references_become_logical_aliases() {
    assert_eq!(
        LogicalId::parse_legacy("memory::42"),
        Some(LogicalId::new("memory", "42"))
    );
    assert_eq!(
        LogicalId::parse_legacy("decision::7").map(|id| id.canonical()),
        Some("decision:7".into())
    );
    assert_eq!(LogicalId::parse_legacy("memory::"), None);
    assert_eq!(LogicalId::parse_legacy("memory::4x"), None);
    assert_eq!(
        LogicalId::from_legacy("decision", 3),
        LogicalId::new("decision", "3")
    );
}

#[test]
fn invalid_port_is_a_configuration_error_not_a_fallback() {
    use cortex_daemon::auth::parse_port;
    assert_eq!(parse_port("7437"), Ok(7437));
    assert!(parse_port("0").is_err());
    assert!(parse_port("99999").is_err());
    assert!(parse_port("abc").is_err());
}
