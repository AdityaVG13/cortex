//! admin_acl — privilege-matrix contract for the 13 `/admin/*` routes.
//!
//! Bead cortex-hau (HYP-025); SPEC-055: "Team-mode destructive endpoints require
//! admin + rated auth" (Info/security-rules.md:90).
//!
//! CODE TRUTH pinned here (not docs truth — all 13 routes are undocumented in
//! Info/; docs debt belongs to cortex-70s):
//!
//! Router: crates/daemon/src/server/router.rs:96-108 registers
//!   POST /admin/user/add           handlers/admin/users.rs:10   (ensure_auth_rated:11 -> ensure_admin:15)
//!   POST /admin/user/rotate-key    handlers/admin/users.rs:60   (61 -> 65)
//!   POST /admin/user/remove        handlers/admin/users.rs:97   (98 -> 102)
//!   GET  /admin/users              handlers/admin/users.rs:129  (130 -> 134)
//!   POST /admin/team/create        handlers/admin/teams.rs:10   (11 -> 15)
//!   POST /admin/team/add-member    handlers/admin/teams.rs:36   (37 -> 41)
//!   POST /admin/team/remove-member handlers/admin/teams.rs:69   (70 -> 74)
//!   GET  /admin/teams              handlers/admin/teams.rs:91   (92 -> 96)
//!   GET  /admin/unowned            handlers/admin/data.rs:10    (11 -> 15)
//!   POST /admin/assign-owner       handlers/admin/data.rs:26    (27 -> 31)
//!   POST /admin/set-visibility     handlers/admin/data.rs:65    (66 -> 70)
//!   POST /admin/archive            handlers/admin/data.rs:93    (94 -> 98)
//!   GET  /admin/stats              handlers/admin/data.rs:115   (116 -> 120)
//!
//! Gate chain (identical on every route):
//!   1. `ensure_auth_rated` (handlers/auth/mod.rs:184) -> `ensure_auth`:
//!      missing `X-Cortex-Request: true` -> 403 {"error":"Missing X-Cortex-Request
//!      header","hint":...} (auth/mod.rs:20-29; the SSRF header check precedes the
//!      token check); missing/unknown Bearer token -> 401 {"error":"Unauthorized"}
//!      (auth/mod.rs:31-40).
//!   2. `ensure_admin` (handlers/auth/mod.rs:83-96) -> `ensure_auth_with_caller`:
//!      a Bearer token equal to the daemon runtime token maps to caller=None ->
//!      403 {"error":"Admin endpoints require team mode"} (auth/mod.rs:85-90);
//!      a `ctx_` key matching a team-user hash yields caller=Some(user_id) whose
//!      `users.role` must be `owner` or `admin`, else
//!      403 {"error":"Insufficient permissions"} (auth/mod.rs:91-94).
//!
//! Team-mode boot fact pinned: the daemon REFUSES plain HTTP in team mode at
//! boot (server/runtime.rs:89-96 exits 1 before serving, even on loopback), so
//! the team-mode matrix is exercised through `build_router` in process; the
//! solo-mode gates stay pinned over real spawned-daemon HTTP.
//!
//! Roles are global (`users.role`, db/team.rs:24-26); team-membership role
//! (`team_members.role`, db/team.rs:36-37) confers NO admin authority.

#[path = "../support/mod.rs"]
mod support;

use axum::body::Body;
use cortex_daemon::auth::{generate_ctx_api_key, hash_api_key_argon2id};
use cortex_daemon::db;
use cortex_daemon::server::build_router;
use cortex_tests::support::team_state;
use serde_json::{json, Value};
use std::io::Read;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};
use support::{
    daemon_spawn_test_guard, read_token, request_json, reserve_port, shutdown_daemon, spawn_daemon,
    unique_temp_dir, wait_for_exit, wait_for_health,
};use tower::ServiceExt;

fn unauthorized() -> Value {
    json!({"error":"Unauthorized"})
}
fn insufficient() -> Value {
    json!({"error":"Insufficient permissions"})
}
fn requires_team_mode() -> Value {
    json!({"error":"Admin endpoints require team mode"})
}
fn missing_request_header() -> Value {
    json!({
        "error":"Missing X-Cortex-Request header",
        "hint":"Include header X-Cortex-Request: true on all Cortex HTTP requests"
    })
}

/// The 13 admin routes with request bodies that pass each handler's `Json`
/// extractor, so probes reach the in-handler auth gates instead of dying in
/// deserialization (422) ahead of them.
fn admin_routes() -> Vec<(&'static str, &'static str, Option<Value>)> {
    vec![
        ("POST", "/admin/user/add", Some(json!({"username":"probe-user"}))),
        ("POST", "/admin/user/rotate-key", Some(json!({"username":"probe-user"}))),
        ("POST", "/admin/user/remove", Some(json!({"username":"probe-user"}))),
        ("GET", "/admin/users", None),
        ("POST", "/admin/team/create", Some(json!({"name":"probe-team"}))),
        ("POST", "/admin/team/add-member", Some(json!({"team_name":"probe-team","username":"probe-user"}))),
        ("POST", "/admin/team/remove-member", Some(json!({"team_name":"probe-team","username":"probe-user"}))),
        ("GET", "/admin/teams", None),
        ("GET", "/admin/unowned", None),
        ("POST", "/admin/assign-owner", Some(json!({"to_user":"probe-user"}))),
        ("POST", "/admin/set-visibility", Some(json!({"table":"memories","ids":[],"visibility":"team"}))),
        ("POST", "/admin/archive", Some(json!({"table":"memories","ids":[]}))),
        ("GET", "/admin/stats", None),
    ]
}

