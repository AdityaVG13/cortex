//! Repeatable CPU+SQLite hot-path bench for store/conflict/recall/boot.
//!
//! ```text
//! cargo run -p cortex-tests --example hotpath_bench --profile release-perf
//! HOTPATH_ROUNDS=20 samply record --save-only -o docs/internal/perf/cpu.json -- \
//!   ./target/release-perf/examples/hotpath_bench
//! ```
use cortex_daemon::compiler;
use cortex_daemon::conflict;
use cortex_daemon::db;
use cortex_daemon::handlers::recall::{execute_unified_recall, RecallContext};
use cortex_daemon::handlers::store::store_decision_with_ttl;
use cortex_tests::support::runtime_state;
use rusqlite::Connection;
use std::path::Path;
use std::time::Instant;

fn open_file_db(path: &Path) -> Connection {
    let conn = Connection::open(path).expect("open sqlite");
    db::configure(&conn).expect("configure");
    db::initialize_schema(&conn).expect("schema");
    db::run_pending_migrations(&conn);
    conn
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).floor() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn time_loop<F: FnMut()>(iters: usize, mut body: F) -> (f64, f64, f64, f64) {
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t0 = Instant::now();
        body();
        samples.push(t0.elapsed().as_secs_f64() * 1e3);
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    (mean, percentile(&samples, 0.50), percentile(&samples, 0.95), percentile(&samples, 0.99))
}

fn seed_decisions(conn: &mut Connection, n: usize, prefix: &str) {
    for i in 0..n {
        let decision = format!("{prefix} {i}: persist sqlite wal checkpoints in cortex-daemon/src/db/maintenance.rs after store_decision");
        store_decision_with_ttl(
            conn,
            &decision,
            Some(format!("seed::{i}")),
            Some("decision".into()),
            "hotpath-bench".into(),
            Some(0.9),
            None,
            None,
        )
        .unwrap_or_else(|err| panic!("seed {i}: {err}"));
    }
}

fn print_row(name: &str, ops: usize, mean: f64, p50: f64, p95: f64, p99: f64) {
    let ops_sec = if mean > 0.0 { 1000.0 / mean } else { 0.0 };
    println!("{name:<28} ops={ops:<5} mean={mean:8.3}ms p50={p50:8.3}ms p95={p95:8.3}ms p99={p99:8.3}ms ops/s={ops_sec:10.1}");
}

fn maybe_print_row(round: usize, name: &str, ops: usize, mean: f64, p50: f64, p95: f64, p99: f64) {
    if round == 0 {
        print_row(name, ops, mean, p50, p95, p99);
    }
}

#[tokio::main]
async fn main() {
    let rounds: usize = std::env::var("HOTPATH_ROUNDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(1);
    for round in 0..rounds {
        run_round(round).await;
    }
}

async fn run_round(round: usize) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("hotpath.db");
    let mut write = open_file_db(&db_path);
    let read = open_file_db(&db_path);

    let t_seed = Instant::now();
    seed_decisions(&mut write, 800, "seed");
    if round == 0 {
        println!("seed_800_store_ms={:.1}", t_seed.elapsed().as_secs_f64() * 1e3);
    }

    let corpus_a = "persist sqlite wal checkpoints in cortex-daemon/src/db/maintenance.rs after store_decision";
    let corpus_b = "hybrid keyword plus semantic recall uses rrf fusion in handlers/recall/engine.rs";
    let (mean, p50, p95, p99) = time_loop(8_000, || {
        let _ = conflict::jaccard_similarity(corpus_a, corpus_b);
        let _ = conflict::jaccard_similarity(corpus_a, corpus_a);
    });
    maybe_print_row(round, "jaccard_pair", 8000, mean, p50, p95, p99);

    let (mean, p50, p95, p99) = time_loop(200, || {
        conflict::detect_conflict(&write, corpus_a, "hotpath-bench", None).expect("detect");
    });
    maybe_print_row(round, "detect_conflict", 200, mean, p50, p95, p99);

    let mut store_i = 0usize;
    let (mean, p50, p95, p99) = time_loop(200, || {
        let decision = format!("live {store_i}: keep FTS5 porter tokenizer aligned with recall MATCH queries in engine.rs");
        store_decision_with_ttl(
            &mut write,
            &decision,
            Some(format!("live::{store_i}")),
            Some("decision".into()),
            "hotpath-bench".into(),
            Some(0.9),
            None,
            None,
        )
        .expect("store");
        store_i += 1;
    });
    maybe_print_row(round, "store_decision", 200, mean, p50, p95, p99);

    let home = dir.path();
    let (mean, p50, p95, p99) = time_loop(40, || {
        let _ = compiler::compile(&write, home, "hotpath-bench", 320);
    });
    maybe_print_row(round, "boot_compile", 40, mean, p50, p95, p99);

    let state = runtime_state(
        open_file_db(&db_path),
        read,
        false,
        None,
        cortex_daemon::rerank::RerankConfig::off(),
        None,
    );
    let ctx = RecallContext::solo();
    let queries = [
        "sqlite wal checkpoint",
        "fts5 porter tokenizer recall",
        "store_decision conflict jaccard",
        "boot capsule compile budget",
    ];
    let mut samples = Vec::with_capacity(80);
    for i in 0..80 {
        let query = queries[i % queries.len()];
        let t0 = Instant::now();
        execute_unified_recall(&state, query, 320, 12, "hotpath-bench", &ctx, None)
            .await
            .expect("recall");
        samples.push(t0.elapsed().as_secs_f64() * 1e3);
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    let mean = samples.iter().sum::<f64>() / samples.len() as f64;
    maybe_print_row(
        round,
        "unified_recall",
        80,
        mean,
        percentile(&samples, 0.50),
        percentile(&samples, 0.95),
        percentile(&samples, 0.99),
    );
}
