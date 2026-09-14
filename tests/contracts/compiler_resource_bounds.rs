// B8 resource/boundary contracts (compiler capsules bucket).
//
// Failure-first provenance:
// - boot_pending_messages_capsule_is_bounded was RED before the pending
//   messages query gained a LIMIT: every message ever present for the
//   recipient was rendered into the boot capsule (25 lines vs the capped 10).
// - boot_unread_feed_capsule_pins_newest_ten_after_ack is an equivalence
//   guard for the bounded feed-scan rewrite: the old code full-scanned the
//   (unpruned) feed table per boot but already capped its output at the last
//   10 unread entries, so this pins the observable semantics the rewrite
//   must preserve (newest 10 unread, chronological order, positional ack).
//
// Note: a symlink-cycle indexer test was drafted here and REMOVED — probing
// showed index_directory recursion over a directory symlink cycle is already
// bounded by the kernel (each level appends a path component, hitting
// MAXSYMLINKS/ELOOP at ~32), so the test passed before any fix and was a
// tautology. No indexer change was needed.

#[path = "../support/mod.rs"]
mod support;

use cortex_kernel::compiler;
use cortex_tests::support::test_conn;
use tempfile::Builder;

const AGENT: &str = "b8-agent";

fn ts(i: usize) -> String {
    format!("2026-01-01T00:00:{:02}Z", i)
}

fn temp_home() -> std::path::PathBuf {
    Builder::new()
        .prefix("b8-resource-")
        .tempdir()
        .expect("temp home")
        .keep()
}

/// Extracts one `\n\n`-delimited capsule section (header inclusive).
fn section_between(prompt: &str, header: &str) -> String {
    let start = prompt
        .find(header)
        .unwrap_or_else(|| panic!("boot prompt must contain {header:?}: {prompt}"));
    let rest = &prompt[start..];
    let end = rest.find("\n\n").unwrap_or(rest.len());
    rest[..end].to_string()
}

#[test]
fn boot_pending_messages_capsule_is_bounded() {
    let conn = test_conn();
    for i in 0..25 {
        conn.execute(
            "INSERT INTO messages (id, sender, recipient, message, timestamp) \
             VALUES (?1, 'peer', ?2, ?3, ?4)",
            rusqlite::params![
                format!("msg-{i:02}"),
                AGENT,
                format!("message {i:02}"),
                ts(i)
            ],
        )
        .expect("insert message");
    }
    let home = temp_home();
    let result = compiler::compile(&conn, &home, AGENT, 4000);
    let section = section_between(&result.boot_prompt, "## Pending Messages");
    let lines: Vec<&str> = section
        .lines()
        .filter(|line| line.starts_with("- From "))
        .collect();
    assert!(
        lines.len() <= 10,
        "pending messages capsule must be bounded to the newest messages, got {} lines: {section}",
        lines.len()
    );
    for i in 15..25 {
        assert!(
            section.contains(&format!("message {i:02}")),
            "newest message {i:02} must be rendered: {section}"
        );
    }
    assert!(
        !section.contains("message 14"),
        "messages older than the newest ten must not be rendered: {section}"
    );
}

#[test]
fn boot_unread_feed_capsule_pins_newest_ten_after_ack() {
    let conn = test_conn();
    for i in 0..30 {
        conn.execute(
            "INSERT INTO feed (id, agent, kind, summary, timestamp) \
             VALUES (?1, 'worker', 'status', ?2, ?3)",
            rusqlite::params![format!("feed-{i:02}"), format!("entry {i:02}"), ts(i)],
        )
        .expect("insert feed entry");
    }
    conn.execute(
        "INSERT INTO feed_acks (agent, last_seen_id, updated_at) \
         VALUES (?1, 'feed-05', ?2)",
        rusqlite::params![AGENT, ts(5)],
    )
    .expect("insert feed ack");
    let home = temp_home();
    let result = compiler::compile(&conn, &home, AGENT, 4000);
    let section = section_between(&result.boot_prompt, "## Feed");
    let lines: Vec<&str> = section
        .lines()
        .filter(|line| line.starts_with("- ["))
        .collect();
    assert_eq!(
        lines.len(),
        10,
        "feed capsule must render exactly the newest ten unread entries: {section}"
    );
    for (slot, i) in (20..30).enumerate() {
        assert!(
            lines[slot].contains(&format!("entry {i:02}")),
            "feed capsule slot {slot} must be the chronologically newest unread entry {}: {section}",
            20 + slot
        );
    }
}

