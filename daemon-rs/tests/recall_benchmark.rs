use reqwest::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TOKEN_TIMEOUT: Duration = Duration::from_secs(15);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const RECALL_BUDGET: &str = "300";
const MAX_QUERY_TOKENS: u64 = 300;
const MAX_AVG_QUERY_TOKENS: f64 = 300.0;
const MAX_P95_QUERY_TOKENS: u64 = 300;
const MIN_TOP1_HIT_RATE: f64 = 0.55;
const MIN_RECALL_COVERAGE: f64 = 0.90;
const MAX_TOKENS_PER_RELEVANT_HIT: f64 = 250.0;
const PROXY_TOP1_MIN_AGREEMENT: f64 = 0.65;
const PROXY_MAX_MEAN_ABS_RANK_ERROR: f64 = 0.90;
const PROXY_MIN_PAIRWISE_AGREEMENT: f64 = 0.75;
const PROXY_PAIRWISE_MIN_RELEVANCE_GAP: f64 = 0.01;
const PROXY_MIN_EVALUATED_QUERY_COVERAGE: f64 = 0.80;
const APP_REQUIRED_ENV: &str = "CORTEX_APP_REQUIRED";
const DAEMON_LOCAL_SPAWN_ENV: &str = "CORTEX_DAEMON_OWNER_LOCAL_SPAWN";
const APP_CLIENT_ENV: &str = "CORTEX_APP_CLIENT";
const RECALL_PROFILE_ENV: &str = "CORTEX_RECALL_BENCHMARK_PROFILE";

struct BenchmarkCase {
    slug: &'static str,
    query: &'static str,
    ground_truth: &'static [&'static str],
}

const BENCHMARK_CASES: &[BenchmarkCase] = &[
    BenchmarkCase {
        slug: "token-optimization",
        query: "token optimization settings",
        ground_truth: &[
            "token optimization",
            "ENABLE_TOOL_SEARCH",
            "thinking_tokens",
            "output_tokens",
            "compression",
        ],
    },
    BenchmarkCase {
        slug: "cache-expiry-guard",
        query: "cache expiry guard hook",
        ground_truth: &[
            "cache expiry",
            "cache-expiry-guard",
            "UserPromptSubmit",
            "idle",
            "TTL",
        ],
    },
    BenchmarkCase {
        slug: "rtk-path-fix",
        query: "RTK path fix bashrc",
        ground_truth: &["rtk", "bashrc", "PATH", "rtk-real", "local/bin"],
    },
    BenchmarkCase {
        slug: "ccmeter-dashboard",
        query: "CCMeter analytics dashboard",
        ground_truth: &["ccmeter", "analytics", "dashboard", "heatmap", "session"],
    },
    BenchmarkCase {
        slug: "browser-cleanup",
        query: "browser cleanup playwright",
        ground_truth: &["browser", "playwright", "chrome", "dev-browser", "cleanup"],
    },
    BenchmarkCase {
        slug: "uv-python",
        query: "uv python package management",
        ground_truth: &["uv", "python", "pip", "package", "pytest"],
    },
    BenchmarkCase {
        slug: "no-em-dashes",
        query: "never use em-dashes",
        ground_truth: &["em-dash", "em dash", "double hyphen", "--", "emdash"],
    },
    BenchmarkCase {
        slug: "recall-before-investigation",
        query: "cortex recall before investigation",
        ground_truth: &[
            "cortex_recall",
            "recall",
            "before",
            "investigation",
            "debug",
        ],
    },
    BenchmarkCase {
        slug: "codex-agent",
        query: "codex agent contributions",
        ground_truth: &["codex", "agent", "scout", "batch", "build"],
    },
    BenchmarkCase {
        slug: "gemini-agent",
        query: "gemini agent decisions",
        ground_truth: &["gemini", "agent", "decision", "model"],
    },
    BenchmarkCase {
        slug: "factory-droid",
        query: "factory droid builds",
        ground_truth: &["factory", "droid", "build", "automation"],
    },
    BenchmarkCase {
        slug: "shared-state",
        query: "multi-agent shared state",
        ground_truth: &["multi-agent", "shared", "conductor", "session", "team"],
    },
    BenchmarkCase {
        slug: "boot-compiler",
        query: "boot compiler capsule system",
        ground_truth: &["boot", "capsule", "compiler", "identity", "delta"],
    },
    BenchmarkCase {
        slug: "conflict-detection",
        query: "conflict detection jaccard cosine",
        ground_truth: &["conflict", "jaccard", "cosine", "similarity", "supersed"],
    },
    BenchmarkCase {
        slug: "embedding-engine",
        query: "embedding engine MiniLM",
        ground_truth: &["embedding", "MiniLM", "ONNX", "vector", "cosine"],
    },
    BenchmarkCase {
        slug: "crystal-cluster",
        query: "crystal cluster formation",
        ground_truth: &["crystal", "cluster", "leiden", "community", "pattern"],
    },
    BenchmarkCase {
        slug: "writing-voice",
        query: "user writing voice style",
        ground_truth: &["writing", "voice", "style", "confident", "earnest"],
    },
    BenchmarkCase {
        slug: "self-improvement",
        query: "self improvement engine goals",
        ground_truth: &[
            "self-improvement",
            "improvement",
            "compound",
            "autoresearch",
            "lesson",
        ],
    },
    BenchmarkCase {
        slug: "tauri-dashboard",
        query: "tauri dashboard control center",
        ground_truth: &["tauri", "dashboard", "control", "desktop", "metrics"],
    },
    BenchmarkCase {
        slug: "job-applicator",
        query: "job applicator skill",
        ground_truth: &["job", "applicat", "indeed", "skill", "tracker"],
    },
];

struct TestDaemon {
    child: Child,
    home: PathBuf,
    base_url: String,
    token: String,
    client: Client,
}

enum SpawnError {
    Skip(String),
    Fatal(String),
}

#[derive(Clone, Debug)]
struct ProxyRankPoint {
    full_rank: usize,
    full_relevance: f64,
    proxy_score: f64,
}

impl TestDaemon {
    async fn spawn() -> Result<Self, SpawnError> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(REQUEST_TIMEOUT)
            .build()
            .unwrap();

