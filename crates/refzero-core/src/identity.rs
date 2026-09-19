//! Identity-slot scanner. Flags digest URIs in identity fields only.
//! Does not redact file payload text.

use serde_json::Value;

const IDENTITY_KEYS: &[&str] = &[
    "handle",
    "loc",
    "refetch",
    "identity",
    "zeroHandle",
    "zero_handle",
    "target",
    "next",
    "exact",
    "source",
];

/// JSON pointers (RFC 6901) to identity fields that still look like `z://blob/` or raw 64-hex.
pub fn identity_slot_hits(value: &Value) -> Vec<String> {
    let mut hits = Vec::new();
    walk(value, "", &mut hits);
    hits
}

fn walk(value: &Value, pointer: &str, hits: &mut Vec<String>) {
    match value {
        Value::Object(map) => map.iter().for_each(|(key, child)| {
            let next = format!("{pointer}/{}", escape_pointer(key));
            hits.extend(
                child
                    .as_str()
                    .filter(|_| is_identity_key(key))
                    .filter(|s| crate::is_digest_spelling(s))
                    .map(|_| next.clone()),
            );
            [None, Some(child)][usize::from(!is_payload_key(key))]
                .inspect(|child| walk(child, &next, hits));
        }),
        Value::Array(items) => items.iter().enumerate().for_each(|(i, child)| {
            walk(child, &format!("{pointer}/{i}"), hits);
        }),
        _ => {}
    }
}

fn is_identity_key(key: &str) -> bool {
    IDENTITY_KEYS.contains(&key)
}

fn is_payload_key(key: &str) -> bool {
    matches!(key, "text" | "inline_utf8" | "content" | "view" | "body")
}

fn escape_pointer(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}
