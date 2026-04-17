use cdcx_core::schema::{EndpointSchema, ParamSchema, SchemaRegistry};
use rmcp::model::Tool;
use serde_json::json;
use std::sync::Arc;

/// Maps MCP service group names to schema group names.
/// MCP groups can map to one or more schema groups.
pub fn mcp_to_schema_groups(mcp_group: &str) -> Vec<&'static str> {
    match mcp_group {
        "market" => vec!["market"],
        "account" => vec!["account", "history"],
        "trade" => vec!["trade"],
        "advanced" => vec!["advanced"],
        "margin" => vec!["margin"],
        "staking" => vec!["staking"],
        "funding" => vec!["wallet"],
        "fiat" => vec!["fiat"],
        "stream" => vec!["stream"], // stream tools aren't in schema yet
        "otc" => vec!["otc"],
        "all" => vec![
            "market", "account", "history", "trade", "advanced", "wallet", "fiat", "staking",
            "margin", "otc",
        ],
        _ => vec![],
    }
}

/// Maps schema group names back to MCP group names (for tool name generation).
/// For "wallet" schema group, use "funding" in tool names.
/// For "account" and "history" schemas, use "account" in tool names.
fn schema_group_to_mcp(schema_group: &str) -> String {
    match schema_group {
        "wallet" => "funding".to_string(),
        "history" => "account".to_string(),
        other => other.to_string(),
    }
}

/// Generates MCP Tool objects from the schema registry for enabled service groups.
pub fn generate_tools(registry: &SchemaRegistry, service_groups: &[String]) -> Vec<Tool> {
    let mut tools = Vec::new();
    let mut seen_methods = std::collections::HashSet::new();

    // Expand MCP groups to schema groups
    let mut schema_groups_set = std::collections::HashSet::new();
    for service_group in service_groups {
        for schema_group in mcp_to_schema_groups(service_group) {
            schema_groups_set.insert(schema_group);
        }
    }

    // For each schema group, get all endpoints
    for schema_group in schema_groups_set {
        let endpoints = registry.get_by_group(schema_group);
        for endpoint in endpoints {
            // Skip duplicates (shouldn't happen, but be safe)
            if seen_methods.contains(&endpoint.method) {
                continue;
            }
            seen_methods.insert(endpoint.method.clone());

            let mcp_group = schema_group_to_mcp(schema_group);
            let tool_name = format!("cdcx_{}_{}", mcp_group, endpoint.command);

            let input_schema = build_input_schema(endpoint);
            let input_schema_obj = input_schema.as_object().unwrap();

            tools.push(Tool::new(
                tool_name,
                endpoint.description.clone(),
                Arc::new(input_schema_obj.clone()),
            ));
        }
    }

    tools
}

/// Builds a JSON Schema for the input parameters of an endpoint.
/// Includes all parameters from the schema plus an "acknowledged" boolean for mutate/dangerous.
fn build_input_schema(endpoint: &EndpointSchema) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();

    // Add parameters from schema
    for param in &endpoint.params {
        let param_schema = param_to_json_schema(param);
        properties.insert(param.name.clone(), param_schema);
        if param.required {
            required.push(param.name.clone());
        }
    }

    // For mutate/dangerous operations, add "acknowledged" parameter
    let is_mutate_or_dangerous = matches!(endpoint.safety_tier.as_str(), "mutate" | "dangerous");
    if is_mutate_or_dangerous {
        properties.insert(
            "acknowledged".to_string(),
            json!({
                "type": "boolean",
                "description": "Set to true to confirm this mutating operation",
            }),
        );
        required.push("acknowledged".to_string());
    }

    json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

