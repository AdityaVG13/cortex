//! MCP 2026-07-28 protocol surface: discovery, per-request eras, prompts,
//! completions, resource templates, and MRTR elicitation.
//!
//! Dual-era policy (spec `basic/versioning`): a request carrying modern
//! per-request `_meta` is served statelessly per this revision; `initialize`
//! always selects the byte-identical 2024-11-05 handshake; era-ambiguous
//! requests (no `_meta`) are served under legacy semantics. An explicit but
//! unsupported version is rejected with `UnsupportedProtocolVersionError`
//! (-32022), never silently downgraded.
//!
//! Clarification is stateless MRTR, not server-initiated requests: when a
//! `tools/call` fails on a flat missing field and the client declared
//! `elicitation.form`, the server answers `input_required` with a form
//! request. Retries are self-contained (`arguments` + `inputResponses`), so
//! no `requestState` is minted and there is nothing to integrity-protect or
//! replay. Anything else falls back to the plain `invalid_request` envelope.
//!
//! Transport boundary (honest, not aspirational): this handler answers one
//! JSON-RPC message per call and owns no server-to-client push channel, so
//! `subscriptions/listen` is answered as explicitly unsupported. Sampling,
//! roots, and logging are deprecated in this revision and unimplemented.
//! Long operations belong to the `io.modelcontextprotocol/tasks` extension
//! once a genuinely long MCP-triggered op exists; all current ops are
//! ms-scale, so the extension is not advertised.

use serde_json::{Value, json};

pub const LATEST_PROTOCOL_VERSION: &str = "2026-07-28";
pub const LEGACY_PROTOCOL_VERSION: &str = "2024-11-05";

pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

/// Catalog cache hint. Tool/prompt/resource catalogs are process-static pure
/// functions carrying no user data, so a long public TTL is honest.
pub const CATALOG_TTL_MS: u64 = 3_600_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestEra {
    Modern,
    Legacy,
}

/// Era for this request from its `_meta`. An explicit current version is
/// modern; a missing version (or an explicit legacy one) is legacy; any
/// other explicit version is rejected with the requested value so the caller
/// can answer `UnsupportedProtocolVersionError` (-32022).
pub fn request_era(msg: &Value) -> Result<RequestEra, String> {
    match msg.pointer("/params/_meta").and_then(|meta| meta.get(META_PROTOCOL_VERSION)).and_then(Value::as_str) {
        None | Some(LEGACY_PROTOCOL_VERSION) => Ok(RequestEra::Legacy),
        Some(LATEST_PROTOCOL_VERSION) => Ok(RequestEra::Modern),
        Some(other) => Err(other.to_string()),
    }
}

/// Per-request client capabilities. Missing or malformed `_meta` means no
/// declared capabilities; the server must never infer them from other
/// requests, so elicitation-gated behavior falls back to plain envelopes.
pub fn request_client_capabilities(msg: &Value) -> Value {
    msg.pointer("/params/_meta")
        .and_then(|meta| meta.get(META_CLIENT_CAPABILITIES))
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

/// Whether the declared capabilities accept form-mode elicitation carried as
/// MRTR `inputRequests` (there are no server-initiated requests in 2026-07-28).
pub fn client_supports_elicitation_form(caps: &Value) -> bool {
    caps.get("elicitation").and_then(|e| e.get("form")).is_some()
}

pub fn server_info() -> Value {
    json!({"name": "cortex", "version": env!("CARGO_PKG_VERSION")})
}

/// Modern result framing: required `resultType` plus the SHOULD-strength
/// server identity in the result `_meta`.
pub fn complete_result(mut result: Value) -> Value {
    if let Some(map) = result.as_object_mut() {
        map.insert("resultType".to_string(), json!("complete"));
        let mut meta = map.get("_meta").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
        meta[META_SERVER_INFO] = server_info();
        map.insert("_meta".to_string(), meta);
    }
    result
}

/// Cacheability framing for the catalog results (`tools/list`,
/// `prompts/list`, `resources/list`, `resources/templates/list`,
/// `resources/read`, `server/discover`).
pub fn cacheable_result(mut result: Value) -> Value {
    if let Some(map) = result.as_object_mut() {
        map.insert("ttlMs".to_string(), json!(CATALOG_TTL_MS));
        map.insert("cacheScope".to_string(), json!("public"));
    }
    result
}

/// Natural-language server guidance for the calling model. Complements tool
/// descriptions; states how this memory server wants to be used.
pub const SERVER_INSTRUCTIONS: &str = "Cortex is an evidence-admission memory brain, not a search engine. \
Call cortex_orient once when you start, then cortex_query with a profile; empty results (no_match) and \
ambiguous results are honest answers, not failures — narrow the need or pick a candidate instead of \
assuming. Expand Card aliases (m1, m2 …) with their receipt for exact sources; cite aliases, never \
invented ids. Deposit durable facts with cortex_commit (idempotency keys make retries safe) and report \
outcomes with cortex_feedback so usefulness statistics stay separate from truth.";

pub fn supported_versions() -> Vec<&'static str> {
    vec![LATEST_PROTOCOL_VERSION, LEGACY_PROTOCOL_VERSION]
}

