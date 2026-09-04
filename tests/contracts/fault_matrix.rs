//! Fault-class contracts F13-F15 and F18-F20 (phase6 conformance table §(b), bead cortex-c47).
//!
//! Each test pins the EXACT observable behavior the production code implements
//! under one deterministic fault injection:
//!
//! - F13 port-conflict bind failure: the front door is the serve preflight
//!   `crates/daemon/src/cli/daemon/startup.rs::startup_single_daemon_preflight`
//!   — a probe bind that fails with EADDRINUSE plus failed readiness/health
//!   probes produces "[cortex] FATAL: daemon startup denied: cannot bind
//!   127.0.0.1:{port} (Address already in use ...) ..." and exit(1); the
//!   `server/runtime.rs` run_plain/run_tls "Cannot bind" FATAL lines are the
//!   second line of defense behind that preflight (not reachable
//!   deterministically). A daemon on another port must be unaffected.
//! - F14 partial TLS config: `crates/daemon/src/tls.rs` half-config errors ("TLS cert
//!   found but key missing", empty-PEM "No certificates found in cert file") plus the
//!   `runtime.rs` policy: exit(1) refusal on non-local bind, warn-and-serve-plain
//!   degradation on local solo bind.
//! - F15 TLS handshake failure mid-accept: `runtime.rs` `run_tls` select loop logs
//!   "[cortex] TLS handshake failed: ..." and drops the connection; the accept loop
//!   must survive and the next valid TLS request must succeed.
//! - F18 oversized request body: axum 0.8 `DefaultBodyLimit` (2_097_152 bytes, no
//!   explicit override anywhere in `crates/daemon/src`); body of exactly the limit is
//!   accepted, limit+1 is rejected 413 with axum's exact LengthLimitError message
//!   enveloped as {"error": ...} (see the F18 comment at the assertion), and the daemon keeps serving. Ground truth: axum 0.8.9 /
//!   axum-core 0.5.6 + http-body-util 0.1.x
//!   (`data.remaining() > remaining` => `LengthLimitError` => PAYLOAD_TOO_LARGE).
//! - F19 slowloris: NO read/header timeout layer exists (`runtime.rs` layers are
//!   activity-tracking, CatchPanic, CORS only — the no-claim ledger entry lives in the
//!   gauntlet workspace). This contract pins the survivability half only: a held
//!   partial request must not deny service to other clients.
//! - F20 store write failure: `handlers/store/handler.rs` maps `StoreError::Internal`
//!   to an honest 500 `{"error":"Store failed: ..."}` (never a fake ack); the daemon
//!   must recover once the fault clears. Fault injection: a second SQLite connection
//!   holds the write lock (`BEGIN EXCLUSIVE` in WAL); the daemon's pinned
//!   `busy_timeout` of 5000 ms (`db/connection.rs`) then surfaces SQLITE_BUSY.

#[path = "../support/mod.rs"]
mod support;

use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cortex_tests::cortex_bin;
use support::{
    health_ok, post_raw, read_token, request_json, reserve_port, shutdown_daemon, spawn_daemon,
    unique_temp_dir, wait_for_exit, wait_for_health, SpawnTrackedExt,
};

/// axum-core's compiled-in default (`DefaultBodyLimit`), bytes. Not overridden
/// anywhere in the daemon (`grep DefaultBodyLimit crates/daemon/src` is empty).
const AXUM_DEFAULT_BODY_LIMIT: usize = 2_097_152;

fn spawn_serve_with_env(home: &Path, port: u16, extra_envs: &[(&str, &str)]) -> Child {
    let mut command = Command::new(cortex_bin());
    command
        .args(["serve", "--home", &home.to_string_lossy(), "--port", &port.to_string()])
        .env("CORTEX_SINGLE_DAEMON_TEST_BYPASS", "1")
        .env("CORTEX_BIND", "127.0.0.1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (key, value) in extra_envs {
        command.env(key, value);
    }
    command.spawn_tracked("fault_matrix: spawn cortex serve")
}

