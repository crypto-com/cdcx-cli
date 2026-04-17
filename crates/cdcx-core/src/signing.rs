use crate::error::CdcxError;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn generate_nonce() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_millis() as u64
}

/// Build the param string by recursively sorting keys alphabetically
/// and concatenating key+value pairs. Handles nested objects and arrays
/// per the CDC Exchange API signing specification.
pub fn build_param_string(params: &serde_json::Value) -> String {
    let mut out = String::new();
    build_param_recursive(params, &mut out);
    out
}

fn build_param_recursive(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                out.push_str(key);
                if let Some(v) = map.get(key) {
                    build_param_recursive(v, out);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                build_param_recursive(item, out);
            }
        }
        serde_json::Value::String(s) => out.push_str(s),
        serde_json::Value::Number(n) => out.push_str(&n.to_string()),
        serde_json::Value::Bool(b) => out.push_str(&b.to_string()),
        serde_json::Value::Null => out.push_str("null"),
    }
}

pub fn sign_request(
    method: &str,
    id: u64,
    api_key: &str,
    secret: &str,
    params: &serde_json::Value,
    nonce: u64,
) -> Result<String, CdcxError> {
    let param_string = build_param_string(params);
    let sig_payload = format!("{}{}{}{}{}", method, id, api_key, param_string, nonce);
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| CdcxError::Config("Invalid HMAC key length".into()))?;
    mac.update(sig_payload.as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_build_param_string_flat() {
        let params = json!({"instrument_name": "BTC_USDT", "count": 10});
        let s = build_param_string(&params);
        assert_eq!(s, "count10instrument_nameBTC_USDT");
    }

    #[test]
    fn test_build_param_string_sorted() {
        let params = json!({"z_param": "last", "a_param": "first", "m_param": "middle"});
        let s = build_param_string(&params);
        assert_eq!(s, "a_paramfirstm_parammiddlez_paramlast");
    }

    #[test]
    fn test_build_param_string_nested_object() {
        let params = json!({"outer": {"b": "2", "a": "1"}});
        let s = build_param_string(&params);
        assert_eq!(s, "outera1b2");
    }

    #[test]
    fn test_build_param_string_array() {
        let params = json!({"items": [{"b": "2"}, {"a": "1"}]});
        let s = build_param_string(&params);
        assert_eq!(s, "itemsb2a1");
    }

    #[test]
    fn test_build_param_string_nested_order_list() {
        // Simulates OTOCO/OCO order_list parameter
        let params = json!({
            "contingency_type": "OCO",
            "order_list": [
                {"instrument_name": "BTC_USDT", "side": "BUY", "type": "LIMIT", "price": "29000"},
                {"instrument_name": "BTC_USDT", "side": "BUY", "type": "LIMIT", "price": "31000"}
            ]
        });
        let s = build_param_string(&params);
        // contingency_type + OCO + order_list + [array items recursed in order]
        assert!(s.starts_with("contingency_typeOCOorder_list"));
        assert!(s.contains("instrument_nameBTC_USDT"));
    }

    #[test]
    fn test_build_param_string_null() {
        let params = json!({"key": null});
        let s = build_param_string(&params);
        assert_eq!(s, "keynull");
    }

    #[test]
    fn test_sign_request_produces_hex() {
        let sig = sign_request(
            "private/get-order-detail",
            1,
            "api_key",
            "secret",
            &json!({}),
            1234567890,
        )
        .unwrap();
        assert_eq!(sig.len(), 64);
        assert!(sig.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_sign_request_deterministic() {
        let params = json!({"instrument_name": "BTC_USDT"});
        let sig1 = sign_request("method", 1, "key", "secret", &params, 100).unwrap();
        let sig2 = sign_request("method", 1, "key", "secret", &params, 100).unwrap();
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn test_sign_request_with_nested_params() {
        let params = json!({
            "contingency_type": "OCO",
            "order_list": [
                {"instrument_name": "BTC_USDT", "side": "BUY", "type": "LIMIT", "price": "29000"},
                {"instrument_name": "BTC_USDT", "side": "BUY", "type": "LIMIT", "price": "31000"}
            ]
        });
        let sig = sign_request(
            "private/advanced/create-oco",
            5,
            "key",
            "secret",
            &params,
            100,
        )
        .unwrap();
        assert_eq!(sig.len(), 64);
    }

    #[test]
    fn test_generate_nonce() {
        let nonce = generate_nonce();
        assert!(nonce > 1_577_836_800_000);
    }
}
