use crate::auth;

use super::common::{admin_request, parse_flag_value, required_cli_positional_or_exit, validate_cli_options_or_exit};

fn fail(usage: &str) -> ! {
    eprintln!("{usage}");
    std::process::exit(1);
}

fn print_daemon_error(result: Result<serde_json::Value, String>) {
    match result {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())),
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    }
}

pub async fn run_user_cli(paths: &auth::CortexPaths, args: &[String]) {
    match args.get(2).map(String::as_str).unwrap_or("") {
        "list" => {
            validate_cli_options_or_exit(&args[3..], &[], &[]);
            print_daemon_error(admin_request(paths, "GET", "/admin/users", None).await);
        }
        "add" => {
            let name = required_cli_positional_or_exit(args, 3, "Usage: cortex user add <name> [--role member|admin] [--display-name <name>]");
            validate_cli_options_or_exit(&args[4..], &["--role", "--display-name"], &[]);
            let role = parse_flag_value(&args[4..], "--role");
            let display_name = parse_flag_value(&args[4..], "--display-name");
            // Wire contract: UserAddBody (handlers/admin/types.rs:21) deserializes
            // `username` plus optional `role`/`display_name`; the handler defaults
            // role to "member", so parsed flags must be forwarded verbatim or the
            // CLI silently creates a plain member (cortex-xw3).
            print_daemon_error(admin_request(paths, "POST", "/admin/user/add", Some(serde_json::json!({"username":name,"role":role,"display_name":display_name}))).await);
        }
        "rotate-key" => {
            let name = required_cli_positional_or_exit(args, 3, "Usage: cortex user rotate-key <name>");
            validate_cli_options_or_exit(&args[4..], &[], &[]);
            print_daemon_error(admin_request(paths, "POST", "/admin/user/rotate-key", Some(serde_json::json!({"username":name}))).await);
        }
        "remove" => {
            let name = required_cli_positional_or_exit(args, 3, "Usage: cortex user remove <name>");
            validate_cli_options_or_exit(&args[4..], &[], &[]);
            print_daemon_error(admin_request(paths, "POST", "/admin/user/remove", Some(serde_json::json!({"username":name}))).await);
        }
        _ => fail("Usage: cortex user <add|rotate-key|remove|list>"),
    }
}

pub async fn run_team_cli(paths: &auth::CortexPaths, args: &[String]) {
    match args.get(2).map(String::as_str).unwrap_or("") {
        "list" => {
            validate_cli_options_or_exit(&args[3..], &[], &[]);
            print_daemon_error(admin_request(paths, "GET", "/admin/teams", None).await);
        }
        "create" => {
            let team = required_cli_positional_or_exit(args, 3, "Usage: cortex team create <name>");
            validate_cli_options_or_exit(&args[4..], &[], &[]);
            // Wire contract: TeamCreateBody (handlers/admin/types.rs:31) deserializes
            // the field `name`; anything else dies in the Json extractor as 422.
            print_daemon_error(admin_request(paths, "POST", "/admin/team/create", Some(serde_json::json!({"name":team}))).await);
        }
        "add" => {
            let team = required_cli_positional_or_exit(args, 3, "Usage: cortex team add <team> <user> [--role member|admin]");
            let user = required_cli_positional_or_exit(args, 4, "Usage: cortex team add <team> <user> [--role member|admin]");
            validate_cli_options_or_exit(&args[5..], &["--role"], &[]);
            let role = parse_flag_value(&args[5..], "--role");
            // Wire contract: TeamMemberBody (handlers/admin/types.rs:35) deserializes
            // the fields `team_name` + `username` (+ optional `role`); the legacy
            // `team` key died in the Json extractor as 422 before the handler ran
            // (cortex-xw3).
            print_daemon_error(admin_request(paths, "POST", "/admin/team/add-member", Some(serde_json::json!({"team_name":team,"username":user,"role":role}))).await);
        }
        "remove" => {
            let team = required_cli_positional_or_exit(args, 3, "Usage: cortex team remove <team> <user>");
            let user = required_cli_positional_or_exit(args, 4, "Usage: cortex team remove <team> <user>");
            validate_cli_options_or_exit(&args[5..], &[], &[]);
            // Wire contract: TeamRemoveMemberBody (handlers/admin/types.rs:41)
            // deserializes `team_name` + `username`; the legacy `team` key died in
            // the Json extractor as 422 before the handler ran (cortex-xw3).
            print_daemon_error(admin_request(paths, "POST", "/admin/team/remove-member", Some(serde_json::json!({"team_name":team,"username":user}))).await);
        }
        _ => fail("Usage: cortex team <create|add|remove|list>"),
    }
}

