//! Portable cross-product blob references (`z://blob/<blake3>`).
//!
//! Port of ZeroStack `zero-ref`: the parser, fragment selector, and error
//! classes shared verbatim across products. Domain adapters share the vectors
//! in `tests/fixtures/zeroref-fixtures.json` (vendored from the reference).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Identity algorithm and hex length shared by the parser and the fixture.
pub const HASH_ALGORITHM: &str = "blake3";
pub const HASH_HEX_LEN: usize = 64;

/// Hash case accepted by the parser and advertised by capability
/// descriptors. Only lowercase input is valid; uppercase is malformed.
pub const HASH_CASE: &str = "lower";

/// The only portable ref kind. Everything else is domain-owned.
pub const PORTABLE_KINDS: [&str; 1] = ["blob"];

/// Exact fragment-semantics strings shared by the contract and conformance tests.
pub const BYTE_FRAGMENT_SEMANTICS: &str = "#B zero-based half-open";
pub const LINE_FRAGMENT_SEMANTICS: &str = "#L one-based inclusive";

/// Stable error classes shared verbatim across products (fixtures error_classes).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZeroRefErrorClass {
    /// Input does not match the portable grammar (bad hash, bad fragment, overflow).
    Malformed,
    /// Recognizable ref that is not a portable blob ref (engine-owned
    /// kinds, unknown schemes, compact forms).
    Unsupported,
    /// Fragment bounds exceed the real byte length or line count under strict
    /// selection. Strict selection never clamps.
    RangeOutOfBounds,
    /// #L selection over bytes that are not valid UTF-8.
    NotUtf8,
    /// Object not present in any reachable store. Reserved for
    /// store/resolution layers. The text parser never emits this class.
    Missing,
    /// Store I/O failed while resolving. Reserved for
    /// store/resolution layers. The text parser never emits this class.
    Io,
    /// Resolved bytes do not hash to the ref identity.
    DigestMismatch,
    /// Resolution denied by storage policy (e.g. shared root not opted in).
    /// Reserved for store/resolution layers. The text parser never emits this class.
    PolicyDenied,
}

impl ZeroRefErrorClass {
    pub const ALL: [ZeroRefErrorClass; 8] = [
        Self::Malformed,
        Self::Unsupported,
        Self::RangeOutOfBounds,
        Self::NotUtf8,
        Self::Missing,
        Self::Io,
        Self::DigestMismatch,
        Self::PolicyDenied,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Malformed => "malformed",
            Self::Unsupported => "unsupported",
            Self::RangeOutOfBounds => "range_out_of_bounds",
            Self::NotUtf8 => "not_utf8",
            Self::Missing => "missing",
            Self::Io => "io",
            Self::DigestMismatch => "digest_mismatch",
            Self::PolicyDenied => "policy_denied",
        }
    }

    /// Classes the text parser and selector construct today.
    pub const PARSER_AND_SELECTOR: [ZeroRefErrorClass; 5] = [
        Self::Malformed,
        Self::Unsupported,
        Self::RangeOutOfBounds,
        Self::NotUtf8,
        Self::DigestMismatch,
    ];

    /// Classes reserved for store/resolution layers. The parser never emits them.
    pub const RESERVED_FOR_RESOLUTION: [ZeroRefErrorClass; 3] =
        [Self::Missing, Self::Io, Self::PolicyDenied];
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZeroRefError {
    pub class: ZeroRefErrorClass,
    pub message: String,
}

impl ZeroRefError {
    pub fn new(class: ZeroRefErrorClass, message: impl Into<String>) -> Self {
        Self {
            class,
            message: message.into(),
        }
    }
}

impl fmt::Display for ZeroRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.class.as_str(), self.message)
    }
}

impl std::error::Error for ZeroRefError {}

/// Identity scheme. One family: `z://blob/<digest>`.
/// Retired product schemes `fz`/`gz`/`tz` do not parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZeroScheme {
    Z,
}

impl ZeroScheme {
    /// Every scheme the parser accepts. The parser iterates THIS list and
    /// capability descriptors report it, so acceptance and advertisement
    /// cannot drift.
    pub const ALL: [ZeroScheme; 1] = [Self::Z];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Z => "z",
        }
    }

    /// Scheme lookup against [ZeroScheme::ALL].
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|scheme| scheme.as_str() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZeroFragment {
    None,
    /// Zero-based half-open byte span. start == end is an allowed empty
    /// selection; start > end never parses.
    Bytes {
        start: u64,
        end: u64,
    },
    /// One-based inclusive line span. start >= 1 and start <= end are
    /// enforced at parse time; the real line count is checked at selection.
    Lines {
        start: u64,
        end: u64,
    },
}

