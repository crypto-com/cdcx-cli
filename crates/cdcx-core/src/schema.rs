use crate::openapi::types::ResponseSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

#[derive(Debug)]
pub enum SchemaError {
    NoSpec(String),
    Parse(crate::openapi::parser::ParseError),
}

impl std::fmt::Display for SchemaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSpec(msg) => write!(f, "{}", msg),
            Self::Parse(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for SchemaError {}

impl From<crate::openapi::parser::ParseError> for SchemaError {
    fn from(e: crate::openapi::parser::ParseError) -> Self {
        Self::Parse(e)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EndpointSchema {
    pub method: String,
    pub command: String,
    pub description: String,
    pub safety_tier: String,
    pub auth_required: bool,
    pub http_method: String,
    #[serde(default)]
    pub group: String,
    #[serde(default)]
    pub params: Vec<ParamSchema>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ParamSchema {
    pub name: String,
    #[serde(rename = "type")]
    pub param_type: String,
    pub required: bool,
    pub description: String,
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub position: Option<u32>,
    #[serde(default)]
    pub default: Option<String>,
}

/// Overlay group — CLI presentation overrides only.
#[derive(Debug, Deserialize)]
pub struct OverlayGroup {
    pub group: OverlayGroupMeta,
    #[serde(default)]
    pub endpoints: Vec<OverlayEndpoint>,
}

#[derive(Debug, Deserialize)]
pub struct OverlayGroupMeta {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct OverlayEndpoint {
    pub method: String,
    pub command: Option<String>,
    #[serde(default)]
    pub params: Vec<OverlayParam>,
}

#[derive(Debug, Deserialize)]
pub struct OverlayParam {
    pub name: String,
    pub position: Option<u32>,
    pub default: Option<String>,
    pub required: Option<bool>,
}

pub struct SchemaRegistry {
    by_method: HashMap<String, EndpointSchema>,
    by_group: BTreeMap<String, Vec<EndpointSchema>>,
    group_descriptions: BTreeMap<String, String>,
    response_schemas: HashMap<String, ResponseSchema>,
}

impl SchemaRegistry {
    pub fn new() -> Result<Self, SchemaError> {
        let cached = crate::openapi::fetcher::SpecFetcher::default()
            .load_cache()
            .ok_or_else(|| {
                SchemaError::NoSpec(
                    "No API schema cached. Run 'cdcx setup' or 'cdcx schema update' first.".into(),
                )
            })?;

        let overlays = Self::load_overlays();
        Self::from_openapi_with_overlay(&cached, &overlays).map_err(SchemaError::from)
    }

    /// Create a registry from the bundled test fixture (test-spec.yaml).
    /// Uses the committed fixture instead of the user's local cache — no network needed.
    /// Intended for tests and CI.
    pub fn from_fixture() -> Result<Self, SchemaError> {
        let spec = include_str!("../../../tests/fixtures/test-spec.yaml");
        Self::from_openapi(spec).map_err(SchemaError::from)
    }

    fn load_overlays() -> Vec<OverlayGroup> {
        [
            include_str!("../../../schemas/market.toml"),
            include_str!("../../../schemas/account.toml"),
            include_str!("../../../schemas/trade.toml"),
            include_str!("../../../schemas/advanced.toml"),
            include_str!("../../../schemas/wallet.toml"),
            include_str!("../../../schemas/fiat.toml"),
            include_str!("../../../schemas/staking.toml"),
            include_str!("../../../schemas/margin.toml"),
            include_str!("../../../schemas/history.toml"),
        ]
        .iter()
        .map(|s| {
            toml::from_str::<OverlayGroup>(s)
                .expect("Failed to parse embedded overlay TOML — fix the schema file")
        })
        .collect()
    }

    pub fn from_openapi(yaml: &str) -> Result<Self, crate::openapi::parser::ParseError> {
        let parsed = crate::openapi::parser::parse_openapi_spec(yaml)?;

        let mut registry = Self {
            by_method: HashMap::new(),
            by_group: BTreeMap::new(),
            group_descriptions: BTreeMap::new(),
            response_schemas: parsed.response_schemas,
        };

        for tag in &parsed.unmapped_tags {
            eprintln!("Warning: skipping endpoints with unmapped tag '{}'", tag);
        }

        for parsed_ep in parsed.endpoints {
            let group = parsed_ep.group;
            let mut endpoint = parsed_ep.endpoint;
            endpoint.group = group.clone();

            registry
                .group_descriptions
                .entry(group.clone())
                .or_insert_with(|| {
                    crate::openapi::parser::group_description_for(&group).to_string()
                });

            registry
                .by_method
                .insert(endpoint.method.clone(), endpoint.clone());
            registry.by_group.entry(group).or_default().push(endpoint);
        }

        Ok(registry)
    }

    pub fn from_openapi_with_overlay(
        yaml: &str,
        overlays: &[OverlayGroup],
    ) -> Result<Self, crate::openapi::parser::ParseError> {
        let parsed = crate::openapi::parser::parse_openapi_spec(yaml)?;

        // Build overlay index
        let mut overlay_commands: HashMap<String, String> = HashMap::new();
        let mut overlay_params: HashMap<String, Vec<&OverlayParam>> = HashMap::new();
        for group in overlays {
            for ep in &group.endpoints {
                if let Some(ref cmd) = ep.command {
                    overlay_commands.insert(ep.method.clone(), cmd.clone());
                }
                if !ep.params.is_empty() {
                    overlay_params.insert(ep.method.clone(), ep.params.iter().collect());
                }
            }
        }

        let mut registry = Self {
            by_method: HashMap::new(),
            by_group: BTreeMap::new(),
            group_descriptions: BTreeMap::new(),
            response_schemas: parsed.response_schemas,
        };

        for tag in &parsed.unmapped_tags {
            eprintln!("Warning: skipping endpoints with unmapped tag '{}'", tag);
        }

        for parsed_ep in parsed.endpoints {
            let group = parsed_ep.group;
            let mut endpoint = parsed_ep.endpoint;
            endpoint.group = group.clone();

            // Apply command override from overlay
            if let Some(cmd) = overlay_commands.get(&endpoint.method) {
                endpoint.command = cmd.clone();
            }

            // Apply param overrides (position, default)
            if let Some(param_overlays) = overlay_params.get(&endpoint.method) {
                for po in param_overlays {
                    if let Some(param) = endpoint.params.iter_mut().find(|p| p.name == po.name) {
                        if let Some(pos) = po.position {
                            param.position = Some(pos);
                        }
                        if let Some(ref def) = po.default {
                            param.default = Some(def.clone());
                        }
                        if let Some(req) = po.required {
                            param.required = req;
                        }
                    }
                }
            }

            // Set safety_tier from the authoritative safety module
            endpoint.safety_tier = match crate::safety::SafetyTier::from_method(&endpoint.method) {
                crate::safety::SafetyTier::Read => "read".to_string(),
                crate::safety::SafetyTier::SensitiveRead => "sensitive_read".to_string(),
                crate::safety::SafetyTier::Mutate => "mutate".to_string(),
                crate::safety::SafetyTier::Dangerous => "dangerous".to_string(),
            };

            registry
                .group_descriptions
                .entry(group.clone())
                .or_insert_with(|| {
                    crate::openapi::parser::group_description_for(&group).to_string()
                });

            registry
                .by_method
                .insert(endpoint.method.clone(), endpoint.clone());
            registry.by_group.entry(group).or_default().push(endpoint);
        }

        Ok(registry)
    }

    pub fn get_by_method(&self, method: &str) -> Option<&EndpointSchema> {
        self.by_method.get(method)
    }

    pub fn get_by_group(&self, group: &str) -> Vec<&EndpointSchema> {
        self.by_group
            .get(group)
            .map(|v| v.iter().collect())
            .unwrap_or_default()
    }

    pub fn get_response_schema(&self, method: &str) -> Option<&ResponseSchema> {
        self.response_schemas.get(method)
    }

    pub fn list_all(&self) -> Vec<&EndpointSchema> {
        self.by_method.values().collect()
    }

    pub fn groups(&self) -> Vec<&str> {
        self.by_group.keys().map(|s| s.as_str()).collect()
    }

    pub fn group_description(&self, group: &str) -> Option<&str> {
        self.group_descriptions.get(group).map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SPEC: &str = include_str!("../../../tests/fixtures/test-spec.yaml");

    fn test_registry() -> SchemaRegistry {
        SchemaRegistry::from_openapi(TEST_SPEC).expect("Failed to parse test fixture")
    }

    #[test]
    fn test_registry_lookup() {
        let registry = test_registry();
        let ticker = registry.get_by_method("public/get-tickers").unwrap();
        assert_eq!(ticker.command, "ticker");
        assert_eq!(ticker.safety_tier, "read");
    }

    #[test]
    fn test_registry_list_by_group() {
        let registry = test_registry();
        let market = registry.get_by_group("market");
        assert!(
            market.len() >= 3,
            "Expected at least 3 market endpoints, got {}",
            market.len()
        );
    }

    #[test]
    fn test_schema_to_json() {
        let registry = test_registry();
        let ticker = registry.get_by_method("public/get-tickers").unwrap();
        let json = serde_json::to_value(ticker).unwrap();
        assert_eq!(json["method"], "public/get-tickers");
        assert!(json["params"].is_array());
    }

    #[test]
    fn test_registry_group_description() {
        let registry = test_registry();
        let desc = registry.group_description("market").unwrap();
        assert_eq!(desc, "Public market data endpoints");
    }

    #[test]
    fn test_registry_groups_sorted() {
        let registry = test_registry();
        let groups = registry.groups();
        let mut sorted = groups.clone();
        sorted.sort();
        assert_eq!(groups, sorted, "Groups should be returned in sorted order");
    }

    #[test]
    fn test_from_openapi_minimal() {
        let yaml = r#"
openapi: 3.0.3
info:
  title: Test
  version: 1.0.0
paths:
  /public/get-instruments:
    get:
      tags:
        - Reference and Market Data
      summary: public/get-instruments
      description: Get all instruments
      responses:
        '200':
          description: Success
  /private/create-order:
    post:
      tags:
        - Trading
      summary: private/create-order
      description: Create an order
      responses:
        '200':
          description: Success
"#;
        let registry = SchemaRegistry::from_openapi(yaml).unwrap();
        assert!(registry.get_by_method("public/get-instruments").is_some());
        assert!(registry.get_by_method("private/create-order").is_some());
        let market = registry.get_by_group("market");
        assert_eq!(market.len(), 1);
        let trade = registry.get_by_group("trade");
        assert_eq!(trade.len(), 1);
    }

    #[test]
    fn test_endpoint_group_field_populated() {
        let yaml = r#"
openapi: 3.0.3
info:
  title: Test
  version: 1.0.0
paths:
  /public/get-instruments:
    get:
      tags:
        - Reference and Market Data
      summary: public/get-instruments
      description: Get all instruments
      responses:
        '200':
          description: Success
  /private/create-order:
    post:
      tags:
        - Trading
      summary: private/create-order
      description: Create an order
      responses:
        '200':
          description: Success
"#;
        let registry = SchemaRegistry::from_openapi(yaml).unwrap();

        // Test that endpoints loaded from OpenAPI have correct group field
        let market_ep = registry.get_by_method("public/get-instruments").unwrap();
        assert_eq!(
            market_ep.group, "market",
            "Market endpoint should have group='market'"
        );

        let trade_ep = registry.get_by_method("private/create-order").unwrap();
        assert_eq!(
            trade_ep.group, "trade",
            "Trade endpoint should have group='trade'"
        );
    }

    #[test]
    fn test_list_all_preserves_group_field() {
        let registry = SchemaRegistry::from_fixture().expect("fixture spec should parse");
        let all_endpoints = registry.list_all();

        // Verify that all endpoints have their group field populated
        for ep in all_endpoints {
            assert!(
                !ep.group.is_empty(),
                "Endpoint {} should have a non-empty group field",
                ep.method
            );

            // Verify the group field corresponds to a known group in the registry
            assert!(
                registry.groups().contains(&ep.group.as_str()),
                "Endpoint {} has unexpected group: {}",
                ep.method,
                ep.group
            );
        }
    }

    #[test]
    fn test_endpoint_json_serialization_includes_group() {
        let registry = SchemaRegistry::from_fixture().expect("fixture spec should parse");

        // Get an endpoint from each group and verify serialization includes group
        for group_name in registry.groups() {
            let endpoints = registry.get_by_group(group_name);
            if let Some(ep) = endpoints.first() {
                let json = serde_json::to_value(ep).unwrap();

                // Verify that the serialized JSON includes the group field
                assert!(
                    json.get("group").is_some(),
                    "Serialized endpoint for group '{}' should include 'group' field",
                    group_name
                );

                // Verify that the group value matches the expected group
                assert_eq!(
                    json["group"].as_str(),
                    Some(group_name),
                    "Endpoint in group '{}' should serialize with group='{}'",
                    group_name,
                    group_name
                );
            }
        }
    }

    #[test]
    fn test_overlay_parsing() {
        let toml_str = r#"
            [group]
            name = "trade"

            [[endpoints]]
            method = "private/create-order"
            command = "order"

            [[endpoints.params]]
            name = "side"
            position = 0

            [[endpoints.params]]
            name = "type"
            default = "MARKET"
        "#;
        let overlay: OverlayGroup = toml::from_str(toml_str).unwrap();
        assert_eq!(overlay.group.name, "trade");
        assert_eq!(overlay.endpoints.len(), 1);
        assert_eq!(overlay.endpoints[0].command, Some("order".to_string()));
        assert_eq!(overlay.endpoints[0].params.len(), 2);
        assert_eq!(overlay.endpoints[0].params[0].position, Some(0));
        assert_eq!(
            overlay.endpoints[0].params[1].default,
            Some("MARKET".to_string())
        );
    }

    #[test]
    fn test_overlay_merge() {
        let yaml = r#"
openapi: 3.0.3
info:
  title: Test
  version: 1.0.0
paths:
  /private/create-order:
    post:
      tags:
        - Trading
      summary: private/create-order
      description: Create an order
      requestBody:
        content:
          application/json:
            schema:
              type: object
              properties:
                params:
                  type: object
                  properties:
                    instrument_name:
                      type: string
                      description: Instrument name
                    quantity:
                      type: string
                      description: Quantity
                  required:
                    - instrument_name
                    - quantity
      responses:
        '200':
          description: Success
"#;
        let overlay: OverlayGroup = toml::from_str(
            r#"
            [group]
            name = "trade"
            [[endpoints]]
            method = "private/create-order"
            command = "my-order"
            [[endpoints.params]]
            name = "instrument_name"
            position = 0
            [[endpoints.params]]
            name = "quantity"
            position = 1
        "#,
        )
        .unwrap();

        let registry = SchemaRegistry::from_openapi_with_overlay(yaml, &[overlay]).unwrap();
        let ep = registry.get_by_method("private/create-order").unwrap();

        assert_eq!(ep.command, "my-order", "Overlay overrides command");
        assert_eq!(ep.safety_tier, "mutate", "Safety tier from safety.rs");

        let inst = ep
            .params
            .iter()
            .find(|p| p.name == "instrument_name")
            .unwrap();
        assert_eq!(inst.position, Some(0), "Overlay sets position");
        assert!(inst.required, "Required preserved from OpenAPI");

        let qty = ep.params.iter().find(|p| p.name == "quantity").unwrap();
        assert_eq!(qty.position, Some(1), "Overlay sets multi-position");
    }
}
