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

pub(super) fn normalize_token(token: &str) -> String {
    token
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

pub(super) fn kind_class(token: &str) -> Option<&'static str> {
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

pub(super) fn same_qualifier(a: &str, b: &str) -> bool {
    a == b
        || synonym_cluster(a)
            .zip(synonym_cluster(b))
            .is_some_and(|(x, y)| x == y)
}

/// Closed developer lexicon used for query expansion. Not a general thesaurus.
pub fn lexical_cluster_mates(token: &str) -> &'static [&'static str] {
    let norm = normalize_token(token);
    if norm.is_empty() {
        return &[];
    }
    synonym_cluster(&norm)
        .map(|index| SYNONYM_CLUSTERS[index])
        .unwrap_or(&[])
}

fn split_identifier_parts(token: &str) -> Vec<String> {
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
    parts
}

pub(crate) fn is_ticket(trimmed: &str) -> bool {
    let Some((prefix, number)) = trimmed.split_once('-') else {
        return false;
    };
    prefix.len() >= 2
        && prefix.bytes().all(|b| b.is_ascii_alphabetic())
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
}

pub(crate) fn looks_like_http_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

fn is_path_token(trimmed: &str) -> bool {
    trimmed.contains('/') && trimmed.len() > 3 && !looks_like_http_url(trimmed)
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
        let parts = split_identifier_parts(token);
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
        if is_ticket(trimmed) {
            push(
                trimmed.to_string(),
                trimmed.to_lowercase(),
                "ticket".to_string(),
            );
        }
    }
    for token in &tokens {
        let trimmed =
            token.trim_matches(|c: char| c == '`' || c == '"' || c == '\'' || c == ',' || c == '.');
        if is_path_token(trimmed) {
            push(
                trimmed.to_string(),
                trimmed.to_lowercase(),
                "path".to_string(),
            );
        }
    }
    mentions
}
