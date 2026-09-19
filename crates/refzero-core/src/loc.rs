//! Guest loc grammar. Identity is `@N`. Digest spellings are not locs.

use crate::LocNo;

/// JavaScript `Number.MAX_SAFE_INTEGER`. Guest JSON must not emit larger loc numbers.
pub const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

/// Why a string is not a loc.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocParseError {
    /// Empty, `@0`, `@04`, `@s…`, `@readme.md`, `z://blob/…`, 64-hex, whitespace.
    NotALoc,
}

/// Parse a guest identity string. Anything that is not `@[1-9][0-9]*` is `NotALoc`.
pub fn parse_loc(s: &str) -> Result<LocNo, LocParseError> {
    let rest = s.strip_prefix('@').ok_or(LocParseError::NotALoc)?;
    if rest.is_empty() || rest.as_bytes()[0] == b'0' {
        return Err(LocParseError::NotALoc);
    }
    if !rest.bytes().all(|b| b.is_ascii_digit()) {
        return Err(LocParseError::NotALoc);
    }
    let no = rest.parse::<LocNo>().map_err(|_| LocParseError::NotALoc)?;
    if no > MAX_SAFE_INTEGER {
        return Err(LocParseError::NotALoc);
    }
    Ok(no)
}

/// Format a loc for a guest. Inverse of [`parse_loc`].
pub fn format_loc(no: LocNo) -> Result<String, LocParseError> {
    if no == 0 || no > MAX_SAFE_INTEGER {
        return Err(LocParseError::NotALoc);
    }
    Ok(format!("@{no}"))
}

/// Digest-shaped spelling. Not a loc and not a guest identity.
pub fn is_digest_spelling(s: &str) -> bool {
    if let Some(hex) = s.strip_prefix("z://blob/") {
        return hex.len() == 64 && is_lower_hex(hex);
    }
    s.len() == 64 && is_lower_hex(s)
}

fn is_lower_hex(s: &str) -> bool {
    s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
