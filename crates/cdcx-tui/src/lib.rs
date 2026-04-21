#![allow(
    clippy::collapsible_else_if,
    clippy::collapsible_if,
    clippy::double_ended_iterator_last,
    clippy::filter_map_identity,
    clippy::get_first,
    clippy::manual_is_multiple_of,
    clippy::manual_clamp,
    clippy::needless_borrow,
    clippy::needless_borrows_for_generic_args,
    clippy::unnecessary_filter_map,
    clippy::needless_range_loop,
    clippy::new_without_default,
    clippy::redundant_closure,
    clippy::result_large_err
)]

pub mod app;
pub mod config;
pub mod event;
pub mod format;
pub mod loading;
pub mod setup;
pub mod state;
pub mod streaming;
pub mod tabs;
pub mod theme;
pub mod widgets;
pub mod workflows;

use app::App;
use config::TuiConfig;
use event::{Event, EventHandler};
use state::{AppState, ConnectionStatus, RestRequest, TickerData};
use streaming::StreamManager;
use tabs::DataEvent;
use theme::Theme;

use cdcx_core::api_client::ApiClient;
use cdcx_core::auth::Credentials;
use cdcx_core::env::Environment;
use std::sync::Arc;

pub struct TuiOptions {
    pub env: Environment,
    pub profile: Option<String>,
    pub theme: Option<String>,
    pub setup: bool,
}

