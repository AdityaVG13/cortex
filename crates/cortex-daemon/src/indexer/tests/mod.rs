// SPDX-License-Identifier: MIT
use super::*;

use super::*;
use rusqlite::Connection;
#[test]
fn index_all_empty_home_indexes_nothing() {
    let tmp = std::env::temp_dir().join(format!("cortex_ix_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    let conn = Connection::open_in_memory().unwrap();
    crate::db::initialize_schema(&conn).unwrap();
    let n = index_all(&conn, tmp.as_path(), None);
    assert_eq!(n, 0);
    let _ = std::fs::remove_dir_all(&tmp);
}
#[test]
fn matches_glob_works() {
    assert!(super::matches_glob(Path::new("foo.md"), "*.md"));
    assert!(!super::matches_glob(Path::new("foo.rs"), "*.md"));
    assert!(super::matches_glob(Path::new("anything"), "*"));
    assert!(super::matches_glob(Path::new("data.jsonl"), "*.jsonl"));
}
#[test]
fn expand_tilde_resolves_home() {
    let p = super::expand_tilde("~/test/path");
    assert!(p.components().count() > 2);
    assert!(!p.to_string_lossy().contains('~'));
}
#[test]
fn index_custom_sources_from_toml() {
    let tmp = std::env::temp_dir().join(format!("cortex_cs_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let cortex_dir = tmp.join(".cortex");
    std::fs::create_dir_all(&cortex_dir).unwrap();
    let notes_dir = tmp.join("test-notes");
    std::fs::create_dir_all(&notes_dir).unwrap();
    std::fs::write(notes_dir.join("alpha.md"), "Alpha note content").unwrap();
    std::fs::write(notes_dir.join("beta.md"), "Beta note content").unwrap();
    std::fs::write(notes_dir.join("ignore.txt"), "Should be skipped").unwrap();
    let single_file = tmp.join("single.json");
    std::fs::write(&single_file, r#"{"key": "value"}"#).unwrap();
    let toml_content = format!(
        r#"
[[source]]
name = "notes"
path = "{}"
mem_type = "note"
glob = "*.md"
[[source]]
name = "config"
path = "{}"
mem_type = "config"
"#,
        notes_dir.to_string_lossy().replace('\\', "/"),
        single_file.to_string_lossy().replace('\\', "/"),
    );
    std::fs::write(cortex_dir.join("sources.toml"), &toml_content).unwrap();
    let conn = Connection::open_in_memory().unwrap();
    crate::db::initialize_schema(&conn).unwrap();
    let n = super::index_custom_sources(&conn, &tmp, None);
    assert_eq!(n, 3, "expected 3 indexed entries (2 md + 1 json)");
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM memories WHERE source LIKE 'notes::%'", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 2, "expected 2 note memories");
    let count: i64 = conn.query_row("SELECT COUNT(*) FROM memories WHERE source LIKE 'config::%'", [], |r| r.get(0)).unwrap();
    assert_eq!(count, 1, "expected 1 config memory");
    let mem_type: String = conn.query_row("SELECT type FROM memories WHERE source LIKE 'notes::%' LIMIT 1", [], |r| r.get(0)).unwrap();
    assert_eq!(mem_type, "note");
    let _ = std::fs::remove_dir_all(&tmp);
}
#[test]
fn index_custom_sources_truncate() {
    let tmp = std::env::temp_dir().join(format!("cortex_tr_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let cortex_dir = tmp.join(".cortex");
    std::fs::create_dir_all(&cortex_dir).unwrap();
    let docs_dir = tmp.join("docs");
    std::fs::create_dir_all(&docs_dir).unwrap();
    std::fs::write(docs_dir.join("long.md"), "A".repeat(5000)).unwrap();
    let toml_content = format!(
        r#"
[[source]]
name = "docs"
path = "{}"
mem_type = "doc"
glob = "*.md"
truncate = 100
"#,
        docs_dir.to_string_lossy().replace('\\', "/"),
    );
    std::fs::write(cortex_dir.join("sources.toml"), &toml_content).unwrap();
    let conn = Connection::open_in_memory().unwrap();
    crate::db::initialize_schema(&conn).unwrap();
    let n = super::index_custom_sources(&conn, &tmp, None);
    assert_eq!(n, 1);
    let text: String = conn.query_row("SELECT text FROM memories WHERE source = 'docs::long'", [], |r| r.get(0)).unwrap();
    assert_eq!(text.len(), 100, "text should be truncated to 100 chars");
    let _ = std::fs::remove_dir_all(&tmp);
}
