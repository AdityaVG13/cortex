use super::common::{
    open_cli_connection, parse_flag_usize, parse_flag_value, validate_cli_options_or_exit,
};
use crate::auth;
use crate::eval;
use serde_json::{json, Value};
pub(crate) fn run_eval_cli(paths: &auth::CortexPaths, args: &[String]) {
    validate_cli_options_or_exit(
        args,
        &["--baseline-file", "--max-regression", "--window-days"],
        &["--json", "--fail-on-regression"],
    );
    let json_output = args.iter().any(|arg| arg == "--json");
    let fail_on_regression = args.iter().any(|arg| arg == "--fail-on-regression");
    let baseline_file = parse_flag_value(args, "--baseline-file");
    let max_regression = match parse_flag_value(args, "--max-regression") {
        Some(raw) => {
            let parsed = raw
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("invalid value for --max-regression: '{raw}'"))
                .unwrap_or_else(|err| {
                    eprintln!("{err}");
                    std::process::exit(1);
                });
            if !(0.0..=1.0).contains(&parsed) {
                eprintln!("--max-regression must be between 0.0 and 1.0");
                std::process::exit(1);
            }
            parsed
        }
        None => 0.10,
    };
    let window_days = match parse_flag_usize(args, "--window-days") {
        Ok(Some(value)) => value.min(180) as i64,
        Ok(None) => 30,
        Err(err) => {
            eprintln!("Invalid --window-days value: {err}");
            std::process::exit(1);
        }
    };
    let conn = match open_cli_connection(&paths.db) {
        Ok(conn) => conn,
        Err(err) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
    };
    let mut snapshot = eval::build_eval_snapshot(&conn, window_days);
    let regression_gate = baseline_file.as_deref().map(|path| {
        let baseline_raw = std::fs::read_to_string(path).unwrap_or_else(|err| {
            eprintln!("Failed to read baseline snapshot file '{path}': {err}");
            std::process::exit(1);
        });
        let baseline_json: Value = serde_json::from_str(&baseline_raw).unwrap_or_else(|err| {
            eprintln!("Invalid baseline snapshot JSON in '{path}': {err}");
            std::process::exit(1);
        });
        eval::build_eval_regression_gate(&snapshot, &baseline_json, max_regression)
    });
    if let (Some(gate), Value::Object(map)) = (regression_gate.clone(), &mut snapshot) {
        map.insert("regressionGate".to_string(), gate);
    }
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&snapshot).unwrap_or_else(|_| "{}".to_string())
        );
        if fail_on_regression
            && regression_gate
                .as_ref()
                .and_then(|gate| gate.get("ok"))
                .and_then(Value::as_bool)
                == Some(false)
        {
            std::process::exit(2);
        }
        return;
    }
    let totals = snapshot.get("totals").cloned().unwrap_or_else(|| json!({}));
    let window = snapshot.get("window").cloned().unwrap_or_else(|| json!({}));
    let signals = snapshot
        .get("signals")
        .cloned()
        .unwrap_or_else(|| json!({}));
    println!("Eval snapshot ({window_days}d)");
    println!(
        "active: memories={}, decisions={}, open_conflicts={}",
        totals
            .get("activeMemories")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        totals
            .get("activeDecisions")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        totals
            .get("openConflicts")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0)
    );
    println!(
        "window: conflicts={}, resolutions={}, recalls={}",
        window
            .get("recentConflicts")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        window
            .get("recentResolutions")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0),
        window
            .get("recentRecallQueries")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0)
    );
    println!(
"signals: conflict_burden={:.4}, decay_burden={:.4}, resolution_velocity={:.4}, contradiction_rate={:.4}",signals.get(
"conflictBurden").and_then(serde_json::Value::as_f64).unwrap_or(0.0),signals.get("decayBurden").and_then(serde_json::Value::as_f64
).unwrap_or(0.0),signals.get("resolutionVelocity").and_then(serde_json::Value::as_f64).unwrap_or(0.0),signals.get(
"contradictionRate").and_then(serde_json::Value::as_f64).unwrap_or(0.0));
    println!(
        "task: success_rate={:.4}, first_pass={:.4}, median_time_ms={:.2}, retry_count={:.4}",
        signals
            .get("taskSuccessRate")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0),
        signals
            .get("firstPassSuccess")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0),
        signals
            .get("medianTimeToValidResultMs")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0),
        signals
            .get("retryCount")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0)
    );
    println!(
        "memory_quality: stale_hit_rate={:.4}, low_trust_hit_rate={:.4}, consensus_precision={:.4}",
        signals
            .get("staleMemoryHitRate")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0),
        signals
            .get("lowTrustHitRate")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0),
        signals
            .get("consensusPromotionPrecision")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0)
    );
    if let Some(gate) = regression_gate {
        let gate_ok = gate.get("ok").and_then(Value::as_bool).unwrap_or(true);
        println!(
            "regression_gate: ok={}, max_regression={:.3}",
            gate_ok, max_regression
        );
        if fail_on_regression && !gate_ok {
            std::process::exit(2);
        }
    }
}
