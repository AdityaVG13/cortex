use super::*;
use serde::Deserialize;
#[derive(Deserialize, Default)]
pub struct ForgetRequest {
    pub keyword: Option<String>,
    pub source: Option<String>,
}
#[derive(Deserialize, Default)]
pub struct ResolveRequest {
    #[serde(rename = "keepId", alias = "winnerId")]
    pub keep_id: Option<i64>,
    pub action: Option<String>,
    #[serde(rename = "supersededId", alias = "loserId")]
    pub superseded_id: Option<i64>,
    #[serde(rename = "conflictId", alias = "id")]
    pub conflict_id: Option<String>,
}
#[derive(Deserialize, Default)]
pub struct ArchiveRequest {
    pub table: Option<String>,
    pub ids: Option<Vec<i64>>,
}
#[derive(Deserialize, Default)]
pub struct ConflictListQuery {
    pub status: Option<String>,
    pub classification: Option<String>,
    #[serde(rename = "id")]
    pub conflict_id: Option<String>,
    pub limit: Option<usize>,
}
#[derive(Deserialize, Default)]
pub struct PermissionGrantRequest {
    pub client: Option<String>,
    pub permission: Option<String>,
    pub scope: Option<String>,
    #[serde(rename = "grantedBy", alias = "granted_by")]
    pub granted_by: Option<String>,
}
#[derive(Deserialize, Default)]
pub struct PermissionRevokeRequest {
    pub client: Option<String>,
    pub permission: Option<String>,
    pub scope: Option<String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictStatusFilter {
    Open,
    Resolved,
    All,
}
impl ConflictStatusFilter {
    pub fn parse(raw: Option<&str>) -> Result<Self, String> {
        match raw.map(str::trim).filter(|v| !v.is_empty()) {
            None => Ok(Self::Open),
            Some(value) => match value.to_ascii_lowercase().as_str() {
                "open" => Ok(Self::Open),
                "resolved" => Ok(Self::Resolved),
                "all" => Ok(Self::All),
                _ => Err("Invalid status filter. Expected open, resolved, or all.".to_string()),
            },
        }
    }
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Resolved => "resolved",
            Self::All => "all",
        }
    }
}
#[derive(Debug, Clone)]
pub struct ConflictListOptions {
    pub status: ConflictStatusFilter,
    pub classification: Option<String>,
    pub conflict_id: Option<String>,
}
impl Default for ConflictListOptions {
    fn default() -> Self {
        Self { status: ConflictStatusFilter::Open, classification: None, conflict_id: None }
    }
}
impl ConflictListOptions {
    pub(crate) fn from_query(query: ConflictListQuery) -> Result<Self, String> {
        let status = ConflictStatusFilter::parse(query.status.as_deref())?;
        let classification = match query.classification.as_deref().map(str::trim).filter(|value| !value.is_empty()) {
            Some(raw) => Some(
                normalize_conflict_classification(raw)
                    .ok_or_else(|| "Invalid classification filter. Expected AGREES, CONTRADICTS, REFINES, or UNRELATED.".to_string())?,
            ),
            None => None,
        };
        let conflict_id = query.conflict_id.as_deref().map(str::trim).filter(|value| !value.is_empty()).map(str::to_string);
        if let Some(id) = conflict_id.as_deref() {
            if parse_conflict_id(id).is_none() {
                return Err("Invalid conflict id. Expected decision:<id>:<id> or <id>:<id>.".into());
            }
        }
        let _ = query.limit;
        Ok(Self { status, classification, conflict_id })
    }
}
#[derive(Debug, Clone, Default)]
pub struct ResolutionMetadata;
