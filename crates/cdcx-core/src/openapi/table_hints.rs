/// Static table display hints keyed by OpenAPI method path.
#[derive(Debug, Clone)]
pub struct TableHint {
    pub columns: &'static [&'static str],
    pub headers: &'static [&'static str],
    pub data_path: &'static str,
    pub is_book_layout: bool,
}

/// All methods that have explicit table hints.
pub const ALL_HINTED_METHODS: &[&str] = &[
    "public/get-instruments",
    "public/get-tickers",
    "public/get-book",
    "public/get-trades",
    "public/get-candlestick",
    "private/user-balance",
    "private/get-open-orders",
    "private/get-order-history",
    "private/get-trades",
    "private/get-deposit-history",
    "private/get-withdrawal-history",
];

pub fn get_table_hint(method: &str) -> Option<TableHint> {
    match method {
        "public/get-instruments" => Some(TableHint {
            columns: &[
                "symbol",
                "display_name",
                "base_ccy",
                "quote_ccy",
                "inst_type",
                "tradable",
            ],
            headers: &["INSTRUMENT", "NAME", "BASE", "QUOTE", "TYPE", "TRADABLE"],
            data_path: "data",
            is_book_layout: false,
        }),
        "public/get-tickers" => Some(TableHint {
            columns: &["i", "a", "b", "k", "c", "v"],
            headers: &["INSTRUMENT", "LAST", "BID", "ASK", "24H_CHANGE", "VOLUME"],
            data_path: "data",
            is_book_layout: false,
        }),
        "public/get-book" => Some(TableHint {
            columns: &["bids", "asks"],
            headers: &[
                "BID_PRICE",
                "BID_QTY",
                "BID_COUNT",
                "ASK_PRICE",
                "ASK_QTY",
                "ASK_COUNT",
            ],
            data_path: "data.0",
            is_book_layout: true,
        }),
        "public/get-trades" => Some(TableHint {
            columns: &["t", "i", "s", "p", "q"],
            headers: &["TIME", "INSTRUMENT", "SIDE", "PRICE", "QUANTITY"],
            data_path: "data",
            is_book_layout: false,
        }),
        "public/get-candlestick" => Some(TableHint {
            columns: &["t", "o", "h", "l", "c", "v"],
            headers: &["TIME", "OPEN", "HIGH", "LOW", "CLOSE", "VOLUME"],
            data_path: "data",
            is_book_layout: false,
        }),
        "private/user-balance" => Some(TableHint {
            columns: &[
                "instrument_name",
                "total_available_balance",
                "total_margin_balance",
                "total_position_value",
            ],
            headers: &["INSTRUMENT", "AVAILABLE", "MARGIN", "POSITION_VALUE"],
            data_path: "position_balances",
            is_book_layout: false,
        }),
        "private/get-open-orders" | "private/get-order-history" => Some(TableHint {
            columns: &[
                "order_id",
                "instrument_name",
                "side",
                "order_type",
                "limit_price",
                "quantity",
                "status",
            ],
            headers: &[
                "ORDER_ID",
                "INSTRUMENT",
                "SIDE",
                "TYPE",
                "PRICE",
                "QTY",
                "STATUS",
            ],
            data_path: "data",
            is_book_layout: false,
        }),
        "private/get-trades" => Some(TableHint {
            columns: &[
                "trade_id",
                "instrument_name",
                "side",
                "traded_price",
                "traded_quantity",
                "fee",
            ],
            headers: &["TRADE_ID", "INSTRUMENT", "SIDE", "PRICE", "QTY", "FEE"],
            data_path: "data",
            is_book_layout: false,
        }),
        "private/get-deposit-history" | "private/get-withdrawal-history" => Some(TableHint {
            columns: &["id", "currency", "amount", "status", "create_time"],
            headers: &["ID", "CURRENCY", "AMOUNT", "STATUS", "TIME"],
            data_path: "data",
            is_book_layout: false,
        }),
        _ => None,
    }
}

/// Map a WebSocket channel name to the equivalent REST OpenAPI method.
pub fn channel_to_method(channel: &str) -> Option<&'static str> {
    match channel {
        "ticker" => Some("public/get-tickers"),
        "trade" => Some("public/get-trades"),
        "book" => Some("public/get-book"),
        "candlestick" => Some("public/get-candlestick"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_hint_returns_some() {
        let hint = get_table_hint("public/get-instruments").unwrap();
        assert_eq!(hint.columns[0], "symbol");
        assert_eq!(hint.headers[0], "INSTRUMENT");
        assert_eq!(hint.columns.len(), hint.headers.len());
    }

    #[test]
    fn test_unknown_method_returns_none() {
        assert!(get_table_hint("unknown/method").is_none());
    }

    #[test]
    fn test_book_hint_is_special() {
        let hint = get_table_hint("public/get-book").unwrap();
        assert!(hint.is_book_layout);
    }

    #[test]
    fn test_all_hints_have_matching_column_header_counts() {
        for method in ALL_HINTED_METHODS {
            let hint = get_table_hint(method).unwrap();
            // book layout has different column/header semantics
            if !hint.is_book_layout {
                assert_eq!(
                    hint.columns.len(),
                    hint.headers.len(),
                    "Mismatch for {}",
                    method
                );
            }
        }
    }

    #[test]
    fn test_channel_to_method() {
        assert_eq!(channel_to_method("ticker"), Some("public/get-tickers"));
        assert_eq!(channel_to_method("book"), Some("public/get-book"));
        assert_eq!(channel_to_method("unknown"), None);
    }
}
