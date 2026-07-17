use super::*;use crate::api_types::RetentionClass;use crate::conflict::{detect_conflict,jaccard_similarity,ConflictClassification}
;use crate::db::checkpoint_wal_best_effort;use crate::handlers::{log_event,now_iso,truncate_chars};use rusqlite::{params,
Connection};use serde_json::{json,Value};#[allow(clippy::too_many_arguments,
dead_code)]pub fn store_decision_with_ttl(conn:&mut Connection,decision:&str,context:Option<String>,entry_type:Option<String>,
source_agent:String,confidence:Option<f64>,ttl_seconds:Option<i64>,owner_id:Option<i64>,)->Result<(Value,Option<i64>),String>{let
provenance=DecisionProvenance::from_fields(&source_agent,None,None);store_decision_internal(conn,decision,context,entry_type,
source_agent,provenance,confidence,ttl_seconds,None,None,owner_id).map_err(|err|err.to_string())}#[allow(clippy::
too_many_arguments,dead_code)]pub(crate)fn store_decision_with_input_embedding(conn:&mut Connection,decision:&str,context:Option<
String>,entry_type:Option<String>,source_agent:String,confidence:Option<f64>,ttl_seconds:Option<i64>,query_embedding:Option<&[f32]
>,owner_id:Option<i64>,)->Result<(Value,Option<i64>),StoreError>{let provenance=DecisionProvenance::from_fields(&source_agent,None
,None);store_decision_with_input_embedding_and_provenance(conn,decision,context,entry_type,source_agent,provenance,confidence,
ttl_seconds,query_embedding,owner_id)}#[allow(clippy::too_many_arguments)]pub(crate)fn
store_decision_with_input_embedding_and_provenance(conn:&mut Connection,decision:&str,context:Option<String>,entry_type:Option<
String>,source_agent:String,provenance:DecisionProvenance,confidence:Option<f64>,ttl_seconds:Option<i64>,query_embedding:Option<&[
f32]>,owner_id:Option<i64>,)->Result<(Value,Option<i64>),StoreError>{store_decision_with_input_embedding_and_provenance_retention(
conn,decision,context,entry_type,source_agent,provenance,confidence,ttl_seconds,None,query_embedding,owner_id)}#[allow(clippy::
too_many_arguments)]pub(crate)fn store_decision_with_input_embedding_and_provenance_retention(conn:&mut Connection,decision:&str,
context:Option<String>,entry_type:Option<String>,source_agent:String,provenance:DecisionProvenance,confidence:Option<f64>,
ttl_seconds:Option<i64>,retention_class:Option<RetentionClass>,query_embedding:Option<&[f32]>,owner_id:Option<i64>,)->Result<(
Value,Option<i64>),StoreError>{store_decision_internal(conn,decision,context,entry_type,source_agent,provenance,confidence,
ttl_seconds,retention_class,query_embedding,owner_id,)}#[allow(clippy::too_many_arguments)]pub(crate)fn store_decision_internal(
conn:&mut Connection,decision:&str,context:Option<String>,entry_type:Option<String>,source_agent:String,provenance:
DecisionProvenance,confidence:Option<f64>,ttl_seconds:Option<i64>,retention_class:Option<RetentionClass>,query_embedding:Option<&[
f32]>,owner_id:Option<i64>,)->Result<(Value,Option<i64>),StoreError>{let entry_type=entry_type.unwrap_or_else(||"decision".
to_string());let suppress_benchmark_events=is_benchmark_entry_type(&entry_type)||is_benchmark_source_agent(&source_agent);let mut
decision_text=decision.trim().to_string();let decision_chars=decision_text.chars().count();let decision_truncated=!
is_benchmark_entry_type(&entry_type)&&decision_chars>MAX_DECISION_CHARS;if decision_truncated{decision_text=truncate_chars(&
decision_text,MAX_DECISION_CHARS);}let decision=decision_text.as_str();let quality=assess_quality(decision);let confidence=
confidence.unwrap_or(0.8);let trust_score=provenance.trust_score(confidence);let ts=now_iso();let retention_class=RetentionClass::
classify(retention_class,&entry_type,decision,context.as_deref());let ttl_seconds=validate_explicit_ttl_seconds(ttl_seconds)?;let
effective_ttl_seconds=ttl_seconds.or_else(||retention_class.default_ttl_seconds());let expires_at=compute_expires_at(conn,
effective_ttl_seconds).map_err(StoreError::Internal)?;if decision_truncated{let _=log_event(conn,"decision_truncated",json!({
"source_agent":source_agent,"entry_type":entry_type.as_str(),"original_chars":decision_chars,"stored_chars":MAX_DECISION_CHARS,
"preview":truncate_chars(decision,180),}),"rust-daemon",);}if is_benchmark_entry_type(&entry_type){return insert_decision(conn,
decision,context,&entry_type,&source_agent,&provenance,confidence,trust_score,quality.score,retention_class,expires_at,&ts,
owner_id,1.0,!suppress_benchmark_events,);}if quality.score<TOO_VAGUE_THRESHOLD{return Err(StoreError::Validation{message:
"Memory too vague",quality:quality.score,factors:quality.factors,});}if let Some(query_vector)=query_embedding{let candidates=
fetch_top_semantic_candidates(conn,query_vector,owner_id)?;let dedup_action=choose_semantic_dedup_action(&candidates,decision);let
best_similarity=candidates.first().map(|candidate|candidate.similarity as f64).unwrap_or(0.0);if let SemanticDedupAction::Merge{
target_id,similarity,jaccard}=dedup_action{return merge_into_existing_decision(conn,target_id,decision,context.as_deref(),&
source_agent,quality.score,similarity,jaccard,&ts,owner_id);}return insert_decision(conn,decision,context,&entry_type,&
source_agent,&provenance,confidence,trust_score,quality.score,retention_class,expires_at,&ts,owner_id,(1.0-best_similarity).clamp(
0.0,1.0),!suppress_benchmark_events,);}store_decision_legacy(conn,decision,context,&entry_type,&source_agent,&provenance,
confidence,trust_score,quality.score,retention_class,expires_at,&ts,owner_id,)}#[allow(clippy::too_many_arguments)]pub(crate)fn
store_decision_legacy(conn:&mut Connection,decision:&str,context:Option<String>,entry_type:&str,source_agent:&str,provenance:&
DecisionProvenance,confidence:f64,trust_score:f64,quality:i32,retention_class:RetentionClass,expires_at:Option<String>,ts:&str,
owner_id:Option<i64>,)->Result<(Value,Option<i64>),StoreError>{let relation=detect_conflict(conn,decision,source_agent,owner_id).
map_err(StoreError::Internal)?;match relation.classification{ConflictClassification::Contradicts=>{return
handle_contradiction_policy(conn,decision,context.as_deref(),entry_type,source_agent,provenance,confidence,trust_score,quality,
retention_class,expires_at.as_deref(),ts,owner_id,&relation,);}ConflictClassification::Agrees=>{return handle_agreement_policy(
conn,decision,context.as_deref(),source_agent,quality,ts,&relation);}ConflictClassification::Refines=>{return
handle_refinement_policy(conn,decision,context.as_deref(),entry_type,source_agent,provenance,confidence,trust_score,quality,
retention_class,expires_at.as_deref(),ts,owner_id,&relation,);}ConflictClassification::Unrelated=>{}}let existing:Vec<String>=if
let Some(owner_id)=owner_id{let mut stmt=conn.prepare(
"SELECT decision FROM decisions \
                 WHERE owner_id = ?1 \
                 AND status = 'active' \
                 AND (expires_at IS NULL OR expires_at > datetime('now')) \
                 ORDER BY created_at DESC LIMIT 50"
,).map_err(|e|StoreError::Internal(e.to_string()))?;let rows=stmt.query_map(params![owner_id],|row|row.get(0)).map_err(|e|
StoreError::Internal(e.to_string()))?;rows.filter_map(|row|row.ok()).collect()}else{let mut stmt=conn.prepare(
"SELECT decision FROM decisions \
                 WHERE status = 'active' \
                 AND (expires_at IS NULL OR expires_at > datetime('now')) \
                 ORDER BY created_at DESC LIMIT 50"
,).map_err(|e|StoreError::Internal(e.to_string()))?;let rows=stmt.query_map([],|row|row.get(0)).map_err(|e|StoreError::Internal(e.
to_string()))?;rows.filter_map(|row|row.ok()).collect()};let max_sim=existing.iter().map(|text|jaccard_similarity(decision,text)).
fold(0.0_f64,f64::max);let surprise=1.0-max_sim;if surprise<0.25{let _=log_event(conn,"decision_rejected_duplicate",json!({
"decision":&decision[..decision.len().min(100)],"surprise":surprise,"source_agent":source_agent,"quality":quality,}),"rust-daemon"
,);checkpoint_wal_best_effort(conn);let mut entry=json!({"stored":false,"reason":"duplicate","surprise":surprise,"quality":quality
,});decorate_entry_with_relation(&mut entry,&relation,None);return Ok((entry,None));}let(mut entry,new_id)=insert_decision(conn,
decision,context,entry_type,source_agent,provenance,confidence,trust_score,quality,retention_class,expires_at,ts,owner_id,surprise
,!(is_benchmark_entry_type(entry_type)||is_benchmark_source_agent(source_agent)),)?;decorate_entry_with_relation(&mut entry,&
relation,None);Ok((entry,new_id))}
