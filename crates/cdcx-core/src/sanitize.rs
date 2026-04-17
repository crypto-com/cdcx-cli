use crate::error::ErrorEnvelope;
use regex::Regex;
use std::sync::OnceLock;

/// Static ANSI escape sequence regex pattern
fn ansi_regex() -> &'static Regex {
    static ANSI: OnceLock<Regex> = OnceLock::new();
    ANSI.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*[a-zA-Z]").expect("ANSI regex should be valid"))
}

/// Validates input against adversarial attacks.
///
/// Checks for:
/// - Path traversal (`..`)
/// - Embedded query params (`?`, `&`)
/// - Double-encoding (`%25`)
/// - Control characters (C0 range except \n\t)
///
/// Returns `Ok(())` if input is valid, otherwise returns a `Validation` error.
pub fn validate_input(field_name: &str, value: &str) -> Result<(), ErrorEnvelope> {
    // Check for path traversal
    if value.contains("..") {
        return Err(ErrorEnvelope::validation(&format!(
            "Field '{}' contains path traversal pattern",
            field_name
        )));
    }

    // Check for both forward slash and backslash variants
    if value.contains("..\\") {
        return Err(ErrorEnvelope::validation(&format!(
            "Field '{}' contains path traversal pattern",
            field_name
        )));
    }

    // Check for embedded query params
    if value.contains('?') || value.contains('&') {
        return Err(ErrorEnvelope::validation(&format!(
            "Field '{}' contains embedded query parameters",
            field_name
        )));
    }

    // Check for double-encoding
    if value.contains("%25") {
        return Err(ErrorEnvelope::validation(&format!(
            "Field '{}' contains double-encoded characters",
            field_name
        )));
    }

    // Check for control characters (C0 range, except \n and \t)
    for ch in value.chars() {
        let code = ch as u32;
        // C0 control characters: 0x00-0x1F
        if code <= 0x1F {
            // Allow \n (0x0A) and \t (0x09)
            if code != 0x0A && code != 0x09 {
                return Err(ErrorEnvelope::validation(&format!(
                    "Field '{}' contains invalid control character (0x{:02X})",
                    field_name, code
                )));
            }
        }
    }

    Ok(())
}

