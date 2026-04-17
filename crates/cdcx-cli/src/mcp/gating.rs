use cdcx_core::safety::{check_acknowledged, SafetyTier};
use serde_json::Map;

/// Checks MCP safety gating for tool calls.
/// Returns an MCP ErrorData if the operation is gated and not acknowledged.
pub fn check_mcp_safety(
    method: &str,
    arguments: &Option<Map<String, serde_json::Value>>,
    allow_dangerous: bool,
) -> Result<(), rmcp::ErrorData> {
    let tier = SafetyTier::from_method(method);

    // Extract acknowledged parameter if present — must be a boolean if provided
    if let Some(args) = arguments.as_ref() {
        if let Some(ack_val) = args.get("acknowledged") {
            if !ack_val.is_boolean() {
                return Err(rmcp::ErrorData::new(
                    rmcp::model::ErrorCode::INVALID_PARAMS,
                    "Parameter 'acknowledged' must be a boolean".to_string(),
                    None,
                ));
            }
        }
    }

    let acknowledged = arguments
        .as_ref()
        .and_then(|args| args.get("acknowledged"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Use the core check_acknowledged function
    check_acknowledged(tier, acknowledged, allow_dangerous).map_err(|error_envelope| {
        // Convert ErrorEnvelope to rmcp::ErrorData
        rmcp::ErrorData::new(
            rmcp::model::ErrorCode::INVALID_PARAMS,
            error_envelope.message.clone(),
            None,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gating_read_always_passes() {
        // Read-only methods should always pass
        let result = check_mcp_safety("public/get-tickers", &None, false);
        assert!(result.is_ok());

        // With acknowledged=false
        let mut args = Map::new();
        args.insert("acknowledged".to_string(), serde_json::json!(false));
        let result = check_mcp_safety("public/get-tickers", &Some(args), false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_gating_sensitive_read_always_passes() {
        // Sensitive read should pass without acknowledged
        let result = check_mcp_safety("private/get-accounts", &None, false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_gating_mutate_needs_acknowledged() {
        // Mutate without acknowledged -> error
        let result = check_mcp_safety("private/create-order", &None, false);
        assert!(result.is_err());

        // Mutate with acknowledged=false -> error
        let mut args = Map::new();
        args.insert("acknowledged".to_string(), serde_json::json!(false));
        let result = check_mcp_safety("private/create-order", &Some(args), false);
        assert!(result.is_err());

        // Mutate with acknowledged=true -> ok
        let mut args = Map::new();
        args.insert("acknowledged".to_string(), serde_json::json!(true));
        let result = check_mcp_safety("private/create-order", &Some(args), false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_gating_dangerous_needs_both() {
        // Dangerous without allow_dangerous -> error even with acknowledged=true
        let mut args = Map::new();
        args.insert("acknowledged".to_string(), serde_json::json!(true));
        let result = check_mcp_safety("private/cancel-all-orders", &Some(args), false);
        assert!(result.is_err());

        // Dangerous with allow_dangerous but no acknowledged -> error
        let result = check_mcp_safety("private/cancel-all-orders", &None, true);
        assert!(result.is_err());

        // Dangerous with both allow_dangerous and acknowledged -> ok
        let mut args = Map::new();
        args.insert("acknowledged".to_string(), serde_json::json!(true));
        let result = check_mcp_safety("private/cancel-all-orders", &Some(args), true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_gating_dangerous_withdrawal() {
        // Create-withdrawal is dangerous
        let result = check_mcp_safety("private/create-withdrawal", &None, false);
        assert!(result.is_err());

        let mut args = Map::new();
        args.insert("acknowledged".to_string(), serde_json::json!(true));
        let result = check_mcp_safety("private/create-withdrawal", &Some(args.clone()), false);
        assert!(result.is_err()); // Still needs allow_dangerous

        let result = check_mcp_safety("private/create-withdrawal", &Some(args), true);
        assert!(result.is_ok()); // Both flags set -> ok
    }
}