        let mut last_error = String::new();
        for attempt in 1..=6u32 {
            let home = unique_temp_dir(&format!("recall_benchmark_{attempt}"));
            fs::create_dir_all(home.join("models")).unwrap();
            write_dummy_model_files(&home);

            let port = reserve_port();
            let base_url = format!("http://127.0.0.1:{port}");
            let mut child = Command::new(env!("CARGO_BIN_EXE_cortex"))
                .arg("serve")
                .arg("--home")
                .arg(&home)
                .arg("--port")
                .arg(port.to_string())
                .arg("--bind")
                .arg("127.0.0.1")
                // Keep benchmark spawning deterministic even when the parent shell
                // is running attach-only app client env contracts.
                .env_remove(APP_REQUIRED_ENV)
                .env_remove(DAEMON_LOCAL_SPAWN_ENV)
                .env_remove(APP_CLIENT_ENV)
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .expect("failed to spawn cortex daemon");

            match wait_for_health(&client, &base_url, &mut child).await {
                Ok(()) => {
                    let token = wait_for_token(&home, &mut child)
                        .await
                        .expect("token file was not written");
                    return Ok(Self {
                        child,
                        home,
                        base_url,
                        token,
                        client,
                    });
                }
                Err(err) => {
                    if should_skip_for_active_singleton(&err) {
                        let _ = child.kill();
                        let _ = child.wait();
                        let _ = fs::remove_dir_all(&home);
                        return Err(SpawnError::Skip(format!(
                            "recall benchmark skipped because another Cortex daemon is already active: {err}"
                        )));
                    }
                    last_error = err;
                    let _ = child.kill();
                    let _ = child.wait();
                    let _ = fs::remove_dir_all(&home);
                    if attempt < 6 {
                        tokio::time::sleep(Duration::from_millis(200 * attempt as u64)).await;
                    }
                }
            }
        }

        Err(SpawnError::Fatal(last_error))
    }

    fn request(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .header("Authorization", format!("Bearer {}", self.token))
            .header("X-Cortex-Request", "true")
            .header("X-Source-Agent", "benchmark-test")
    }

    async fn store_case(&self, case: &BenchmarkCase) {
        let decision = build_case_decision(case);
        let response = tokio::time::timeout(
            REQUEST_TIMEOUT,
            self.request(
                self.client
                    .post(format!("{}/store", self.base_url))
                    .json(&json!({
                        "decision": decision,
                        "context": format!("benchmark::{}", case.slug),
                    })),
            )
            .send(),
        )
        .await
        .unwrap_or_else(|_| panic!("POST /store timed out for {}", case.slug))
        .expect("failed to POST /store");
        assert!(
            response.status().is_success(),
            "store failed for {}: {}",
            case.slug,
            response.status()
        );
    }

    async fn recall_case(&self, case: &BenchmarkCase) -> Value {
        let response = tokio::time::timeout(
            REQUEST_TIMEOUT,
            self.request(
                self.client
                    .get(format!("{}/recall", self.base_url))
                    .query(&[("q", case.query), ("budget", RECALL_BUDGET), ("k", "2")]),
            )
            .send(),
        )
        .await
        .unwrap_or_else(|_| panic!("GET /recall timed out for {}", case.slug))
        .expect("failed to GET /recall");
        assert!(
            response.status().is_success(),
            "recall failed for {}: {}",
            case.slug,
            response.status()
        );
        response.json::<Value>().await.expect("invalid recall JSON")
    }

    async fn recall_explain_case(&self, case: &BenchmarkCase) -> Value {
        let response = tokio::time::timeout(
            REQUEST_TIMEOUT,
            self.request(
                self.client
                    .get(format!("{}/recall/explain", self.base_url))
                    .query(&[
                        ("q", case.query),
                        ("budget", RECALL_BUDGET),
                        ("k", "8"),
                        ("pool_k", "32"),
                    ]),
            )
            .send(),
        )
        .await
        .unwrap_or_else(|_| panic!("GET /recall/explain timed out for {}", case.slug))
        .expect("failed to GET /recall/explain");
        assert!(
            response.status().is_success(),
            "recall explain failed for {}: {}",
            case.slug,
            response.status()
        );
        response
            .json::<Value>()
            .await
            .expect("invalid recall explain JSON")
    }
}

impl Drop for TestDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.home);
    }
}

struct RecallBenchmarkProfile {
    enabled: bool,
    scenario: &'static str,
    run_id: String,
    out_dir: PathBuf,
    samples: Vec<RecallProfileSample>,
}

struct RecallProfileSample {
    span_name: &'static str,
    case_slug: Option<&'static str>,
    duration: Duration,
}

struct RecallSpanSummary {
    span_name: String,
    count: usize,
    cumulative_us: u64,
    p50_us: u64,
    p95_us: u64,
    p99_us: u64,
    max_us: u64,
    mean_us: f64,
}

struct RecallBenchmarkQuality {
    avg_precision: f64,
    avg_mrr: f64,
    avg_tokens: f64,
    top1_hit_rate: f64,
    recall_coverage: f64,
    p95_query_tokens: u64,
    tokens_per_relevant_hit: f64,
}

impl RecallBenchmarkProfile {
    fn new(scenario: &'static str) -> Self {
        let enabled = std::env::var(RECALL_PROFILE_ENV).ok().is_some_and(|value| {
            !matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "" | "0" | "false" | "no"
            )
        });
        let run_id = if enabled {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            format!("{}_{:06}", now.as_secs(), now.subsec_micros())
        } else {
            String::new()
        };
        let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("artifacts")
            .join("perf")
            .join(&run_id);

