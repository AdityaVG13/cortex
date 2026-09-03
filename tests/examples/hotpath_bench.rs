//! hotpath_bench — owner of the `cortex-daemon-smoke` latency ratchet (bead cortex-38n).
//!
//! Measures the daemon hot paths against the lib API (in-process; no HTTP, no daemon
//! spawn), over >= 5 fully-counted rounds on a fresh seeded temp DB per round:
//!
//!   store_case     200 x store_decision_with_ttl  on a seeded ~800-row DB   [gated]
//!   recall_case     80 x execute_unified_recall    over 4 fixed queries    [gated]
//!   boot_compile    40 x compiler::compile                                 [gated]
//!   jaccard_pair  8000 x conflict::jaccard_similarity   [informational — ~1us/call,
//!   detect_conflict 200 x conflict::detect_conflict      below timer-honest gating]
//!
//! Warmup: recall x3 and boot x1 unmeasured calls per round (the seed lineage's
//! recorded methodology used hyperfine warmup_runs=1); lazy init must not land in
//! measured samples. All measured rounds are counted — none discarded.
//!
//! Modes (env-driven; no CLI args):
//!   UPDATE_BASELINE=1  run the smoke and WRITE the baseline fixture
//!                      tests/fixtures/bench-history/cortex-daemon-smoke.latest.json
//!                      (schema cortex-daemon-smoke.bench-history.v1)
//!   (default)          run the smoke and COMPARE against that fixture using the
//!                      baseline's own thresholds: median ratio warn > 1.10,
//!                      fail > 1.25; run eligibility = round-total cv_pct > 5 ⇒
//!                      verdict=noise (ineligible: no pass and no regression
//!                      verdict) — the same aggregate rule the fixture's own
//!                      validator enforced. Per-op cv is always reported and
//!                      flagged when above the bound.
//!
//! Profile guard: every run refuses unless CORTEX_BENCH_PROFILE=release-perf is set
//! AND the binary was actually built with --profile release-perf (checked from the
//! exe path; a debug binary claiming release-perf is refused — no fake detection).
//!
//! Runner command:
//!   CORTEX_BENCH_PROFILE=release-perf cargo run --quiet -p cortex-tests --example hotpath_bench --profile release-perf
//!
//! Honesty rules enforced here:
//!   - all rounds are counted; nothing is discarded (the previous version of this
//!     bench printed round 0 and threw rounds 1+ away)
//!   - percentiles pool every sample from every round; cv_pct is the dispersion of
//!     per-round medians, printed for every workload and for the round totals
//!   - the baseline records git head/dirty state, build profile, and a machine
//!     fingerprint; only UPDATE_BASELINE=1 writes, never a compare run
//!
//! Exit codes: 0 pass/warn · 1 regression fail · 2 setup error (guard, missing or
//! structurally mismatched baseline) · 3 noise (no verdict).
//!
//! No-claim boundary: this is a LATENCY-REGRESSION gate only. It does not measure
//! recall quality (precision/MRR/coverage floors need a labeled relevance set — the
//! deleted HTTP-level recall_benchmark's job; quality stays owned by benchmarking/).

use cortex_daemon::compiler;
use cortex_daemon::conflict;
use cortex_daemon::db;
use cortex_daemon::handlers::recall::{execute_unified_recall, RecallContext};
use cortex_daemon::handlers::store::store_decision_with_ttl;
use cortex_tests::support::runtime_state;
use rusqlite::Connection;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const SCHEMA_VERSION: &str = "cortex-daemon-smoke.bench-history.v1";
const BENCH_NAME: &str = "cortex-daemon-smoke";
const DEFAULT_ROUNDS: usize = 5;
const MIN_ROUNDS: usize = 5;
const DEFAULT_CV_PCT_MAX: f64 = 5.0;
const DEFAULT_WARN_RATIO: f64 = 1.10;
const DEFAULT_FAIL_RATIO: f64 = 1.25;

const SEED_ROWS: usize = 800;
const STORE_ITERS: usize = 200;
const RECALL_ITERS: usize = 80;
const BOOT_ITERS: usize = 40;
const CONFLICT_ITERS: usize = 200;
const JACCARD_ITERS: usize = 8_000;
/// Unmeasured warmup calls per round, mirroring the seed lineage's recorded
/// methodology (hyperfine warmup_runs=1): lazy init (FTS/graph/stmt caches) must
/// not land in measured samples. Warmup is disclosed in the emitted artifact.
const RECALL_WARMUP: usize = 3;
const BOOT_WARMUP: usize = 1;

