use cdcx_core::openapi::types::navigate;
use serde_json::json;

#[test]
fn test_navigate_simple_key() {
    let v = json!({"data": [1, 2, 3]});
    assert_eq!(navigate(&v, "data"), Some(&json!([1, 2, 3])));
}

#[test]
fn test_navigate_nested_key() {
    let v = json!({"result": {"data": [1]}});
    assert_eq!(navigate(&v, "result.data"), Some(&json!([1])));
}

#[test]
fn test_navigate_array_index() {
    let v = json!({"data": [{"bids": [1]}, {"bids": [2]}]});
    assert_eq!(navigate(&v, "data.0"), Some(&json!({"bids": [1]})));
}

#[test]
fn test_navigate_missing_returns_none() {
    let v = json!({"data": []});
    assert!(navigate(&v, "missing").is_none());
    assert!(navigate(&v, "data.0").is_none());
}
