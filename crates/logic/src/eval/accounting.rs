//! Token economics kept honest: five quantities that are never summed.
//!
//! * `D_task` — bytes the task itself needed regardless of memory.
//! * `D_read` — bytes the agent read from memory (Views, expansions).
//! * `D_transport` — bytes moved on the wire / IPC.
//! * `D_context` — bytes that ended up in the model context window.
//! * `D_price` — the price actually charged, in the provider's unit.
//!
//! Bytes are counted without a tokenizer; a tokenizer estimate is labeled as
//! such and never replaces the byte count.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Accounting {
    pub d_task_bytes: u64,
    pub d_read_bytes: u64,
    pub d_transport_bytes: u64,
    pub d_context_bytes: u64,
    /// Provider unit (e.g. USD); `None` when unknown.
    pub d_price: Option<f64>,
    #[serde(default)]
    pub price_unit: Option<String>,
    /// Optional labeled estimate; informational only.
    #[serde(default)]
    pub tokenizer_estimate: Option<TokenizerEstimate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TokenizerEstimate {
    pub tokenizer: String,
    pub context_tokens: u64,
}

impl Accounting {
    pub fn add_read(&mut self, bytes: usize) {
        self.d_read_bytes += bytes as u64;
    }
    pub fn add_transport(&mut self, bytes: usize) {
        self.d_transport_bytes += bytes as u64;
    }
    pub fn add_context(&mut self, bytes: usize) {
        self.d_context_bytes += bytes as u64;
    }
    /// Five separate quantities. There is deliberately no `total()`:
    /// summing bytes with prices, or reads with transport, measures nothing.
    pub fn report(&self) -> serde_json::Value {
        serde_json::json!({"unit":"utf8_bytes","d_task":self.d_task_bytes,"d_read":self.d_read_bytes,"d_transport":self.d_transport_bytes,"d_context":self.d_context_bytes,"d_price":self.d_price,"price_unit":self.price_unit,"tokenizer_estimate":self.tokenizer_estimate,"never_summed":true})
    }
    /// Context saved relative to a no-memory baseline for the same task: a
    /// difference of like quantities, reported beside the read cost it took.
    pub fn context_delta_vs(&self, baseline: &Accounting) -> serde_json::Value {
        serde_json::json!({"baseline_context_bytes":baseline.d_context_bytes,"with_memory_context_bytes":self.d_context_bytes,"delta_bytes":baseline.d_context_bytes as i64 - self.d_context_bytes as i64,"read_cost_bytes":self.d_read_bytes,"note":"delta and read cost are separate; a saving that costs more reads than it saves is not a saving"})
    }
}

/// Latency at declared percentiles (nearest-rank).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Latency {
    pub p50_micros: u128,
    pub p95_micros: u128,
    pub p99_micros: u128,
    pub worst_micros: u128,
    pub samples: usize,
}

impl Latency {
    pub fn from_samples(mut micros: Vec<u128>) -> Self {
        micros.sort();
        let pct = |p: f64| -> u128 {
            if micros.is_empty() {
                return 0;
            }
            let rank = ((p / 100.0) * micros.len() as f64).ceil().max(1.0) as usize;
            micros[rank.min(micros.len()) - 1]
        };
        Self {
            p50_micros: pct(50.0),
            p95_micros: pct(95.0),
            p99_micros: pct(99.0),
            worst_micros: micros.last().copied().unwrap_or(0),
            samples: micros.len(),
        }
    }
}

/// Systems performance report: every surface measured separately, with the
/// environment named. Reflex and full retrieval are never one number.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PerfReport {
    pub hardware: String,
    pub build: String,
    pub corpus_records: usize,
    pub reflex: Option<Latency>,
    pub full_answer: Option<Latency>,
    pub commit: Option<Latency>,
    pub hydration: Option<Latency>,
    pub restore: Option<Latency>,
    pub host_integration: Option<Latency>,
    pub throughput_per_sec: Option<f64>,
    pub rss_bytes: Option<u64>,
    pub maintenance_debt: Option<i64>,
    pub reflex_fallback_rate: Option<f64>,
}