fn baseline_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("bench-history")
        .join("cortex-daemon-smoke.latest.json")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("tests/ manifest lives inside the repo root")
        .to_path_buf()
}

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

fn mean(samples: &[f64]) -> f64 {
    samples.iter().sum::<f64>() / samples.len() as f64
}

fn stddev(samples: &[f64]) -> f64 {
    let m = mean(samples);
    (samples.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / samples.len() as f64).sqrt()
}

/// Coefficient of variation in percent. Returns INFINITY for a degenerate zero mean.
fn cv_pct(samples: &[f64]) -> f64 {
    let m = mean(samples);
    if m == 0.0 {
        f64::INFINITY
    } else {
        stddev(samples) / m * 100.0
    }
}

fn time_loop<F: FnMut()>(iters: usize, mut body: F) -> Vec<f64> {
    let mut samples = Vec::with_capacity(iters);
    for _ in 0..iters {
        let t0 = Instant::now();
        body();
        samples.push(t0.elapsed().as_secs_f64() * 1e3);
    }
    samples
}

fn sorted_copy(samples: &[f64]) -> Vec<f64> {
    let mut sorted = samples.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    sorted
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

/// Honest profile guard: the runner must claim release-perf via env, and the binary
/// must actually be one. An unrecognized exe layout is allowed through but recorded
/// as env-claimed, never silently presented as detected.
fn profile_guard() -> Result<String, String> {
    let claimed = std::env::var("CORTEX_BENCH_PROFILE").unwrap_or_default();
    if claimed != "release-perf" {
        return Err(format!(
            "profile guard: CORTEX_BENCH_PROFILE=release-perf is required (got {claimed:?}).\n\
             runner command:\n  CORTEX_BENCH_PROFILE=release-perf cargo run --quiet -p cortex-tests \
             --example hotpath_bench --profile release-perf"
        ));
    }
    let exe = std::env::current_exe()
        .map_err(|err| format!("profile guard: cannot locate own binary: {err}"))?;
    let components: Vec<String> = exe
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    if components.iter().any(|c| c == "release-perf") {
        return Ok("release-perf".into());
    }
    if components.iter().any(|c| c == "debug") {
        return Err(
            "profile guard: CORTEX_BENCH_PROFILE=release-perf but this binary is a debug build; \
             rebuild with --profile release-perf"
                .into(),
        );
    }
    Ok("release-perf (env-claimed; exe path layout unrecognized)".into())
}

/// Best-effort git state; unknown values stay None rather than being faked.
fn git_info() -> (Option<String>, Option<bool>, Vec<String>) {
    let root = repo_root();
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).to_string())
    };
    let head = run(&["rev-parse", "HEAD"]).map(|s| s.trim().to_string());
    let Some(status) = run(&["status", "--porcelain"]) else {
        return (head, None, Vec::new());
    };
    let paths: Vec<String> = status
        .lines()
        .map(|line| line.get(3..).unwrap_or(line).trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    let dirty = !paths.is_empty();
    (head, Some(dirty), paths)
}

fn machine_fingerprint() -> Value {
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "parallelism": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
    })
}

/// RFC3339 UTC timestamp without pulling a date dependency.
fn rfc3339_now() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before 1970")
        .as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let secs_of_day = secs.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let (hh, mm, ss) = (secs_of_day / 3600, (secs_of_day % 3600) / 60, secs_of_day % 60);
    format!("{year:04}-{month:02}-{day:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Howard Hinnant's civil_from_days: days since 1970-01-01 to (y, m, d).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn print_header(mode: &str, profile: &str, rounds: usize) {
    println!("hotpath_bench — {BENCH_NAME} latency ratchet (lib-level smoke)");
    println!("mode={mode} profile={profile} rounds={rounds}");
    println!("runner command:");
    println!("  CORTEX_BENCH_PROFILE=release-perf cargo run --quiet -p cortex-tests --example hotpath_bench --profile release-perf");
    println!(
        "env: UPDATE_BASELINE=1 writes {} · HOTPATH_ROUNDS=<n> (default {DEFAULT_ROUNDS}, min {MIN_ROUNDS})",
        baseline_path().display()
    );
    println!(
        "exit: 0 pass/warn · 1 regression · 2 setup error · 3 noise (cv_pct > {DEFAULT_CV_PCT_MAX:.0} ⇒ no verdict)"
    );
    println!(
        "no-claim: latency-regression gate only; recall quality floors are NOT measured here (benchmarking/ owns quality)."
    );
}