        Self {
            enabled,
            scenario,
            run_id,
            out_dir,
            samples: Vec::new(),
        }
    }

    fn record(
        &mut self,
        span_name: &'static str,
        case_slug: Option<&'static str>,
        duration: Duration,
    ) {
        if self.enabled {
            self.samples.push(RecallProfileSample {
                span_name,
                case_slug,
                duration,
            });
        }
    }

    fn write_artifacts(
        &self,
        quality: &RecallBenchmarkQuality,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if !self.enabled {
            return Ok(());
        }

        fs::create_dir_all(&self.out_dir)?;
        let summaries = self.span_summaries();
        let quality_json = quality.as_json();
        let quality_pretty = serde_json::to_string_pretty(&quality_json)?;
        let mut hasher = Sha256::new();
        hasher.update(quality_pretty.as_bytes());
        let quality_sha256 = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();

        fs::write(
            self.out_dir.join("DEFINE.md"),
            self.render_define(&summaries),
        )?;
        fs::write(
            self.out_dir.join("fingerprint.json"),
            serde_json::to_string_pretty(&self.fingerprint_json())?,
        )?;
        fs::write(
            self.out_dir.join("recall_benchmark_spans.json"),
            serde_json::to_string_pretty(&self.samples_json())?,
        )?;
        fs::write(
            self.out_dir.join("span_summary.json"),
            serde_json::to_string_pretty(&span_summary_json(&summaries))?,
        )?;
        fs::write(self.out_dir.join("quality_summary.json"), quality_pretty)?;
        fs::write(
            self.out_dir.join("BASELINE.md"),
            self.render_baseline(&summaries),
        )?;
        fs::write(
            self.out_dir.join("hotspot_table.md"),
            self.render_hotspot_table(&summaries),
        )?;
        fs::write(
            self.out_dir.join("scaling_law.md"),
            self.render_scaling_law(),
        )?;
        fs::write(
            self.out_dir.join("hypothesis.md"),
            self.render_hypothesis(&summaries),
        )?;
        fs::write(
            self.out_dir.join("golden_checksums.txt"),
            format!("{quality_sha256}  quality_summary.json\n"),
        )?;
        eprintln!(
            "recall benchmark profile artifacts: {}",
            self.out_dir.display()
        );
        Ok(())
    }

    fn span_summaries(&self) -> Vec<RecallSpanSummary> {
        let mut grouped: HashMap<String, Vec<u64>> = HashMap::new();
        for sample in &self.samples {
            grouped
                .entry(sample.span_name.to_string())
                .or_default()
                .push(duration_us(sample.duration));
        }

        let mut summaries = grouped
            .into_iter()
            .map(|(span_name, mut values)| {
                values.sort_unstable();
                let cumulative_us = values.iter().sum();
                let count = values.len();
                RecallSpanSummary {
                    span_name,
                    count,
                    cumulative_us,
                    p50_us: percentile_sorted(&values, 0.50),
                    p95_us: percentile_sorted(&values, 0.95),
                    p99_us: percentile_sorted(&values, 0.99),
                    max_us: values.last().copied().unwrap_or(0),
                    mean_us: cumulative_us as f64 / count.max(1) as f64,
                }
            })
            .collect::<Vec<_>>();
        summaries.sort_by(|left, right| {
            right
                .cumulative_us
                .cmp(&left.cumulative_us)
                .then_with(|| left.span_name.cmp(&right.span_name))
        });
        summaries
    }

    fn render_define(&self, summaries: &[RecallSpanSummary]) -> String {
        let recall_p95_us = summaries
            .iter()
            .find(|summary| summary.span_name == "recall_case")
            .map(|summary| summary.p95_us)
            .unwrap_or(0);
        let advisory_budget_us = (recall_p95_us / 2).max(1);

        format!(
            "# DEFINE - {scenario}\n\n\
             ## Scenario\n\
             Spawn a local Cortex daemon with a temporary home, store {case_count} benchmark fixtures, then issue {case_count} recall queries with budget {budget_tokens}.\n\n\
             ## Metric\n\
             Wall-clock latency in microseconds for daemon startup, store calls, and recall calls; p50/p95/p99 are reported per span.\n\n\
             ## Budget\n\
             Advisory first-pass budget: recall_case p95 <= {advisory_budget_us} us on this fingerprint. This is 50% of the first measured p95 because no prior latency budget exists.\n\n\
             ## Golden output\n\
             golden_checksums.txt contains the SHA-256 of quality_summary.json.\n\n\
             ## Scope boundary\n\
             In scope: test-harness wall-clock attribution for recall benchmark stages and any sidecar external CPU sampler artifacts captured for this run. Out of scope: allocation profiles, kernel tuning, and production daemon workloads.\n",
            scenario = self.scenario,
            case_count = BENCHMARK_CASES.len(),
            budget_tokens = RECALL_BUDGET,
        )
    }

    fn fingerprint_json(&self) -> Value {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        json!({
            "run_id": self.run_id,
            "captured_at_unix_ms": current_unix_ms(),
            "git_sha": command_output("git", &["rev-parse", "HEAD"], Some(manifest_dir)),
            "git_dirty": command_output(
                "git",
                &["status", "--short", "--untracked-files=no"],
                Some(manifest_dir)
            )
            .is_some(),
            "hardware": {
                "processor_identifier": std::env::var("PROCESSOR_IDENTIFIER").ok(),
                "logical_processors": std::env::var("NUMBER_OF_PROCESSORS").ok(),
            },
            "os": {
                "family": std::env::consts::FAMILY,
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
            },
            "toolchain": {
                "rustc": command_output("rustc", &["--version", "--verbose"], Some(manifest_dir)),
                "cargo": command_output("cargo", &["--version"], Some(manifest_dir)),
            },
            "build_profile": build_profile_json(),
            "workload_isolation": "not isolated",
            "cache_state": "warm",
            "external_samplers": {
                "hyperfine": "not required for this in-test fallback",
                "samply": "not required for this in-test fallback"
            }
        })
    }

    fn samples_json(&self) -> Value {
        Value::Array(
            self.samples
                .iter()
                .map(|sample| {
                    json!({
                        "run_id": self.run_id,
                        "span_name": sample.span_name,
                        "case_slug": sample.case_slug,
                        "duration_us": duration_us(sample.duration),
                    })
                })
                .collect(),
        )
    }

    fn render_baseline(&self, summaries: &[RecallSpanSummary]) -> String {
        let mut out = format!(
            "# Baseline - {scenario} - run {run_id}\n\n\
             | Span | Samples | p50 us | p95 us | p99 us | Max us | Mean us | Cumulative us |\n\
             |------|--------:|-------:|-------:|-------:|-------:|--------:|--------------:|\n",
            scenario = self.scenario,
            run_id = self.run_id,
        );
        for summary in summaries {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {:.1} | {} |\n",
                summary.span_name,
                summary.count,
                summary.p50_us,
                summary.p95_us,
                summary.p99_us,
                summary.max_us,
                summary.mean_us,
                summary.cumulative_us,
            ));
        }
        out.push_str(
            "\nNote: sample counts are below 1000, so p99 and higher tails are conservative worst-observed sentinels, not tight percentile estimates.\n",
        );
        out
    }

    fn render_hotspot_table(&self, summaries: &[RecallSpanSummary]) -> String {
        let mut out = format!("# Hotspot table - {}\n\n", self.run_id);
        out.push_str(
            "| Rank | Location | Metric | Value | Category | Evidence |\n\
             |-----:|----------|--------|------:|----------|----------|\n",
        );
        for (idx, summary) in summaries.iter().enumerate() {
            out.push_str(&format!(
                "| {} | {} | cumulative wall time | {} us | wall | span_summary.json; recall_benchmark_spans.json |\n",
                idx + 1,
                summary.span_name,
                summary.cumulative_us,
            ));
        }
        out
    }

    fn render_scaling_law(&self) -> String {
        format!(
            "# Scaling law - {}\n\n\
             Not applicable for this first pass. The scenario uses a fixed fixture count ({}) and has no varied scale axis in this test run.\n",
            self.run_id,
            BENCHMARK_CASES.len(),
        )
    }

    fn render_hypothesis(&self, summaries: &[RecallSpanSummary]) -> String {
        let top = summaries.first();
        let recall = summaries
            .iter()
            .find(|summary| summary.span_name == "recall_case");
        let store = summaries
            .iter()
            .find(|summary| summary.span_name == "store_case");
        let verdict = match (recall, store) {
            (Some(recall), Some(store)) if recall.cumulative_us > store.cumulative_us => "supports",
            (Some(_), Some(_)) => "rejects",
            _ => "unknown",
        };
        let evidence = match (recall, store) {
            (Some(recall), Some(store)) => format!(
                "recall_case cumulative {} us vs store_case cumulative {} us",
                recall.cumulative_us, store.cumulative_us
            ),
            _ => "missing recall or store span summary".to_string(),
        };

        format!(
            "# Hypothesis ledger - {run_id}\n\n\
             | Hypothesis | Verdict | Evidence |\n\
             |------------|---------|----------|\n\
             | The top-ranked benchmark stage owns the current wall-clock cost | {top_verdict} | {top_evidence} |\n\
             | Recall query execution dominates fixture ingestion | {verdict} | {evidence}; span_summary.json |\n",
            run_id = self.run_id,
            top_verdict = if top.is_some() { "supports" } else { "unknown" },
            top_evidence = top
                .map(|summary| format!(
                    "{} ranks first at {} us cumulative",
                    summary.span_name, summary.cumulative_us
                ))
                .unwrap_or_else(|| "no span summaries captured".to_string()),
        )
    }
}

