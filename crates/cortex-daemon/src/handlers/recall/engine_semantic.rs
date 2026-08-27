pub(crate) fn scale_semantic_similarity(sim: f32) -> f64 {
    SEMANTIC_SCALE_BASE + (sim as f64 - SEMANTIC_SIM_FLOOR) * ((1.0 - SEMANTIC_SCALE_BASE) / (1.0 - SEMANTIC_SIM_FLOOR))
}
pub(crate) fn scale_semantic_similarity_with_keyword_overlap(sim: f32, text: &str, keyword_terms: &[String]) -> f64 {
    let mut scaled = scale_semantic_similarity(sim);
    if !keyword_terms.is_empty() {
        let haystack = text.to_lowercase();
        let overlap = keyword_terms.iter().filter(|term| haystack.contains(term.as_str())).count();
        scaled *= if overlap == 0 { 0.82 } else { 1.0 + (overlap as f64 / keyword_terms.len().max(1) as f64) * 0.08 };
    }
    scaled
}
fn upsert_best_semantic_candidate(
    candidates: &mut HashMap<String, SemanticCandidate>, source: String, excerpt: String, relevance: f64, importance: f64, ts: i64,
) {
    let entry = candidates
        .entry(source.clone())
        .or_insert(SemanticCandidate { source, excerpt: excerpt.clone(), relevance, importance, ts });
    if relevance > entry.relevance {
        *entry = SemanticCandidate { source: entry.source.clone(), excerpt, relevance, importance, ts };
    }
}
#[derive(Clone, Copy)]
enum SemanticEmbedKind {
    Memory,
    Decision,
}
impl SemanticEmbedKind {
    fn target_type(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Decision => "decision",
        }
    }
    fn join_table(self) -> &'static str {
        match self {
            Self::Memory => "memories",
            Self::Decision => "decisions",
        }
    }
    fn source_filter(self) -> &'static str {
        match self {
            Self::Memory => "(?5 IS NULL OR m.source LIKE ?5)",
            Self::Decision => "(?5 IS NULL OR d.context LIKE ?5)",
        }
    }
    fn query_with_acl(self) -> String {
        let table = self.join_table();
        let alias = if matches!(self, Self::Memory) { "m" } else { "d" };
        let cols = match self {
            Self::Memory => {
                "e.vector, m.text, m.source, m.owner_id, m.visibility, m.score, \
                 m.trust_score, m.last_accessed, m.created_at"
            }
            Self::Decision => {
                "e.vector, d.decision, d.context, d.owner_id, d.visibility, d.score, \
                 d.trust_score, d.last_accessed, d.created_at"
            }
        };
        self.query(cols, alias, table)
    }
    fn query_without_acl(self) -> String {
        let table = self.join_table();
        let alias = if matches!(self, Self::Memory) { "m" } else { "d" };
        let cols = match self {
            Self::Memory => {
                "e.vector, m.text, m.source, NULL AS owner_id, NULL AS visibility, m.score, \
                 m.trust_score, m.last_accessed, m.created_at"
            }
            Self::Decision => {
                "e.vector, d.decision, d.context, NULL AS owner_id, NULL AS visibility, d.score, \
                 d.trust_score, d.last_accessed, d.created_at"
            }
        };
        self.query(cols, alias, table)
    }
    fn shadow_query_with_acl(self) -> String {
        let table = self.join_table();
        let alias = if matches!(self, Self::Memory) { "m" } else { "d" };
        let cols = match self {
            Self::Memory => "e.vector, m.source, m.owner_id, m.visibility",
            Self::Decision => "e.vector, d.decision, d.context, d.owner_id, d.visibility",
        };
        self.query(cols, alias, table)
    }
    fn shadow_query_without_acl(self) -> String {
        let table = self.join_table();
        let alias = if matches!(self, Self::Memory) { "m" } else { "d" };
        let cols = match self {
            Self::Memory => "e.vector, m.source, NULL AS owner_id, NULL AS visibility",
            Self::Decision => "e.vector, d.decision, d.context, NULL AS owner_id, NULL AS visibility",
        };
        self.query(cols, alias, table)
    }
    fn query(self, cols: &str, alias: &str, table: &str) -> String {
        let order_expr = match self {
            Self::Memory => {
                "COALESCE(m.score, 1.0) * COALESCE(m.trust_score, 0.8) DESC, \
                 COALESCE(m.last_accessed, m.created_at) DESC, m.id DESC"
            }
            Self::Decision => {
                "COALESCE(d.score, 1.0) * COALESCE(d.trust_score, 0.8) DESC, \
                 COALESCE(d.last_accessed, d.created_at) DESC, d.id DESC"
            }
        };
        format!(
            "SELECT {cols}
             FROM embeddings e
             JOIN {table} {alias}
               ON e.target_type = '{target_type}'
              AND e.target_id = {alias}.id
             WHERE {alias}.status = 'active'
               AND ({alias}.expires_at IS NULL OR {alias}.expires_at > datetime('now'))
               AND ({alias}.valid_from IS NULL OR {alias}.valid_from <= datetime('now'))
               AND ({alias}.valid_until IS NULL OR {alias}.valid_until > datetime('now'))
               AND (e.model IS NULL OR LOWER(e.model) = ?1)
               AND (
                    length(e.vector) = ?2
                    OR (length(e.vector) = ?3 AND substr(e.vector, 1, 2) = ?4)
               )
               AND {source_filter}
             ORDER BY {order_expr}
             LIMIT ?6",
            target_type = self.target_type(),
            source_filter = self.source_filter(),
        )
    }
}
fn prepare_acl_stmt<'a>(conn: &'a Connection, with_acl: &str, without_acl: &str) -> Option<rusqlite::Statement<'a>> {
    match conn.prepare(with_acl) {
        Ok(stmt) => Some(stmt),
        Err(err) if is_missing_team_visibility_columns(&err) => conn.prepare(without_acl).ok(),
        Err(_) => None,
    }
}
fn decision_source_key(context: Option<String>, decision: &str) -> String {
    context.unwrap_or_else(|| format!("decision::{}", decision.chars().take(40).collect::<String>()))
}
pub(crate) fn collect_semantic_candidates(
    conn: &Connection, query_vector: &[f32], query_text: &str, ctx: &RecallContext, source_prefix: Option<&str>,
) -> Vec<SemanticCandidate> {
    let selected_model = crate::embeddings::selected_model_key();
    let expected_legacy_vector_bytes = std::mem::size_of_val(query_vector) as i64;
    let expected_pq8_vector_bytes = (query_vector.len() + crate::embeddings::PQ8_HEADER_BYTES) as i64;
    let pq8_prefix = [crate::embeddings::PQ8_MAGIC_BYTE, crate::embeddings::PQ8_FORMAT_VERSION];
    let candidate_limit = MAX_SEMANTIC_SQL_ROWS_PER_KIND as i64;
    let source_like = source_prefix.map(|prefix| format!("{prefix}%"));
    let keyword_terms = extract_search_keywords(query_text);
    let semantic_floor = if keyword_terms.len() >= 3 { SEMANTIC_SIM_FLOOR + 0.12 } else { SEMANTIC_SIM_FLOOR };
    let mut candidates: HashMap<String, SemanticCandidate> = HashMap::new();
    for kind in [SemanticEmbedKind::Memory, SemanticEmbedKind::Decision] {
        let Some(mut stmt) = prepare_acl_stmt(conn, &kind.query_with_acl(), &kind.query_without_acl()) else {
            continue;
        };
        let Ok(rows) = stmt.query_map(
            params![
                selected_model,
                expected_legacy_vector_bytes,
                expected_pq8_vector_bytes,
                pq8_prefix.as_slice(),
                source_like.as_deref(),
                candidate_limit
            ],
            |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<f64>>(5)?,
                    row.get::<_, Option<f64>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            },
        ) else {
            continue;
        };
        for (blob, primary, alt, owner_id, visibility, score, trust_score, last_accessed, created_at) in rows.flatten() {
            if !is_visible(owner_id, visibility.as_deref(), ctx) {
                continue;
            }
            let source = match kind {
                SemanticEmbedKind::Memory => primary.clone(),
                SemanticEmbedKind::Decision => decision_source_key(alt, &primary),
            };
            if !source_matches_prefix(&source, source_prefix) {
                continue;
            }
            let sim = crate::embeddings::cosine_similarity(query_vector, &crate::embeddings::blob_to_vector(&blob));
            if sim <= semantic_floor as f32 {
                continue;
            }
            let scaled = scale_semantic_similarity_with_keyword_overlap(sim, &primary, &keyword_terms);
            let excerpt = query_focused_excerpt(&primary, query_text, 280);
            let importance = blend_importance(score, trust_score);
            let ts = parse_timestamp_ms(last_accessed.as_deref().or(created_at.as_deref()).unwrap_or_default());
            upsert_best_semantic_candidate(&mut candidates, source, excerpt, scaled, importance, ts);
        }
    }
    let mut sorted: Vec<SemanticCandidate> = candidates.into_values().collect();
    sorted.sort_by(|a, b| compare_relevance_desc_source_asc(a.relevance, &a.source, b.relevance, &b.source));
    sorted.truncate(MAX_SEMANTIC_RRF_CANDIDATES);
    sorted
}
