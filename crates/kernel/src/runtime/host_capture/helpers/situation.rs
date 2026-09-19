use serde_json::Value;

const SITUATION_MAX_DEPTH: usize = 8;
const SITUATION_MAX_PARTS: usize = 32;

fn collect_situation(value: &Value, out: &mut Vec<String>) {
    collect_situation_at(value, out, 0);
}

fn collect_situation_at(value: &Value, out: &mut Vec<String>, depth: usize) {
    if depth > SITUATION_MAX_DEPTH || out.len() >= SITUATION_MAX_PARTS {
        return;
    }
    let Some(object) = value.as_object() else {
        return;
    };
    for (key, child) in object {
        if out.len() >= SITUATION_MAX_PARTS {
            return;
        }
        match key.to_ascii_lowercase().as_str() {
            "filepath" | "file_path" | "path" | "stdout" | "stderr" | "content" | "newstring"
            | "oldstring" | "new_string" | "old_string" | "command" | "prompt" => {
                if let Some(text) = child.as_str() {
                    out.push(text.to_string());
                }
            }
            "file" | "tool_input" | "tool_response" => collect_situation_at(child, out, depth + 1),
            "edits" => {
                if let Some(items) = child.as_array() {
                    for item in items.iter().take(SITUATION_MAX_PARTS) {
                        collect_situation_at(item, out, depth + 1);
                        if out.len() >= SITUATION_MAX_PARTS {
                            return;
                        }
                    }
                }
            }
            _ => {}
        }
    }
}
pub(in crate::runtime::host_capture) fn situation_cues(text: &str) -> Vec<String> {
    let tokens = if let Ok(value) = serde_json::from_str::<Value>(text) {
        let mut parts = Vec::new();
        collect_situation(&value, &mut parts);
        parts.join("\n")
    } else {
        text.to_string()
    };
    crate::handlers::split_alnum_keep(&tokens, |c| matches!(c, '_' | '.' | '/'))
        .filter(|s| s.len() <= 128)
        .map(str::to_lowercase)
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .take(32)
        .collect()
}