fn build_profile_json() -> Value {
    let inferred_profile = std::env::current_exe()
        .ok()
        .and_then(|path| cargo_profile_name_from_path(&path))
        .unwrap_or_else(|| {
            if cfg!(debug_assertions) {
                "debug".to_string()
            } else {
                "optimized-unknown".to_string()
            }
        });

    json!({
        "name": inferred_profile,
        "debug_assertions": cfg!(debug_assertions),
        "panic_strategy": if cfg!(panic = "abort") { "abort" } else { "unwind" },
        "known_settings": known_cargo_profile_settings(&inferred_profile),
        "compile_rustflags": compile_rustflags(),
        "runtime_rustflags": std::env::var("RUSTFLAGS").ok(),
        "recommended_profile": "release-perf",
        "recommended_rustflags": "-C force-frame-pointers=yes",
    })
}

fn cargo_profile_name_from_path(path: &Path) -> Option<String> {
    let parts = path
        .components()
        .filter_map(|component| component.as_os_str().to_str())
        .collect::<Vec<_>>();
    let target_idx = parts.iter().position(|part| *part == "target")?;
    let after_target = &parts[target_idx + 1..];
    for part in after_target.iter().take(3) {
        if is_known_cargo_profile_name(part) {
            return Some((*part).to_string());
        }
    }
    after_target.first().map(|part| (*part).to_string())
}

fn is_known_cargo_profile_name(name: &str) -> bool {
    matches!(name, "debug" | "release" | "release-perf")
}

fn known_cargo_profile_settings(profile: &str) -> Value {
    match profile {
        "release-perf" => json!({
            "inherits": "release",
            "opt_level": 3,
            "lto": "thin",
            "codegen_units": 1,
            "debug": "line-tables-only",
            "strip": false,
            "configured_panic": "abort"
        }),
        "release" => json!({
            "opt_level": 3,
            "lto": true,
            "codegen_units": 1,
            "strip": true,
            "configured_panic": "abort"
        }),
        "debug" => json!({
            "opt_level": 0,
            "debug": true,
            "strip": false,
            "configured_panic": "unwind"
        }),
        _ => Value::Null,
    }
}

fn compile_rustflags() -> Option<String> {
    option_env!("CARGO_ENCODED_RUSTFLAGS")
        .map(|flags| flags.replace('\u{1f}', " "))
        .or_else(|| option_env!("RUSTFLAGS").map(str::to_string))
}

impl RecallBenchmarkQuality {
    fn as_json(&self) -> Value {
        json!({
            "avg_precision": self.avg_precision,
            "avg_mrr": self.avg_mrr,
            "avg_tokens": self.avg_tokens,
            "top1_hit_rate": self.top1_hit_rate,
            "recall_coverage": self.recall_coverage,
            "p95_query_tokens": self.p95_query_tokens,
            "tokens_per_relevant_hit": self.tokens_per_relevant_hit,
        })
    }
}

fn span_summary_json(summaries: &[RecallSpanSummary]) -> Value {
    Value::Array(
        summaries
            .iter()
            .map(|summary| {
                json!({
                    "span_name": summary.span_name,
                    "count": summary.count,
                    "cumulative_us": summary.cumulative_us,
                    "p50_us": summary.p50_us,
                    "p95_us": summary.p95_us,
                    "p99_us": summary.p99_us,
                    "max_us": summary.max_us,
                    "mean_us": summary.mean_us,
                    "category": "wall",
                    "evidence": "recall_benchmark_spans.json",
                })
            })
            .collect(),
    )
}

fn duration_us(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}

fn percentile_sorted(values: &[u64], quantile: f64) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let idx = ((values.len() - 1) as f64 * quantile).ceil() as usize;
    values[idx.min(values.len() - 1)]
}

