use super::super::{Caller, arg_str, arg_usize, invalid_field, invalid_request, lens};
use super::*;
use crate::lens::{EvidenceDepth, LensProfile, NeedFrame};
use crate::protocol::ResponseStatus;
use crate::recipe::Limits;
use crate::state::RuntimeState;
use serde_json::{Value, json};

/// Validate an agent-proposed plan structurally by running it against an
/// empty snapshot with tight limits: unknown operators fail at parse time,
/// cycles/limits at evaluation. No natural-language interpretation.
pub fn validate_proposed(plan: &Value) -> Result<(Vec<Step>, Vec<String>), String> {
    let steps: Vec<Step> = serde_json::from_value(plan.get("steps").cloned().unwrap_or(json!([])))
        .map_err(|e| format!("invalid step: {e}"))?;
    let outputs: Vec<String> =
        serde_json::from_value(plan.get("outputs").cloned().unwrap_or(json!([])))
            .map_err(|e| format!("invalid outputs: {e}"))?;
    if steps.is_empty() || outputs.is_empty() {
        return Err("a recipe needs steps and outputs".into());
    }
    let empty = crate::recipe::Snapshot::default();
    crate::recipe::evaluate(&empty, "default", &steps, &outputs, Limits::default())
        .map_err(|e| format!("structural validation failed: {e}"))?;
    Ok((steps, outputs))
}

/// Recipe execution through compiled reads. Named templates only, or an
/// agent-proposed plan under `plan` (validated, replayed, its annotation
/// bytes counted). Unknown names fall back to ordinary recall.
pub(in crate::handlers::operations) async fn run_recipe(
    cx: &asupersync::Cx,
    state: &RuntimeState,
    caller: &Caller<'_>,
    recipe: &str,
    args: &Value,
) -> Result<Value, String> {
    let params = args.get("params").cloned().unwrap_or(json!({}));
    let environment = arg_str(args, &["environment", "artifact"])
        .unwrap_or("unspecified")
        .to_string();
    let (steps, outputs, source) = if recipe == "proposed" {
        let plan = args.get("plan").cloned().unwrap_or(Value::Null);
        let (s, o) = match validate_proposed(&plan) {
            Ok(parsed) => parsed,
            Err(err) => return Ok(invalid_field(err, "plan")),
        };
        (
            s,
            o,
            json!({"kind": "agent_proposed", "annotation_bytes": plan.to_string().len(), "attributed_to": caller.agent}),
        )
    } else {
        match template(recipe, &params) {
            Some((s, o)) => (s, o, json!({"kind": "template", "name": recipe})),
            None => {
                let frame = NeedFrame::build(
                    LensProfile::Answer,
                    arg_str(args, &["need", "task", "query"]).unwrap_or(recipe),
                    &[],
                    EvidenceDepth::Brief,
                );
                let view = lens::run_lens(
                    cx,
                    state,
                    caller,
                    &frame,
                    args,
                    arg_usize(args, &["budget"]).unwrap_or(2000),
                )
                .await?;
                let mut out = view.to_json();
                out["recipe"] =
                    json!({"requested": recipe, "status": "unknown_recipe_fell_back_to_recall"});
                return Ok(out);
            }
        }
    };
    let conn = state.db.lock(cx).await.map_err(|e| e.to_string())?;
    let limits = Limits::default();
    match crate::db::compiled::run_compiled(
        &conn,
        &caller.principal,
        recipe,
        &steps,
        &outputs,
        &params,
        &environment,
        limits,
    ) {
        Ok((value, cached, result)) => Ok(
            json!({"status":ResponseStatus::Ok.as_str(),"profile":"recipe","recipe":{"name":recipe,"source":source,"operator_versions":result.operator_versions,"cached":cached,"environment":environment,"params":params},"values":value["values"],"guards":{"negative":value["guards"],"positive":value["positive"],"brain_epoch":result.brain_epoch,"policy_epoch":result.policy_epoch},"work":result.work,"interpretation":"typed evidence only; unrecognised questions are not mapped onto a recipe"}),
        ),
        Err(err) => {
            let mut out = invalid_request(err);
            out["recipe"] = json!(recipe);
            Ok(out)
        }
    }
}
