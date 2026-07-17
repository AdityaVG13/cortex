use super::*;use crate::api_types::StoreRequest;use crate::budgets::BudgetEndpoint;use crate::handlers::{
ensure_auth_with_caller_rated_for_class,ensure_endpoint_budget,json_response,resolve_source_identity};use crate::rate_limit::
RequestClass;use crate::state::RuntimeState;use axum::extract::State;use axum::http::{HeaderMap,StatusCode};use axum::response::
Response;use axum::Json;use serde_json::json;pub async fn handle_store(State(state):State<RuntimeState>,headers:HeaderMap,Json(
body):Json<StoreRequest>)->Response{let caller_id=match ensure_auth_with_caller_rated_for_class(&headers,&state,RequestClass::
Store).await{Ok(id)=>id,Err(resp)=>return resp,};if state.team_mode&&caller_id.is_none(){return json_response(StatusCode::
FORBIDDEN,json!({"error":"Team mode requires a caller-scoped ctx_ API key"}));}let decision=body.decision.unwrap_or_default();if
decision.trim().is_empty(){return json_response(StatusCode::BAD_REQUEST,json!({"error":"Missing field: decision"}));}let
source_identity=resolve_source_identity(&headers,body.source_agent.as_deref().unwrap_or("http"));let source_agent=source_identity.
agent.clone();if let Err(resp)=ensure_endpoint_budget(&headers,&state,BudgetEndpoint::Store,&source_agent).await{return resp;}let
benchmark_store=body.entry_type.as_deref().map(is_benchmark_entry_type).unwrap_or(false)||is_benchmark_source_agent(&source_agent)
;let provenance=DecisionProvenance::from_fields(&source_agent,body.source_model.as_deref().or(source_identity.model.as_deref()),
body.reasoning_depth.as_deref());if let Err(StoreError::BadRequest(message))=validate_explicit_ttl_seconds(body.ttl_seconds){
return json_response(StatusCode::BAD_REQUEST,json!({"error":message}));}let decision_text=decision.trim().to_string();let
embedding_model_key=state.embedding_engine.as_ref().map(|engine|engine.model_key()).unwrap_or(crate::embeddings::
selected_model_key());let decision_embedding=match state.embedding_engine.clone(){Some(engine)=>engine.embed_async(decision_text.
clone()).await,None=>None,};let mut conn=state.db.lock().await;let result=
store_decision_with_input_embedding_and_provenance_retention(&mut conn,&decision_text,body.context,body.entry_type,source_agent.
clone(),provenance,body.confidence,body.ttl_seconds,body.retention_class,decision_embedding.as_deref(),caller_id,);match result{Ok
((entry,new_id))=>{if let Some(id)=new_id{if let Some(vec)=decision_embedding.as_deref(){if let Err(err)=
persist_decision_embedding(&conn,id,vec,embedding_model_key){eprintln!(
"[store] Warning: failed to persist decision embedding for {id}: {err}");}}else if let Some(engine)=state.embedding_engine.clone()
{let db=state.db.clone();let text=decision_text.clone();tokio::spawn(async move{let model_key=engine.model_key();if let Some(vec)=
engine.embed_async(text).await{let conn=db.lock().await;let _=persist_decision_embedding(&conn,id,&vec,model_key);}});}}if!
benchmark_store{crate::focus::focus_append(&conn,&source_agent,&decision_text);}json_response(StatusCode::OK,json!({"stored":true,
"entry":entry}))}Err(StoreError::BadRequest(message))=>json_response(StatusCode::BAD_REQUEST,json!({"error":message})),Err(
StoreError::Validation{message,quality,factors})=>json_response(StatusCode::BAD_REQUEST,json!({"error":message,"quality":quality,
"factors":factors.as_json(),}),),Err(StoreError::Internal(err))=>json_response(StatusCode::INTERNAL_SERVER_ERROR,json!({"error":
format!("Store failed: {err}")})),}}