/// Recursively validates all string values in a JSON object.
///
/// For each string field, calls `validate_input()` with the field name as context.
/// Returns an error on the first invalid string found.
pub fn validate_json_payload(json: &serde_json::Value) -> Result<(), ErrorEnvelope> {
    match json {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                match value {
                    serde_json::Value::String(s) => {
                        validate_input(key, s)?;
                    }
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                        validate_json_payload(value)?;
                    }
                    _ => {}
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for (idx, item) in arr.iter().enumerate() {
                match item {
                    serde_json::Value::String(s) => {
                        validate_input(&format!("[{}]", idx), s)?;
                    }
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                        validate_json_payload(item)?;
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }

    Ok(())
}

/// Sanitizes a string by removing control characters and ANSI escape sequences.
///
/// - Strips C0 control characters (except \n and \t)
/// - Strips ANSI escape sequences
/// - Truncates to max_field_size with "[truncated]" suffix if necessary
pub fn sanitize_string(s: &str, max_field_size: usize) -> String {
    // First, strip ANSI escape sequences
    let no_ansi = ansi_regex().replace_all(s, "").into_owned();

    // Then, strip C0 control characters except \n and \t
    let no_control: String = no_ansi
        .chars()
        .filter(|ch| {
            let code = *ch as u32;
            // Keep everything except C0 control chars (except \n and \t)
            if code <= 0x1F {
                code == 0x0A || code == 0x09 // Allow \n and \t
            } else {
                true // Keep all other characters
            }
        })
        .collect();

    // Finally, truncate if necessary
    // Use char-boundary-safe truncation to avoid panicking on multi-byte UTF-8 chars
    let char_count = no_control.chars().count();
    if char_count > max_field_size {
        let truncated: String = no_control.chars().take(max_field_size).collect();
        format!("{}[truncated]", truncated)
    } else {
        no_control
    }
}

/// Recursively sanitizes all string values in a JSON response.
///
/// Walks the JSON tree and applies `sanitize_string()` to all string values.
/// Respects field size limits and response size limits.
pub fn sanitize_response(value: serde_json::Value, max_field_size: usize) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut sanitized = serde_json::Map::new();
            for (key, val) in map {
                let sanitized_val = match val {
                    serde_json::Value::String(s) => {
                        serde_json::Value::String(sanitize_string(&s, max_field_size))
                    }
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                        sanitize_response(val, max_field_size)
                    }
                    other => other,
                };
                sanitized.insert(key, sanitized_val);
            }
            serde_json::Value::Object(sanitized)
        }
        serde_json::Value::Array(arr) => {
            let sanitized: Vec<serde_json::Value> = arr
                .into_iter()
                .map(|item| match item {
                    serde_json::Value::String(s) => {
                        serde_json::Value::String(sanitize_string(&s, max_field_size))
                    }
                    serde_json::Value::Object(_) | serde_json::Value::Array(_) => {
                        sanitize_response(item, max_field_size)
                    }
                    other => other,
                })
                .collect();
            serde_json::Value::Array(sanitized)
        }
        serde_json::Value::String(s) => {
            serde_json::Value::String(sanitize_string(&s, max_field_size))
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== TASK 8: INPUT VALIDATION TESTS =====

    #[test]
    fn test_reject_path_traversal() {
        assert!(validate_input("instrument", "../etc/passwd").is_err());
        assert!(validate_input("instrument", "..\\windows").is_err());
    }

    #[test]
    fn test_reject_embedded_query_params() {
        assert!(validate_input("instrument", "BTC?foo=bar").is_err());
        assert!(validate_input("instrument", "BTC&x=1").is_err());
    }

    #[test]
    fn test_reject_double_encoding() {
        assert!(validate_input("instrument", "BTC%2525").is_err());
    }

    #[test]
    fn test_reject_control_chars() {
        assert!(validate_input("instrument", "BTC\x00USDT").is_err());
    }

    #[test]
    fn test_accept_normal_input() {
        assert!(validate_input("instrument", "BTC_USDT").is_ok());
        assert!(validate_input("instrument", "ETH_CRO").is_ok());
    }

    // ===== TASK 9: RESPONSE SANITIZATION TESTS =====

    #[test]
    fn test_strip_c0_chars() {
        assert_eq!(sanitize_string("hello\x01world", 10240), "helloworld");
        assert_eq!(sanitize_string("hello\nworld", 10240), "hello\nworld");
    }

    #[test]
    fn test_strip_ansi() {
        assert_eq!(
            sanitize_string("hello\x1b[31mred\x1b[0m", 10240),
            "hellored"
        );
    }

    #[test]
    fn test_truncate_large_field() {
        let big = "x".repeat(20_000);
        let result = sanitize_string(&big, 10240);
        assert!(result.len() <= 10240 + "[truncated]".len());
        assert!(result.ends_with("[truncated]"));
    }

    #[test]
    fn test_sanitize_response_walks_json() {
        let input = serde_json::json!({"name": "test\x01", "nested": {"val": "ok\x1b[31m"}});
        let output = sanitize_response(input, 10240);
        assert_eq!(output["name"], "test");
        assert_eq!(output["nested"]["val"], "ok");
    }

    // ===== ADDITIONAL VALIDATION TESTS =====

    #[test]
    fn test_validate_json_payload_nested() {
        let valid = serde_json::json!({"field": "value", "nested": {"inner": "data"}});
        assert!(validate_json_payload(&valid).is_ok());

        let invalid = serde_json::json!({"field": "../etc/passwd"});
        assert!(validate_json_payload(&invalid).is_err());
    }

    #[test]
    fn test_validate_json_payload_array() {
        let valid = serde_json::json!({"items": ["one", "two", "three"]});
        assert!(validate_json_payload(&valid).is_ok());

        let invalid = serde_json::json!({"items": ["ok", "BTC?foo=bar"]});
        assert!(validate_json_payload(&invalid).is_err());
    }

    #[test]
    fn test_sanitize_preserves_tabs() {
        assert_eq!(sanitize_string("hello\tworld", 10240), "hello\tworld");
    }

    #[test]
    fn test_sanitize_multiple_ansi_sequences() {
        let input = "test\x1b[1;31mred\x1b[0m\x1b[32mgreen\x1b[0m";
        let output = sanitize_string(input, 10240);
        assert_eq!(output, "testredgreen");
    }

    // ===== UTF-8 MULTI-BYTE TRUNCATION TESTS =====

    #[test]
    fn test_truncate_ascii_at_exact_boundary() {
        let input = "abcdefghij";
        let result = sanitize_string(input, 5);
        assert_eq!(result, "abcde[truncated]");
    }

    #[test]
    fn test_truncate_chinese_chars_safe_boundary() {
        // Chinese character '你' is 3 bytes in UTF-8
        let input = "你好世界"; // 4 characters, 12 bytes
        let result = sanitize_string(input, 2);
        assert_eq!(result, "你好[truncated]");
    }

    #[test]
    fn test_truncate_emoji_at_boundary() {
        // Emoji like '🚀' is 4 bytes in UTF-8
        let input = "hello🚀world"; // 11 chars (emoji counts as 1 char)
        let result = sanitize_string(input, 6);
        assert_eq!(result, "hello🚀[truncated]");
    }

    #[test]
    fn test_truncate_mixed_multibyte_safe() {
        // Mix of ASCII, Chinese (3-byte), emoji (4-byte)
        let input = "a你b🚀c"; // 5 chars total
        let result = sanitize_string(input, 3);
        assert_eq!(result, "a你b[truncated]");
    }

    #[test]
    fn test_truncate_empty_string() {
        let input = "";
        let result = sanitize_string(input, 5);
        assert_eq!(result, "");
    }

    #[test]
    fn test_truncate_string_shorter_than_limit() {
        let input = "short";
        let result = sanitize_string(input, 100);
        assert_eq!(result, "short");
    }

    #[test]
    fn test_truncate_single_char_limit() {
        let input = "你好";
        let result = sanitize_string(input, 1);
        assert_eq!(result, "你[truncated]");
    }

    #[test]
    fn test_truncate_zero_limit() {
        let input = "abc";
        let result = sanitize_string(input, 0);
        assert_eq!(result, "[truncated]");
    }
}
