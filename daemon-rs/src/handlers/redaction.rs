// SPDX-License-Identifier: MIT
use regex::Regex;
use std::sync::OnceLock;

static BEARER_REDACTION_RE: OnceLock<Option<Regex>> = OnceLock::new();
static HASH_REDACTION_RE: OnceLock<Option<Regex>> = OnceLock::new();
static CREDENTIAL_REDACTION_RE: OnceLock<Option<Regex>> = OnceLock::new();

// Apply redactions in three passes so broad credential masking does not hide
// structured bearer/hash patterns from earlier, more specific replacements.
pub fn redact_secrets(text: &str) -> String {
    let bearer = BEARER_REDACTION_RE
        .get_or_init(|| Regex::new(r"Bearer\s+[a-f0-9]{32,}").ok())
        .as_ref()
        .map(|re| re.replace_all(text, "Bearer [REDACTED]").to_string())
        .unwrap_or_else(|| text.to_string());
    let hashes = HASH_REDACTION_RE
        .get_or_init(|| Regex::new(r"[a-f0-9]{40,}").ok())
        .as_ref()
        .map(|re| re.replace_all(&bearer, "[HASH_REDACTED]").to_string())
        .unwrap_or(bearer);
    CREDENTIAL_REDACTION_RE
        .get_or_init(|| Regex::new(r"(?i)(?:token|key|secret|password)\s*[:=]\s*\S+").ok())
        .as_ref()
        .map(|re| re.replace_all(&hashes, "[CREDENTIAL_REDACTED]").to_string())
        .unwrap_or(hashes)
}
