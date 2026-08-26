use crate::config::{get_home_dir, read_json_file, write_json_file};
use crate::error::AppError;
use crate::provider::Provider;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;

pub const MANAGED_MODEL_ID: &str = "cc-switch-workbuddy";
pub const PROXY_TOKEN_PLACEHOLDER: &str = "cc-switch-proxy";

pub fn get_workbuddy_dir() -> PathBuf {
    get_home_dir().join(".workbuddy")
}

pub fn get_workbuddy_models_path() -> PathBuf {
    get_workbuddy_dir().join("models.json")
}

pub fn read_models() -> Result<Value, AppError> {
    let path = get_workbuddy_models_path();
    if !path.exists() {
        return Ok(json!([]));
    }
    let value: Value = read_json_file(&path)?;
    if !value.is_array() {
        return Err(AppError::Config(format!(
            "WorkBuddy models.json must contain a JSON array: {}",
            path.display()
        )));
    }
    Ok(value)
}

pub fn write_models(value: &Value) -> Result<(), AppError> {
    if !value.is_array() {
        return Err(AppError::Config(
            "WorkBuddy models.json must contain a JSON array".to_string(),
        ));
    }
    let path = get_workbuddy_models_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| AppError::io(parent, error))?;
    }
    write_json_file(&path, value)
}

pub fn is_managed_model(model: &Value) -> bool {
    model.get("id").and_then(Value::as_str) == Some(MANAGED_MODEL_ID)
}

pub fn contains_managed_proxy(value: &Value) -> bool {
    value
        .as_array()
        .is_some_and(|models| models.iter().any(is_managed_model))
}

pub fn install_managed_model(
    current: &mut Value,
    provider: &Provider,
    proxy_url: &str,
) -> Result<(), AppError> {
    let models = current.as_array_mut().ok_or_else(|| {
        AppError::Config("WorkBuddy models.json must contain a JSON array".to_string())
    })?;
    models.retain(|model| !is_managed_model(model));

    let max_output_tokens = provider
        .meta
        .as_ref()
        .and_then(|meta| meta.max_output_tokens)
        .filter(|value| *value > 0)
        .unwrap_or(32_768);

    models.push(json!({
        "id": MANAGED_MODEL_ID,
        "name": format!("CC Switch · {}", provider.name),
        "vendor": "Custom",
        "url": proxy_url,
        "apiKey": PROXY_TOKEN_PLACEHOLDER,
        "supportsToolCall": true,
        "supportsImages": true,
        "supportsReasoning": true,
        "useCustomProtocol": true,
        "reasoning": {
            "defaultEffort": "high",
            "supportedEfforts": ["low", "medium", "high", "xhigh"]
        },
        "maxInputTokens": 1_000_000,
        "maxOutputTokens": max_output_tokens
    }));
    Ok(())
}

pub fn remove_managed_model(current: &mut Value) -> Result<bool, AppError> {
    let models = current.as_array_mut().ok_or_else(|| {
        AppError::Config("WorkBuddy models.json must contain a JSON array".to_string())
    })?;
    let before = models.len();
    models.retain(|model| !is_managed_model(model));
    Ok(models.len() != before)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_model_preserves_unmanaged_models() {
        let provider = Provider::with_id(
            "p1".to_string(),
            "Responses upstream".to_string(),
            json!({}),
            None,
        );
        let mut models =
            json!([{"id":"user-model","url":"https://example.com/v1/chat/completions"}]);
        install_managed_model(
            &mut models,
            &provider,
            "http://127.0.0.1:15721/workbuddy/v1/chat/completions",
        )
        .unwrap();
        assert_eq!(models.as_array().unwrap().len(), 2);
        assert!(contains_managed_proxy(&models));
        remove_managed_model(&mut models).unwrap();
        assert_eq!(
            models,
            json!([{"id":"user-model","url":"https://example.com/v1/chat/completions"}])
        );
    }
}
