use super::{Caller, arg_list, invalid_field, no_match};
use crate::db::records;
use crate::handlers::recall::{RecallContext, unfold_source};
use crate::protocol::ResponseStatus;
use crate::state::RuntimeState;
use serde_json::{Value, json};

fn compare_kind_is_ident(kind: &str) -> bool {
    let mut chars = kind.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn token_set(text: &str) -> std::collections::BTreeSet<String> {
    text.split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_ascii_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

pub(super) async fn compare(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: &Caller<'_>,
    args: &Value,
) -> Result<Value, String> {
    let refs = arg_list(args, &["compare", "references", "refs"]);
    if refs.len() != 2 {
        return Ok(invalid_field(
            "compare needs exactly two references (decision::N or alias with receipt)",
            "compare",
        ));
    }
    let ctx = RecallContext::from_caller(caller.owner_id, state);
    let conn = state.db_read.lock(cx).await.map_err(|e| e.to_string())?;
    let mut sides = Vec::new();
    for reference in &refs {
        match load_side(&conn, &ctx, reference)? {
            SideOut::Reply(reply) => return Ok(reply),
            SideOut::Side(side) => sides.push(side),
        }
    }
    let a_words = token_set(sides[0]["text"].as_str().unwrap_or(""));
    let b_words = token_set(sides[1]["text"].as_str().unwrap_or(""));
    let mut differing_fields = Vec::new();
    for field in ["kind", "status", "retention", "valid_from", "valid_until"] {
        if sides[0][field] != sides[1][field] {
            differing_fields
                .push(json!({"field": field, "a": sides[0][field], "b": sides[1][field]}));
        }
    }
    let linked = sides[0]["conflicts"].as_array().unwrap().iter().any(|c| {
        let other = refs[1]
            .split_once("::")
            .and_then(|(_, id)| id.parse::<i64>().ok());
        c["source"].as_i64() == other || c["target"].as_i64() == other
    });
    let ordered = sides[0]["created_at"].as_str() <= sides[1]["created_at"].as_str();
    Ok(
        json!({"status":ResponseStatus::Ok.as_str(),"profile":"compare","rule":"compare/1","sides":sides,"comparison":{"differing_fields":differing_fields,"text":{"only_in_a":a_words.difference(&b_words).collect::<Vec<_>>(),"only_in_b":b_words.difference(&a_words).collect::<Vec<_>>(),"shared":a_words.intersection(&b_words).count()},"directly_related":linked,"chronological":if ordered { "a_before_b" } else { "b_before_a" },"interpretation":"field and token alignment only; semantic judgement is the reader's and must be attributed"}}),
    )
}

enum SideOut {
    Side(Value),
    Reply(Value),
}

fn compare_fail(reference: &str, detail: &str) -> Result<SideOut, String> {
    Ok(SideOut::Reply(invalid_field(
        format!("`{reference}` {detail}"),
        "compare",
    )))
}

fn load_side(
    conn: &rusqlite::Connection,
    ctx: &RecallContext,
    reference: &str,
) -> Result<SideOut, String> {
    let Some((kind, id)) = reference.split_once("::") else {
        return compare_fail(reference, "is not a logical reference");
    };
    let Ok(id_num) = id.parse::<i64>() else {
        return compare_fail(reference, "has a non-numeric id");
    };
    let (table, address_ns) = if kind == "decision" {
        ("decisions", "decision")
    } else if compare_kind_is_ident(kind) {
        ("memories", "memory")
    } else {
        return compare_fail(reference, "is not a logical reference");
    };
    let Some(source) = unfold_source(conn, reference, ctx) else {
        return Ok(SideOut::Reply(no_match(format!(
            "`{reference}` is not readable in your scope"
        ))));
    };
    let (kind_col, status, retention, created, valid_from, valid_until) = conn.query_row(&format!("SELECT COALESCE(type, ?2), status, COALESCE(retention_class,'operational'), created_at, valid_from, valid_until FROM {table} WHERE id = ?1"), rusqlite::params![id_num, kind], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, Option<String>>(5)?))).map_err(|e| e.to_string())?;
    let record_id =
        records::record_for_legacy(conn, address_ns, id_num).map_err(|e| e.to_string())?;
    let heads = record_id
        .as_ref()
        .map(|r| records::heads(conn, r))
        .transpose()
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    let conflicts: Vec<Value> = if kind == "decision" {
        let mut stmt = conn.prepare("SELECT id, classification, status, source_decision_id, target_decision_id FROM decision_conflicts WHERE source_decision_id = ?1 OR target_decision_id = ?1 ORDER BY id").map_err(|e| e.to_string())?;
        stmt.query_map(rusqlite::params![id_num], |r| Ok(json!({"conflict": r.get::<_, i64>(0)?, "classification": r.get::<_, String>(1)?, "status": r.get::<_, String>(2)?, "source": r.get::<_, Option<i64>>(3)?, "target": r.get::<_, i64>(4)?}))).map_err(|e| e.to_string())?.flatten().collect()
    } else {
        Vec::new()
    };
    Ok(SideOut::Side(
        json!({"reference": reference, "text": source["text"], "kind": kind_col, "status": status, "retention": retention, "created_at": created, "valid_from": valid_from, "valid_until": valid_until, "record": record_id, "heads": heads, "conflicts": conflicts}),
    ))
}