/// A parsed canonical ZeroRef portable blob ref.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZeroRef {
    pub scheme: ZeroScheme,
    /// Full lowercase 64-hex BLAKE3 of the complete unfragmented bytes.
    pub hash: String,
    pub fragment: ZeroFragment,
}

fn malformed(message: impl Into<String>) -> ZeroRefError {
    ZeroRefError::new(ZeroRefErrorClass::Malformed, message)
}

fn unsupported(message: impl Into<String>) -> ZeroRefError {
    ZeroRefError::new(ZeroRefErrorClass::Unsupported, message)
}

/// Full lowercase 64-hex check shared by the parser and store layers.
pub fn is_full_lower_hex(s: &str) -> bool {
    s.len() == HASH_HEX_LEN
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Lowercase hex BLAKE3 of the complete bytes.
pub fn content_hash_hex(bytes: &[u8]) -> String {
    let mut h = blake3::Hasher::new();
    h.update(bytes);
    let digest: [u8; 32] = *h.finalize().as_bytes();
    digest.iter().fold(String::with_capacity(64), |mut out, b| {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
        out
    })
}

/// Binary BLAKE3 digest used by structured identities and span references.
pub type Digest = [u8; 32];

/// Canonical structured identity for a complete object.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct ObjectId(pub Digest);

/// Object identity metadata shared with structured certificate wires.
pub const OBJECT_ID_HASH_ALGORITHM: &str = HASH_ALGORITHM;
pub const OBJECT_ID_HEX_LENGTH: usize = HASH_HEX_LEN;

/// Non-hot-path portable rendering of the object identity convention.
pub fn object_identity_hex(bytes: &[u8]) -> String {
    content_hash_hex(bytes)
}

/// A digest-bound byte selection. Its serde field names and digest arrays are
/// the stable structured wire shape; it does not extend the portable text grammar.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SpanRef {
    pub byte_len: u64,
    pub byte_start: u64,
    pub object_digest: Digest,
    pub object_id: ObjectId,
    pub span_digest: Digest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SpanRefError {
    Selection(ZeroRefError),
    RangeOverflow,
    RangeOutOfBounds,
    ObjectIdentityMismatch,
    ObjectDigestMismatch,
    SpanDigestMismatch,
    PayloadLengthMismatch,
}

impl fmt::Display for SpanRefError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Selection(error) => write!(f, "selection failed: {error}"),
            other => write!(f, "{other:?}"),
        }
    }
}

impl std::error::Error for SpanRefError {}

impl From<ZeroRefError> for SpanRefError {
    fn from(error: ZeroRefError) -> Self {
        Self::Selection(error)
    }
}

