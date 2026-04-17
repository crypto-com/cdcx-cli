use crate::auth::Credentials;
use crate::env::Environment;
use crate::error::{CdcxError, ErrorEnvelope};
use crate::sanitize::sanitize_response;
use crate::signing::{generate_nonce, sign_request};
use reqwest::Client;
use serde_json::json;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// REST API client that signs and sends requests to the Crypto.com Exchange API.
/// Returns `serde_json::Value` for responses.
pub struct ApiClient {
    credentials: Option<Credentials>,
    environment: Environment,
    http: Client,
    rest_url_override: Option<String>,
    request_counter: Arc<AtomicU64>,
}

impl ApiClient {
    /// Creates a new API client.
    pub fn new(credentials: Option<Credentials>, environment: Environment) -> Self {
        Self {
            credentials,
            environment,
            http: Client::new(),
            rest_url_override: None,
            request_counter: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Sets an override URL for testing (e.g., wiremock server).
    pub fn with_rest_url_override(mut self, url: String) -> Self {
        self.rest_url_override = Some(url);
        self
    }

    /// Sends a signed private request.
    /// POST request with signed JSON body.
    pub async fn request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, CdcxError> {
        let creds = self
            .credentials
            .as_ref()
            .ok_or_else(|| CdcxError::Config("Credentials required for private request".into()))?;

        let id = self.request_counter.fetch_add(1, Ordering::SeqCst);
        let nonce = generate_nonce();
        let sig = sign_request(
            method,
            id,
            &creds.api_key,
            &creds.api_secret,
            &params,
            nonce,
        )?;

        let body = json!({
            "id": id,
            "method": method,
            "api_key": &creds.api_key,
            "params": params,
            "nonce": nonce,
            "sig": sig
        });

        let base_url = self
            .rest_url_override
            .clone()
            .unwrap_or_else(|| self.environment.rest_url());

        let url = format!("{}/{}", base_url, method);

        let response = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(CdcxError::from)?;

        let status = response.status();
        let body_text = response.text().await.map_err(CdcxError::from)?;

        // Try to parse as JSON — the CDC API returns structured errors
        // (code + message) even on 4xx status codes. Read the body first
        // so we never lose the API's error message.
        if let Ok(response_body) = serde_json::from_str::<serde_json::Value>(&body_text) {
            if response_body.get("code").is_some() {
                return self.parse_response(response_body);
            }
        }

        // Non-JSON or no API error code — fall back to HTTP status
        if status.is_client_error() || status.is_server_error() {
            return Err(CdcxError::Api(ErrorEnvelope::network(&format!(
                "HTTP {} for {}",
                status, url
            ))));
        }

        // 2xx with valid JSON
        serde_json::from_str::<serde_json::Value>(&body_text)
            .map_err(CdcxError::from)
            .and_then(|body| self.parse_response(body))
    }

    /// Sends a public request (no authentication).
    /// GET request with query parameters.
    pub async fn public_request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, CdcxError> {
        let base_url = self
            .rest_url_override
            .clone()
            .unwrap_or_else(|| self.environment.rest_url());

        let url = format!("{}/{}", base_url, method);

        // Build query parameters with proper URL encoding via reqwest
        let mut query_pairs: Vec<(String, String)> = Vec::new();
        if let Some(obj) = params.as_object() {
            for (key, value) in obj {
                let value_str = match value {
                    serde_json::Value::String(s) => s.clone(),
                    serde_json::Value::Number(n) => n.to_string(),
                    serde_json::Value::Bool(b) => b.to_string(),
                    _ => value.to_string(),
                };
                query_pairs.push((key.clone(), value_str));
            }
        }

        let response = self
            .http
            .get(&url)
            .query(&query_pairs)
            .send()
            .await
            .map_err(CdcxError::from)?;

        let status = response.status();
        let body_text = response.text().await.map_err(CdcxError::from)?;

        if let Ok(response_body) = serde_json::from_str::<serde_json::Value>(&body_text) {
            if response_body.get("code").is_some() {
                return self.parse_response(response_body);
            }
        }

        if status.is_client_error() || status.is_server_error() {
            return Err(CdcxError::Api(ErrorEnvelope::network(&format!(
                "HTTP {} for {}",
                status, url
            ))));
        }

        serde_json::from_str::<serde_json::Value>(&body_text)
            .map_err(CdcxError::from)
            .and_then(|body| self.parse_response(body))
    }

    /// Parses API response and returns the result field or an error.
    /// When the API returns a non-zero code WITH a populated result, the operation
    /// succeeded (e.g. FAR_AWAY_LIMIT_PRICE, IMMEDIATE_OR_CANCEL). Pass through
    /// the API's code and message alongside the result data.
    fn parse_response(&self, response: serde_json::Value) -> Result<serde_json::Value, CdcxError> {
        let code = response["code"]
            .as_i64()
            .ok_or_else(|| CdcxError::Config("Response missing 'code' field".into()))?;

        let result = response["result"].clone();
        let has_result = result.as_object().is_some_and(|o| !o.is_empty());

        if code == 0 {
            Ok(sanitize_response(result, 10240))
        } else if has_result {
            let message = response["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            let mut enriched = result;
            if let Some(obj) = enriched.as_object_mut() {
                obj.insert("code".into(), json!(code));
                obj.insert("message".into(), json!(message));
            }
            Ok(sanitize_response(enriched, 10240))
        } else {
            let message = response["message"]
                .as_str()
                .unwrap_or("Unknown error")
                .to_string();
            Err(CdcxError::Api(ErrorEnvelope::api(code, &message)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Credentials;
    use crate::env::Environment;
    use crate::error::ErrorCategory;

    #[tokio::test]
    async fn test_signed_request() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"id": 1, "code": 0, "result": {"data": [{"a": "BTC_USDT"}]}}),
            ))
            .mount(&mock_server)
            .await;

        let client = ApiClient::new(
            Some(Credentials {
                api_key: "key".into(),
                api_secret: "secret".into(),
            }),
            Environment::Production,
        )
        .with_rest_url_override(mock_server.uri());

        let result = client
            .request(
                "private/get-order-detail",
                serde_json::json!({"order_id": "123"}),
            )
            .await
            .unwrap();
        assert!(result["data"].is_array());
    }

    #[tokio::test]
    async fn test_public_request() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"id": -1, "code": 0, "result": {"data": [{"i": "BTC_USDT"}]}}),
            ))
            .mount(&mock_server)
            .await;

        let client =
            ApiClient::new(None, Environment::Production).with_rest_url_override(mock_server.uri());
        let result = client
            .public_request(
                "public/get-tickers",
                serde_json::json!({"instrument_name": "BTC_USDT"}),
            )
            .await
            .unwrap();
        assert!(result["data"].is_array());
    }

    #[tokio::test]
    async fn test_cdc_error_response() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"id": 1, "code": 10002, "message": "UNAUTHORIZED"}),
            ))
            .mount(&mock_server)
            .await;

        let client = ApiClient::new(
            Some(Credentials {
                api_key: "bad".into(),
                api_secret: "bad".into(),
            }),
            Environment::Production,
        )
        .with_rest_url_override(mock_server.uri());

        let err = client
            .request("private/get-order-detail", serde_json::json!({}))
            .await
            .unwrap_err();
        let envelope = err.to_envelope();
        assert_eq!(envelope.category, ErrorCategory::Auth);
    }

    #[tokio::test]
    async fn test_http_502_error_for_signed_request() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(502)
                    .set_body_string("<html><body>Bad Gateway</body></html>"),
            )
            .mount(&mock_server)
            .await;

        let client = ApiClient::new(
            Some(Credentials {
                api_key: "key".into(),
                api_secret: "secret".into(),
            }),
            Environment::Production,
        )
        .with_rest_url_override(mock_server.uri());

        let err = client
            .request("private/get-order-detail", serde_json::json!({}))
            .await
            .unwrap_err();

        let envelope = err.to_envelope();
        // Should be Network error, not JSON parse error
        assert_eq!(envelope.category, ErrorCategory::Network);
        // Message should mention HTTP 502, not JSON parse
        assert!(
            envelope.message.contains("502") || envelope.message.contains("Bad Gateway"),
            "Expected 502 Bad Gateway in error message, got: {}",
            envelope.message
        );
    }

    #[tokio::test]
    async fn test_http_502_error_for_public_request() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(
                wiremock::ResponseTemplate::new(502)
                    .set_body_string("<html><body>Bad Gateway</body></html>"),
            )
            .mount(&mock_server)
            .await;

        let client =
            ApiClient::new(None, Environment::Production).with_rest_url_override(mock_server.uri());

        let err = client
            .public_request("public/get-tickers", serde_json::json!({}))
            .await
            .unwrap_err();

        let envelope = err.to_envelope();
        // Should be Network error, not JSON parse error
        assert_eq!(envelope.category, ErrorCategory::Network);
        // Message should mention HTTP 502, not JSON parse
        assert!(
            envelope.message.contains("502") || envelope.message.contains("Bad Gateway"),
            "Expected 502 Bad Gateway in error message, got: {}",
            envelope.message
        );
    }

    #[tokio::test]
    async fn test_http_500_error_for_signed_request() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(500))
            .mount(&mock_server)
            .await;

        let client = ApiClient::new(
            Some(Credentials {
                api_key: "key".into(),
                api_secret: "secret".into(),
            }),
            Environment::Production,
        )
        .with_rest_url_override(mock_server.uri());

        let err = client
            .request("private/get-order-detail", serde_json::json!({}))
            .await
            .unwrap_err();

        let envelope = err.to_envelope();
        assert_eq!(envelope.category, ErrorCategory::Network);
        assert!(envelope.retryable);
    }

    #[tokio::test]
    async fn test_http_429_rate_limit_error() {
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(wiremock::ResponseTemplate::new(429))
            .mount(&mock_server)
            .await;

        let client =
            ApiClient::new(None, Environment::Production).with_rest_url_override(mock_server.uri());

        let err = client
            .public_request("public/get-tickers", serde_json::json!({}))
            .await
            .unwrap_err();

        let envelope = err.to_envelope();
        // 429 should be Network category (retryable)
        assert_eq!(envelope.category, ErrorCategory::Network);
        assert!(envelope.retryable);
    }

    #[tokio::test]
    async fn test_http_400_with_api_error_body() {
        // The CDC API returns 400 with a JSON body containing the actual error.
        // We must surface the API error, not the HTTP status.
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(400).set_body_json(
                serde_json::json!({"id": 1, "code": 20001, "message": "INSUFFICIENT_AVAILABLE_BALANCE"}),
            ))
            .mount(&mock_server)
            .await;

        let client = ApiClient::new(
            Some(Credentials {
                api_key: "key".into(),
                api_secret: "secret".into(),
            }),
            Environment::Production,
        )
        .with_rest_url_override(mock_server.uri());

        let err = client
            .request("private/create-order", serde_json::json!({}))
            .await
            .unwrap_err();

        let envelope = err.to_envelope();
        assert_eq!(envelope.category, ErrorCategory::InsufficientFunds);
        assert_eq!(envelope.code, 20001);
        assert_eq!(envelope.message, "INSUFFICIENT_AVAILABLE_BALANCE");
        assert!(!envelope.retryable);
    }

    #[tokio::test]
    async fn test_warning_response_with_result() {
        // API returns non-zero code WITH a result (e.g. FAR_AWAY_LIMIT_PRICE, IMMEDIATE_OR_CANCEL).
        // The operation succeeded — we must return Ok with the result, not Err.
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": 1,
                    "code": 315,
                    "message": "FAR_AWAY_LIMIT_PRICE",
                    "result": {
                        "client_oid": "abc123",
                        "order_id": "5368290798789202966"
                    }
                })),
            )
            .mount(&mock_server)
            .await;

        let client = ApiClient::new(
            Some(Credentials {
                api_key: "key".into(),
                api_secret: "secret".into(),
            }),
            Environment::Production,
        )
        .with_rest_url_override(mock_server.uri());

        let result = client
            .request("private/create-order", serde_json::json!({}))
            .await
            .unwrap();

        assert_eq!(result["order_id"], "5368290798789202966");
        assert_eq!(result["client_oid"], "abc123");
        assert_eq!(result["code"], 315);
        assert_eq!(result["message"], "FAR_AWAY_LIMIT_PRICE");
    }

    #[tokio::test]
    async fn test_error_without_result_still_errors() {
        // API returns non-zero code WITHOUT a result — this is a real error.
        let mock_server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"id": 1, "code": 10002, "message": "UNAUTHORIZED"}),
            ))
            .mount(&mock_server)
            .await;

        let client = ApiClient::new(
            Some(Credentials {
                api_key: "key".into(),
                api_secret: "secret".into(),
            }),
            Environment::Production,
        )
        .with_rest_url_override(mock_server.uri());

        let err = client
            .request("private/create-order", serde_json::json!({}))
            .await
            .unwrap_err();

        let envelope = err.to_envelope();
        assert_eq!(envelope.category, ErrorCategory::Auth);
        assert_eq!(envelope.code, 10002);
    }
}
