use crate::openapi::types::ResponseSchema;
use crate::schema::{EndpointSchema, ParamSchema};
use serde_yaml::Value as YamlValue;
use std::collections::{HashMap, HashSet};

/// Separator used to join OpenAPI + schema YAML files for combined caching.
pub const SCHEMA_SEPARATOR: &str = "---SCHEMA_SEPARATOR---";

/// Result of parsing an OpenAPI spec.
pub struct ParsedSpec {
    pub endpoints: Vec<ParsedEndpoint>,
    pub response_schemas: HashMap<String, ResponseSchema>,
    pub unmapped_tags: Vec<String>,
}

pub struct ParsedEndpoint {
    pub group: String,
    pub endpoint: EndpointSchema,
}

#[derive(Debug)]
pub struct ParseError(pub String);

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OpenAPI parse error: {}", self.0)
    }
}

impl std::error::Error for ParseError {}

/// Maps an OpenAPI tag name to a CLI group name.
pub fn tag_to_group(tag: &str) -> Option<&'static str> {
    match tag {
        "Reference and Market Data" => Some("market"),
        "Account Balance and Positions" => Some("account"),
        "Trading" => Some("trade"),
        "Advanced Order Management" => Some("advanced"),
        "Crypto Wallet" => Some("wallet"),
        "Fiat Wallet" => Some("fiat"),
        "Staking" => Some("staking"),
        "Transaction History" => Some("history"),
        "OTC RFQ for Taker" => Some("otc"),
        _ => None,
    }
}

/// Group descriptions for CLI help text.
pub fn group_description_for(group: &str) -> &'static str {
    match group {
        "market" => "Public market data endpoints",
        "account" => "Account balance and position endpoints",
        "trade" => "Trading endpoints",
        "advanced" => "Advanced order management (OTO/OTOCO)",
        "wallet" => "Crypto wallet endpoints",
        "fiat" => "Fiat wallet endpoints",
        "staking" => "Staking endpoints",
        "history" => "Transaction history endpoints",
        "otc" => "OTC RFQ trading endpoints",
        _ => "API endpoints",
    }
}

/// Derives a CLI-friendly command name from an OpenAPI method path.
pub fn derive_command_name(method: &str, group: &str) -> String {
    // Hardcoded overrides for backward compat
    if method == "public/get-tickers" {
        return "ticker".to_string();
    }

    let mut name = method.to_string();

    // Strip path prefixes (order matters — most specific first)
    let group_prefix = format!("private/{}/", group);
    let prefixes: &[&str] = &[
        &group_prefix,
        "private/staking/",
        "private/otc/",
        "private/advanced/",
        "private/fiat/",
        "public/staking/",
        "public/",
        "private/",
    ];
    for prefix in prefixes {
        if let Some(rest) = name.strip_prefix(prefix) {
            name = rest.to_string();
            break;
        }
    }

    // Track if we stripped a mutation verb (change-, create-) for later group-specific logic
    let had_mutation_verb = name.starts_with("change-") || name.starts_with("create-");

    // Strip group-specific prefix FIRST for groups that have one (fiat-, staking-)
    // This must be done before stripping operation prefixes (get-, create-, etc.)
    let group_dash = format!("{}-", group);
    if matches!(group, "fiat" | "staking") {
        if let Some(rest) = name.strip_prefix(&group_dash) {
            name = rest.to_string();
        }
    }

    // Strip get- prefix for read operations
    if let Some(rest) = name.strip_prefix("get-") {
        name = rest.to_string();
    }

    // Strip create- prefix for mutation commands
    if let Some(rest) = name.strip_prefix("create-") {
        name = rest.to_string();
    }

    // Cancel shortening — exact matches to avoid substring bugs
    name = match name.as_str() {
        "cancel-order" => "cancel".to_string(),
        "cancel-order-list" => "cancel-list".to_string(),
        "cancel-all-orders" => "cancel-all".to_string(),
        _ => name,
    };

    // Strip change- prefix
    if let Some(rest) = name.strip_prefix("change-") {
        name = rest.to_string();
    }

    // Group-specific prefix stripping (applied after operation prefixes)
    match group {
        "account" => {
            // Strip account- prefix only if preceded by change- (change-account-settings -> settings)
            // Keep account- if it's standalone (get-account-settings -> account-settings)
            if had_mutation_verb {
                if let Some(rest) = name.strip_prefix("account-") {
                    name = rest.to_string();
                }
            }
            // Always strip user- prefix (user-balance -> balance)
            if let Some(rest) = name.strip_prefix("user-") {
                name = rest.to_string();
            }
        }
        "margin" => {
            // "isolated-margin-transfer" -> "transfer"
            if let Some(rest) = name.strip_prefix("isolated-margin-") {
                name = rest.to_string();
            }
        }
        "otc" => {
            // Strip otc- prefix if present
            if let Some(rest) = name.strip_prefix("otc-") {
                name = rest.to_string();
            }
        }
        _ => {}
    }

    name
}