fn digest_bytes(bytes: &[u8]) -> Digest {
    let mut hasher = blake3::Hasher::new();
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

impl SpanRef {
    /// Construct a digest-bound span from one borrowed object buffer. Selection
    /// delegates to the canonical ZeroRef selector and returns the selected
    /// borrow with both digests in the SpanRef. No second read is required.
    pub fn from_fragment<'a>(
        object: &'a [u8],
        fragment: &ZeroFragment,
        context: &str,
    ) -> Result<(Self, &'a [u8]), SpanRefError> {
        let selected = select_fragment(object, fragment, context)?;
        let byte_start = (selected.as_ptr() as usize)
            .checked_sub(object.as_ptr() as usize)
            .ok_or(SpanRefError::RangeOverflow)?;
        let byte_start = u64::try_from(byte_start).map_err(|_| SpanRefError::RangeOverflow)?;
        let byte_len = u64::try_from(selected.len()).map_err(|_| SpanRefError::RangeOverflow)?;
        byte_start
            .checked_add(byte_len)
            .ok_or(SpanRefError::RangeOverflow)?;
        let object_digest = digest_bytes(object);
        let span = Self {
            object_id: ObjectId(object_digest),
            byte_start,
            byte_len,
            object_digest,
            span_digest: digest_bytes(selected),
        };
        Ok((span, selected))
    }

    /// Verify only a supplied selected payload. The complete object is not needed.
    pub fn verify_span(&self, payload: &[u8]) -> Result<(), SpanRefError> {
        if u64::try_from(payload.len()).ok() != Some(self.byte_len) {
            return Err(SpanRefError::PayloadLengthMismatch);
        }
        if digest_bytes(payload) != self.span_digest {
            return Err(SpanRefError::SpanDigestMismatch);
        }
        Ok(())
    }

    /// Verify complete-object identity and digest, select the bound byte range,
    /// then verify its independent span digest.
    pub fn verify_and_select<'a>(&self, object: &'a [u8]) -> Result<&'a [u8], SpanRefError> {
        let actual = digest_bytes(object);
        if actual != self.object_id.0 {
            return Err(SpanRefError::ObjectIdentityMismatch);
        }
        if actual != self.object_digest {
            return Err(SpanRefError::ObjectDigestMismatch);
        }
        let end = self
            .byte_start
            .checked_add(self.byte_len)
            .ok_or(SpanRefError::RangeOverflow)?;
        let start = usize::try_from(self.byte_start).map_err(|_| SpanRefError::RangeOutOfBounds)?;
        let end = usize::try_from(end).map_err(|_| SpanRefError::RangeOutOfBounds)?;
        let selected = object
            .get(start..end)
            .ok_or(SpanRefError::RangeOutOfBounds)?;
        self.verify_span(selected)?;
        Ok(selected)
    }
}

fn parse_u64_strict(s: &str, input: &str) -> Result<u64, ZeroRefError> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return Err(malformed(format!("invalid number '{s}' in ref: {input}")));
    }
    s.parse::<u64>()
        .map_err(|_| malformed(format!("number '{s}' overflows u64 in ref: {input}")))
}

impl ZeroRef {
    /// Parse a portable blob ref. Engine-owned forms (non-blob kinds,
    /// compact g:/q: refs) fail as [ZeroRefErrorClass::Unsupported];
    /// anything outside the grammar fails as [ZeroRefErrorClass::Malformed].
    pub fn parse(input: &str) -> Result<Self, ZeroRefError> {
        let Some((scheme_str, rest)) = input.split_once("://") else {
            return Err([
                malformed(format!("not a ZeroRef: {input}")),
                unsupported(format!(
                    "engine compact ref is not a portable ZeroRef: {input}"
                )),
            ][usize::from(input.starts_with("g:") | input.starts_with("q:"))]
            .clone());
        };
        let Some(scheme) = ZeroScheme::parse(scheme_str) else {
            // Unsupported is for recognizable non-portable refs (kind/id shape,
            // fixture vectors unknown_kind/unknown_scheme). A scheme-prefixed
            // string with no path (bare_scheme/kind_without_id fixtures) is
            // outside the grammar entirely and fails as Malformed, matching the
            // live-scheme arms below.
            return Err([
                malformed(format!("missing ref path: {input}")),
                unsupported(format!("unknown ZeroRef scheme '{scheme_str}': {input}")),
            ][usize::from(rest.contains('/'))]
            .clone());
        };
        let Some((kind, tail)) = rest.split_once('/') else {
            return Err(malformed(format!("missing ref path: {input}")));
        };
        let true = PORTABLE_KINDS.contains(&kind) else {
            return Err(unsupported(format!(
                "non-blob ref kind '{kind}' is engine-owned, not portable: {input}"
            )));
        };
        let (hash, frag) = tail
            .split_once('#')
            .map(|(h, f)| (h, Some(f)))
            .unwrap_or((tail, None));
        let false = hash.contains('/') else {
            return Err(malformed(format!(
                "extra path segments after blob hash: {input}"
            )));
        };
        let true = is_full_lower_hex(hash) else {
            return Err(malformed(format!(
                "blob hash must be full lowercase 64-hex BLAKE3: {input}"
            )));
        };
        let fragment = frag
            .map(|f| parse_fragment(f, input))
            .transpose()?
            .unwrap_or(ZeroFragment::None);
        Ok(Self {
            scheme,
            hash: hash.to_string(),
            fragment,
        })
    }

