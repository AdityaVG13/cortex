//! JSON-RPC / MCP argument readers.
//!
//! Clients send integers as i64, u64, whole floats, or decimal strings.
//! `Value::as_i64()` alone dropped `"7"` / `7.0` and silently applied defaults.

use super::nonempty_str;
use serde_json::Value;

/// Host payload aliases for the caller working directory.
pub const CWD_KEYS: &[&str] = &["cwd", "cwd_path", "working_directory"];

pub fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|n| i64::try_from(n).ok()))
        .or_else(|| {
            value
                .as_f64()
                .and_then(|x| (x.is_finite() && x.fract() == 0.0).then_some(x as i64))
        })
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
}

pub fn arg_str<'a>(args: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|k| args.get(*k).and_then(Value::as_str))
        .and_then(nonempty_str)
}

pub fn arg_i64(args: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|k| args.get(*k).and_then(json_i64))
}

pub fn arg_usize(args: &Value, keys: &[&str]) -> Option<usize> {
    arg_i64(args, keys).and_then(|n| usize::try_from(n).ok())
}

pub fn arg_bool(args: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|k| args.get(*k).and_then(Value::as_bool))
}

pub fn json_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|n| n as f64))
        .or_else(|| value.as_u64().map(|n| n as f64))
        .or_else(|| value.as_str().and_then(|s| s.trim().parse().ok()))
        .filter(|x| x.is_finite())
}

pub fn arg_f64(args: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| args.get(*k).and_then(json_f64))
}

pub fn json_str<'a>(value: &'a Value, key: &str, fallback: &'a str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or(fallback)
}

pub fn arg_list(args: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|k| args.get(*k))
        .map(|v| match v {
            Value::Array(items) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            Value::String(s) => s
                .split(',')
                .filter_map(nonempty_str)
                .map(str::to_string)
                .collect(),
            _ => Vec::new(),
        })
        .unwrap_or_default()
}