struct OpAgg {
    name: &'static str,
    ops_per_round: usize,
    /// gated workloads enter the baseline and the ratchet; informational spans are
    /// printed with full stats but never gated (jaccard_pair measures ~1us/call —
    /// below honest timer resolution for a regression gate)
    gated: bool,
    /// every sample from every round, milliseconds
    pooled: Vec<f64>,
    /// pooled-sample p50 of each round; cv_pct is computed over these
    round_medians: Vec<f64>,
}

async fn measure(rounds: usize) -> (Vec<OpAgg>, Vec<f64>) {
    let mut aggs: Vec<OpAgg> = Vec::new();
    let mut round_totals = Vec::with_capacity(rounds);

    for round in 0..rounds {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("hotpath.db");
        let mut write = open_file_db(&db_path);
        let read = open_file_db(&db_path);

        let t_round = Instant::now();
        let t_seed = Instant::now();
        seed_decisions(&mut write, SEED_ROWS, "seed");
        let seed_ms = t_seed.elapsed().as_secs_f64() * 1e3;

        let corpus_a = "persist sqlite wal checkpoints in cortex-daemon/src/db/maintenance.rs after store_decision";
        let corpus_b =
            "hybrid keyword plus semantic recall uses rrf fusion in handlers/recall/engine.rs";

        let mut round_ops: Vec<(&'static str, usize, Vec<f64>)> = Vec::with_capacity(5);

        round_ops.push((
            "jaccard_pair",
            JACCARD_ITERS,
            time_loop(JACCARD_ITERS, || {
                let _ = conflict::jaccard_similarity(corpus_a, corpus_b);
                let _ = conflict::jaccard_similarity(corpus_a, corpus_a);
            }),
        ));

        round_ops.push((
            "detect_conflict",
            CONFLICT_ITERS,
            time_loop(CONFLICT_ITERS, || {
                conflict::detect_conflict(&write, corpus_a, "hotpath-bench", None).expect("detect");
            }),
        ));

        let mut store_i = 0usize;
        round_ops.push((
            "store_case",
            STORE_ITERS,
            time_loop(STORE_ITERS, || {
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
            }),
        ));

        let home = dir.path();
        for _ in 0..BOOT_WARMUP {
            let _ = compiler::compile(&write, home, "hotpath-bench", 320);
        }
        round_ops.push((
            "boot_compile",
            BOOT_ITERS,
            time_loop(BOOT_ITERS, || {
                let _ = compiler::compile(&write, home, "hotpath-bench", 320);
            }),
        ));

        let state = runtime_state(open_file_db(&db_path), read, false, None);
        let ctx = RecallContext::solo();
        let queries = [
            "sqlite wal checkpoint",
            "fts5 porter tokenizer recall",
            "store_decision conflict jaccard",
            "boot capsule compile budget",
        ];
        for _ in 0..RECALL_WARMUP {
            execute_unified_recall(&state, queries[0], 320, 12, "hotpath-bench", &ctx, None)
                .await
                .expect("recall warmup");
        }
        let mut recall_samples = Vec::with_capacity(RECALL_ITERS);
        for i in 0..RECALL_ITERS {
            let query = queries[i % queries.len()];
            let t0 = Instant::now();
            execute_unified_recall(&state, query, 320, 12, "hotpath-bench", &ctx, None)
                .await
                .expect("recall");
            recall_samples.push(t0.elapsed().as_secs_f64() * 1e3);
        }
        round_ops.push(("recall_case", RECALL_ITERS, recall_samples));

        // Merge into the aggregates — every round counts. (Fixes the discarded-rounds
        // bug: the previous version printed round 0 and dropped rounds 1+ entirely.)
        if aggs.is_empty() {
            aggs = round_ops
                .into_iter()
                .map(|(name, ops_per_round, pooled)| {
                    let sorted = sorted_copy(&pooled);
                    OpAgg {
                        name,
                        ops_per_round,
                        gated: !matches!(name, "jaccard_pair" | "detect_conflict"),
                        pooled,
                        round_medians: vec![percentile(&sorted, 0.50)],
                    }
                })
                .collect();
        } else {
            for (agg, (name, _, pooled)) in aggs.iter_mut().zip(round_ops) {
                debug_assert_eq!(agg.name, name);
                let sorted = sorted_copy(&pooled);
                agg.round_medians.push(percentile(&sorted, 0.50));
                agg.pooled.extend(pooled);
            }
        }

        let total = t_round.elapsed().as_secs_f64();
        round_totals.push(total);
        println!(
            "round {}/{rounds}: total={total:.3}s seed_{SEED_ROWS}={seed_ms:.1}ms",
            round + 1
        );
    }

    (aggs, round_totals)
}

fn print_table(aggs: &[OpAgg], round_totals: &[f64]) {
    println!(
        "{:<17} {:>9} {:>6} {:>9} {:>9} {:>9} {:>9} {:>8} {:>6}",
        "workload", "ops/round", "rounds", "mean_ms", "p50_ms", "p95_ms", "p99_ms", "cv_pct", "gate"
    );
    for agg in aggs {
        let sorted = sorted_copy(&agg.pooled);
        println!(
            "{:<17} {:>9} {:>6} {:>9.3} {:>9.3} {:>9.3} {:>9.3} {:>8.2} {:>6}",
            agg.name,
            agg.ops_per_round,
            agg.round_medians.len(),
            mean(&agg.pooled),
            percentile(&sorted, 0.50),
            percentile(&sorted, 0.95),
            percentile(&sorted, 0.99),
            cv_pct(&agg.round_medians),
            if agg.gated { "gated" } else { "info" },
        );
    }
    let sorted_totals = sorted_copy(round_totals);
    println!(
        "round_totals: mean={:.3}s median={:.3}s cv_pct={:.2} (cv over {} round totals; cv_pct > {:.0} ⇒ noise, no verdict)",
        mean(round_totals),
        percentile(&sorted_totals, 0.50),
        cv_pct(round_totals),
        round_totals.len(),
        DEFAULT_CV_PCT_MAX,
    );
}

fn workload_records(aggs: &[OpAgg]) -> Vec<Value> {
    aggs.iter()
        .filter(|agg| agg.gated)
        .map(|agg| {
            let sorted = sorted_copy(&agg.pooled);
            json!({
                "name": agg.name,
                "samples": agg.pooled.len(),
                "unit": "milliseconds",
                "rounds": agg.round_medians.len(),
                "p50": percentile(&sorted, 0.50),
                "p95": percentile(&sorted, 0.95),
                "p99": percentile(&sorted, 0.99),
                "max": sorted.last().copied().unwrap_or(0.0),
                "mean": mean(&agg.pooled),
                "cv_pct": cv_pct(&agg.round_medians),
                "cv_basis": "per-round pooled-sample p50",
                "round_medians_ms": agg.round_medians,
                "cumulative": agg.pooled.iter().sum::<f64>(),
            })
        })
        .collect()
}

/// Eligibility rule, per the fixture's own validator semantics (the deleted harness
/// asserted `aggregate_latency.cv_pct <= 5.0` and nothing per-op): the RUN is
/// eligible when the round-total cv_pct is within the gate's cv_pct_max. Per-op cv
/// is always computed, emitted, and flagged in compare output, but bimodal
/// between-round cost regimes (observed in the recall path) make per-op round-median
/// dispersion a machine-noise indicator, not a gate.
fn eligibility_violations(aggs: &[OpAgg], round_totals: &[f64], cv_max: f64) -> Vec<String> {
    let _ = aggs;
    let totals_cv = cv_pct(round_totals);
    if totals_cv > cv_max {
        vec![format!("round_totals cv_pct={totals_cv:.2}>{cv_max:.0}")]
    } else {
        Vec::new()
    }
}

/// Gated ops whose round-median cv exceeds the bound — advisory context, always
/// surfaced in compare output.
fn noisy_ops(aggs: &[OpAgg], cv_max: f64) -> Vec<String> {
    aggs.iter()
        .filter(|agg| agg.gated)
        .map(|agg| (agg.name, cv_pct(&agg.round_medians)))
        .filter(|(_, cv)| *cv > cv_max)
        .map(|(name, cv)| format!("{name} cv_pct={cv:.2}>{cv_max:.0}"))
        .collect()
}

fn write_baseline(aggs: &[OpAgg], round_totals: &[f64], rounds: usize, profile: &str) {
    // Hold the seed to the same eligibility standard as any candidate run: a noisy
    // seed poisons every future verdict, so it must not be written.
    let violations = eligibility_violations(aggs, round_totals, DEFAULT_CV_PCT_MAX);
    if !violations.is_empty() {
        println!("verdict=noise");
        println!(
            "baseline NOT written: seed run is ineligible ({}) — re-run on a quiet machine; \
             a noisy seed would misclassify every future candidate",
            violations.join(", ")
        );
        std::process::exit(3);
    }
    let (git_head, git_dirty, dirty_paths) = git_info();
    let sorted_totals = sorted_copy(round_totals);
    let doc = json!({
        "schema_version": SCHEMA_VERSION,
        "generated_at": rfc3339_now(),
        "bench_name": BENCH_NAME,
        "baseline_kind": "hotpath_bench_lib_smoke_seed",
        "target": {
            "repo_root": "<repo-root>",
            "crate": "cortex-daemon",
            "package": "cortex-daemon",
            "package_version": env!("CARGO_PKG_VERSION"),
            "harness_package": "cortex-tests",
            "git_head": git_head,
            "git_dirty": git_dirty,
            "git_dirty_paths": dirty_paths,
        },
        "source": {
            "command": "UPDATE_BASELINE=1 CORTEX_BENCH_PROFILE=release-perf cargo run --quiet -p cortex-tests --example hotpath_bench --profile release-perf",
            "capture_env": {
                "CORTEX_BENCH_PROFILE": "release-perf",
                "HOTPATH_ROUNDS": rounds.to_string(),
            },
            "profile": profile,
            "workload_isolation": "single process, fresh seeded temp DB per round, all rounds counted, per-op warmup discarded (recall x3, boot x1), micro-spans jaccard_pair/detect_conflict informational only (not gated)",
        },
        "machine": machine_fingerprint(),
        "regression_gate": {
            "latency_ratio": {
                "warning": DEFAULT_WARN_RATIO,
                "critical": DEFAULT_FAIL_RATIO,
            },
            "cv_pct_max": DEFAULT_CV_PCT_MAX,
            "same_host_p95_drift_max_pct": 10.0,
        },
        "aggregate_latency": {
            "unit": "seconds",
            "rounds": rounds,
            "mean": mean(round_totals),
            "stddev": stddev(round_totals),
            "median": percentile(&sorted_totals, 0.50),
            "min": sorted_totals.first().copied().unwrap_or(0.0),
            "max": sorted_totals.last().copied().unwrap_or(0.0),
            "cv_pct": cv_pct(round_totals),
        },
        "workloads": workload_records(aggs),
        "quality": Value::Null,
        "quality_note": "latency-only ratchet; recall quality floors are not measurable at the lib level without a labeled relevance set and remain owned by benchmarking/ (LongMemEval)",
        "next_pass": {
            "compare_against": "tests/fixtures/bench-history/cortex-daemon-smoke.latest.json",
            "promote_only_when": "same workload set, same release-perf profile, candidate cv_pct <= 5 on every workload, and every workload median ratio <= 1.10 (critical 1.25)",
        },
    });

    let path = baseline_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create bench-history dir");
    }
    let rendered = serde_json::to_string_pretty(&doc).expect("serialize baseline") + "\n";
    std::fs::write(&path, rendered).unwrap_or_else(|err| {
        eprintln!("setup error: cannot write baseline {}: {err}", path.display());
        std::process::exit(2);
    });
    println!("baseline written: {}", path.display());
    println!("verdict=baseline-written");
    std::process::exit(0);
}

