//! perf_smoke — the bench-ratchet contract (bead cortex-38n / HYP-010).
//!
//! Runs the hotpath_bench example in compare mode against the committed baseline
//! `tests/fixtures/bench-history/cortex-daemon-smoke.latest.json` and asserts the
//! ratchet verdict: the run must be eligible (every workload cv_pct <= 5) and every
//! workload's median ratio must stay under the baseline's critical 1.25x (advisory
//! 1.10x warn is tolerated). The bench shells out to the same release-perf binary
//! the baseline was seeded from; on a fresh tree the first run pays the
//! `--profile release-perf` build of the daemon.
//!
//! A missing baseline FAILS with the seed command — it never skips silently.

use std::path::Path;
use std::process::Command;

#[test]
fn hotpath_bench_compare_matches_committed_baseline() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().expect("repo root");
    let baseline = manifest_dir
        .join("fixtures")
        .join("bench-history")
        .join("cortex-daemon-smoke.latest.json");
    assert!(
        baseline.is_file(),
        "bench baseline missing at {}\nseed it on a quiet machine with:\n  \
         UPDATE_BASELINE=1 CORTEX_BENCH_PROFILE=release-perf cargo run --quiet -p cortex-tests \
         --example hotpath_bench --profile release-perf",
        baseline.display()
    );

    let output = Command::new("cargo")
        .args([
            "run",
            "--quiet",
            "-p",
            "cortex-tests",
            "--example",
            "hotpath_bench",
            "--profile",
            "release-perf",
        ])
        .current_dir(repo_root)
        .env("CORTEX_BENCH_PROFILE", "release-perf")
        .env_remove("UPDATE_BASELINE")
        .env_remove("CARGO_MANIFEST_DIR")
        .output()
        .expect("spawn cargo run for hotpath_bench example");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    let verdict_line = stdout
        .lines()
        .find(|line| line.starts_with("verdict="))
        .unwrap_or_else(|| {
            panic!(
                "hotpath_bench printed no verdict= line (exit {:?})\nstdout:\n{stdout}\nstderr:\n{stderr}",
                output.status.code()
            )
        });

    assert!(
        verdict_line == "verdict=pass" || verdict_line == "verdict=warn",
        "bench ratchet verdict was '{verdict_line}' (exit {:?}) — fail = median ratio > 1.25x \
         baseline on some workload; noise = candidate cv_pct > 5 (re-run on a quiet machine)\n\
         stdout:\n{stdout}\nstderr:\n{stderr}",
        output.status.code()
    );
}