/// Converts a ParamSchema to a JSON Schema property value.
fn param_to_json_schema(param: &ParamSchema) -> serde_json::Value {
    let mut schema = serde_json::Map::new();

    match param.param_type.as_str() {
        "string" => {
            schema.insert("type".to_string(), json!("string"));
        }
        "number" => {
            schema.insert("type".to_string(), json!("number"));
        }
        "boolean" => {
            schema.insert("type".to_string(), json!("boolean"));
        }
        "enum" => {
            schema.insert("type".to_string(), json!("string"));
            schema.insert("enum".to_string(), json!(param.values.clone()));
        }
        "enum_array" | "enum[]" => {
            schema.insert("type".to_string(), json!("array"));
            schema.insert(
                "items".to_string(),
                json!({
                    "type": "string",
                    "enum": param.values.clone(),
                }),
            );
        }
        _ => {
            schema.insert("type".to_string(), json!("string"));
        }
    }

    schema.insert("description".to_string(), json!(&param.description));

    serde_json::Value::Object(schema)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_group_mapping() {
        assert_eq!(mcp_to_schema_groups("market"), vec!["market"]);
        assert_eq!(mcp_to_schema_groups("account"), vec!["account", "history"]);
        assert_eq!(mcp_to_schema_groups("funding"), vec!["wallet"]);
        assert_eq!(mcp_to_schema_groups("fiat"), vec!["fiat"]);
        assert!(mcp_to_schema_groups("unknown").is_empty());
    }

    const TEST_SPEC: &str = include_str!("../../../../tests/fixtures/test-spec.yaml");

    fn test_registry() -> SchemaRegistry {
        SchemaRegistry::from_openapi(TEST_SPEC).expect("Failed to parse test fixture")
    }

    #[test]
    fn test_schema_group_to_mcp() {
        assert_eq!(schema_group_to_mcp("market"), "market");
        assert_eq!(schema_group_to_mcp("wallet"), "funding");
        assert_eq!(schema_group_to_mcp("history"), "account");
        assert_eq!(schema_group_to_mcp("trade"), "trade");
    }

    #[test]
    fn test_tool_generation_market_only() {
        let registry = test_registry();
        let tools = generate_tools(&registry, &["market".to_string()]);

        // Should have market tools
        assert!(tools.iter().any(|t| t.name == "cdcx_market_ticker"));
        assert!(tools.iter().any(|t| t.name == "cdcx_market_book"));

        // Should not have trade tools
        assert!(!tools.iter().any(|t| t.name.starts_with("cdcx_trade")));

        // Market tools should not require acknowledged parameter (they're read-only)
        let ticker_tool = tools
            .iter()
            .find(|t| t.name == "cdcx_market_ticker")
            .unwrap();
        let schema = &ticker_tool.input_schema;
        if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
            assert!(!props.contains_key("acknowledged"));
        }
    }

    #[test]
    fn test_tool_generation_all_groups() {
        let registry = test_registry();
        let tools = generate_tools(&registry, &["all".to_string()]);

        // Should have tools from all groups
        assert!(tools.iter().any(|t| t.name.starts_with("cdcx_market_")));
        assert!(tools.iter().any(|t| t.name.starts_with("cdcx_trade_")));
        assert!(tools.iter().any(|t| t.name.starts_with("cdcx_account_")));
        assert!(tools.iter().any(|t| t.name.starts_with("cdcx_funding_")));
        assert!(tools.iter().any(|t| t.name.starts_with("cdcx_fiat_")));

        // Should have tools from all fixture groups
        assert!(
            tools.len() >= 10,
            "Should have at least 10 tools, got {}",
            tools.len()
        );
    }

    #[test]
    fn test_tool_generation_filters_groups() {
        let registry = test_registry();
        let tools = generate_tools(&registry, &["market".to_string(), "trade".to_string()]);

        // Check that we have both market and trade tools
        let has_market = tools.iter().any(|t| t.name.starts_with("cdcx_market_"));
        let has_trade = tools.iter().any(|t| t.name.starts_with("cdcx_trade_"));

        assert!(has_market, "Should have market tools");
        assert!(has_trade, "Should have trade tools");

        // Check that we don't have account tools
        let has_account = tools.iter().any(|t| t.name.starts_with("cdcx_account_"));
        assert!(!has_account, "Should not have account tools");
    }

    #[test]
    fn test_tool_generation_funding_mapping() {
        let registry = test_registry();
        let tools = generate_tools(&registry, &["funding".to_string()]);

        // Should have funding tools (not wallet)
        assert!(tools.iter().any(|t| t.name.starts_with("cdcx_funding_")));
        assert!(!tools.iter().any(|t| t.name.starts_with("cdcx_wallet_")));
    }

    #[test]
    fn test_tool_generation_account_includes_history() {
        let registry = test_registry();
        let tools = generate_tools(&registry, &["account".to_string()]);

        // Should have account tools from both account and history schemas
        let account_tools: Vec<_> = tools
            .iter()
            .filter(|t| t.name.starts_with("cdcx_account_"))
            .collect();
        assert!(!account_tools.is_empty(), "Should have account tools");
    }

    #[test]
    fn test_mutate_tool_has_acknowledged_param() {
        let registry = test_registry();
        let tools = generate_tools(&registry, &["trade".to_string()]);

        let order_tool = tools
            .iter()
            .find(|t| t.name == "cdcx_trade_order")
            .expect("TOML schema should have cdcx_trade_order");

        let schema_obj = order_tool.input_schema.as_ref();
        let properties = schema_obj
            .get("properties")
            .and_then(|p| p.as_object())
            .expect("Should have properties");

        assert!(
            properties.contains_key("acknowledged"),
            "Mutate tool should have acknowledged parameter"
        );

        let required = schema_obj
            .get("required")
            .and_then(|r| r.as_array())
            .expect("Should have required array");
        assert!(
            required.iter().any(|v| v.as_str() == Some("acknowledged")),
            "acknowledged should be in required list for mutate tools"
        );
    }

    #[test]
    fn test_json_schema_generation_enum() {
        let param = ParamSchema {
            name: "timeframe".to_string(),
            param_type: "enum".to_string(),
            required: false,
            description: "Candlestick interval".to_string(),
            values: vec!["1m".to_string(), "5m".to_string(), "1h".to_string()],
            position: None,
            default: None,
        };

        let schema = param_to_json_schema(&param);
        assert_eq!(schema["type"], "string");
        assert!(schema["enum"].is_array());
        let enum_vals = schema["enum"].as_array().unwrap();
        assert_eq!(enum_vals.len(), 3);
    }

    #[test]
    fn test_json_schema_generation_enum_array() {
        let param = ParamSchema {
            name: "statuses".to_string(),
            param_type: "enum[]".to_string(),
            required: false,
            description: "Order statuses to filter".to_string(),
            values: vec!["ACTIVE".to_string(), "FILLED".to_string()],
            position: None,
            default: None,
        };

        let schema = param_to_json_schema(&param);
        assert_eq!(schema["type"], "array");
        assert_eq!(schema["items"]["type"], "string");
        assert!(schema["items"]["enum"].is_array());
    }

    #[test]
    fn test_json_schema_generation_number() {
        let param = ParamSchema {
            name: "quantity".to_string(),
            param_type: "number".to_string(),
            required: true,
            description: "Order quantity".to_string(),
            values: vec![],
            position: None,
            default: None,
        };

        let schema = param_to_json_schema(&param);
        assert_eq!(schema["type"], "number");
        assert_eq!(schema["description"], "Order quantity");
    }
}
