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
    ConnectionStatus(ConnectionStatusEvent),
}

#[derive(Debug)]
pub enum ConnectionStatusEvent {
    MarketConnected,
    MarketReconnecting,
    MarketError(String),
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
