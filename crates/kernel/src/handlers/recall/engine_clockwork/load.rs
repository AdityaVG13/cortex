use super::{
    RecallContext, ScoredCandidate, SearchTableKind, as_of_bind, caller_acl_param,
    candidate_matches_source_scope, feedback_use_score, is_visible, like_prefix, qualified_acl,
    search_source_key, source_prefix_applies_to_kind, source_prefix_is_path, source_scope_sql,
    temporal_gates,
};
use rusqlite::{Connection, OptionalExtension, params};

struct FtsKind {
    target_type: &'static str,
    alias: &'static str,
    excerpt: &'static str,
    source: &'static str,
    ident: &'static str,
    table: &'static str,
    fts: &'static str,
    bm25: &'static str,
}

const DECISION_FTS: FtsKind = FtsKind {
    target_type: "decision",
    alias: "d",
    excerpt: "d.decision",
    source: "COALESCE(d.context, 'decision::' || d.id)",
    ident: "'decision::' || d.id",
    table: "decisions",
    fts: "decisions_fts",
    bm25: "6.6, 1.0",
};

const MEMORY_FTS: FtsKind = FtsKind {
    target_type: "memory",
    alias: "m",
    excerpt: "m.text",
    source: "COALESCE(m.source, 'memory::' || m.id)",
    ident: "'memory::' || m.id",
    table: "memories",
    fts: "memories_fts",
    bm25: "4.6, 1.7, 2.2",
};

fn fts_kind(kind: &str) -> FtsKind {
    if kind == "decision" {
        DECISION_FTS
    } else {
        MEMORY_FTS
    }
}

pub(super) fn fts_rows(
    conn: &Connection,
    kind: &str,
    fts_query: &str,
    limit: usize,
    source_prefix: Option<&str>,
    ctx: &RecallContext,
) -> Result<Vec<LoadedRow>, String> {
    if !source_prefix_applies_to_kind(kind, source_prefix) {
        return Ok(Vec::new());
    }
    let spec = fts_kind(kind);
    let source_like = source_prefix.map(like_prefix);
    let path_guard = source_prefix.filter(|p| source_prefix_is_path(p));
    let caller = caller_acl_param(ctx);
    let gates = temporal_gates(spec.alias, ctx, "?5");
    let acl = qualified_acl(spec.alias, "?4");
    let scope = source_scope_sql(spec.source, spec.ident);
    let sql = format!(
        "SELECT {a}.id, {excerpt}, {source}, {a}.owner_id, {a}.visibility, {a}.created_at, {a}.status, {a}.valid_from, {a}.valid_until FROM {fts} fts JOIN {table} {a} ON {a}.id = fts.rowid WHERE {fts} MATCH ?1 AND {gates} AND {scope} {acl} ORDER BY bm25({fts}, {bm25}) LIMIT ?2",
        a = spec.alias,
        excerpt = spec.excerpt,
        source = spec.source,
        fts = spec.fts,
        table = spec.table,
        bm25 = spec.bm25
    );
    let mut stmt = conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
    let as_of = as_of_bind(ctx);
    let rows = stmt
        .query_map(
            params![
                fts_query,
                limit as i64,
                source_like,
                caller,
                as_of,
                path_guard
            ],
            |row| {
                Ok(LoadedRow {
                    target_type: spec.target_type.to_string(),
                    target_id: row.get(0)?,
                    excerpt: row.get(1)?,
                    source: row.get(2)?,
                    owner_id: row.get(3)?,
                    visibility: row.get(4)?,
                    ts_raw: row.get(5)?,
                    status: row.get(6)?,
                    valid_from: row.get(7)?,
                    valid_until: row.get(8)?,
                })
            },
        )
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for row in rows.flatten() {
        if !is_visible(row.owner_id, row.visibility.as_deref(), ctx) {
            continue;
        }
        if !candidate_matches_source_scope(
            &row.target_type,
            row.target_id,
            &row.source,
            source_prefix,
        ) {
            continue;
        }
        out.push(row);
    }
    Ok(out)
}

pub(super) struct LoadedRow {
    pub(super) target_type: String,
    pub(super) target_id: i64,
    pub(super) excerpt: String,
    pub(super) source: String,
    pub(super) owner_id: Option<i64>,
    pub(super) visibility: Option<String>,
    pub(super) ts_raw: Option<String>,
    pub(super) status: String,
    pub(super) valid_from: Option<String>,
    pub(super) valid_until: Option<String>,
}

pub(super) fn loaded_candidate(
    conn: &Connection,
    row: &LoadedRow,
    hops: u8,
    write: u8,
    truth: u8,
    task: u8,
    history: u8,
    hard_anchor: bool,
    strong_lexical: bool,
    specificity: u8,
) -> Result<ScoredCandidate, String> {
    let kind = if row.target_type == "decision" {
        SearchTableKind::Decisions
    } else {
        SearchTableKind::Memories
    };
    let source = search_source_key(kind, row.target_id, Some(row.source.as_str()));
    let use_score = feedback_use_score(conn, &source)?;
    Ok(ScoredCandidate {
        target_type: row.target_type.clone(),
        target_id: row.target_id,
        source,
        excerpt: row.excerpt.clone(),
        owner_id: row.owner_id,
        visibility: row.visibility.clone(),
        ts: crate::handlers::parse_timestamp_ms(row.ts_raw.as_deref().unwrap_or("")),
        hops,
        write,
        truth,
        task,
        history,
        hard_anchor,
        strong_lexical,
        specificity,
        fts_rank: (write as i64) * 100,
        use_score,
        anchors: Vec::new(),
        links: Vec::new(),
        arms: Vec::new(),
        status: row.status.clone(),
        valid_from: row.valid_from.clone(),
        valid_until: row.valid_until.clone(),
        witnesses: Vec::new(),
        required_role: false,
        contradiction: false,
    })
}

pub(super) fn load_target(
    conn: &Connection,
    target_type: &str,
    target_id: i64,
    ctx: &RecallContext,
) -> Result<Option<ScoredCandidate>, String> {
    let caller = caller_acl_param(ctx);
    let as_of = as_of_bind(ctx);
    let gates = temporal_gates("x", ctx, "?3").replace("x.", "");
    let acl = qualified_acl("x", "?2").replace("x.", "");
    let (excerpt_col, source_sql, table) = if target_type == "memory" {
        ("text", "COALESCE(source, 'memory::' || id)", "memories")
    } else {
        (
            "decision",
            "COALESCE(context, 'decision::' || id)",
            "decisions",
        )
    };
    let sql = format!(
        "SELECT {excerpt_col}, {source_sql}, owner_id, visibility, created_at, status, valid_from, valid_until FROM {table} WHERE id = ?1 AND {gates} {acl}"
    );
    let mut stmt = conn.prepare_cached(&sql).map_err(|e| e.to_string())?;
    let row = stmt
        .query_row(params![target_id, caller, as_of], |row| {
            Ok(LoadedRow {
                target_type: target_type.to_string(),
                target_id,
                excerpt: row.get(0)?,
                source: row.get(1)?,
                owner_id: row.get(2)?,
                visibility: row.get(3)?,
                ts_raw: row.get(4)?,
                status: row.get(5)?,
                valid_from: row.get(6)?,
                valid_until: row.get(7)?,
            })
        })
        .optional()
        .map_err(|e| e.to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    if !is_visible(row.owner_id, row.visibility.as_deref(), ctx) {
        return Ok(None);
    }
    loaded_candidate(conn, &row, 0, 0, 0, 0, 0, false, false, 0).map(Some)
}
