//! Acknowledged MCP-stdio writes survive actual process death and kernel reopen.
//! Retired: serve/port/HTTP readiness choreography. No graceful shutdown or
//! explicit checkpoint occurs at the crash boundary.
use cortex_daemon::handlers::health::build_health_payload;
use cortex_daemon::{runtime::LensInput, CortexRuntime};
use cortex_tests::support::run_with_cx;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const SENTINELS: [&str; 8] = [
    "Ingest queue rings at 4096 entries before the oldest tile is evicted.",
    "Ledger compaction runs nightly at 03:00 UTC on the primary shard.",
    "Retrieval index rebuilds incrementally after every 500 stores.",
    "External API gateway pins TLS 1.3 and rejects older handshakes.",
    "Config loader treats unknown keys as fatal boot errors.",
    "Scheduler caps eight concurrent tiles per worker process.",
    "Export writer flushes to disk every 32 megabytes.",
    "Health probe times out a backend after 2500 milliseconds.",
];

struct McpChild {
    child: Child,
    input: ChildStdin,
    output: Receiver<String>,
}
impl Drop for McpChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl McpChild {
    fn open(home: &std::path::Path) -> Self {
        let mut child = Command::new(cortex_tests::cortex_bin())
            .arg("mcp")
            .arg("--home")
            .arg(home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn local MCP process");
        let input = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (send, output) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if send.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            child,
            input,
            output,
        }
    }
    fn call(&mut self, id: usize, tool: &str, args: Value) -> Value {
        writeln!(self.input, "{}", json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":tool,"arguments":args}})).unwrap();
        self.input.flush().unwrap();
        let line = self
            .output
            .recv_timeout(Duration::from_secs(30))
            .expect("MCP response before timeout (stderr inherited)");
        let response: Value = serde_json::from_str(&line).expect("MCP JSON line");
        assert_eq!(response["id"], json!(id));
        assert!(response.get("error").is_none(), "{response}");
        assert_ne!(response["result"]["isError"], true, "{response}");
        serde_json::from_str(response["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    }
    fn crash(&mut self) {
        self.child.kill().expect("kill MCP process without cleanup");
        let status = self.child.wait().unwrap();
        assert!(!status.success());
        #[cfg(unix)]
        {
            use std::os::unix::process::ExitStatusExt;
            assert_eq!(status.signal(), Some(9), "actual SIGKILL required");
        }
    }
}

#[test]
fn acked_stores_survive_sigkill_and_reopen_healthy() {
    let home = tempfile::tempdir().unwrap();
    let mut child = McpChild::open(home.path());
    for (id, text) in SENTINELS.iter().enumerate() {
        let ack = child.call(
            id,
            "cortex_commit",
            json!({"decision":text,"agent":"crash-durability"}),
        );
        assert_eq!(ack["status"], "ok", "{ack}");
        assert_eq!(ack["stored"], 1);
        assert_eq!(ack["legacy_entries"][0]["action"], "inserted");
        assert!(ack["receipt"].is_object());
    }
    let alive = child.call(
        100,
        "cortex_recall",
        json!({"query":SENTINELS[2],"budget":320,"k":10}),
    );
    assert!(alive["results"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["excerpt"] == SENTINELS[2]));
    child.crash();
    run_with_cx(|cx| async move {
        let runtime = CortexRuntime::open_db(&home.path().join("cortex.db"))
            .expect("reopen after process death");
        for text in SENTINELS {
            let recall = runtime
                .lens(
                    &cx,
                    LensInput {
                        query: text.into(),
                        budget: 320,
                        k: 10,
                        agent: "crash-durability".into(),
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert!(
                recall["results"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r["excerpt"] == text),
                "lost {text:?}: {recall}"
            );
        }
        let health = build_health_payload(&cx, runtime.state(), false)
            .await
            .unwrap();
        assert_eq!(health["status"], "ok");
        assert_eq!(health["degraded"], false);
        assert_eq!(health["db_corrupted"], false);
        let conn = runtime.state().db.lock(&cx).await.unwrap();
        let mut stmt = conn.prepare("PRAGMA quick_check").unwrap();
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(rows, ["ok"]);
    });
}
