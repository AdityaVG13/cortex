use super::super::CortexRuntime;
use super::*;
use asupersync::Cx;
use rusqlite::params;

mod batch;
mod prepare;
pub(super) use super::normalize::origin;

impl CortexRuntime {
    /// Explicit grants authorize only this finite adapter contract. There is no
    /// discovery, installation, host configuration change, or implicit grant.
    pub async fn register_host_capture(
        &self,
        cx: &Cx,
        grant: HostCaptureGrant,
    ) -> Result<(), String> {
        let probe = HostCaptureContext {
            host_version: grant.host_version.clone(),
            session_id: "validation".into(),
            generation: "validation".into(),
            original_event_key: None,
            origins: vec![],
        };
        validate(
            &grant,
            &probe,
            if grant.live {
                HostRoute::Live
            } else {
                HostRoute::History
            },
        )?;
        let sources = [
            (
                source_key(&grant.key, HostRecordKind::User),
                HostRecordKind::User.role(),
            ),
            (
                source_key(&grant.key, HostRecordKind::Tool),
                HostRecordKind::Tool.role(),
            ),
            (
                source_key(&grant.key, HostRecordKind::Final),
                HostRecordKind::Final.role(),
            ),
            (cursor_key(&grant.key), ObservationRole::DeliveryOnly),
        ];
        for (key, role) in sources {
            self.register_source(
                cx,
                SourceSpec {
                    key,
                    scope: grant.scope.clone(),
                    role,
                    max_bytes: grant.max_bytes,
                },
            )
            .await?;
        }
        self.with_locked_db(cx, |conn, principal| {
            conn.execute_batch(DDL).map_err(|e| e.to_string())?;
            let serialized = serde_json::to_string(&grant).map_err(|e| e.to_string())?;
            conn.execute(
                "INSERT OR IGNORE INTO host_capture_grants VALUES(?1,?2,?3)",
                params![principal, grant.key, serialized],
            )
            .map_err(|e| e.to_string())?;
            let old: String =
                grant_spec_json(conn, principal, &grant.key).map_err(|e| e.to_string())?;
            if old != serialized {
                return Err("host_registration_conflict".into());
            }
            Ok(())
        })
        .await
    }

    pub async fn host_capture_offset(
        &self,
        cx: &Cx,
        grant_key: &str,
        context: &HostCaptureContext,
    ) -> Result<u64, String> {
        self.source_offset(cx, &cursor_key(grant_key), &generation(context))
            .await
    }

    pub async fn capture_host_event(
        &self,
        cx: &Cx,
        grant_key: &str,
        context: &HostCaptureContext,
        raw: &[u8],
    ) -> Result<HostCaptureReceipt, String> {
        self.capture_host_batch(cx, grant_key, context, HostRoute::Live, None, raw)
            .await
    }

    /// `chunk` starts exactly at the durable raw JSONL byte cursor. Complete
    /// records, duplicates, exclusions, and the RAW (not normalized) byte cursor
    /// commit together. Malformed/unknown records roll the whole batch back.
    /// Caller must safely open an authorized file; this API accepts bytes only.
    pub async fn tail_host_transcript(
        &self,
        cx: &Cx,
        grant_key: &str,
        context: &HostCaptureContext,
        start: u64,
        chunk: &[u8],
    ) -> Result<HostCaptureReceipt, String> {
        self.capture_host_batch(
            cx,
            grant_key,
            context,
            HostRoute::History,
            Some(start),
            chunk,
        )
        .await
    }
}