fn current_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn command_output(program: &str, args: &[&str], cwd: Option<&Path>) -> Option<String> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_benchmark_regression_thresholds_hold() {
    let mut profile = RecallBenchmarkProfile::new("recall_benchmark_regression_thresholds_hold");
    let spawn_started = Instant::now();
    let daemon = match TestDaemon::spawn().await {
        Ok(daemon) => daemon,
        Err(SpawnError::Skip(reason)) => {
            if should_fail_on_singleton_skip() {
                panic!("recall benchmark skipped under CI singleton policy: {reason}");
            }
            eprintln!("{reason}");
            return;
        }
        Err(SpawnError::Fatal(err)) => {
            panic!("failed to spawn healthy benchmark daemon after retries: {err}")
        }
    };
    profile.record("daemon.spawn_ready", None, spawn_started.elapsed());

    for case in BENCHMARK_CASES {
        let started = Instant::now();
        daemon.store_case(case).await;
        profile.record("store_case", Some(case.slug), started.elapsed());
    }

    let mut precision_sum = 0.0;
    let mut mrr_sum = 0.0;
    let mut token_sum = 0u64;
    let mut query_count = 0u64;
    let mut top1_hits = 0u64;
    let mut recall_coverage_hits = 0u64;
    let mut relevant_hits_total = 0u64;
    let mut query_token_samples = Vec::with_capacity(BENCHMARK_CASES.len());

    for case in BENCHMARK_CASES {
        let started = Instant::now();
        let payload = daemon.recall_case(case).await;
        profile.record("recall_case", Some(case.slug), started.elapsed());
        let results = payload["results"].as_array().cloned().unwrap_or_default();
        let relevant = results
            .iter()
            .filter(|result| matches_ground_truth(case, result))
            .count();
        let precision = if results.is_empty() {
            0.0
        } else {
            relevant as f64 / results.len() as f64
        };
        let mrr = results
            .iter()
            .position(|result| matches_ground_truth(case, result))
            .map(|idx| 1.0 / (idx as f64 + 1.0))
            .unwrap_or(0.0);
        let query_tokens: u64 = results
            .iter()
            .map(|result| result["tokens"].as_u64().unwrap_or(0))
            .sum();
        if results
            .first()
            .is_some_and(|result| matches_ground_truth(case, result))
        {
            top1_hits += 1;
        }
        if relevant > 0 {
            recall_coverage_hits += 1;
        }
        relevant_hits_total += relevant as u64;
        query_token_samples.push(query_tokens);
        assert!(
            query_tokens <= MAX_QUERY_TOKENS,
            "benchmark token regression for {}: got {}, need <= {}",
            case.slug,
            query_tokens,
            MAX_QUERY_TOKENS
        );

        precision_sum += precision;
        mrr_sum += mrr;
        token_sum += query_tokens;
        query_count += 1;
    }

    let query_count_f64 = BENCHMARK_CASES.len() as f64;
    let avg_precision = precision_sum / query_count_f64;
    let avg_mrr = mrr_sum / query_count_f64;
    let avg_tokens = token_sum as f64 / query_count as f64;
    let top1_hit_rate = top1_hits as f64 / query_count as f64;
    let recall_coverage = recall_coverage_hits as f64 / query_count as f64;
    let p95_query_tokens = percentile_95(&query_token_samples);
    let tokens_per_relevant_hit = if relevant_hits_total == 0 {
        f64::INFINITY
    } else {
        token_sum as f64 / relevant_hits_total as f64
    };
    let quality = RecallBenchmarkQuality {
        avg_precision,
        avg_mrr,
        avg_tokens,
        top1_hit_rate,
        recall_coverage,
        p95_query_tokens,
        tokens_per_relevant_hit,
    };

    assert!(
        avg_precision >= 0.50,
        "benchmark precision regression: got {:.3}, need >= 0.50",
        avg_precision
    );
    assert!(
        avg_mrr >= 0.70,
        "benchmark MRR regression: got {:.3}, need >= 0.70",
        avg_mrr
    );
    assert!(
        avg_tokens <= MAX_AVG_QUERY_TOKENS,
        "benchmark avg token regression: got {:.2}, need <= {:.2}",
        avg_tokens,
        MAX_AVG_QUERY_TOKENS
    );
    assert!(
        top1_hit_rate >= MIN_TOP1_HIT_RATE,
        "benchmark top-1 hit rate regression: got {:.3}, need >= {:.2}",
        top1_hit_rate,
        MIN_TOP1_HIT_RATE
    );
    assert!(
        recall_coverage >= MIN_RECALL_COVERAGE,
        "benchmark recall coverage regression: got {:.3}, need >= {:.2}",
        recall_coverage,
        MIN_RECALL_COVERAGE
    );
    assert!(
        p95_query_tokens <= MAX_P95_QUERY_TOKENS,
        "benchmark p95 token regression: got {}, need <= {}",
        p95_query_tokens,
        MAX_P95_QUERY_TOKENS
    );
    assert!(
        tokens_per_relevant_hit <= MAX_TOKENS_PER_RELEVANT_HIT,
        "benchmark tokens-per-relevant-hit regression: got {:.2}, need <= {:.2}",
        tokens_per_relevant_hit,
        MAX_TOKENS_PER_RELEVANT_HIT
    );
    profile
        .write_artifacts(&quality)
        .expect("failed to write recall benchmark profile artifacts");
}

