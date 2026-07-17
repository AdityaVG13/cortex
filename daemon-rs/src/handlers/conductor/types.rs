use serde::Deserialize;
pub(crate) const SESSION_TTL_SECONDS: i64 = 7200;
pub(crate) const MAX_REQUEST_TTL_SECONDS: i64 = 100 * 365 * 24 * 60 * 60;
#[derive(Deserialize, Default)]
pub struct LockRequest {
    pub path: Option<String>,
    pub agent: Option<String>,
    pub ttl: Option<i64>,
}
#[derive(Deserialize, Default)]
pub struct ActivityRequest {
    pub agent: Option<String>,
    pub description: Option<String>,
}
#[derive(Deserialize, Default)]
pub struct SinceQuery {}
#[derive(Deserialize, Default)]
pub struct MessageRequest {
    pub from: Option<String>,
    pub to: Option<String>,
    pub message: Option<String>,
}
#[derive(Deserialize, Default)]
pub struct MessagesQuery {}
#[derive(Deserialize, Default)]
pub struct SessionStartRequest {
    pub agent: Option<String>,
}
#[derive(Deserialize, Default)]
pub struct SessionHeartbeatRequest {
    pub agent: Option<String>,
}
#[derive(Deserialize, Default)]
pub struct SessionEndRequest {
    pub agent: Option<String>,
}
#[derive(Deserialize, Default)]
pub struct TaskCreateRequest {
    pub title: Option<String>,
}
#[derive(Deserialize, Default)]
pub struct TaskQuery {}
#[derive(Deserialize, Default)]
pub struct TaskClaimRequest {
    #[serde(rename = "taskId")]
    pub task_id: Option<String>,
    pub agent: Option<String>,
}
#[derive(Deserialize, Default)]
pub struct TaskCompleteRequest {
    #[serde(rename = "taskId")]
    pub task_id: Option<String>,
    pub agent: Option<String>,
}
#[derive(Deserialize, Default)]
pub struct TaskAbandonRequest {
    #[serde(rename = "taskId")]
    pub task_id: Option<String>,
    pub agent: Option<String>,
}
#[derive(Deserialize, Default)]
pub struct TaskDeleteRequest {
    #[serde(rename = "taskId")]
    pub task_id: Option<String>,
}
#[derive(Deserialize, Default)]
pub struct NextTaskQuery {}
