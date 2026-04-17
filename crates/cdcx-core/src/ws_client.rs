use crate::auth::Credentials;
use crate::error::CdcxError;
use crate::signing;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::{connect_async, tungstenite::Message};

pub fn backoff_ms(attempt: u32) -> u64 {
    std::cmp::min(2u64.pow(attempt) * 100, 30_000)
}

pub struct WsClient {
    write: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    read: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    subscribed_channels: Vec<String>,
}

impl WsClient {
    pub async fn connect(url: &str) -> Result<Self, CdcxError> {
        if !url.starts_with("wss://") {
            return Err(CdcxError::Config(
                "WebSocket connections must use wss:// (TLS required)".into(),
            ));
        }
        let (ws_stream, _) = connect_async(url).await?;
        let (write, read) = ws_stream.split();
        Ok(Self {
            write,
            read,
            subscribed_channels: Vec::new(),
        })
    }

    pub async fn subscribe(&mut self, channels: Vec<String>) -> Result<(), CdcxError> {
        let msg = serde_json::json!({
            "id": 1,
            "method": "subscribe",
            "params": { "channels": channels },
        });
        self.write.send(Message::Text(msg.to_string())).await?;
        self.subscribed_channels = channels;
        Ok(())
    }

    pub async fn unsubscribe(&mut self, channels: Vec<String>) -> Result<(), CdcxError> {
        let msg = serde_json::json!({
            "id": 2,
            "method": "unsubscribe",
            "params": { "channels": channels },
        });
        self.write.send(Message::Text(msg.to_string())).await?;
        self.subscribed_channels.retain(|c| !channels.contains(c));
        Ok(())
    }

    pub async fn authenticated_connect(
        url: &str,
        credentials: &Credentials,
    ) -> Result<Self, CdcxError> {
        let mut client = Self::connect(url).await?;
        // Send auth message
        let nonce = signing::generate_nonce();
        let id = 1u64;
        let sig = signing::sign_request(
            "public/auth",
            id,
            &credentials.api_key,
            &credentials.api_secret,
            &serde_json::json!({}),
            nonce,
        )?;
        let auth_msg = serde_json::json!({
            "id": id,
            "method": "public/auth",
            "api_key": credentials.api_key,
            "sig": sig,
            "nonce": nonce,
        });
        client
            .write
            .send(Message::Text(auth_msg.to_string()))
            .await?;
        // Wait for auth response
        while let Some(msg) = client.read.next().await {
            match msg? {
                Message::Text(text) => {
                    let resp: serde_json::Value = serde_json::from_str(&text)?;
                    if resp.get("method").and_then(|m| m.as_str()) == Some("public/auth") {
                        let code = resp.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
                        if code != 0 {
                            return Err(CdcxError::Config(
                                "WebSocket authentication failed".into(),
                            ));
                        }
                        break;
                    }
                }
                Message::Ping(data) => {
                    client.write.send(Message::Pong(data)).await?;
                }
                _ => {}
            }
        }
        Ok(client)
    }

    /// Read the next JSON message, handling pings and heartbeats automatically
    pub async fn next_message(&mut self) -> Option<Result<serde_json::Value, CdcxError>> {
        loop {
            match self.read.next().await? {
                Ok(Message::Text(text)) => {
                    let value: serde_json::Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(e) => return Some(Err(CdcxError::from(e))),
                    };
                    // Respond to application-level heartbeats (required by Crypto.com Exchange API)
                    if value.get("method").and_then(|m| m.as_str()) == Some("public/heartbeat") {
                        let id = value.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
                        let resp = serde_json::json!({
                            "id": id,
                            "method": "public/respond-heartbeat"
                        });
                        let _ = self.write.send(Message::Text(resp.to_string())).await;
                        continue;
                    }
                    return Some(Ok(value));
                }
                Ok(Message::Ping(data)) => {
                    let _ = self.write.send(Message::Pong(data)).await;
                    continue;
                }
                Ok(Message::Close(_)) => return None,
                Err(e) => return Some(Err(e.into())),
                _ => continue,
            }
        }
    }

    pub async fn close(&mut self) -> Result<(), CdcxError> {
        self.write.send(Message::Close(None)).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_calculation() {
        assert_eq!(backoff_ms(0), 100);
        assert_eq!(backoff_ms(1), 200);
        assert_eq!(backoff_ms(2), 400);
        assert_eq!(backoff_ms(10), 30_000); // capped
        assert_eq!(backoff_ms(11), 30_000); // still capped
    }

    #[test]
    fn test_auth_message_format() {
        let nonce = 1234567890u64;
        let sig = crate::signing::sign_request(
            "public/auth",
            1,
            "test_key",
            "test_secret",
            &serde_json::json!({}),
            nonce,
        )
        .unwrap();
        assert_eq!(sig.len(), 64);
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
