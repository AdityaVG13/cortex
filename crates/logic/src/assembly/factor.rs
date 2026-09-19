use super::{FactorCodec, canonical_json};
use serde_json::{Map, Value};

pub fn factor_records(rows: &[Value]) -> Value {
    if rows.is_empty() {
        return serde_json::json!({"codec": FactorCodec::Raw.as_str(), "rows": []});
    }
    let first = match &rows[0] {
        Value::Object(map) => map,
        _ => return serde_json::json!({"codec": FactorCodec::Raw.as_str(), "rows": rows}),
    };
    let mut template = Map::new();
    for (key, value) in first {
        if rows.iter().all(|row| {
            row.as_object()
                .and_then(|map| map.get(key))
                .is_some_and(|other| canonical_json(other) == canonical_json(value))
        }) {
            template.insert(key.clone(), value.clone());
        }
    }
    let residuals: Vec<Value> = rows
        .iter()
        .map(|row| {
            let mut residual = Map::new();
            if let Value::Object(map) = row {
                for (key, value) in map {
                    if !template.contains_key(key) {
                        residual.insert(key.clone(), value.clone());
                    }
                }
            }
            Value::Object(residual)
        })
        .collect();
    let factored = serde_json::json!({"codec": FactorCodec::TemplateResidual.as_str(), "template": Value::Object(template), "residuals": residuals});
    let plain = serde_json::json!({"codec": FactorCodec::Raw.as_str(), "rows": rows});
    if canonical_json(&factored).len() < canonical_json(&plain).len() {
        factored
    } else {
        plain
    }
}

pub fn expand_records(pack: &Value) -> Result<Vec<Value>, String> {
    let codec = pack
        .get("codec")
        .and_then(Value::as_str)
        .ok_or("unknown_factor_codec")?;
    match FactorCodec::parse(codec)? {
        FactorCodec::Raw => pack
            .get("rows")
            .and_then(Value::as_array)
            .cloned()
            .ok_or_else(|| "unknown_factor_codec".into()),
        FactorCodec::TemplateResidual => {
            let template = pack
                .get("template")
                .cloned()
                .unwrap_or(Value::Object(Map::new()));
            let residuals = pack
                .get("residuals")
                .and_then(Value::as_array)
                .ok_or("unknown_factor_codec")?;
            Ok(residuals
                .iter()
                .map(|row| {
                    let mut out = match &template {
                        Value::Object(map) => map.clone(),
                        _ => Map::new(),
                    };
                    if let Value::Object(extra) = row {
                        for (key, value) in extra {
                            out.insert(key.clone(), value.clone());
                        }
                    }
                    Value::Object(out)
                })
                .collect())
        }
    }
}
