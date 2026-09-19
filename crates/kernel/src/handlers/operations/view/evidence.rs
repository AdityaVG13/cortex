use super::super::closure::close_revision;
use super::{Card, View, is_constraint_kind, legacy_record, mark_contested};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

impl View {
    /// Required qualifiers and counterevidence travel with every claim.
    pub fn close_evidence(&mut self, conn: &Connection) -> rusqlite::Result<()> {
        for card in &mut self.cards {
            close_card(conn, card)?;
        }
        self.select_within_budget();
        self.watermark(conn);
        Ok(())
    }
}

fn close_card(conn: &Connection, card: &mut Card) -> rusqlite::Result<()> {
    let Some(record_id) = legacy_record(conn, &card.reference)? else {
        if card.reference.contains("::") {
            mark_contested(card, "qualification_unavailable: no record mapping");
        }
        return Ok(());
    };
    let heads = crate::db::records::heads(conn, &record_id)?;
    let Some(revision) = heads.first() else {
        mark_contested(card, "qualification_unavailable: no record head");
        return Ok(());
    };
    let legacy_id = refresh_legacy_kind(conn, card)?;
    let Ok((items, contrary)) = close_revision(conn, revision, legacy_id) else {
        // Fail closed: unavailable exceptions cannot become an asserted claim.
        mark_contested(card, "qualification_unavailable: evidence closure failed");
        return Ok(());
    };
    card.exceptions.extend(
        items
            .into_iter()
            .filter(|item| item.role.is_required())
            .map(|item| format!("{}: {}", item.role.as_str(), item.text)),
    );
    append_contrary(card, &contrary, heads.len());
    append_counterexamples(conn, card, revision);
    // Retention governs deletion, not delivery.
    card.required = card.epistemic == "contested" || is_constraint_kind(&card.kind);
    card.bytes = card.statement.len()
        + card.exceptions.iter().map(|e| e.len() + 2).sum::<usize>()
        + card.exact_text.as_ref().map(|t| t.len()).unwrap_or(0);
    Ok(())
}

fn refresh_legacy_kind(conn: &Connection, card: &mut Card) -> rusqlite::Result<Option<i64>> {
    let Some((namespace, id)) = card
        .reference
        .split_once("::")
        .and_then(|(kind, id)| id.parse::<i64>().ok().map(|id| (kind, id)))
    else {
        return Ok(None);
    };
    let sql = match namespace {
        "decision" => {
            "SELECT COALESCE(type,'decision'), COALESCE(retention_class,'operational') FROM decisions WHERE id = ?1"
        }
        "memory" => {
            "SELECT COALESCE(type,'memory'), COALESCE(retention_class,'operational') FROM memories WHERE id = ?1"
        }
        _ => return Ok(None),
    };
    if let Some((kind, retention)) = conn
        .query_row(sql, params![id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .optional()?
    {
        card.kind = kind;
        card.retention = retention;
    }
    Ok((namespace == "decision").then_some(id))
}

fn append_contrary(card: &mut Card, contrary: &[Value], heads: usize) {
    if !contrary.is_empty() {
        card.epistemic = "contested";
        card.exceptions.extend(contrary.iter().map(|c| {
            format!(
                "contrary head {}: {}",
                c["other"].as_str().unwrap_or("?"),
                c["text"].as_str().unwrap_or("")
            )
        }));
    }
    if heads > 1 {
        card.epistemic = "contested";
        card.exceptions
            .push(format!("{heads} concurrent heads unresolved"));
    }
}

fn append_counterexamples(conn: &Connection, card: &mut Card, revision: &str) {
    // Cases and procedures retain counterexamples with the same preconditions
    // or subject. This best-effort lookup does not change closure error handling.
    if !matches!(card.kind.as_str(), "case" | "procedure") {
        return;
    }
    let Ok(Some(body)) = crate::db::records::revision_body(conn, revision) else {
        return;
    };
    let Ok(mut stmt) = conn.prepare("SELECT r.record_id, v.body_json FROM records r JOIN record_heads h ON h.record_id = r.record_id JOIN revisions v ON v.revision_id = h.revision_id WHERE r.kind = 'counterexample' ORDER BY r.record_id") else { return };
    let Ok(rows) = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
    else {
        return;
    };
    for (id, raw) in rows.flatten() {
        let Ok(other) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        if ["preconditions", "subject"]
            .iter()
            .any(|&key| !body[key].is_null() && body[key] == other[key])
        {
            card.exceptions.push(format!(
                "counterexample {id}: {}",
                other["text"].as_str().unwrap_or("")
            ));
        }
    }
}
