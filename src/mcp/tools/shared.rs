//! Shared utilities for MCP tool implementations.

/// Serialize a JSON error response string from a display-able error value.
pub fn error_json(msg: impl std::fmt::Display) -> String {
    serde_json::json!({ "error": msg.to_string() }).to_string()
}

/// Deserialize a `serde_json::Value` JSON array into a typed `Vec<T>`.
///
/// Items that fail to deserialize are silently skipped.
#[must_use]
pub fn parse_json_array<T: serde::de::DeserializeOwned>(value: &serde_json::Value) -> Vec<T> {
    value
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|item| serde_json::from_value(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Serialize a value to a pretty-printed JSON string, falling back to an error JSON on failure.
pub fn to_json_pretty(value: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(value)
        .unwrap_or_else(|e| error_json(format!("Serialization failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[test]
    fn test_error_json_formats_message() {
        let out = error_json("something went wrong");
        assert_eq!(out, r#"{"error":"something went wrong"}"#);
    }

    #[test]
    fn test_parse_json_array_valid() {
        #[derive(Deserialize)]
        struct Item {
            id: i32,
        }
        let json = serde_json::json!([{"id": 1}, {"id": 2}]);
        let items: Vec<Item> = parse_json_array(&json);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, 1);
    }

    #[test]
    fn test_parse_json_array_skips_bad_entries() {
        #[derive(Deserialize)]
        struct Item {
            #[allow(dead_code)] // only used by serde, not read directly in test
            id: i32,
        }
        let json = serde_json::json!([{"id": 1}, {"name": "no id"}, {"id": 3}]);
        let items: Vec<Item> = parse_json_array(&json);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_parse_json_array_non_array_returns_empty() {
        let json = serde_json::json!({"not": "an array"});
        let items: Vec<serde_json::Value> = parse_json_array(&json);
        assert!(items.is_empty());
    }

    #[test]
    fn test_to_json_pretty_roundtrip() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Foo {
            x: i32,
        }
        let foo = Foo { x: 42 };
        let s = to_json_pretty(&foo);
        let back: Foo = serde_json::from_str(&s).unwrap();
        assert_eq!(back, foo);
    }
}
