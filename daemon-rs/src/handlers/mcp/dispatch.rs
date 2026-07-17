use super::{arg_f64,arg_i64,arg_str,arg_usize,clear_served_scope_for_boot,enforce_client_permission,fetch_last_call,
normalize_permission_client_id,parse_client_permission,refresh_mcp_session_presence,source_agent_for_tool,
source_client_for_permissions,source_model_for_tool,upsert_mcp_session,McpPresenceDisposition,};use crate::api_types::
RetentionClass;use crate::handlers::diary::{write_diary_entry,DiaryRequest};use crate::handlers::feedback::{
build_agent_feedback_stats_payload,recommend_recall_k,record_agent_feedback_from_value};use crate::handlers::health::{build_digest
,build_health_payload};use crate::handlers::mutate::{forget_keyword_scoped,list_conflicts_payload,parse_conflict_id,
resolve_decision,resolve_decision_with_metadata,ConflictListOptions,ConflictStatusFilter,ResolutionMetadata,};use crate::handlers
::recall::{execute_recall_policy_explain,execute_semantic_recall,execute_unified_recall,parse_recall_policy_mode,
resolve_recall_budget_k,unfold_source,RecallContext};use crate::handlers::store::{persist_decision_embedding,
store_decision_with_input_embedding_and_provenance_retention,validate_explicit_ttl_seconds,DecisionProvenance};use crate::handlers
::{estimate_tokens,SourceIdentity};use crate::state::RuntimeState;use crate::{aging,db,indexer};use serde_json::{json,Value};use
std::time::Instant;pub(crate)async fn mcp_dispatch(state:&RuntimeState,caller_id:Option<i64>,tool_name:&str,args:&Value,source:
Option<&SourceIdentity>)->Result<Value,String>{if state.team_mode&&caller_id.is_none(){return Err(
"Team mode MCP calls require a caller-scoped ctx_ API key".to_string());}enforce_client_permission(state,caller_id,tool_name,args,
source).await?;match tool_name{"cortex_boot"=>{let profile=args.get("profile").and_then(|v|v.as_str()).map(str::to_string);let
raw_agent=arg_str(args,&["agent","source_agent"]).map(str::to_string).unwrap_or_else(||source_agent_for_tool(source,"mcp"));let
model=source_model_for_tool(source,args);let budget=args.get("budget").and_then(|v|v.as_u64()).unwrap_or(600)as usize;let
profile_str=profile.unwrap_or_else(||"full".to_string());let(agent,_expires_at)=upsert_mcp_session(state,caller_id,&raw_agent,
model,"MCP boot session").await?;let ctx=RecallContext::from_caller(caller_id,state);clear_served_scope_for_boot(state,&agent,&ctx
).await;let conn=state.db.lock().await;let boot_started=Instant::now();let result=crate::compiler::compile(&conn,&state.home,&
agent,budget);crate::handlers::boot::record_boot_audit_best_effort(&conn,&agent,&profile_str,budget,&result,boot_started.elapsed()
.as_millis()as i64);if let Ok(latest_id)=conn.query_row("SELECT id FROM feed ORDER BY timestamp DESC LIMIT 1",[],|row|row.get::<_,
String>(0)){if state.team_mode{if let Some(owner_id)=ctx.caller_id{let _=conn.execute(
"INSERT INTO feed_acks (owner_id, agent, last_seen_id, updated_at) VALUES (?1, ?2, ?3, datetime('now')) \
                             ON CONFLICT(owner_id, agent) DO UPDATE SET last_seen_id = excluded.last_seen_id, updated_at = excluded.updated_at"
,rusqlite::params![owner_id,agent,latest_id],);}}else{let _=conn.execute(
"INSERT INTO feed_acks (agent, last_seen_id, updated_at) VALUES (?1, ?2, datetime('now')) \
                         ON CONFLICT(agent) DO UPDATE SET last_seen_id = excluded.last_seen_id, updated_at = excluded.updated_at"
,rusqlite::params![agent,latest_id],);}}crate::db::checkpoint_wal_best_effort(&conn);state.emit("session",json!({"action":
"started","agent":agent.clone()}));state.emit("agent_boot",json!({"agent":agent.clone(),"profile":profile_str.clone()}));let saved
=result.savings.get("saved").and_then(|value|value.as_i64()).unwrap_or(0);Ok(json!({"bootPrompt":result.boot_prompt,
"tokenEstimate":result.token_estimate,"profile":if profile_str=="full"{"capsules"}else{&profile_str},"capsules":result.capsules,
"savings":result.savings,"tokenUsage":{"used":result.token_estimate,"saved":saved,"budget":budget},"tokenUsageLine":format!(
"Token usage: used {} tokens, saved {} of {} during boot compile.",result.token_estimate,saved,budget)}))}"cortex_boot_audit"=>{
let limit=arg_usize(args,&["limit"]);let agent=arg_str(args,&["agent","source_agent"]).map(str::trim);let agent=agent.filter(|
value|!value.is_empty());let conn=state.db.lock().await;crate::handlers::boot::query_boot_audits(&conn,agent,limit).map_err(|err|
format!("boot_audits query failed: {err}"))}"cortex_reconnect"=>{let agent=arg_str(args,&["agent"]).map(str::to_string).
unwrap_or_else(||source_agent_for_tool(source,"mcp"));let model=source_model_for_tool(source,args);let(display_agent,expires_at)=
upsert_mcp_session(state,caller_id,&agent,model,"MCP reconnect").await?;state.emit("session",json!({"action":"reconnected","agent"
:display_agent}));Ok(json!({"reconnected":true,"agent":display_agent,"expiresAt":expires_at}))}"cortex_peek"=>{let query=args.get(
"query").and_then(|v|v.as_str()).ok_or_else(||"Missing required argument: query".to_string())?;let limit=args.get("limit").
and_then(|v|v.as_u64()).unwrap_or(10)as usize;let agent=source_agent_for_tool(source,"mcp");let model=source_model_for_tool(source
,args);let(display_agent,_,disposition)=refresh_mcp_session_presence(state,caller_id,&agent,model,"MCP active session").await?;if
disposition==McpPresenceDisposition::Started{state.emit("session",json!({"action":"started","agent":display_agent}));}let ctx=
RecallContext::from_caller(caller_id,state);let results=execute_unified_recall(state,query,0,limit,"mcp",&ctx,None).await?;Ok(
results)}"cortex_recall"=>{let query=arg_str(args,&["query","q"]).ok_or_else(||"Missing required argument: query".to_string())?;
let requested_policy_mode=parse_recall_policy_mode(arg_str(args,&["policyMode","policy_mode"]))?;let(budget,mut k,
resolved_policy_mode)=resolve_recall_budget_k(requested_policy_mode,arg_usize(args,&["budget","b"]),arg_usize(args,&["k","limit"])
);let agent=arg_str(args,&["agent","source_agent"]).unwrap_or_else(||source.as_ref().map(|s|s.agent.as_str()).unwrap_or("mcp"));
let task_class=arg_str(args,&["taskClass","task_class"]);let adaptive=args.get("adaptive").and_then(|value|value.as_bool()).
unwrap_or(false);let mut adaptive_policy:Option<Value>=None;if adaptive{let owner_id=if state.team_mode{caller_id.
unwrap_or_default()}else{0};let conn=state.db.lock().await;if let Some(policy)=recommend_recall_k(&conn,owner_id,agent,task_class,
k)?{if let Some(recommended_k)=policy.get("recommendedK").and_then(|value|value.as_u64()){k=recommended_k as usize;}
adaptive_policy=Some(policy);}}let model=source_model_for_tool(source,args);let(display_agent,_,disposition)=
refresh_mcp_session_presence(state,caller_id,agent,model,"MCP active session").await?;if disposition==McpPresenceDisposition::
Started{state.emit("session",json!({"action":"started","agent":display_agent}));}let ctx=RecallContext::from_caller(caller_id,
state);let mut payload=execute_unified_recall(state,query,budget,k,agent,&ctx,None).await?;if let Value::Object(map)=&mut payload{
map.insert("policyMode".to_string(),Value::String(resolved_policy_mode.as_str().to_string()));if let Some(mode)=
requested_policy_mode{map.insert("requestedPolicyMode".to_string(),Value::String(mode.as_str().to_string()));}}if let(Some(policy)
,Value::Object(map))=(adaptive_policy,&mut payload){map.insert("adaptivePolicy".to_string(),policy);}Ok(payload)}
"cortex_recall_policy_explain"=>{let query=arg_str(args,&["query","q"]).ok_or_else(||"Missing required argument: query".to_string(
))?;let requested_policy_mode=parse_recall_policy_mode(arg_str(args,&["policyMode","policy_mode"]))?;let(budget,k,
resolved_policy_mode)=resolve_recall_budget_k(requested_policy_mode,arg_usize(args,&["budget","b"]),arg_usize(args,&["k","limit"])
);let pool_k=arg_usize(args,&["pool_k","poolK","candidate_pool"]).unwrap_or((k.max(8)*3).min(64));let agent=arg_str(args,&["agent"
,"source_agent"]).unwrap_or_else(||source.as_ref().map(|s|s.agent.as_str()).unwrap_or("mcp"));let model=source_model_for_tool(
source,args);let(display_agent,_,disposition)=refresh_mcp_session_presence(state,caller_id,agent,model,"MCP active session").await
?;if disposition==McpPresenceDisposition::Started{state.emit("session",json!({"action":"started","agent":display_agent}));}let ctx
=RecallContext::from_caller(caller_id,state);let mut payload=execute_recall_policy_explain(state,query,budget,k,agent,&ctx,None,
pool_k,None).await?;if let Value::Object(map)=&mut payload{map.insert("policyMode".to_string(),Value::String(resolved_policy_mode.
as_str().to_string()));if let Some(mode)=requested_policy_mode{map.insert("requestedPolicyMode".to_string(),Value::String(mode.
as_str().to_string()));}}Ok(payload)}"cortex_semantic_recall"=>{let query=arg_str(args,&["query","q"]).ok_or_else(||
"Missing required argument: query".to_string())?;let budget=arg_usize(args,&["budget","b"]).unwrap_or(200);let k=arg_usize(args,&[
"k","limit"]).unwrap_or(if budget<=220{14}else{10});let agent=arg_str(args,&["agent","source_agent"]).unwrap_or_else(||source.
as_ref().map(|s|s.agent.as_str()).unwrap_or("mcp"));let model=source_model_for_tool(source,args);let(display_agent,_,disposition)=
refresh_mcp_session_presence(state,caller_id,agent,model,"MCP active session").await?;if disposition==McpPresenceDisposition::
Started{state.emit("session",json!({"action":"started","agent":display_agent}));}let ctx=RecallContext::from_caller(caller_id,
state);execute_semantic_recall(state,query,budget,k,agent,&ctx,None).await}"cortex_store"=>{let decision=arg_str(args,&["decision"
,"d"]).ok_or_else(||"Missing required argument: decision".to_string())?;let context=arg_str(args,&["context","c"]).map(str::
to_string);let entry_type=arg_str(args,&["type","t"]).map(str::to_string);let source_agent=source_agent_for_tool(source,arg_str(
args,&["source_agent"]).unwrap_or("mcp"));let source_model=source_model_for_tool(source,args);let reasoning_depth=arg_str(args,&[
"reasoning_depth","reasoningDepth"]);let provenance=DecisionProvenance::from_fields(&source_agent,source_model,reasoning_depth);
let confidence=arg_f64(args,&["confidence","conf"]);let ttl_seconds=arg_i64(args,&["ttl_seconds","ttl"]);let retention_class=match
arg_str(args,&["retention_class","retentionClass"]){Some(raw)=>Some(RetentionClass::parse(raw).ok_or_else(||format!(
"Invalid retention_class: {raw}"))?),None=>None,};validate_explicit_ttl_seconds(ttl_seconds).map_err(|err|err.to_string())?;let
decision_embedding=match state.embedding_engine.clone(){Some(engine)=>engine.embed_async(decision.to_string()).await,None=>None,};
let mut conn=state.db.lock().await;let(entry,new_id)=store_decision_with_input_embedding_and_provenance_retention(&mut conn,
decision,context,entry_type,source_agent.clone(),provenance,confidence,ttl_seconds,retention_class,decision_embedding.as_deref(),
caller_id,).map_err(|err|err.to_string())?;if let(Some(id),Some(vec))=(new_id,decision_embedding.as_deref()){let model_key=state.
embedding_engine.as_ref().map(|engine|engine.model_key()).unwrap_or(crate::embeddings::selected_model_key());let _=
persist_decision_embedding(&conn,id,vec,model_key);}crate::focus::focus_append(&conn,&source_agent,decision);Ok(json!({"stored":
true,"id":new_id,"sourceAgent":source_agent,"kind":entry.get("kind").cloned().unwrap_or(Value::Null),"action":entry.get("action").
cloned().unwrap_or_else(||json!("stored")),"retention_class":entry.get("retention_class").cloned().unwrap_or(Value::Null),}))}
"cortex_agent_feedback_record"=>{let owner_id=if state.team_mode{caller_id.unwrap_or_default()}else{0};let fallback_agent=source.
as_ref().map(|identity|identity.agent.as_str()).unwrap_or("mcp");let conn=state.db.lock().await;record_agent_feedback_from_value(&
conn,owner_id,args,fallback_agent)}"cortex_agent_feedback_stats"=>{let owner_id=if state.team_mode{caller_id.unwrap_or_default()}
else{0};let horizon_days=arg_i64(args,&["horizonDays","horizon_days"]).unwrap_or(30);let limit=arg_usize(args,&["limit"]).
unwrap_or(400);let task_class=arg_str(args,&["taskClass","task_class"]);let agent=arg_str(args,&["agent","source_agent"]);let conn
=state.db.lock().await;build_agent_feedback_stats_payload(&conn,owner_id,horizon_days,limit,task_class,agent)}"cortex_health"=>Ok(
build_health_payload(state,false).await),"cortex_digest"=>{let conn=state.db.lock().await;build_digest(&conn)}"cortex_unfold"=>{
const MAX_UNFOLD_SOURCES:usize=50;let sources:Vec<String>=match args.get("sources"){Some(Value::Array(arr))=>arr.iter().filter_map
(|v|v.as_str().map(String::from)).collect(),Some(Value::String(s))=>s.split(',').map(|s|s.trim().to_string()).filter(|s|!s.
is_empty()).collect(),_=>{return Err("Missing required argument: sources (array of source strings)".to_string());}};if sources.
is_empty(){return Err("sources array is empty".to_string());}if sources.len()>MAX_UNFOLD_SOURCES{return Err(format!(
"Too many sources (max {MAX_UNFOLD_SOURCES})"));}let agent=arg_str(args,&["agent","source_agent"]).unwrap_or_else(||source.as_ref(
).map(|s|s.agent.as_str()).unwrap_or("mcp"));let model=source_model_for_tool(source,args);let(display_agent,_,disposition)=
refresh_mcp_session_presence(state,caller_id,agent,model,"MCP active session").await?;if disposition==McpPresenceDisposition::
Started{state.emit("session",json!({"action":"started","agent":display_agent}));}let ctx=RecallContext::from_caller(caller_id,
state);let conn=state.db_read.lock().await;let mut results:Vec<Value>=Vec::new();let mut total_tokens=0usize;let mut found_sources
:Vec<String>=Vec::new();for source in&sources{if source.starts_with("crystal::"){if let Some(id_str)=source.split("::").nth(1){if
let Ok(crystal_id)=id_str.parse::<i64>(){let members=crate::crystallize::unfold_crystal(&conn,crystal_id);let crystal_text=conn.
query_row("SELECT consolidated_text FROM memory_clusters WHERE id = ?1",rusqlite::params![crystal_id],|row|{row.get::<_,String>(0)
}).unwrap_or_default();let tokens=estimate_tokens(&crystal_text);total_tokens+=tokens;found_sources.push(source.clone());results.
push(json!({"source":source,"text":crystal_text,"type":"crystal","tokens":tokens,"members":members,}));continue;}}}if let Some(
item)=unfold_source(&conn,source,&ctx){let tokens=estimate_tokens(item["text"].as_str().unwrap_or(""));total_tokens+=tokens;
found_sources.push(source.clone());results.push(json!({"source":source,"text":item["text"],"type":item["type"],"tokens":tokens,}))
;}else{results.push(json!({"source":source,"text":null,"type":"not_found","tokens":0,}));}}drop(conn);if!found_sources.is_empty(){
let query_text="";let query_blob=match state.embedding_engine.clone(){Some(engine)=>engine.embed_query_async(query_text.to_string(
)).await.map(|v|crate::embeddings::vector_to_blob(&v)),None=>None,};let conn=state.db.lock().await;crate::handlers::feedback::
record_unfold_feedback(&conn,&found_sources,agent,query_text,query_blob.as_deref());}Ok(json!({"results":results,"totalTokens":
total_tokens,"count":results.iter().filter(|r|r["type"]!="not_found").count(),"feedbackRecorded":found_sources.len(),}))}
"cortex_forget"=>{let keyword=args.get("source").and_then(|v|v.as_str()).ok_or_else(||"Missing required argument: source".
to_string())?;let mut conn=state.db.lock().await;let owner_id=if state.team_mode{caller_id}else{None};let affected=
forget_keyword_scoped(&mut conn,keyword,owner_id)?;Ok(json!({"affected":affected}))}"cortex_resolve"=>{let keep_id=args.get(
"keepId").and_then(|v|v.as_i64()).ok_or_else(||"Missing required argument: keepId".to_string())?;let action=args.get("action").
and_then(|v|v.as_str()).ok_or_else(||"Missing required argument: action".to_string())?;let superseded_id=args.get("supersededId").
and_then(|v|v.as_i64());let mut conn=state.db.lock().await;resolve_decision(&mut conn,keep_id,action,superseded_id)?;Ok(json!({
"resolved":true}))}"cortex_conflicts_list"=>{let status=ConflictStatusFilter::parse(arg_str(args,&["status"]))?;let classification
=arg_str(args,&["classification"]).map(str::trim).map(str::to_string);let conflict_id=arg_str(args,&["conflictId","conflict_id",
"id"]).map(str::trim).filter(|value|!value.is_empty()).map(str::to_string);let limit=arg_usize(args,&["limit"]).unwrap_or(100).
clamp(1,500);let options=ConflictListOptions{status,classification,conflict_id,limit,};let conn=state.db.lock().await;
list_conflicts_payload(&conn,&options)}"cortex_conflicts_get"=>{let conflict_id=arg_str(args,&["conflictId","conflict_id","id"]).
ok_or_else(||"Missing required argument: conflictId".to_string())?.to_string();let options=ConflictListOptions{status:
ConflictStatusFilter::All,classification:None,conflict_id:Some(conflict_id.clone()),limit:200,};let conn=state.db.lock().await;let
payload=list_conflicts_payload(&conn,&options)?;let found=payload.get("count").and_then(|value|value.as_u64()).map(|value|value>0)
.unwrap_or(false);Ok(json!({"found":found,"conflictId":conflict_id,"conflict":payload.get("conflict").cloned().unwrap_or(Value::
Null),}))}"cortex_conflicts_resolve"=>{let action=arg_str(args,&["action"]).ok_or_else(||"Missing required argument: action".
to_string())?;let mut winner_id=arg_i64(args,&["winnerId","keepId"]);let mut superseded_id=arg_i64(args,&["supersededId","loserId"
]);let conflict_id=arg_str(args,&["conflictId","conflict_id","id"]).map(str::trim).filter(|value|!value.is_empty()).map(str::
to_string);if let Some((left,right))=conflict_id.as_deref().and_then(parse_conflict_id){if winner_id.is_none(){winner_id=Some(left
);}if superseded_id.is_none(){superseded_id=winner_id.map(|winner|{if winner==left{right}else if winner==right{left}else{right}});
}}let winner_id=winner_id.ok_or_else(||"Missing required argument: winnerId (or keepId)".to_string())?;let resolved_by=arg_str(
args,&["resolvedBy","resolved_by"]).map(str::to_string).unwrap_or_else(||source_agent_for_tool(source,"mcp"));let metadata=
ResolutionMetadata{conflict_id,classification:arg_str(args,&["classification"]).map(str::to_string),notes:arg_str(args,&["notes"])
.map(str::to_string),resolved_by:Some(resolved_by),similarity:arg_f64(args,&["similarity"]),};let mut conn=state.db.lock().await;
resolve_decision_with_metadata(&mut conn,winner_id,action,superseded_id,metadata)}"cortex_consensus_promote"=>{let limit=arg_usize
(args,&["limit"]).unwrap_or(50).clamp(1,500);let min_margin=arg_f64(args,&["minMargin","min_margin"]).unwrap_or(0.1).clamp(0.0,1.0
);let dry_run=args.get("dryRun").and_then(|value|value.as_bool()).unwrap_or(false);let resolved_by=source_agent_for_tool(source,
"mcp");let mut conn=state.db.lock().await;let list_payload=list_conflicts_payload(&conn,&ConflictListOptions{status:
ConflictStatusFilter::Open,classification:None,conflict_id:None,limit,},)?;let conflicts=list_payload.get("conflicts").and_then(|
value|value.as_array()).cloned().unwrap_or_default();let mut promoted=Vec::new();let mut skipped=Vec::new();let mut failed=Vec::
new();for conflict in conflicts{let Some(conflict_id)=conflict.get("id").and_then(|value|value.as_str())else{skipped.push(json!({
"reason":"missing_conflict_id","conflict":conflict}));continue;};let left=conflict.get("left").cloned().unwrap_or(Value::Null);let
right=conflict.get("right").cloned().unwrap_or(Value::Null);let left_id=left.get("id").and_then(|value|value.as_i64());let
right_id=right.get("id").and_then(|value|value.as_i64());let(Some(left_id),Some(right_id))=(left_id,right_id)else{skipped.push(
json!({"conflictId":conflict_id,"reason":"missing_decision_ids"}));continue;};let left_score=left.get("trustScore").and_then(|
value|value.as_f64()).or_else(||left.get("confidence").and_then(|value|value.as_f64())).unwrap_or(0.0);let right_score=right.get(
"trustScore").and_then(|value|value.as_f64()).or_else(||right.get("confidence").and_then(|value|value.as_f64())).unwrap_or(0.0);
let recommended=conflict.get("trustContext").and_then(|value|value.get("recommendedWinnerId")).and_then(|value|value.as_i64());let
(winner_id,loser_id,winner_score,loser_score)=match recommended{Some(id)if id==left_id=>(left_id,right_id,left_score,right_score),
Some(id)if id==right_id=>(right_id,left_id,right_score,left_score),_ if left_score>=right_score=>(left_id,right_id,left_score,
right_score),_=>(right_id,left_id,right_score,left_score),};let margin=(winner_score-loser_score).abs();if margin<min_margin{
skipped.push(json!({"conflictId":conflict_id,"reason":"margin_below_threshold","winnerId":winner_id,"loserId":loser_id,
"winnerScore":winner_score,"loserScore":loser_score,"margin":margin,"minMargin":min_margin}));continue;}if dry_run{promoted.push(
json!({"conflictId":conflict_id,"winnerId":winner_id,"supersededId":loser_id,"winnerScore":winner_score,"loserScore":loser_score,
"margin":margin,"applied":false}));continue;}let metadata=ResolutionMetadata{conflict_id:Some(conflict_id.to_string()),
classification:conflict.get("classification").and_then(|value|value.as_str()).map(str::to_string),notes:Some(format!(
"Auto-promoted by cortex_consensus_promote (margin {margin:.3})")),resolved_by:Some(resolved_by.clone()),similarity:conflict.get(
"similarity").and_then(|value|value.as_f64()),};match resolve_decision_with_metadata(&mut conn,winner_id,"keep",Some(loser_id),
metadata){Ok(payload)=>promoted.push(payload),Err(err)=>failed.push(json!({"conflictId":conflict_id,"winnerId":winner_id,
"supersededId":loser_id,"error":err})),}}let scanned=promoted.len()+skipped.len()+failed.len();state.emit("consensus",json!({
"action":if dry_run{"promote_dry_run"}else{"promoted"},"scanned":scanned,"promoted":promoted.len(),"skipped":skipped.len(),
"failed":failed.len()}),);Ok(json!({"dryRun":dry_run,"limit":limit,"minMargin":min_margin,"scanned":scanned,"promotedCount":
promoted.len(),"skippedCount":skipped.len(),"failedCount":failed.len(),"promoted":promoted,"skipped":skipped,"failed":failed}))}
"cortex_memory_decay_run"=>{let include_aging=args.get("includeAging").and_then(|value|value.as_bool()).unwrap_or(true);let
cleanup_expired=args.get("cleanupExpired").and_then(|value|value.as_bool()).unwrap_or(true);let conn=state.db.lock().await;let
decayed=indexer::decay_pass(&conn);let(compressed,archived)=if include_aging{aging::run_aging_pass(&conn)}else{(0,0)};let
expired_cleanup=if cleanup_expired{Some(db::delete_expired_entries(&conn).map_err(|err|err.to_string())?)}else{None};let
expired_memories=expired_cleanup.map(|counts|counts.memories_deleted).unwrap_or(0);let expired_decisions=expired_cleanup.map(|
counts|counts.decisions_deleted).unwrap_or(0);state.emit("maintenance",json!({"action":"memory_decay_run","decayed":decayed,
"compressed":compressed,"archived":archived,"expiredMemoriesDeleted":expired_memories,"expiredDecisionsDeleted":expired_decisions}
),);Ok(json!({"ok":true,"decayed":decayed,"aging":{"ran":include_aging,"compressed":compressed,"archived":archived},
"expiredCleanup":{"ran":cleanup_expired,"memoriesDeleted":expired_memories,"decisionsDeleted":expired_decisions}}))}
"cortex_eval_run"=>{let horizon_days=arg_i64(args,&["horizonDays","horizon_days"]).unwrap_or(30).clamp(1,180);let conn=state.db.
lock().await;Ok(crate::eval::build_eval_snapshot(&conn,horizon_days))}"cortex_focus_start"=>{let label=args.get("label").and_then(
|v|v.as_str()).ok_or_else(||"Missing required argument: label".to_string())?;let agent=arg_str(args,&["agent"]).unwrap_or_else(||
source.as_ref().map(|s|s.agent.as_str()).unwrap_or("mcp"));let conn=state.db.lock().await;crate::focus::focus_start(&conn,label,
agent)}"cortex_focus_end"=>{let label=args.get("label").and_then(|v|v.as_str()).ok_or_else(||"Missing required argument: label".
to_string())?;let agent=arg_str(args,&["agent"]).unwrap_or_else(||source.as_ref().map(|s|s.agent.as_str()).unwrap_or("mcp"));let
conn=state.db.lock().await;crate::focus::focus_end(&conn,label,agent,caller_id)}"cortex_focus_status"=>{let agent=arg_str(args,&[
"agent"]).unwrap_or_else(||source.as_ref().map(|s|s.agent.as_str()).unwrap_or("mcp"));let conn=state.db.lock().await;let current=
crate::focus::focus_current(&conn,agent);let mut recent:Vec<Value>=Vec::new();if let Ok(mut stmt)=conn.prepare(
"SELECT id, label, summary, tokens_before, tokens_after, started_at, ended_at \
                 FROM focus_sessions WHERE agent = ?1 AND status = 'closed' \
                 ORDER BY ended_at DESC LIMIT 5"
,){if let Ok(rows)=stmt.query_map(rusqlite::params![agent],|row|{Ok(json!({"id":row.get::<_,i64>(0)?,"label":row.get::<_,String>(1
)?,"summary":row.get::<_,Option<String>>(2)?,"tokensBefore":row.get::<_,Option<i64>>(3)?,"tokensAfter":row.get::<_,Option<i64>>(4)
?,"startedAt":row.get::<_,String>(5)?,"endedAt":row.get::<_,Option<String>>(6)?}))}){for row in rows.flatten(){recent.push(row);}}
}Ok(json!({"active":current,"recent":recent,"count":recent.len()}))}"cortex_diary"=>{let body=DiaryRequest{accomplished:arg_str(
args,&["accomplished","done"]).map(str::to_string),next_steps:arg_str(args,&["nextSteps","next_steps","next"]).map(str::to_string)
,decisions:arg_str(args,&["decisions","dec"]).map(str::to_string),key_decisions:arg_str(args,&["keyDecisions"]).map(str::to_string
),pending:arg_str(args,&["pending","pend"]).map(str::to_string),known_issues:arg_str(args,&["knownIssues","known_issues","issues"]
).map(str::to_string),};let source_agent=source_agent_for_tool(source,"mcp");let path=write_diary_entry(state,&body,&source_agent)
.await?;Ok(json!({"written":true,"agent":source_agent,"path":path}))}"cortex_lastCall"=>{let kind=arg_str(args,&["kind"]);let
agent_filter=arg_str(args,&["agent","source_agent"]);let ctx=RecallContext::from_caller(caller_id,state);let conn=state.db.lock().
await;fetch_last_call(&conn,kind,agent_filter,&ctx)}"cortex_permissions_list"=>{let owner_id=if state.team_mode{caller_id.
unwrap_or_default()}else{0};let conn=state.db.lock().await;let mut stmt=conn.prepare(
"SELECT client_id, permission, scope, granted_by, granted_at
                     FROM client_permissions
                     WHERE owner_id = ?1
                     ORDER BY client_id ASC, permission ASC, scope ASC"
,).map_err(|err|err.to_string())?;let rows=stmt.query_map(rusqlite::params![owner_id],|row|{Ok(json!({"client":row.get::<_,String>
(0)?,"permission":row.get::<_,String>(1)?,"scope":row.get::<_,String>(2)?,"grantedBy":row.get::<_,String>(3)?,"grantedAt":row.get
::<_,String>(4)?,}))}).map_err(|err|err.to_string())?;let grants:Vec<Value>=rows.filter_map(Result::ok).collect();Ok(json!({
"ownerId":owner_id,"count":grants.len(),"grants":grants}))}"cortex_permissions_grant"=>{let owner_id=if state.team_mode{caller_id.
unwrap_or_default()}else{0};let client=arg_str(args,&["client","client_id"]).ok_or_else(||"Missing required argument: client".
to_string())?;let client=if client.trim()=="*"{"*".to_string()}else{normalize_permission_client_id(client)};let permission_raw=
arg_str(args,&["permission"]).ok_or_else(||"Missing required argument: permission".to_string())?;let permission=
parse_client_permission(permission_raw).ok_or_else(||"Invalid permission; expected read, write, or admin".to_string())?;let scope=
arg_str(args,&["scope"]).map(str::to_string).unwrap_or_else(||"*".to_string());let granted_by=source_client_for_permissions(source
,args);let conn=state.db.lock().await;conn.execute(
"INSERT INTO client_permissions (owner_id, client_id, permission, scope, granted_by, granted_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))
                 ON CONFLICT(owner_id, client_id, permission, scope)
                 DO UPDATE SET granted_by = excluded.granted_by, granted_at = excluded.granted_at"
,rusqlite::params![owner_id,client,permission.as_str(),scope,granted_by],).map_err(|err|err.to_string())?;Ok(json!({"granted":true
,"ownerId":owner_id,"client":client,"permission":permission.as_str(),"scope":scope,}))}"cortex_permissions_revoke"=>{let owner_id=
if state.team_mode{caller_id.unwrap_or_default()}else{0};let client=arg_str(args,&["client","client_id"]).ok_or_else(||
"Missing required argument: client".to_string())?;let client=if client.trim()=="*"{"*".to_string()}else{
normalize_permission_client_id(client)};let permission_raw=arg_str(args,&["permission"]).ok_or_else(||
"Missing required argument: permission".to_string())?;let permission=parse_client_permission(permission_raw).ok_or_else(||
"Invalid permission; expected read, write, or admin".to_string())?;let scope=arg_str(args,&["scope"]).map(str::to_string).
unwrap_or_else(||"*".to_string());let conn=state.db.lock().await;let deleted=conn.execute(
"DELETE FROM client_permissions
                     WHERE owner_id = ?1 AND client_id = ?2 AND permission = ?3 AND scope = ?4",
rusqlite::params![owner_id,client,permission.as_str(),scope],).map_err(|err|err.to_string())?;Ok(json!({"revoked":deleted>0,
"deleted":deleted,"ownerId":owner_id,"client":client,"permission":permission.as_str(),"scope":scope,}))}_=>Err(format!(
"Unknown tool: {tool_name}")),}}
