use crate::mcp::{gating, tools};
use cdcx_core::api_client::ApiClient;
use cdcx_core::env::Environment;
use cdcx_core::schema::SchemaRegistry;
use std::sync::Arc;

/// MCP server for exposing Crypto.com Exchange API through Model Context Protocol
#[derive(Clone)]
pub struct CdcxMcpServer {
    api_client: Option<Arc<ApiClient>>,
    schema_registry: Arc<SchemaRegistry>,
    service_groups: Vec<String>,
    allow_dangerous: bool,
    environment: Environment,
}

impl CdcxMcpServer {
    pub fn new(
        api_client: Option<ApiClient>,
        service_groups: Vec<String>,
        allow_dangerous: bool,
        environment: Environment,
    ) -> Result<Self, String> {
        let schema_registry = SchemaRegistry::new().map_err(|e| {
            format!(
                "Cannot start MCP server: {}. Run 'cdcx schema update' first.",
                e
            )
        })?;
        Ok(Self {
            api_client: api_client.map(Arc::new),
            schema_registry: Arc::new(schema_registry),
            service_groups,
            allow_dangerous,
            environment,
        })
    }

    #[allow(dead_code)]
    pub fn schema_registry(&self) -> &SchemaRegistry {
        &self.schema_registry
    }

    #[allow(dead_code)]
    pub fn service_groups(&self) -> &[String] {
        &self.service_groups
    }

    #[allow(dead_code)]
    pub fn allow_dangerous(&self) -> bool {
        self.allow_dangerous
    }
}

// ServerHandler trait implementation
impl rmcp::handler::server::ServerHandler for CdcxMcpServer {
    fn get_info(&self) -> rmcp::model::ServerInfo {
        use rmcp::model::{Implementation, ServerCapabilities};

        rmcp::model::InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("cdcx", env!("CARGO_PKG_VERSION")))
    }

    fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<rmcp::model::ListToolsResult, rmcp::ErrorData>>
           + Send
           + '_ {
        let tools = tools::generate_tools(&self.schema_registry, &self.service_groups);
        std::future::ready(Ok(rmcp::model::ListToolsResult {
            meta: None,
            next_cursor: None,
            tools,
        }))
    }

    fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        _context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> impl std::future::Future<Output = Result<rmcp::model::CallToolResult, rmcp::ErrorData>>
           + Send
           + '_ {
        let server = self.clone();
        async move {
            // Parse tool name: cdcx_{group}_{command}
            let tool_name = &request.name;
            let parts: Vec<&str> = tool_name.splitn(3, '_').collect();

            if parts.len() != 3 || parts[0] != "cdcx" {
                return Err(rmcp::ErrorData::invalid_request(
                    "Invalid tool name format",
                    None,
                ));
            }

            let mcp_group = parts[1];
            let command = parts[2];

            // Map the MCP group back to schema groups to avoid command name collisions
            // (e.g., "withdraw" exists in both wallet and fiat schemas)
            let target_schema_groups = tools::mcp_to_schema_groups(mcp_group);

            let mut endpoint = None;
            for schema_group in target_schema_groups {
                let endpoints = server.schema_registry.get_by_group(schema_group);
                if let Some(found) = endpoints.iter().find(|ep| ep.command == command) {
                    endpoint = Some((*found).clone());
                    break;
                }
            }

            let endpoint = match endpoint {
                Some(ep) => ep,
                None => {
                    return Err(rmcp::ErrorData::new(
                        rmcp::model::ErrorCode::RESOURCE_NOT_FOUND,
                        format!("Tool not found: {}", tool_name),
                        None,
                    ))
                }
            };

            // Check safety gating
            gating::check_mcp_safety(&endpoint.method, &request.arguments, server.allow_dangerous)?;

            // Extract parameters (excluding "acknowledged")
            let mut params = serde_json::Map::new();
            if let Some(args) = &request.arguments {
                for (key, value) in args {
                    if key != "acknowledged" {
                        params.insert(key.clone(), value.clone());
                    }
                }
            }
            let mut params_value = serde_json::Value::Object(params);

            // Stamp cx2- MCP origin prefix on order-creating payloads so orders placed
            // by AI agents are distinguishable from CLI/TUI flow in exchange databases.
            // See cdcx-core::origin for the scheme.
            {
                use cdcx_core::origin::{tag_order_list_legs, tag_params_in_place, OriginChannel};
                let method = endpoint.method.as_str();
                if method == "private/create-order" || method == "private/advanced/create-order" {
                    let _ = tag_params_in_place(&mut params_value, OriginChannel::Mcp);
                } else if method == "private/create-order-list"
                    || method == "private/create-oco-order"
                    || method == "private/create-oto-order"
                    || method == "private/create-otoco-order"
                {
                    tag_order_list_legs(&mut params_value, OriginChannel::Mcp);
                }
            }

            // Validate params against adversarial input patterns
            cdcx_core::sanitize::validate_json_payload(&params_value).map_err(|e| {
                rmcp::ErrorData::new(rmcp::model::ErrorCode::INVALID_PARAMS, e.message, None)
            })?;

            // Dispatch to API client
            let result = if endpoint.auth_required {
                // Private request requires authentication
                match &server.api_client {
                    Some(client) => match client.request(&endpoint.method, params_value).await {
                        Ok(value) => value,
                        Err(e) => {
                            return Err(rmcp::ErrorData::new(
                                rmcp::model::ErrorCode::INTERNAL_ERROR,
                                e.to_string(),
                                None,
                            ))
                        }
                    },
                    None => {
                        return Err(rmcp::ErrorData::new(
                            rmcp::model::ErrorCode::INTERNAL_ERROR,
                            "API client not available for authenticated request",
                            None,
                        ))
                    }
                }
            } else {
                // Public request
                match &server.api_client {
                    Some(client) => {
                        match client.public_request(&endpoint.method, params_value).await {
                            Ok(value) => value,
                            Err(e) => {
                                return Err(rmcp::ErrorData::new(
                                    rmcp::model::ErrorCode::INTERNAL_ERROR,
                                    e.to_string(),
                                    None,
                                ))
                            }
                        }
                    }
                    None => {
                        // Create a temporary client for public requests using configured environment
                        let temp_client = ApiClient::new(None, server.environment);
                        match temp_client
                            .public_request(&endpoint.method, params_value)
                            .await
                        {
                            Ok(value) => value,
                            Err(e) => {
                                return Err(rmcp::ErrorData::new(
                                    rmcp::model::ErrorCode::INTERNAL_ERROR,
                                    e.to_string(),
                                    None,
                                ))
                            }
                        }
                    }
                }
            };

            Ok(rmcp::model::CallToolResult::success(vec![
                rmcp::model::Content::text(
                    serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string()),
                ),
            ]))
        }
    }
}
