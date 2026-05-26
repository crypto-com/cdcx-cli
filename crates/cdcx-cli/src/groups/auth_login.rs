use cdcx_core::error::CdcxError;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use serde::Deserialize;
use std::io;
use std::time::{Duration, Instant};
use uuid::Uuid;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const POLL_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Deserialize)]
struct PollResponse {
    code: i64,
    result: Option<PollResult>,
}

#[derive(Deserialize)]
struct PollResult {
    api_key: String,
    secret_key: String,
}

struct RawModeGuard;

impl RawModeGuard {
    fn enable() -> io::Result<Self> {
        crossterm::terminal::enable_raw_mode()?;
        Ok(Self)
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

fn oauth_url(environment: &str, session_id: &str) -> String {
    match environment {
        "uat" => format!(
            "https://dpre-xweb.3ona.co/exchange/authorize_broker?broker_id=cdcx-cli\
             &redirect_uri=https%3A%2F%2Fx-growth.crypto.com%2Fstatic%2Fcli%2Fauthenticate%2Fuat%3Fid%3D{session_id}\
             &broker_reference_id={session_id}"
        ),
        _ => format!(
            "https://crypto.com/exchange/authorize_broker?broker_id=cdcx-cli\
             &redirect_uri=https%3A%2F%2Fx-growth.crypto.com%2Fstatic%2Fcli%2Fauthenticate%3Fid%3D{session_id}\
             &broker_reference_id={session_id}"
        ),
    }
}

fn poll_url(environment: &str) -> &'static str {
    match environment {
        "uat" => "https://x-growth.crypto.com/static/cli/authenticate/uat",
        _ => "https://x-growth.crypto.com/static/cli/authenticate",
    }
}

/// Run the browser-based OAuth flow for the given environment.
/// Returns `Ok(Some((api_key, secret_key)))` on success, `Ok(None)` if cancelled.
pub async fn browser_oauth(environment: &str) -> Result<Option<(String, String)>, CdcxError> {
    let session_id = Uuid::new_v4().to_string();

    let url = oauth_url(environment, &session_id);
    println!();
    println!("  Opening browser for authentication...");
    println!("  {}", url);

    if let Err(_) = open::that(&url) {
        println!();
        println!("  Could not open browser automatically.");
        println!("  Please open the URL above manually.");
    }

    println!();
    println!("  Waiting for authentication... (press ESC to cancel)");
    println!();

    poll_for_credentials(environment, &session_id).await
}

pub async fn run_auth_login() -> Result<(), CdcxError> {
    super::setup::run_setup().await
}

async fn poll_for_credentials(
    environment: &str,
    session_id: &str,
) -> Result<Option<(String, String)>, CdcxError> {
    let _guard = RawModeGuard::enable()
        .map_err(|e| CdcxError::Config(format!("Failed to enable raw mode: {}", e)))?;

    // Spawn a blocking thread for ESC key detection, communicating via channel
    let (esc_tx, mut esc_rx) = tokio::sync::mpsc::channel::<()>(1);
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel_clone = cancel.clone();
    tokio::task::spawn_blocking(move || {
        while !cancel_clone.load(std::sync::atomic::Ordering::Relaxed) {
            if let Ok(true) = event::poll(Duration::from_millis(100)) {
                if let Ok(Event::Key(key_event)) = event::read() {
                    if key_event.kind == KeyEventKind::Press && key_event.code == KeyCode::Esc {
                        let _ = esc_tx.blocking_send(());
                        return;
                    }
                }
            }
        }
    });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .no_proxy()
        .pool_max_idle_per_host(0)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let base_url = poll_url(environment);
    let start = Instant::now();

    let result = loop {
        if start.elapsed() > POLL_TIMEOUT {
            break Err(CdcxError::Config(
                "Authentication timed out after 5 minutes".into(),
            ));
        }
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let url = format!("{}?ts={}", base_url, ts);
        let body = serde_json::json!({ "session": session_id, "ts": ts });
        tokio::select! {
            resp = client.post(&url).json(&body)
                .header("Cache-Control", "no-cache, no-store")
                .header("Pragma", "no-cache")
                .send() => {
                if let Ok(r) = resp {
                    if r.status() == reqwest::StatusCode::OK {
                        if let Ok(poll_resp) = r.json::<PollResponse>().await {
                            if poll_resp.code == 0 {
                                if let Some(result) = poll_resp.result {
                                    break Ok(Some((result.api_key, result.secret_key)));
                                }
                            }
                        }
                    }
                }
            }
            _ = esc_rx.recv() => {
                break Ok(None);
            }
        }

        // Wait before next poll, still listening for ESC
        tokio::select! {
            _ = tokio::time::sleep(POLL_INTERVAL) => {}
            _ = esc_rx.recv() => {
                break Ok(None);
            }
        }
    };

    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── URL generation ───

    #[test]
    fn test_oauth_url_production() {
        let url = oauth_url("production", "abc-123");
        assert!(url.contains("broker_id=cdcx-cli"));
        assert!(url.contains("broker_reference_id=abc-123"));
        assert!(url.contains("authenticate%3Fid%3Dabc-123"));
        assert!(!url.contains("/uat"));
    }

    #[test]
    fn test_oauth_url_uat() {
        let url = oauth_url("uat", "abc-123");
        assert!(url.contains("broker_id=cdcx-cli"));
        assert!(url.contains("broker_reference_id=abc-123"));
        assert!(url.contains("authenticate%2Fuat%3Fid%3Dabc-123"));
    }

    #[test]
    fn test_oauth_url_uses_session_id_in_both_params() {
        let url = oauth_url("production", "my-uuid-value");
        assert_eq!(
            url.matches("my-uuid-value").count(),
            2,
            "session ID should appear in both redirect_uri and broker_reference_id"
        );
    }

    #[test]
    fn test_poll_url_production() {
        assert_eq!(
            poll_url("production"),
            "https://x-growth.crypto.com/static/cli/authenticate"
        );
    }

    #[test]
    fn test_poll_url_uat() {
        assert_eq!(
            poll_url("uat"),
            "https://x-growth.crypto.com/static/cli/authenticate/uat"
        );
    }

    #[test]
    fn test_poll_url_unknown_defaults_to_production() {
        assert_eq!(poll_url("anything"), poll_url("production"));
    }

    // ─── Poll response deserialization ───

    #[test]
    fn test_poll_response_success() {
        let json = r#"{"id":1,"method":"private/broker/create-fast-api-key","code":0,"result":{"ts":1779687323265,"api_key":"the_key","secret_key":"the_secret","enabled_trading":true,"enabled_withdrawal":false}}"#;
        let resp: PollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        let result = resp.result.unwrap();
        assert_eq!(result.api_key, "the_key");
        assert_eq!(result.secret_key, "the_secret");
    }

    #[test]
    fn test_poll_response_non_zero_code() {
        let json = r#"{"id":1,"method":"something","code":10001}"#;
        let resp: PollResponse = serde_json::from_str(json).unwrap();
        assert_ne!(resp.code, 0);
        assert!(resp.result.is_none());
    }

    #[test]
    fn test_poll_response_with_extra_fields() {
        let json = r#"{"id":1,"method":"private/broker/create-fast-api-key","code":0,"result":{"ts":123,"api_key":"k","secret_key":"s","enabled_trading":true,"enabled_withdrawal":false,"extra":"ignored"}}"#;
        let resp: PollResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.code, 0);
        assert_eq!(resp.result.unwrap().api_key, "k");
    }
}
