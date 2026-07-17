use serde::Deserialize;pub(crate)const SESSION_TTL_SECONDS:i64=7200;pub(crate)const ACTIVE_SESSION_WINDOW_SECONDS:i64=75;pub(crate
)const MAX_ACTIVITIES:i64=1000;pub(crate)const MAX_MESSAGES_PER_AGENT:i64=100;pub(crate)const MAX_TASKS:i64=500;pub(crate)const
DEFAULT_TASK_QUERY_LIMIT:usize=200;pub(crate)const MAX_TASK_QUERY_LIMIT:usize=500;pub(crate)const SESSION_FRESHNESS_IDLE_SECONDS:
i64=24*60*60;pub(crate)const MAX_REQUEST_TTL_SECONDS:i64=100*365*24*60*60;pub(crate)type SqlParam=Box<dyn rusqlite::types::ToSql>;
#[derive(Deserialize,Default)]pub struct LockRequest{pub path:Option<String>,pub agent:Option<String>,pub ttl:Option<i64>,}#[
derive(Deserialize,Default)]pub struct ActivityRequest{pub agent:Option<String>,pub description:Option<String>,pub files:Option<
Vec<String>>,}#[derive(Deserialize,Default)]pub struct SinceQuery{pub since:Option<String>,}#[derive(Deserialize,Default)]pub
struct MessageRequest{pub from:Option<String>,pub to:Option<String>,pub message:Option<String>,}#[derive(Deserialize,Default)]pub
struct MessagesQuery{pub agent:Option<String>,}#[derive(Deserialize,Default)]pub struct SessionStartRequest{pub agent:Option<
String>,pub project:Option<String>,pub files:Option<Vec<String>>,pub description:Option<String>,pub ttl:Option<i64>,}#[derive(
Deserialize,Default)]pub struct SessionHeartbeatRequest{pub agent:Option<String>,pub files:Option<Vec<String>>,pub description:
Option<String>,}#[derive(Deserialize,Default)]pub struct SessionEndRequest{pub agent:Option<String>,}#[derive(Deserialize,Default)
]pub struct TaskCreateRequest{pub title:Option<String>,pub description:Option<String>,pub project:Option<String>,pub files:Option<
Vec<String>>,pub priority:Option<String>,#[serde(rename="requiredCapability")]pub required_capability:Option<String>,}#[derive(
Deserialize,Default)]pub struct TaskQuery{pub status:Option<String>,pub project:Option<String>,pub limit:Option<usize>,pub offset:
Option<usize>,}#[derive(Deserialize,Default)]pub struct TaskClaimRequest{#[serde(rename="taskId")]pub task_id:Option<String>,pub
agent:Option<String>,}#[derive(Deserialize,Default)]pub struct TaskCompleteRequest{#[serde(rename="taskId")]pub task_id:Option<
String>,pub agent:Option<String>,pub summary:Option<String>,}#[derive(Deserialize,Default)]pub struct TaskAbandonRequest{#[serde(
rename="taskId")]pub task_id:Option<String>,pub agent:Option<String>,}#[derive(Deserialize,Default)]pub struct TaskDeleteRequest{#
[serde(rename="taskId")]pub task_id:Option<String>,}#[derive(Deserialize,Default)]pub struct NextTaskQuery{pub agent:Option<String
>,pub capability:Option<String>,}