pub async fn run(opts: TuiOptions) -> Result<(), Box<dyn std::error::Error>> {
    // Run setup wizard if --setup flag or first launch (no tui.toml)
    let needs_setup = opts.setup || !TuiConfig::exists();
    if needs_setup {
        let mut terminal = ratatui::init();
        let mut events = EventHandler::new(250);
        let proceed = setup::run_setup(&mut events, &mut terminal).await;
        ratatui::restore();
        if !proceed {
            return Ok(());
        }
        // Small pause so terminal resets cleanly before re-entering
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }

    // Load TUI config (re-read after setup may have written it)
    let config = TuiConfig::load();
    let theme_name = opts
        .theme
        .as_deref()
        .or(config.theme.as_deref())
        .unwrap_or("terminal-pro");
    // Try builtin first, then custom themes from tui.toml
    let theme = Theme::builtin(theme_name).unwrap_or_else(|| {
        config
            .themes
            .get(theme_name)
            .map(|custom| custom.to_theme(theme_name, &Theme::default().colors))
            .unwrap_or_default()
    });

    // Resolve credentials (optional — dashboard works without auth for market data)
    let cdcx_config = load_cdcx_config()?;
    if cdcx_config.is_some() {
        if let Some(path) = cdcx_core::config::Config::default_path() {
            cdcx_core::config::check_config_permissions(&path)?;
        }
    }
    let credentials = Credentials::resolve(cdcx_config.as_ref(), opts.profile.as_deref()).ok();
    let authenticated = credentials.is_some();
    let api = Arc::new(ApiClient::new(credentials, opts.env));

    // Enter terminal immediately for the loading screen
    let mut terminal = ratatui::init();
    // Enable mouse capture for click/scroll support
    crossterm::execute!(std::io::stdout(), crossterm::event::EnableMouseCapture).ok();
    let mut loading_events = EventHandler::new(60); // fast tick for smooth animation
    let mut loading_state = loading::LoadingState::new();

    // Spawn both fetches concurrently, collect results with animated loading
    let (instruments, instrument_types, initial_tickers) = {
        use tokio::sync::oneshot;

        let (inst_tx, mut inst_rx) =
            oneshot::channel::<(Vec<String>, std::collections::HashMap<String, String>)>();
        let (tick_tx, mut tick_rx) =
            oneshot::channel::<std::collections::HashMap<String, TickerData>>();

        let api_inst = api.clone();
        tokio::spawn(async move {
            let result = fetch_instruments(&api_inst).await;
            let _ = inst_tx.send(result);
        });

        let api_tick = api.clone();
        tokio::spawn(async move {
            let result = fetch_tickers(&api_tick).await;
            let _ = tick_tx.send(result);
        });

        let mut instruments = None;
        let mut tickers = None;
        let loading_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);

        loop {
            terminal.draw(|f| loading::draw_loading(f, &loading_state, &theme.colors))?;

            if loading_state.step == loading::LoadingStep::Done {
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                break;
            }

            tokio::select! {
                biased;
                result = &mut inst_rx, if instruments.is_none() => {
                    if let Ok(data) = result {
                        loading_state.instrument_count = data.0.len();
                        instruments = Some(data);
                        if loading_state.step == loading::LoadingStep::Instruments {
                            loading_state.step = loading::LoadingStep::Tickers;
                        }
                    }
                }
                result = &mut tick_rx, if tickers.is_none() => {
                    if let Ok(data) = result {
                        loading_state.ticker_count = data.len();
                        tickers = Some(data);
                        if loading_state.step == loading::LoadingStep::Tickers
                            || loading_state.step == loading::LoadingStep::Instruments {
                            // If instruments already done, advance; otherwise stay
                            if instruments.is_some() {
                                loading_state.step = loading::LoadingStep::Tickers;
                            }
                        }
                    }
                }
                event = loading_events.next() => {
                    match event {
                        Some(event::Event::Key(key))
                            if key.code == crossterm::event::KeyCode::Char('q')
                                || key.code == crossterm::event::KeyCode::Esc
                                || (key.code == crossterm::event::KeyCode::Char('c')
                                    && key.modifiers
                                        == crossterm::event::KeyModifiers::CONTROL) =>
                        {
                            ratatui::restore();
                            return Ok(());
                        }
                        Some(event::Event::Tick) => {
                            loading_state.tick();
                        }
                        _ => {}
                    }
                }
            }

            // Check if all data is ready, or if we've timed out
            let all_ready = instruments.is_some() && tickers.is_some();
            let timed_out = tokio::time::Instant::now() >= loading_deadline;
            if (all_ready || timed_out) && loading_state.step != loading::LoadingStep::Done {
                loading_state.step = loading::LoadingStep::Connecting;
                terminal.draw(|f| loading::draw_loading(f, &loading_state, &theme.colors))?;
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                loading_state.step = loading::LoadingStep::Done;
            }
        }

        let (inst_names, inst_types) = instruments.unwrap_or_default();
        (inst_names, inst_types, tickers.unwrap_or_default())
    };

    // Transition from loading to dashboard — drop the fast-tick event handler
    drop(loading_events);

    // REST request/response channels — tabs send requests, main loop spawns tasks
    let (rest_req_tx, mut rest_req_rx) = tokio::sync::mpsc::unbounded_channel::<RestRequest>();
    let (rest_resp_tx, mut rest_resp_rx) =
        tokio::sync::mpsc::unbounded_channel::<(String, Result<serde_json::Value, String>)>();

    let state = AppState {
        instruments,
        instrument_types,
        tickers: initial_tickers,
        sparklines: std::collections::HashMap::new(),
        alerts: vec![],
        authenticated,
        env: opts.env,
        theme,
        terminal_size: crossterm::terminal::size().unwrap_or((80, 24)),
        market_connection: ConnectionStatus::Connecting,
        api: api.clone(),
        rest_tx: rest_req_tx,
        toast: None,
        session_start_value: None,
        current_portfolio_value: 0.0,
        ticker_speed_divisor: config.ticker_speed_divisor(),
        price_flashes: std::collections::HashMap::new(),
        paper_mode: false,
        paper_engine: cdcx_core::paper::engine::PaperEngine::load_or_init(10000.0).ok(),
        volume_unit: crate::state::VolumeUnit::Usd,
        pending_navigation: None,
    };

    let watchlist = config.watchlist.clone();
    let mut app = App::new(state, &watchlist);
    app.tick_rate_ms = config.tick_rate();

    // Initialize active tab with data
    app.on_tick();

    let mut events = EventHandler::new(config.tick_rate());

    // Stream manager for real-time WebSocket data
    let (stream_tx, mut stream_rx) = tokio::sync::mpsc::unbounded_channel();
    let stream_mgr = StreamManager::spawn(opts.env, stream_tx);

    // Initial subscriptions based on active tab
    let mut last_subs = app.active_subscriptions();
    stream_mgr.update_subscriptions(last_subs.clone());

    // Paper mode flag shared between event loop and REST processor
    let paper_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let paper_flag_rest = paper_flag.clone();

    // Spawn REST request processor — takes requests from tabs, spawns API calls
    let rest_api = api.clone();
    tokio::spawn(async move {
        while let Some(req) = rest_req_rx.recv().await {
            let api = rest_api.clone();
            let tx = rest_resp_tx.clone();
            let is_paper = paper_flag_rest.load(std::sync::atomic::Ordering::SeqCst);

            // In paper mode, block private trading requests
            if is_paper && req.is_private {
                let _ = tx.send((
                    req.method,
                    Err("Paper mode — use paper trading commands".into()),
                ));
                continue;
            }

            tokio::spawn(async move {
                let (api_method, response_tag) = if req.method == "sparkline-candles" {
                    (
                        "public/get-candlestick".to_string(),
                        "sparkline-candles".to_string(),
                    )
                } else {
                    (req.method.clone(), req.method.clone())
                };
                let result = if req.is_private {
                    api.request(&api_method, req.params).await
                } else {
                    api.public_request(&api_method, req.params).await
                };
                match result {
                    Ok(data) => {
                        let _ = tx.send((response_tag, Ok(data)));
                    }
                    Err(e) => {
                        let _ = tx.send((response_tag, Err(e.to_string())));
                    }
                }
            });
        }
    });

    // Continue using the terminal from loading screen
    while !app.should_quit {
        terminal.draw(|f| app.draw(f))?;

        tokio::select! {
            event = events.next() => {
                if let Some(event) = event {
                    match event {
                        Event::Key(key) => {
                            // Sync paper flag BEFORE key processing so REST processor
                            // sees current state before any new requests are queued
                            paper_flag.store(app.state.paper_mode, std::sync::atomic::Ordering::SeqCst);
                            let prev_tab = app.active_tab;
                            app.on_key(key);
                            // Sync again AFTER — catches 'p' toggle that happened during on_key
                            paper_flag.store(app.state.paper_mode, std::sync::atomic::Ordering::SeqCst);
                            if app.active_tab != prev_tab {
                                app.activate_current_tab();
                                app.on_tick(); // init new tab
                                // Full buffer clear on tab switch — different tabs
                                // have completely different layouts, so ratatui's
                                // cell-level diff can produce artifacts
                                let _ = terminal.clear();
                            }
                            // Check subscriptions — view mode changes within a
                            // tab (table → chart → compare) change what channels
                            // are needed, and layout changes need a full redraw
                            let subs = app.active_subscriptions();
                            if subs != last_subs {
                                stream_mgr.update_subscriptions(subs.clone());
                                last_subs = subs;
                                // Clear buffer when subscriptions change — layout
                                // is likely different, avoid diff artifacts
                                let _ = terminal.clear();
                            }
                        }
                        Event::Mouse(mouse) => {
                            let prev_tab = app.active_tab;
                            app.on_mouse(mouse);
                            if app.active_tab != prev_tab {
                                app.activate_current_tab();
                                app.on_tick();
                                let _ = terminal.clear();
                            }
                            let subs = app.active_subscriptions();
                            if subs != last_subs {
                                stream_mgr.update_subscriptions(subs.clone());
                                last_subs = subs;
                                let _ = terminal.clear();
                            }
                        }
                        Event::Tick => {
                            app.on_tick();
                        }
                        Event::Resize(w, h) => app.on_resize(w, h),
                    }
                }
            }
            stream_event = stream_rx.recv() => {
                if let Some(se) = stream_event {
                    match se {
                        streaming::StreamEvent::TickerUpdate(val) => {
                            if let Some(ticker) = TickerData::from_json(&val) {
                                app.on_data(DataEvent::TickerUpdate(ticker));
                            }
                        }
                        streaming::StreamEvent::CandleUpdate { instrument, data } => {
                            if let Some(candle) = crate::widgets::candlestick::Candle::from_json(&data) {
                                app.on_data(DataEvent::CandleUpdate { instrument, candle });
                            }
                        }
                        streaming::StreamEvent::BookUpdate(val) => {
                            app.on_data(DataEvent::BookSnapshot(val));
                        }
                        streaming::StreamEvent::TradeUpdate(val) => {
                            app.on_data(DataEvent::TradeSnapshot(val));
                        }
                        streaming::StreamEvent::ConnectionStatus(cs) => {
                            match cs {
                                streaming::ConnectionStatusEvent::MarketConnected => {
                                    if app.state.market_connection != ConnectionStatus::Connected {
                                        app.state.toast("WebSocket connected", state::ToastStyle::Success);
                                    }
                                    app.state.market_connection = ConnectionStatus::Connected;
                                }
                                streaming::ConnectionStatusEvent::MarketReconnecting => {
                                    app.state.toast("Reconnecting...", state::ToastStyle::Info);
                                    app.state.market_connection = ConnectionStatus::Reconnecting;
                                }
                                streaming::ConnectionStatusEvent::MarketError(_) => {
                                    app.state.toast("Connection lost", state::ToastStyle::Error);
                                    app.state.market_connection = ConnectionStatus::Error;
                                }
                            }
                        }
                    }
                }
            }
            rest_resp = rest_resp_rx.recv() => {
                if let Some((method, result)) = rest_resp {
                    match result {
                        Ok(data) => {
                            app.on_data(DataEvent::RestResponse { method, data });
                        }
                        Err(err) => {
                            // Show error as toast — don't silently swallow failures
                            let short = if err.len() > 60 { format!("{}...", &err[..57]) } else { err };
                            app.state.toast(
                                format!("{}: {}", method.split('/').next_back().unwrap_or(&method), short),
                                state::ToastStyle::Error,
                            );
                        }
                    }
                }
            }
        }
    }

    stream_mgr.shutdown();
    crossterm::execute!(std::io::stdout(), crossterm::event::DisableMouseCapture).ok();
    ratatui::restore();
    Ok(())
}

