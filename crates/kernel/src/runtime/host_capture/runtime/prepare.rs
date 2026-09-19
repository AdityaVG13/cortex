use super::super::*;
use crate::runtime::CortexRuntime;
use asupersync::Cx;
use serde_json::json;

impl CortexRuntime {
    /// Capture first, then mechanically maintain a scoped need from this supported event.
    /// Returned payload is for the adapter's supported context channel, never a store narration.
    pub async fn capture_and_prepare_host(
        &self,
        cx: &Cx,
        grant_key: &str,
        context: &HostCaptureContext,
        raw: &[u8],
        invocation_context: &str,
        present_delivery: Option<&str>,
    ) -> Result<
        (
            HostCaptureReceipt,
            Option<crate::runtime::cycle::PreparedView>,
        ),
        String,
    > {
        let grant: HostCaptureGrant = self
            .with_locked_db(cx, |conn, principal| {
                let spec: String =
                    grant_spec_json(conn, principal, grant_key).map_err(|e| e.to_string())?;
                serde_json::from_str(&spec).map_err(|e| e.to_string())
            })
            .await?;
        validate(&grant, context, HostRoute::Live)?;
        let hook = live_hook_name(raw)?;
        let value = parse_host_json(raw)?;
        let scope = observation_scope_for(&grant, &value);
        if hook.as_deref() == Some("PreToolUse") {
            if field(&value, "session_id")? != context.session_id {
                return Err("host_session_mismatch".into());
            }
            let input = value
                .get("tool_input")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let cues = situation_cues(&input.to_string());
            let receipt = HostCaptureReceipt {
                adapter_version: grant.adapter_version.clone(),
                accepted: vec![],
                excluded_deliveries: 0,
                ignored_metadata: 0,
                next_offset: None,
                uncommitted_tail_bytes: 0,
            };
            if cues.is_empty() {
                return Ok((receipt, None));
            }
            let view = self
                .prepare_host_need(
                    cx,
                    grant_key,
                    context,
                    &scope,
                    cues,
                    invocation_context,
                    present_delivery,
                )
                .await?;
            return Ok((receipt, Some(view)));
        }
        let receipt = self.capture_host_event(cx, grant_key, context, raw).await?;
        if receipt.accepted.is_empty() || hook.as_deref() == Some("PreCompact") {
            return Ok((receipt, None));
        }
        let event = normalize_host_event(&grant, context, HostRoute::Live, raw)?;
        if event.kind == HostRecordKind::Final {
            return Ok((receipt, None));
        }
        let cues = situation_cues(&event.event.text);
        if cues.is_empty() {
            return Ok((receipt, None));
        }
        let view = self
            .prepare_host_need(
                cx,
                grant_key,
                context,
                &scope,
                cues,
                invocation_context,
                present_delivery,
            )
            .await?;
        Ok((receipt, Some(view)))
    }

    async fn prepare_host_need(
        &self,
        cx: &Cx,
        grant_key: &str,
        context: &HostCaptureContext,
        scope: &str,
        cues: Vec<String>,
        invocation_context: &str,
        present_delivery: Option<&str>,
    ) -> Result<crate::runtime::cycle::PreparedView, String> {
        let id = format!(
            "host-need:{}",
            cortex_logic::traces::content_hash(
                &json!([grant_key, context.session_id, context.generation]).to_string()
            )
        );
        self.subscribe_observations(
            cx,
            crate::runtime::cycle::NeedSpec {
                id: id.clone(),
                scope: observation::normalize_scope(scope),
                cues: cues.clone(),
                exclude_cues: Vec::new(),
                max_results: 32,
                max_bytes: 32768,
                ttl_seconds: 3600,
                learned: false,
            },
        )
        .await?;
        let mut view = self
            .prepare_observations(cx, &id, invocation_context, present_delivery)
            .await?;
        if let Ok(compiled) = self
            .compile_assemblies(cx, scope, &cues, 4, None, None, None, "")
            .await
        {
            view.assembly_brief = compiled.brief;
        }
        Ok(view)
    }
}
