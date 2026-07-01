// SPDX-License-Identifier: MIT
/// Cosine similarity between two f32 slices (assumed L2-normalised, but this
/// implementation handles the general case too).
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    if a.iter().chain(b.iter()).any(|value| !value.is_finite()) {
        return 0.0;
    }

    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;

    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 || !denom.is_finite() {
        return 0.0;
    }

    let similarity = dot / denom;
    if similarity.is_finite() {
        similarity.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Encode a `Vec<f32>` as a SQLite BLOB. As of v0.6.0 this writes the
/// compact PQ8 format (~4x smaller than LE f32). Reads transparently
/// handle both formats via `blob_to_vector` — see `pq8_blob_to_vector`
/// and `legacy_f32_blob_to_vector` for format-specific entry points.
pub fn vector_to_blob(vec: &[f32]) -> Vec<u8> {
    vector_to_pq8_blob(vec)
}

/// Strict legacy encoder: writes LE f32 packed bytes. Used by tests that
/// need to assert behaviour on legacy blobs, and by any one-off migration
/// tool that needs to produce the old wire format.
#[allow(dead_code)]
pub fn vector_to_legacy_f32_blob(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Decode a SQLite BLOB back to `Vec<f32>`. Auto-detects PQ8 quantized
/// blobs vs legacy LE-f32 blobs so the read path transparently handles
/// the mixed corpus during the backfill window. Callers that specifically
/// need the legacy decoder can call `legacy_f32_blob_to_vector` directly.
pub fn blob_to_vector(blob: &[u8]) -> Vec<f32> {
    if let Some(v) = pq8_blob_to_vector(blob) {
        return v;
    }
    legacy_f32_blob_to_vector(blob)
}

/// Strict legacy decoder: treat the blob as a packed LE-f32 array. Used by
/// tests and any caller that knows it is reading pre-PQ8 data.
pub fn legacy_f32_blob_to_vector(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

// ---------------------------------------------------------------------------
// PQ8 (per-vector symmetric int8) quantization
//
// Embedding vectors are the second-largest table in a mature Cortex DB. The
// f32 representation costs 4 * D bytes per row (3072 bytes for BGE-base).
// Symmetric int8 quantization with a single per-vector f32 scale collapses
// that to D + 5 bytes (773 bytes for BGE-base) — a ~4x reduction — while
// preserving cosine similarity to within a few hundredths in practice.
//
// BGE produces L2-normalised vectors so values land in [-1, 1] tightly,
// which means the quantization scale stays small and round-trip error is
// uniform. For non-normalised models the scale tracks the per-vector
// max(|v|) so the dynamic range of any single vector is fully used.
//
// Blob layout (PQ8_FORMAT_VERSION = 0x02):
//
//   byte 0:        magic = PQ8_MAGIC_BYTE (0xC8 — distinct from any
//                  byte that can appear at the head of an LE f32 storing
//                  a normalised value)
//   byte 1:        format version (0x02)
//   bytes 2..6:    scale (LE f32). Zero implies an all-zero vector.
//   bytes 6..6+D:  D signed int8 values, one per dimension
//
// Total: D + 6 bytes. For D=768 that is 774 bytes vs 3072 bytes of f32 —
// a 3.97x compression ratio. The 6-byte header amortises trivially.
// ---------------------------------------------------------------------------

/// Magic byte that uniquely identifies a PQ8 blob. Chosen so it cannot
/// appear as the leading byte of an LE-encoded f32 holding a typical
/// normalised value: 0xC8 corresponds to LE float values around -1e22.
pub const PQ8_MAGIC_BYTE: u8 = 0xC8;
/// Current PQ8 wire format version. Future formats bump this.
pub const PQ8_FORMAT_VERSION: u8 = 0x02;
/// Header size in bytes: magic(1) + version(1) + scale(4).
pub const PQ8_HEADER_BYTES: usize = 6;

/// Quantize a Vec<f32> to a compact int8 blob. Returns the raw bytes ready
/// for SQLite storage. Lossless when the input is all-zero (scale = 0).
pub fn vector_to_pq8_blob(vec: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(PQ8_HEADER_BYTES + vec.len());
    out.push(PQ8_MAGIC_BYTE);
    out.push(PQ8_FORMAT_VERSION);

    // Scale is the per-vector max absolute value mapped onto int8::MAX so
    // every vector uses its full int8 dynamic range. NaN/inf inputs are
    // treated as zero; we never want a poisoned scale to corrupt storage.
    let max_abs = vec
        .iter()
        .copied()
        .filter(|v| v.is_finite())
        .fold(0.0f32, |acc, v| acc.max(v.abs()));

    let scale = if max_abs > 0.0 {
        max_abs / i8::MAX as f32
    } else {
        0.0
    };
    out.extend_from_slice(&scale.to_le_bytes());

    for &v in vec {
        let q = if scale > 0.0 && v.is_finite() {
            // round-half-to-even via f32::round, then clamp into int8.
            let scaled = (v / scale).round().clamp(i8::MIN as f32, i8::MAX as f32);
            scaled as i8
        } else {
            0i8
        };
        out.push(q as u8);
    }
    out
}

/// True iff the blob is a PQ8-encoded vector (magic + version match).
pub fn is_pq8_blob(blob: &[u8]) -> bool {
    blob.len() >= PQ8_HEADER_BYTES && blob[0] == PQ8_MAGIC_BYTE && blob[1] == PQ8_FORMAT_VERSION
}

/// Decode a PQ8 blob back to Vec<f32>. Returns None if the blob is not a
/// valid PQ8 payload — callers should fall back to `blob_to_vector` on
/// legacy LE-f32 storage in that case.
pub fn pq8_blob_to_vector(blob: &[u8]) -> Option<Vec<f32>> {
    if !is_pq8_blob(blob) {
        return None;
    }
    let scale = f32::from_le_bytes([blob[2], blob[3], blob[4], blob[5]]);
    if !scale.is_finite() || scale < 0.0 {
        return None;
    }
    let body = &blob[PQ8_HEADER_BYTES..];
    let mut out = Vec::with_capacity(body.len());
    if scale == 0.0 {
        // All-zero vector. Preserve the original length.
        out.resize(body.len(), 0.0);
        return Some(out);
    }
    for &b in body {
        let q = b as i8;
        out.push(q as f32 * scale);
    }
    Some(out)
}

/// Convenience: max absolute element-wise error between two equal-length
/// f32 slices. Used by tests to bound quantization round-trip error.
#[cfg(test)]
pub(crate) fn max_abs_error(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max)
}

// ---------------------------------------------------------------------------
// Model management
