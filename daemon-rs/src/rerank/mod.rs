// SPDX-License-Identifier: MIT
//! Cross-encoder reranker support for recall.

mod config;
mod assets;
mod engine;

#[cfg(test)]
mod tests;

pub use config::{RerankConfig, RerankMode};
pub use assets::{
    ensure_reranker_downloaded, ensure_reranker_downloaded_in, selected_reranker_assets_exist,
    selected_reranker_selection, RerankerSelection,
};
pub use engine::{
    fuse_scores, MiniLmReranker, RerankCandidate, RerankedScore, Reranker,
};
