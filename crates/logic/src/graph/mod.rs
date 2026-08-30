use rusqlite::{params, Connection, OptionalExtension};

const KIND_CLASSES: &[(&str, &str)] = &[
    ("service", "service"),
    ("microservice", "service"),
    ("system", "service"),
    ("daemon", "service"),
    ("server", "service"),
    ("api", "api"),
    ("endpoint", "api"),
    ("db", "store"),
    ("database", "store"),
    ("store", "store"),
    ("cache", "store"),
    ("table", "store"),
    ("queue", "queue"),
    ("topic", "queue"),
    ("pipeline", "pipeline"),
    ("job", "pipeline"),
    ("cluster", "infra"),
    ("cli", "tool"),
    ("binary", "tool"),
];

const SYNONYM_CLUSTERS: &[&[&str]] = &[
    &[
        "auth",
        "authentication",
        "authorization",
        "login",
        "signin",
        "sso",
        "oauth",
        "oauth2",
        "identity",
        "authenticate",
    ],
    &["db", "database", "postgres", "postgresql", "sqlite"],
    &["cache", "caching", "redis", "memcached"],
    &["payments", "payment", "billing", "checkout"],
    &["deploy", "deployment", "release", "rollout"],
    &["log", "logging", "logs", "telemetry", "tracing"],
    &["search", "indexing", "index", "fts"],
    &["queue", "messaging", "broker", "kafka", "rabbitmq"],
    &["webhook", "webhooks", "callback", "callbacks"],
];

#[derive(Debug, Clone, PartialEq)]
pub struct Mention {
    pub surface: String,
    pub qualifier: String,
    pub kind: String,
}

