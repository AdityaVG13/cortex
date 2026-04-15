// SPDX-License-Identifier: MIT
use crate::auth::CortexPaths;

fn normalized_host(value: &str) -> String {
    value
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase()
}

pub fn local_http_base_url(paths: &CortexPaths) -> String {
    let bind = paths.bind.trim();
    let host = if bind.is_empty() || matches!(bind, "0.0.0.0" | "::" | "[::]") {
        "127.0.0.1"
    } else {
        bind
    };
    format!("http://{host}:{}", paths.port)
}

pub fn is_local_http_base_url(base_url: &str, paths: &CortexPaths) -> bool {
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    if url.port_or_known_default() != Some(paths.port) {
        return false;
    }
    let host_norm = normalized_host(host);
    let bind_norm = normalized_host(&paths.bind);
    matches!(host_norm.as_str(), "127.0.0.1" | "localhost" | "::1")
        || (!bind_norm.is_empty()
            && !matches!(bind_norm.as_str(), "0.0.0.0" | "::")
            && host_norm == bind_norm)
}