/// Drive one request through the real router (routing + extractors +
/// in-handler gates) and return (status, parsed JSON body). `with_ssrf_header`
/// controls the `X-Cortex-Request` gate explicitly.
async fn call(
    router: &axum::Router,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    with_ssrf_header: bool,
    body: Option<Value>,
) -> (u16, Value) {
    let mut builder = axum::http::Request::builder().method(method).uri(path);
    if let Some(token) = bearer {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    if with_ssrf_header {
        builder = builder.header("x-cortex-request", "true");
    }
    let request = match body {
        Some(value) => builder
            .header("content-type", "application/json")
            .body(Body::from(value.to_string()))
            .expect("build request"),
        None => builder.body(Body::empty()).expect("build request"),
    };
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("router service responds");
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("read response body");
    let value = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!("{method} {path}: non-JSON body ({e}): {}", String::from_utf8_lossy(&bytes))
    });
    (status, value)
}

async fn assert_envelope(
    router: &axum::Router,
    method: &str,
    path: &str,
    bearer: Option<&str>,
    body: Option<Value>,
    expected_status: u16,
    expected_body: &Value,
    label: &str,
) {
    let (status, value) = call(router, method, path, bearer, true, body).await;
    assert_eq!(
        status, expected_status,
        "{label} must return {expected_status}, got {status} body {value}"
    );
    assert_eq!(
        &value, expected_body,
        "{label} must return the exact {expected_status} envelope, got {value}"
    );
}

/// Seed a team user row exactly as /admin/user/add would (argon2id hash) and
/// return (user_id, plaintext key, stored hash).
fn seed_team_user(conn: &rusqlite::Connection, username: &str, role: &str) -> (i64, String, String) {
    let key = generate_ctx_api_key();
    let hash = hash_api_key_argon2id(&key).expect("hash seeded api key");
    conn.execute(
        "INSERT INTO users (username, display_name, api_key_hash, role) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![username, username, hash, role],
    )
    .expect("seed team user");
    (conn.last_insert_rowid(), key, hash)
}

