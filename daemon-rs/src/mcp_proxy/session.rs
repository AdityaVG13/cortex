use crate::auth::CortexPaths;use crate::daemon_lifecycle;use std::path::{Path,PathBuf};use std::sync::{Mutex,OnceLock};use sysinfo
::{ProcessesToUpdate,System};use tokio::io::{AsyncReadExt,AsyncWriteExt};pub(crate)const HEALTH_CHECK_ATTEMPTS:u32
=5;pub(crate)const REQUEST_ATTEMPTS:u32=3;pub(crate)const SESSION_HEARTBEAT_SECS:u64=15;pub(crate)const SESSION_RESTART_ATTEMPTS:
u32=4;pub(crate)const SESSION_RESTART_DELAY_MS:u64=250;pub(crate)const HEARTBEAT_RECOVERY_FAILURES:u32=5;pub(crate)const
STARTUP_IDLE_TIMEOUT_SECS:u64=60;pub(crate)const ORPHAN_CHECK_SECS:u64=15;pub(crate)const MAX_AGENT_HEADER_LEN:usize=160;pub(crate
)const MAX_MODEL_HEADER_LEN:usize=160;pub(crate)const AUTH_TOKEN_CACHE_TTL_MS:u64=1_000;pub(crate)const STDIN_CHANNEL_CAPACITY:
usize=32;#[derive(Default)]pub(crate)struct AuthTokenCacheEntry{token_path:Option<PathBuf>,token:Option<String>,read_at:Option<std
::time::Instant>,}static AUTH_TOKEN_CACHE:OnceLock<Mutex<AuthTokenCacheEntry>>=OnceLock::new();pub(crate)fn auth_token_cache()->&
'static Mutex<AuthTokenCacheEntry>{AUTH_TOKEN_CACHE.get_or_init(||Mutex::new(AuthTokenCacheEntry::default()))}#[cfg(test)]static
AUTH_TOKEN_CACHE_TEST_LOCK:OnceLock<Mutex<()>>=OnceLock::new();#[cfg(test)]pub(crate)fn auth_token_cache_test_lock()->&'static
Mutex<()>{AUTH_TOKEN_CACHE_TEST_LOCK.get_or_init(||Mutex::new(()))}pub(crate)fn read_auth_token()->Option<String>{let token_path=
crate::auth::CortexPaths::resolve().token;read_auth_token_with_cache(&token_path)}pub(crate)fn read_auth_token_with_cache(
token_path:&Path)->Option<String>{#[cfg(test)]let _guard=auth_token_cache_test_lock().lock().ok();read_auth_token_with_cache_inner
(token_path)}pub(crate)fn read_auth_token_with_cache_inner(token_path:&Path)->Option<String>{let now=std::time::Instant::now();if
let Ok(cache)=auth_token_cache().lock(){if cache.token_path.as_deref()==Some(token_path){if let Some(read_at)=cache.read_at{if now
.duration_since(read_at).as_millis()<AUTH_TOKEN_CACHE_TTL_MS as u128{return cache.token.clone();}}}}let token=
read_auth_token_uncached(token_path);if let Ok(mut cache)=auth_token_cache().lock(){cache.token_path=Some(token_path.to_path_buf()
);cache.token=token.clone();cache.read_at=Some(now);}token}pub(crate)fn read_auth_token_uncached(token_path:&Path)->Option<String>
{match std::fs::read_to_string(token_path){Ok(token)=>{let trimmed=token.trim();if trimmed.is_empty(){eprintln!(
"[cortex-mcp] Auth token file is empty: {}",token_path.display());None}else{Some(trimmed.to_string())}}Err(e)if e.kind()==std::io
::ErrorKind::NotFound=>None,Err(e)=>{eprintln!("[cortex-mcp] Failed to read auth token {}: {e}",token_path.display());None}}}pub(
crate)fn invalidate_auth_token_cache(){#[cfg(test)]let _guard=auth_token_cache_test_lock().lock().ok();
invalidate_auth_token_cache_inner();}pub(crate)fn invalidate_auth_token_cache_inner(){if let Ok(mut cache)=auth_token_cache().lock
(){*cache=AuthTokenCacheEntry::default();}}pub(crate)fn detect_team_mode(api_key:Option<&str>)->bool{api_key.is_some()}pub(crate)
fn startup_idle_timeout()->std::time::Duration{let secs=std::env::var("CORTEX_MCP_HANDSHAKE_TIMEOUT_SECS").ok().and_then(|value|
value.parse::<u64>().ok()).unwrap_or(STARTUP_IDLE_TIMEOUT_SECS);std::time::Duration::from_secs(secs.max(1))}pub(crate)fn
env_trimmed(key:&str)->Option<String>{std::env::var(key).ok().map(|value|value.trim().to_string()).filter(|value|!value.is_empty()
)}pub(crate)fn normalize_header_value(raw:&str,max_len:usize)->Option<String>{let trimmed=raw.trim();if trimmed.is_empty()||
trimmed.len()>max_len{return None;}if!trimmed.is_ascii(){return None;}if trimmed.as_bytes().iter().any(|byte|*byte<=31||*byte==127
){return None;}Some(trimmed.to_string())}pub(crate)fn normalize_api_key(api_key:Option<&str>)->Option<&str>{api_key.map(str::trim)
.filter(|value|!value.is_empty())}pub(crate)fn detect_agent_hint(value:&str)->Option<&'static str>{let value=value.trim().
to_ascii_lowercase();if value.is_empty(){return None;}if value.contains("codex"){return Some("codex");}if value.contains("cursor")
{return Some("cursor");}if value.contains("gemini"){return Some("gemini");}if value.contains("claude"){return Some("claude-code");
}if value.contains("cline"){return Some("cline");}None}pub(crate)fn infer_agent_from_process_tree()->Option<String>{let mut system
=System::new_all();system.refresh_processes(ProcessesToUpdate::All,true);let current_pid=sysinfo::get_current_pid().ok()?;let mut
next_pid=Some(current_pid);let mut depth=0usize;while let Some(pid)=next_pid{let process=system.process(pid)?;let candidates=[
process.name().to_string_lossy().into_owned(),process.exe().map(|path|path.to_string_lossy().into_owned()).unwrap_or_default(),
process.cmd().iter().map(|part|part.to_string_lossy()).collect::<Vec<_>>().join(" "),];for candidate in candidates{if let Some(
agent)=detect_agent_hint(&candidate){return Some(agent.to_string());}}next_pid=process.parent();depth+=1;if depth>=6{break;}}None}
#[derive(Clone,Copy,Debug)]pub(crate)struct ParentProcessRef{pid:sysinfo::Pid,start_time:u64,}pub(crate)fn current_parent_process(
)->Option<ParentProcessRef>{let mut system=System::new_all();system.refresh_processes(ProcessesToUpdate::All,true);let current_pid
=sysinfo::get_current_pid().ok()?;let parent_pid=system.process(current_pid)?.parent()?;let parent=system.process(parent_pid)?;
Some(ParentProcessRef{pid:parent_pid,start_time:parent.start_time(),})}pub(crate)fn process_is_alive(parent:ParentProcessRef)->
bool{let mut system=System::new_all();system.refresh_processes(ProcessesToUpdate::Some(&[parent.pid]),true);system.process(parent.
pid).is_some_and(|process|process.start_time()==parent.start_time)}pub(crate)fn resolve_agent_identity(agent_arg:Option<&str>)->(
String,Option<String>){let model=env_trimmed("CORTEX_AGENT_MODEL").or_else(||env_trimmed("CORTEX_MODEL")).and_then(|value|
normalize_header_value(&value,MAX_MODEL_HEADER_LEN));let mut agent=env_trimmed("CORTEX_AGENT_DISPLAY").or_else(||agent_arg.map(|v|
v.trim().to_string()).filter(|v|!v.is_empty())).or_else(||env_trimmed("CORTEX_AGENT_NAME")).or_else(infer_agent_from_process_tree)
.unwrap_or_else(||"mcp".to_string());if!agent.contains('('){if let Some(model_name)=model.as_deref(){if agent.eq_ignore_ascii_case
("droid"){agent=format!("DROID ({model_name})");}else{agent=format!("{agent} ({model_name})");}}}let agent=match
normalize_header_value(&agent,MAX_AGENT_HEADER_LEN){Some(agent)=>agent,None=>{eprintln!(
"[cortex-mcp] Invalid source agent label after normalization; falling back to 'mcp'");"mcp".to_string()}};(agent,model)}pub(crate)
fn local_daemon_base_from_paths(paths:&CortexPaths)->String{crate::transport::local_http_base_url(paths)}pub(crate)fn
is_local_daemon_base(base_url:&str)->bool{let paths=CortexPaths::resolve();crate::transport::is_local_http_base_url(base_url,&
paths)}pub(crate)fn resolve_local_ipc_endpoint(base_url:&str,api_key:Option<&str>)->Option<String>{if api_key.is_some()||!
is_local_daemon_base(base_url){return None;}CortexPaths::resolve().ipc_endpoint}pub(crate)fn split_base_and_path(url:&str)->Option
<(String,String)>{let parsed=reqwest::Url::parse(url).ok()?;let mut base=parsed.clone();base.set_path("");base.set_query(None);
base.set_fragment(None);let mut path=parsed.path().to_string();if path.is_empty(){path.push('/');}if let Some(query)=parsed.query(
){path.push('?');path.push_str(query);}Some((base.to_string().trim_end_matches('/').to_string(),path))}pub(crate)fn
parse_http_response(raw:&[u8])->Result<(reqwest::StatusCode,String),String>{crate::transport::parse_http_response_bytes(raw,
"IPC endpoint")}pub(crate)async fn send_http_over_stream<S>(stream:&mut S,method:&str,path:&str,headers:&[(String,String)],body:
Option<&str>)->Result<(reqwest::StatusCode,String),String>where S:tokio::io::AsyncRead+tokio::io::AsyncWrite+Unpin,{let body=body.
unwrap_or("");let mut request=String::new();request.push_str(method);request.push(' ');request.push_str(path);request.push_str(
" HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n");for(name,value)in headers{request.push_str(name);request.push_str(": ");
request.push_str(value);request.push_str("\r\n");}request.push_str("Content-Length: ");request.push_str(&body.len().to_string());
request.push_str("\r\n\r\n");request.push_str(body);stream.write_all(request.as_bytes()).await.map_err(|e|format!(
"IPC write failed: {e}"))?;stream.flush().await.map_err(|e|format!("IPC flush failed: {e}"))?;let mut response=Vec::new();stream.
read_to_end(&mut response).await.map_err(|e|format!("IPC read failed: {e}"))?;parse_http_response(&response)}pub(crate)async fn
ipc_http_request(endpoint:&str,method:&str,path:&str,headers:&[(String,String)],body:Option<&str>,timeout:std::time::Duration,)->
Result<(reqwest::StatusCode,String),String>{let fut=async{#[cfg(unix)]{let mut stream=tokio::net::UnixStream::connect(endpoint).
await.map_err(|e|format!("IPC connect failed: {e}"))?;return send_http_over_stream(&mut stream,method,path,headers,body).await;}#[
cfg(windows)]{let mut stream=tokio::net::windows::named_pipe::ClientOptions::new().open(endpoint).map_err(|e|format!(
"IPC connect failed: {e}"))?;return send_http_over_stream(&mut stream,method,path,headers,body).await;}#[allow(unreachable_code)]
Err("IPC transport is unsupported on this platform".to_string())};tokio::time::timeout(timeout,fut).await.map_err(|_|
"IPC request timed out".to_string())?}#[allow(clippy::too_many_arguments)]pub(crate)async fn transport_request(client:&reqwest::
Client,method:&str,base_url:&str,path:&str,api_key:Option<&str>,allow_local_token_fallback:bool,headers:&[(String,String)],body:
Option<&str>,timeout:std::time::Duration,)->Result<(reqwest::StatusCode,String),String>{let mut all_headers=Vec::with_capacity(
headers.len()+1);all_headers.extend_from_slice(headers);if let Some(auth)=build_auth_header(base_url,api_key,
allow_local_token_fallback){all_headers.push(("authorization".to_string(),auth));}if let Some(endpoint)=resolve_local_ipc_endpoint
(base_url,api_key){match ipc_http_request(&endpoint,method,path,&all_headers,body,timeout).await{Ok(response)=>return Ok(response)
,Err(err)=>{eprintln!("[cortex-mcp] IPC request failed for {method} {path} ({endpoint}): {err}; falling back to HTTP");}}}let url=
format!("{base_url}{path}");let mut req=match method{"GET"=>client.get(&url),"POST"=>client.post(&url),other=>return Err(format!(
"Unsupported request method '{other}'")),};req=req.timeout(timeout);for(name,value)in&all_headers{req=req.header(name,value);}if
let Some(payload)=body{req=req.body(payload.to_string());}let response=req.send().await.map_err(|e|e.to_string())?;let status=
response.status();let body=response.text().await.map_err(|e|e.to_string())?;Ok((status,body))}pub(crate)async fn
transport_request_for_url(client:&reqwest::Client,method:&str,url:&str,headers:&[(String,String)],body:Option<&str>,timeout:std::
time::Duration,)->Result<(reqwest::StatusCode,String),String>{let Some((base_url,path))=split_base_and_path(url)else{let mut req=
match method{"GET"=>client.get(url),"POST"=>client.post(url),other=>return Err(format!("Unsupported request method '{other}'")),};
req=req.timeout(timeout);for(name,value)in headers{req=req.header(name,value);}if let Some(payload)=body{req=req.body(payload.
to_string());}let response=req.send().await.map_err(|e|e.to_string())?;let status=response.status();let body=response.text().await
.map_err(|e|e.to_string())?;return Ok((status,body));};transport_request(client,method,&base_url,&path,None,false,headers,body,
timeout).await}pub(crate)fn local_token_fallback_required(base_url:&str,api_key:Option<&str>)->bool{api_key.is_none()&&
is_local_daemon_base(base_url)}pub(crate)fn build_auth_header(base_url:&str,api_key:Option<&str>,allow_local_token_fallback:bool)
->Option<String>{if let Some(key)=api_key{return Some(format!("Bearer {key}"));}if allow_local_token_fallback&&
local_token_fallback_required(base_url,api_key){return read_auth_token().map(|token|format!("Bearer {token}"));}None}pub(crate)fn
requires_explicit_api_key(base_url:&str,api_key:Option<&str>)->bool{api_key.is_none()&&!is_local_daemon_base(base_url)}pub(crate)
fn validate_target_base_url(base_url:&str)->Result<(),String>{let parsed=reqwest::Url::parse(base_url).map_err(|_|format!(
"Invalid Cortex target URL '{base_url}'. Use an absolute http:// or https:// base URL."))?;if!matches!(parsed.scheme(),"http"|
"https"){return Err(format!("Unsupported Cortex target URL scheme '{}' in '{base_url}'. Use http or https.",parsed.scheme()));}if
parsed.host_str().is_none(){return Err(format!("Invalid Cortex target URL '{base_url}': missing host."));}if!parsed.username().
is_empty()||parsed.password().is_some(){return Err(
"Cortex target URL must not include embedded credentials; pass --api-key instead.".to_string());}if parsed.query().is_some()||
parsed.fragment().is_some(){return Err("Cortex target URL must not include query parameters or fragments.".to_string());}Ok(())}
pub(crate)fn expected_port_from_url(url:&str)->Option<u16>{reqwest::Url::parse(url).ok().and_then(|parsed|parsed.
port_or_known_default())}pub(crate)fn fallback_health_probe_url(probe_url:&str)->Option<String>{probe_url.strip_suffix(
"/readiness").map(|base|format!("{base}/health"))}pub(crate)fn internal_health_probe_headers()->[(String,String);1]{[(String::from
("X-Cortex-Request"),String::from("true"))]}pub(crate)fn is_cortex_health_response(status:reqwest::StatusCode,body:&str,probe_url:
&str)->bool{let local_paths=if is_local_daemon_base(probe_url){Some(CortexPaths::resolve())}else{None};if let Some(ready)=
daemon_lifecycle::readiness_state_from_payload(status.as_u16(),body,expected_port_from_url(probe_url),local_paths.as_ref()){return
ready;}daemon_lifecycle::is_cortex_health_payload(status.as_u16(),body,expected_port_from_url(probe_url),local_paths.as_ref())}pub
(crate)async fn health_check_ready(client:&reqwest::Client,probe_url:&str)->bool{let probe_headers=internal_health_probe_headers()
;let(status,body)=match transport_request_for_url(client,"GET",probe_url,&probe_headers,None,std::time::Duration::from_secs(5)).
await{Ok(response)=>response,Err(_)=>return false,};if is_cortex_health_response(status,&body,probe_url){return true;}let Some(
health_url)=fallback_health_probe_url(probe_url)else{return false;};let(status,body)=match transport_request_for_url(client,"GET",
&health_url,&probe_headers,None,std::time::Duration::from_secs(5)).await{Ok(response)=>response,Err(_)=>return false,};
is_cortex_health_response(status,&body,&health_url)}pub(crate)fn is_auth_recovery_status(status:reqwest::StatusCode)->bool{status
==reqwest::StatusCode::UNAUTHORIZED||status==reqwest::StatusCode::FORBIDDEN}pub(crate)async fn recover_solo_auth(client:&reqwest::
Client,health_url:&str,base_url:&str,agent:&str,model:Option<&str>,allow_local_token_fallback:&mut bool)->bool{if!
health_check_ready(client,health_url).await{*allow_local_token_fallback=false;return false;}*allow_local_token_fallback=true;if!
session_start_with_retry(client,base_url,None,agent,model,*allow_local_token_fallback).await{eprintln!(
"[cortex-mcp] Auth recovered but session re-registration did not succeed yet");return false;}true}pub(crate)async fn
session_start_with_retry(client:&reqwest::Client,base_url:&str,api_key:Option<&str>,agent:&str,model:Option<&str>,
allow_local_token_fallback:bool)->bool{for attempt in 1..=SESSION_RESTART_ATTEMPTS.max(1){if session_start(client,base_url,api_key
,agent,model,allow_local_token_fallback).await{return true;}if attempt<SESSION_RESTART_ATTEMPTS{tokio::time::sleep(std::time::
Duration::from_millis(SESSION_RESTART_DELAY_MS*attempt as u64)).await;}}false}pub(crate)fn persist_write_buffer(buffer_path:&std::
path::Path,remaining:&[String])->Result<(),std::io::Error>{use std::io::{BufWriter,Write};let parent=buffer_path.parent().filter(|
parent|!parent.as_os_str().is_empty()).unwrap_or_else(||std::path::Path::new("."));std::fs::create_dir_all(parent)?;let mut tmp=
tempfile::NamedTempFile::new_in(parent)?;{let mut writer=BufWriter::new(tmp.as_file_mut());for line in remaining{writeln!(writer,
"{line}")?;}writer.flush()?;}tmp.as_file().sync_all()?;tmp.persist(buffer_path).map_err(|err|err.error)?;sync_parent_dir(parent)?;
Ok(())}#[cfg(unix)]pub(crate)fn sync_parent_dir(parent:&std::path::Path)->std::io::Result<()>{std::fs::File::open(parent)?.
sync_all()}#[cfg(not(unix))]pub(crate)fn sync_parent_dir(_parent:&std::path::Path)->std::io::Result<()>{Ok(())}pub(crate)async fn
drain_write_buffer(client:&reqwest::Client,base_url:&str,api_key:Option<&str>,agent:&str,model:Option<&str>,paths:&CortexPaths,
allow_local_token_fallback:bool){let buffer_path=&paths.write_buffer;let content=match std::fs::read_to_string(buffer_path){Ok(
content)if!content.trim().is_empty()=>content,_=>return,};let lines:Vec<String>=content.lines().map(str::trim).filter(|line|!line.
is_empty()).map(|line|line.to_string()).collect();if lines.is_empty(){return;}let mut remaining=Vec::new();let mut drained=0usize;
for line in lines{let mut headers=vec![("content-type".to_string(),"application/json".to_string()),("x-cortex-request".to_string()
,"true".to_string()),("x-source-agent".to_string(),agent.to_string()),];if let Some(model_name)=model{headers.push((
"x-source-model".to_string(),model_name.to_string()));}match transport_request(client,"POST",base_url,"/mcp-rpc",api_key,
allow_local_token_fallback,&headers,Some(&line),std::time::Duration::from_secs(10),).await{Ok((status,_))if status.is_success()=>{
drained+=1;}_=>remaining.push(line),}}if let Err(e)=persist_write_buffer(buffer_path,&remaining){eprintln!(
"[cortex-mcp] Failed to compact write buffer {}: {e}",buffer_path.display());return;}if drained>0{eprintln!(
"[cortex-mcp] Drained {drained} buffered writes and compacted {}",buffer_path.display());}}pub(crate)async fn session_start(client
:&reqwest::Client,base_url:&str,api_key:Option<&str>,agent:&str,model:Option<&str>,allow_local_token_fallback:bool)->bool{let
payload=serde_json::json!({"agent":agent,"ttl":7200,"description":model.map(|m|format!("MCP session - {m}")).unwrap_or_else(||
"MCP session".to_string())}).to_string();let headers=vec![("content-type".to_string(),"application/json".to_string()),(
"x-cortex-request".to_string(),"true".to_string())];match transport_request(client,"POST",base_url,"/session/start",api_key,
allow_local_token_fallback,&headers,Some(&payload),std::time::Duration::from_secs(10),).await{Ok((status,_))=>status.is_success(),
Err(_)=>false,}}pub(crate)enum SessionHeartbeatOutcome{Renewed,MissingSession,Failed,}pub(crate)async fn session_heartbeat(client:
&reqwest::Client,base_url:&str,api_key:Option<&str>,agent:&str,model:Option<&str>,allow_local_token_fallback:bool)->
SessionHeartbeatOutcome{let payload=serde_json::json!({"agent":agent,"description":model.map(|m|format!("MCP session - {m}")).
unwrap_or_else(||"MCP session".to_string())}).to_string();let headers=vec![("content-type".to_string(),"application/json".
to_string()),("x-cortex-request".to_string(),"true".to_string())];match transport_request(client,"POST",base_url,
"/session/heartbeat",api_key,allow_local_token_fallback,&headers,Some(&payload),std::time::Duration::from_secs(8),).await{Ok((
status,_))if status.is_success()=>SessionHeartbeatOutcome::Renewed,Ok((status,_))if status==reqwest::StatusCode::NOT_FOUND=>
SessionHeartbeatOutcome::MissingSession,Ok(_)|Err(_)=>SessionHeartbeatOutcome::Failed,}}pub(crate)async fn session_end(client:&
reqwest::Client,base_url:&str,api_key:Option<&str>,agent:&str,allow_local_token_fallback:bool)->bool{let payload=serde_json::json!
({"agent":agent}).to_string();let headers=vec![("content-type".to_string(),"application/json".to_string()),("x-cortex-request".
to_string(),"true".to_string())];match transport_request(client,"POST",base_url,"/session/end",api_key,allow_local_token_fallback,
&headers,Some(&payload),std::time::Duration::from_secs(8),).await{Ok((status,_))=>status.is_success(),Err(_)=>false,}}pub(crate)
async fn finalize_proxy_session(client:&reqwest::Client,base_url:&str,api_key:Option<&str>,agent:&str,allow_local_token_fallback:
bool){let _=session_end(client,base_url,api_key,agent,allow_local_token_fallback).await;}
