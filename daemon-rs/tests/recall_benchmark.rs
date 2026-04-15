use reqwest::Client;
use serde_json::{json, Value};
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
