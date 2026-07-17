// SPDX-License-Identifier: MIT
use super::*;

    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    fn unique_temp_dir(name: &str) -> PathBuf {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        std::env::temp_dir().join(format!("cortex_prompt_inject_{name}_{unique}"))
    }
    #[test]
    fn parse_args_supports_short_and_long_flags() {
        let args = vec![
            "--file".to_string(),
            "C:/tmp/system.txt".to_string(),
            "-a".to_string(),
            "codex".to_string(),
            "--budget".to_string(),
            "512".to_string(),
            "-w".to_string(),
        ];
        let parsed = parse_args(&args).expect("args should parse");
        assert_eq!(parsed.file_path, PathBuf::from("C:/tmp/system.txt"));
        assert_eq!(parsed.agent, "codex");
        assert_eq!(parsed.budget, 512);
        assert!(parsed.watch);
    }
    #[test]
    fn parse_args_requires_file() {
        let args = vec!["--agent".to_string(), "codex".to_string()];
        let err = parse_args(&args).expect_err("missing file should error");
        assert!(err.contains("Missing required --file <path>"));
    }
    #[test]
    fn parse_args_missing_agent_value_errors() {
        let args = vec!["--file".to_string(), "prompt.txt".to_string(), "--agent".to_string()];
        let err = parse_args(&args).expect_err("missing agent value should error");
        assert!(err.contains("Missing value for --agent"));
    }
    #[test]
    fn parse_args_invalid_budget_errors() {
        let args = vec!["--file".to_string(), "prompt.txt".to_string(), "--budget".to_string(), "not-a-number".to_string()];
        let err = parse_args(&args).expect_err("invalid budget should error");
        assert!(err.contains("Invalid --budget"));
    }
    #[test]
    fn parse_args_rejects_unknown_flags() {
        let args = vec!["--file".to_string(), "prompt.txt".to_string(), "--budegt".to_string(), "512".to_string()];
        let err = parse_args(&args).expect_err("unknown flag should error");
        assert!(err.contains("Unknown option: --budegt"));
        assert!(err.contains(USAGE));
    }
    #[test]
    fn compose_injected_prompt_appends_cortex_context() {
        let output = compose_injected_prompt("base prompt", "<!-- context -->");
        assert_eq!(output, "base prompt\n\n<!-- context -->");
    }
    #[test]
    fn file_modified_returns_zero_for_missing_path() {
        let path = PathBuf::from("__missing_prompt_inject_file__.txt");
        assert_eq!(file_modified(&path), 0);
    }
    #[test]
    fn output_path_appends_injected_suffix() {
        let path = PathBuf::from("C:/tmp/system.txt");
        let out = output_path_for(&path);
        assert_eq!(out, PathBuf::from("C:/tmp/system.txt.injected"));
        let dotfile = PathBuf::from("C:/tmp/.env");
        let dot_out = output_path_for(&dotfile);
        assert_eq!(dot_out, PathBuf::from("C:/tmp/.env.injected"));
    }
    #[test]
    fn read_auth_token_from_path_reads_trimmed_token() {
        let temp_home = unique_temp_dir("token");
        std::fs::create_dir_all(&temp_home).expect("create temp home");
        let token_path = temp_home.join("cortex.token");
        std::fs::write(&token_path, "ctx_prompt_token\n").expect("write token file");
        let token = read_auth_token_from_path(&token_path);
        assert_eq!(token.as_deref(), Some("ctx_prompt_token"));
        let _ = std::fs::remove_dir_all(&temp_home);
    }