pub fn migrate_entity_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS entities (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          canonical_name TEXT NOT NULL,
          qualifier TEXT NOT NULL,
          kind TEXT NOT NULL DEFAULT '',
          owner_id INTEGER,
          created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE INDEX IF NOT EXISTS idx_entities_qualifier_kind ON entities(qualifier, kind);
        CREATE TABLE IF NOT EXISTS entity_aliases (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          entity_id INTEGER NOT NULL,
          alias TEXT NOT NULL,
          qualifier TEXT NOT NULL,
          status TEXT NOT NULL DEFAULT 'confirmed'
            CHECK (status IN ('confirmed', 'candidate')),
          source_trace_id INTEGER,
          created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_entity_aliases_alias ON entity_aliases(alias, entity_id);
        CREATE INDEX IF NOT EXISTS idx_entity_aliases_entity ON entity_aliases(entity_id);
        CREATE TABLE IF NOT EXISTS entity_mentions (
          id INTEGER PRIMARY KEY AUTOINCREMENT,
          entity_id INTEGER NOT NULL,
          target_type TEXT NOT NULL,
          target_id INTEGER NOT NULL,
          created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_entity_mentions_unique ON entity_mentions(entity_id, target_type, target_id);
        CREATE INDEX IF NOT EXISTS idx_entity_mentions_target ON entity_mentions(target_type, target_id);
        "#,
    )
}

fn normalize_token(token: &str) -> String {
    token
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

fn kind_class(token: &str) -> Option<&'static str> {
    let norm = normalize_token(token);
    KIND_CLASSES
        .iter()
        .find(|(suffix, _)| *suffix == norm)
        .map(|(_, class)| *class)
}

fn synonym_cluster(qualifier: &str) -> Option<usize> {
    SYNONYM_CLUSTERS
        .iter()
        .position(|cluster| cluster.contains(&qualifier))
}

fn same_qualifier(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (synonym_cluster(a), synonym_cluster(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// Closed developer lexicon used for query expansion. Not a general thesaurus.
pub fn lexical_cluster_mates(token: &str) -> &'static [&'static str] {
    let norm = normalize_token(token);
    if norm.is_empty() {
        return &[];
    }
    for cluster in SYNONYM_CLUSTERS {
        if cluster.iter().any(|member| *member == norm) {
            return *cluster;
        }
    }
    &[]
}

/// Extracts deterministic entity mentions from free text.
pub fn extract_mentions(text: &str) -> Vec<Mention> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut mentions: Vec<Mention> = Vec::new();
    let mut push = |surface: String, qualifier: String, kind: String| {
        if qualifier.is_empty() {
            return;
        }
        if !mentions
            .iter()
            .any(|m| m.qualifier == qualifier && m.kind == kind)
        {
            mentions.push(Mention {
                surface,
                qualifier,
                kind,
            });
        }
    };
    for window in tokens.windows(2) {
        if let Some(class) = kind_class(window[1]) {
            let qualifier = normalize_token(window[0]);
            if qualifier.len() > 1 && kind_class(window[0]).is_none() {
                push(
                    format!("{} {}", window[0], window[1]),
                    qualifier,
                    class.to_string(),
                );
            }
        }
    }
    for token in &tokens {
        let cleaned: String = token
            .chars()
            .map(|c| if c == '-' || c == '_' { ' ' } else { c })
            .collect();
        let mut parts: Vec<String> = Vec::new();
        for part in cleaned.split(' ') {
            let mut current = String::new();
            for ch in part.chars() {
                if ch.is_uppercase()
                    && !current.is_empty()
                    && current.chars().last().is_some_and(|p| p.is_lowercase())
                {
                    parts.push(std::mem::take(&mut current));
                }
                current.push(ch);
            }
            if !current.is_empty() {
                parts.push(current);
            }
        }
        if parts.len() == 2 {
            if let Some(class) = kind_class(&parts[1]) {
                let qualifier = normalize_token(&parts[0]);
                if qualifier.len() > 1 && kind_class(&parts[0]).is_none() {
                    push((*token).to_string(), qualifier, class.to_string());
                }
            }
        }
    }
    for token in &tokens {
        let trimmed = token.trim_matches(|c: char| !c.is_alphanumeric());
        let mut split = trimmed.splitn(2, '-');
        if let (Some(prefix), Some(number)) = (split.next(), split.next()) {
            if prefix.len() >= 2
                && prefix.chars().all(|c| c.is_ascii_uppercase())
                && !number.is_empty()
                && number.chars().all(|c| c.is_ascii_digit())
            {
                push(
                    trimmed.to_string(),
                    trimmed.to_lowercase(),
                    "ticket".to_string(),
                );
            }
        }
    }
    for token in &tokens {
        let trimmed =
            token.trim_matches(|c: char| c == '`' || c == '"' || c == '\'' || c == ',' || c == '.');
        if trimmed.contains('/') && trimmed.len() > 3 && !trimmed.starts_with("http") {
            push(
                trimmed.to_string(),
                trimmed.to_lowercase(),
                "path".to_string(),
            );
        }
    }
    mentions
}

/// Resolves a mention to an existing compatible entity or creates one.
/// Records each resolved surface form as an alias.
pub fn resolve_mention(
    conn: &Connection,
    mention: &Mention,
    trace_id: Option<i64>,
    owner_id: Option<i64>,
) -> Option<i64> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT entity_id FROM entity_aliases WHERE alias = ?1 LIMIT 1",
            params![mention.surface.to_lowercase()],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten();
    if let Some(id) = existing {
        return Some(id);
    }
    let mut resolved: Option<i64> = None;
    if let Ok(mut stmt) =
        conn.prepare_cached("SELECT id, qualifier FROM entities WHERE kind = ?1 ORDER BY id ASC")
    {
        if let Ok(rows) = stmt.query_map(params![mention.kind], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        }) {
            for (id, qualifier) in rows.flatten() {
                if same_qualifier(&mention.qualifier, &qualifier) {
                    resolved = Some(id);
                    break;
                }
            }
        }
    }
    let entity_id = match resolved {
        Some(id) => id,
        None => {
            conn.execute(
                "INSERT INTO entities (canonical_name, qualifier, kind, owner_id) VALUES (?1, ?2, ?3, ?4)",
                params![mention.surface, mention.qualifier, mention.kind, owner_id],
            )
            .ok()?;
            conn.last_insert_rowid()
        }
    };
    let _ = conn.execute(
        "INSERT OR IGNORE INTO entity_aliases (entity_id, alias, qualifier, status, source_trace_id) VALUES (?1, ?2, ?3, 'confirmed', ?4)",
        params![entity_id, mention.surface.to_lowercase(), mention.qualifier, trace_id],
    );
    Some(entity_id)
}

/// Resolves text mentions and links their entities to a stored row.
pub fn ingest_for_target(
    conn: &Connection,
    text: &str,
    target_type: &str,
    target_id: Option<i64>,
    trace_id: Option<i64>,
    owner_id: Option<i64>,
) -> Vec<i64> {
    let ids: Vec<i64> = extract_mentions(text)
        .iter()
        .filter_map(|m| resolve_mention(conn, m, trace_id, owner_id))
        .collect();
    if let Some(target) = target_id {
        for entity_id in &ids {
            let _ = conn.execute(
                "INSERT OR IGNORE INTO entity_mentions (entity_id, target_type, target_id) VALUES (?1, ?2, ?3)",
                params![entity_id, target_type, target],
            );
        }
    }
    ids
}

