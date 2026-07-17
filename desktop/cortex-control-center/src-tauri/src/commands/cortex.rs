use crate::cortex_http::{send_cortex_request, FetchCortexResponse};

#[tauri::command]
pub async fn fetch_cortex(
    path: String,
    auth_token: String,
    timeout_ms: Option<u64>,
) -> Result<FetchCortexResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        send_cortex_request("GET", &path, &auth_token, None, timeout_ms)
    })
    .await
    .map_err(|err| format!("fetch_cortex task failed: {err}"))?
}

#[tauri::command]
pub async fn post_cortex(
    path: String,
    auth_token: String,
    body: String,
    timeout_ms: Option<u64>,
) -> Result<FetchCortexResponse, String> {
    tauri::async_runtime::spawn_blocking(move || {
        send_cortex_request("POST", &path, &auth_token, Some(&body), timeout_ms)
    })
    .await
    .map_err(|err| format!("post_cortex task failed: {err}"))?
}
