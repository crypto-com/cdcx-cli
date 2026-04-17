use crate::openapi::table_hints::{channel_to_method, get_table_hint, TableHint};
use crate::openapi::types::{navigate, ResponseSchema};
use serde_json::Value;

/// Format API response as a table using table hints and/or response schema.
/// `method` is the OpenAPI method (e.g., "public/get-tickers").
pub fn format_table(
    method: &str,
    data: &Value,
    _response_schema: Option<&ResponseSchema>,
) -> Option<String> {
    let hint = get_table_hint(method)?;

    if hint.is_book_layout {
        return format_book_table(data, &hint);
    }

    let items = navigate(data, hint.data_path)?.as_array()?;

    let headers: Vec<String> = hint.headers.iter().map(|h| h.to_string()).collect();

    let rows: Vec<Vec<String>> = items
        .iter()
        .map(|item| {
            hint.columns
                .iter()
                .map(|col| extract_display_value(item, col))
                .collect()
        })
        .collect();

    render_table(&headers, &rows)
}

/// Format a book response with dual bid/ask columns.
fn format_book_table(data: &Value, hint: &TableHint) -> Option<String> {
    let book_data = navigate(data, hint.data_path)?;
    let bids = book_data.get("bids")?.as_array()?;
    let asks = book_data.get("asks")?.as_array()?;

    let headers = vec![
        "BID_PRICE".to_string(),
        "BID_QTY".to_string(),
        "BID_COUNT".to_string(),
        "  ".to_string(),
        "ASK_PRICE".to_string(),
        "ASK_QTY".to_string(),
        "ASK_COUNT".to_string(),
    ];

    let max_len = bids.len().max(asks.len());
    let mut rows = Vec::new();
    for i in 0..max_len {
        let bid = bids.get(i).and_then(|b| b.as_array());
        let ask = asks.get(i).and_then(|a| a.as_array());

        let row = vec![
            bid.and_then(|b| b.first())
                .and_then(|v| v.as_str())
                .unwrap_or("-")
                .to_string(),
            bid.and_then(|b| b.get(1))
                .and_then(|v| v.as_str())
                .unwrap_or("-")
                .to_string(),
            bid.and_then(|b| b.get(2))
                .and_then(|v| v.as_str())
                .unwrap_or("-")
                .to_string(),
            "|".to_string(),
            ask.and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .unwrap_or("-")
                .to_string(),
            ask.and_then(|a| a.get(1))
                .and_then(|v| v.as_str())
                .unwrap_or("-")
                .to_string(),
            ask.and_then(|a| a.get(2))
                .and_then(|v| v.as_str())
                .unwrap_or("-")
                .to_string(),
        ];
        rows.push(row);
    }

    render_table(&headers, &rows)
}

/// Extract a display value from a JSON item, applying timestamp formatting.
fn extract_display_value(item: &Value, field: &str) -> String {
    let raw = match item.get(field) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => {
            if *b {
                "yes".to_string()
            } else {
                "no".to_string()
            }
        }
        Some(Value::Null) | None => return "-".to_string(),
        Some(other) => other.to_string(),
    };

    // Timestamp formatting for known timestamp fields
    if field == "t" || field.ends_with("_time") || field.ends_with("_timestamp_ms") {
        if let Ok(ms) = raw.parse::<i64>() {
            if let Some(dt) = chrono::DateTime::from_timestamp_millis(ms) {
                return if field.len() <= 2 {
                    dt.format("%H:%M:%S").to_string()
                } else {
                    dt.format("%Y-%m-%d %H:%M").to_string()
                };
            }
        }
    }

    raw
}

/// Render a table with dynamic column widths.
fn render_table(headers: &[String], rows: &[Vec<String>]) -> Option<String> {
    if headers.is_empty() {
        return None;
    }

    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.len());
            }
        }
    }

    let header_line: String = headers
        .iter()
        .enumerate()
        .map(|(i, h)| format!("{:<width$}", h, width = widths[i]))
        .collect::<Vec<_>>()
        .join("  ");

    let mut lines = vec![header_line];

    for row in rows {
        let line: String = row
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                let w = widths.get(i).copied().unwrap_or(cell.len());
                format!("{:<width$}", cell, width = w)
            })
            .collect::<Vec<_>>()
            .join("  ");
        lines.push(line);
    }

    Some(lines.join("\n"))
}

/// Returns the header line for a streaming channel's table output.
pub fn stream_header(channel: &str) -> Option<&'static str> {
    match channel {
        "ticker" => Some(
            "INSTRUMENT       LAST           BID            ASK            24H_CHANGE     VOLUME",
        ),
        "trade" => Some("TIME             INSTRUMENT       SIDE     PRICE            QUANTITY"),
        "book" => Some("INSTRUMENT       BID            BID_QTY        ASK            ASK_QTY"),
        "mark" | "index" | "settlement" | "funding" | "estimatedfunding" => {
            Some("INSTRUMENT       TIME                        VALUE")
        }
        _ => None,
    }
}

