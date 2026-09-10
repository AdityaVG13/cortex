//! Optional hybrid/vector search backed by Dicklesworthstone crates.
//!
//! Feature-gated (`hybrid-search`, `vector-search`, `toon-payloads`) so the
//! default build keeps the SQLite FTS5 + Clock-Quorum recall path untouched.
//!
//! No-claim boundary: these adapters are in-memory only; nothing here is
//! persisted to the brain database, and recall quality is unmeasured until a
//! contract test and benchmark exist.

#[cfg(feature = "hybrid-search")]
use std::collections::HashMap;

#[cfg(feature = "hybrid-search")]
use frankensearch::{
    HashAlgorithm, HashEmbedder, InMemoryTwoTierIndex, InMemoryVectorIndex, ScoreSource,
    ScoredResult, SearchResult, SyncLexicalSearch, SyncTwoTierSearcher, TwoTierConfig,
};
#[cfg(feature = "hybrid-search")]
use frankensearch::core::generation::EmbeddingIdentityBundleV1;
#[cfg(feature = "hybrid-search")]
use frankensearch::core::types::{BoundQueryEmbedding, TieredQueryEmbeddings};

#[cfg(feature = "hybrid-search")]
const EMBED_DIM: usize = 384;

#[cfg(feature = "hybrid-search")]
struct FtsLexical {
    /// Precomputed lexical candidates for the current query.
    hits: Vec<ScoredResult>,
}

#[cfg(feature = "hybrid-search")]
impl SyncLexicalSearch for FtsLexical {
    fn search_sync(&self, _query_vec: &[f32], limit: usize) -> SearchResult<Vec<ScoredResult>> {
        Ok(self.hits.iter().take(limit).cloned().collect())
    }
}

/// In-memory hybrid searcher: hash-embedded vectors fused (RRF) with the
/// existing FTS5 lexical candidates.
#[cfg(feature = "hybrid-search")]
pub struct HybridRecallIndex {
    searcher: SyncTwoTierSearcher,
    embedder: HashEmbedder,
    texts: HashMap<String, String>,
}

#[cfg(feature = "hybrid-search")]
impl HybridRecallIndex {
    /// Build an index over `(doc_id, text)` pairs. `lexical_hits` are the
    /// FTS5 candidates for a query, pre-scored by the caller (BM25).
    pub fn build(
        documents: &[(String, String)],
        lexical_hits: Vec<(String, f32)>,
    ) -> Result<Self, String> {
        let embedder = HashEmbedder::new(EMBED_DIM, HashAlgorithm::FnvModular);
        let mut doc_ids = Vec::with_capacity(documents.len());
        let mut vectors = Vec::with_capacity(documents.len());
        let mut texts = HashMap::with_capacity(documents.len());
        for (id, text) in documents {
            doc_ids.push(id.clone());
            vectors.push(embedder.embed_sync(text));
            texts.insert(id.clone(), text.clone());
        }
        let fast_index = InMemoryVectorIndex::from_vectors(doc_ids, vectors, EMBED_DIM)
            .map_err(|e| format!("build vector index: {e}"))?;
        let index = std::sync::Arc::new(InMemoryTwoTierIndex::new(fast_index, None));
        let lexical = FtsLexical {
            hits: lexical_hits
                .into_iter()
                .map(|(doc_id, score)| ScoredResult {
                    doc_id: doc_id.into(),
                    score,
                    source: ScoreSource::Lexical,
                    index: None,
                    fast_score: None,
                    quality_score: None,
                    lexical_score: Some(score),
                    rerank_score: None,
                    explanation: None,
                    metadata: None,
                })
                .collect(),
        };
        let searcher = SyncTwoTierSearcher::new(index, TwoTierConfig::default())
            .with_lexical(std::sync::Arc::new(lexical));
        Ok(Self {
            searcher,
            embedder,
            texts,
        })
    }

    /// Fuse semantic + lexical candidates for `query`; returns
    /// `(doc_id, fused_score, excerpt)` ranked by fused score.
    pub fn search(&self, query: &str, k: usize) -> Result<Vec<(String, f32, String)>, String> {
        let query_vec = self.embedder.embed_sync(query);
        // Synthetic identity is explicit: hash-controlled vectors, not a
        // semantic model. See `explicit_test_model` contract upstream.
        let identity = EmbeddingIdentityBundleV1::explicit_test_model(
            "cortex-hash-control-v1",
            EMBED_DIM as u32,
        );
        let bound = BoundQueryEmbedding::new(query_vec, identity)
            .map_err(|e| format!("bind query embedding: {e}"))?;
        let tiered = TieredQueryEmbeddings::fast_only(bound);
        let (results, _metrics) = self
            .searcher
            .search_collect(&tiered, k)
            .map_err(|e| format!("hybrid search: {e}"))?;
        Ok(results
            .into_iter()
            .map(|r| {
                let excerpt = self
                    .texts
                    .get(r.doc_id.as_str())
                    .map(|t| crate::handlers::truncate_chars(t, 240))
                    .unwrap_or_default();
                (r.doc_id.to_string(), r.score, excerpt)
            })
            .collect())
    }
}

/// Approximate nearest-neighbour index (HNSW) over precomputed embeddings.
///
/// Deterministic: hnswlib-rs is seed-stable for a fixed insertion order.
#[cfg(feature = "vector-search")]
pub struct AnnIndex {
    hnsw: hnswlib_rs::hnsw::Hnsw<String, hnswlib_rs::metric::Cosine<f32>>,
    vectors: hnswlib_rs::vectors::InMemoryVectorStore<f32>,
    dim: usize,
    count: usize,
}

#[cfg(feature = "vector-search")]
impl AnnIndex {
    pub fn new(dim: usize, max_nodes: usize) -> Self {
        let cfg = hnswlib_rs::hnsw::HnswConfig::new(dim, max_nodes);
        Self {
            hnsw: hnswlib_rs::hnsw::Hnsw::new(hnswlib_rs::metric::Cosine::new(), cfg),
            vectors: hnswlib_rs::vectors::InMemoryVectorStore::new(dim, max_nodes),
            dim,
            count: 0,
        }
    }

    pub fn insert(&mut self, key: String, vector: &[f32]) -> Result<(), String> {
        if vector.len() != self.dim {
            return Err(format!(
                "dimension mismatch: expected {}, got {}",
                self.dim,
                vector.len()
            ));
        }
        self.hnsw
            .insert(&self.vectors, key, vector)
            .map_err(|e| format!("hnsw insert: {e}"))?;
        self.count += 1;
        Ok(())
    }

    /// Returns `(key, distance)` pairs, ascending distance.
    pub fn search(&self, query: &[f32], k: usize) -> Result<Vec<(String, f32)>, String> {
        if self.count == 0 {
            return Ok(Vec::new());
        }
        let hits = self
            .hnsw
            .search(&self.vectors, query, k, None)
            .map_err(|e| format!("hnsw search: {e}"))?;
        Ok(hits.into_iter().map(|h| (h.key, h.distance)).collect())
    }
}

/// Encode a JSON value as TOON (Token-Oriented Object Notation) for compact
/// LLM payloads. Falls back to JSON on failure.
#[cfg(feature = "toon-payloads")]
pub fn to_toon_or_json(value: &serde_json::Value) -> String {
    toon_rust::to_string(value).unwrap_or_else(|_| value.to_string())
}