pub async fn run_admin_cli(paths: &auth::CortexPaths, args: &[String]) {
    match args.get(2).map(String::as_str).unwrap_or("") {
        "list-unowned" => {
            validate_cli_options_or_exit(&args[3..], &[], &[]);
            print_daemon_error(admin_request(paths, "GET", "/admin/unowned", None).await);
        }
        "assign-owner" => {
            validate_cli_options_or_exit(&args[3..], &["--from", "--to", "--table"], &[]);
            let to_user = match parse_flag_value(&args[3..], "--to") {
                Some(to_user) => to_user,
                None => fail("Usage: cortex admin assign-owner --to <user> [--from <user>] [--table <table>]"),
            };
            let from_user = parse_flag_value(&args[3..], "--from");
            let table = parse_flag_value(&args[3..], "--table");
            // Wire contract: AssignOwnerBody (handlers/admin/types.rs:46) requires
            // `to_user`; `from_user`/`table` are optional. The previous empty `{}`
            // body died in the Json extractor as 422 before the handler ran
            // (cortex-xw3).
            print_daemon_error(admin_request(paths, "POST", "/admin/assign-owner", Some(serde_json::json!({"to_user":to_user,"from_user":from_user,"table":table}))).await);
        }
        "stats" => {
            validate_cli_options_or_exit(&args[3..], &[], &[]);
            print_daemon_error(admin_request(paths, "GET", "/admin/stats", None).await);
        }
        // Wire truth (cortex-04h companion): no /admin/budgets/* route exists in
        // server/router.rs and BudgetEndpoint is only used for MCP rate limiting
        // (router.rs:172), so both subcommands 404 today. Building the surface vs
        // removing these subcommands is tracked in
        // cortex-admin-budgets-surface-missing-crm (removal needs Rule-1 approval).
        "budgets" => match args.get(3).map(String::as_str).unwrap_or("") {
            "status" => {
                validate_cli_options_or_exit(&args[4..], &[], &["--json"]);
                print_daemon_error(admin_request(paths, "GET", "/admin/budgets/status", None).await);
            }
            "validate" => {
                validate_cli_options_or_exit(&args[4..], &["--path"], &["--json"]);
                print_daemon_error(admin_request(paths, "POST", "/admin/budgets/validate", Some(serde_json::json!({}))).await);
            }
            _ => fail("Usage: cortex admin budgets <status|validate> [--json]"),
        },
        "rollback" => {
            validate_cli_options_or_exit(&args[3..], &["--session-id"], &["--apply", "--json"]);
            let session_id = match parse_flag_value(&args[3..], "--session-id") {
                Some(value) => value,
                None => fail("Usage: cortex admin rollback --session-id <id> [--apply] [--json]"),
            };
            // Wire contract (cortex-04h): the rollback route is TOP-LEVEL
            // /rollback (server/router.rs:59, handle_rollback), not
            // /admin/rollback — the old path 404'd — and RollbackRequest
            // (server/handlers.rs:91) requires the i64 field `to`, a
            // versions.id consumed by traces::rollback_to. Forward it as a
            // number; a non-numeric value is rejected here instead of as an
            // opaque extractor 422. `--apply` is accepted but has NO wire
            // field (RollbackRequest deserializes only `to`): no dry-run
            // semantics exist on the wire, so the flag is currently
            // accepted-and-ignored (cortex-04h close note).
            let to: i64 = match session_id.parse() {
                Ok(value) => value,
                Err(_) => fail("--session-id must be an integer versions.id"),
            };
            print_daemon_error(admin_request(paths, "POST", "/rollback", Some(serde_json::json!({"to": to}))).await);
        }
        _ => fail("Usage: cortex admin <list-unowned|assign-owner|stats|budgets|rollback>"),
    }
}
