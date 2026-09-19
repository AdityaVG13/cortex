use super::*;

fn intent(id: &str, key: Option<&str>) -> WriteIntent {
    WriteIntent {
        request_id: id.into(),
        idempotency_key: key.map(str::to_string),
        principal: "conformance".into(),
        expected_heads: Vec::new(),
    }
}

fn op(name: &str, text: &str) -> Op {
    Op::InsertDecision {
        local_name: name.into(),
        text: text.into(),
        context: None,
        agent: "conformance".into(),
        owner_id: None,
    }
}

/// Run the in-process laws against any provider. Every law is failure-first:
/// it names the behavior a wrong backend would exhibit.
pub fn run_suite<S: BrainStore>(store: &mut S, manifest: &ProviderManifest) -> ConformanceReport {
    let mut laws = Vec::new();
    let mut law = |level: ConformanceLevel, name: &str, outcome: Result<String, String>| {
        laws.push(LawResult {
            level,
            law: name.into(),
            passed: outcome.is_ok(),
            detail: outcome.unwrap_or_else(|e| e),
        });
    };

    // core: write then read returns the same row at a frontier at/after the commit.
    let core_rw: Result<String, String> = (|| {
        let mut tx = store
            .begin_write(intent("c-rw", None))
            .map_err(|e| e.to_string())?;
        tx.apply(op("a", "conformance write/read law"))
            .map_err(|e| e.to_string())?;
        let receipt = tx
            .commit(Durability::ProcessCrash)
            .map_err(|e| e.to_string())?;
        let frontier = receipt
            .durability
            .local_commit
            .clone()
            .ok_or("commit without local frontier")?;
        let id = receipt
            .entries
            .get("a")
            .cloned()
            .ok_or("entry name not bound")?;
        let snap = store.read_snapshot().map_err(|e| e.to_string())?;
        if snap.frontier() != &frontier {
            return Err(format!(
                "snapshot frontier {:?} != commit frontier {:?}",
                snap.frontier(),
                frontier
            ));
        }
        let rows = snap
            .get(std::slice::from_ref(&id))
            .map_err(|e| e.to_string())?;
        if rows.len() != 1 || rows[0].body["text"] != "conformance write/read law" {
            return Err(format!("read after write returned {rows:?}"));
        }
        Ok(format!(
            "{} readable at {}",
            id.canonical(),
            frontier.restore_epoch
        ))
    })();
    law(ConformanceLevel::Core, "write_then_read", core_rw);

    let core_abort: Result<String, String> = (|| {
        let before = store
            .read_snapshot()
            .map_err(|e| e.to_string())?
            .scan(
                &Predicate::Kind("decision".into()),
                None,
                ScanLimits::default(),
            )
            .map_err(|e| e.to_string())?
            .rows
            .len();
        let mut tx = store
            .begin_write(intent("c-abort", None))
            .map_err(|e| e.to_string())?;
        tx.apply(op("x", "must vanish"))
            .map_err(|e| e.to_string())?;
        tx.abort();
        let after = store
            .read_snapshot()
            .map_err(|e| e.to_string())?
            .scan(
                &Predicate::Kind("decision".into()),
                None,
                ScanLimits::default(),
            )
            .map_err(|e| e.to_string())?
            .rows
            .len();
        if before != after {
            return Err(format!("abort leaked rows: {before} -> {after}"));
        }
        Ok("abort leaves no partial success".into())
    })();
    law(ConformanceLevel::Core, "abort_is_atomic", core_abort);

    let core_history: Result<String, String> = (|| {
        let start = store
            .read_snapshot()
            .map_err(|e| e.to_string())?
            .frontier()
            .clone();
        let mut tx = store
            .begin_write(intent("c-hist", None))
            .map_err(|e| e.to_string())?;
        tx.apply(op("h", "history law"))
            .map_err(|e| e.to_string())?;
        let receipt = tx
            .commit(Durability::ProcessCrash)
            .map_err(|e| e.to_string())?;
        let (changes, _) = store.read_changes(&start, 16).map_err(|e| e.to_string())?;
        let id = receipt.entries.get("h").cloned().ok_or("entry")?;
        if !changes.iter().any(|c| c.target == id) {
            return Err(format!(
                "history after {start:?} does not contain {}: {changes:?}",
                id.canonical()
            ));
        }
        Ok(format!("{} change(s) since frontier", changes.len()))
    })();
    law(
        ConformanceLevel::Core,
        "history_is_replayable_from_a_frontier",
        core_history,
    );

    let core_idem: Result<String, String> = (|| {
        let ops = vec![op("i", "idempotency law")];
        let mut tx = store
            .begin_write(intent("c-idem-1", Some("k1")))
            .map_err(|e| e.to_string())?;
        tx.apply(ops[0].clone()).map_err(|e| e.to_string())?;
        let first = tx
            .commit(Durability::ProcessCrash)
            .map_err(|e| e.to_string())?;
        let replay = store
            .replay("conformance", "k1", &ops)
            .map_err(|e| e.to_string())?;
        if replay.as_ref() != Some(&first) {
            return Err(format!(
                "replay did not return the original receipt: {replay:?}"
            ));
        }
        match store.replay("conformance", "k1", &[op("i", "different payload")]) {
            Err(StoreSpiError::IdempotencyConflict { .. }) => {
                Ok("same key + same payload replays; different payload conflicts".into())
            }
            other => Err(format!(
                "different payload under the same key must conflict: {other:?}"
            )),
        }
    })();
    law(
        ConformanceLevel::Core,
        "idempotency_key_is_exact",
        core_idem,
    );

    let core_exact: Result<String, String> = (|| {
        let snap = store.read_snapshot().map_err(|e| e.to_string())?;
        let page = snap
            .candidates(
                &CandidateProfile::ExactLexical,
                &["history law".into()],
                ScanLimits::default(),
            )
            .map_err(|e| e.to_string())?;
        if page.rows.len() != 1 || !page.coverage.exhausted {
            return Err(format!(
                "exact lexical profile must be complete: {:?}",
                page.coverage
            ));
        }
        Ok("exact lexical candidates are complete and ordered".into())
    })();
    law(
        ConformanceLevel::Core,
        "exact_candidate_profile_is_complete",
        core_exact,
    );

    let durable: Result<String, String> = (|| {
        let diag = store.diagnose().map_err(|e| e.to_string())?;
        let mut tx = store
            .begin_write(intent("c-dur", None))
            .map_err(|e| e.to_string())?;
        tx.apply(op("d", "durability law"))
            .map_err(|e| e.to_string())?;
        let receipt = tx
            .commit(Durability::PowerLossAssumed)
            .map_err(|e| e.to_string())?;
        if !receipt.is_locally_durable() {
            return Err("power-loss commit without a local frontier".into());
        }
        if !diag.integrity_ok {
            return Err("integrity check failed before the durability law".into());
        }
        Ok(format!("ack profile {:?}", receipt.durability.ack_profile))
    })();
    law(
        ConformanceLevel::DurableLocal,
        "power_loss_commit_reports_a_frontier",
        durable,
    );

    let tamper: Result<String, String> = (|| {
        let descriptor = manifest
            .integrity
            .first()
            .ok_or("no integrity descriptor declared")?;
        let digest = descriptor
            .digest(b"tamper law")
            .ok_or_else(|| format!("descriptor {} unsupported", descriptor.key()))?;
        match verify_digest(descriptor, b"tamper law!", &digest) {
            IntegrityVerdict::Mismatch { .. } => {
                Ok(format!("{} detects a one-byte change", descriptor.key()))
            }
            other => Err(format!("tampered bytes verified as {other:?}")),
        }
    })();
    law(
        ConformanceLevel::TamperEvident,
        "digest_detects_mutation",
        tamper,
    );

    let verified: BTreeSet<ConformanceLevel> = ConformanceLevel::ALL
        .iter()
        .copied()
        .filter(|level| {
            let mine: Vec<&LawResult> = laws.iter().filter(|l| l.level == *level).collect();
            !mine.is_empty() && mine.iter().all(|l| l.passed)
        })
        .collect();
    let exercised: BTreeSet<ConformanceLevel> = laws.iter().map(|l| l.level).collect();
    let unverified_claims = manifest
        .claims
        .iter()
        .copied()
        .filter(|c| !exercised.contains(c))
        .collect();
    ConformanceReport {
        provider: manifest.name.clone(),
        claimed: manifest.claims.clone(),
        verified,
        unverified_claims,
        laws,
    }
}
