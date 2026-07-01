// SPDX-License-Identifier: MIT
//! Relevance Feedback Loop — learns which recalled results are actually useful.

mod recall;
mod agent;
mod handlers;

#[cfg(test)]
mod tests;

pub use recall::{
    compute_boost, compute_boosts, has_retrieval_immunity, record_unfold_feedback, FeedbackRequest,
    IMMUNITY_THRESHOLD, IMMUNITY_WINDOW_DAYS,
};
pub use agent::{
    build_agent_feedback_stats_payload, recommend_recall_k, record_agent_feedback_from_value,
    AgentFeedbackRecordRequest, AgentFeedbackStatsQuery,
};
pub use handlers::{handle_agent_feedback_record, handle_agent_feedback_stats};
pub use recall::{handle_feedback, handle_feedback_stats};