#[test]
fn cortex_daemon_smoke_bench_history_baseline_is_valid() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let baseline_path = manifest_dir
        .join("tests")
        .join("fixtures")
        .join("bench-history")
        .join("cortex-daemon-smoke.latest.json");
    let baseline_text = fs::read_to_string(&baseline_path).unwrap_or_else(|err| {
        panic!(
            "missing bench-history baseline {}: {err}",
            baseline_path.display()
        )
    });
    let baseline: Value =
        serde_json::from_str(&baseline_text).expect("bench-history baseline should be valid JSON");

    assert_eq!(
        baseline["schema_version"],
        json!("cortex-daemon-smoke.bench-history.v1")
    );
    assert_eq!(baseline["bench_name"], json!("cortex-daemon-smoke"));
    assert_eq!(
        baseline["source"]["workload_isolation"],
        json!("single test process, --test-threads=1, temporary daemon home per run")
    );
    assert!(
        baseline["source"]["command"]
            .as_str()
            .is_some_and(|command| command.contains("recall_benchmark_regression_thresholds_hold")),
        "baseline should pin the exact smoke benchmark command"
    );

    let latency_gate = &baseline["regression_gate"]["latency_ratio"];
    assert_eq!(latency_gate["warning"].as_f64(), Some(1.1));
    assert_eq!(latency_gate["critical"].as_f64(), Some(1.25));
    assert_eq!(
        baseline["regression_gate"]["throughput_drop_pct"]["warning"].as_f64(),
        Some(10.0)
    );
    assert_eq!(
        baseline["regression_gate"]["throughput_drop_pct"]["critical"].as_f64(),
        Some(20.0)
    );

    let workloads = baseline["workloads"]
        .as_array()
        .expect("baseline workloads should be an array");
    for expected in ["daemon.spawn_ready", "store_case", "recall_case"] {
        assert!(
            workloads
                .iter()
                .any(|workload| workload["name"] == expected),
            "baseline missing workload {expected}"
        );
    }

    assert!(
        baseline["aggregate_latency"]["cv_pct"]
            .as_f64()
            .unwrap_or(f64::INFINITY)
            <= 5.0,
        "initial baseline variance should stay inside the Phase 5 smoke envelope"
    );
    assert!(
        baseline["quality"]["recall_coverage"]
            .as_f64()
            .unwrap_or_default()
            >= 0.9,
        "baseline should preserve the recall coverage floor"
    );

    let artifacts = baseline["evidence"]["artifacts"]
        .as_array()
        .expect("baseline evidence artifacts should be an array");
    assert!(
        artifacts.len() >= 3,
        "baseline should link hyperfine, quality, and span evidence"
    );
    for artifact in artifacts {
        let path = artifact["path"].as_str().expect("evidence path string");
        let hash = artifact["sha256"].as_str().expect("evidence sha256 string");
        assert!(!path.trim().is_empty(), "evidence path should not be empty");
        assert_eq!(
            hash.len(),
            64,
            "evidence hash should be a SHA-256 hex digest"
        );
        assert!(
            hash.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "evidence hash should be hex"
        );
    }
}

