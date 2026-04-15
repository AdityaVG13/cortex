use reqwest::Client;
use serde_json::{json, Value};
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
const PROXY_TOP1_MIN_AGREEMENT: f64 = 0.65;
const PROXY_MAX_MEAN_ABS_RANK_ERROR: f64 = 0.90;

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

#[derive(Clone, Debug)]
struct ProxyRankPoint {
    full_rank: usize,
    proxy_score: f64,
}

impl TestDaemon {
    async fn spawn() -> Self {
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
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .expect("failed to spawn cortex daemon");

            match wait_for_health(&client, &base_url, &mut child).await {
                Ok(()) => {
                    let token = wait_for_token(&home, &mut child)
                        .await
                        .expect("token file was not written");
                    return Self {
                        child,
                        home,
                        base_url,
                        token,
                        client,
                    };
                }
                Err(err) => {
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

        panic!("failed to spawn healthy benchmark daemon after retries: {last_error}");
    }

    fn request(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .header("Authorization", format!("Bearer {}", self.token))
            .header("X-Cortex-Request", "true")
            .header("X-Source-Agent", "benchmark-test")
    }

    async fn store_case(&self, case: &BenchmarkCase) {
        let decision = format!(
            "{} benchmark note covering {}.",
            case.query,
            case.ground_truth.join(", ")
        );
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
                        ("k", "6"),
                        ("pool_k", "24"),
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_benchmark_regression_thresholds_hold() {
    let daemon = TestDaemon::spawn().await;

    for case in BENCHMARK_CASES {
        daemon.store_case(case).await;
    }

    let mut precision_sum = 0.0;
    let mut mrr_sum = 0.0;
    let mut token_sum = 0u64;
    let mut query_count = 0u64;

    for case in BENCHMARK_CASES {
        let payload = daemon.recall_case(case).await;
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
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recall_different_queries_do_not_cross_dedup_for_same_agent() {
    let daemon = TestDaemon::spawn().await;

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
    let daemon = TestDaemon::spawn().await;

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
        let returned = payload["explain"]["returned"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        if returned.is_empty() {
            continue;
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

    assert!(
        evaluated_queries >= BENCHMARK_CASES.len() / 2,
        "proxy comparison evaluated too few queries: {evaluated_queries}"
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
