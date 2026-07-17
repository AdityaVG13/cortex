mod download;mod engine;mod profiles;mod vectors;#[cfg(test)]mod tests;pub use download::{ensure_model_downloaded,
ensure_model_downloaded_in};pub use engine::EmbeddingEngine;pub use profiles::{selected_model_assets_exist,selected_model_key,
selected_model_selection};pub use vectors::{blob_to_vector,cosine_similarity,legacy_f32_blob_to_vector,vector_to_blob,
vector_to_pq8_blob,PQ8_FORMAT_VERSION,PQ8_MAGIC_BYTE,};
