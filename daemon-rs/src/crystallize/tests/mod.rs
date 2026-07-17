// SPDX-License-Identifier: MIT
use super::*;
#[test]
fn test_jaccard_identical() {
    assert!((jaccard_words("hello world test", "hello world test") - 1.0).abs() < 0.001);
}
#[test]
fn test_jaccard_different() {
    let sim = jaccard_words("the quick brown fox", "lazy purple elephant jumps");
    assert!(sim < 0.2);
}
#[test]
fn test_jaccard_partial() {
    let sim = jaccard_words("use python for backend", "use python for frontend");
    assert!(sim > 0.5, "Shared 'use python for' should give >0.5");
}
#[test]
fn test_compute_centroid() {
    let v1 = [1.0, 0.0, 0.0];
    let v2 = [0.0, 1.0, 0.0];
    let centroid = compute_centroid(&[&v1[..], &v2[..]]);
    assert!(centroid[0] > 0.6 && centroid[0] < 0.8);
    assert!(centroid[1] > 0.6 && centroid[1] < 0.8);
    assert!(centroid[2].abs() < 0.001);
}
#[test]
fn test_generate_label_empty() {
    let entries: Vec<&EmbeddedEntry> = vec![];
    assert_eq!(generate_cluster_label(&entries), "misc");
}
#[test]
fn test_synthesize_deduplicates() {
    let e1 = EmbeddedEntry { target_type: "memory".to_string(), target_id: 1, vector: vec![], source: "test1".to_string(), text: "Always use uv for Python package management. Never use pip directly.".to_string() };
    let e2 = EmbeddedEntry { target_type: "memory".to_string(), target_id: 2, vector: vec![], source: "test2".to_string(), text: "Use uv for Python package management instead of pip.".to_string() };
    let e3 = EmbeddedEntry { target_type: "memory".to_string(), target_id: 3, vector: vec![], source: "test3".to_string(), text: "Python type hints required on all function signatures.".to_string() };
    let result = synthesize_crystal(&vec![&e1, &e2, &e3]);
    assert!(result.matches('.').count() <= 3, "Should deduplicate similar sentences, got: {result}");
}
#[test]
fn test_cluster_entries_basic() {
    let make_entry = |id: i64, vec: Vec<f32>| EmbeddedEntry { target_type: "memory".to_string(), target_id: id, vector: vec, source: format!("test::{id}"), text: format!("Entry {id}") };
    let entries = vec![
        make_entry(1, vec![1.0, 0.0, 0.0]), make_entry(2, vec![0.98, 0.1, 0.0]), make_entry(3, vec![0.95, 0.15, 0.0]),
        make_entry(4, vec![0.0, 1.0, 0.0]), make_entry(5, vec![0.1, 0.98, 0.0]), make_entry(6, vec![0.15, 0.95, 0.0]),
    ];
    assert_eq!(cluster_entries(&entries).len(), 2, "Should find 2 clusters");
}
#[test]
fn test_full_crystallize_pass() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    crate::db::configure(&conn).unwrap();
    crate::db::initialize_schema(&conn).unwrap();
    migrate_crystal_tables(&conn);
    for i in 1..=4 {
        conn.execute("INSERT INTO memories (id, text, source, type, status) VALUES (?1, ?2, ?3, 'memory', 'active')", params![i, format!("Python requires uv for package management rule {i}"), format!("test::python_{i}")]).unwrap();
        let mut vec = vec![0.0f32; 384];
        vec[0] = 1.0;
        vec[1] = 0.01 * i as f32;
        conn.execute("INSERT INTO embeddings (target_type, target_id, vector) VALUES ('memory', ?1, ?2)", params![i, embeddings::vector_to_blob(&vec)]).unwrap();
    }
    let result = run_crystallize_pass_with_brain(&conn, None, None, &None);
    assert_eq!(result.clusters_found, 1);
    assert_eq!(result.crystals_created, 1);
    assert_eq!(result.entries_consolidated, 4);
    let crystals = list_crystals(&conn);
    assert_eq!(crystals.len(), 1);
    assert!(crystals[0]["members"].as_i64().unwrap() >= 4);
}
