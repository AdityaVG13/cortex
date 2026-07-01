// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::support::*;
    use super::super::*;

    #[test]
    fn search_memories_excludes_temporally_invalid_rows() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO memories (text, type, source, status, expires_at, created_at, updated_at)
             VALUES ('expired memory', 'note', 'expired-memory', 'active', datetime('now', '-1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memories (text, type, source, status, expires_at, created_at, updated_at)
             VALUES ('active memory', 'note', 'active-memory', 'active', datetime('now', '+1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();

        let results = search_memories(&conn, "", 10, None).unwrap();
        let sources: Vec<&str> = results.iter().map(|item| item.source.as_str()).collect();
        assert!(sources.contains(&"active-memory"));
        assert!(!sources.contains(&"expired-memory"));
    }

    #[test]
    fn search_decisions_excludes_temporally_invalid_rows() {
        let conn = test_conn();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at, created_at, updated_at)
             VALUES ('expired decision', 'expired-decision', 'active', datetime('now', '-1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO decisions (decision, context, status, expires_at, created_at, updated_at)
             VALUES ('active decision', 'active-decision', 'active', datetime('now', '+1 hour'), datetime('now'), datetime('now'))",
            [],
        )
        .unwrap();

        let results = search_decisions(&conn, "", 10, None).unwrap();
        let sources: Vec<&str> = results.iter().map(|item| item.source.as_str()).collect();
        assert!(sources.contains(&"active-decision"));
        assert!(!sources.contains(&"expired-decision"));
    }

    #[test]
    fn store_then_keyword_recall_ranks_expected_entry_first() {
        let mut conn = test_conn();
        store_decision_with_embedding(
            &mut conn,
            "Truncate write_buffer.jsonl after buffered entries flush into SQLite.",
            "decision::write-buffer",
            &[0.0, 0.0, 0.0, 0.0, 1.0],
        );

        let results =
            run_budget_recall(&mut conn, "write buffer", 400, 5, &solo_ctx(), None).unwrap();
        assert_eq!(results[0].source, "decision::write-buffer");
    }

    #[test]
    fn is_visible_team_private_hidden_from_other() {
        let ctx = team_ctx(2);
        assert!(!is_visible(Some(1), Some("private"), &ctx));
    }

    #[test]
    fn is_visible_team_shared_visible_to_other() {
        let ctx = team_ctx(2);
        assert!(is_visible(Some(1), Some("shared"), &ctx));
    }

    #[test]
    fn recall_scopes_are_owner_isolated_in_team_mode() {
        let a = team_ctx(101);
        let b = team_ctx(202);
        assert_ne!(recall_scope_key("codex", &a), recall_scope_key("codex", &b));
    }
}
