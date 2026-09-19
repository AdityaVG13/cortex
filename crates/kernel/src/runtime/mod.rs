//! CortexRuntime: the library entry point every adapter (HTTP, MCP, CLI,
//! hooks, SDK subprocess) composes. It owns the memory semantics; adapters own
//! transport, authentication and presentation.
//!
//! Invariants: no HTTP, process supervision or provider clients are imported
//! here; a handle caches connections but the durable brain and request
//! identity live outside it; multiple handles may open the same backend.
//! Determinism class: deterministic given the database state and inputs.

pub mod assembly;
pub mod associations;
pub mod cycle;
mod deposit;
pub mod host_capture;
pub mod inventory;
pub mod observation;
mod open;

pub use deposit::{
    DepositInput, DepositOutcome, ack_profile_label as ack_profile_label_pub, deposit_decision,
};
pub use open::{BootInput, CortexError, CortexRuntime, LensInput};

use asupersync::Cx;
use rusqlite::{Connection, Transaction, TransactionBehavior, params};

impl CortexRuntime {
    pub(crate) async fn with_locked_db<R>(
        &self,
        cx: &Cx,
        f: impl FnOnce(&mut Connection, &str) -> Result<R, String>,
    ) -> Result<R, String> {
        let principal = self.observation_principal()?;
        let mut conn = self.state().db.lock(cx).await.map_err(|e| e.to_string())?;
        f(&mut conn, &principal)
    }

    pub(crate) async fn with_db_tx<R>(
        &self,
        cx: &Cx,
        behavior: TransactionBehavior,
        setup: impl FnOnce(&Connection) -> Result<(), String>,
        f: impl FnOnce(&Transaction<'_>, &str) -> Result<R, String>,
    ) -> Result<R, String> {
        self.with_locked_db(cx, |conn, principal| {
            setup(conn)?;
            let tx = conn
                .transaction_with_behavior(behavior)
                .map_err(|e| e.to_string())?;
            let result = f(&tx, principal)?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(result)
        })
        .await
    }
}

pub(crate) fn upsert_scope_enabled(
    conn: &Connection,
    table: &str,
    principal: &str,
    scope: &str,
    enabled: i64,
) -> Result<(), String> {
    conn.execute(&format!("INSERT INTO {table} VALUES(?1,?2,?3) ON CONFLICT(principal,scope_label) DO UPDATE SET enabled=excluded.enabled"), params![principal, scope, enabled]).map_err(|e| e.to_string())?;
    Ok(())
}