/// Resolves a free-text query to entity IDs.
pub fn resolve_query(conn: &Connection, query: &str) -> Vec<i64> {
    let mut ids: Vec<i64> = extract_mentions(query)
        .iter()
        .filter_map(|m| resolve_mention_to_existing(conn, m))
        .collect();
    for token in query.split_whitespace() {
        let norm = normalize_token(token);
        if norm.len() < 2 || kind_class(&norm).is_some() {
            continue;
        }
        if let Some(id) = lookup_alias(conn, &norm) {
            ids.push(id);
            continue;
        }
        if let Some(id) = lookup_entity_by_qualifier(conn, &norm) {
            ids.push(id);
        }
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn lookup_alias(conn: &Connection, alias: &str) -> Option<i64> {
    conn.query_row(
        "SELECT entity_id FROM entity_aliases WHERE alias = ?1 LIMIT 1",
        params![alias],
        |row| row.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

fn lookup_entity_by_qualifier(conn: &Connection, qualifier: &str) -> Option<i64> {
    let mut stmt = conn
        .prepare_cached("SELECT id, qualifier FROM entities ORDER BY id ASC")
        .ok()?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .ok()?;
    for (id, existing) in rows.flatten() {
        if same_qualifier(qualifier, &existing) {
            return Some(id);
        }
    }
    None
}

fn resolve_mention_to_existing(conn: &Connection, mention: &Mention) -> Option<i64> {
    if let Some(id) = lookup_alias(conn, &mention.surface.to_lowercase()) {
        return Some(id);
    }
    let mut stmt = conn
        .prepare_cached("SELECT id, qualifier FROM entities WHERE kind = ?1 ORDER BY id ASC")
        .ok()?;
    let rows = stmt
        .query_map(params![mention.kind], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .ok()?;
    for (id, qualifier) in rows.flatten() {
        if same_qualifier(&mention.qualifier, &qualifier) {
            return Some(id);
        }
    }
    lookup_entity_by_qualifier(conn, &mention.qualifier)
}

/// Returns rows linked directly or one hop from entities resolved from a query.
/// Uses recall-compatible source keys for rank fusion.
pub fn entity_arm_candidates(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> Vec<(String, String, f64)> {
    let seed_ids = resolve_query(conn, query);
    if seed_ids.is_empty() {
        return Vec::new();
    }
    let mut entity_ids = seed_ids.clone();
    if let Ok(mut stmt) = conn.prepare_cached(
        "SELECT DISTINCT other.entity_id FROM entity_mentions seed \
         JOIN entity_mentions other ON other.target_type = seed.target_type AND other.target_id = seed.target_id \
         WHERE seed.entity_id = ?1 AND other.entity_id != ?1 LIMIT 8",
    ) {
        for seed in &seed_ids {
            if let Ok(rows) = stmt.query_map(params![seed], |row| row.get::<_, i64>(0)) {
                for id in rows.flatten() {
                    if !entity_ids.contains(&id) {
                        entity_ids.push(id);
                    }
                }
            }
        }
    }
    let mut out: Vec<(String, String, f64)> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (hop, entity_id) in entity_ids.iter().enumerate() {
        let score = if seed_ids.contains(entity_id) {
            1.0
        } else {
            0.6
        };
        let _ = hop;
        if let Ok(mut stmt) = conn.prepare_cached(
            "SELECT COALESCE(d.context, 'decision::' || d.id), d.decision FROM entity_mentions em \
             JOIN decisions d ON em.target_type = 'decision' AND d.id = em.target_id \
             WHERE em.entity_id = ?1 AND d.status NOT IN ('superseded','archived') \
               AND (d.expires_at IS NULL OR julianday(d.expires_at) > julianday('now')) \
               AND (d.valid_from IS NULL OR julianday(d.valid_from) <= julianday('now')) AND (d.valid_until IS NULL OR julianday(d.valid_until) > julianday('now')) \
               AND (d.version_id IS NULL OR d.version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned')) \
             ORDER BY d.id DESC LIMIT ?2",
        ) {
            if let Ok(rows) = stmt.query_map(params![entity_id, limit as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
                for (source, text) in rows.flatten() {
                    if seen.insert(source.clone()) {
                        out.push((source, text, score));
                    }
                }
            }
        }
        if let Ok(mut stmt) = conn.prepare_cached(
            "SELECT COALESCE(m.source, 'memory::' || m.id), m.text FROM entity_mentions em \
             JOIN memories m ON em.target_type = 'memory' AND m.id = em.target_id \
             WHERE em.entity_id = ?1 AND m.status NOT IN ('superseded','archived') \
               AND (m.expires_at IS NULL OR julianday(m.expires_at) > julianday('now')) \
               AND (m.valid_from IS NULL OR julianday(m.valid_from) <= julianday('now')) AND (m.valid_until IS NULL OR julianday(m.valid_until) > julianday('now')) \
               AND (m.version_id IS NULL OR m.version_id NOT IN (SELECT id FROM versions WHERE status = 'orphaned')) \
             ORDER BY m.id DESC LIMIT ?2",
        ) {
            if let Ok(rows) = stmt.query_map(params![entity_id, limit as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            }) {
                for (source, text) in rows.flatten() {
                    if seen.insert(source.clone()) {
                        out.push((source, text, score));
                    }
                }
            }
        }
        if out.len() >= limit {
            break;
        }
    }
    out.truncate(limit);
    out
}
