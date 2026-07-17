// SPDX-License-Identifier: MIT
use super::*;

    use super::*;
    use rusqlite::Connection;
    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::db::configure(&conn).unwrap();
        crate::db::initialize_schema(&conn).unwrap();
        conn
    }
    #[test]
    fn test_record_and_predict() {
        let conn = setup();
        let sources = vec!["source_a".to_string(), "source_b".to_string(), "source_c".to_string()];
        record(&conn, &sources).unwrap();
        record(&conn, &sources).unwrap(); // Second call increases counts
        let predictions = predict(&conn, &["source_a".to_string()], 5).unwrap();
        assert!(!predictions.is_empty());
        for p in &predictions {
            assert!(p["coScore"].as_i64().unwrap() > 0);
        }
    }
    #[test]
    fn test_predict_excludes_known_sources() {
        let conn = setup();
        let sources = vec!["source_a".to_string(), "source_b".to_string()];
        record(&conn, &sources).unwrap();
        let predictions = predict(&conn, &sources, 5).unwrap();
        for p in &predictions {
            let s = p["source"].as_str().unwrap();
            assert_ne!(s, "source_a");
            assert_ne!(s, "source_b");
        }
    }
    #[test]
    fn test_reset() {
        let conn = setup();
        let sources = vec!["source_a".to_string(), "source_b".to_string()];
        record(&conn, &sources).unwrap();
        reset(&conn).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM co_occurrence", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0);
    }
    #[test]
    fn test_record_fewer_than_two_sources_is_noop() {
        let conn = setup();
        record(&conn, &["only_one".to_string()]).unwrap();
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM co_occurrence", [], |row| row.get(0)).unwrap();
        assert_eq!(count, 0);
    }
    #[test]
    fn test_predict_empty_recalled_sources() {
        let conn = setup();
        let results = predict(&conn, &[], 5).unwrap();
        assert!(results.is_empty());
    }
