use super::common::{
    admin_request, api_key_output_masked, confirm_action, format_api_key_for_output, json_field, json_str, json_str_or, parse_flag_value,
    required_cli_positional_or_exit, validate_cli_options_or_exit,
};
use crate::auth;
use crate::budgets;
use crate::db;
use serde_json::{json, Value};
pub(crate) fn run_admin_budgets_cli(paths: &auth::CortexPaths, args: &[String]) {
    let subcmd = args.first().map(String::as_str).unwrap_or("");
    let json_output = args.iter().any(|arg| arg == "--json");
    match subcmd {
        "status" => {
            validate_cli_options_or_exit(&args[1..], &[], &["--json"]);
            let status = budgets::BudgetConfigStatus::load_from_home(&paths.home);
            let payload = status.to_health_json(0);
            if json_output {
                println!("{}", serde_json::to_string_pretty(&payload).unwrap());
                return;
            }
            print_budget_status_human(&payload);
        }
        "validate" => {
            validate_cli_options_or_exit(&args[1..], &["--path"], &["--json"]);
            let Some(path) = parse_flag_value(args, "--path") else {
                eprintln!("Usage: cortex admin budgets validate --path <file> [--json]");
                std::process::exit(1);
            };
            let status = budgets::BudgetConfigStatus::load_from_path(path);
            let mut payload = status.to_health_json(0);
            if !status.config_loaded && status.error.is_none() {
                payload["error"] = json!({"code":"not_found","message":"budget config file was not found","endpoint":null,
"field":null});
            }
            if json_output {
                println!("{}", serde_json::to_string_pretty(&payload).unwrap());
            } else {
                print_budget_status_human(&payload);
            }
            if payload["error"].is_object() {
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("Usage: cortex admin budgets <status|validate --path <file>> [--json]");
            std::process::exit(1);
        }
    }
}
fn print_budget_status_human(payload: &Value) {
    println!("Cortex Budget Governance");
    println!("{}", "=".repeat(50));
    println!("Source: {}", json_str(payload, "source"));
    println!("Config loaded: {}", json_field(payload, "configLoaded"));
    println!("Enabled: {}", json_field(payload, "enabled"));
    if let Some(error) = payload.get("error").and_then(Value::as_object) {
        println!(
            "Error: {} ({})",
            error.get("message").and_then(Value::as_str).unwrap_or("unknown error"),
            error.get("code").and_then(Value::as_str).unwrap_or("unknown")
        );
        return;
    }
    if let Some(endpoints) = payload.get("endpoints").and_then(Value::as_object) {
        if endpoints.is_empty() {
            println!("Endpoints: unlimited");
            return;
        }
        println!();
        println!("{:<12} {:<10} WINDOW", "ENDPOINT", "LIMIT");
        println!("{}", "-".repeat(36));
        for (endpoint, budget) in endpoints {
            println!(
                "{:<12} {:<10} {}s",
                endpoint,
                budget.get("limit").and_then(Value::as_u64).unwrap_or(0),
                budget.get("windowSeconds").and_then(Value::as_u64).unwrap_or(0)
            );
        }
    }
}
pub(crate) fn run_admin_rollback_cli(paths: &auth::CortexPaths, args: &[String]) {
    validate_cli_options_or_exit(args, &["--session-id", "--session"], &["--apply", "--json", "--help", "-h"]);
    let mut session_id: Option<String> = None;
    let mut apply = false;
    let mut json_output = false;
    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--session-id" | "--session" => {
                if let Some(v) = args.get(i + 1) {
                    session_id = Some(v.clone());
                    i += 1;
                }
            }
            "--apply" => apply = true,
            "--json" => json_output = true,
            "--help" | "-h" => {
                println!(
                    "Usage: cortex admin rollback --session-id <id> [--apply] [--json]\n\
                     \n\
                     Soft-deletes every memory + decision written by the session's\n\
                     agent since the session started. Dry-run by default; pass\n\
                     --apply to write. Idempotent."
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown flag: {other}");
                eprintln!("Usage: cortex admin rollback --session-id <id> [--apply] [--json]");
                std::process::exit(1);
            }
        }
        i += 1;
    }
    let Some(session_id) = session_id else {
        eprintln!("Usage: cortex admin rollback --session-id <id> [--apply] [--json]");
        std::process::exit(1);
    };
    let conn = match db::open(&paths.db) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("Error: failed to open database for rollback: {err}");
            std::process::exit(1);
        }
    };
    if let Err(err) = db::configure(&conn) {
        eprintln!("Error: failed to configure database for rollback: {err}");
        std::process::exit(1);
    }
    if let Err(err) = db::initialize_schema(&conn) {
        eprintln!("Error: failed to initialize schema: {err}");
        std::process::exit(1);
    }
    db::run_pending_migrations(&conn);
    let stats = match crate::admin::rollback_session_by_id(&conn, &session_id, apply) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Error: rollback failed: {err}");
            std::process::exit(1);
        }
    };
    if apply && !stats.agent.is_empty() {
        let payload = json!({
"session_id":stats.session_id,"agent":stats.agent,"session_started_at":stats.session_started_at,"memories_affected":stats.
memories_affected,"decisions_affected":stats.decisions_affected,"already_rolled_back":stats.already_rolled_back,});
        let _ = conn.execute(
            "INSERT INTO events(type, data, source_agent) VALUES (?1, ?2, ?3)",
            rusqlite::params!["session.rolled_back", payload.to_string(), stats.agent,],
        );
    }
    if json_output {
        println!(
            "{}",
            json!({"rollback":true,"applied":stats.applied,"session_id":stats.
session_id,"agent":stats.agent,"session_started_at":stats.session_started_at,"memories_affected":stats.memories_affected,
"decisions_affected":stats.decisions_affected,"already_rolled_back":stats.already_rolled_back,})
        );
    } else if stats.agent.is_empty() {
        eprintln!(
            "Session not found: '{session_id}'. The sessions table is keyed\n\
             by agent + current session_id; expired / superseded sessions\n\
             cannot be rolled back by id alone."
        );
        std::process::exit(1);
    } else {
        let label = if stats.applied { "applied" } else { "dry-run" };
        println!("[rollback {label}] session={} agent={} started_at={}", stats.session_id, stats.agent, stats.session_started_at);
        println!("  memories to flip: {}    decisions to flip: {}", stats.memories_affected, stats.decisions_affected);
        if stats.already_rolled_back {
            println!("  note: session already rolled back previously; nothing to do.");
        }
        if !stats.applied {
            println!("  Dry-run only. Pass --apply to persist.");
        }
    }
    std::process::exit(0);
}
pub(crate) async fn run_user_cli(paths: &auth::CortexPaths, args: &[String]) {
    let subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("");
    match subcmd {
        "add" => {
            let username = required_cli_positional_or_exit(
                &args,
                3,
                "Usage: cortex user add <username> [--role member|admin] [--display-name \"...\"]",
            );
            validate_cli_options_or_exit(&args[4..], &["--role", "--display-name"], &[]);
            let mut role = "member".to_string();
            let mut display_name: Option<String> = None;
            let mut i = 4usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--role" => {
                        if let Some(v) = args.get(i + 1) {
                            role = v.clone();
                            i += 1;
                        }
                    }
                    "--display-name" => {
                        if let Some(v) = args.get(i + 1) {
                            display_name = Some(v.clone());
                            i += 1;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            let mut body = serde_json::json!({"username":username,"role":role,});
            if let Some(dn) = display_name {
                body["display_name"] = serde_json::json!(dn);
            }
            match admin_request(&paths, "POST", "/admin/user/add", Some(body)).await {
                Ok(json) => {
                    let api_key = json_str(&json, "api_key");
                    let key_masked = api_key_output_masked();
                    println!("User created:");
                    println!("  Username:  {}", json_str(&json, "username"));
                    println!("  User ID:   {}", json_field(&json, "user_id"));
                    println!("  Role:      {}", json_str(&json, "role"));
                    println!("  API Key:   {}", format_api_key_for_output(&api_key));
                    if key_masked {
                        println!("  NOTE: API key is masked because stdout is non-interactive.");
                        println!("        Re-run this command in a terminal to display the full key.");
                    }
                    println!();
                    println!("Save the API key -- it cannot be retrieved later.");
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        "rotate-key" => {
            let username = required_cli_positional_or_exit(&args, 3, "Usage: cortex user rotate-key <username>");
            validate_cli_options_or_exit(&args[4..], &[], &[]);
            let body = serde_json::json!({"username":username});
            match admin_request(&paths, "POST", "/admin/user/rotate-key", Some(body)).await {
                Ok(json) => {
                    let api_key = json_str(&json, "api_key");
                    let key_masked = api_key_output_masked();
                    println!("API key rotated for '{}':", json_str(&json, "username"));
                    println!("  New API Key: {}", format_api_key_for_output(&api_key));
                    if key_masked {
                        println!("  NOTE: API key is masked because stdout is non-interactive.");
                        println!("        Re-run this command in a terminal to display the full key.");
                    }
                    println!();
                    println!("Save the API key -- it cannot be retrieved later.");
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        "remove" => {
            let username = required_cli_positional_or_exit(&args, 3, "Usage: cortex user remove <username>");
            validate_cli_options_or_exit(&args[4..], &[], &[]);
            if !confirm_action(&format!("Remove user '{username}'?")) {
                eprintln!("Cancelled.");
                std::process::exit(0);
            }
            let body = serde_json::json!({"username":username});
            match admin_request(&paths, "POST", "/admin/user/remove", Some(body)).await {
                Ok(json) => {
                    println!("Removed user '{}'", json_str(&json, "removed"));
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        "list" => {
            validate_cli_options_or_exit(&args[3..], &[], &[]);
            match admin_request(&paths, "GET", "/admin/users", None).await {
                Ok(json) => {
                    let users = json["users"].as_array();
                    match users {
                        Some(arr) if !arr.is_empty() => {
                            println!("{:<6} {:<20} {:<20} {:<10} CREATED", "ID", "USERNAME", "DISPLAY NAME", "ROLE");
                            println!("{}", "-".repeat(80));
                            for u in arr {
                                println!(
                                    "{:<6} {:<20} {:<20} {:<10} {}",
                                    json_field(u, "id"),
                                    json_str(u, "username"),
                                    json_str_or(u, "display_name", "-"),
                                    json_str(u, "role"),
                                    json_str_or(u, "created_at", "-"),
                                );
                            }
                            println!();
                            println!("{} user(s)", arr.len());
                        }
                        _ => println!("No users found."),
                    }
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("Usage: cortex user <add|rotate-key|remove|list>");
            std::process::exit(1);
        }
    }
}
pub(crate) async fn run_team_cli(paths: &auth::CortexPaths, args: &[String]) {
    let subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("");
    match subcmd {
        "create" => {
            let name = required_cli_positional_or_exit(&args, 3, "Usage: cortex team create <name>");
            validate_cli_options_or_exit(&args[4..], &[], &[]);
            let body = serde_json::json!({"name":name});
            match admin_request(&paths, "POST", "/admin/team/create", Some(body)).await {
                Ok(json) => {
                    println!("Team created:");
                    println!("  Name:    {}", json_str(&json, "name"));
                    println!("  Team ID: {}", json_field(&json, "team_id"));
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        "add" => {
            let usage = "Usage: cortex team add <team> <username> [--role member|admin]";
            let team_name = required_cli_positional_or_exit(&args, 3, usage);
            let username = required_cli_positional_or_exit(&args, 4, usage);
            validate_cli_options_or_exit(&args[5..], &["--role"], &[]);
            let mut role = "member".to_string();
            let mut i = 5usize;
            while i < args.len() {
                if args[i] == "--role" {
                    if let Some(v) = args.get(i + 1) {
                        role = v.clone();
                        i += 1;
                    }
                }
                i += 1;
            }
            let body = serde_json::json!({"team_name":team_name,"username":username,"role":role,});
            match admin_request(&paths, "POST", "/admin/team/add-member", Some(body)).await {
                Ok(json) => {
                    println!(
                        "Added '{}' to team '{}' as {}",
                        json_str(&json, "username"),
                        json_str(&json, "team"),
                        json_str(&json, "role"),
                    );
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        "remove" => {
            let usage = "Usage: cortex team remove <team> <username>";
            let team_name = required_cli_positional_or_exit(&args, 3, usage);
            let username = required_cli_positional_or_exit(&args, 4, usage);
            validate_cli_options_or_exit(&args[5..], &[], &[]);
            if !confirm_action(&format!("Remove '{username}' from team '{team_name}'?")) {
                eprintln!("Cancelled.");
                std::process::exit(0);
            }
            let body = serde_json::json!({
"team_name":team_name,"username":username,});
            match admin_request(&paths, "POST", "/admin/team/remove-member", Some(body)).await {
                Ok(json) => {
                    let removed = &json["removed"];
                    println!("Removed '{}' from team '{}'", json_str(removed, "username"), json_str(removed, "team"),);
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        "list" => {
            validate_cli_options_or_exit(&args[3..], &[], &[]);
            match admin_request(&paths, "GET", "/admin/teams", None).await {
                Ok(json) => {
                    let teams = json["teams"].as_array();
                    match teams {
                        Some(arr) if !arr.is_empty() => {
                            println!("{:<6} {:<30} {:<10} CREATED", "ID", "NAME", "MEMBERS");
                            println!("{}", "-".repeat(70));
                            for t in arr {
                                println!(
                                    "{:<6} {:<30} {:<10} {}",
                                    json_field(t, "id"),
                                    json_str(t, "name"),
                                    json_field(t, "member_count"),
                                    json_str_or(t, "created_at", "-"),
                                );
                            }
                            println!();
                            println!("{} team(s)", arr.len());
                        }
                        _ => println!("No teams found."),
                    }
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        _ => {
            eprintln!("Usage: cortex team <create|add|remove|list>");
            std::process::exit(1);
        }
    }
}
pub(crate) async fn run_admin_cli(paths: &auth::CortexPaths, args: &[String]) {
    let subcmd = args.get(2).map(|s| s.as_str()).unwrap_or("");
    match subcmd {
        "list-unowned" => {
            validate_cli_options_or_exit(&args[3..], &[], &[]);
            match admin_request(&paths, "GET", "/admin/unowned", None).await {
                Ok(json) => {
                    let unowned = json["unowned"].as_object();
                    match unowned {
                        Some(map) if !map.is_empty() => {
                            println!("{:<25} UNOWNED ROWS", "TABLE");
                            println!("{}", "-".repeat(40));
                            let mut total: i64 = 0;
                            for (table, count) in map {
                                let n = count.as_i64().unwrap_or(0);
                                total += n;
                                println!("{:<25} {}", table, n);
                            }
                            println!("{}", "-".repeat(40));
                            println!("{:<25} {}", "TOTAL", total);
                        }
                        _ => println!("No unowned data found."),
                    }
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        "assign-owner" => {
            validate_cli_options_or_exit(&args[3..], &["--from", "--to", "--table"], &[]);
            let mut from_user: Option<String> = None;
            let mut to_user: Option<String> = None;
            let mut table: Option<String> = None;
            let mut i = 3usize;
            while i < args.len() {
                match args[i].as_str() {
                    "--from" => {
                        if let Some(v) = args.get(i + 1) {
                            from_user = Some(v.clone());
                            i += 1;
                        }
                    }
                    "--to" => {
                        if let Some(v) = args.get(i + 1) {
                            to_user = Some(v.clone());
                            i += 1;
                        }
                    }
                    "--table" => {
                        if let Some(v) = args.get(i + 1) {
                            table = Some(v.clone());
                            i += 1;
                        }
                    }
                    _ => {}
                }
                i += 1;
            }
            let Some(to) = to_user else {
                eprintln!("Usage: cortex admin assign-owner [--from <user>] --to <user> [--table <table>]");
                std::process::exit(1);
            };
            let mut body = serde_json
::json!({"to_user":to});
            if let Some(from) = from_user {
                body["from_user"] = serde_json::json!(from);
            }
            if let Some(t) = table {
                body["table"] = serde_json::json!(t);
            }
            match admin_request(&paths, "POST", "/admin/assign-owner", Some(body)).await {
                Ok(json) => {
                    let assigned = json["assigned"].as_object();
                    match assigned {
                        Some(map) if !map.is_empty() => {
                            println!("{:<25} ROWS ASSIGNED", "TABLE");
                            println!("{}", "-".repeat(40));
                            let mut total: i64 = 0;
                            for (tbl, count) in map {
                                let n = count.as_i64().unwrap_or(0);
                                total += n;
                                println!("{:<25} {}", tbl, n);
                            }
                            println!("{}", "-".repeat(40));
                            println!("{:<25} {}", "TOTAL", total);
                        }
                        _ => println!("No rows assigned."),
                    }
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        "stats" => {
            validate_cli_options_or_exit(&args[3..], &[], &[]);
            match admin_request(&paths, "GET", "/admin/stats", None).await {
                Ok(json) => {
                    println!("Cortex Admin Stats");
                    println!("{}", "=".repeat(50));
                    println!();
                    println!(
                        "Users: {}    Teams: {}    DB Size: {}",
                        json_field(&json, "user_count"),
                        json_field(&json, "team_count"),
                        json_str_or(&json, "db_size_mb", "?"),
                    );
                    println!();
                    if let Some(tables) = json["tables"].as_object() {
                        println!("{:<25} ROWS", "TABLE");
                        println!("{}", "-".repeat(40));
                        for (tbl, count) in tables {
                            println!("{:<25} {}", tbl, count);
                        }
                    }
                    if let Some(per_user) = json["per_user"].as_array() {
                        if !per_user.is_empty() {
                            println!();
                            println!("Per-User Breakdown:");
                            println!("  {:<20} {:<10} {:<10} CRYSTALS", "USERNAME", "MEMORIES", "DECISIONS");
                            println!("  {}", "-".repeat(55));
                            for u in per_user {
                                println!(
                                    "  {:<20} {:<10} {:<10} {}",
                                    json_str(u, "username"),
                                    json_field(u, "memories"),
                                    json_field(u, "decisions"),
                                    json_field(u, "crystals"),
                                );
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }
            }
        }
        "budgets" => {
            run_admin_budgets_cli(&paths, &args[3..]);
        }
        "rollback" => {
            run_admin_rollback_cli(&paths, &args[3..]);
        }
        _ => {
            eprintln!("Usage: cortex admin <list-unowned|assign-owner|stats|budgets|rollback>");
            std::process::exit(1);
        }
    }
}
