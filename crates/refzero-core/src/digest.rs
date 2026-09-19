//! Deterministic digest helpers. Content identity is BLAKE3 over the
//! complete bytes (64 lowercase hex). Port of ZeroStack `zero-abi` digest.

use serde_json::Value;

/// Deterministic JSON encoding with sorted object keys (recursive).
pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()),
        Value::Array(arr) => {
            let parts: Vec<String> = arr.iter().map(canonical_json).collect();
            format!("[{}]", parts.join(","))
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let parts: Vec<String> = keys
                .into_iter()
                .map(|k| {
                    let key = serde_json::to_string(k).unwrap_or_else(|_| "\"\"".into());
                    format!("{key}:{0}", canonical_json(&map[k]))
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
    }
}

/// BLAKE3 of arbitrary bytes, truncated to the 32-byte contract width.
pub fn digest32(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

/// BLAKE3 of `domain || payload`.
pub fn prefixed_digest(domain: &[u8], payload: &[u8]) -> [u8; 32] {
    let mut bound = Vec::with_capacity(domain.len() + payload.len());
    bound.extend_from_slice(domain);
    bound.extend_from_slice(payload);
    digest32(&bound)
}

/// BLAKE3 of `domain || payload_len_be64 || payload`.
pub fn length_prefixed_digest(domain: &[u8], payload: &[u8]) -> [u8; 32] {
    let mut bound = Vec::with_capacity(domain.len() + 8 + payload.len());
    bound.extend_from_slice(domain);
    bound.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    bound.extend_from_slice(payload);
    digest32(&bound)
}

/// BLAKE3 of arbitrary bytes as lowercase hex (64 chars).
pub fn digest_hex(bytes: &[u8]) -> String {
    hex_lower_32(digest32(bytes))
}

/// Digest of a contract manifest: canonical JSON encoding, then BLAKE3 hex.
pub fn contract_digest_hex(manifest: &Value) -> String {
    digest_hex(canonical_json(manifest).as_bytes())
}

/// Raw digest bytes of a contract manifest (BLAKE3 over canonical JSON).
pub fn contract_digest(manifest: &Value) -> [u8; 32] {
    digest32(canonical_json(manifest).as_bytes())
}

/// Lowercase-hex encode arbitrary bytes without rehashing.
pub fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

/// Lowercase-hex encode a 32-byte digest without rehashing.
pub fn hex_lower_32(digest: [u8; 32]) -> String {
    hex_lower(&digest)
}
