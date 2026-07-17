use super::common::{ensure_remote_target_has_api_key,is_local_client_base_url,local_daemon_base_url,parse_flag_usize,
parse_flag_value,resolve_client_target,validate_cli_options};use super::daemon::ensure_daemon;use crate::auth;use crate::
daemon_lifecycle::daemon_healthy;use crate::transport;use std::time::Duration;const DEFAULT_BOOT_BUDGET:usize=600;pub(crate)fn
read_auth_token_from_path(token_path:&std::path::Path)->Option<String>{std::fs::read_to_string(token_path).ok().and_then(|token|{
let trimmed=token.trim();if trimmed.is_empty(){None}else{Some(trimmed.to_string())}})}pub(crate)fn resolve_boot_auth_header(
token_path:&std::path::Path,api_key:Option<&str>,allow_local_token_fallback:bool)->Option<String>{if let Some(api_key)=api_key{let
trimmed=api_key.trim();if!trimmed.is_empty(){return Some(format!("Bearer {trimmed}"));}}if allow_local_token_fallback{return
read_auth_token_from_path(token_path).map(|token|format!("Bearer {token}"));}None}pub(crate)async fn request_boot_payload(paths:&
auth::CortexPaths,base_url:&str,token_path:&std::path::Path,api_key:Option<&str>,allow_local_token_fallback:bool,agent:&str,budget
:usize,)->Result<serde_json::Value,String>{let client=reqwest::Client::builder().timeout(Duration::from_secs(10)).build().map_err(
|e|format!("create boot client: {e}"))?;let mut boot_url=reqwest::Url::parse(&format!("{}/boot",base_url.trim_end_matches('/'))).
map_err(|e|format!("invalid boot URL '{base_url}': {e}"))?;boot_url.query_pairs_mut().append_pair("agent",agent).append_pair(
"budget",&budget.to_string());let mut headers=vec![("x-cortex-request".to_string(),"true".to_string()),("x-source-agent".to_string
(),agent.to_string())];if let Some(auth)=resolve_boot_auth_header(token_path,api_key,allow_local_token_fallback){headers.push((
"authorization".to_string(),auth));}let(status,body)=transport::request_url_with_local_ipc_fallback(&client,"GET",boot_url.as_ref(
),paths,&headers,None,Duration::from_secs(10)).await.map_err(|e|format!("boot request failed: {e}"))?;if!status.is_success(){let
detail=body.trim();return if detail.is_empty(){Err(format!("boot returned {status}"))}else{Err(format!(
"boot returned {status}: {detail}"))};}serde_json::from_str::<serde_json::Value>(&body).map_err(|e|format!(
"parse boot response failed: {e}"))}pub(crate)async fn run_boot_cli(paths:&auth::CortexPaths,args:&[String])->Result<(),String>{
validate_cli_options(args,&["--agent","--budget","--url","--api-key"],&["--json"])?;let agent=parse_flag_value(args,"--agent").
unwrap_or_else(||"cli".to_string());let agent=agent.trim();if agent.is_empty(){return Err("agent cannot be empty".to_string());}
let budget=parse_flag_usize(args,"--budget")?.unwrap_or(DEFAULT_BOOT_BUDGET);let json_output=args.iter().any(|arg|arg=="--json");
let(base_url,api_key,local_owner_mode)=resolve_client_target(args,paths);ensure_remote_target_has_api_key(&base_url,api_key.
as_deref(),paths)?;if local_owner_mode{ensure_daemon(paths,None,false,false).await?;}let local_target_identity_valid=if
local_owner_mode{false}else if is_local_client_base_url(&base_url,paths){daemon_healthy(paths).await}else{false};let
allow_local_token_fallback=local_owner_mode||local_target_identity_valid;let payload=request_boot_payload(paths,&base_url,&paths.
token,api_key.as_deref(),allow_local_token_fallback,agent,budget).await?;if json_output{println!("{}",serde_json::to_string_pretty
(&payload).map_err(|e|format!("serialize boot response failed: {e}"))?);}else{let boot_prompt=payload.get("bootPrompt").and_then(|
value|value.as_str()).ok_or_else(||"boot response missing bootPrompt".to_string())?;println!("{boot_prompt}");}Ok(())}pub(crate)
async fn boot_agent(paths:&auth::CortexPaths,agent:&str)->Result<(),String>{let base_url=local_daemon_base_url(paths);
request_boot_payload(paths,&base_url,&paths.token,None,true,agent,200).await.map(|_|())}
