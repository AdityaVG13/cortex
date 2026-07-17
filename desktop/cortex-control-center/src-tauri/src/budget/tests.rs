use super::*;
use std::fs;
use std::path::Path;


    #[test]
    fn budget_editor_snapshot_parses_valid_config() {
        let snapshot = budget_snapshot_from_contents(
            Path::new("C:/cortex-test/testuser/.cortex/budgets.toml"),
            r#"
[defaults]
enabled = true

[endpoints.recall]
limit = 300
window_seconds = 60
"#,
        );

        assert!(snapshot.config_loaded);
        assert!(snapshot.enabled);
        assert_eq!(
            snapshot.error.as_ref().map(|error| error.code.as_str()),
            None
        );
        assert_eq!(snapshot.source, "budgets.toml");
        assert_eq!(snapshot.endpoints["recall"].limit, 300);
        assert_eq!(snapshot.endpoints["recall"].window_seconds, 60);
    }


    #[test]
    fn budget_editor_snapshot_returns_structured_errors() {
        let snapshot = budget_snapshot_from_contents(
            Path::new("C:/cortex-test/testuser/.cortex/budgets.toml"),
            r#"
[endpoints.unknown]
limit = 1
window_seconds = 60
"#,
        );

        let error = snapshot.error.expect("unknown endpoint should be invalid");
        assert_eq!(error.code, "unknown_endpoint");
        assert_eq!(error.endpoint.as_deref(), Some("unknown"));
        assert!(!snapshot.enabled);
    }


    #[test]
    fn budget_editor_draft_serializes_only_enabled_endpoints() {
        let config = validate_budget_draft(BudgetConfigDraft {
            enabled: true,
            endpoints: vec![
                BudgetEndpointDraft {
                    endpoint: "store".to_string(),
                    enabled: false,
                    limit: Some(120),
                    window_seconds: Some(60),
                },
                BudgetEndpointDraft {
                    endpoint: "recall".to_string(),
                    enabled: true,
                    limit: Some(42),
                    window_seconds: Some(15),
                },
            ],
        })
        .expect("draft should validate");

        assert!(config.defaults.enabled);
        assert_eq!(config.endpoints.len(), 1);
        assert_eq!(config.endpoints["recall"].limit, 42);
    }


    #[test]
    fn budget_editor_rejects_duplicate_or_invalid_endpoint_drafts() {
        let duplicate = validate_budget_draft(BudgetConfigDraft {
            enabled: true,
            endpoints: vec![
                BudgetEndpointDraft {
                    endpoint: "recall".to_string(),
                    enabled: true,
                    limit: Some(1),
                    window_seconds: Some(60),
                },
                BudgetEndpointDraft {
                    endpoint: "recall".to_string(),
                    enabled: true,
                    limit: Some(2),
                    window_seconds: Some(60),
                },
            ],
        })
        .unwrap_err();
        assert!(duplicate.contains("Duplicate budget endpoint"));

        let invalid = validate_budget_draft(BudgetConfigDraft {
            enabled: true,
            endpoints: vec![BudgetEndpointDraft {
                endpoint: "recall".to_string(),
                enabled: true,
                limit: Some(0),
                window_seconds: Some(60),
            }],
        })
        .unwrap_err();
        assert!(invalid.contains("limit must be between 1"));
    }


    #[test]
    fn budget_editor_write_replaces_file_atomically() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("cortex-budget-editor-{unique}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("budgets.toml");

        write_budget_config_file(&path, "[defaults]\nenabled = true\n")
            .expect("write initial file");
        write_budget_config_file(&path, "[defaults]\nenabled = false\n")
            .expect("replace existing file");

        let contents = fs::read_to_string(&path).expect("read replaced file");
        assert!(contents.contains("enabled = false"));
        let _ = fs::remove_dir_all(&dir);
    }
