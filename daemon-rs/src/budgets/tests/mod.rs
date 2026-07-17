// SPDX-License-Identifier: MIT
use super::*;

    use super::*;
    fn valid_config() -> &'static str {
        r#"
[defaults]
enabled = true
[endpoints.store]
limit = 120
window_seconds = 60
[endpoints.recall]
limit = 300
window_seconds = 60
[endpoints.boot]
limit = 60
window_seconds = 60
[endpoints.mcp]
limit = 240
window_seconds = 60
"#
    }
    #[test]
    fn missing_file_disables_budgets_without_error() {
        let path = std::env::temp_dir().join(format!("cortex-missing-budgets-{}.toml", uuid::Uuid::new_v4()));
        let status = BudgetConfigStatus::load_from_path(path);
        assert!(!status.config_loaded);
        assert!(!status.enabled());
        assert!(status.error.is_none());
        assert!(status.budget_for(BudgetEndpoint::Store).is_none());
    }
    #[test]
    fn valid_config_parses_all_endpoint_budgets() {
        let config = BudgetConfig::parse_toml_str(valid_config()).unwrap();
        assert!(config.enabled);
        assert_eq!(config.budget_for(BudgetEndpoint::Store), Some(EndpointBudget { limit: 120, window_seconds: 60 }));
        assert_eq!(config.budget_for(BudgetEndpoint::Recall), Some(EndpointBudget { limit: 300, window_seconds: 60 }));
        assert_eq!(config.budget_for(BudgetEndpoint::Boot), Some(EndpointBudget { limit: 60, window_seconds: 60 }));
        assert_eq!(config.budget_for(BudgetEndpoint::Mcp), Some(EndpointBudget { limit: 240, window_seconds: 60 }));
    }
    #[test]
    fn disabled_config_validates_but_does_not_enforce() {
        let status = BudgetConfigStatus::from_contents(
            PathBuf::from("budgets.toml"),
            r#"
[defaults]
enabled = false
[endpoints.recall]
limit = 1
window_seconds = 60
"#,
        );
        assert!(status.config_loaded);
        assert!(status.error.is_none());
        assert!(!status.enabled());
        assert!(status.budget_for(BudgetEndpoint::Recall).is_none());
    }
    #[test]
    fn health_json_uses_portable_budget_source_label() {
        let status = BudgetConfigStatus::from_contents(PathBuf::from("C:/cortex-test/testuser/.cortex/budgets.toml"), valid_config());
        let payload = status.to_health_json(0);
        assert_eq!(payload["source"], BUDGET_SOURCE);
    }
    #[test]
    fn missing_endpoint_is_unlimited_for_that_endpoint() {
        let config = BudgetConfig::parse_toml_str(
            r#"
[defaults]
enabled = true
[endpoints.store]
limit = 2
window_seconds = 60
"#,
        )
        .unwrap();
        assert!(config.budget_for(BudgetEndpoint::Recall).is_none());
    }
    #[test]
    fn zero_limit_is_structured_error() {
        let err = BudgetConfig::parse_toml_str(
            r#"
[endpoints.store]
limit = 0
window_seconds = 60
"#,
        )
        .unwrap_err();
        assert_eq!(err.code, "invalid_limit");
        assert_eq!(err.endpoint.as_deref(), Some("store"));
        assert_eq!(err.field.as_deref(), Some("limit"));
    }
    #[test]
    fn negative_limit_is_structured_error() {
        let err = BudgetConfig::parse_toml_str(
            r#"
[endpoints.store]
limit = -1
window_seconds = 60
"#,
        )
        .unwrap_err();
        assert_eq!(err.code, "invalid_limit");
    }
    #[test]
    fn zero_window_is_structured_error() {
        let err = BudgetConfig::parse_toml_str(
            r#"
[endpoints.recall]
limit = 1
window_seconds = 0
"#,
        )
        .unwrap_err();
        assert_eq!(err.code, "invalid_window_seconds");
        assert_eq!(err.endpoint.as_deref(), Some("recall"));
        assert_eq!(err.field.as_deref(), Some("window_seconds"));
    }
    #[test]
    fn negative_window_is_structured_error() {
        let err = BudgetConfig::parse_toml_str(
            r#"
[endpoints.recall]
limit = 1
window_seconds = -30
"#,
        )
        .unwrap_err();
        assert_eq!(err.code, "invalid_window_seconds");
    }
    #[test]
    fn unknown_endpoint_is_structured_error() {
        let err = BudgetConfig::parse_toml_str(
            r#"
[endpoints.search]
limit = 1
window_seconds = 60
"#,
        )
        .unwrap_err();
        assert_eq!(err.code, "unknown_endpoint");
        assert_eq!(err.endpoint.as_deref(), Some("search"));
    }
