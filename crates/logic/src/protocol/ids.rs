use serde::{Deserialize, Serialize};

/// Logical identity of a Record, Revision, Thread, source, operation or
/// Receipt. Opaque; survives storage migration. Never a physical location.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LogicalId {
    pub namespace: String,
    pub value: String,
}

impl LogicalId {
    pub fn new(namespace: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            namespace: namespace.into(),
            value: value.into(),
        }
    }
    /// Canonical `namespace:value` rendering used for stable tie-breaking.
    pub fn canonical(&self) -> String {
        format!("{}:{}", self.namespace, self.value)
    }
    /// Legacy integer rows keep resolvable aliases: `memory::42` → (`memory`, `42`).
    pub fn from_legacy(kind: &str, id: i64) -> Self {
        Self::new(kind, id.to_string())
    }
    pub fn parse_legacy(reference: &str) -> Option<Self> {
        let (kind, id) = reference.split_once("::")?;
        if kind.is_empty() || id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Some(Self::new(kind, id))
    }
}

/// A Record pinned to one immutable Revision.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExactRef {
    pub record: LogicalId,
    pub revision: LogicalId,
}

/// Physical location inside one storage provider. Not an identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Locator {
    pub provider: String,
    pub namespace: String,
    #[serde(with = "serde_bytes_hex")]
    pub opaque: Vec<u8>,
}

/// Integrity evidence under a named algorithm. Equality of digests is
/// evidence of byte equality under that algorithm's assumptions only.
/// No algorithm or digest size is mandated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Integrity {
    pub algorithm: String,
    pub domain: String,
    pub canonicalization: String,
    #[serde(with = "serde_bytes_hex")]
    pub value: Vec<u8>,
}

/// Provider-defined read/commit frontier, scoped to a restore epoch so a
/// frontier from before a restore cannot be mistaken for a current one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frontier {
    pub provider: String,
    pub restore_epoch: String,
    #[serde(with = "serde_bytes_hex")]
    pub opaque: Vec<u8>,
}

/// Authenticated caller as established by the transport/application boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    pub authenticated_id: String,
    pub policy_epoch: String,
}

mod serde_bytes_hex {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(bytes: &[u8], s: S) -> Result<S::Ok, S::Error> {
        let mut out = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            out.push_str(&format!("{b:02x}"));
        }
        s.serialize_str(&out)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        if text.len() % 2 != 0 {
            return Err(serde::de::Error::custom("hex string must have even length"));
        }
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(serde::de::Error::custom))
            .collect()
    }
}