    /// Apply the fragment using the canonical bounds policy without verifying the ref
    /// digest. This is safe only after the bytes have been independently authenticated.
    pub fn unchecked_select<'a>(&self, bytes: &'a [u8]) -> Result<&'a [u8], ZeroRefError> {
        select_fragment(bytes, &self.fragment, &self.to_string())
    }

    /// Apply the fragment with an explicit line-end policy. This is for
    /// compatibility checks and strict validation, not engine defaults.
    pub fn select_with_policy<'a>(
        &self,
        bytes: &'a [u8],
        policy: LineEndPolicy,
    ) -> Result<&'a [u8], ZeroRefError> {
        select_fragment_with_policy(bytes, &self.fragment, &self.to_string(), policy)
    }

    /// Verify the complete unfragmented bytes against the ref identity, then
    /// select the fragment with the canonical policy.
    pub fn verify_and_select<'a>(&self, bytes: &'a [u8]) -> Result<&'a [u8], ZeroRefError> {
        let actual = content_hash_hex(bytes);
        if actual != self.hash {
            return Err(ZeroRefError::new(
                ZeroRefErrorClass::DigestMismatch,
                format!("bytes hash to {actual}, ref claims {}", self.hash),
            ));
        }
        self.unchecked_select(bytes)
    }

    /// Verify the complete bytes, then select with an explicit line-end policy.
    pub fn verify_and_select_with_policy<'a>(
        &self,
        bytes: &'a [u8],
        policy: LineEndPolicy,
    ) -> Result<&'a [u8], ZeroRefError> {
        let actual = content_hash_hex(bytes);
        if actual != self.hash {
            return Err(ZeroRefError::new(
                ZeroRefErrorClass::DigestMismatch,
                format!("bytes hash to {actual}, ref claims {}", self.hash),
            ));
        }
        self.select_with_policy(bytes, policy)
    }
}

fn parse_fragment(f: &str, input: &str) -> Result<ZeroFragment, ZeroRefError> {
    if let Some(span) = f.strip_prefix('B') {
        let Some((s, e)) = span.split_once('-') else {
            return Err(malformed(format!(
                "malformed byte fragment '#B{span}': {input}"
            )));
        };
        let (start, end) = (parse_u64_strict(s, input)?, parse_u64_strict(e, input)?);
        let true = start <= end else {
            return Err(malformed(format!("byte span end before start: {input}")));
        };
        return Ok(ZeroFragment::Bytes { start, end });
    }
    if let Some(span) = f.strip_prefix('L') {
        let Some((a, b)) = span.split_once('-') else {
            return Err(malformed(format!(
                "malformed line fragment '#L{span}': {input}"
            )));
        };
        let (start, end) = (parse_u64_strict(a, input)?, parse_u64_strict(b, input)?);
        let true = start != 0 else {
            return Err(malformed(format!("line numbering is one-based: {input}")));
        };
        let true = start <= end else {
            return Err(malformed(format!("line span end before start: {input}")));
        };
        return Ok(ZeroFragment::Lines { start, end });
    }
    Err(malformed(format!("unknown fragment '{f}': {input}")))
}

/// How line spans whose end runs past EOF are treated at selection time.
/// The canonical policy is LineEndPolicy::ClampEnd. Strict remains
/// available for compatibility checks and callers that validate exact bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEndPolicy {
    Strict,
    ClampEnd,
}

/// Canonical ZeroRef line-end policy. Byte bounds and line starts remain strict.
pub const CANONICAL_LINE_END_POLICY: LineEndPolicy = LineEndPolicy::ClampEnd;

/// Shared canonical fragment selector: byte bounds and line starts are strict;
/// a line end past EOF clamps to the final line.
/// Callers must digest-verify the complete object bytes first.
pub fn select_fragment<'a>(
    bytes: &'a [u8],
    fragment: &ZeroFragment,
    context: &str,
) -> Result<&'a [u8], ZeroRefError> {
    select_fragment_with_policy(bytes, fragment, context, CANONICAL_LINE_END_POLICY)
}

