use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum FormatHint {
    Scalar,
    TimestampMs,
    Decimal,
    Boolean,
}

#[derive(Debug, Clone)]
pub struct ResponseField {
    pub name: String,
    pub display_name: String,
    pub field_type: String,
    pub format: Option<String>,
    pub format_hint: FormatHint,
}

#[derive(Debug, Clone)]
pub struct ResponseSchema {
    pub data_path: String,
    pub fields: Vec<ResponseField>,
}

/// Navigate a JSON value by a dot-separated path.
/// Numeric segments index into arrays, string segments index into objects.
pub fn navigate<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = value;
    for segment in path.split('.') {
        if segment.is_empty() {
            continue;
        }
        if let Ok(index) = segment.parse::<usize>() {
            current = current.get(index)?;
        } else {
            current = current.get(segment)?;
        }
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
