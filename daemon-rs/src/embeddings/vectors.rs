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
pub fn vector_to_blob(vec: &[f32]) -> Vec<u8> {
    vector_to_pq8_blob(vec)
}
#[allow(dead_code)]
pub fn vector_to_legacy_f32_blob(vec: &[f32]) -> Vec<u8> {
    vec.iter().flat_map(|f| f.to_le_bytes()).collect()
}
pub fn blob_to_vector(blob: &[u8]) -> Vec<f32> {
    if let Some(v) = pq8_blob_to_vector(blob) {
        return v;
    }
    legacy_f32_blob_to_vector(blob)
}
pub fn legacy_f32_blob_to_vector(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}
pub const PQ8_MAGIC_BYTE: u8 = 0xC8;
pub const PQ8_FORMAT_VERSION: u8 = 0x02;
pub const PQ8_HEADER_BYTES: usize = 6;
pub fn vector_to_pq8_blob(vec: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(PQ8_HEADER_BYTES + vec.len());
    out.push(PQ8_MAGIC_BYTE);
    out.push(PQ8_FORMAT_VERSION);
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
            let scaled = (v / scale).round().clamp(i8::MIN as f32, i8::MAX as f32);
            scaled as i8
        } else {
            0i8
        };
        out.push(q as u8);
    }
    out
}
pub fn is_pq8_blob(blob: &[u8]) -> bool {
    blob.len() >= PQ8_HEADER_BYTES && blob[0] == PQ8_MAGIC_BYTE && blob[1] == PQ8_FORMAT_VERSION
}
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
        out.resize(body.len(), 0.0);
        return Some(out);
    }
    for &b in body {
        let q = b as i8;
        out.push(q as f32 * scale);
    }
    Some(out)
}
#[cfg(test)]
pub(crate) fn max_abs_error(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0f32, f32::max)
}
