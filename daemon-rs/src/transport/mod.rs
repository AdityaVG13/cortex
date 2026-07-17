use crate::auth::CortexPaths;use tokio::io::{AsyncRead,AsyncReadExt,AsyncWrite,AsyncWriteExt};fn normalized_host(value:&str)->
String{value.trim().trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase()}pub(crate)fn http_host_for_bind(bind:&str)
->String{let bind=bind.trim();if bind.is_empty()||matches!(bind,"0.0.0.0"|"::"|"[::]"){"127.0.0.1".to_string()}else if bind.
starts_with('[')&&bind.ends_with(']'){bind.to_string()}else if bind.contains(':'){format!("[{bind}]")}else{bind.to_string()}}pub
fn local_http_base_url(paths:&CortexPaths)->String{let host=http_host_for_bind(&paths.bind);format!("http://{host}:{}",paths.port)
}pub fn is_local_http_base_url(base_url:&str,paths:&CortexPaths)->bool{let Ok(url)=reqwest::Url::parse(base_url)else{return false;
};let Some(host)=url.host_str()else{return false;};if url.port_or_known_default()!=Some(paths.port){return false;}let host_norm=
normalized_host(host);let bind_norm=normalized_host(&paths.bind);matches!(host_norm.as_str(),"127.0.0.1"|"localhost"|"::1")||(!
bind_norm.is_empty()&&!matches!(bind_norm.as_str(),"0.0.0.0"|"::")&&host_norm==bind_norm)}pub fn local_ipc_endpoint_for_base_url(
base_url:&str,paths:&CortexPaths)->Option<String>{if!is_local_http_base_url(base_url,paths){return None;}paths.ipc_endpoint.clone(
)}fn split_base_and_path(url:&str)->Option<(String,String)>{let parsed=reqwest::Url::parse(url).ok()?;let mut base=parsed.clone();
base.set_path("");base.set_query(None);base.set_fragment(None);let mut path=parsed.path().to_string();if path.is_empty(){path.push
('/');}if let Some(query)=parsed.query(){path.push('?');path.push_str(query);}Some((base.to_string().trim_end_matches('/').
to_string(),path))}pub(crate)fn parse_http_response_bytes(raw:&[u8],source_label:&str)->Result<(reqwest::StatusCode,String),String
>{let Some(header_end)=raw.windows(4).position(|window|window==b"\r\n\r\n")else{return Err(format!(
"invalid HTTP response from {source_label}"));};let header=std::str::from_utf8(&raw[..header_end]).map_err(|_|format!(
"{source_label} response headers are not valid UTF-8"))?;let status_line=header.lines().next().ok_or_else(||format!(
"{source_label} response missing valid HTTP status line"))?;let status=parse_http_status_line(status_line,source_label)?;let body=
String::from_utf8_lossy(&raw[header_end+4..]).to_string();Ok((status,body))}fn parse_http_status_line(status_line:&str,
source_label:&str)->Result<reqwest::StatusCode,String>{let mut fields=status_line.split_whitespace();let version=fields.next().
ok_or_else(||format!("{source_label} response missing HTTP version"))?;if!matches!(version,"HTTP/1.0"|"HTTP/1.1"){return Err(
format!("{source_label} response has unsupported HTTP version '{version}'"));}let status_code_raw=fields.next().ok_or_else(||
format!("{source_label} response missing status code"))?;if status_code_raw.len()!=3||!status_code_raw.bytes().all(|byte|byte.
is_ascii_digit()){return Err(format!("{source_label} response has malformed status code '{status_code_raw}'"));}let status_code=
status_code_raw.parse::<u16>().map_err(|_|format!("{source_label} response has malformed status code"))?;reqwest::StatusCode::
from_u16(status_code).map_err(|_|format!("{source_label} response returned invalid status code {status_code}"))}fn
parse_http_response(raw:&[u8])->Result<(reqwest::StatusCode,String),String>{parse_http_response_bytes(raw,"IPC endpoint")}async fn
send_http_over_stream<S>(stream:&mut S,method:&str,path:&str,headers:&[(String,String)],body:Option<&str>)->Result<(reqwest::
StatusCode,String),String>where S:AsyncRead+AsyncWrite+Unpin,{let body=body.unwrap_or("");let mut request=String::new();request.
push_str(method);request.push(' ');request.push_str(path);request.push_str(" HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n"
);for(name,value)in headers{request.push_str(name);request.push_str(": ");request.push_str(value);request.push_str("\r\n");}
request.push_str("Content-Length: ");request.push_str(&body.len().to_string());request.push_str("\r\n\r\n");request.push_str(body)
;stream.write_all(request.as_bytes()).await.map_err(|e|format!("IPC write failed: {e}"))?;stream.flush().await.map_err(|e|format!(
"IPC flush failed: {e}"))?;let mut response=Vec::new();stream.read_to_end(&mut response).await.map_err(|e|format!(
"IPC read failed: {e}"))?;parse_http_response(&response)}async fn ipc_http_request(endpoint:&str,method:&str,path:&str,headers:&[(
String,String)],body:Option<&str>,timeout:std::time::Duration)->Result<(reqwest::StatusCode,String),String>{let fut=async{#[cfg(
unix)]{let mut stream=tokio::net::UnixStream::connect(endpoint).await.map_err(|e|format!("IPC connect failed: {e}"))?;return
send_http_over_stream(&mut stream,method,path,headers,body).await;}#[cfg(windows)]{let mut stream=tokio::net::windows::named_pipe
::ClientOptions::new().open(endpoint).map_err(|e|format!("IPC connect failed: {e}"))?;return send_http_over_stream(&mut stream,
method,path,headers,body).await;}#[allow(unreachable_code)]Err("IPC transport is unsupported on this platform".to_string())};tokio
::time::timeout(timeout,fut).await.map_err(|_|"IPC request timed out".to_string())?}async fn send_http_request(client:&reqwest::
Client,method:&str,url:&str,headers:&[(String,String)],body:Option<&str>,timeout:std::time::Duration,)->Result<(reqwest::
StatusCode,String),String>{let mut req=match method{"GET"=>client.get(url),"POST"=>client.post(url),other=>return Err(format!(
"Unsupported request method '{other}'")),};req=req.timeout(timeout);for(name,value)in headers{req=req.header(name,value);}if let
Some(payload)=body{req=req.body(payload.to_string());}let response=req.send().await.map_err(|e|e.to_string())?;let status=response
.status();let body=response.text().await.map_err(|e|e.to_string())?;Ok((status,body))}#[allow(clippy::too_many_arguments)]pub
async fn request_with_local_ipc_fallback(client:&reqwest::Client,method:&str,base_url:&str,path:&str,paths:&CortexPaths,headers:&[
(String,String)],body:Option<&str>,timeout:std::time::Duration,)->Result<(reqwest::StatusCode,String),String>{if let Some(endpoint
)=local_ipc_endpoint_for_base_url(base_url,paths){match ipc_http_request(&endpoint,method,path,headers,body,timeout).await{Ok(
response)=>return Ok(response),Err(err)=>{eprintln!(
"[cortex-transport] IPC request failed for {method} {path} ({endpoint}): {err}; falling back to HTTP");}}}let normalized_base=
base_url.trim_end_matches('/');let normalized_path=if path.starts_with('/'){path.to_string()}else{format!("/{path}")};let url=
format!("{normalized_base}{normalized_path}");send_http_request(client,method,&url,headers,body,timeout).await}pub async fn
request_url_with_local_ipc_fallback(client:&reqwest::Client,method:&str,url:&str,paths:&CortexPaths,headers:&[(String,String)],
body:Option<&str>,timeout:std::time::Duration,)->Result<(reqwest::StatusCode,String),String>{if let Some((base_url,path))=
split_base_and_path(url){return request_with_local_ipc_fallback(client,method,&base_url,&path,paths,headers,body,timeout).await;}
send_http_request(client,method,url,headers,body,timeout).await}#[cfg(test)]mod tests;
