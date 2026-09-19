use super::{
    CapabilityManifest, EventFrame, EventKind, HookDecision, HookOutcome, Presence, SnapshotState,
};

pub fn decide(frame: &EventFrame, snapshot: SnapshotState) -> HookOutcome {
    let m = &frame.capabilities;
    let overflow = frame.input_bytes > m.max_input_bytes;
    let presence = if m.attest_context_presence {
        Presence::Present
    } else if m.observe_compaction
        && matches!(frame.kind, Some(EventKind::Compaction | EventKind::Handoff))
    {
        Presence::Absent
    } else {
        Presence::Unknown
    };
    let outcome = |decision, reason: String, automatic_capture, counted| HookOutcome {
        decision,
        reason,
        overflow,
        presence,
        automatic_capture,
        counted,
    };
    let quiet = |decision, reason: String| outcome(decision, reason, false, false);
    let Some(kind) = frame.kind else {
        return quiet(HookDecision::Noop, "unknown event kind".into());
    };
    if let Some((flag, held)) = kind.required_flag(m) {
        if !held {
            return quiet(
                HookDecision::Noop,
                format!("host does not declare {flag}; nothing observed, nothing claimed"),
            );
        }
    }
    match kind {
        EventKind::ToolResult => {
            return if m.read_only {
                quiet(
                    HookDecision::Noop,
                    "read_only adapter: capture disabled".into(),
                )
            } else {
                outcome(
                    HookDecision::Deliver,
                    "tool result observed: deterministic capture".into(),
                    m.durable_local_capture,
                    false,
                )
            };
        }
        EventKind::Compaction | EventKind::Handoff => {
            return if m.checkpoint_on_transition {
                quiet(
                    HookDecision::Deliver,
                    "transition observed: checkpoint Thread state".into(),
                )
            } else {
                quiet(
                    HookDecision::Noop,
                    "host cannot checkpoint on transition".into(),
                )
            };
        }
        EventKind::SessionEnd => {
            return quiet(
                HookDecision::Noop,
                "session end: outcome receipts only".into(),
            );
        }
        EventKind::SessionStart
        | EventKind::PromptDelta
        | EventKind::ArtifactRevision
        | EventKind::NewTurn => {}
    }
    if !m.inject_context {
        return outcome(
            HookDecision::QueryRequired,
            "host cannot inject context: agent-initiated path, tokens counted".into(),
            false,
            true,
        );
    }
    match snapshot {
        SnapshotState::Unavailable => quiet(
            HookDecision::Unavailable,
            "memory unavailable: say so, never pretend emptiness".into(),
        ),
        SnapshotState::ProjectionPending => quiet(
            HookDecision::ProjectionPending,
            "projections behind the commit frontier".into(),
        ),
        SnapshotState::Expired => outcome(
            HookDecision::QueryRequired,
            "snapshot expired: an expired snapshot is not no memory".into(),
            false,
            true,
        ),
        SnapshotState::Fresh if frame.needs.is_empty() && kind == EventKind::PromptDelta => {
            quiet(HookDecision::Noop, "no needs on this delta".into())
        }
        SnapshotState::Fresh => outcome(
            HookDecision::Deliver,
            format!("deliver for {kind:?}"),
            false,
            true,
        ),
    }
}

/// The degradation matrix for one manifest: every event kind against the
/// fresh-snapshot decision, so a host sees exactly what it loses.
pub fn degradation_matrix(manifest: &CapabilityManifest) -> Vec<(EventKind, HookOutcome)> {
    EventKind::ALL
        .iter()
        .map(|kind| {
            let frame = EventFrame {
                kind: Some(*kind),
                needs: vec!["current_constraints".into()],
                capabilities: manifest.clone(),
                ..Default::default()
            };
            (*kind, decide(&frame, SnapshotState::Fresh))
        })
        .collect()
}
