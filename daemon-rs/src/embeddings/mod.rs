// SPDX-License-Identifier: MIT
//! In-process ONNX embedding engine.

mod profiles;
mod engine;
mod vectors;
mod download;

#[cfg(test)]
mod tests;

pub use profiles::{
    selected_model_assets_exist, selected_model_key, selected_model_selection, EmbeddingModelSelection,
};
pub use engine::EmbeddingEngine;
pub use vectors::{
    blob_to_vector, cosine_similarity, is_pq8_blob, legacy_f32_blob_to_vector, pq8_blob_to_vector,
    vector_to_blob, vector_to_legacy_f32_blob, vector_to_pq8_blob, PQ8_FORMAT_VERSION,
    PQ8_HEADER_BYTES, PQ8_MAGIC_BYTE,
};
pub use download::{ensure_model_downloaded, ensure_model_downloaded_in};