/// Derives a safety tier from the method path.
pub fn derive_safety_tier(method: &str) -> &'static str {
    // Dangerous operations
    if method == "private/create-withdrawal" || method == "private/fiat/fiat-create-withdraw" {
        return "dangerous";
    }

    // Public endpoints are always read
    if method.starts_with("public/") {
        return "read";
    }

    // Private read operations
    if method.contains("/get-")
        || method.ends_with("/user-balance")
        || method.ends_with("/user-balance-history")
        || method.ends_with("/get-accounts")
        || method.ends_with("/get-account-settings")
    {
        return "read";
    }

    // Everything else is mutate (write operations)
    "mutate"
}

/// Override group assignment for specific methods that the OpenAPI spec
/// puts under a generic tag but we want in a dedicated CLI group.
fn method_group_override(method: &str) -> Option<&'static str> {
    match method {
        "private/get-order-history" | "private/get-trades" | "private/get-transactions" => {
            Some("history")
        }
        "private/create-isolated-margin-transfer" | "private/change-isolated-margin-leverage" => {
            Some("margin")
        }
        _ => None,
    }
}

/// Resolve a $ref pointer like '#/components/schemas/FooRequest' in a YAML document.
fn resolve_ref<'a>(doc: &'a YamlValue, ref_path: &str) -> Option<&'a YamlValue> {
    let path = ref_path.strip_prefix("#/")?;
    let mut current = doc;
    for segment in path.split('/') {
        current = current.get(segment)?;
    }
    Some(current)
}

/// Flatten a schema that may use `allOf` composition by merging all properties and
/// required lists into a single owned YamlValue with `properties` and `required` keys.
/// Recursively resolves `$ref` within allOf items.
fn flatten_allof(
    schema: &YamlValue,
    openapi_doc: &YamlValue,
    schema_doc: Option<&YamlValue>,
) -> YamlValue {
    // If there's no allOf, return as-is
    let all_of = match schema.get("allOf").and_then(|a| a.as_sequence()) {
        Some(seq) => seq,
        None => return schema.clone(),
    };

    let mut merged_props = serde_yaml::Mapping::new();
    let mut merged_required: Vec<YamlValue> = Vec::new();

    for item in all_of {
        // Resolve $ref if present
        let resolved = if let Some(ref_path) = item.get("$ref").and_then(|r| r.as_str()) {
            let internal_ref = ref_path
                .split('#')
                .next_back()
                .map(|p| format!("#{}", p))
                .unwrap_or_default();
            schema_doc
                .and_then(|sd| resolve_ref(sd, &internal_ref))
                .or_else(|| resolve_ref(openapi_doc, ref_path))
                .unwrap_or(item)
        } else {
            item
        };

        // Recursively flatten if this item also has allOf
        let flat = if resolved.get("allOf").is_some() {
            flatten_allof(resolved, openapi_doc, schema_doc)
        } else {
            resolved.clone()
        };

        // Merge properties
        if let Some(props) = flat.get("properties").and_then(|p| p.as_mapping()) {
            for (k, v) in props {
                merged_props.insert(k.clone(), v.clone());
            }
        }

        // Merge required lists
        if let Some(req) = flat.get("required").and_then(|r| r.as_sequence()) {
            for r in req {
                if !merged_required.contains(r) {
                    merged_required.push(r.clone());
                }
            }
        }
    }

    // Also include any properties/required defined at the same level as allOf
    if let Some(props) = schema.get("properties").and_then(|p| p.as_mapping()) {
        for (k, v) in props {
            merged_props.insert(k.clone(), v.clone());
        }
    }
    if let Some(req) = schema.get("required").and_then(|r| r.as_sequence()) {
        for r in req {
            if !merged_required.contains(r) {
                merged_required.push(r.clone());
            }
        }
    }

    let mut result = serde_yaml::Mapping::new();
    result.insert(
        YamlValue::String("type".into()),
        YamlValue::String("object".into()),
    );
    result.insert(
        YamlValue::String("properties".into()),
        YamlValue::Mapping(merged_props),
    );
    result.insert(
        YamlValue::String("required".into()),
        YamlValue::Sequence(merged_required),
    );
    YamlValue::Mapping(result)
}

