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

/// Deserialize the first element of a JSON array into `T`.
///
/// Returns `None` when the value is not an array, the array is empty,
/// or the first element fails to deserialize. The helper does *not*
/// scan ahead to find a successfully-deserializing item — `parse_first`
/// means the first, not the first that happens to deserialize.
#[must_use]
pub fn parse_first<T: serde::de::DeserializeOwned>(value: &serde_json::Value) -> Option<T> {
    value
        .as_array()?
        .first()
        .and_then(|item| serde_json::from_value(item.clone()).ok())
}

/// Parse a comma-separated list of trimmed, non-empty strings.
///
/// `"11.0101, 11.0701, "` → `["11.0101", "11.0701"]`
#[must_use]
pub fn parse_comma_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(String::from)
        .collect()
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
    fn test_parse_first_returns_first_element() {
        #[derive(Deserialize)]
        struct Item {
            id: i32,
        }
        let json = serde_json::json!([{"id": 1}, {"id": 2}]);
        let item: Option<Item> = parse_first(&json);
        assert_eq!(item.unwrap().id, 1);
    }

    #[test]
    fn test_parse_first_empty_array_returns_none() {
        let json = serde_json::json!([]);
        let item: Option<serde_json::Value> = parse_first(&json);
        assert!(item.is_none());
    }

    #[test]
    fn test_parse_first_non_array_returns_none() {
        let json = serde_json::json!({"not": "an array"});
        let item: Option<serde_json::Value> = parse_first(&json);
        assert!(item.is_none());
    }

    #[test]
    fn test_parse_first_failed_deserialize_returns_none() {
        #[derive(Deserialize)]
        struct Item {
            #[allow(dead_code)] // only used by serde, not read directly in test
            id: i32,
        }
        // First element is missing the required `id` field; the helper must
        // return None rather than scanning ahead to the second element.
        let json = serde_json::json!([{"name": "no id"}, {"id": 1}]);
        let item: Option<Item> = parse_first(&json);
        assert!(
            item.is_none(),
            "parse_first must not skip ahead past a failed first element"
        );
    }

    #[test]
    fn test_parse_comma_list_basic() {
        assert_eq!(
            parse_comma_list("11.0101,11.0701"),
            vec!["11.0101", "11.0701"]
        );
    }

    #[test]
    fn test_parse_comma_list_empty_string() {
        assert!(parse_comma_list("").is_empty());
    }

    #[test]
    fn test_parse_comma_list_trims_whitespace() {
        assert_eq!(
            parse_comma_list("  11.0101  ,  11.0701  "),
            vec!["11.0101", "11.0701"]
        );
    }

    #[test]
    fn test_parse_comma_list_filters_empty_entries() {
        // Leading/trailing/consecutive commas produce no empty strings
        assert_eq!(
            parse_comma_list(",11.0101,,11.0701,"),
            vec!["11.0101", "11.0701"]
        );
    }

    #[test]
    fn test_parse_comma_list_only_commas_returns_empty() {
        assert!(parse_comma_list(",,,").is_empty());
    }

    #[test]
    fn test_parse_comma_list_single_entry() {
        assert_eq!(parse_comma_list("11.0101"), vec!["11.0101"]);
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