fn wait_for_exit_status(child: &mut Child, timeout: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().expect("poll daemon child") {
            Some(status) => return status,
            None => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("daemon child did not exit within {timeout:?}");
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn stderr_of_exited_child(child: &mut Child) -> String {
    let mut stderr = String::new();
    if let Some(mut handle) = child.stderr.take() {
        let _ = handle.read_to_string(&mut stderr);
    }
    stderr
}

fn drain_stderr(child: &mut Child) -> Arc<Mutex<String>> {
    let shared = Arc::new(Mutex::new(String::new()));
    if let Some(mut handle) = child.stderr.take() {
        let buffer = Arc::clone(&shared);
        thread::spawn(move || {
            // Incremental reads: the daemon stays alive for most of the test,
            // so the buffer must grow per chunk instead of waiting for EOF.
            let mut chunk = [0u8; 4096];
            loop {
                match handle.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let text = String::from_utf8_lossy(&chunk[..n]).into_owned();
                        buffer
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .push_str(&text);
                    }
                }
            }
        });
    }
    shared
}

fn captured_stderr(buffer: &Arc<Mutex<String>>) -> String {
    buffer
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn wait_for_captured_line(buffer: &Arc<Mutex<String>>, needle: &str, timeout: Duration) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        let snapshot = captured_stderr(buffer);
        if snapshot.contains(needle) {
            return snapshot;
        }
        if Instant::now() >= deadline {
            panic!("daemon stderr did not contain {needle:?} within {timeout:?}; captured:\n{snapshot}");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn generate_self_signed_cert(dir: &Path) -> (PathBuf, PathBuf) {
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    let status = Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-sha256",
            "-days",
            "2",
            "-nodes",
            "-subj",
            "/CN=127.0.0.1",
            "-keyout",
        ])
        .arg(&key_path)
        .args(["-out"])
        .arg(&cert_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("openssl must be available on PATH for the F15 TLS fault contract");
    assert!(
        status.success(),
        "openssl self-signed cert generation failed: {status}"
    );
    assert!(
        cert_path.is_file() && key_path.is_file(),
        "openssl must write both cert and key files"
    );
    (cert_path, key_path)
}

fn https_probe_code(port: u16, method: &str, path: &str, extra_args: &[&str]) -> Option<String> {
    let output = Command::new("curl")
        .args([
            "-k",
            "-s",
            "--max-time",
            "5",
            "-o",
            "/dev/null",
            "-w",
            "%{http_code}",
            "-X",
            method,
        ])
        .arg(format!("https://127.0.0.1:{port}{path}"))
        .args(extra_args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// F18 body constructor: valid store JSON whose TOTAL byte length is exactly
/// `total_bytes` (all ASCII, so char counts are byte counts). The padding rides in
/// a `filler` field that serde drops on the floor (StoreRequest does not
/// deny_unknown_fields), so the handler-side behavior is identical to a small store.
fn store_body_of_exact_bytes(total_bytes: usize) -> String {
    let prefix = r#"{"decision":"Fault matrix F18 boundary sentinel: the ingest pipeline commits exactly one durable row per accepted store call.","type":"decision","source_agent":"fault-matrix","confidence":0.9,"filler":""#;
    let suffix = r#""}"#;
    assert!(
        total_bytes >= prefix.len() + suffix.len(),
        "requested body size {total_bytes} too small for the fixed JSON envelope"
    );
    let filler = "a".repeat(total_bytes - prefix.len() - suffix.len());
    format!("{prefix}{filler}{suffix}")
}

// ---------------------------------------------------------------------------
// F13 — port conflict at bind
// ---------------------------------------------------------------------------

#[test]
fn f13_port_conflict_bind_exits_fatal_and_leaves_other_daemon_unaffected() {
    // The unaffected control daemon: healthy on its own port and home.
    let healthy_home = unique_temp_dir("fm_f13_healthy");
    fs::create_dir_all(&healthy_home).expect("create temp home");
    let healthy_port = reserve_port();
    let healthy_home_str = healthy_home.to_string_lossy().to_string();
    let mut healthy = spawn_daemon(&healthy_home_str, healthy_port);
    wait_for_health(healthy_port, &mut healthy);

    // Occupy a second port so the conflicting daemon's bind must fail.
    let conflict_port = reserve_port();
    let _blocker = TcpListener::bind(("127.0.0.1", conflict_port))
        .expect("hold port to force bind conflict");

    let conflict_home = unique_temp_dir("fm_f13_conflict");
    fs::create_dir_all(&conflict_home).expect("create temp home");
    let mut conflict = spawn_serve_with_env(&conflict_home, conflict_port, &[]);
    let status = wait_for_exit_status(&mut conflict, Duration::from_secs(20));
    assert_eq!(
        status.code(),
        Some(1),
        "bind conflict must exit(1), got {status}"
    );
    let stderr = stderr_of_exited_child(&mut conflict);
    assert!(
        stderr.contains(&format!(
            "[cortex] FATAL: daemon startup denied: cannot bind 127.0.0.1:{conflict_port} (Address already in use"
        )),
        "must log the exact FATAL startup-denial bind line, got: {stderr}"
    );
    assert!(
        stderr.contains(&format!(
            "readiness probe at http://127.0.0.1:{conflict_port}/readiness failed ("
        )),
        "denial must report the failed readiness probe, got: {stderr}"
    );
    assert!(
        stderr.contains(&format!(
            "fallback health probe at http://127.0.0.1:{conflict_port}/health also failed ("
        )),
        "denial must report the failed fallback health probe, got: {stderr}"
    );
    assert!(
        health_ok(healthy_port),
        "daemon on another port must be unaffected by the failed bind elsewhere"
    );

    shutdown_daemon(healthy_port, &healthy_home);
    wait_for_exit(&mut healthy, Duration::from_secs(10));
}

// ---------------------------------------------------------------------------
// F14 — partial TLS configuration
// ---------------------------------------------------------------------------

#[test]
fn f14_partial_tls_config_refused_on_nonlocal_bind_and_degrades_on_local_solo() {
    // Phase 1: cert present + key missing + non-local bind => refusal exit(1)
    // BEFORE any bind, with the exact tls.rs + runtime.rs messages.
    let refusal_home = unique_temp_dir("fm_f14_refusal");
    fs::create_dir_all(&refusal_home).expect("create temp home");
    let cert_only = refusal_home.join("cert-only.pem");
    fs::write(&cert_only, "-----BEGIN CERTIFICATE-----\nplaceholder\n-----END CERTIFICATE-----\n")
        .expect("write cert-only file");
    let missing_key = refusal_home.join("missing-key.pem");
    let refusal_port = reserve_port();
    let mut refusal = spawn_serve_with_env(
        &refusal_home,
        refusal_port,
        &[
            ("CORTEX_BIND", "0.0.0.0"),
            ("CORTEX_TLS_CERT", cert_only.to_str().expect("utf-8 path")),
            ("CORTEX_TLS_KEY", missing_key.to_str().expect("utf-8 path")),
        ],
    );
    let status = wait_for_exit_status(&mut refusal, Duration::from_secs(20));
    assert_eq!(
        status.code(),
        Some(1),
        "broken TLS config on non-local bind must exit(1), got {status}"
    );
    let stderr = stderr_of_exited_child(&mut refusal);
    assert!(
        stderr.contains(&format!(
            "[cortex] TLS configuration error: TLS cert found but key missing at {}",
            missing_key.display()
        )),
        "must log the exact half-config TLS error naming the missing key, got: {stderr}"
    );
    assert!(
        stderr.contains("Refusing insecure HTTP fallback for non-local bind '0.0.0.0'."),
        "must log the exact non-local refusal, got: {stderr}"
    );
    assert!(
        stderr.contains(
            "Fix TLS certs, bind to localhost, or set CORTEX_ALLOW_INSECURE_REMOTE=1 for explicit temporary override."
        ),
        "must log the exact remediation hint, got: {stderr}"
    );

    // Phase 2: the SAME half config on a local solo bind => exact degradation
    // policy: warn, then serve plain HTTP on the same port.
    let solo_home = unique_temp_dir("fm_f14_solo");
    fs::create_dir_all(&solo_home).expect("create temp home");
    let solo_cert = solo_home.join("cert-only.pem");
    fs::write(&solo_cert, "-----BEGIN CERTIFICATE-----\nplaceholder\n-----END CERTIFICATE-----\n")
        .expect("write solo cert-only file");
    let solo_missing_key = solo_home.join("missing-key.pem");
    let solo_port = reserve_port();
    let mut solo = spawn_serve_with_env(
        &solo_home,
        solo_port,
        &[
            ("CORTEX_TLS_CERT", solo_cert.to_str().expect("utf-8 path")),
            ("CORTEX_TLS_KEY", solo_missing_key.to_str().expect("utf-8 path")),
        ],
    );
    let solo_stderr = drain_stderr(&mut solo);
    wait_for_health(solo_port, &mut solo);
    wait_for_captured_line(
        &solo_stderr,
        &format!(
            "[cortex] TLS certificate error: TLS cert found but key missing at {}",
            solo_missing_key.display()
        ),
        Duration::from_secs(5),
    );
    let solo_snapshot = wait_for_captured_line(
        &solo_stderr,
        "[cortex] Starting without TLS (solo mode -- localhost bind)",
        Duration::from_secs(5),
    );
    assert!(
        solo_snapshot.contains(&format!("[cortex] Listening on http://127.0.0.1:{solo_port}")),
        "degraded solo daemon must serve plain http, got: {solo_snapshot}"
    );

    // Phase 3: garbage PEM cert + existing key => exact empty-PEM parse error
    // (rustls-pki-types finds no PEM sections; tls.rs rejects the empty cert
    // list), and the same local-solo plain-HTTP degradation applies.
    let pem_home = unique_temp_dir("fm_f14_badpem");
    fs::create_dir_all(&pem_home).expect("create temp home");
    let garbage_cert = pem_home.join("cert.pem");
    fs::write(&garbage_cert, "this file contains no PEM sections at all").expect("write garbage cert");
    let garbage_key = pem_home.join("key.pem");
    fs::write(&garbage_key, "also not a key").expect("write garbage key");
    let pem_port = reserve_port();
    let mut pem_daemon = spawn_serve_with_env(
        &pem_home,
        pem_port,
        &[
            ("CORTEX_TLS_CERT", garbage_cert.to_str().expect("utf-8 path")),
            ("CORTEX_TLS_KEY", garbage_key.to_str().expect("utf-8 path")),
        ],
    );
    let pem_stderr = drain_stderr(&mut pem_daemon);
    wait_for_health(pem_port, &mut pem_daemon);
    let pem_snapshot = wait_for_captured_line(
        &pem_stderr,
        "[cortex] TLS certificate error: No certificates found in cert file",
        Duration::from_secs(5),
    );
    assert!(
        pem_snapshot.contains("[cortex] Starting without TLS (solo mode -- localhost bind)"),
        "bad-PEM config must use the same solo degradation, got: {pem_snapshot}"
    );

    shutdown_daemon(solo_port, &solo_home);
    wait_for_exit(&mut solo, Duration::from_secs(10));
    shutdown_daemon(pem_port, &pem_home);
    wait_for_exit(&mut pem_daemon, Duration::from_secs(10));
}

// ---------------------------------------------------------------------------
// F15 — TLS handshake failure mid-accept
// ---------------------------------------------------------------------------

#[test]
fn f15_tls_handshake_failure_mid_accept_is_survived_and_next_request_succeeds() {
    let home_dir = unique_temp_dir("fm_f15_tls");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let (cert_path, key_path) = generate_self_signed_cert(&home_dir);
    let port = reserve_port();
    let mut daemon = spawn_serve_with_env(
        &home_dir,
        port,
        &[
            ("CORTEX_TLS_CERT", cert_path.to_str().expect("utf-8 path")),
            ("CORTEX_TLS_KEY", key_path.to_str().expect("utf-8 path")),
        ],
    );
    let stderr_buffer = drain_stderr(&mut daemon);

    // TLS readiness: poll over real TLS (plain-HTTP probes must NOT succeed
    // here, so this loop uses a TLS client; curl is the dependency-free one).
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if https_probe_code(port, "GET", "/health", &[]).as_deref() == Some("200") {
            break;
        }
        if Instant::now() >= deadline {
            let snapshot = captured_stderr(&stderr_buffer);
            panic!("TLS daemon did not become healthy on port {port}; captured:\n{snapshot}");
        }
        thread::sleep(Duration::from_millis(250));
    }
    wait_for_captured_line(
        &stderr_buffer,
        &format!("[cortex] Listening on https://127.0.0.1:{port}"),
        Duration::from_secs(5),
    );

    // Fault: plaintext HTTP bytes straight into the TLS port. rustls must fail
    // the handshake, the loop logs and drops this one connection only.
    {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to TLS port");
        stream
            .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
            .expect("write plaintext bytes into TLS port");
        let mut rejected = Vec::new();
        let _ = stream.read_to_end(&mut rejected);
    }

    assert!(
        daemon.try_wait().expect("poll daemon").is_none(),
        "daemon must survive a failed TLS handshake mid-accept"
    );
    assert_eq!(
        https_probe_code(port, "GET", "/health", &[]).as_deref(),
        Some("200"),
        "next valid TLS request must succeed after the handshake failure"
    );
    wait_for_captured_line(
        &stderr_buffer,
        "[cortex] TLS handshake failed:",
        Duration::from_secs(5),
    );

    // Graceful shutdown over the same TLS surface.
    let token = read_token(&home_dir);
    let _ = https_probe_code(
        port,
        "POST",
        "/shutdown",
        &[
            "-H",
            &format!("Authorization: Bearer {token}"),
            "-H",
            "X-Cortex-Request: true",
            "-H",
            "Content-Type: application/json",
            "--data",
            "{}",
        ],
    );
    wait_for_exit(&mut daemon, Duration::from_secs(10));
}

// ---------------------------------------------------------------------------
// F18 — oversized request body (axum DefaultBodyLimit pin)
// ---------------------------------------------------------------------------

#[test]
fn f18_oversized_body_rejected_413_at_exact_2mb_default_and_daemon_survives() {
    let home_dir = unique_temp_dir("fm_f18_body");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);
    let bearer = format!("Bearer {token}");

    let raw_store = |body: String| -> (u16, String) {
        let response = post_raw(
            port,
            "/store",
            &[
                ("Authorization", bearer.as_str()),
                ("X-Cortex-Request", "true"),
                ("Content-Type", "application/json"),
            ],
            &body,
        )
        .expect("store request");
        let status = support::http_status(&response);
        let body = support::split_http_body(&response)
            .expect("http body")
            .trim()
            .to_string();
        (status, body)
    };

    // At the limit exactly: accepted, handler runs, honest stored ack.
    let at_limit = store_body_of_exact_bytes(AXUM_DEFAULT_BODY_LIMIT);
    let (status, body) = raw_store(at_limit);
    assert_eq!(
        status, 200,
        "a body of exactly the 2MB default limit must be accepted"
    );
    let parsed: Value =
        serde_json::from_str(&body).unwrap_or_else(|err| panic!("ack must be json: {err}; body={body}"));
    assert_eq!(
        parsed["stored"].as_bool(),
        Some(true),
        "at-limit store must ack stored:true, got {body}"
    );

    // One byte past the limit: 413 carrying axum's exact LengthLimitError message.
    // Since 2c857f4 the daemon's re-wrapping Json extractor envelopes EVERY extractor
    // rejection as {"error": <body_text()>} application/json (the repo-wide wire
    // contract, wire_error_contract.rs); the status and the exact message string are
    // unchanged, only the envelope is new — oracle updated for that commit.
    let over_limit = store_body_of_exact_bytes(AXUM_DEFAULT_BODY_LIMIT + 1);
    let (status, body) = raw_store(over_limit);
    assert_eq!(
        status, 413,
        "a body one byte past the default limit must be rejected 413"
    );
    assert_eq!(
        body,
        "{\"error\":\"Failed to buffer the request body: length limit exceeded\"}",
        "413 must envelope axum's exact LengthLimitError message as JSON"
    );
    let envelope: Value = serde_json::from_str(&body)
        .unwrap_or_else(|err| panic!("413 body must be json: {err}; body={body}"));
    assert_eq!(
        envelope["error"].as_str(),
        Some("Failed to buffer the request body: length limit exceeded"),
        "the raw axum message must survive verbatim inside the error envelope"
    );

    // The daemon and its store path survive the rejection untouched.
    let recovery = request_json(
        port,
        "POST",
        "/store",
        Some(&token),
        Some(json!({
            "decision": "Fault matrix F18 recovery sentinel: the daemon serves normally after rejecting an oversized body.",
            "type": "decision",
            "source_agent": "fault-matrix",
            "confidence": 0.9,
        })),
    )
    .expect("recovery store request");
    assert_eq!(
        recovery.status, 200,
        "daemon must keep serving after a 413 rejection, got {} body {}",
        recovery.status, recovery.body
    );
    assert_eq!(recovery.body["stored"].as_bool(), Some(true));

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
}

// ---------------------------------------------------------------------------
// F19 — slowloris / partial-request stall (survivability half; the missing
// timeout layer itself is a no-claim ledger entry, not a contract here)
// ---------------------------------------------------------------------------

#[test]
fn f19_partial_request_stall_does_not_deny_service_to_other_clients() {
    let home_dir = unique_temp_dir("fm_f19_stall");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);

    // Open a socket and deliver only a fragment of the request line. With no
    // read-timeout layer this connection stays pending (that exposure is the
    // documented no-claim); the pinned contract is that it cannot starve the
    // accept loop while it is held.
    let mut stalled = TcpStream::connect(("127.0.0.1", port)).expect("open stalled socket");
    stalled
        .write_all(b"GET /health HTTP/1.1\r\nHo")
        .expect("write partial request line");
    let _ = stalled.flush();
    thread::sleep(Duration::from_millis(1500));

    let during = request_json(port, "GET", "/health", None, None).expect("health during stall");
    assert_eq!(
        during.status, 200,
        "a second client must be served while a partial request is held open, got {} body {}",
        during.status, during.body
    );
    assert_eq!(
        during.body["status"].as_str(),
        Some("ok"),
        "health must answer normally during the stall, got {}",
        during.body
    );

    drop(stalled);
    let after = request_json(port, "GET", "/health", None, None).expect("health after stall close");
    assert_eq!(
        after.status, 200,
        "daemon must keep serving after the stalled socket is dropped"
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
}

// ---------------------------------------------------------------------------
// F20 — store write failure => honest 500 + recovery
// ---------------------------------------------------------------------------

#[test]
fn f20_store_write_failure_returns_honest_500_and_daemon_recovers() {
    let home_dir = unique_temp_dir("fm_f20_disk");
    fs::create_dir_all(&home_dir).expect("create temp home");
    let port = reserve_port();
    let home = home_dir.to_string_lossy().to_string();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    let baseline = request_json(
        port,
        "POST",
        "/store",
        Some(&token),
        Some(json!({
            "decision": "Fault matrix F20 baseline sentinel recorded before the induced write failure.",
            "type": "decision",
            "source_agent": "fault-matrix",
            "confidence": 0.9,
        })),
    )
    .expect("baseline store");
    assert_eq!(baseline.status, 200, "baseline store must succeed: {}", baseline.body);
    assert_eq!(baseline.body["stored"].as_bool(), Some(true));

    // Fault injection: hold the SQLite write lock from a second connection.
    // WAL readers proceed, so the daemon stays alive and healthy; its next
    // write blocks for the pinned 5s busy_timeout, then fails SQLITE_BUSY.
    let lock = rusqlite::Connection::open(home_dir.join("cortex.db"))
        .expect("open daemon db for lock injection");
    lock.execute_batch("BEGIN EXCLUSIVE")
        .expect("acquire exclusive write lock");

    let blocked = request_json(
        port,
        "POST",
        "/store",
        Some(&token),
        Some(json!({
            "decision": "Fault matrix F20 blocked sentinel issued while the write lock is held externally.",
            "type": "decision",
            "source_agent": "fault-matrix",
            "confidence": 0.9,
        })),
    )
    .expect("store during lock injection");
    assert_eq!(
        blocked.status, 500,
        "failed store write must be an honest 500, got {} body {}",
        blocked.status, blocked.body
    );
    assert_eq!(
        blocked.body["error"],
        json!("Store failed: database is locked"),
        "500 must carry the exact internal error envelope, got {}",
        blocked.body
    );

    lock.execute_batch("ROLLBACK").expect("release write lock");
    drop(lock);

    let recovered = request_json(
        port,
        "POST",
        "/store",
        Some(&token),
        Some(json!({
            "decision": "Fault matrix F20 recovery sentinel proves stores succeed again after the lock clears.",
            "type": "decision",
            "source_agent": "fault-matrix",
            "confidence": 0.9,
        })),
    )
    .expect("store after fault cleared");
    assert_eq!(
        recovered.status, 200,
        "store must succeed once the fault clears, got {} body {}",
        recovered.status, recovered.body
    );
    assert_eq!(recovered.body["stored"].as_bool(), Some(true));
    assert!(
        daemon.try_wait().expect("poll daemon").is_none(),
        "daemon must survive the failed store"
    );

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
}