/// Parses an OpenAPI spec and extracts endpoints and schemas.
pub fn parse_openapi_spec(yaml_content: &str) -> Result<ParsedSpec, ParseError> {
    // Split if concatenated (fetcher joins openapi + schema files)
    let (openapi_yaml, schema_yaml) = if let Some(idx) = yaml_content.find(SCHEMA_SEPARATOR) {
        let sep_end = idx + SCHEMA_SEPARATOR.len();
        let rest = &yaml_content[sep_end..];
        let rest = rest.strip_prefix('\n').unwrap_or(rest);
        (&yaml_content[..idx], Some(rest))
    } else {
        (yaml_content, None)
    };

    let doc: YamlValue = serde_yaml::from_str(openapi_yaml)
        .map_err(|e| ParseError(format!("Invalid YAML: {}", e)))?;

    // Parse schema file if present (for $ref resolution)
    let schema_doc: Option<YamlValue> = schema_yaml.and_then(|s| serde_yaml::from_str(s).ok());

    let paths = doc
        .get("paths")
        .and_then(|p| p.as_mapping())
        .ok_or_else(|| ParseError("Missing 'paths' in OpenAPI spec".into()))?;

    let mut endpoints = Vec::new();
    let mut unmapped_tags = Vec::new();
    let mut seen_unmapped: HashSet<String> = HashSet::new();
    let mut response_schemas = HashMap::new();

    for (path_val, methods) in paths {
        let path = path_val.as_str().unwrap_or_default();
        let methods_map = match methods.as_mapping() {
            Some(m) => m,
            None => continue,
        };

        for (http_method_val, operation) in methods_map {
            let http_method = http_method_val.as_str().unwrap_or_default().to_uppercase();

            let tag = operation
                .get("tags")
                .and_then(|t| t.as_sequence())
                .and_then(|seq| seq.first())
                .and_then(|t| t.as_str())
                .unwrap_or_default();

            let group = match tag_to_group(tag) {
                Some(g) => g,
                None => {
                    if !tag.is_empty() && seen_unmapped.insert(tag.to_string()) {
                        unmapped_tags.push(tag.to_string());
                    }
                    continue;
                }
            };

            let method = operation
                .get("summary")
                .and_then(|s| s.as_str())
                .unwrap_or(path.trim_start_matches('/'))
                .split('(')
                .next()
                .unwrap_or("")
                .trim();

            // Allow method-specific group overrides (e.g., history, margin)
            let group = method_group_override(method).unwrap_or(group);

            let description = operation
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or_default()
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();

            let command = derive_command_name(method, group);
            let safety_tier = derive_safety_tier(method).to_string();
            let auth_required =
                method.starts_with("private/") || method == "public/staking/get-conversion-rate"; // Actually requires auth despite public prefix
            let params = extract_parameters(operation, &doc, schema_doc.as_ref());

            // Extract response schema for table formatting
            if let Some(rs) = extract_response_schema(operation) {
                response_schemas.insert(method.to_string(), rs);
            }

            let endpoint = EndpointSchema {
                method: method.to_string(),
                command,
                description,
                safety_tier,
                auth_required,
                http_method: http_method.clone(),
                group: group.to_string(),
                params,
            };

            endpoints.push(ParsedEndpoint {
                group: group.to_string(),
                endpoint,
            });
        }
    }

    Ok(ParsedSpec {
        endpoints,
        response_schemas,
        unmapped_tags,
    })
}

/// Parameter overrides for cases where the OpenAPI spec is incorrect.
fn param_override(method: &str, param_name: &str) -> Option<(bool, Option<String>)> {
    // (required, default) — returns None if no override
    match (method, param_name) {
        // depth is required in spec but API defaults to 50
        ("public/get-book", "depth") => Some((false, Some("50".to_string()))),
        // account_id is required in spec but API auto-resolves from authenticated session
        ("private/change-account-leverage", "account_id") => Some((false, None)),
        // price is conditionally required (LIMIT types only, not MARKET)
        ("private/create-order", "price") => Some((false, None)),
        ("private/advanced/create-order", "price") => Some((false, None)),
        _ => None,
    }
}

/// Extracts enum values from an OpenAPI property schema.
/// Checks: inline `enum`, `$ref` to enum schema, `allOf` with enum.
fn extract_enum_values(
    property: &YamlValue,
    openapi_doc: &YamlValue,
    schema_doc: Option<&YamlValue>,
) -> Vec<String> {
    // 1. Inline enum
    if let Some(values) = property.get("enum").and_then(|e| e.as_sequence()) {
        return values
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
    }

    // 2. $ref to enum schema
    if let Some(ref_path) = property.get("$ref").and_then(|r| r.as_str()) {
        let internal_ref = ref_path
            .split('#')
            .next_back()
            .map(|p| format!("#{}", p))
            .unwrap_or_default();
        let resolved = schema_doc
            .and_then(|sd| resolve_ref(sd, &internal_ref))
            .or_else(|| resolve_ref(openapi_doc, ref_path));
        if let Some(target) = resolved {
            if let Some(values) = target.get("enum").and_then(|e| e.as_sequence()) {
                return values
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
            }
        }
    }

    // 3. allOf — find first item with enum
    if let Some(all_of) = property.get("allOf").and_then(|a| a.as_sequence()) {
        for item in all_of {
            if let Some(ref_path) = item.get("$ref").and_then(|r| r.as_str()) {
                let internal_ref = ref_path
                    .split('#')
                    .next_back()
                    .map(|p| format!("#{}", p))
                    .unwrap_or_default();
                let resolved = schema_doc
                    .and_then(|sd| resolve_ref(sd, &internal_ref))
                    .or_else(|| resolve_ref(openapi_doc, ref_path));
                if let Some(target) = resolved {
                    if let Some(values) = target.get("enum").and_then(|e| e.as_sequence()) {
                        return values
                            .iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect();
                    }
                }
            }
            if let Some(values) = item.get("enum").and_then(|e| e.as_sequence()) {
                return values
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
            }
        }
    }

    Vec::new()
}

