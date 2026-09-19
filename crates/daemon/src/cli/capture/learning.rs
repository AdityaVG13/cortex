use super::json_input;
use crate::runtime::{CortexRuntime, assembly::AssemblySpec, associations::AssociationAssessment};
use asupersync::Cx;
use serde_json::{Value, json};

pub(super) async fn associations(cx: &Cx, runtime: &CortexRuntime, command: &str, required: &dyn Fn(&str) -> Result<String, String>) -> Result<Value, String> {
    match command {
        "learn" => Ok(json!({"sources":runtime.rebuild_associations(cx, &required("--scope")?).await?})),
        "learn-reset" => {
            runtime.reset_associations(cx, &required("--scope")?).await?;
            Ok(json!({"status":"disabled"}))
        }
        "learn-explain" => serde_json::to_value(runtime.explain_associations(cx, &required("--scope")?, &[required("--query")?], 32).await?),
        "assess" => {
            let assessment = match required("--assessment")?.as_str() {
                "useful" => AssociationAssessment::Useful,
                "harmful" => AssociationAssessment::Harmful,
                "neutral" => AssociationAssessment::Neutral,
                _ => return Err("invalid_assessment".into()),
            };
            Ok(json!({"recorded":runtime.record_association_feedback(cx, &required("--scope")?, &required("--event-key")?, &required("--id")?, assessment).await?}))
        }
        "unassess" => {
            runtime.retract_association_feedback(cx, &required("--scope")?, &required("--event-key")?).await?;
            Ok(json!({"status":"retracted"}))
        }
        _ => unreachable!(),
    }.map_err(|e: serde_json::Error| e.to_string())
}

pub(super) async fn assemblies(cx: &Cx, runtime: &CortexRuntime, command: &str, required: &dyn Fn(&str) -> Result<String, String>) -> Result<Value, String> {
    match command {
        "assemble" => serde_json::to_value(runtime.put_assembly(cx, json_input::<AssemblySpec>()?).await?),
        "assembly-get" => serde_json::to_value(runtime.get_assembly(cx, &required("--id")?).await?),
        "assembly-expand" => serde_json::to_value(runtime.expand_assembly(cx, &required("--id")?).await?),
        "learn-event" => {
            let event = json_input::<cortex_kernel::assembly::LearningEvent>()?;
            Ok(json!({"recorded":runtime.record_learning_event(cx, event).await?}))
        }
        "learn-retract" => {
            runtime.retract_learning_event(cx, &required("--origin")?, &required("--event")?, &required("--reason")?).await?;
            Ok(json!({"status":"retracted"}))
        }
        "learn-erase" => Ok(json!({"retracted":runtime.erase_learning_source(cx, &required("--source")?).await?})),
        "routes-rebuild" => Ok(json!({"edges":runtime.rebuild_assembly_routes(cx, &required("--scope")?).await?})),
        "routes-reset" => {
            runtime.reset_assembly_routes(cx, &required("--scope")?).await?;
            Ok(json!({"status":"disabled"}))
        }
        "why" => serde_json::to_value(runtime.explain_assembly_routes(cx, &required("--scope")?, &[required("--query")?], 8).await?),
        _ => unreachable!(),
    }
    .map_err(|e: serde_json::Error| e.to_string())
}