#[test]
fn perf_fingerprint_infers_cargo_profile_from_test_binary_path() {
    let release_perf = ["target", "release-perf", "deps", "recall_benchmark.exe"]
        .iter()
        .collect::<PathBuf>();
    assert_eq!(
        cargo_profile_name_from_path(&release_perf).as_deref(),
        Some("release-perf")
    );

    let target_triple = [
        "target",
        "x86_64-pc-windows-msvc",
        "release-perf",
        "deps",
        "recall_benchmark.exe",
    ]
    .iter()
    .collect::<PathBuf>();
    assert_eq!(
        cargo_profile_name_from_path(&target_triple).as_deref(),
        Some("release-perf")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_different_queries_do_not_cross_dedup_for_same_agent() {
    let daemon = match TestDaemon::spawn().await {
        Ok(daemon) => daemon,
        Err(SpawnError::Skip(reason)) => {
            if should_fail_on_singleton_skip() {
                panic!("recall benchmark skipped under CI singleton policy: {reason}");
            }
            eprintln!("{reason}");
            return;
        }
        Err(SpawnError::Fatal(err)) => {
            panic!("failed to spawn healthy benchmark daemon after retries: {err}")
        }
    };

    for case in BENCHMARK_CASES {
        daemon.store_case(case).await;
    }

    for slug in ["gemini-agent", "shared-state", "tauri-dashboard"] {
        let case = benchmark_case(slug);
        let payload = daemon.recall_case(case).await;
        let results = payload["results"].as_array().cloned().unwrap_or_default();
        assert!(
            results
                .iter()
                .any(|result| matches_ground_truth(case, result)),
            "cross-query dedup regression for {slug}: {:?}",
            results
                .iter()
                .map(|result| result["source"].as_str().unwrap_or_default())
                .collect::<Vec<_>>()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn distilled_proxy_tracks_full_recall_ranking() {
    let daemon = match TestDaemon::spawn().await {
        Ok(daemon) => daemon,
        Err(SpawnError::Skip(reason)) => {
            if should_fail_on_singleton_skip() {
                panic!("recall benchmark skipped under CI singleton policy: {reason}");
            }
            eprintln!("{reason}");
            return;
        }
        Err(SpawnError::Fatal(err)) => {
            panic!("failed to spawn healthy benchmark daemon after retries: {err}")
        }
    };

    for case in BENCHMARK_CASES {
        daemon.store_case(case).await;
    }

    let mut evaluated_queries = 0usize;
    let mut top1_matches = 0usize;
    let mut concordant_pairs = 0usize;
    let mut total_pairs = 0usize;
    let mut abs_rank_error_sum = 0.0;
    let mut abs_rank_error_count = 0usize;

    for case in BENCHMARK_CASES {
        let payload = daemon.recall_explain_case(case).await;
        let shadow_status = payload["explain"]["shadowSemantic"]["status"].as_str();
        assert_eq!(
            payload["explain"]["shadowSemantic"]["enabled"].as_bool(),
            Some(true),
            "missing shadow semantic diagnostics for {}",
            case.slug
        );
        assert!(
            matches!(
                shadow_status,
                Some("ok") | Some("unavailable") | Some("error")
            ),
            "unexpected shadow semantic status for {}: {:?}",
            case.slug,
            shadow_status
        );
        let returned = payload["explain"]["returned"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if returned.is_empty() {
            continue;
        }
        let spent_tokens = payload["policy"]["budgetReasoning"]["spent"]
            .as_u64()
            .unwrap_or(0);
        let returned_tokens: u64 = returned
            .iter()
            .map(|item| item["tokens"].as_u64().unwrap_or(0))
            .sum();
        assert_eq!(
            spent_tokens, returned_tokens,
            "policy spent tokens should equal returned-token sum for {}",
            case.slug
        );
        let dropped_count = payload["policy"]["budgetReasoning"]["droppedCount"]
            .as_u64()
            .unwrap_or(0);
        let family_compacted = payload["policy"]["budgetReasoning"]["familyCompactedCount"]
            .as_u64()
            .unwrap_or(0);
        let total_pre_budget_drops = payload["policy"]["budgetReasoning"]["totalPreBudgetDrops"]
            .as_u64()
            .unwrap_or(0);
        assert_eq!(
            total_pre_budget_drops,
            dropped_count + family_compacted,
            "policy pre-budget drop accounting mismatch for {}",
            case.slug
        );
        for item in &returned {
            let factors = item["rankingFactors"].as_object().unwrap_or_else(|| {
                panic!(
                    "missing rankingFactors object in explain returned item for {}",
                    case.slug
                )
            });
            assert!(
                factors.contains_key("tokenCost"),
                "missing rankingFactors.tokenCost for {}",
                case.slug
            );
            assert!(
                factors.contains_key("budgetCostRatio"),
                "missing rankingFactors.budgetCostRatio for {}",
                case.slug
            );
            assert!(
                factors.contains_key("entityMatches"),
                "missing rankingFactors.entityMatches for {}",
                case.slug
            );
        }

        let max_token_cost = returned
            .iter()
            .filter_map(|item| {
                item["rankingFactors"]["tokenCost"]
                    .as_u64()
                    .map(|value| value as f64)
            })
            .fold(0.0, f64::max);

        let mut points = Vec::new();
        for (full_rank, item) in returned.iter().enumerate() {
            if item["source"].as_str().is_none() {
                continue;
            }
            if let Some(proxy_score) = distilled_proxy_score_from_explain_item(item, max_token_cost)
            {
                points.push(ProxyRankPoint {
                    full_rank,
                    full_relevance: item["rankingFactors"]["relevance"].as_f64().unwrap_or(0.0),
                    proxy_score,
                });
            }
        }

        if points.is_empty() {
            continue;
        }

        evaluated_queries += 1;
        let mut proxy_sorted = points.clone();
        proxy_sorted.sort_by(compare_proxy_points);
        if proxy_sorted
            .first()
            .is_some_and(|point| point.full_rank == 0)
        {
            top1_matches += 1;
        }

        let mut proxy_rank_by_full_rank = HashMap::new();
        for (idx, point) in proxy_sorted.iter().enumerate() {
            proxy_rank_by_full_rank.insert(point.full_rank, idx);
        }

        for point in &points {
            let proxy_rank = *proxy_rank_by_full_rank
                .get(&point.full_rank)
                .expect("missing proxy rank for full rank");
            abs_rank_error_sum += point.full_rank.abs_diff(proxy_rank) as f64;
            abs_rank_error_count += 1;
        }

        for i in 0..points.len() {
            for j in (i + 1)..points.len() {
                let left = &points[i];
                let right = &points[j];
                if (left.full_relevance - right.full_relevance).abs()
                    < PROXY_PAIRWISE_MIN_RELEVANCE_GAP
                {
                    continue;
                }
                let left_proxy_rank = *proxy_rank_by_full_rank
                    .get(&left.full_rank)
                    .expect("missing proxy rank for left full rank");
                let right_proxy_rank = *proxy_rank_by_full_rank
                    .get(&right.full_rank)
                    .expect("missing proxy rank for right full rank");
                if (left.full_rank < right.full_rank) == (left_proxy_rank < right_proxy_rank) {
                    concordant_pairs += 1;
                }
                total_pairs += 1;
            }
        }
    }

    let min_evaluated_queries =
        (BENCHMARK_CASES.len() as f64 * PROXY_MIN_EVALUATED_QUERY_COVERAGE).ceil() as usize;
    assert!(
        evaluated_queries >= min_evaluated_queries,
        "proxy comparison evaluated too few queries: got {evaluated_queries}, need >= {min_evaluated_queries} ({:.0}% of {})",
        PROXY_MIN_EVALUATED_QUERY_COVERAGE * 100.0,
        BENCHMARK_CASES.len()
    );

    let top1_agreement = top1_matches as f64 / evaluated_queries as f64;
    let pairwise_agreement = if total_pairs == 0 {
        1.0
    } else {
        concordant_pairs as f64 / total_pairs as f64
    };
    let mean_abs_rank_error = if abs_rank_error_count == 0 {
        0.0
    } else {
        abs_rank_error_sum / abs_rank_error_count as f64
    };

    assert!(
        top1_agreement >= PROXY_TOP1_MIN_AGREEMENT,
        "proxy top-1 agreement regression: got {:.3}, need >= {:.2}",
        top1_agreement,
        PROXY_TOP1_MIN_AGREEMENT
    );
    assert!(
        mean_abs_rank_error <= PROXY_MAX_MEAN_ABS_RANK_ERROR,
        "proxy mean absolute rank error regression: got {:.3}, need <= {:.2} (pairwise={:.3})",
        mean_abs_rank_error,
        PROXY_MAX_MEAN_ABS_RANK_ERROR,
        pairwise_agreement
    );
    assert!(
        pairwise_agreement >= PROXY_MIN_PAIRWISE_AGREEMENT,
        "proxy material-pairwise agreement regression: got {:.3}, need >= {:.2}",
        pairwise_agreement,
        PROXY_MIN_PAIRWISE_AGREEMENT
    );
}

fn matches_ground_truth(case: &BenchmarkCase, result: &Value) -> bool {
    let source = result["source"].as_str().unwrap_or_default().to_lowercase();
    let excerpt = result["excerpt"]
        .as_str()
        .unwrap_or_default()
        .to_lowercase();
    case.ground_truth.iter().any(|needle| {
        let needle = needle.to_lowercase();
        source.contains(&needle) || excerpt.contains(&needle)
    })
}

fn ci_requires_benchmark_execution(ci_value: Option<&str>) -> bool {
    ci_value.is_some_and(|value| {
        !matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "" | "0" | "false" | "no"
        )
    })
}

fn should_fail_on_singleton_skip() -> bool {
    ci_requires_benchmark_execution(std::env::var("CI").ok().as_deref())
}

fn build_case_decision(case: &BenchmarkCase) -> String {
    format!(
        "Benchmark fixture for {}. Canonical terms: {}.",
        case.slug.replace('-', " "),
        case.ground_truth.join(", ")
    )
}

fn benchmark_case(slug: &str) -> &'static BenchmarkCase {
    BENCHMARK_CASES
        .iter()
        .find(|case| case.slug == slug)
        .unwrap_or_else(|| panic!("missing benchmark case: {slug}"))
}

fn distilled_proxy_score_from_explain_item(item: &Value, max_token_cost: f64) -> Option<f64> {
    let factors = item.get("rankingFactors")?;
    let relevance = factors.get("relevance")?.as_f64()?;
    let budget_cost_ratio = factors
        .get("budgetCostRatio")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .clamp(0.0, 1.0);
    let entropy_normalized = factors
        .get("entropy")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .clamp(0.0, 6.0)
        / 6.0;
    let token_cost = factors
        .get("tokenCost")
        .and_then(Value::as_u64)
        .unwrap_or(0) as f64;
    let token_cost_ratio = if max_token_cost <= f64::EPSILON {
        0.0
    } else {
        (token_cost / max_token_cost).clamp(0.0, 1.0)
    };
    let method = factors
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let method_hybrid = f64::from(method.eq_ignore_ascii_case("hybrid"));
    let method_semantic = f64::from(method.eq_ignore_ascii_case("semantic"));

    // Distilled 6-feature proxy:
    // relevance, budget-cost ratio, entropy, token-cost ratio, hybrid flag, semantic flag.
    let proxy_score = (relevance * 1.20) - (budget_cost_ratio * 0.25) + (entropy_normalized * 0.10)
        - (token_cost_ratio * 0.12)
        + (method_hybrid * 0.16)
        + (method_semantic * 0.05);
    Some(proxy_score)
}

fn compare_proxy_points(left: &ProxyRankPoint, right: &ProxyRankPoint) -> Ordering {
    right
        .proxy_score
        .partial_cmp(&left.proxy_score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| left.full_rank.cmp(&right.full_rank))
}

fn percentile_95(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let idx = ((sorted.len() - 1) as f64 * 0.95).ceil() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn reserve_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("failed to reserve local port")
        .local_addr()
        .expect("missing local addr")
        .port()
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    std::env::temp_dir().join(format!("cortex_{prefix}_{unique}"))
}

fn write_dummy_model_files(home: &Path) {
    let model_dir = home.join("models");
    fs::write(model_dir.join("all-MiniLM-L6-v2.onnx"), b"test").unwrap();
    fs::write(model_dir.join("tokenizer.json"), b"{}").unwrap();
}

async fn wait_for_health(client: &Client, base_url: &str, child: &mut Child) -> Result<(), String> {
    let deadline = Instant::now() + HEALTH_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("failed to poll daemon") {
            let stderr = read_child_stderr(child);
            return Err(format!(
                "daemon exited before health check succeeded: {status}; stderr: {stderr}"
            ));
        }

        let response = client.get(format!("{base_url}/health")).send().await;
        if let Ok(response) = response {
            if response.status().is_success() {
                return Ok(());
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Err(format!(
        "daemon did not become healthy within {:?}",
        HEALTH_TIMEOUT
    ))
}

async fn wait_for_token(home: &Path, child: &mut Child) -> Result<String, String> {
    let token_path = home.join("cortex.token");
    let deadline = Instant::now() + TOKEN_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("failed to poll daemon") {
            let stderr = read_child_stderr(child);
            return Err(format!(
                "daemon exited before token was written: {status}; stderr: {stderr}"
            ));
        }

        if let Ok(token) = fs::read_to_string(&token_path) {
            let token = token.trim().to_string();
            if !token.is_empty() {
                return Ok(token);
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Err(format!(
        "token file was not written within {:?}",
        TOKEN_TIMEOUT
    ))
}

fn read_child_stderr(child: &mut Child) -> String {
    let mut stderr = String::new();
    if let Some(handle) = child.stderr.as_mut() {
        let _ = handle.read_to_string(&mut stderr);
    }
    stderr
}

fn should_skip_for_active_singleton(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("daemon startup denied: cortex already has an active daemon process")
        || normalized.contains("daemon startup denied: canonical cortex instance is already ready")
        || normalized
            .contains("daemon startup denied: canonical cortex instance is already starting")
        || normalized
            .contains("daemon startup denied: canonical cortex instance is already healthy")
        || normalized.contains("another cortex instance holds the lock")
        || normalized.contains("another process still holds the daemon lock")
        || normalized.contains("app_init_required:")
}

#[test]
fn singleton_skip_detection_matches_known_denial_signals() {
    assert!(should_skip_for_active_singleton(
        "daemon startup denied: Cortex already has an active daemon process (PID 1234)"
    ));
    assert!(should_skip_for_active_singleton(
        "APP_INIT_REQUIRED: codex is attach-only and cannot start the daemon automatically"
    ));
    assert!(should_skip_for_active_singleton(
        "another cortex instance holds the lock"
    ));
    assert!(should_skip_for_active_singleton(
        "daemon startup denied: canonical Cortex instance is already ready on port 7437"
    ));
    assert!(should_skip_for_active_singleton(
        "daemon startup denied: canonical Cortex instance is already starting on port 7437"
    ));
    assert!(should_skip_for_active_singleton(
        "daemon startup denied: canonical Cortex instance is already healthy on port 7437"
    ));
    assert!(should_skip_for_active_singleton(
        "daemon is not healthy on port 7437 and another process still holds the daemon lock"
    ));
}

#[test]
fn singleton_skip_detection_ignores_unrelated_errors() {
    assert!(!should_skip_for_active_singleton(
        "token file was not written within 15s"
    ));
    assert!(!should_skip_for_active_singleton(
        "connection refused while probing readiness endpoint"
    ));
}

#[test]
fn ci_requires_execution_parser_matches_expected_values() {
    assert!(ci_requires_benchmark_execution(Some("true")));
    assert!(ci_requires_benchmark_execution(Some("1")));
    assert!(ci_requires_benchmark_execution(Some("yes")));
    assert!(!ci_requires_benchmark_execution(Some("false")));
    assert!(!ci_requires_benchmark_execution(Some("0")));
    assert!(!ci_requires_benchmark_execution(Some("no")));
    assert!(!ci_requires_benchmark_execution(None));
}

#[test]
fn build_case_decision_does_not_embed_query_literal() {
    let case = benchmark_case("token-optimization");
    let decision = build_case_decision(case);
    assert!(
        !decision.to_ascii_lowercase().contains(case.query),
        "benchmark fixture should not echo raw query"
    );
    assert!(decision.contains("Canonical terms:"));
}