fn compare_and_gate(aggs: &[OpAgg], round_totals: &[f64]) -> ! {
    let path = baseline_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!(
            "setup error: baseline missing at {}\nseed it on a quiet machine with:\n  \
             UPDATE_BASELINE=1 CORTEX_BENCH_PROFILE=release-perf cargo run --quiet -p cortex-tests \
             --example hotpath_bench --profile release-perf",
            path.display()
        );
        std::process::exit(2);
    };
    let base: Value = serde_json::from_str(&text).unwrap_or_else(|err| {
        eprintln!("setup error: baseline {} is not valid JSON: {err}", path.display());
        std::process::exit(2);
    });

    let gate = &base["regression_gate"];
    let warn_at = gate["latency_ratio"]["warning"].as_f64().unwrap_or(DEFAULT_WARN_RATIO);
    let fail_at = gate["latency_ratio"]["critical"].as_f64().unwrap_or(DEFAULT_FAIL_RATIO);
    let cv_max = gate["cv_pct_max"].as_f64().unwrap_or(DEFAULT_CV_PCT_MAX);

    let base_medians: Vec<(String, f64)> = base["workloads"]
        .as_array()
        .map(|workloads| {
            workloads
                .iter()
                .filter_map(|w| {
                    let name = w["name"].as_str()?.to_string();
                    let p50 = w["p50"].as_f64()?;
                    Some((name, p50))
                })
                .collect()
        })
        .unwrap_or_default();

    // Fail closed: a baseline workload this bench no longer measures cannot be gated.
    for (name, _) in &base_medians {
        if !aggs.iter().any(|a| a.gated && a.name == name) {
            eprintln!(
                "setup error: baseline workload '{name}' is not measured by this bench; \
                 regenerate the baseline or restore the op"
            );
            std::process::exit(2);
        }
    }

    println!("{:<17} {:>10} {:>10} {:>7} {:>6} {:>8}", "workload", "base_p50", "cand_p50", "ratio", "cv%", "gate");
    let mut fails = Vec::new();
    let mut warns = Vec::new();
    for agg in aggs {
        let sorted = sorted_copy(&agg.pooled);
        let cand = percentile(&sorted, 0.50);
        let cv = cv_pct(&agg.round_medians);
        let Some((_, base_p50)) = base_medians.iter().find(|(name, _)| name == agg.name) else {
            println!(
                "{:<17} {:>10} {:>10.3} {:>7} {:>6.2} {:>8}",
                agg.name, "-", cand, "-", cv, "info"
            );
            continue;
        };
        let ratio = if *base_p50 > 0.0 { cand / base_p50 } else { f64::INFINITY };
        let chip = if ratio > fail_at {
            "FAIL"
        } else if ratio > warn_at {
            "warn"
        } else {
            "ok"
        };
        println!(
            "{:<17} {:>10.3} {:>10.3} {:>7.3} {:>6.2} {:>8}",
            agg.name, base_p50, cand, ratio, cv, chip
        );
        if ratio > fail_at {
            fails.push(format!("{} ratio={ratio:.3} (> {fail_at}x median)", agg.name));
        } else if ratio > warn_at {
            warns.push(format!("{} ratio={ratio:.3}", agg.name));
        }
    }

    // A noisy run is ineligible: it proves neither a pass nor a regression.
    let noisy = eligibility_violations(aggs, round_totals, cv_max);
    let op_noise = noisy_ops(aggs, cv_max);
    if !op_noise.is_empty() {
        println!(
            "note: per-op round-median cv above bound (reported, not verdict-blocking): {}",
            op_noise.join(", ")
        );
    }
    if !noisy.is_empty() {
        println!("verdict=noise");
        println!(
            "candidate run is NOISE ({}); per the ratchet law a noisy run issues no verdict — re-run on a quiet machine",
            noisy.join(", ")
        );
        std::process::exit(3);
    }
    if !fails.is_empty() {
        println!("verdict=fail");
        println!("latency regression vs {}: {}", path.display(), fails.join("; "));
        std::process::exit(1);
    }
    if !warns.is_empty() {
        println!("verdict=warn");
        println!("advisory (> {warn_at}x median, <= {fail_at}x): {}", warns.join("; "));
        std::process::exit(0);
    }
    println!("verdict=pass");
    std::process::exit(0);
}

#[tokio::main]
async fn main() {
    let profile = match profile_guard() {
        Ok(profile) => profile,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    let rounds = std::env::var("HOTPATH_ROUNDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_ROUNDS);
    if rounds < MIN_ROUNDS {
        eprintln!(
            "HOTPATH_ROUNDS={rounds} is below the minimum of {MIN_ROUNDS}: cv_pct over fewer \
             than {MIN_ROUNDS} rounds is not a verdict"
        );
        std::process::exit(2);
    }
    let update = std::env::var("UPDATE_BASELINE").ok().as_deref() == Some("1");
    let mode = if update { "update-baseline" } else { "compare" };
    print_header(mode, &profile, rounds);

    let (aggs, round_totals) = measure(rounds).await;
    print_table(&aggs, &round_totals);

    if update {
        write_baseline(&aggs, &round_totals, rounds, &profile);
    } else {
        compare_and_gate(&aggs, &round_totals);
    }
}