pub fn server_capabilities() -> Value {
    json!({
        "tools": {},
        "resources": {},
        "prompts": {},
        "completions": {},
    })
}

/// `server/discover` is a modern-only method, so its result is always fully
/// modern: `resultType`, server identity, and cacheability, regardless of
/// whether the request carried `_meta`. The method implies the era.
pub fn discover_result() -> Value {
    cacheable_result(complete_result(json!({
        "supportedVersions": supported_versions(),
        "capabilities": server_capabilities(),
        "instructions": SERVER_INSTRUCTIONS,
    })))
}

pub fn mcp_resource_templates() -> Vec<Value> {
    vec![
        json!({"uriTemplate":"cortex://tooling/{doc}","name":"Cortex tooling docs","description":"Discovery documents: capabilities, tools.","mimeType":"application/json"}),
    ]
}

pub fn mcp_prompts() -> Vec<Value> {
    vec![
        json!({"name":"recall-briefing","description":"Start work with memory: orient on the task, then query with a profile. Returns the workflow as messages.","arguments":[{"name":"task","description":"What you are trying to do","required":true},{"name":"profile","description":"Lens profile for the query (answer, changes, attempts, procedures, conflicts, uncertainty, compare, history, audit, map)"},{"name":"thread","description":"Thread id or label (optional)"}]}),
        json!({"name":"capture-decision","description":"Store a durable decision with retention guidance and idempotency. Returns the commit workflow as messages.","arguments":[{"name":"decision","description":"The decision text","required":true},{"name":"context","description":"Supporting context"},{"name":"retention","description":"Retention class: durable, operational, audit, ephemeral"}]}),
        json!({"name":"session-recap","description":"Checkpoint thread state so a successor resumes from durable state, not transcript. Returns the checkpoint workflow as messages.","arguments":[{"name":"thread","description":"Thread id or label","required":true},{"name":"goal","description":"Current goal"}]}),
    ]
}

