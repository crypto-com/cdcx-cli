use crate::error::ErrorEnvelope;
use crate::openapi::types::ResponseSchema;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputFormat {
    Json,
    Table,
    Ndjson,
}

impl OutputFormat {
    pub fn resolve(explicit: Option<&str>) -> Self {
        match explicit {
            Some("table") => Self::Table,
            Some("ndjson") => Self::Ndjson,
            _ => Self::Json, // JSON is always the default
        }
    }
}

pub fn format_success(
    data: &serde_json::Value,
    format: OutputFormat,
    method: Option<&str>,
    response_schema: Option<&ResponseSchema>,
) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(&serde_json::json!({
            "status": "ok",
            "data": data
        }))
        .unwrap(),
        OutputFormat::Table => {
            if let Some(m) = method {
                if let Some(table) = crate::tables::format_table(m, data, response_schema) {
                    return table;
                }
            }
            // Fall back to JSON if no table formatter
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "ok",
                "data": data
            }))
            .unwrap()
        }
        OutputFormat::Ndjson => serde_json::to_string(&data).unwrap(),
    }
}

pub fn format_error(envelope: &ErrorEnvelope, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json | OutputFormat::Table => {
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "error",
                "error": envelope
            }))
            .unwrap()
        }
        OutputFormat::Ndjson => {
            serde_json::to_string(&serde_json::json!({"status": "error", "error": envelope}))
                .unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_success_envelope() {
        let data = serde_json::json!({"price": "50000"});
        let output = format_success(&data, OutputFormat::Json, None, None);
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["data"]["price"], "50000");
    }

    #[test]
    fn test_error_envelope() {
        let err = ErrorEnvelope::api(10002, "UNAUTHORIZED");
        let output = format_error(&err, OutputFormat::Json);
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["status"], "error");
        assert_eq!(parsed["error"]["category"], "auth");
    }

    #[test]
    fn test_output_format_default_is_json() {
        // No explicit flag -> Json
        assert_eq!(OutputFormat::resolve(None), OutputFormat::Json);
        assert_eq!(OutputFormat::resolve(Some("table")), OutputFormat::Table);
        assert_eq!(OutputFormat::resolve(Some("ndjson")), OutputFormat::Ndjson);
        assert_eq!(OutputFormat::resolve(Some("json")), OutputFormat::Json);
    }

    #[test]
    fn test_ndjson_output() {
        let data = serde_json::json!({"price": "50000"});
        let output = format_success(&data, OutputFormat::Ndjson, None, None);
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["price"], "50000");
    }

    #[test]
    fn test_table_falls_back_to_json() {
        let data = serde_json::json!({"price": "50000"});
        let output = format_success(&data, OutputFormat::Table, None, None);
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["status"], "ok");
        assert_eq!(parsed["data"]["price"], "50000");
    }

    #[test]
    fn test_error_ndjson_output() {
        let err = ErrorEnvelope::api(10002, "UNAUTHORIZED");
        let output = format_error(&err, OutputFormat::Ndjson);
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["status"], "error");
        assert_eq!(parsed["error"]["category"], "auth");
    }
}