#[tokio::test]
async fn admin_acl_team_mode_matrix() {
    let mut state = team_state(1);
    let owner_key;
    let member_key;
    let admin_key;
    let owner_two_key;
    let decision_ids;
    {
        let conn = state.db.lock().await;
        db::create_team_mode_tables(&conn).expect("create team tables");

        // Seeding order fixes user ids: owner=1 (matches team_state(1)),
        // member=2, admin=3, owner-two=4, disposable=5.
        let (owner_id, o_key, o_hash) = seed_team_user(&conn, "owner-one", "owner");
        assert_eq!(owner_id, 1, "owner-one is user id 1");
        owner_key = o_key;
        let (member_id, m_key, m_hash) = seed_team_user(&conn, "member-one", "member");
        assert_eq!(member_id, 2, "member-one is user id 2");
        member_key = m_key;
        let (admin_id, a_key, a_hash) = seed_team_user(&conn, "admin-one", "admin");
        assert_eq!(admin_id, 3, "admin-one is user id 3");
        admin_key = a_key;
        let (owner_two_id, o2_key, o2_hash) = seed_team_user(&conn, "owner-two", "owner");
        assert_eq!(owner_two_id, 4, "owner-two is user id 4");
        owner_two_key = o2_key;
        let (disposable_id, _, d_hash) = seed_team_user(&conn, "disposable", "member");
        assert_eq!(disposable_id, 5, "disposable is user id 5");

        conn.execute("INSERT INTO decisions (decision) VALUES (?1)", ["ACL_SEED_DECISION_ALPHA"])
            .expect("seed decision");
        conn.execute("INSERT INTO decisions (decision) VALUES (?1)", ["ACL_SEED_DECISION_BETA"])
            .expect("seed decision");
        let mut stmt = conn
            .prepare("SELECT id FROM decisions WHERE decision LIKE 'ACL_SEED%' ORDER BY id")
            .expect("prepare decision ids");
        decision_ids = stmt
            .query_map([], |r| r.get::<_, i64>(0))
            .expect("query decision ids")
            .map(|r| r.expect("decision id"))
            .collect::<Vec<i64>>();
        assert_eq!(decision_ids.len(), 2, "both seeded decisions present");

        // Populate the auth cache exactly as the daemon does at boot
        // (state/init.rs:90-99: every users row's id + api_key_hash).
        let mut hashes = state.team_api_key_hashes.write().expect("hash cache lock");
        hashes.push((owner_id, o_hash));
        hashes.push((member_id, m_hash));
        hashes.push((admin_id, a_hash));
        hashes.push((owner_two_id, o2_hash));
        hashes.push((disposable_id, d_hash));
    }
    // Team-mode daemon semantics (state/init.rs:103-104): the runtime token is
    // the token-file content — the owner's ctx_ key, unrotated.
    state.token = Arc::new(owner_key.clone());

    let router = build_router(state, 7437);
    let router = &router;

    // --- Positive controls: users.role='admin' (admin-one) reaches every
    // handler family through the real router.

    // users family
    let (status, body) = call(router, "GET", "/admin/users", Some(&admin_key), true, None).await;
    assert_eq!(status, 200, "users.role=admin must list users, body {body}");
    let listed: Vec<(String, String)> = body["users"]
        .as_array()
        .expect("users array")
        .iter()
        .map(|u| {
            (
                u["username"].as_str().unwrap_or_default().to_string(),
                u["role"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    assert_eq!(listed.len(), 5, "all seeded users listed, got {listed:?}");
    assert!(listed.contains(&("owner-one".into(), "owner".into())), "owner-one listed, got {listed:?}");
    assert!(listed.contains(&("member-one".into(), "member".into())), "member-one listed, got {listed:?}");
    assert!(listed.contains(&("admin-one".into(), "admin".into())), "admin-one listed, got {listed:?}");

    let (status, body) = call(
        router,
        "POST",
        "/admin/user/add",
        Some(&admin_key),
        true,
        Some(json!({"username":"rotate-me","role":"member"})),
    )
    .await;
    assert_eq!(status, 200, "user add must succeed, body {body}");
    assert_eq!(body["username"], "rotate-me", "user add echoes username");
    assert_eq!(body["role"], "member", "user add echoes role");
    let rotate_me_old = body["api_key"].as_str().expect("api_key in add response").to_string();
    assert!(rotate_me_old.starts_with("ctx_"), "generated key is ctx_-prefixed");

    let (status, body) = call(
        router,
        "POST",
        "/admin/user/rotate-key",
        Some(&admin_key),
        true,
        Some(json!({"username":"rotate-me"})),
    )
    .await;
    assert_eq!(status, 200, "rotate-key must succeed, body {body}");
    let rotate_me_new = body["api_key"].as_str().expect("rotated api_key").to_string();
    assert_ne!(rotate_me_old, rotate_me_new, "rotation must issue a different key");

    // teams family
    let (status, body) = call(
        router,
        "POST",
        "/admin/team/create",
        Some(&admin_key),
        true,
        Some(json!({"name":"core"})),
    )
    .await;
    assert_eq!(status, 200, "team create must succeed, body {body}");
    assert_eq!(body["name"], "core", "team create echoes name");
    assert!(body["team_id"].is_i64(), "team create returns team_id");

    let (status, body) = call(
        router,
        "POST",
        "/admin/team/add-member",
        Some(&admin_key),
        true,
        Some(json!({"team_name":"core","username":"member-one","role":"admin"})),
    )
    .await;
    assert_eq!(status, 200, "add-member must succeed, body {body}");
    assert_eq!(body["role"], "admin", "member-one joins core as team-role admin");

    let (status, body) = call(router, "GET", "/admin/teams", Some(&admin_key), true, None).await;
    assert_eq!(status, 200, "team list must succeed, body {body}");
    let core = body["teams"]
        .as_array()
        .expect("teams array")
        .iter()
        .find(|t| t["name"] == "core")
        .unwrap_or_else(|| panic!("team core must be listed, body {body}"));
    assert_eq!(core["member_count"], 1, "core has exactly member-one");

    let (status, body) = call(
        router,
        "POST",
        "/admin/team/add-member",
        Some(&admin_key),
        true,
        Some(json!({"team_name":"core","username":"disposable","role":"member"})),
    )
    .await;
    assert_eq!(status, 200, "add disposable must succeed, body {body}");

    let (status, body) = call(
        router,
        "POST",
        "/admin/team/remove-member",
        Some(&admin_key),
        true,
        Some(json!({"team_name":"core","username":"disposable"})),
    )
    .await;
    assert_eq!(status, 200, "remove-member must succeed, body {body}");
    assert_eq!(
        body,
        json!({"removed":{"team":"core","username":"disposable"}}),
        "remove-member exact envelope"
    );

    // data family
    let (status, body) = call(
        router,
        "POST",
        "/admin/set-visibility",
        Some(&admin_key),
        true,
        Some(json!({"table":"decisions","ids":decision_ids,"visibility":"shared"})),
    )
    .await;
    assert_eq!(status, 200, "set-visibility must succeed, body {body}");
    assert_eq!(body["updated"], 2, "both seeded rows flipped to shared");

    let (status, body) = call(
        router,
        "POST",
        "/admin/archive",
        Some(&admin_key),
        true,
        Some(json!({"table":"decisions","ids":[decision_ids[1]]})),
    )
    .await;
    assert_eq!(status, 200, "archive must succeed, body {body}");
    assert_eq!(body["archived"], 1, "second seeded row archived");

    let (status, body) = call(
        router,
        "POST",
        "/admin/assign-owner",
        Some(&admin_key),
        true,
        Some(json!({"to_user":"member-one","from_user":"admin-one","table":"decisions"})),
    )
    .await;
    assert_eq!(status, 200, "assign-owner must succeed, body {body}");
    assert!(body["assigned"]["decisions"].is_i64(), "assigned reports per-table counts");

    let (status, body) = call(router, "GET", "/admin/unowned", Some(&admin_key), true, None).await;
    assert_eq!(status, 200, "unowned must succeed, body {body}");
    let unowned_map = body["unowned"].as_object().expect("unowned object");
    assert_eq!(unowned_map.len(), 12, "unowned reports all 12 owner tables");

    let (status, body) = call(router, "GET", "/admin/stats", Some(&admin_key), true, None).await;
    assert_eq!(status, 200, "stats must succeed, body {body}");
    assert_eq!(body["user_count"], 6, "5 seeded + rotate-me");
    assert_eq!(body["team_count"], 1, "only core exists");

    // Removal + live eviction of the removed user's key from the auth cache.
    let (status, body) = call(
        router,
        "POST",
        "/admin/user/remove",
        Some(&admin_key),
        true,
        Some(json!({"username":"rotate-me"})),
    )
    .await;
    assert_eq!(status, 200, "user remove must succeed, body {body}");
    assert_eq!(body, json!({"removed":"rotate-me"}), "user remove exact envelope");
    let (status, body) = call(router, "GET", "/admin/users", Some(&rotate_me_new), true, None).await;
    assert_eq!(status, 401, "removed user's key must stop working");
    assert_eq!(body, unauthorized(), "removed user's key gets the exact 401 envelope");

    // --- Role gradation.
    // users.role='owner' with a team-user key (owner-two, NOT the runtime
    // token) passes ensure_admin.
    let (status, body) = call(router, "GET", "/admin/users", Some(&owner_two_key), true, None).await;
    assert_eq!(status, 200, "users.role=owner team key must pass, body {body}");

    // users.role='member' is rejected on EVERY admin route even though
    // member-one is a team-role admin of core — team_members.role confers no
    // admin authority (ensure_admin reads users.role only).
    for (method, path, probe_body) in admin_routes() {
        assert_envelope(
            router,
            method,
            path,
            Some(&member_key),
            probe_body.clone(),
            403,
            &insufficient(),
            &format!("member {method} {path}"),
        )
        .await;
    }

    // --- Unauthenticated: every route rejects with the exact 401 envelope
    // (SSRF header present so the token check is reached).
    for (method, path, probe_body) in admin_routes() {
        assert_envelope(
            router,
            method,
            path,
            None,
            probe_body.clone(),
            401,
            &unauthorized(),
            &format!("unauthenticated {method} {path}"),
        )
        .await;
    }

    // --- Envelope precedence specifics on a representative route.
    let (status, body) = call(router, "GET", "/admin/stats", None, false, None).await;
    assert_eq!(status, 403, "no headers at all still hits the header gate");
    assert_eq!(body, missing_request_header(), "missing X-Cortex-Request exact envelope");
    let (status, body) = call(
        router,
        "GET",
        "/admin/stats",
        Some(&member_key),
        false,
        None,
    )
    .await;
    assert_eq!(status, 403, "token without SSRF header still hits the header gate first");
    assert_eq!(body, missing_request_header(), "SSRF header check precedes token check");

    // Garbage bearer token: exact 401 envelope.
    assert_envelope(
        router,
        "GET",
        "/admin/stats",
        Some("not-a-real-token"),
        None,
        401,
        &unauthorized(),
        "garbage token",
    )
    .await;

    // Well-formed ctx_ key that matches no user: exact 401 envelope.
    let stranger = generate_ctx_api_key();
    assert_envelope(
        router,
        "GET",
        "/admin/stats",
        Some(&stranger),
        None,
        401,
        &unauthorized(),
        "unknown ctx_ key",
    )
    .await;

    // --- CONFIRMED FINDING (availability, pinned as-is): in team mode the
    // daemon's runtime token IS the persisted owner key (state/init.rs:103-104
    // reads cortex.token written by `setup --team` without rotating it), and
    // any token equal to the runtime token maps to caller=None in
    // ensure_admin (handlers/auth/mod.rs:85-90). The owner credential is
    // therefore locked out of the admin surface with the team-mode 403
    // envelope. This fails closed (no escalation) but breaks the documented
    // owner flow `cortex user add` (Info/team-mode-setup.md:93), which runs
    // with exactly this credential.
    let (status, body) = call(router, "GET", "/admin/users", Some(&owner_key), true, None).await;
    assert_eq!(
        status, 403,
        "owner key == runtime token is locked out of the admin surface"
    );
    assert_eq!(
        body,
        requires_team_mode(),
        "owner runtime-token lockout uses the team-mode 403 envelope"
    );
}

/// Regression (cortex-70s): `cortex team create <name>` (cli/admin.rs) used to
/// POST `{"team":<name>}`, but the handler's `TeamCreateBody`
/// (handlers/admin/types.rs:31) deserializes the field `name` — every CLI team
/// create died in the Json extractor as 422 before the handler ran. The
/// handler is the wire truth, so this pins BOTH shapes through the same
/// oneshot router as the matrix test: the CLI's `{"name": ...}` payload is
/// accepted (200) and the team actually exists with the requested name, while
/// the legacy `{"team": ...}` shape stays pinned as a 422 that creates nothing
/// (so neither side of the contract can silently drift back to the bug).
#[tokio::test]
async fn admin_acl_team_create_wire_payload_contract() {
    let mut state = team_state(1);
    let admin_key;
    {
        let conn = state.db.lock().await;
        db::create_team_mode_tables(&conn).expect("create team tables");
        let (admin_id, a_key, a_hash) = seed_team_user(&conn, "admin-one", "admin");
        assert_eq!(admin_id, 1, "admin-one is user id 1");
        admin_key = a_key;
        let mut hashes = state.team_api_key_hashes.write().expect("hash cache lock");
        hashes.push((admin_id, a_hash));
    }
    let router = build_router(state, 7438);
    let router = &router;

    // The exact payload `cortex team create cli-made` now sends.
    let (status, body) = call(
        router,
        "POST",
        "/admin/team/create",
        Some(&admin_key),
        true,
        Some(json!({"name":"cli-made"})),
    )
    .await;
    assert_eq!(status, 200, "CLI-shaped {{\"name\":...}} payload must be accepted, body {body}");
    assert_eq!(body["name"], "cli-made", "team create echoes the requested name");
    assert!(body["team_id"].is_i64(), "team create returns team_id");

    // The team must actually exist under the requested name, not just echo it.
    let (status, body) = call(router, "GET", "/admin/teams", Some(&admin_key), true, None).await;
    assert_eq!(status, 200, "team list must succeed, body {body}");
    let created = body["teams"]
        .as_array()
        .expect("teams array")
        .iter()
        .find(|t| t["name"] == "cli-made")
        .unwrap_or_else(|| panic!("team cli-made must exist after CLI-shaped create, body {body}"));
    assert_eq!(created["member_count"], 0, "freshly created team has no members");

    // The legacy buggy CLI shape is rejected by the Json extractor (422)
    // BEFORE the handler, and creates nothing. The extractor rejection body is
    // plain text (not JSON), so this probe reads the raw response instead of
    // the JSON-parsing `call` helper.
    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/admin/team/create")
        .header("authorization", format!("Bearer {admin_key}"))
        .header("x-cortex-request", "true")
        .header("content-type", "application/json")
        .body(Body::from(json!({"team":"legacy-shape"}).to_string()))
        .expect("build legacy-shape request");
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("router service responds");
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("read legacy-shape response body");
    let legacy_body = String::from_utf8_lossy(&bytes);
    assert_eq!(status, 422, "legacy {{\"team\":...}} shape must die in the extractor, body {legacy_body}");
    assert!(
        legacy_body.contains("missing field `name`"),
        "extractor rejection must cite the missing `name` field, body {legacy_body}"
    );
    let (status, body) = call(router, "GET", "/admin/teams", Some(&admin_key), true, None).await;
    assert_eq!(status, 200, "team list must succeed after legacy probe, body {body}");
    assert!(
        !body["teams"]
            .as_array()
            .expect("teams array")
            .iter()
            .any(|t| t["name"] == "legacy-shape"),
        "legacy-shaped create must not create a team, body {body}"
    );
}

/// Drive one raw request and return (status, raw body text). The axum `Json`
/// extractor rejects bad wire shapes (422) with a plain-text body, so
/// wire-shape probes that expect rejection must read the body unparsed.
async fn raw_call(router: &axum::Router, method: &str, path: &str, bearer: &str, body: Value) -> (u16, String) {
    let request = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {bearer}"))
        .header("x-cortex-request", "true")
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("build raw request");
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("router service responds");
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("read raw response body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Shared wire-contract fixture (cortex-xw3 class): team-mode tables, an
/// admin team user ("admin-one", id 1) plus `extras` seeded as members in id
/// order starting at 2, auth cache populated, and `seed` run against the same
/// connection before the router is built. Returns the real router so payload
/// probes cross routing + extractors exactly as the daemon would see them,
/// plus the admin ctx_ key.
async fn wire_contract_router(
    port: u16,
    extras: &[&str],
    seed: impl FnOnce(&rusqlite::Connection),
) -> (axum::Router, String) {
    let mut state = team_state(1);
    let admin_key;
    {
        let conn = state.db.lock().await;
        db::create_team_mode_tables(&conn).expect("create team tables");
        let (admin_id, a_key, a_hash) = seed_team_user(&conn, "admin-one", "admin");
        assert_eq!(admin_id, 1, "admin-one is user id 1");
        admin_key = a_key;
        let mut hashes = state.team_api_key_hashes.write().expect("hash cache lock");
        hashes.push((admin_id, a_hash));
        for (idx, name) in extras.iter().enumerate() {
            let (id, _, hash) = seed_team_user(&conn, name, "member");
            assert_eq!(id, idx as i64 + 2, "seed order fixes member ids at 2..");
            hashes.push((id, hash));
        }
        seed(&conn);
    }
    (build_router(state, port), admin_key)
}

/// Read helper: `member_count` of `team` via GET /admin/teams, or None when
/// the team is absent — the read model a CLI team add/remove must move.
async fn member_count(router: &axum::Router, admin_key: &str, team: &str) -> Option<i64> {
    let (status, body) = call(router, "GET", "/admin/teams", Some(admin_key), true, None).await;
    assert_eq!(status, 200, "team list read must succeed, body {body}");
    body["teams"]
        .as_array()
        .expect("teams array")
        .iter()
        .find(|t| t["name"] == team)
        .and_then(|t| t["member_count"].as_i64())
}

/// Read helper: the unowned decisions count via GET /admin/unowned — the read
/// model a CLI assign-owner must move.
async fn unowned_decisions(router: &axum::Router, admin_key: &str) -> i64 {
    let (status, body) = call(router, "GET", "/admin/unowned", Some(admin_key), true, None).await;
    assert_eq!(status, 200, "unowned read must succeed, body {body}");
    body["unowned"]["decisions"].as_i64().expect("decisions count")
}

/// Regression (cortex-xw3): `cortex team add/remove <team> <user>` used to
/// POST `{"team":...,"username":...}`, but the handlers deserialize
/// `TeamMemberBody`/`TeamRemoveMemberBody` (handlers/admin/types.rs:35,41)
/// with the field `team_name` — every CLI team add/remove died in the Json
/// extractor as 422 before the handler ran. Same bug class as the cortex-70s
/// create fix. Pins BOTH sides of the wire: the CLI's corrected
/// `{"team_name":...}` payload is accepted (200) with the membership effect
/// verified through GET /admin/teams reads, while the legacy `{"team":...}`
/// shape stays pinned as a 422 that changes nothing.
#[tokio::test]
async fn admin_acl_team_add_remove_wire_payload_contract() {
    let (router, admin_key) = wire_contract_router(7439, &["member-one", "member-two"], |_| {}).await;
    let router = &router;

    // Fixture team via the already-pinned correct create shape.
    let (status, body) = call(
        router,
        "POST",
        "/admin/team/create",
        Some(&admin_key),
        true,
        Some(json!({"name":"cli-team"})),
    )
    .await;
    assert_eq!(status, 200, "team create fixture must succeed, body {body}");
    assert_eq!(member_count(router, &admin_key, "cli-team").await, Some(0), "fixture team starts empty");

    // Legacy buggy add shape: 422 in the extractor, membership unchanged.
    let (status, raw) = raw_call(
        router,
        "POST",
        "/admin/team/add-member",
        &admin_key,
        json!({"team":"cli-team","username":"member-one"}),
    )
    .await;
    assert_eq!(status, 422, "legacy {{\"team\":...}} add shape must die in the extractor, body {raw}");
    assert!(
        raw.contains("missing field `team_name`"),
        "add rejection must cite the missing `team_name` field, body {raw}"
    );
    assert_eq!(
        member_count(router, &admin_key, "cli-team").await,
        Some(0),
        "legacy-shaped add must not add a member"
    );

    // Corrected CLI add shape (no --role): 200, handler defaults role to member.
    let (status, body) = call(
        router,
        "POST",
        "/admin/team/add-member",
        Some(&admin_key),
        true,
        Some(json!({"team_name":"cli-team","username":"member-one"})),
    )
    .await;
    assert_eq!(status, 200, "CLI-shaped add must be accepted, body {body}");
    assert_eq!(body["team"], "cli-team", "add echoes team_name as team");
    assert_eq!(body["username"], "member-one", "add echoes username");
    assert_eq!(body["role"], "member", "role defaults to member without --role");
    assert_eq!(member_count(router, &admin_key, "cli-team").await, Some(1), "add must be visible in the read model");

    // Corrected CLI add shape with the forwarded --role admin flag.
    let (status, body) = call(
        router,
        "POST",
        "/admin/team/add-member",
        Some(&admin_key),
        true,
        Some(json!({"team_name":"cli-team","username":"member-two","role":"admin"})),
    )
    .await;
    assert_eq!(status, 200, "CLI-shaped add with --role must be accepted, body {body}");
    assert_eq!(body["role"], "admin", "forwarded --role must reach the handler");
    assert_eq!(member_count(router, &admin_key, "cli-team").await, Some(2), "second add visible in the read model");

    // Legacy buggy remove shape: 422 in the extractor, membership unchanged.
    let (status, raw) = raw_call(
        router,
        "POST",
        "/admin/team/remove-member",
        &admin_key,
        json!({"team":"cli-team","username":"member-two"}),
    )
    .await;
    assert_eq!(status, 422, "legacy {{\"team\":...}} remove shape must die in the extractor, body {raw}");
    assert!(
        raw.contains("missing field `team_name`"),
        "remove rejection must cite the missing `team_name` field, body {raw}"
    );
    assert_eq!(
        member_count(router, &admin_key, "cli-team").await,
        Some(2),
        "legacy-shaped remove must not remove a member"
    );

    // Corrected CLI remove shape: exact envelope + effect verified by read.
    let (status, body) = call(
        router,
        "POST",
        "/admin/team/remove-member",
        Some(&admin_key),
        true,
        Some(json!({"team_name":"cli-team","username":"member-two"})),
    )
    .await;
    assert_eq!(status, 200, "CLI-shaped remove must be accepted, body {body}");
    assert_eq!(
        body,
        json!({"removed":{"team":"cli-team","username":"member-two"}}),
        "remove-member exact envelope"
    );
    assert_eq!(
        member_count(router, &admin_key, "cli-team").await,
        Some(1),
        "removed member must vanish from the read model"
    );
}

/// Regression (cortex-xw3): `cortex admin assign-owner` used to POST `{}`
/// (the --from/--to/--table flags were validated, then dropped), but
/// `AssignOwnerBody` (handlers/admin/types.rs:46) requires `to_user` — every
/// CLI assign-owner died in the Json extractor as 422. Pins the corrected CLI
/// shape (`to_user` + null-able `from_user`/`table`, and the full-flag
/// variant) with the reassignment effect verified through GET /admin/unowned
/// and /admin/stats reads, and the legacy `{}` shape as a 422 that reassigns
/// nothing.
#[tokio::test]
async fn admin_acl_assign_owner_wire_payload_contract() {
    let (router, admin_key) = wire_contract_router(7440, &["member-one"], |conn| {
        conn.execute("INSERT INTO decisions (decision) VALUES (?1)", ["WIRE_ASSIGN_A"])
            .expect("seed decision a");
        conn.execute("INSERT INTO decisions (decision) VALUES (?1)", ["WIRE_ASSIGN_B"])
            .expect("seed decision b");
    })
    .await;
    let router = &router;

    assert_eq!(unowned_decisions(router, &admin_key).await, 2, "both seeded decisions start unowned");

    // Legacy buggy empty-{} shape: 422 in the extractor, nothing reassigned.
    let (status, raw) = raw_call(router, "POST", "/admin/assign-owner", &admin_key, json!({})).await;
    assert_eq!(status, 422, "legacy {{}} assign-owner shape must die in the extractor, body {raw}");
    assert!(
        raw.contains("missing field `to_user`"),
        "rejection must cite the missing `to_user` field, body {raw}"
    );
    assert_eq!(unowned_decisions(router, &admin_key).await, 2, "legacy-shaped assign-owner must reassign nothing");

    // Corrected CLI shape with flags absent (from_user/table serialize as
    // null): reassigns all NULL-owner rows to --to.
    let from_user: Option<String> = None;
    let table: Option<String> = None;
    let (status, body) = call(
        router,
        "POST",
        "/admin/assign-owner",
        Some(&admin_key),
        true,
        Some(json!({"to_user":"member-one","from_user":from_user,"table":table})),
    )
    .await;
    assert_eq!(status, 200, "CLI-shaped assign-owner must be accepted, body {body}");
    assert_eq!(body["assigned"]["decisions"], 2, "both NULL-owner decisions reassigned");
    assert_eq!(unowned_decisions(router, &admin_key).await, 0, "unowned read must drop to zero");

    // Corrected CLI shape with --from/--table forwarded: scoped transfer back.
    let (status, body) = call(
        router,
        "POST",
        "/admin/assign-owner",
        Some(&admin_key),
        true,
        Some(json!({"to_user":"admin-one","from_user":"member-one","table":"decisions"})),
    )
    .await;
    assert_eq!(status, 200, "full-flag assign-owner must be accepted, body {body}");
    assert_eq!(body["assigned"]["decisions"], 2, "scoped transfer moves both rows");
    let (status, body) = call(router, "GET", "/admin/stats", Some(&admin_key), true, None).await;
    assert_eq!(status, 200, "stats read must succeed, body {body}");
    let admin_row = body["per_user"]
        .as_array()
        .expect("per_user array")
        .iter()
        .find(|u| u["username"] == "admin-one")
        .unwrap_or_else(|| panic!("admin-one in per_user stats, body {body}"));
    assert_eq!(admin_row["decisions"], 2, "from_user/table forwarding must be visible in the read model");
}

/// Regression (cortex-xw3 companion): `cortex user add` validated --role and
/// --display-name but dropped both from the payload; `UserAddBody`
/// (handlers/admin/types.rs:21) accepts them as optional fields, so the
/// handler silently defaulted role to "member" and the CLI's advertised
/// flags never took effect. Pins the corrected CLI shape (username + role +
/// display_name, optional fields serialized as null when absent) with
/// role/display_name verified through the GET /admin/users read. There is no
/// legacy-422 half here: the pre-fix payload `{"username":...}` was and stays
/// wire-valid — the defect was silent flag loss, pinned by the CLI diff.
#[tokio::test]
async fn admin_acl_user_add_wire_payload_contract() {
    let (router, admin_key) = wire_contract_router(7441, &[], |_| {}).await;
    let router = &router;

    let (status, body) = call(
        router,
        "POST",
        "/admin/user/add",
        Some(&admin_key),
        true,
        Some(json!({"username":"cli-user","role":"admin","display_name":"CLI User"})),
    )
    .await;
    assert_eq!(status, 200, "CLI-shaped user add must be accepted, body {body}");
    assert_eq!(body["username"], "cli-user", "user add echoes username");
    assert_eq!(body["role"], "admin", "forwarded --role must reach the handler");
    let api_key = body["api_key"].as_str().expect("api_key in add response");
    assert!(api_key.starts_with("ctx_"), "generated key is ctx_-prefixed");

    let (status, body) = call(router, "GET", "/admin/users", Some(&admin_key), true, None).await;
    assert_eq!(status, 200, "users read must succeed, body {body}");
    let user = body["users"]
        .as_array()
        .expect("users array")
        .iter()
        .find(|u| u["username"] == "cli-user")
        .unwrap_or_else(|| panic!("cli-user must exist after CLI-shaped add, body {body}"));
    assert_eq!(user["role"], "admin", "role persisted from the forwarded flag");
    assert_eq!(user["display_name"], "CLI User", "display_name persisted from the forwarded flag");
}

/// Regression (cortex-04h): `cortex admin rollback` used to POST
/// `/admin/rollback` (a route that does not exist anywhere in
/// server/router.rs -> daemon 404) with an empty `{}` body that
/// `RollbackRequest` (server/handlers.rs:91, required field `to`: i64) would
/// reject as 422 even on the right path. The corrected CLI POSTs the
/// TOP-LEVEL `/rollback` route (server/router.rs:59) with
/// `{"to": <versions.id>}`. A 200 is deliberately not pinned here: asserting
/// success would require seeding real trace versions through
/// crate::traces, which is out of scope for this contract. Instead the
/// corrected payload is pinned at the handler's explicit BAD_REQUEST branch
/// (server/handlers.rs:110, `{"error": ...}` surfaced by
/// traces::rollback_to) for an unknown version id — which simultaneously
/// proves the route exists (a 404 would fail) and that the `{"to": N}` body
/// passes the Json extractor — while the legacy empty-`{}` shape stays
/// pinned as a 422 that names the missing `to` field.
#[tokio::test]
async fn rollback_wire_contract() {
    let (router, admin_key) = wire_contract_router(7442, &[], |_| {}).await;
    let router = &router;

    // Legacy buggy CLI shape: empty {} body dies in the extractor.
    let (status, raw) = raw_call(router, "POST", "/rollback", &admin_key, json!({})).await;
    assert_eq!(status, 422, "legacy {{}} rollback body must die in the extractor, body {raw}");
    assert!(
        raw.contains("missing field `to`"),
        "extractor rejection must cite the missing `to` field, body {raw}"
    );

    // Corrected CLI shape: the route exists and the body parses; the unknown
    // version id reaches the handler and returns its explicit 400 envelope.
    let (status, raw) = raw_call(router, "POST", "/rollback", &admin_key, json!({"to": 424242})).await;
    assert_eq!(
        status, 400,
        "/rollback must exist (404 would mean the route regressed) and accept {{\"to\":N}}, body {raw}"
    );
    let body: Value = serde_json::from_str(&raw).expect("400 body must be the handler's JSON error envelope");
    assert!(
        body.get("error").and_then(Value::as_str).is_some(),
        "400 must carry the handler's error field, body {raw}"
    );
    assert!(
        body["error"].as_str().unwrap_or_default().contains("Unknown version id: 424242"),
        "error must name the unknown version id, body {raw}"
    );
}

#[test]
fn admin_acl_team_mode_refuses_plain_http_at_boot() {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir("admin-acl-tls");
    std::fs::create_dir_all(&home_dir).expect("create temp home");
    let home = home_dir.to_string_lossy().to_string();
    let port = reserve_port();

    let output = Command::new(cortex_tests::cortex_bin())
        .args(["setup", "--team", "--owner", "owner-one"])
        .env("CORTEX_HOME", &home_dir)
        .env_remove("CORTEX_DB")
        .env_remove("CORTEX_PORT")
        .env_remove("CORTEX_BIND")
        .output()
        .expect("run cortex setup --team");
    assert!(
        output.status.success(),
        "cortex setup --team must succeed, status {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr),
    );
    let owner_key = read_token(&home_dir);
    assert!(
        owner_key.starts_with("ctx_") && owner_key.len() == 50,
        "setup --team must persist a 50-char ctx_ owner key, got {} chars",
        owner_key.len()
    );

    let mut daemon = spawn_daemon(&home, port);
    // Team mode without TLS must exit(1) before serving
    // (server/runtime.rs:89-96, TeamMode rejection is unconditional — even
    // loopback binds). Poll briefly for the exit and capture stderr.
    let deadline = Instant::now() + Duration::from_secs(10);
    let exited = loop {
        if let Some(status) = daemon.try_wait().expect("poll daemon") {
            break Some(status);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut stderr = String::new();
    if let Some(handle) = daemon.stderr.as_mut() {
        let _ = handle.read_to_string(&mut stderr);
    }
    match exited {
        Some(status) => {
            assert!(
                !status.success(),
                "team mode must refuse plain HTTP boot, exit status {status}\n{stderr}"
            );
            assert!(
                stderr.contains("Team mode requires valid TLS"),
                "boot refusal must cite the team-mode TLS requirement, stderr: {stderr}"
            );
        }
        None => {
            shutdown_daemon(port, &home_dir);
            wait_for_exit(&mut daemon, Duration::from_secs(10));
            panic!("team-mode daemon must not serve plain HTTP; it stayed up\n{stderr}");
        }
    }
}

#[test]
fn admin_acl_solo_mode_rejects_admin_surface() {
    let _guard = daemon_spawn_test_guard();
    let home_dir = unique_temp_dir("admin-acl-solo");
    std::fs::create_dir_all(&home_dir).expect("create temp home");
    let home = home_dir.to_string_lossy().to_string();
    let port = reserve_port();
    let mut daemon = spawn_daemon(&home, port);
    wait_for_health(port, &mut daemon);
    let token = read_token(&home_dir);

    // Solo runtime token authenticates (ensure_auth passes) but ensure_admin
    // has no team caller to resolve -> exact team-mode 403 envelope.
    let resp = request_json(port, "GET", "/admin/stats", Some(&token), None)
        .expect("solo token admin probe");
    assert_eq!(resp.status, 403, "solo runtime token on /admin/stats, body {}", resp.body);
    assert_eq!(resp.body, requires_team_mode(), "solo runtime token 403 envelope");

    // A well-formed ctx_ key in solo mode is not a team credential -> 401.
    let stray_ctx = generate_ctx_api_key();
    let resp = request_json(port, "GET", "/admin/stats", Some(&stray_ctx), None)
        .expect("ctx key in solo mode probe");
    assert_eq!(resp.status, 401, "ctx_ key in solo mode, body {}", resp.body);
    assert_eq!(resp.body, unauthorized(), "ctx_ key in solo mode 401 envelope");

    shutdown_daemon(port, &home_dir);
    wait_for_exit(&mut daemon, Duration::from_secs(10));
}