fn prompt_arg(args: &Value, name: &str) -> Option<String> {
    args.get(name).and_then(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Render a prompt's messages with arguments interpolated. Unknown prompt
/// names return `None`; missing required args are named in the message text
/// rather than failing, so the model can self-correct.
pub fn prompt_messages(name: &str, args: &Value) -> Option<Value> {
    let args = if args.is_object() { args } else { &json!({}) };
    match name {
        "recall-briefing" => {
            let Some(task) = prompt_arg(args, "task") else {
                return Some(prompt_text("recall-briefing needs a `task` argument: what are you trying to do?"));
            };
            let profile = prompt_arg(args, "profile").unwrap_or_else(|| "answer".into());
            let thread = prompt_arg(args, "thread").map(|t| format!(" in thread {t}")).unwrap_or_default();
            Some(prompt_text(&format!(
                "Brief yourself from memory before acting.\n1. Call cortex_orient with task {task:?}{thread}.\n2. Call cortex_query with need {task:?} and profile {profile:?}.\n3. Treat no_match/ambiguous as honest answers: narrow the need or pick a candidate.\n4. Expand Card aliases with their receipts for exact sources; cite aliases."
            )))
        }
        "capture-decision" => {
            let Some(decision) = prompt_arg(args, "decision") else {
                return Some(prompt_text("capture-decision needs a `decision` argument: the decision text."));
            };
            let context = prompt_arg(args, "context").map(|c| format!(" Context: {c}")).unwrap_or_default();
            let retention = prompt_arg(args, "retention").unwrap_or_else(|| "operational".into());
            Some(prompt_text(&format!(
                "Store this durably: {decision:?}.{context}\nCall cortex_commit with the decision, retention_class {retention:?}, and an idempotency key so retries are safe. Quote the receipt back."
            )))
        }
        "session-recap" => {
            let Some(thread) = prompt_arg(args, "thread") else {
                return Some(prompt_text("session-recap needs a `thread` argument: thread id or label."));
            };
            let goal = prompt_arg(args, "goal").map(|g| format!(" Goal: {g}")).unwrap_or_default();
            Some(prompt_text(&format!(
                "Checkpoint thread {thread:?} so a successor resumes from durable state.{goal}\nCall cortex_checkpoint action=status, then action=checkpoint with goal and state. Record open obligations and failed attempts explicitly."
            )))
        }
        _ => None,
    }
}

fn prompt_text(text: &str) -> Value {
    json!({"messages":[{"role":"user","content":{"type":"text","text":text}}]})
}

/// Argument completions. Real values only: Lens profiles for the
/// recall-briefing `profile` argument, tooling doc names for the
/// `cortex://tooling/{doc}` template variable. Everything else completes
/// empty (valid per spec), never invented.
pub fn complete_argument(ref_type: &str, ref_name: &str, arg_name: &str, value: &str) -> Vec<String> {
    let candidates: Vec<String> = match (ref_type, ref_name, arg_name) {
        ("ref/prompt", "recall-briefing", "profile") => cortex_kernel::lens::LensProfile::ALL.iter().map(|p| p.as_str().to_string()).collect(),
        ("ref/resource", uri, "doc") if uri == "cortex://tooling/{doc}" => {
            vec!["capabilities".into(), "tools".into()]
        }
        _ => Vec::new(),
    };
    let needle = value.to_ascii_lowercase();
    candidates.into_iter().filter(|c| c.to_ascii_lowercase().starts_with(&needle)).take(100).collect()
}

/// Flat `invalid_field` rejections the server can elicit as an MRTR form,
/// mapped to primitive schemas. Structured rejections (entries, evidence,
/// predicates-as-objects) stay plain envelopes — a flat form cannot express
/// them, and guessing would corrupt the call.
fn elicitable_schema(field: &str) -> Option<Value> {
    let (title, description): (&str, &str) = match field {
        "need" => ("Need", "The question or need"),
        "thread" => ("Thread", "Thread id or label"),
        "title" => ("Title", "Short title"),
        "obligation" => ("Obligation", "Obligation record id"),
        "record" => ("Record", "Record reference to resolve"),
        "rationale" => ("Rationale", "Why this resolution is correct"),
        "alias" => ("Alias", "Card alias such as m1"),
        "receipt" => ("Receipt", "Receipt id from the View (aliases are receipt-scoped)"),
        "predicate" => ("Predicate", "Checker predicate on the artifact"),
        "artifact" => ("Artifact", "Artifact the predicate checks"),
        _ => return None,
    };
    Some(json!({"type": "string", "title": title, "description": description}))
}

/// `action` is the one elicitable enum: an unknown checkpoint action becomes
/// a single-select over the known actions.
fn elicitable_action_schema() -> Value {
    json!({
        "type": "string",
        "title": "Checkpoint action",
        "description": "Which checkpoint operation to run",
        "enum": ["checkpoint", "status", "obligation", "transition", "verify", "revalidate", "attempt"],
    })
}

fn elicitation_property(field: &str) -> Option<Value> {
    if field == "action" { Some(elicitable_action_schema()) } else { elicitable_schema(field) }
}

/// Field + error of an `invalid_field` tool result that MRTR elicitation can
/// satisfy, if the client declared form support.
pub fn missing_field_of(result: &Value) -> Option<(String, String)> {
    let field = result.get("field")?.as_str()?;
    if result.get("status")?.as_str()? != "invalid_request" {
        return None;
    }
    elicitation_property(field)?;
    Some((field.to_string(), result.get("error").and_then(Value::as_str).unwrap_or("a required field is missing").to_string()))
}

/// Key for a field's elicitation round-trip. The `missing_` prefix namespaces
/// server-minted keys so unsolicited client keys never collide.
pub fn elicitation_key(field: &str) -> String {
    format!("missing_{field}")
}

/// `input_required` result asking for one flat field. No `requestState`: the
/// retry is self-contained (`arguments` plus `inputResponses`), so there is
/// no server context to protect, replay, or expire.
pub fn input_required_result(field: &str, message: &str) -> Value {
    let schema = elicitation_property(field).unwrap_or_else(|| json!({"type": "string"}));
    let mut properties = serde_json::Map::new();
    properties.insert(field.to_string(), schema);
    let mut requests = serde_json::Map::new();
    requests.insert(
        elicitation_key(field),
        json!({
            "method": "elicitation/create",
            "params": {
                "mode": "form",
                "message": message,
                "requestedSchema": {
                    "type": "object",
                    "properties": properties,
                    "required": [field],
                },
            },
        }),
    );
    let mut meta = serde_json::Map::new();
    meta.insert(META_SERVER_INFO.to_string(), server_info());
    json!({
        "resultType": "input_required",
        "inputRequests": requests,
        "_meta": meta,
    })
}

/// Outcome of folding a retry's `inputResponses` into the tool arguments.
pub enum RetryInput {
    /// Elicited values merged; `answered` names the fields the client
    /// supplied (used to bound re-prompts: a field answered this retry that
    /// still fails falls back to the plain envelope, never a second prompt).
    Merged { args: Value, answered: Vec<String> },
    /// The caller refused to supply the field; the call ends here.
    Declined { field: String },
}

/// Merge `inputResponses` into a copy of `args`. `Ok(None)` when the params
/// carry no retry input. `Err` is a protocol violation (malformed shape or a
/// value outside the requested primitive schema) for a -32602 answer.
/// Unknown keys and the never-minted `requestState` echo are ignored.
pub fn apply_input_responses(args: &Value, params: &Value) -> Result<Option<RetryInput>, String> {
    let Some(responses) = params.get("inputResponses") else {
        return Ok(None);
    };
    let responses = responses.as_object().ok_or_else(|| "inputResponses must be an object".to_string())?;
    let mut merged = args.clone();
    let mut answered = Vec::new();
    for (key, response) in responses {
        let Some(field) = key.strip_prefix("missing_") else { continue };
        if elicitation_property(field).is_none() {
            continue;
        }
        let response = response.as_object().ok_or_else(|| format!("input response `{key}` must be an object"))?;
        match response.get("action").and_then(Value::as_str) {
            Some("accept") => {}
            Some("decline") | Some("cancel") => return Ok(Some(RetryInput::Declined { field: field.to_string() })),
            other => return Err(format!("input response `{key}` has invalid action {other:?}; want accept, decline, or cancel")),
        }
        let content = response.get("content").and_then(Value::as_object);
        let Some(value) = content.and_then(|c| c.get(field)) else {
            // Accepted but empty: the re-dispatch reports the field missing
            // again and the server re-prompts (spec: missing info asks again).
            continue;
        };
        let text = value.as_str().ok_or_else(|| format!("elicited `{field}` must be a string"))?;
        if text.trim().is_empty() {
            continue;
        }
        if field == "action" && !["checkpoint", "status", "obligation", "transition", "verify", "revalidate", "attempt"].contains(&text) {
            return Err(format!("elicited `action` must be a known checkpoint action, got {text:?}"));
        }
        if !merged.is_object() {
            merged = json!({});
        }
        merged[field] = Value::String(text.to_string());
        answered.push(field.to_string());
    }
    Ok(Some(RetryInput::Merged { args: merged, answered }))
}