fn load_cdcx_config() -> Result<Option<cdcx_core::config::Config>, cdcx_core::error::CdcxError> {
    cdcx_core::config::Config::load_default()
}

/// Fetches instruments and their types from the API.
/// Returns (sorted instrument names, symbol → inst_type mapping).
async fn fetch_instruments(
    api: &ApiClient,
) -> (Vec<String>, std::collections::HashMap<String, String>) {
    match api
        .public_request("public/get-instruments", serde_json::json!({}))
        .await
    {
        Ok(val) => {
            let mut names = Vec::new();
            let mut types = std::collections::HashMap::new();
            if let Some(arr) = val.get("data").and_then(|d| d.as_array()) {
                for item in arr {
                    if let Some(symbol) = item
                        .get("symbol")
                        .or_else(|| item.get("instrument_name"))
                        .and_then(|v| v.as_str())
                    {
                        let inst_type = item
                            .get("inst_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("CCY_PAIR")
                            .to_string();
                        names.push(symbol.to_string());
                        types.insert(symbol.to_string(), inst_type);
                    }
                }
            }
            names.sort();
            (names, types)
        }
        Err(_) => (vec![], std::collections::HashMap::new()),
    }
}

async fn fetch_tickers(api: &ApiClient) -> std::collections::HashMap<String, TickerData> {
    let mut map = std::collections::HashMap::new();
    if let Ok(val) = api
        .public_request("public/get-tickers", serde_json::json!({}))
        .await
    {
        if let Some(arr) = val.get("data").and_then(|d| d.as_array()) {
            for item in arr {
                if let Some(ticker) = TickerData::from_json(item) {
                    map.insert(ticker.instrument.clone(), ticker);
                }
            }
        }
    }
    map
}
