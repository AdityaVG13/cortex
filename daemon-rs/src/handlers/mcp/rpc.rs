use super::{mcp_tools,required_permission_for_tool,ClientPermission};use crate::handlers::{estimate_tokens,now_iso};use crate::
state::RuntimeState;use serde_json::{json,Value};use std::collections::BTreeMap;pub fn mcp_success(id:Value,result:Value)->Value{
json!({"jsonrpc":"2.0","id":id,"result":result})}pub fn mcp_error(id:Value,code:i64,message:&str)->Value{json!({"jsonrpc":"2.0",
"id":id,"error":{"code":code,"message":message}})}pub(crate)fn mcp_error_with_data(id:Value,code:i64,message:&str,data:Value)->
Value{json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message,"data":data}})}pub(crate)fn mcp_resource_uris()->Vec<&
'static str>{vec!["cortex://tooling/capabilities","cortex://tooling/tools"]}pub(crate)fn mcp_resources()->Vec<Value>{vec![json!({
"uri":"cortex://tooling/capabilities","name":"Cortex MCP capabilities","description":
"Read-only discovery summary of Cortex tool clusters, permission tiers, and next actions for agents.","mimeType":
"application/json"}),json!({"uri":"cortex://tooling/tools","name":"Cortex MCP tool catalog","description":
"Compact clustered catalog of advertised Cortex MCP tools with required args and permission tier.","mimeType":"application/json"})
,]}pub(crate)fn mcp_tool_cluster(tool_name:&str)->&'static str{match tool_name{"cortex_boot"|"cortex_boot_audit"|
"cortex_reconnect"=>"session","cortex_peek"|"cortex_recall"|"cortex_recall_policy_explain"|"cortex_semantic_recall"|
"cortex_unfold"=>"recall","cortex_store"|"cortex_forget"|"cortex_resolve"|"cortex_conflicts_list"|"cortex_conflicts_get"|
"cortex_conflicts_resolve"|"cortex_consensus_promote"|"cortex_memory_decay_run"|"cortex_eval_run"=>"memory-governance",
"cortex_focus_start"|"cortex_focus_end"|"cortex_focus_status"|"cortex_diary"=>"continuity","cortex_agent_feedback_record"|
"cortex_agent_feedback_stats"|"cortex_health"|"cortex_digest"|"cortex_lastCall"=>"observability","cortex_permissions_list"|
"cortex_permissions_grant"|"cortex_permissions_revoke"=>"admin",_=>"other",}}pub(crate)fn mcp_tool_permission(tool_name:&str)->&
'static str{required_permission_for_tool(tool_name).map(ClientPermission::as_str).unwrap_or("unknown")}pub(crate)fn
tooling_capabilities_payload()->Value{let mut clusters:BTreeMap<String,Vec<String>>=BTreeMap::new();let mut permissions:BTreeMap<
String,Vec<String>>=BTreeMap::new();for tool in mcp_tools(){if let Some(name)=tool.get("name").and_then(Value::as_str){clusters.
entry(mcp_tool_cluster(name).to_string()).or_default().push(name.to_string());permissions.entry(mcp_tool_permission(name).
to_string()).or_default().push(name.to_string());}}let tool_count=clusters.values().map(Vec::len).sum::<usize>();json!({"server":
"cortex","toolCount":tool_count,"clusters":clusters,"permissions":permissions,"resources":mcp_resource_uris(),"nextActions":[
"Call tools/list for full JSON schemas.","Read cortex://tooling/tools for a compact clustered tool catalog.",
"Use cortex_health before mutation-heavy workflows when daemon state is uncertain."]})}pub(crate)fn tooling_tools_payload()->Value
{let tools=mcp_tools().into_iter().filter_map(|tool|{let name=tool.get("name").and_then(Value::as_str)?.to_string();let
description=tool.get("description").and_then(Value::as_str).unwrap_or_default().to_string();let required=tool.pointer(
"/inputSchema/required").cloned().unwrap_or_else(||json!([]));let mut parameters=tool.pointer("/inputSchema/properties").and_then(
Value::as_object).map(|properties|properties.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();parameters.sort();Some(json!
({"name":name,"cluster":mcp_tool_cluster(&name),"permission":mcp_tool_permission(&name),"description":description,"required":
required,"parameters":parameters}))}).collect::<Vec<_>>();json!({"tools":tools,"discovery":{"fullSchemas":"Call tools/list.",
"capabilities":"Read cortex://tooling/capabilities.","health":"Call cortex_health to confirm daemon liveness and database state."}
,"commonMistakes":["Use exact tool names from this catalog; aliases are not accepted.",
"Use read tools before admin or mutation tools when you are unsure of current state.",
"Do not assume a write/admin tool is available in team mode without a caller-scoped API key."]})}pub(crate)fn mcp_resource_payload
(uri:&str)->Option<Value>{match uri{"cortex://tooling/capabilities"=>Some(tooling_capabilities_payload()),"cortex://tooling/tools"
=>Some(tooling_tools_payload()),_=>None,}}pub(crate)fn mcp_resource_read_result(uri:&str,payload:Value)->Value{json!({"contents":[
{"uri":uri,"mimeType":"application/json","text":payload.to_string()}]})}pub(crate)fn common_prefix_len(left:&str,right:&str)->
usize{left.chars().zip(right.chars()).take_while(|(left,right)|left==right).count()}pub(crate)fn tool_name_suggestions(provided:&
str)->Vec<String>{let needle=provided.trim().to_ascii_lowercase();if needle.is_empty(){return Vec::new();}let mut scored=mcp_tools
().into_iter().filter_map(|tool|{let name=tool.get("name").and_then(Value::as_str)?.to_string();let lower=name.to_ascii_lowercase(
);let short=lower.strip_prefix("cortex_").unwrap_or(&lower);let score=if lower==needle||short==needle{100}else if lower.
starts_with(&needle)||short.starts_with(&needle){90}else if lower.contains(&needle)||short.contains(&needle){80}else{let prefix=
common_prefix_len(&lower,&needle).max(common_prefix_len(short,&needle));if prefix>=4{50+prefix as i32}else{0}};(score>0).then_some
((score,name))}).collect::<Vec<_>>();scored.sort_by(|left,right|right.0.cmp(&left.0).then_with(||left.1.cmp(&right.1)));scored.
into_iter().take(5).map(|(_,name)|name).collect()}pub(crate)fn payload_token_usage(data:&Value)->(usize,Option<i64>,Option<usize>)
{match data{Value::Object(map)=>{let used=map.get("spent").and_then(|value|value.as_u64()).or_else(||map.get("tokenEstimate").
and_then(|value|value.as_u64())).or_else(||map.get("totalTokens").and_then(|value|value.as_u64())).or_else(||map.get("tokensUsed")
.and_then(|value|value.as_u64())).or_else(||map.get("savings").and_then(|value|value.get("served")).and_then(|value|value.as_u64()
)).unwrap_or_else(||estimate_tokens(&data.to_string())as u64)as usize;let saved=map.get("saved").and_then(|value|value.as_i64()).
or_else(||map.get("savings").and_then(|value|value.get("saved")).and_then(|value|value.as_i64()));let budget=map.get("budget").
and_then(|value|value.as_u64()).or_else(||map.get("policy").and_then(|value|value.get("budget")).and_then(|value|value.as_u64())).
map(|value|value as usize);(used,saved,budget)}_=>(estimate_tokens(&data.to_string()),None,None),}}pub(crate)fn token_usage_line(
used:usize,saved:Option<i64>,budget:Option<usize>)->String{match(saved,budget){(Some(saved),Some(budget))if saved>=0=>{format!(
"Token usage: used {used} tokens, saved {saved} of {budget}.")}(Some(saved),Some(budget))=>{format!(
"Token usage: used {used} tokens ({} over budget {budget}).",saved.abs())}(Some(saved),None)if saved>=0=>{format!(
"Token usage: used {used} tokens, saved {saved}.")}(Some(saved),None)=>{format!(
"Token usage: used {used} tokens ({} over budget).",saved.abs())}(None,Some(budget))=>format!(
"Token usage: used {used} tokens (budget {budget})."),(None,None)=>format!("Token usage: used {used} tokens."),}}pub(crate)fn
decorate_tool_payload_with_token_usage(data:Value)->Value{let(used,saved,budget)=payload_token_usage(&data);let line=
token_usage_line(used,saved,budget);match data{Value::Object(mut map)=>{map.entry("tokenUsage".to_string()).or_insert_with(||{json
!({"used":used,"saved":saved,"budget":budget})});map.entry("tokenUsageLine".to_string()).or_insert_with(||Value::String(line));
Value::Object(map)}other=>json!({"value":other,"tokenUsage":{"used":used,"saved":saved,"budget":budget},"tokenUsageLine":line}),}}
pub(crate)fn wrap_mcp_tool_result(_state:&RuntimeState,data:Value)->Value{let decorated=decorate_tool_payload_with_token_usage(
data);let text=match&decorated{Value::String(s)=>s.clone(),other=>other.to_string(),};json!({"content":[{"type":"text","text":text
}]})}pub(crate)fn wrap_mcp_tool_result_verbose(state:&RuntimeState,data:Value)->Value{let calls=state.next_mcp_call();let base=
decorate_tool_payload_with_token_usage(data);let decorated=match base{Value::Object(mut map)=>{map.insert("_liveness".to_string(),
Value::Bool(true));map.insert("_ts".to_string(),Value::String(now_iso()));map.insert("_calls".to_string(),Value::Number(calls.into
()));Value::Object(map)}other=>json!({"value":other,"_liveness":true,"_ts":now_iso(),"_calls":calls}),};json!({"content":[{"type":
"text","text":decorated.to_string()}]})}pub(crate)fn arg_str<'a>(args:&'a Value,keys:&[&str])->Option<&'a str>{keys.iter().
find_map(|key|args.get(*key).and_then(|value|value.as_str())).map(str::trim).filter(|value|!value.is_empty())}pub(crate)fn arg_f64
(args:&Value,keys:&[&str])->Option<f64>{keys.iter().find_map(|key|args.get(*key).and_then(|value|value.as_f64()))}pub(crate)fn
arg_i64(args:&Value,keys:&[&str])->Option<i64>{keys.iter().find_map(|key|args.get(*key).and_then(|value|value.as_i64()))}pub(crate
)fn arg_usize(args:&Value,keys:&[&str])->Option<usize>{keys.iter().find_map(|key|args.get(*key).and_then(|value|value.as_u64())).
map(|value|value as usize)}