/// Extracts parameter schemas from an OpenAPI operation.
/// Resolves $ref pointers in both the openapi doc and schema doc.
fn extract_parameters(
    operation: &YamlValue,
    openapi_doc: &YamlValue,
    schema_doc: Option<&YamlValue>,
) -> Vec<ParamSchema> {
    let mut params = Vec::new();

    // Detect method for overrides
    let method = operation
        .get("summary")
        .and_then(|s| s.as_str())
        .unwrap_or_default();

    // GET-style: parameters array
    if let Some(param_list) = operation.get("parameters").and_then(|p| p.as_sequence()) {
        for (i, param) in param_list.iter().enumerate() {
            let name = param
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or_default();
            let mut required = param
                .get("required")
                .and_then(|r| r.as_bool())
                .unwrap_or(false);
            let description = param
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or_default();
            let param_type = param
                .get("schema")
                .and_then(|s| s.get("type"))
                .and_then(|t| t.as_str())
                .unwrap_or("string");

            // Extract enum values — reuse the same helper as POST params
            // GET params have their schema nested under "schema" key
            let schema_node = param.get("schema").unwrap_or(param);
            let enum_values = extract_enum_values(schema_node, openapi_doc, schema_doc);

            let (mapped_type, values) = if !enum_values.is_empty() {
                ("enum".to_string(), enum_values)
            } else if param_type == "array" {
                let items_node = schema_node.get("items").unwrap_or(schema_node);
                let item_enums = extract_enum_values(items_node, openapi_doc, schema_doc);
                if !item_enums.is_empty() {
                    ("enum_array".to_string(), item_enums)
                } else {
                    ("json".to_string(), Vec::new())
                }
            } else {
                let t = match param_type {
                    "integer" | "number" => "number",
                    "boolean" => "boolean",
                    "object" => "json",
                    _ => "string",
                };
                (t.to_string(), Vec::new())
            };

            // Apply overrides
            let mut default = None;
            if let Some((req_override, def_override)) = param_override(method, name) {
                required = req_override;
                default = def_override;
            }

            params.push(ParamSchema {
                name: name.to_string(),
                param_type: mapped_type,
                required,
                description: description.to_string(),
                values,
                // First instrument_name param is always positional for ergonomic CLI
                // e.g. `cdcx market ticker BTC_USD` instead of `--instrument-name BTC_USD`
                position: if i == 0 && (required || name == "instrument_name") {
                    Some(0)
                } else {
                    None
                },
                default,
            });
        }
    }

    // POST-style: requestBody with $ref resolution
    if let Some(schema_node) = operation
        .get("requestBody")
        .and_then(|rb| rb.get("content"))
        .and_then(|c| c.get("application/json"))
        .and_then(|aj| aj.get("schema"))
    {
        // Resolve the request schema (may be $ref or inline)
        let request_schema =
            if let Some(ref_path) = schema_node.get("$ref").and_then(|r| r.as_str()) {
                // Try schema doc first (external $ref like exchange-schema.yaml#/...),
                // then openapi doc (internal $ref like #/components/schemas/...)
                let internal_ref = ref_path
                    .split('#')
                    .next_back()
                    .map(|p| format!("#{}", p))
                    .unwrap_or_default();
                schema_doc
                    .and_then(|sd| resolve_ref(sd, &internal_ref))
                    .or_else(|| resolve_ref(openapi_doc, ref_path))
            } else {
                Some(schema_node)
            };

        if let Some(req_schema) = request_schema {
            // The actual user params are in `properties.params`, not top-level
            let params_schema = req_schema.get("properties").and_then(|p| p.get("params"));

            if let Some(params_node) = params_schema {
                // Resolve params $ref if needed
                let resolved_ref =
                    if let Some(ref_path) = params_node.get("$ref").and_then(|r| r.as_str()) {
                        let internal_ref = ref_path
                            .split('#')
                            .next_back()
                            .map(|p| format!("#{}", p))
                            .unwrap_or_default();
                        schema_doc
                            .and_then(|sd| resolve_ref(sd, &internal_ref))
                            .or_else(|| resolve_ref(openapi_doc, ref_path))
                            .unwrap_or(params_node)
                    } else {
                        params_node
                    };

                // Flatten allOf composition (merges properties + required from sub-schemas)
                let flattened;
                let resolved_params = if resolved_ref.get("allOf").is_some() {
                    flattened = flatten_allof(resolved_ref, openapi_doc, schema_doc);
                    &flattened
                } else {
                    resolved_ref
                };

                let props = resolved_params
                    .get("properties")
                    .and_then(|p| p.as_mapping());
                let required_list: Vec<String> = resolved_params
                    .get("required")
                    .and_then(|r| r.as_sequence())
                    .map(|seq| {
                        seq.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                if let Some(props) = props {
                    for (i, (key, val)) in props.iter().enumerate() {
                        let name = key.as_str().unwrap_or_default();
                        let description = val
                            .get("description")
                            .and_then(|d| d.as_str())
                            .unwrap_or_default();
                        let raw_type = val.get("type").and_then(|t| t.as_str()).unwrap_or("string");
                        let mut is_required = required_list.contains(&name.to_string());

                        let enum_values = extract_enum_values(val, openapi_doc, schema_doc);

                        let (mapped_type, values) = if !enum_values.is_empty() {
                            ("enum".to_string(), enum_values)
                        } else if raw_type == "array" {
                            // Check if array items have enum values (e.g. exec_inst)
                            let items_node = val.get("items").unwrap_or(val);
                            let item_enums =
                                extract_enum_values(items_node, openapi_doc, schema_doc);
                            if !item_enums.is_empty() {
                                ("enum_array".to_string(), item_enums)
                            } else {
                                ("json".to_string(), Vec::new())
                            }
                        } else {
                            let t = match raw_type {
                                "integer" | "number" => "number",
                                "boolean" => "boolean",
                                "object" => "json",
                                _ => "string",
                            };
                            (t.to_string(), Vec::new())
                        };

                        // Apply overrides
                        let mut default = None;
                        if let Some((req_override, def_override)) = param_override(method, name) {
                            is_required = req_override;
                            default = def_override;
                        }

                        params.push(ParamSchema {
                            name: name.to_string(),
                            param_type: mapped_type,
                            required: is_required,
                            description: description.to_string(),
                            values,
                            position: if i == 0 && (is_required || name == "instrument_name") {
                                Some(0)
                            } else {
                                None
                            },
                            default,
                        });
                    }
                }
            }
        }
    }

    params
}

/// Extracts response schema from an OpenAPI operation.
fn extract_response_schema(operation: &YamlValue) -> Option<ResponseSchema> {
    use crate::openapi::types::ResponseField;

    let result_schema = operation
        .get("responses")?
        .get("200")?
        .get("content")?
        .get("application/json")?
        .get("schema")?
        .get("properties")?
        .get("result")?;

    let (data_path, item_props) =
        if let Some(data) = result_schema.get("properties").and_then(|p| p.get("data")) {
            if let Some(items) = data.get("items") {
                let props = items.get("properties").and_then(|p| p.as_mapping())?;
                ("data".to_string(), props.clone())
            } else {
                return None;
            }
        } else {
            let props = result_schema
                .get("properties")
                .and_then(|p| p.as_mapping())?;
            ("".to_string(), props.clone())
        };

    let fields: Vec<ResponseField> = item_props
        .iter()
        .filter_map(|(key, val)| {
            let name = key.as_str()?.to_string();
            let field_type = val
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("string")
                .to_string();
            let format = val.get("format").and_then(|f| f.as_str()).map(String::from);
            let description = val
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or_default();
            let format_hint =
                derive_format_hint(&name, &field_type, format.as_deref(), description);

            Some(ResponseField {
                display_name: name.replace('_', " ").to_uppercase(),
                name,
                field_type,
                format,
                format_hint,
            })
        })
        .collect();

    Some(ResponseSchema { data_path, fields })
}

/// Derives a format hint for response field display.
fn derive_format_hint(
    name: &str,
    field_type: &str,
    format: Option<&str>,
    description: &str,
) -> crate::openapi::types::FormatHint {
    use crate::openapi::types::FormatHint;

    if field_type == "boolean" {
        return FormatHint::Boolean;
    }
    if name.ends_with("_time")
        || name.ends_with("_time_ns")
        || name.ends_with("_timestamp_ms")
        || name == "t"
    {
        return FormatHint::TimestampMs;
    }
    if format == Some("int64") && description.to_lowercase().contains("timestamp") {
        return FormatHint::TimestampMs;
    }
    if format == Some("decimal") {
        return FormatHint::Decimal;
    }
    FormatHint::Scalar
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tag_to_group_known_tags() {
        assert_eq!(tag_to_group("Reference and Market Data"), Some("market"));
        assert_eq!(tag_to_group("Trading"), Some("trade"));
        assert_eq!(tag_to_group("OTC RFQ for Taker"), Some("otc"));
        assert_eq!(tag_to_group("Staking"), Some("staking"));
        assert_eq!(tag_to_group("Fiat Wallet"), Some("fiat"));
    }

    #[test]
    fn test_tag_to_group_unknown() {
        assert_eq!(tag_to_group("Unknown Tag"), None);
    }

    #[test]
    fn test_derive_command_name() {
        assert_eq!(
            derive_command_name("public/get-instruments", "market"),
            "instruments"
        );
        assert_eq!(
            derive_command_name("public/get-tickers", "market"),
            "ticker"
        );
        assert_eq!(derive_command_name("public/get-book", "market"), "book");
        assert_eq!(
            derive_command_name("private/create-order", "trade"),
            "order"
        );
        assert_eq!(
            derive_command_name("private/cancel-order", "trade"),
            "cancel"
        );
        assert_eq!(
            derive_command_name("private/get-open-orders", "trade"),
            "open-orders"
        );
        assert_eq!(
            derive_command_name("private/fiat/fiat-deposit-history", "fiat"),
            "deposit-history"
        );
        assert_eq!(
            derive_command_name("private/staking/stake", "staking"),
            "stake"
        );
        assert_eq!(
            derive_command_name("private/otc/get-open-deals", "otc"),
            "open-deals"
        );
    }

    #[test]
    fn test_derive_command_name_comprehensive() {
        // Market (public)
        assert_eq!(
            derive_command_name("public/get-tickers", "market"),
            "ticker"
        );
        assert_eq!(
            derive_command_name("public/get-instruments", "market"),
            "instruments"
        );
        assert_eq!(derive_command_name("public/get-book", "market"), "book");
        assert_eq!(derive_command_name("public/get-trades", "market"), "trades");
        assert_eq!(
            derive_command_name("public/get-candlestick", "market"),
            "candlestick"
        );
        assert_eq!(
            derive_command_name("public/get-valuations", "market"),
            "valuations"
        );
        assert_eq!(
            derive_command_name("public/get-announcements", "market"),
            "announcements"
        );
        assert_eq!(
            derive_command_name("public/get-risk-parameters", "market"),
            "risk-parameters"
        );
        assert_eq!(
            derive_command_name("public/get-expired-settlement-price", "market"),
            "expired-settlement-price"
        );
        assert_eq!(
            derive_command_name("public/get-insurance", "market"),
            "insurance"
        );

        // Trade
        assert_eq!(
            derive_command_name("private/create-order", "trade"),
            "order"
        );
        assert_eq!(
            derive_command_name("private/create-order-list", "trade"),
            "order-list"
        );
        assert_eq!(
            derive_command_name("private/amend-order", "trade"),
            "amend-order"
        );
        assert_eq!(
            derive_command_name("private/cancel-order", "trade"),
            "cancel"
        );
        assert_eq!(
            derive_command_name("private/cancel-order-list", "trade"),
            "cancel-list"
        );
        assert_eq!(
            derive_command_name("private/cancel-all-orders", "trade"),
            "cancel-all"
        );
        assert_eq!(
            derive_command_name("private/close-position", "trade"),
            "close-position"
        );
        assert_eq!(
            derive_command_name("private/get-open-orders", "trade"),
            "open-orders"
        );
        assert_eq!(
            derive_command_name("private/get-order-detail", "trade"),
            "order-detail"
        );

        // Account
        assert_eq!(
            derive_command_name("private/user-balance", "account"),
            "balance"
        );
        assert_eq!(
            derive_command_name("private/user-balance-history", "account"),
            "balance-history"
        );
        assert_eq!(
            derive_command_name("private/get-accounts", "account"),
            "accounts"
        );
        assert_eq!(
            derive_command_name("private/get-positions", "account"),
            "positions"
        );
        assert_eq!(
            derive_command_name("private/create-subaccount-transfer", "account"),
            "subaccount-transfer"
        );
        assert_eq!(
            derive_command_name("private/get-subaccount-balances", "account"),
            "subaccount-balances"
        );
        assert_eq!(
            derive_command_name("private/change-account-leverage", "account"),
            "leverage"
        );
        assert_eq!(
            derive_command_name("private/change-account-settings", "account"),
            "settings"
        );
        assert_eq!(
            derive_command_name("private/get-account-settings", "account"),
            "account-settings"
        );
        assert_eq!(
            derive_command_name("private/get-fee-rate", "account"),
            "fee-rate"
        );
        assert_eq!(
            derive_command_name("private/get-instrument-fee-rate", "account"),
            "instrument-fee-rate"
        );

        // Wallet
        assert_eq!(
            derive_command_name("private/create-withdrawal", "wallet"),
            "withdrawal"
        );
        assert_eq!(
            derive_command_name("private/get-currency-networks", "wallet"),
            "currency-networks"
        );
        assert_eq!(
            derive_command_name("private/get-deposit-address", "wallet"),
            "deposit-address"
        );
        assert_eq!(
            derive_command_name("private/get-deposit-history", "wallet"),
            "deposit-history"
        );
        assert_eq!(
            derive_command_name("private/get-withdrawal-history", "wallet"),
            "withdrawal-history"
        );

        // Fiat
        assert_eq!(
            derive_command_name("private/fiat/fiat-deposit-info", "fiat"),
            "deposit-info"
        );
        assert_eq!(
            derive_command_name("private/fiat/fiat-deposit-history", "fiat"),
            "deposit-history"
        );
        assert_eq!(
            derive_command_name("private/fiat/fiat-withdraw-history", "fiat"),
            "withdraw-history"
        );
        assert_eq!(
            derive_command_name("private/fiat/fiat-create-withdraw", "fiat"),
            "withdraw"
        );
        assert_eq!(
            derive_command_name("private/fiat/fiat-transaction-quota", "fiat"),
            "transaction-quota"
        );
        assert_eq!(
            derive_command_name("private/fiat/fiat-transaction-limit", "fiat"),
            "transaction-limit"
        );
        assert_eq!(
            derive_command_name("private/fiat/fiat-get-bank-accounts", "fiat"),
            "bank-accounts"
        );

        // Staking
        assert_eq!(
            derive_command_name("private/staking/stake", "staking"),
            "stake"
        );
        assert_eq!(
            derive_command_name("private/staking/unstake", "staking"),
            "unstake"
        );
        assert_eq!(
            derive_command_name("private/staking/get-staking-position", "staking"),
            "staking-position"
        );
        assert_eq!(
            derive_command_name("private/staking/get-staking-instruments", "staking"),
            "staking-instruments"
        );
        assert_eq!(
            derive_command_name("private/staking/get-open-stake", "staking"),
            "open-stake"
        );
        assert_eq!(
            derive_command_name("private/staking/get-stake-history", "staking"),
            "stake-history"
        );
        assert_eq!(
            derive_command_name("private/staking/get-reward-history", "staking"),
            "reward-history"
        );
        assert_eq!(
            derive_command_name("private/staking/convert", "staking"),
            "convert"
        );
        assert_eq!(
            derive_command_name("private/staking/get-open-convert", "staking"),
            "open-convert"
        );
        assert_eq!(
            derive_command_name("private/staking/get-convert-history", "staking"),
            "convert-history"
        );
        assert_eq!(
            derive_command_name("public/staking/get-conversion-rate", "staking"),
            "conversion-rate"
        );

        // Advanced
        assert_eq!(
            derive_command_name("private/advanced/create-order", "advanced"),
            "order"
        );
        assert_eq!(
            derive_command_name("private/advanced/create-oco", "advanced"),
            "oco"
        );
        assert_eq!(
            derive_command_name("private/advanced/cancel-oco", "advanced"),
            "cancel-oco"
        );
        assert_eq!(
            derive_command_name("private/advanced/create-oto", "advanced"),
            "oto"
        );
        assert_eq!(
            derive_command_name("private/advanced/cancel-oto", "advanced"),
            "cancel-oto"
        );
        assert_eq!(
            derive_command_name("private/advanced/create-otoco", "advanced"),
            "otoco"
        );
        assert_eq!(
            derive_command_name("private/advanced/cancel-otoco", "advanced"),
            "cancel-otoco"
        );
        assert_eq!(
            derive_command_name("private/advanced/cancel-order", "advanced"),
            "cancel"
        );
        assert_eq!(
            derive_command_name("private/advanced/cancel-all-orders", "advanced"),
            "cancel-all"
        );
        assert_eq!(
            derive_command_name("private/advanced/get-open-orders", "advanced"),
            "open-orders"
        );
        assert_eq!(
            derive_command_name("private/advanced/get-order-detail", "advanced"),
            "order-detail"
        );
        assert_eq!(
            derive_command_name("private/advanced/get-order-history", "advanced"),
            "order-history"
        );

        // Margin
        assert_eq!(
            derive_command_name("private/create-isolated-margin-transfer", "margin"),
            "transfer"
        );
        assert_eq!(
            derive_command_name("private/change-isolated-margin-leverage", "margin"),
            "leverage"
        );

        // History
        assert_eq!(
            derive_command_name("private/get-order-history", "history"),
            "order-history"
        );
        assert_eq!(
            derive_command_name("private/get-trades", "history"),
            "trades"
        );
        assert_eq!(
            derive_command_name("private/get-transactions", "history"),
            "transactions"
        );
    }

    #[test]
    fn test_derive_safety_tier() {
        assert_eq!(derive_safety_tier("public/get-instruments"), "read");
        assert_eq!(derive_safety_tier("private/get-open-orders"), "read");
        assert_eq!(derive_safety_tier("private/create-order"), "mutate");
        assert_eq!(derive_safety_tier("private/cancel-order"), "mutate");
        assert_eq!(derive_safety_tier("private/create-withdrawal"), "dangerous");
        assert_eq!(
            derive_safety_tier("private/fiat/fiat-create-withdraw"),
            "dangerous"
        );
        assert_eq!(
            derive_safety_tier("private/change-account-leverage"),
            "mutate"
        );
        assert_eq!(derive_safety_tier("private/user-balance"), "read");
    }

    #[test]
    fn test_group_description() {
        assert_eq!(
            group_description_for("market"),
            "Public market data endpoints"
        );
        assert_eq!(group_description_for("otc"), "OTC RFQ trading endpoints");
        assert_eq!(group_description_for("unknown"), "API endpoints");
    }

    #[test]
    fn test_parse_openapi_minimal() {
        let yaml = r#"
openapi: 3.0.3
info:
  title: Test API
  version: 1.0.0
paths:
  /public/get-instruments:
    get:
      tags:
        - Reference and Market Data
      summary: public/get-instruments
      description: Get all instruments.
      responses:
        '200':
          description: Success
          content:
            application/json:
              schema:
                type: object
                properties:
                  result:
                    type: object
                    properties:
                      data:
                        type: array
                        items:
                          type: object
                          properties:
                            symbol:
                              type: string
                              description: Instrument symbol
                            base_ccy:
                              type: string
                              description: Base currency
"#;
        let result = parse_openapi_spec(yaml);
        assert!(result.is_ok(), "Parse failed: {:?}", result.err());

        let parsed = result.unwrap();
        assert_eq!(parsed.endpoints.len(), 1);

        let ep = &parsed.endpoints[0];
        assert_eq!(ep.group, "market");
        assert_eq!(ep.endpoint.method, "public/get-instruments");
        assert_eq!(ep.endpoint.command, "instruments");
        assert_eq!(ep.endpoint.safety_tier, "read");
        assert!(!ep.endpoint.auth_required);
        assert_eq!(ep.endpoint.http_method, "GET");
    }

    #[test]
    fn test_parse_openapi_with_parameters() {
        let yaml = r#"
openapi: 3.0.3
info:
  title: Test
  version: 1.0.0
paths:
  /public/get-book:
    get:
      tags:
        - Reference and Market Data
      summary: public/get-book
      description: Get order book
      parameters:
        - name: instrument_name
          in: query
          required: true
          schema:
            type: string
          description: Instrument name
        - name: depth
          in: query
          required: false
          schema:
            type: integer
          description: Book depth
      responses:
        '200':
          description: Success
"#;
        let result = parse_openapi_spec(yaml).unwrap();
        let ep = &result.endpoints[0];
        assert_eq!(ep.endpoint.params.len(), 2);
        assert_eq!(ep.endpoint.params[0].name, "instrument_name");
        assert!(ep.endpoint.params[0].required);
        assert_eq!(ep.endpoint.params[0].param_type, "string");
        assert_eq!(ep.endpoint.params[1].name, "depth");
        assert!(!ep.endpoint.params[1].required);
        assert_eq!(ep.endpoint.params[1].param_type, "number");
    }

    #[test]
    fn test_unmapped_tags_are_skipped() {
        let yaml = r#"
openapi: 3.0.3
info:
  title: Test
  version: 1.0.0
paths:
  /some/endpoint:
    get:
      tags:
        - Unknown New Tag
      summary: some/endpoint
      description: Test
      responses:
        '200':
          description: Success
"#;
        let result = parse_openapi_spec(yaml).unwrap();
        assert_eq!(result.endpoints.len(), 0);
        assert_eq!(result.unmapped_tags.len(), 1);
        assert!(result
            .unmapped_tags
            .contains(&"Unknown New Tag".to_string()));
    }

    #[test]
    fn test_response_schema_extraction() {
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
      description: Get instruments
      responses:
        '200':
          description: Success
          content:
            application/json:
              schema:
                type: object
                properties:
                  result:
                    type: object
                    properties:
                      data:
                        type: array
                        items:
                          type: object
                          properties:
                            symbol:
                              type: string
                              description: e.g. BTC_USDT
                            tradable:
                              type: boolean
                              description: true or false
                            create_time:
                              type: string
                              format: int64
                              description: Creation timestamp ms
"#;
        let result = parse_openapi_spec(yaml).unwrap();
        let rs = result
            .response_schemas
            .get("public/get-instruments")
            .unwrap();
        assert_eq!(rs.data_path, "data");
        assert!(rs.fields.len() >= 3);

        // Check format hints
        use crate::openapi::types::FormatHint;
        let tradable = rs.fields.iter().find(|f| f.name == "tradable").unwrap();
        assert_eq!(tradable.format_hint, FormatHint::Boolean);

        let create_time = rs.fields.iter().find(|f| f.name == "create_time").unwrap();
        assert_eq!(create_time.format_hint, FormatHint::TimestampMs);
    }

    #[test]
    fn test_extract_enum_values_from_ref() {
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
                    side:
                      $ref: '#/components/schemas/OrderSide'
                    instrument_name:
                      type: string
                      description: Instrument name
                  required:
                    - side
                    - instrument_name
      responses:
        '200':
          description: Success
components:
  schemas:
    OrderSide:
      type: string
      enum:
        - BUY
        - SELL
"#;
        let result = parse_openapi_spec(yaml).unwrap();
        let ep = &result.endpoints[0];
        let side_param = ep
            .endpoint
            .params
            .iter()
            .find(|p| p.name == "side")
            .unwrap();
        assert_eq!(side_param.values, vec!["BUY", "SELL"]);
        assert_eq!(side_param.param_type, "enum");
    }

    #[test]
    fn test_extract_inline_enum_values() {
        let yaml = r#"
openapi: 3.0.3
info:
  title: Test
  version: 1.0.0
paths:
  /private/close-position:
    post:
      tags:
        - Trading
      summary: private/close-position
      description: Close a position
      requestBody:
        content:
          application/json:
            schema:
              type: object
              properties:
                params:
                  type: object
                  properties:
                    type:
                      type: string
                      enum:
                        - LIMIT
                        - MARKET
                      description: Close type
                  required:
                    - type
      responses:
        '200':
          description: Success
"#;
        let result = parse_openapi_spec(yaml).unwrap();
        let ep = &result.endpoints[0];
        let type_param = ep
            .endpoint
            .params
            .iter()
            .find(|p| p.name == "type")
            .unwrap();
        assert_eq!(type_param.values, vec!["LIMIT", "MARKET"]);
        assert_eq!(type_param.param_type, "enum");
    }
}