/// Fragment selector with an explicit line-end policy. Byte spans are always
/// exact; only line-span end handling varies by policy.
pub fn select_fragment_with_policy<'a>(
    bytes: &'a [u8],
    fragment: &ZeroFragment,
    context: &str,
    policy: LineEndPolicy,
) -> Result<&'a [u8], ZeroRefError> {
    match *fragment {
        ZeroFragment::None => Ok(bytes),
        ZeroFragment::Bytes { start, end } => {
            if start > end {
                // The parser rejects reversed spans, but legacy surfaces
                // can construct fragments directly; never let one panic.
                return Err(malformed(format!("byte span end before start: {context}")));
            }
            let len = bytes.len() as u64;
            if end > len {
                return Err(ZeroRefError::new(
                    ZeroRefErrorClass::RangeOutOfBounds,
                    format!("byte span {start}-{end} exceeds blob length {len}: {context}"),
                ));
            }
            Ok(&bytes[start as usize..end as usize])
        }
        ZeroFragment::Lines { start, end } => {
            if start == 0 || start > end {
                return Err(malformed(format!("invalid line span: {context}")));
            }
            select_lines(bytes, start, end, context, policy)
        }
    }
}

/// Line selection semantics (annex line-fragment rules): lines terminate at
/// LF; a selected line keeps its terminating LF when present; CR is ordinary
/// content; the final line may be unterminated; the empty blob has zero lines.
fn select_lines<'a>(
    bytes: &'a [u8],
    start: u64,
    end: u64,
    context: &str,
    policy: LineEndPolicy,
) -> Result<&'a [u8], ZeroRefError> {
    if std::str::from_utf8(bytes).is_err() {
        return Err(ZeroRefError::new(
            ZeroRefErrorClass::NotUtf8,
            format!("line fragment over non-UTF-8 content: {context}"),
        ));
    }
    // line_starts[i] is the byte offset where line i+1 begins.
    let line_starts = line_start_offsets(bytes);
    let line_count = line_starts.len() as u64;
    let end = resolve_policy_end(policy, start, end, line_count, context)?;
    Ok(line_span_bytes(bytes, &line_starts, start, end))
}

/// Byte offsets where each 1-based line begins. Empty blob → empty vec.
fn line_start_offsets(bytes: &[u8]) -> Vec<usize> {
    let mut line_starts: Vec<usize> = Vec::new();
    if !bytes.is_empty() {
        line_starts.push(0);
        for (i, b) in bytes.iter().enumerate() {
            if *b == b'\n' && i + 1 < bytes.len() {
                line_starts.push(i + 1);
            }
        }
    }
    line_starts
}

/// Resolve inclusive end line under Strict or ClampEnd. Error messages stay
/// policy-specific (user-facing #L hints); do not merge arms into one predicate.
fn resolve_policy_end(
    policy: LineEndPolicy,
    start: u64,
    end: u64,
    line_count: u64,
    context: &str,
) -> Result<u64, ZeroRefError> {
    match policy {
        LineEndPolicy::Strict => {
            if end > line_count {
                return Err(ZeroRefError::new(
                    ZeroRefErrorClass::RangeOutOfBounds,
                    format!("line span {start}-{end} exceeds line count {line_count}: {context}"),
                ));
            }
            Ok(end)
        }
        LineEndPolicy::ClampEnd => {
            if line_count == 0 {
                return Err(ZeroRefError::new(
                    ZeroRefErrorClass::RangeOutOfBounds,
                    format!("line span {start}-{end} on empty blob (0 lines): {context}"),
                ));
            }
            if start > line_count {
                return Err(ZeroRefError::new(
                    ZeroRefErrorClass::RangeOutOfBounds,
                    format!(
                        "line span start {start} exceeds line count {line_count}: {context}; use #L1-{line_count} or omit the fragment for full content"
                    ),
                ));
            }
            Ok(end.min(line_count))
        }
    }
}

/// Slice bytes for 1-based inclusive line range [start, end] using precomputed starts.
fn line_span_bytes<'a>(bytes: &'a [u8], line_starts: &[usize], start: u64, end: u64) -> &'a [u8] {
    let start_byte = line_starts[(start - 1) as usize];
    let end_byte = if (end as usize) < line_starts.len() {
        line_starts[end as usize]
    } else {
        bytes.len()
    };
    &bytes[start_byte..end_byte]
}

impl fmt::Display for ZeroRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://blob/{}", self.scheme.as_str(), self.hash)?;
        match self.fragment {
            ZeroFragment::None => Ok(()),
            ZeroFragment::Bytes { start, end } => write!(f, "#B{start}-{end}"),
            ZeroFragment::Lines { start, end } => write!(f, "#L{start}-{end}"),
        }
    }
}

impl FromStr for ZeroRef {
    type Err = ZeroRefError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for ZeroRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for ZeroRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}
