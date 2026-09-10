use super::deposit::{DepositInput, DepositOutcome, deposit_decision};
use crate::auth::CortexPaths;
use crate::handlers::recall::{RecallContext, execute_unified_recall};
use crate::handlers::store::{DecisionProvenance, StoreError};
use crate::state::RuntimeState;
use asupersync::Cx;
use serde_json::Value;
use std::path::Path;

/// Library-boundary error. Adapters map it to their transport (HTTP status,
/// JSON-RPC code, exit code); a host matches on the variant.
#[derive(Debug, Clone, PartialEq)]
pub enum CortexError {
    /// The brain at that path could not be opened or initialised.
    Open(String),
    /// The request was rejected before any write (validation, bad input).
    Rejected(String),
    /// The write conflicted with existing state (idempotency, heads).
    Conflict(String),
    /// Retrieval failed (engine or storage error).
    Recall(String),
    /// Storage or internal failure.
    Internal(String),
    /// Lock acquisition failed, including cooperative cancellation.
    Lock(asupersync::sync::LockError),
}

impl std::fmt::Display for CortexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open(m) => write!(f, "open: {m}"),
            Self::Rejected(m) => write!(f, "rejected: {m}"),
            Self::Conflict(m) => write!(f, "conflict: {m}"),
            Self::Recall(m) => write!(f, "recall: {m}"),
            Self::Internal(m) => write!(f, "internal: {m}"),
            Self::Lock(e) => write!(f, "lock: {e}"),
        }
    }
}

impl std::error::Error for CortexError {}

impl From<asupersync::sync::LockError> for CortexError {
    fn from(error: asupersync::sync::LockError) -> Self {
        Self::Lock(error)
    }
}

impl From<StoreError> for CortexError {
    fn from(err: StoreError) -> Self {
        match err {
            StoreError::BadRequest(m) if m.starts_with("idempotency_conflict") => {
                Self::Conflict(m.to_string())
            }
            StoreError::BadRequest(m) => Self::Rejected(m.to_string()),
            StoreError::Validation { .. } => Self::Rejected(err.to_string()),
            StoreError::Internal(m) => Self::Internal(m),
        }
    }
}

/// Inputs of one boot compile over the library boundary.
#[derive(Debug, Clone, Default)]
pub struct BootInput {
    pub agent: String,
    pub max_tokens: usize,
    pub owner_id: Option<i64>,
}

/// Inputs of one Lens call over the library boundary.
#[derive(Debug, Clone, Default)]
pub struct LensInput {
    pub query: String,
    pub budget: usize,
    pub k: usize,
    pub agent: String,
    pub paths: Vec<String>,
    pub symbols: Vec<String>,
    pub as_of: Option<String>,
    pub owner_id: Option<i64>,
}

/// An open brain. Cloning shares the same connections; opening twice on the
/// same path yields independent handles over the same durable backend.
#[derive(Clone)]
pub struct CortexRuntime {
    state: RuntimeState,
}

impl CortexRuntime {
    pub fn open(paths: &CortexPaths) -> Result<Self, CortexError> {
        let (state, _shutdown) =
            crate::state::initialize(paths, false).map_err(CortexError::Open)?;
        state
            .readiness
            .store(true, std::sync::atomic::Ordering::Release);
        Ok(Self { state })
    }

    /// Open a brain from an explicit database path (home defaults to its parent).
    pub fn open_db(db_path: &Path) -> Result<Self, CortexError> {
        let home = db_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| Path::new(".").to_path_buf());
        let paths = CortexPaths::resolve_with_overrides(
            Some(&home.to_string_lossy()),
            Some(&db_path.to_string_lossy()),
            None,
            None,
        );
        Self::open(&paths)
    }

    /// Wrap an already-built runtime state (used by adapters that own state).
    pub fn from_state(state: RuntimeState) -> Self {
        Self { state }
    }

    pub fn state(&self) -> &RuntimeState {
        &self.state
    }

    /// Deposit one decision. Same semantics as HTTP `/store`.
    pub async fn deposit(
        &self,
        cx: &Cx,
        request_id: &str,
        text: &str,
        agent: &str,
        owner_id: Option<i64>,
    ) -> Result<DepositOutcome, CortexError> {
        self.deposit_with_key(cx, request_id, None, text, agent, owner_id)
            .await
    }

    /// Deposit with a principal-scoped idempotency key.
    pub async fn deposit_with_key(
        &self,
        cx: &Cx,
        request_id: &str,
        idempotency_key: Option<&str>,
        text: &str,
        agent: &str,
        owner_id: Option<i64>,
    ) -> Result<DepositOutcome, CortexError> {
        let mut conn = self.state.db.lock(cx).await?;
        Ok(deposit_decision(
            &mut conn,
            DepositInput {
                request_id,
                idempotency_key: idempotency_key.map(str::to_string),
                principal: owner_id
                    .map(|id| format!("user:{id}"))
                    .unwrap_or_else(|| "solo".into()),
                text,
                context: None,
                entry_type: Some("decision".into()),
                source_agent: agent.to_string(),
                provenance: DecisionProvenance::from_fields(agent, None, None),
                confidence: None,
                ttl_seconds: None,
                retention_class: None,
                anchors: Vec::new(),
                fields: None,
                owner_id,
                benchmark: false,
            },
        )?)
    }

    /// Compile the boot capsule (same compiler as HTTP `/boot`), in-process.
    pub async fn boot(
        &self,
        cx: &Cx,
        input: BootInput,
    ) -> Result<crate::compiler::BootResult, CortexError> {
        let conn = self.state.db_read.lock(cx).await?;
        let max_tokens = if input.max_tokens == 0 {
            600
        } else {
            input.max_tokens
        };
        let owner = if self.state.team_mode {
            input.owner_id
        } else {
            None
        };
        Ok(crate::compiler::compile_for_owner(
            &conn,
            &self.state.home,
            &input.agent,
            max_tokens,
            owner,
        ))
    }

    /// Run one Lens through the same engine `/recall` uses.
    pub async fn lens(&self, cx: &Cx, input: LensInput) -> Result<Value, CortexError> {
        let mut ctx = RecallContext::from_caller(input.owner_id, &self.state);
        ctx.paths = input.paths;
        ctx.symbols = input.symbols;
        ctx.as_of = input.as_of;
        let budget = if input.budget == 0 { 320 } else { input.budget };
        let k = if input.k == 0 { 8 } else { input.k };
        execute_unified_recall(
            cx,
            &self.state,
            input.query.trim(),
            budget,
            k,
            &input.agent,
            &ctx,
            None,
        )
        .await
        .map_err(CortexError::Recall)
    }
}
