use crate::handlers::{estimate_tokens,estimate_tokens_from_chars};use rusqlite::{params,Connection,OptionalExtension};use
serde_json::{json,Value};use std::collections::HashSet;use std::path::Path;pub(crate)fn get_last_boot_time(conn:&Connection,agent:
&str)->Option<String>{conn.query_row(
"SELECT data FROM events WHERE type = 'agent_boot' AND source_agent = ?1 ORDER BY created_at DESC LIMIT 1",params![agent],|r|r.get
::<_,String>(0),).ok().and_then(|data|serde_json::from_str::<Value>(&data).ok()?.get("timestamp")?.as_str().map(|s|s.to_string()))
}pub(crate)fn fetch_messages_for_agent(conn:&Connection,agent:&str)->Vec<Value>{let mut out=Vec::new();if let Ok(mut stmt)=conn.
prepare("SELECT sender, message FROM messages WHERE recipient = ?1 ORDER BY timestamp ASC"){if let Ok(rows)=stmt.query_map(params!
[agent],|r|{Ok(json!({"from":r.get::<_,String>(0)?,"message":r.get::<_,String>(1)?}))}){for row in rows.flatten(){out.push(row);}}
}out}pub(crate)fn fetch_sessions(conn:&Connection)->Vec<Value>{let mut out=Vec::new();if let Ok(mut stmt)=conn.prepare(
"SELECT agent, project, description, files_json FROM sessions WHERE expires_at > ?1"){let now=chrono::Utc::now().to_rfc3339_opts(
chrono::SecondsFormat::Millis,true);if let Ok(rows)=stmt.query_map(params![now],|r|{let files_json:String=r.get(3)?;Ok(json!({
"agent":r.get::<_,String>(0)?,"project":r.get::<_,Option<String>>(1)?,"description":r.get::<_,Option<String>>(2)?,"files":
serde_json::from_str::<Value>(&files_json).unwrap_or(json!([]))}))}){for row in rows.flatten(){out.push(row);}}}out}pub(crate)fn
fetch_locks(conn:&Connection)->Vec<Value>{let mut out=Vec::new();let now=chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat
::Millis,true);if let Ok(mut stmt)=conn.prepare("SELECT path, agent, expires_at FROM locks WHERE expires_at > ?1"){if let Ok(rows)
=stmt.query_map(params![now],|r|{Ok(json!({"path":r.get::<_,String>(0)?,"agent":r.get::<_,String>(1)?,"expiresAt":r.get::<_,String
>(2)?}))}){for row in rows.flatten(){out.push(row);}}}out}pub(crate)fn fetch_unread_feed(conn:&Connection,agent:&str)->Vec<Value>{
let ack:Option<String>=conn.query_row("SELECT last_seen_id FROM feed_acks WHERE agent = ?1",params![agent],|row|row.get(0)).
optional().ok().flatten();let mut all:Vec<(String,String,String,String)>=Vec::new();if let Ok(mut stmt)=conn.prepare(
"SELECT id, agent, kind, summary FROM feed ORDER BY timestamp ASC"){if let Ok(rows)=stmt.query_map([],|r|Ok((r.get::<_,String>(0)?
,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))){for row in rows.flatten(){all.push(row);}}}if let Some(
ack_id)=ack{let mut past_ack=false;let mut unread=Vec::new();for(id,entry_agent,kind,summary)in all{if id==ack_id{past_ack=true;
continue;}if past_ack&&entry_agent!=agent{unread.push(json!({"kind":kind,"agent":entry_agent,"summary":summary}));}}unread}else{
all.into_iter().filter(|(_,entry_agent,_,_)|entry_agent!=agent).map(|(_,entry_agent,kind,summary)|{json!({"kind":kind,"agent":
entry_agent,"summary":summary})}).collect()}}pub(crate)fn fetch_pending_tasks(conn:&Connection)->Vec<Value>{let mut out=Vec::new()
;if let Ok(mut stmt)=conn.prepare(
"SELECT task_id, title, priority, project, files_json FROM tasks WHERE status = 'pending' ORDER BY created_at ASC LIMIT 5"){if let
Ok(rows)=stmt.query_map([],|r|{let files_json:String=r.get(4)?;Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,
"priority":r.get::<_,String>(2)?,"project":r.get::<_,Option<String>>(3)?,"files":serde_json::from_str::<Value>(&files_json).
unwrap_or(json!([]))}))}){for row in rows.flatten(){out.push(row);}}}out}pub(crate)fn fetch_claimed_tasks_for_agent(conn:&
Connection,agent:&str)->Vec<Value>{let mut out=Vec::new();if let Ok(mut stmt)=conn.prepare(
"SELECT task_id, title, priority, claimed_at FROM tasks WHERE status = 'claimed' AND claimed_by = ?1 ORDER BY claimed_at ASC"){if
let Ok(rows)=stmt.query_map(params![agent],|r|{Ok(json!({"id":r.get::<_,String>(0)?,"title":r.get::<_,String>(1)?,"priority":r.get
::<_,String>(2)?,"claimedAt":r.get::<_,Option<String>>(3)?}))}){for row in rows.flatten(){out.push(row);}}}out}pub(crate)fn
build_delta_capsule(conn:&Connection,agent:&str)->(String,usize,String){let last_boot=get_last_boot_time(conn,agent);let mut parts
:Vec<String>=Vec::new();let messages=fetch_messages_for_agent(conn,agent);if!messages.is_empty(){let lines:Vec<String>=messages.
iter().map(|m|{let from=m.get("from").and_then(|v|v.as_str()).unwrap_or("?");let msg=m.get("message").and_then(|v|v.as_str()).
unwrap_or("");let truncated:String=msg.chars().take(200).collect();format!("- From {from}: \"{truncated}\"")}).collect();parts.
push(format!("## Pending Messages\n{}",lines.join("\n")));}let sessions=fetch_sessions(conn);let other_sessions:Vec<&Value>=
sessions.iter().filter(|s|s.get("agent").and_then(|v|v.as_str())!=Some(agent)).collect();if!other_sessions.is_empty(){let lines:
Vec<String>=other_sessions.iter().map(|s|{let ag=s.get("agent").and_then(|v|v.as_str()).unwrap_or("?");let proj=s.get("project").
and_then(|v|v.as_str()).unwrap_or("unknown");let desc=s.get("description").and_then(|v|v.as_str()).unwrap_or("no description");
format!("- {ag} working on {proj}: \"{desc}\"")}).collect();parts.push(format!("## Active Agents\n{}",lines.join("\n")));}let
locks=fetch_locks(conn);if!locks.is_empty(){let lines:Vec<String>=locks.iter().map(|l|{let path=l.get("path").and_then(|v|v.as_str
()).unwrap_or("?");let ag=l.get("agent").and_then(|v|v.as_str()).unwrap_or("?");format!("- {path} locked by {ag}")}).collect();
parts.push(format!("## Active Locks\n{}",lines.join("\n")));}let mut feed=fetch_unread_feed(conn,agent);if feed.len()>10{feed=feed
.split_off(feed.len()-10);}if!feed.is_empty(){let lines:Vec<String>=feed.iter().map(|e|{let kind=e.get("kind").and_then(|v|v.
as_str()).unwrap_or("?");let ag=e.get("agent").and_then(|v|v.as_str()).unwrap_or("?");let summary=e.get("summary").and_then(|v|v.
as_str()).unwrap_or("");format!("- [{kind}] {ag}: {summary}")}).collect();parts.push(format!("## Feed\n{}",lines.join("\n")));}let
pending_tasks=fetch_pending_tasks(conn);if!pending_tasks.is_empty(){let lines:Vec<String>=pending_tasks.iter().map(|t|{let pri=t.
get("priority").and_then(|v|v.as_str()).unwrap_or("?");let title=t.get("title").and_then(|v|v.as_str()).unwrap_or("?");format!(
"- [{pri}] {title}")}).collect();parts.push(format!("## Pending Tasks\n{}",lines.join("\n")));}let my_tasks=
fetch_claimed_tasks_for_agent(conn,agent);if!my_tasks.is_empty(){let lines:Vec<String>=my_tasks.iter().map(|t|{let pri=t.get(
"priority").and_then(|v|v.as_str()).unwrap_or("?");let title=t.get("title").and_then(|v|v.as_str()).unwrap_or("?");format!(
"- [{pri}] {title}")}).collect();parts.push(format!("## Your Active Tasks\n{}",lines.join("\n")));}if let Ok(mut stmt)=conn.
prepare("SELECT id, decision, source_agent, disputes_id FROM decisions WHERE status = 'disputed' ORDER BY created_at DESC LIMIT 6"
){if let Ok(rows)=stmt.query_map([],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,Option<i64>>(3
)?))){let mut seen=HashSet::new();let mut lines:Vec<String>=Vec::new();for(id,decision,source_agent,disputes_id)in rows.flatten(){
if seen.contains(&id){continue;}seen.insert(id);if let Some(did)=disputes_id{seen.insert(did);}let mut line=format!(
"#{id} ({source_agent}): {decision}");if let Some(did)=disputes_id{if let Ok((partner_dec,partner_agent))=conn.query_row(
"SELECT decision, source_agent FROM decisions WHERE id = ?1",params![did],|r|{Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))}){
line.push_str(&format!(" vs #{did} ({partner_agent}): {partner_dec}"));}}lines.push(line);}if!lines.is_empty(){parts.push(format!(
"CONFLICTS:\n{}",lines.iter().map(|l|format!("- {l}")).collect::<Vec<_>>().join("\n")));}}}if let Some(focus)=crate::focus::
focus_current(conn,agent){let label=focus.get("label").and_then(|v|v.as_str()).unwrap_or("?");let entries=focus.get("entries").
and_then(|v|v.as_u64()).unwrap_or(0);parts.push(format!("## Active Focus\n- {label} ({entries} entries)"));}if let Some(ref lb)=
last_boot{if let Ok(mut stmt)=conn.prepare(
"SELECT decision, context, source_agent FROM decisions WHERE status = 'active' AND created_at >= ?1 ORDER BY created_at DESC LIMIT 5"
){if let Ok(rows)=stmt.query_map(params![lb],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?,r.get::<_,String>(2)?))){
let lines:Vec<String>=rows.flatten().map(|(dec,ctx,ag)|{let c=ctx.map(|c|format!(" ({c})")).unwrap_or_default();format!(
"- [{ag}] {dec}{c}")}).collect();if!lines.is_empty(){parts.push(format!("New decisions:\n{}",lines.join("\n")));}}}if let Ok(mut
stmt)=conn.prepare(
"SELECT text, type FROM memories WHERE status = 'active' AND updated_at >= ?1 AND type != 'state' ORDER BY updated_at DESC LIMIT 3"
){if let Ok(rows)=stmt.query_map(params![lb],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?))){let lines:Vec<String>=rows.
flatten().map(|(text,mtype)|{let truncated:String=text.chars().take(100).collect();format!("- [{mtype}] {truncated}")}).collect();
if!lines.is_empty(){parts.push(format!("New knowledge:\n{}",lines.join("\n")));}}}if let Ok(mut stmt)=conn.prepare(
"SELECT type, COUNT(*) as cnt FROM events WHERE created_at > ?1 AND type NOT IN ('brain_init', 'index_all', 'agent_boot') GROUP BY type"
){if let Ok(rows)=stmt.query_map(params![lb],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?))){let entries:Vec<String>=rows.
flatten().map(|(etype,cnt)|format!("{cnt} {}",etype.replace('_'," "))).collect();if!entries.is_empty(){parts.push(format!(
"Activity since last boot: {}",entries.join(", ")));}}}}else{if let Ok(mut stmt)=conn.prepare(
"SELECT decision, context FROM decisions WHERE status = 'active' ORDER BY created_at DESC LIMIT 5"){if let Ok(rows)=stmt.query_map
([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?))){let lines:Vec<String>=rows.flatten().map(|(dec,ctx)|{let c=ctx.
map(|c|format!(" — {c}")).unwrap_or_default();format!("- {dec}{c}")}).collect();if!lines.is_empty(){parts.push(format!(
"Recent decisions:\n{}",lines.join("\n")));}}}}let text=parts.join("\n\n");let tokens=estimate_tokens(&text);let freshness=
last_boot.as_ref().map(|lb|{let prefix:String=lb.chars().take(16).collect();format!("since {prefix}")}).unwrap_or_else(||
"first boot".to_string());(text,tokens,freshness)}pub(crate)fn estimate_raw_baseline(conn:&Connection,_home:&Path)->usize{let mut
total_chars:usize=0;let mem_chars:i64=conn.query_row("SELECT COALESCE(SUM(LENGTH(text)), 0) FROM memories WHERE status = 'active'"
,[],|r|r.get(0)).unwrap_or(0);total_chars+=mem_chars as usize;let dec_chars:i64=conn.query_row(
"SELECT COALESCE(SUM(LENGTH(decision)), 0) FROM decisions WHERE status = 'active'",[],|r|r.get(0)).unwrap_or(0);total_chars+=
dec_chars as usize;estimate_tokens_from_chars(total_chars)}pub(crate)fn record_boot(conn:&Connection,agent:&str){let now=chrono::
Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis,true);let _=conn.execute(
"INSERT INTO events (type, data, source_agent) VALUES (?1, ?2, ?3)",params!["agent_boot",serde_json::to_string(&json!({"timestamp"
:&now,"agent":agent})).unwrap_or_default(),agent],);}
