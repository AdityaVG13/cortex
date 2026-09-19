//! Backend certification boundary.
//!
//! A provider *claims* levels; the suite *verifies* the ones it can exercise
//! in-process. Integrity is a pluggable descriptor: an unsupported algorithm
//! yields `Unverified`, never a silent pass. An unknown optional index falls
//! back to the exact profile; an unknown *required* security scheme blocks.

use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum ConformanceLevel {
    /// Exact read/write/history/idempotency semantics.
    Core,
    /// Local commit survives process death (and power loss when the ack profile says so).
    DurableLocal,
    /// Concurrent local writers see one serial history.
    ConcurrentLocal,
    /// Causal merge, origin dedup, revocation and tombstones across replicas.
    Replicated,
    /// Digests over records detect tampering.
    TamperEvident,
}

impl ConformanceLevel {
    pub const ALL: [Self; 5] = [
        Self::Core,
        Self::DurableLocal,
        Self::ConcurrentLocal,
        Self::Replicated,
        Self::TamperEvident,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Core => "core",
            Self::DurableLocal => "durable_local",
            Self::ConcurrentLocal => "concurrent_local",
            Self::Replicated => "replicated",
            Self::TamperEvident => "tamper_evident",
        }
    }
}

/// One digest descriptor: algorithm, domain separator and canonicalization
/// are all part of the identity; two descriptors may live side by side
/// during rotation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DigestDescriptor {
    pub algorithm: String,
    pub domain: String,
    pub canonicalization: String,
}

impl DigestDescriptor {
    /// BLAKE3 over the complete record bytes (`raw`: no trim). The domain
    /// stays part of the descriptor identity and the digest input.
    pub fn blake3(domain: &str) -> Self {
        Self {
            algorithm: "blake3".into(),
            domain: domain.into(),
            canonicalization: "raw".into(),
        }
    }
    pub fn key(&self) -> String {
        format!(
            "{}/{}/{}",
            self.algorithm, self.domain, self.canonicalization
        )
    }
    fn canonical(&self, bytes: &[u8]) -> Option<Vec<u8>> {
        match self.canonicalization.as_str() {
            // Integrity paths hash the complete bytes. The retired
            // `utf8-nfc-trim` spelling is unsupported: legacy descriptors
            // verify as `Unverified`, never as a silent pass.
            "raw" => Some(bytes.to_vec()),
            _ => None,
        }
    }
    /// Digest `bytes` under this descriptor. `None` when the algorithm or
    /// canonicalization is not supported by this build.
    pub fn digest(&self, bytes: &[u8]) -> Option<String> {
        let canonical = self.canonical(bytes)?;
        match self.algorithm.as_str() {
            "blake3" => {
                let mut h = blake3::Hasher::new();
                h.update(self.domain.as_bytes());
                h.update(&[0u8]);
                h.update(&canonical);
                Some(h.finalize().to_hex().to_string())
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "verdict")]
pub enum IntegrityVerdict {
    Verified,
    Mismatch {
        expected: String,
        actual: String,
    },
    /// Not a failure and not a pass: the descriptor cannot be evaluated here.
    Unverified {
        reason: String,
    },
}

pub fn verify_digest(
    descriptor: &DigestDescriptor,
    bytes: &[u8],
    expected: &str,
) -> IntegrityVerdict {
    match descriptor.digest(bytes) {
        None => IntegrityVerdict::Unverified {
            reason: format!("unsupported digest descriptor {}", descriptor.key()),
        },
        Some(actual) if actual == expected => IntegrityVerdict::Verified,
        Some(actual) => IntegrityVerdict::Mismatch {
            expected: expected.to_string(),
            actual,
        },
    }
}

/// Rotation: verify against every descriptor stored side by side; the
/// record is verified when any supported descriptor verifies, unverified when
/// none is supported, mismatched when a supported one disagrees.
pub fn verify_rotation(digests: &[(DigestDescriptor, String)], bytes: &[u8]) -> IntegrityVerdict {
    let mut unverified = Vec::new();
    for (descriptor, expected) in digests {
        match verify_digest(descriptor, bytes, expected) {
            IntegrityVerdict::Verified => return IntegrityVerdict::Verified,
            IntegrityVerdict::Mismatch { expected, actual } => {
                return IntegrityVerdict::Mismatch { expected, actual };
            }
            IntegrityVerdict::Unverified { reason } => unverified.push(reason),
        }
    }
    IntegrityVerdict::Unverified {
        reason: if unverified.is_empty() {
            "no digests recorded".into()
        } else {
            unverified.join("; ")
        },
    }
}

/// Optional accelerators a provider may offer; unknown ones degrade to exact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexResolution {
    Accelerated(String),
    ExactFallback { requested: String },
}

pub fn resolve_index(manifest: &ProviderManifest, requested: &str) -> IndexResolution {
    if manifest.optional_indexes.contains(requested) {
        IndexResolution::Accelerated(requested.to_string())
    } else {
        IndexResolution::ExactFallback {
            requested: requested.to_string(),
        }
    }
}

/// Required security schemes never degrade: an unknown one blocks the store.
pub fn resolve_security(
    manifest: &ProviderManifest,
    required: &[String],
) -> Result<(), StoreSpiError> {
    let unknown: Vec<&String> = required
        .iter()
        .filter(|s| !manifest.security_schemes.contains(*s))
        .collect();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(StoreSpiError::Unavailable(format!(
            "required security scheme(s) not provided: {}",
            unknown
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ProviderManifest {
    pub name: String,
    pub claims: BTreeSet<ConformanceLevel>,
    pub integrity: Vec<DigestDescriptor>,
    pub optional_indexes: BTreeSet<String>,
    pub security_schemes: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct LawResult {
    pub level: ConformanceLevel,
    pub law: String,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ConformanceReport {
    pub provider: String,
    pub claimed: BTreeSet<ConformanceLevel>,
    pub verified: BTreeSet<ConformanceLevel>,
    /// Claimed but not exercisable in-process (needs a harness: replicas, power cut).
    pub unverified_claims: BTreeSet<ConformanceLevel>,
    pub laws: Vec<LawResult>,
}

impl ConformanceReport {
    pub fn failures(&self) -> Vec<&LawResult> {
        self.laws.iter().filter(|l| !l.passed).collect()
    }
    /// A provider is certified for the levels it claims and the suite verified.
    pub fn certified(&self) -> BTreeSet<ConformanceLevel> {
        self.claimed.intersection(&self.verified).copied().collect()
    }
    /// Claims that the suite exercised and refuted.
    pub fn refuted(&self) -> BTreeSet<ConformanceLevel> {
        let failed: BTreeSet<ConformanceLevel> = self
            .laws
            .iter()
            .filter(|l| !l.passed)
            .map(|l| l.level)
            .collect();
        self.claimed.intersection(&failed).copied().collect()
    }
}

mod suite;
pub use suite::run_suite;

/// Side-by-side digests for one record, keyed by descriptor.
pub fn digest_set(
    descriptors: &[DigestDescriptor],
    bytes: &[u8],
) -> BTreeMap<String, Option<String>> {
    descriptors
        .iter()
        .map(|d| (d.key(), d.digest(bytes)))
        .collect()
}