#[test]
fn stored_max_timestamp_is_chronological_across_rfc3339_and_sqlite_datetime() {
    let conn = test_conn();
    conn.execute(
        "INSERT INTO memories (text, type, source_agent, status, created_at, updated_at, observed_at, valid_from) \
         VALUES ('earlier rfc', 'fact', 'a', 'active', \
                 '2026-09-13T10:00:00.000Z', '2026-09-13T10:00:00.000Z', \
                 '2026-09-13T10:00:00.000Z', '2026-09-13T10:00:00.000Z')",
        [],
    )
    .expect("insert rfc3339 memory");
    conn.execute(
        "INSERT INTO memories (text, type, source_agent, status, created_at, updated_at, observed_at, valid_from) \
         VALUES ('later sqlite', 'fact', 'a', 'active', \
                 '2026-09-13 12:00:00', '2026-09-13 12:00:00', \
                 '2026-09-13 12:00:00', '2026-09-13 12:00:00')",
        [],
    )
    .expect("insert sqlite-datetime memory");
    let max = compiler::stored_max_timestamp(&conn).expect("max timestamp");
    assert!(
        max.contains("12:00:00"),
        "later space-format updated_at must win over earlier same-day RFC3339; MAX(text) would pick the T-format stamp: {max}"
    );
    assert!(
        !max.contains("10:00:00"),
        "earlier RFC3339 updated_at must not be selected as newest: {max}"
    );
}

#[test]
fn boot_own_session_is_not_listed_as_another_agent() {
    let conn = test_conn();
    let far = "2099-01-01T00:00:00Z";
    let started = ts(1);
    conn.execute(
        "INSERT INTO sessions (agent, session_id, project, files_json, description, started_at, last_heartbeat, expires_at) \
         VALUES (?1, 's-self', 'mine', '[]', 'self session', ?2, ?2, ?3)",
        rusqlite::params!["B8-Agent", started, far],
    )
    .expect("insert same-agent session with different case");
    conn.execute(
        "INSERT INTO sessions (agent, session_id, project, files_json, description, started_at, last_heartbeat, expires_at) \
         VALUES (?1, 's-self-model', 'mine', '[]', 'self with model', ?2, ?2, ?3)",
        rusqlite::params!["b8-agent (opus)", started, far],
    )
    .expect("insert same-agent session with model suffix");
    conn.execute(
        "INSERT INTO sessions (agent, session_id, project, files_json, description, started_at, last_heartbeat, expires_at) \
         VALUES (?1, 's-peer', 'theirs', '[]', 'peer session', ?2, ?2, ?3)",
        rusqlite::params!["peer-agent", started, far],
    )
    .expect("insert other-agent session");
    let home = temp_home();
    let result = compiler::compile(&conn, &home, AGENT, 4000);
    let section = section_between(&result.boot_prompt, "## Active Agents");
    assert!(
        section.contains("peer-agent"),
        "other agents must still appear: {section}"
    );
    assert!(
        !section.contains("B8-Agent") && !section.contains("b8-agent (opus)"),
        "the booting agent must not be listed as another agent: {section}"
    );
}

