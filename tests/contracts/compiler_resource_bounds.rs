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

use cortex_daemon::compiler;
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
    let lines: Vec<&str> = section.lines().filter(|line| line.starts_with("- [")).collect();
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

