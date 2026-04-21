use crate::theme::Theme;
use cdcx_core::api_client::ApiClient;
use cdcx_core::env::Environment;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

#[derive(Debug, Clone, Default)]
pub struct TickerData {
    pub instrument: String,
    pub ask: f64,
    pub bid: f64,
    pub change_pct: f64,
    pub high: f64,
    pub low: f64,
    pub volume: f64,
    pub volume_usd: f64,
    pub funding_rate: f64,
}

impl TickerData {
    pub fn from_json(val: &serde_json::Value) -> Option<Self> {
        let i = val.get("i")?.as_str()?;
        Some(Self {
            instrument: i.to_string(),
            ask: parse_f64(val, "a"),
            bid: parse_f64(val, "b"),
            change_pct: parse_f64(val, "c"),
            high: parse_f64(val, "h"),
            low: parse_f64(val, "l"),
            volume: parse_f64(val, "v"),
            volume_usd: parse_f64(val, "vv"),
            funding_rate: parse_f64(val, "fr"),
        })
    }
}

fn parse_f64(val: &serde_json::Value, key: &str) -> f64 {
    val.get(key)
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    Connecting,
    Connected,
    Reconnecting,
    Error,
}

/// A request from a tab to fetch REST data in the background.
#[derive(Debug, Clone)]
pub struct RestRequest {
    pub method: String,
    pub params: serde_json::Value,
    pub is_private: bool,
}

/// Max number of price samples to keep per instrument for sparklines.
const SPARKLINE_MAX: usize = 30;

#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub style: ToastStyle,
    pub expires_at: std::time::Instant,
}

#[derive(Debug, Clone, Copy)]
pub enum ToastStyle {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone)]
pub struct PriceAlert {
    pub instrument: String,
    pub target_price: f64,
    pub direction: AlertDirection,
    pub triggered: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertDirection {
    Above,
    Below,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VolumeUnit {
    #[default]
    Usd,
    Notional,
}

pub struct AppState {
    pub instruments: Vec<String>,
    pub instrument_types: HashMap<String, String>,
    pub tickers: HashMap<String, TickerData>,
    pub sparklines: HashMap<String, Vec<f64>>,
    pub alerts: Vec<PriceAlert>,
    pub authenticated: bool,
    pub env: Environment,
    pub theme: Theme,
    pub terminal_size: (u16, u16),
    pub market_connection: ConnectionStatus,
    pub api: Arc<ApiClient>,
    pub rest_tx: mpsc::UnboundedSender<RestRequest>,
    pub toast: Option<Toast>,
    pub session_start_value: Option<f64>,
    pub current_portfolio_value: f64,
    pub ticker_speed_divisor: u64,
    /// Tracks price flash animations: instrument → (up=true/down=false, when).
    pub price_flashes: HashMap<String, (bool, std::time::Instant)>,
    pub paper_mode: bool,
    pub paper_engine: Option<cdcx_core::paper::engine::PaperEngine>,
    /// Volume display unit preference
    pub volume_unit: VolumeUnit,
    /// Cross-tab navigation request: (target tab, instrument to show in detail).
    pub pending_navigation: Option<(crate::tabs::TabKind, String)>,
}

impl AppState {
    /// Record a price sample for sparkline rendering.
    /// Check all alerts against current prices, returning triggered messages.
    pub fn check_alerts(&mut self) -> Vec<String> {
        let mut messages = Vec::new();
        for alert in &mut self.alerts {
            if alert.triggered {
                continue;
            }
            if let Some(ticker) = self.tickers.get(&alert.instrument) {
                let triggered = match alert.direction {
                    AlertDirection::Above => ticker.ask >= alert.target_price,
                    AlertDirection::Below => ticker.ask <= alert.target_price,
                };
                if triggered {
                    alert.triggered = true;
                    let dir = match alert.direction {
                        AlertDirection::Above => "\u{2191}",
                        AlertDirection::Below => "\u{2193}",
                    };
                    messages.push(format!(
                        "{} {} {:.2} (now {:.2})",
                        alert.instrument, dir, alert.target_price, ticker.ask
                    ));
                }
            }
        }
        messages
    }

    pub fn record_sparkline(&mut self, instrument: &str, price: f64) {
        if price <= 0.0 {
            return;
        }
        let entry = self.sparklines.entry(instrument.to_string()).or_default();
        // Always record — even same price shows time progression.
        // The sparkline represents "last N ticks" not "last N unique prices".
        entry.push(price);
        if entry.len() > SPARKLINE_MAX {
            entry.remove(0);
        }
    }

    /// Seed sparklines from initial ticker data so they aren't blank on first render.
    pub fn seed_sparklines(&mut self) {
        for (inst, ticker) in &self.tickers {
            if ticker.ask > 0.0 {
                let entry = self.sparklines.entry(inst.clone()).or_default();
                if entry.is_empty() {
                    // Seed with a few data points spanning low→ask to show range
                    let mid = (ticker.low + ticker.high) / 2.0;
                    entry.push(ticker.low);
                    entry.push(mid);
                    entry.push(ticker.ask);
                }
            }
        }
    }
    pub fn toast(&mut self, message: impl Into<String>, style: ToastStyle) {
        self.toast = Some(Toast {
            message: message.into(),
            style,
            expires_at: std::time::Instant::now() + std::time::Duration::from_secs(3),
        });
    }

    pub fn active_toast(&self) -> Option<&Toast> {
        self.toast
            .as_ref()
            .filter(|t| std::time::Instant::now() < t.expires_at)
    }

    pub fn env_label(&self) -> &'static str {
        match self.env {
            Environment::Production => "PROD",
            Environment::Uat => "UAT",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_ticker_from_json() {
        let val = json!({
            "i": "BTC_USDT",
            "a": "67234.50",
            "b": "67230.00",
            "c": "2.34",
            "h": "68100.00",
            "l": "65800.00",
            "v": "1234.5",
            "vv": "82000000"
        });
        let t = TickerData::from_json(&val).unwrap();
        assert_eq!(t.instrument, "BTC_USDT");
        assert!((t.ask - 67234.50).abs() < 0.01);
        assert!((t.change_pct - 2.34).abs() < 0.01);
        assert!((t.volume_usd - 82_000_000.0).abs() < 1.0);
    }

    #[test]
    fn test_ticker_missing_fields() {
        let val = json!({"i": "ETH_USDT"});
        let t = TickerData::from_json(&val).unwrap();
        assert_eq!(t.instrument, "ETH_USDT");
        assert_eq!(t.ask, 0.0);
    }

    #[test]
    fn test_ticker_no_instrument() {
        let val = json!({"a": "100"});
        assert!(TickerData::from_json(&val).is_none());
    }
}
