// SPDX-License-Identifier: MIT

mod budget;
mod cache;
mod core;
mod handlers;
mod nlp;
mod pipeline;
mod rerank;
mod scoring;
mod search;
mod semantic;
mod telemetry;
mod types;
mod unfold;

#[cfg(test)]
mod tests;

pub(crate) use budget::*;
pub(crate) use cache::*;
pub(crate) use core::*;
pub(crate) use nlp::*;
pub(crate) use pipeline::*;
pub(crate) use rerank::*;
pub(crate) use scoring::*;
pub(crate) use search::*;
pub(crate) use semantic::*;
pub(crate) use telemetry::*;
pub(crate) use types::*;
pub(crate) use unfold::*;

pub use handlers::{
    handle_budget_recall, handle_peek, handle_recall, handle_recall_explain, handle_recall_post,
    handle_semantic_recall,
};
pub use unfold::handle_unfold;
pub use pipeline::{
    execute_recall_policy_explain, execute_semantic_recall, execute_unified_recall,
};
pub use types::{parse_recall_policy_mode, resolve_recall_budget_k, RecallContext, RecallPolicyMode};
pub use types::shannon_entropy;
pub use unfold::unfold_source;
