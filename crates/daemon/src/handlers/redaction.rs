use regex::Regex;
use std::sync::OnceLock;

static BEARER_REDACTION_RE: OnceLock<Option<Regex>> = OnceLock::new();
static HASH_REDACTION_RE: OnceLock<Option<Regex>> = OnceLock::new();
static CREDENTIAL_REDACTION_RE: OnceLock<Option<Regex>> = OnceLock::new();
static SK_REDACTION_RE: OnceLock<Option<Regex>> = OnceLock::new();
static GITHUB_REDACTION_RE: OnceLock<Option<Regex>> = OnceLock::new();
static SLACK_REDACTION_RE: OnceLock<Option<Regex>> = OnceLock::new();
static AWS_REDACTION_RE: OnceLock<Option<Regex>> = OnceLock::new();
static GENERIC_SECRET_REDACTION_RE: OnceLock<Option<Regex>> = OnceLock::new();

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
    let credential = CREDENTIAL_REDACTION_RE
        .get_or_init(|| Regex::new(r"(?i)(?:token|key|secret|password)\s*[:=]\s*\S+").ok())
        .as_ref()
        .map(|re| re.replace_all(&hashes, "[CREDENTIAL_REDACTED]").to_string())
        .unwrap_or(hashes);
    let sk = SK_REDACTION_RE
        .get_or_init(|| Regex::new(r"sk-[A-Za-z0-9_-]{16,}").ok())
        .as_ref()
        .map(|re| re.replace_all(&credential, "[redacted]").to_string())
        .unwrap_or(credential);
    let github = GITHUB_REDACTION_RE
        .get_or_init(|| Regex::new(r"(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}").ok())
        .as_ref()
        .map(|re| re.replace_all(&sk, "[redacted]").to_string())
        .unwrap_or(sk);
    let slack = SLACK_REDACTION_RE
        .get_or_init(|| Regex::new(r"xox[bp]-[A-Za-z0-9\-]{10,}").ok())
        .as_ref()
        .map(|re| re.replace_all(&github, "[redacted]").to_string())
        .unwrap_or(github);
    let aws = AWS_REDACTION_RE
        .get_or_init(|| Regex::new(r"AKIA[0-9A-Z]{16}").ok())
        .as_ref()
        .map(|re| re.replace_all(&slack, "[redacted]").to_string())
        .unwrap_or(slack);
    GENERIC_SECRET_REDACTION_RE
        .get_or_init(|| Regex::new(r#"(?i)(?:api[_-]?key|secret|token)\s*[:=]\s*["']?[A-Za-z0-9_\-]{20,}"#).ok())
        .as_ref()
        .map(|re| re.replace_all(&aws, "[redacted]").to_string())
        .unwrap_or(aws)
}
