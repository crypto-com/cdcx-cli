use cdcx_core::env::Environment;
use cdcx_core::ws_client::WsClient;
use std::collections::HashSet;
use tokio::sync::mpsc;

#[derive(Debug)]
pub enum StreamEvent {
    TickerUpdate(serde_json::Value),
    CandleUpdate {
        instrument: String,
        data: serde_json::Value,
    },
    BookUpdate(serde_json::Value),
    TradeUpdate(serde_json::Value),
    /// `user.positions` channel — array of position records.
    PositionsUpdate(Vec<serde_json::Value>),
    /// `user.balance` channel — array of currency balance records.
    BalanceUpdate(Vec<serde_json::Value>),
    /// `user.order` channel — array of order records.
    OrdersUpdate(Vec<serde_json::Value>),
    ConnectionStatus(ConnectionStatusEvent),
}

#[derive(Debug)]
pub enum ConnectionStatusEvent {
    MarketConnected,
    MarketReconnecting,
    MarketError(String),
    UserConnected,
    UserReconnecting,
    UserError(String),
}

#[derive(Debug)]
enum StreamCommand {
    UpdateSubscriptions(Vec<String>),
    Shutdown,
}

pub struct StreamManager {
    command_tx: mpsc::UnboundedSender<StreamCommand>,
}

impl StreamManager {
    pub fn spawn(env: Environment, event_tx: mpsc::UnboundedSender<StreamEvent>) -> Self {
        let (command_tx, mut command_rx) = mpsc::unbounded_channel();

        tokio::spawn(async move {
            let mut current_subs: HashSet<String> = HashSet::new();
            let mut client: Option<WsClient> = None;
            let mut retry_count = 0u32;

            loop {
                // Connect if not connected
                if client.is_none() {
                    let _ = event_tx.send(StreamEvent::ConnectionStatus(
                        ConnectionStatusEvent::MarketReconnecting,
                    ));
                    match WsClient::connect(&env.ws_market_url()).await {
                        Ok(ws) => {
                            client = Some(ws);
                            retry_count = 0;
                            let _ = event_tx.send(StreamEvent::ConnectionStatus(
                                ConnectionStatusEvent::MarketConnected,
                            ));
                            // Resubscribe to current channels
                            if !current_subs.is_empty() {
                                let channels: Vec<String> = current_subs.iter().cloned().collect();
                                if let Some(ref mut ws) = client {
                                    let _ = ws.subscribe(channels).await;
                                }
                            }
                        }
                        Err(e) => {
                            let _ = event_tx.send(StreamEvent::ConnectionStatus(
                                ConnectionStatusEvent::MarketError(e.to_string()),
                            ));
                            retry_count += 1;
                            let delay = std::cmp::min(500 * 2u64.pow(retry_count.min(10)), 30_000);
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                            continue;
                        }
                    }
                }

                let ws = client.as_mut().unwrap();

                tokio::select! {
                    msg = ws.next_message() => {
                        match msg {
                            Some(Ok(value)) => {
                                // Route data messages by channel
                                // API sends: {"result": {"channel": "ticker", "subscription": "ticker.BTC_USDT", "data": [...]}}
                                if let Some(result) = value.get("result") {
                                    let channel = result.get("channel").and_then(|c| c.as_str()).unwrap_or("");
                                    let subscription = result.get("subscription").and_then(|s| s.as_str()).unwrap_or("");

                                    if channel == "ticker" || subscription.starts_with("ticker.") {
                                        if let Some(data) = result.get("data") {
                                            if let Some(arr) = data.as_array() {
                                                for item in arr {
                                                    let _ = event_tx.send(
                                                        StreamEvent::TickerUpdate(item.clone()),
                                                    );
                                                }
                                            } else {
                                                let _ = event_tx.send(
                                                    StreamEvent::TickerUpdate(data.clone()),
                                                );
                                            }
                                        }
                                    }

                                    // Candlestick channel: "candlestick" with subscription "candlestick.1h.BTC_USDT"
                                    if channel == "candlestick" || subscription.starts_with("candlestick.") {
                                        // Extract instrument from subscription: "candlestick.{interval}.{instrument}"
                                        let instrument = subscription
                                            .splitn(3, '.')
                                            .nth(2)
                                            .unwrap_or("")
                                            .to_string();
                                        if let Some(data) = result.get("data") {
                                            if let Some(arr) = data.as_array() {
                                                for item in arr {
                                                    let _ = event_tx.send(
                                                        StreamEvent::CandleUpdate {
                                                            instrument: instrument.clone(),
                                                            data: item.clone(),
                                                        },
                                                    );
                                                }
                                            } else {
                                                let _ = event_tx.send(
                                                    StreamEvent::CandleUpdate {
                                                        instrument,
                                                        data: data.clone(),
                                                    },
                                                );
                                            }
                                        }
                                    }

                                    // Book channel
                                    if channel == "book" || subscription.starts_with("book.") {
                                        let _ = event_tx.send(
                                            StreamEvent::BookUpdate(value.clone()),
                                        );
                                    }

                                    // Trade channel
                                    if channel == "trade" || subscription.starts_with("trade.") {
                                        if let Some(data) = result.get("data") {
                                            let _ = event_tx.send(
                                                StreamEvent::TradeUpdate(data.clone()),
                                            );
                                        }
                                    }
                                }
                            }
                            Some(Err(_)) | None => {
                                // Connection lost — will reconnect on next loop iteration
                                client = None;
                                continue;
                            }
                        }
                    }
                    cmd = command_rx.recv() => {
                        match cmd {
                            Some(StreamCommand::UpdateSubscriptions(new_channels)) => {
                                let new_set: HashSet<String> = new_channels.into_iter().collect();
                                let to_sub: Vec<String> =
                                    new_set.difference(&current_subs).cloned().collect();
                                let to_unsub: Vec<String> =
                                    current_subs.difference(&new_set).cloned().collect();
                                if !to_unsub.is_empty() {
                                    let _ = ws.unsubscribe(to_unsub).await;
                                }
                                if !to_sub.is_empty() {
                                    let _ = ws.subscribe(to_sub).await;
                                }
                                current_subs = new_set;
                            }
                            Some(StreamCommand::Shutdown) | None => break,
                        }
                    }
                }
            }
        });

        Self { command_tx }
    }