/// Formats a single streaming data update as table row(s).
pub fn format_stream_rows(channel: &str, result: &Value) -> Option<String> {
    let data = result.get("data")?.as_array()?;

    // Simple {v, t} channels: mark, index, settlement, funding, estimatedfunding
    if matches!(
        channel,
        "mark" | "index" | "settlement" | "funding" | "estimatedfunding"
    ) {
        let instrument = result
            .get("instrument_name")
            .and_then(|i| i.as_str())
            .unwrap_or("-");
        let rows: Vec<String> = data
            .iter()
            .map(|item| {
                let ts = item
                    .get("t")
                    .and_then(|t| t.as_i64())
                    .and_then(chrono::DateTime::from_timestamp_millis)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_else(|| "-".to_string());
                let value = item
                    .get("v")
                    .map(|v| match v {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        other => other.to_string(),
                    })
                    .unwrap_or_else(|| "-".to_string());
                format!("{:<16} {:<27} {}", instrument, ts, value)
            })
            .collect();
        return Some(rows.join("\n"));
    }

    let method = channel_to_method(channel)?;
    let hint = get_table_hint(method)?;

    if hint.is_book_layout {
        let instrument = result
            .get("instrument_name")
            .and_then(|i| i.as_str())
            .unwrap_or("-");
        if let Some(item) = data.first() {
            let bids = item.get("bids").and_then(|b| b.as_array());
            let asks = item.get("asks").and_then(|a| a.as_array());
            let bid_price = bids
                .and_then(|b| b.first())
                .and_then(|b| b.get(0))
                .and_then(|p| p.as_str())
                .unwrap_or("-");
            let bid_qty = bids
                .and_then(|b| b.first())
                .and_then(|b| b.get(1))
                .and_then(|q| q.as_str())
                .unwrap_or("-");
            let ask_price = asks
                .and_then(|a| a.first())
                .and_then(|a| a.get(0))
                .and_then(|p| p.as_str())
                .unwrap_or("-");
            let ask_qty = asks
                .and_then(|a| a.first())
                .and_then(|a| a.get(1))
                .and_then(|q| q.as_str())
                .unwrap_or("-");
            Some(format!(
                "{:<16} {:<14} {:<14} {:<14} {}",
                instrument, bid_price, bid_qty, ask_price, ask_qty
            ))
        } else {
            None
        }
    } else {
        let rows: Vec<String> = data
            .iter()
            .map(|item| {
                hint.columns
                    .iter()
                    .enumerate()
                    .map(|(i, col)| {
                        let val = extract_display_value(item, col);
                        let width = if i < hint.headers.len() {
                            hint.headers[i].len().max(14)
                        } else {
                            14
                        };
                        format!("{:<width$}", val, width = width)
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        Some(rows.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_instruments_table_with_real_fields() {
        let data = json!({"data": [
            {"symbol": "BTC_USDT", "display_name": "BTC/USDT", "base_ccy": "BTC",
             "quote_ccy": "USDT", "inst_type": "CCY_PAIR", "tradable": true}
        ]});
        let table = format_table("public/get-instruments", &data, None).unwrap();
        assert!(table.contains("BTC_USDT"), "Missing symbol");
        assert!(table.contains("BTC/USDT"), "Missing display_name");
        assert!(table.contains("INSTRUMENT"), "Missing header");
        assert!(table.contains("yes"), "Boolean should be 'yes'");
    }

    #[test]
    fn test_ticker_table_with_real_fields() {
        let data = json!({"data": [
            {"i": "BTC_USDT", "a": "50000", "b": "49999", "k": "50001", "c": "0.015", "v": "1000"}
        ]});
        let table = format_table("public/get-tickers", &data, None).unwrap();
        assert!(table.contains("BTC_USDT"));
        assert!(table.contains("50000"));
        assert!(table.contains("INSTRUMENT"));
    }

    #[test]
    fn test_book_table_with_real_structure() {
        let data = json!({
            "data": [{
                "bids": [["49999.00", "0.5", "3"], ["49998.00", "1.0", "5"]],
                "asks": [["50001.00", "0.3", "2"], ["50002.00", "0.8", "4"]]
            }]
        });
        let table = format_table("public/get-book", &data, None).unwrap();
        assert!(table.contains("49999.00"));
        assert!(table.contains("50001.00"));
        assert!(table.contains("BID_PRICE"));
    }

    #[test]
    fn test_unknown_method_returns_none_without_schema() {
        let data = json!({"data": []});
        assert!(format_table("unknown/method", &data, None).is_none());
    }

    #[test]
    fn test_empty_data_returns_header_only() {
        let data = json!({"data": []});
        let table = format_table("public/get-instruments", &data, None).unwrap();
        assert!(table.contains("INSTRUMENT"));
        assert_eq!(table.lines().count(), 1);
    }

    #[test]
    fn test_trades_table_formats_timestamps() {
        let data = json!({"data": [
            {"t": 1773998941132_i64, "i": "BTC_USDT", "s": "sell", "p": "70518.00", "q": "0.001"}
        ]});
        let table = format_table("public/get-trades", &data, None).unwrap();
        assert!(
            !table.contains("1773998941132"),
            "Timestamp should be formatted, not raw"
        );
        assert!(table.contains(":"), "Should contain time separator");
        assert!(table.contains("BTC_USDT"));
    }

    #[test]
    fn test_stream_rows_ticker() {
        let result = json!({
            "data": [{"i": "BTC_USDT", "a": "50000", "b": "49999", "k": "50001", "c": "0.01", "v": "100"}]
        });
        let rows = format_stream_rows("ticker", &result).unwrap();
        assert!(rows.contains("BTC_USDT"));
        assert!(rows.contains("50000"));
    }

    #[test]
    fn test_stream_rows_book() {
        let result = json!({
            "instrument_name": "BTC_USDT",
            "data": [{"bids": [["50000", "1.0", "3"]], "asks": [["50001", "0.5", "2"]]}]
        });
        let rows = format_stream_rows("book", &result).unwrap();
        assert!(rows.contains("BTC_USDT"));
        assert!(rows.contains("50000"));
    }
}
