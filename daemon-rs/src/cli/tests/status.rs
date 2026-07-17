// SPDX-License-Identifier: MIT
#[cfg(test)]
mod tests {
    use super::support::*;
    use crate::cli::*;
    use crate::*;
    #[test]
    fn cli_usage_exposes_agent_entrypoints() {
        let usage = cli_usage_text();
        assert!(usage.contains("cortex capabilities --json"));
        assert!(usage.contains("status [--json]"));
        assert!(usage.contains("cortex robot-docs guide"));
        assert!(usage.contains("Agent surfaces:"));
    }

    #[test]
    fn cli_capabilities_payload_has_agent_contract() {
        let payload = cli_capabilities_payload();
        assert_eq!(
            payload["contract_version"],
            CLI_CAPABILITIES_CONTRACT_VERSION
        );
        assert_eq!(
            payload["tool"]["default_port"].as_u64(),
            Some(DEFAULT_CORTEX_PORT as u64)
        );
        assert_eq!(payload["commands"]["status"]["side_effects"], "none");
        assert_eq!(payload["commands"]["status"]["output"], "human_or_json");
        assert_eq!(payload["commands"]["paths"]["output"], "json");
        assert_eq!(payload["exit_codes"]["0"], "success");
    }

    fn status_test_paths(name: &str) -> auth::CortexPaths {
        let home = temp_test_dir(name);
        let home_str = home.to_string_lossy().to_string();
        auth::CortexPaths::resolve_with_overrides(
            Some(&home_str),
            None,
            Some(7437),
            Some("127.0.0.1"),
        )
    }

    fn status_check<'a>(payload: &'a Value, name: &str) -> &'a Value {
        payload["checks"]
            .as_array()
            .expect("checks array")
            .iter()
            .find(|check| check["name"] == name)
            .unwrap_or_else(|| panic!("missing status check {name}"))
    }

    #[test]
    fn status_report_ready_json_has_schema_next_action_and_checks() {
        let paths = status_test_paths("status_ready");
        let report = build_status_report(
            &paths,
            StatusRuntimeProbe::Ready("Readiness endpoint reports ready.".to_string()),
            true,
            true,
        );

        assert_eq!(report.exit_code, 0);
        assert_eq!(report.payload["schemaVersion"], STATUS_SCHEMA_VERSION);
        assert_eq!(report.payload["status"], "ready");
        assert_eq!(
            report.payload["nextAction"]["kind"],
            "connect_tool_or_smoke"
        );
        assert_eq!(report.payload["repair"], Value::Null);
        assert_eq!(
            status_check(&report.payload, "runtime_identity")["status"],
            "ok"
        );
        assert_eq!(status_check(&report.payload, "auth_token")["status"], "ok");
    }

    #[test]
    fn status_report_unavailable_returns_repair_action_and_nonzero() {
        let paths = status_test_paths("status_unavailable");
        let report = build_status_report(
            &paths,
            StatusRuntimeProbe::Unavailable(
                "readiness failed: connection refused; health failed: connection refused"
                    .to_string(),
            ),
            true,
            false,
        );

        assert_eq!(report.exit_code, 1);
        assert_eq!(report.payload["status"], "needs_action");
        assert_eq!(report.payload["repair"]["kind"], "start_local_runtime");
        assert_eq!(report.payload["repair"]["command"], "cortex serve");
        assert_eq!(
            status_check(&report.payload, "runtime_identity")["repair"]["kind"],
            "start_local_runtime"
        );
    }

    #[test]
    fn status_report_wrong_identity_is_error_not_ready() {
        let paths = status_test_paths("status_wrong_identity");
        let report = build_status_report(
            &paths,
            StatusRuntimeProbe::WrongIdentity(
                "Health endpoint answered, but home/db/token paths do not match.".to_string(),
            ),
            true,
            true,
        );

        assert_eq!(report.exit_code, 1);
        assert_eq!(report.payload["status"], "error");
        assert_eq!(report.payload["repair"]["kind"], "repair_runtime_identity");
        assert_eq!(
            status_check(&report.payload, "runtime_identity")["status"],
            "fail"
        );
    }

    #[test]
    fn robot_docs_guide_is_paste_ready_for_agents() {
        let guide = cli_robot_docs_guide();
        assert!(guide.contains("cortex capabilities --json"));
        assert!(guide.contains("cortex status --json"));
        assert!(guide.contains("cortex boot --json"));
        assert!(guide.contains("Danger gates:"));
        assert!(guide.contains("Treat exit code 0 as success"));
    }

    #[test]
    fn unknown_command_message_suggests_likely_agent_surface() {
        let message = unknown_cli_command_message("capability");
        assert!(message.contains("Unknown command: capability"));
        assert!(message.contains("Did you mean: `cortex capabilities --json`?"));
        assert!(message.contains("cortex help"));
    }

    #[test]
    fn spawned_owner_parent_probe_child_process() {
        if std::env::var(SPAWN_PARENT_TEST_CHILD_ENV).ok().as_deref() != Some("1") {
            return;
        }
        std::thread::sleep(Duration::from_millis(300));
    }

    #[test]
}
