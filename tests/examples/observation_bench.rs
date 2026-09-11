//! Targeted V5 Path-A measurement. No speedup, power-loss or reader-quality claim.
use cortex_kernel::runtime::{
    CortexRuntime,
    observation::{ObservationEvent, SourceSpec},
};
use cortex_tests::support::run_with_cx;
use serde_json::json;
use std::time::Instant;

fn quantiles(mut values: Vec<u128>) -> serde_json::Value {
    values.sort_unstable();
    json!({"p50_us":values[values.len()/2],"p95_us":values[values.len()*95/100],"p99_us":values[values.len()*99/100],"samples":values.len()})
}
fn main() {
    run_with_cx(|cx| async move {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("brain.db");
        let runtime = CortexRuntime::open_db(&path).unwrap();
        runtime
            .register_source(&cx, SourceSpec::document("fixture", "bench"))
            .await
            .unwrap();
        let mut captures = Vec::new();
        for i in 0..64 {
            let start = Instant::now();
            runtime
                .observe(
                    &cx,
                    "fixture",
                    "g",
                    ObservationEvent {
                        event_key: i.to_string(),
                        text: format!("document {i} keyword{} exact retained evidence", i % 8),
                        observed_at: None,
                    },
                )
                .await
                .unwrap();
            captures.push(start.elapsed().as_micros());
        }
        let mut indexed = Vec::new();
        let mut scan = Vec::new();
        for _ in 0..64 {
            let start = Instant::now();
            let view = runtime
                .query_observations(&cx, "bench", "keyword3", 128, 65536, false)
                .await
                .unwrap();
            indexed.push(start.elapsed().as_micros());
            assert_eq!(view.status, "ready");
            let start = Instant::now();
            let conn = runtime.state().db.lock(&cx).await.unwrap();
            let mut stmt=conn.prepare("SELECT source_id,inline_payload FROM sources WHERE availability='owned_inline' ORDER BY source_id").unwrap();
            let expected: std::collections::BTreeSet<String> = stmt
                .query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
                })
                .unwrap()
                .map(Result::unwrap)
                .filter_map(|(id, bytes)| {
                    let text = String::from_utf8(bytes).unwrap();
                    text.split(|c: char| !c.is_alphanumeric() && c != '_')
                        .any(|term| term == "keyword3")
                        .then_some(id)
                })
                .collect();
            scan.push(start.elapsed().as_micros());
            assert_eq!(
                view.source_refs
                    .into_iter()
                    .collect::<std::collections::BTreeSet<_>>(),
                expected
            );
        }
        let parallel_start = Instant::now();
        let mut children = Vec::new();
        for i in 0..8 {
            use std::io::Write;
            use std::process::{Command, Stdio};
            let mut child = Command::new(cortex_tests::cortex_bin())
                .args([
                    "capture",
                    "put",
                    "--source",
                    "fixture",
                    "--generation",
                    "concurrent",
                    "--home",
                ])
                .arg(home.path())
                .env("CORTEX_DB", &path)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let event = serde_json::to_vec(&ObservationEvent {
                event_key: format!("child-{i}"),
                text: "parallel evidence".into(),
                observed_at: None,
            })
            .unwrap();
            child.stdin.take().unwrap().write_all(&event).unwrap();
            children.push(child);
        }
        for child in children {
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(receipt["duplicate"], false);
        }
        let concurrent_us = parallel_start.elapsed().as_micros();
        {
            let conn = runtime.state().db.lock(&cx).await.unwrap();
            let accepted: i64 = conn
                .query_row(
                    "SELECT count(*) FROM observation_events WHERE generation='concurrent'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                accepted, 8,
                "all process acknowledgements must be present in the database"
            );
        }
        println!(
            "{}",
            json!({"concurrent_processes":8,"wall_us":concurrent_us,"acknowledged_and_retained":8})
        );
        drop(runtime);
        let mut cold = Vec::new();
        for _ in 0..8 {
            let start = Instant::now();
            let runtime = CortexRuntime::open_db(&path).unwrap();
            let view = runtime
                .query_observations(&cx, "bench", "keyword3", 128, 65536, false)
                .await
                .unwrap();
            cold.push(start.elapsed().as_micros());
            assert_eq!(view.source_refs.len(), 8);
        }
        let bytes: u64 = std::fs::read_dir(home.path())
            .unwrap()
            .map(Result::unwrap)
            .map(|e| e.metadata().unwrap().len())
            .sum();
        println!(
            "{}",
            json!({"workload":"64 retained documents, eight literal matches; no learned routing","profile":if cfg!(debug_assertions){"debug"}else{"release"},"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"logical_cpus":std::thread::available_parallelism().map(|n|n.get()).unwrap_or(0),"capture":quantiles(captures),"indexed_full_api":quantiles(indexed),"scan_fixture_only_no_policy_or_receipt_work":quantiles(scan),"cold_open_and_query":quantiles(cold),"database_and_sidecar_bytes":bytes,"acceptance":"measurement only; unmatched API overhead means no speedup claim","unmeasured":["power_loss","reader_quality","model_tokens","large_scale_contention"]})
        );
    });
}