    pub fn update_subscriptions(&self, channels: Vec<String>) {
        let _ = self
            .command_tx
            .send(StreamCommand::UpdateSubscriptions(channels));
    }

    pub fn shutdown(&self) {
        let _ = self.command_tx.send(StreamCommand::Shutdown);
    }
}

/// Authenticated user-stream manager. Connects to ws_user_url, authenticates with the
/// provided credentials, subscribes to the given user.* channels, and pushes updates
/// into the same `StreamEvent` bus the market stream uses. Reconnects with backoff and
/// re-authenticates+resubscribes on drop.
pub struct UserStreamManager {
    shutdown_tx: mpsc::UnboundedSender<()>,
}

impl UserStreamManager {
    pub fn spawn(
        env: Environment,
        credentials: cdcx_core::auth::Credentials,
        channels: Vec<String>,
        event_tx: mpsc::UnboundedSender<StreamEvent>,
    ) -> Self {
        let (shutdown_tx, mut shutdown_rx) = mpsc::unbounded_channel::<()>();

        tokio::spawn(async move {
            let mut client: Option<cdcx_core::ws_client::WsClient> = None;
            let mut retry_count = 0u32;

            loop {
                if client.is_none() {
                    let _ = event_tx.send(StreamEvent::ConnectionStatus(
                        ConnectionStatusEvent::UserReconnecting,
                    ));
                    match cdcx_core::ws_client::WsClient::authenticated_connect(
                        &env.ws_user_url(),
                        &credentials,
                    )
                    .await
                    {
                        Ok(mut ws) => {
                            if let Err(e) = ws.subscribe(channels.clone()).await {
                                let _ = event_tx.send(StreamEvent::ConnectionStatus(
                                    ConnectionStatusEvent::UserError(e.to_string()),
                                ));
                                retry_count += 1;
                                let delay =
                                    std::cmp::min(500 * 2u64.pow(retry_count.min(10)), 30_000);
                                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                                continue;
                            }
                            client = Some(ws);
                            retry_count = 0;
                            let _ = event_tx.send(StreamEvent::ConnectionStatus(
                                ConnectionStatusEvent::UserConnected,
                            ));
                        }
                        Err(e) => {
                            let _ = event_tx.send(StreamEvent::ConnectionStatus(
                                ConnectionStatusEvent::UserError(e.to_string()),
                            ));
                            retry_count += 1;
                            let delay = std::cmp::min(500 * 2u64.pow(retry_count.min(10)), 30_000);
                            tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                            continue;
                        }
                    }
                }

                let ws = client.as_mut().unwrap();

                tokio::select! {
                    msg = ws.next_message() => {
                        match msg {
                            Some(Ok(value)) => {
                                Self::route(&value, &event_tx);
                            }
                            Some(Err(_)) | None => {
                                client = None;
                                continue;
                            }
                        }
                    }
                    _ = shutdown_rx.recv() => break,
                }
            }
        });

        Self { shutdown_tx }
    }

    fn route(value: &serde_json::Value, event_tx: &mpsc::UnboundedSender<StreamEvent>) {
        let Some(result) = value.get("result") else {
            return;
        };
        let channel = result.get("channel").and_then(|c| c.as_str()).unwrap_or("");
        let subscription = result
            .get("subscription")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let is_positions = channel == "user.positions" || subscription == "user.positions";
        let is_balance = channel == "user.balance" || subscription == "user.balance";
        let is_orders = channel == "user.order" || subscription == "user.order";
        if !(is_positions || is_balance || is_orders) {
            return;
        }
        let Some(data) = result.get("data") else {
            return;
        };
        let arr: Vec<serde_json::Value> = data
            .as_array()
            .cloned()
            .unwrap_or_else(|| vec![data.clone()]);
        if is_positions {
            let _ = event_tx.send(StreamEvent::PositionsUpdate(arr));
        } else if is_balance {
            let _ = event_tx.send(StreamEvent::BalanceUpdate(arr));
        } else if is_orders {
            let _ = event_tx.send(StreamEvent::OrdersUpdate(arr));
        }
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(());
    }
}
