use super::*;use crate::api_types::RetentionClass;use crate::conflict::ConflictResult;use crate::db::checkpoint_wal_best_effort;
use crate::handlers::log_event;use rusqlite::{params,Connection};use serde_json::{json,Value};pub(crate)fn
handle_contradiction_policy(conn:&mut Connection,decision:&str,context:Option<&str>,entry_type:&str,source_agent:&str,provenance:&
DecisionProvenance,confidence:f64,trust_score:f64,quality:i32,retention_class:RetentionClass,expires_at:Option<&str>,ts:&str,
owner_id:Option<i64>,relation:&ConflictResult,)->Result<(Value,Option<i64>),StoreError>{let existing_id=relation.matched_id.
ok_or_else(||StoreError::Internal("Missing conflict target id".to_string()))?;let existing_trust=relation.matched_trust_score.
unwrap_or(0.8);let incoming_wins=trust_score>existing_trust;let strategy=if incoming_wins{"trust_score_source_wins"}else{
"trust_score_target_wins"};let tx=conn.transaction().map_err(|e|StoreError::Internal(e.to_string()))?;if incoming_wins{if let Some
(owner_id)=owner_id{tx.execute("UPDATE decisions SET status = 'superseded', updated_at = ?1 WHERE id = ?2 AND owner_id = ?3",
params![ts,existing_id,owner_id],).map_err(|e|StoreError::Internal(e.to_string()))?;}else{tx.execute(
"UPDATE decisions SET status = 'superseded', updated_at = ?1 WHERE id = ?2",params![ts,existing_id]).map_err(|e|StoreError::
Internal(e.to_string()))?;}}let new_id=insert_decision_with_state(&tx,decision,context,entry_type,source_agent,provenance,
confidence,trust_score,quality,retention_class,expires_at,ts,owner_id,if incoming_wins{"active"}else{"disputed"},if incoming_wins{
None}else{Some(existing_id)},if incoming_wins{Some(existing_id)}else{None},Some((1.0-relation.similarity_jaccard).clamp(0.0,1.0)),
)?;let conflict_record_id=insert_conflict_record(&tx,Some(new_id),existing_id,relation.classification,relation.similarity_jaccard,
relation.similarity_cosine,"auto_resolved",Some(strategy),Some("policy_engine"),ts,)?;let _=log_event(&tx,"decision_conflict",json
!({"newId":new_id,"existingId":existing_id,"source_agent":source_agent,"matchedAgent":relation.matched_agent,"strategy":strategy,
"source_trust_score":trust_score,"target_trust_score":existing_trust,"conflict_record_id":conflict_record_id,}),"rust-daemon",);tx
.commit().map_err(|e|StoreError::Internal(e.to_string()))?;checkpoint_wal_best_effort(conn);let mut entry=json!({"action":
"inserted","id":new_id,"status":if incoming_wins{"active"}else{"disputed"},"retention_class":retention_class.as_str(),"quality":
quality,"conflictWith":existing_id,"resolution_strategy":strategy,});if incoming_wins{entry["supersedes"]=json!(existing_id);}
decorate_entry_with_relation(&mut entry,relation,Some(conflict_record_json(conflict_record_id,Some(new_id),existing_id,relation.
classification,"auto_resolved",Some(strategy),)),);Ok((entry,Some(new_id)))}#[allow(clippy::too_many_arguments)]pub(crate)fn
handle_agreement_policy(conn:&mut Connection,decision:&str,context:Option<&str>,source_agent:&str,quality:i32,ts:&str,relation:&
ConflictResult,)->Result<(Value,Option<i64>),StoreError>{let target_id=relation.matched_id.ok_or_else(||StoreError::Internal(
"Missing agreement target id".to_string()))?;let tx=conn.transaction().map_err(|e|StoreError::Internal(e.to_string()))?;let(
existing_decision,existing_context,previous_merged_count):(String,Option<String>,i64)=tx.query_row(
"SELECT decision, context, COALESCE(merged_count, 0) FROM decisions WHERE id = ?1",params![target_id],|row|{Ok((row.get(0)?,row.
get(1)?,row.get(2)?))}).map_err(|e|StoreError::Internal(e.to_string()))?;let merged_context=merge_context(existing_context,&
existing_decision,context,decision);let merged_count=previous_merged_count+1;tx.execute(
"UPDATE decisions \
         SET context = ?1, \
             score = COALESCE(score, 0) + ?2, \
             merged_count = ?3, \
             quality = MAX(COALESCE(quality, 50), ?4), \
             updated_at = ?5 \
         WHERE id = ?6"
,params![merged_context,MERGE_SCORE_BONUS,merged_count,quality,ts,target_id],).map_err(|e|StoreError::Internal(e.to_string()))?;
let conflict_record_id=insert_conflict_record(&tx,None,target_id,relation.classification,relation.similarity_jaccard,relation.
similarity_cosine,"auto_resolved",Some("deduplicated_merge"),Some("policy_engine"),ts,)?;let _=log_event(&tx,
"decision_agreement_merge",json!({"targetId":target_id,"source_agent":source_agent,"similarity_jaccard":relation.
similarity_jaccard,"conflict_record_id":conflict_record_id,}),"rust-daemon",);tx.commit().map_err(|e|StoreError::Internal(e.
to_string()))?;checkpoint_wal_best_effort(conn);let mut entry=json!({"action":"merged","target_id":target_id,"merged_count":
merged_count,"quality":quality,});decorate_entry_with_relation(&mut entry,relation,Some(conflict_record_json(conflict_record_id,
None,target_id,relation.classification,"auto_resolved",Some("deduplicated_merge"),)),);Ok((entry,None))}#[allow(clippy::
too_many_arguments)]pub(crate)fn handle_refinement_policy(conn:&mut Connection,decision:&str,context:Option<&str>,entry_type:&str,
source_agent:&str,provenance:&DecisionProvenance,confidence:f64,trust_score:f64,quality:i32,retention_class:RetentionClass,
expires_at:Option<&str>,ts:&str,owner_id:Option<i64>,relation:&ConflictResult,)->Result<(Value,Option<i64>),StoreError>{let
target_id=relation.matched_id.ok_or_else(||StoreError::Internal("Missing refinement target id".to_string()))?;let target_trust=
relation.matched_trust_score.unwrap_or(0.8);let should_supersede=relation.matched_agent.as_deref()==Some(source_agent)||
trust_score>=target_trust;let tx=conn.transaction().map_err(|e|StoreError::Internal(e.to_string()))?;if should_supersede{if let
Some(owner_id)=owner_id{tx.execute("UPDATE decisions SET status = 'superseded', updated_at = ?1 WHERE id = ?2 AND owner_id = ?3",
params![ts,target_id,owner_id],).map_err(|e|StoreError::Internal(e.to_string()))?;}else{tx.execute(
"UPDATE decisions SET status = 'superseded', updated_at = ?1 WHERE id = ?2",params![ts,target_id]).map_err(|e|StoreError::Internal
(e.to_string()))?;}}let new_id=insert_decision_with_state(&tx,decision,context,entry_type,source_agent,provenance,confidence,
trust_score,quality,retention_class,expires_at,ts,owner_id,if should_supersede{"active"}else{"disputed"},if should_supersede{None}
else{Some(target_id)},if should_supersede{Some(target_id)}else{None},Some((1.0-relation.similarity_jaccard).clamp(0.0,1.0)),)?;let
conflict_status=if should_supersede{"auto_resolved"}else{"open"};let strategy=if should_supersede{Some("refine_supersede")}else{
Some("requires_user_review")};let conflict_record_id=insert_conflict_record(&tx,Some(new_id),target_id,relation.classification,
relation.similarity_jaccard,relation.similarity_cosine,conflict_status,strategy,if should_supersede{Some("policy_engine")}else{
None},ts,)?;let event_name=if should_supersede{"decision_supersede"}else{"decision_refine_pending"};let _=log_event(&tx,event_name
,json!({"newId":new_id,"targetId":target_id,"source_agent":source_agent,"strategy":strategy,"conflict_record_id":
conflict_record_id,}),"rust-daemon",);tx.commit().map_err(|e|StoreError::Internal(e.to_string()))?;checkpoint_wal_best_effort(conn
);let mut entry=json!({"action":"inserted","id":new_id,"status":if should_supersede{"superseded_old"}else{"disputed"},
"retention_class":retention_class.as_str(),"quality":quality,});if should_supersede{entry["supersedes"]=json!(target_id);}else{
entry["conflictWith"]=json!(target_id);}decorate_entry_with_relation(&mut entry,relation,Some(conflict_record_json(
conflict_record_id,Some(new_id),target_id,relation.classification,conflict_status,strategy)),);Ok((entry,Some(new_id)))}