#[test]
fn boot_identity_includes_model_suffixed_mail_and_claimed_tasks() {
    let conn = test_conn();
    conn.execute(
        "INSERT INTO messages (id, sender, recipient, message, timestamp) \
         VALUES ('msg-self', 'peer', 'b8-agent (opus)', 'hello from peer', ?1)",
        rusqlite::params![ts(1)],
    )
    .expect("insert message to model-suffixed recipient");
    for i in 0..12 {
        conn.execute(
            "INSERT INTO messages (id, sender, recipient, message, timestamp) \
             VALUES (?1, 'peer', 'other-agent', ?2, ?3)",
            rusqlite::params![format!("msg-other-{i:02}"), format!("other {i:02}"), ts(10 + i)],
        )
        .expect("insert newer mail for another recipient");
    }
    conn.execute(
        "INSERT INTO tasks (task_id, title, files_json, priority, required_capability, status, claimed_by, created_at, claimed_at) \
         VALUES ('t-self', 'finish hunt', '[]', 'high', 'any', 'claimed', 'B8-Agent (opus)', ?1, ?1)",
        rusqlite::params![ts(1)],
    )
    .expect("insert claimed task under model-suffixed agent");
    conn.execute(
        "INSERT INTO events (type, data, source_agent, created_at) \
         VALUES ('agent_boot', '{}', 'b8-agent (opus)', ?1)",
        rusqlite::params![ts(2)],
    )
    .expect("insert last boot under model suffix");
    conn.execute(
        "INSERT INTO feed (id, agent, kind, summary, timestamp) \
         VALUES ('feed-self', 'worker', 'status', 'after ack', ?1)",
        rusqlite::params![ts(4)],
    )
    .expect("insert feed after ack");
    conn.execute(
        "INSERT INTO feed_acks (agent, last_seen_id, updated_at) \
         VALUES ('b8-agent (opus)', 'feed-self', ?1)",
        rusqlite::params![ts(3)],
    )
    .expect("insert ack under model suffix");
    conn.execute(
        "INSERT INTO focus_sessions (label, agent, status, raw_entries, started_at) \
         VALUES ('suffixed-hunt', 'b8-agent (opus)', 'open', '[]', ?1)",
        rusqlite::params![ts(1)],
    )
    .expect("insert open focus under model suffix");
    let home = temp_home();
    let result = compiler::compile(&conn, &home, AGENT, 4000);
    assert!(
        result.boot_prompt.contains("hello from peer"),
        "mail to `b8-agent (opus)` must appear when booting as `{AGENT}`: {}",
        result.boot_prompt
    );
    assert!(
        result.boot_prompt.contains("finish hunt"),
        "a task claimed by `B8-Agent (opus)` must appear as the booter's work: {}",
        result.boot_prompt
    );
    assert!(
        !result.boot_prompt.contains("after ack"),
        "a feed ack stored as `b8-agent (opus)` must still hide that row on boot as `{AGENT}`: {}",
        result.boot_prompt
    );
    assert!(
        result.boot_prompt.contains("suffixed-hunt"),
        "open focus stored as `b8-agent (opus)` must appear when booting as `{AGENT}`: {}",
        result.boot_prompt
    );
}

#[test]
fn boot_shows_open_focus_when_raw_entries_are_corrupt() {
    let conn = test_conn();
    conn.execute(
        "INSERT INTO focus_sessions (label, agent, status, raw_entries, started_at) \
         VALUES ('hunt', ?1, 'open', '{not-json', ?2)",
        rusqlite::params![AGENT, ts(1)],
    )
    .expect("insert corrupt open focus");
    let home = temp_home();
    let result = compiler::compile(&conn, &home, AGENT, 4000);
    assert!(
        result.boot_prompt.contains("## Active Focus"),
        "open focus must still render when raw_entries is not a JSON array: {}",
        result.boot_prompt
    );
    assert!(
        result.boot_prompt.contains("hunt"),
        "corrupt open focus must keep its label in the boot capsule: {}",
        result.boot_prompt
    );
}
