//! JSON and serde helpers shared by the query engines, the MCP tools and the CLI.
//!
//! Deliberately ungated and dependency-light: `serde`, `serde_json` and `std` only.
//! `core::query` builds under `database` alone and the CLI reaches these without the
//! `mcp` feature, so adding a `core::database` import here would break
//! `--no-default-features`.

use serde::{de, Deserialize, Deserializer};

/// Machine-readable codes a failure payload carries in `code`.
///
/// The MCP envelope passes the code through, so a client can branch on it; the CLI prints
/// the message. One list, so a producer and the envelope cannot disagree on a spelling.
pub mod error_code {
    /// The arguments were invalid: missing, contradictory, or of the wrong shape.
    pub const BAD_ARGUMENTS: &str = "bad_arguments";
    /// The degree, file or record the arguments name does not exist.
    pub const SOURCE_NOT_FOUND: &str = "source_not_found";
    /// A reference matched several records; the payload lists them.
    pub const AMBIGUOUS_REFERENCE: &str = "ambiguous_reference";
    /// A `cache:` handle has expired.
    pub const CACHE_EXPIRED: &str = "cache_expired";
    /// The database the tool needs is not available.
    pub const DB_UNAVAILABLE: &str = "db_unavailable";
    /// An output file could not be written, for a reason other than already existing.
    pub const WRITE_FAILED: &str = "write_failed";
}

/// A failure payload carrying a machine-readable `code` (see [`error_code`]).
#[must_use]
pub fn coded_error(code: &str, message: impl std::fmt::Display) -> serde_json::Value {
    serde_json::json!({ "error": message.to_string(), "code": code })
}

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

/// A JSON value's type as a noun phrase — "an object", "null" — for error messages.
#[must_use]
pub const fn value_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

/// Coerce a JSON value into `Option<T>` accepting native, stringified, or null.
fn coerce_opt<T>(value: serde_json::Value) -> Result<Option<T>, String>
where
    T: serde::de::DeserializeOwned + std::str::FromStr,
    <T as std::str::FromStr>::Err: std::fmt::Display,
{
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(s) if s.is_empty() => Ok(None),
        serde_json::Value::String(s) => s.parse::<T>().map(Some).map_err(|e| e.to_string()),
        other => serde_json::from_value::<T>(other)
            .map(Some)
            .map_err(|e| e.to_string()),
    }
}

/// Deserialize `Option<i32>` accepting `42`, `"42"`, `null`, or `""`.
///
/// # Errors
/// Returns the underlying deserializer error if the input is neither a
/// valid native integer nor a parseable numeric string.
pub fn deserialize_opt_i32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i32>, D::Error> {
    coerce_opt::<i32>(serde_json::Value::deserialize(d)?).map_err(de::Error::custom)
}

/// Deserialize `Option<i64>` accepting `42`, `"42"`, `null`, or `""`.
///
/// # Errors
/// Returns the underlying deserializer error if the input is neither a
/// valid native integer nor a parseable numeric string.
pub fn deserialize_opt_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    coerce_opt::<i64>(serde_json::Value::deserialize(d)?).map_err(de::Error::custom)
}

/// Deserialize `Option<usize>` accepting `42`, `"42"`, `null`, or `""`.
///
/// # Errors
/// Returns the underlying deserializer error if the input is negative,
/// not an integer, or otherwise unparseable.
pub fn deserialize_opt_usize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<usize>, D::Error> {
    coerce_opt::<usize>(serde_json::Value::deserialize(d)?).map_err(de::Error::custom)
}

/// Deserialize `Option<u64>` accepting `42`, `"42"`, `null`, or `""`.
///
/// # Errors
/// Returns the underlying deserializer error if the input is negative,
/// not an integer, or otherwise unparseable.
pub fn deserialize_opt_u64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    coerce_opt::<u64>(serde_json::Value::deserialize(d)?).map_err(de::Error::custom)
}

/// Deserialize `Option<f32>` accepting `1.5`, `"1.5"`, `null`, or `""`.
///
/// # Errors
/// Returns the underlying deserializer error if the input is neither a
/// valid native float nor a parseable numeric string.
pub fn deserialize_opt_f32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f32>, D::Error> {
    coerce_opt::<f32>(serde_json::Value::deserialize(d)?).map_err(de::Error::custom)
}

/// Deserialize `Option<f64>` accepting `1.5`, `"1.5"`, `null`, or `""`.
///
/// # Errors
/// Returns the underlying deserializer error if the input is neither a
/// valid native float nor a parseable numeric string.
pub fn deserialize_opt_f64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    coerce_opt::<f64>(serde_json::Value::deserialize(d)?).map_err(de::Error::custom)
}

/// Deserialize `Option<bool>` accepting `true`/`false`, `"true"`/`"false"`, `"1"`/`"0"`, or null.
///
/// # Errors
/// Returns the underlying deserializer error if the input is not a boolean,
/// a recognized boolean string (`true`/`false`/`yes`/`no`/`1`/`0`), or `0`/`1`
/// as a JSON number.
pub fn deserialize_opt_bool<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Bool(b) => Ok(Some(b)),
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            match trimmed.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" => Ok(Some(true)),
                "false" | "0" | "no" => Ok(Some(false)),
                other => Err(de::Error::custom(format!(
                    "expected boolean (true/false/1/0/yes/no), got {other:?}"
                ))),
            }
        }
        // Accept numeric 1/0 as well (some clients send JSON numbers for booleans)
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(0) => Ok(Some(false)),
            Some(1) => Ok(Some(true)),
            _ => Err(de::Error::custom(format!(
                "expected boolean number 0 or 1, got {n}"
            ))),
        },
        other => Err(de::Error::custom(format!(
            "expected boolean, got {other:?}"
        ))),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[test]
    fn test_coded_error_carries_code_and_message() {
        let v = coded_error(error_code::SOURCE_NOT_FOUND, format_args!("no `{}`", "x"));
        assert_eq!(v["code"], "source_not_found");
        assert_eq!(v["error"], "no `x`");
        assert_eq!(v.as_object().map(serde_json::Map::len), Some(2));
    }

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

    #[test]
    fn test_deserialize_opt_i32_accepts_all_input_shapes() {
        #[derive(Deserialize)]
        struct R {
            #[serde(default, deserialize_with = "deserialize_opt_i32")]
            x: Option<i32>,
        }
        let parse = |v| serde_json::from_value::<R>(v).map(|r| r.x);
        assert_eq!(parse(serde_json::json!({"x": 42})).unwrap(), Some(42));
        assert_eq!(parse(serde_json::json!({"x": "42"})).unwrap(), Some(42));
        assert_eq!(parse(serde_json::json!({"x": -7})).unwrap(), Some(-7));
        assert_eq!(parse(serde_json::json!({"x": null})).unwrap(), None);
        assert_eq!(parse(serde_json::json!({"x": ""})).unwrap(), None);
        assert_eq!(parse(serde_json::json!({})).unwrap(), None);
        assert!(parse(serde_json::json!({"x": "not a number"})).is_err());
    }

    #[test]
    fn test_deserialize_opt_i64_accepts_all_input_shapes() {
        #[derive(Deserialize)]
        struct R {
            #[serde(default, deserialize_with = "deserialize_opt_i64")]
            x: Option<i64>,
        }
        let parse = |v| serde_json::from_value::<R>(v).map(|r| r.x);
        assert_eq!(
            parse(serde_json::json!({"x": 1_000_000_i64})).unwrap(),
            Some(1_000_000)
        );
        assert_eq!(
            parse(serde_json::json!({"x": "1000000"})).unwrap(),
            Some(1_000_000)
        );
        assert_eq!(parse(serde_json::json!({"x": null})).unwrap(), None);
        assert_eq!(parse(serde_json::json!({})).unwrap(), None);
        assert!(parse(serde_json::json!({"x": "abc"})).is_err());
    }

    #[test]
    fn test_deserialize_opt_usize_accepts_all_input_shapes() {
        #[derive(Deserialize)]
        struct R {
            #[serde(default, deserialize_with = "deserialize_opt_usize")]
            x: Option<usize>,
        }
        let parse = |v| serde_json::from_value::<R>(v).map(|r| r.x);
        assert_eq!(parse(serde_json::json!({"x": 25})).unwrap(), Some(25));
        assert_eq!(parse(serde_json::json!({"x": "25"})).unwrap(), Some(25));
        assert_eq!(parse(serde_json::json!({"x": null})).unwrap(), None);
        assert_eq!(parse(serde_json::json!({})).unwrap(), None);
        // Negatives can't fit in usize via either path
        assert!(parse(serde_json::json!({"x": -1})).is_err());
        assert!(parse(serde_json::json!({"x": "-1"})).is_err());
    }

    #[test]
    fn test_deserialize_opt_f32_accepts_all_input_shapes() {
        #[derive(Deserialize)]
        struct R {
            #[serde(default, deserialize_with = "deserialize_opt_f32")]
            x: Option<f32>,
        }
        let parse = |v| serde_json::from_value::<R>(v).map(|r| r.x);
        assert_eq!(parse(serde_json::json!({"x": 1.5})).unwrap(), Some(1.5));
        assert_eq!(parse(serde_json::json!({"x": "1.5"})).unwrap(), Some(1.5));
        assert_eq!(parse(serde_json::json!({"x": null})).unwrap(), None);
        assert!(parse(serde_json::json!({"x": "nope"})).is_err());
    }

    #[test]
    fn test_deserialize_opt_f64_accepts_all_input_shapes() {
        #[derive(Deserialize)]
        struct R {
            #[serde(default, deserialize_with = "deserialize_opt_f64")]
            x: Option<f64>,
        }
        let parse = |v| serde_json::from_value::<R>(v).map(|r| r.x);
        assert_eq!(parse(serde_json::json!({"x": 2.5})).unwrap(), Some(2.5));
        assert_eq!(parse(serde_json::json!({"x": "2.5"})).unwrap(), Some(2.5));
        assert_eq!(parse(serde_json::json!({"x": null})).unwrap(), None);
    }

    #[test]
    fn test_deserialize_opt_bool_accepts_all_input_shapes() {
        #[derive(Deserialize)]
        struct R {
            #[serde(default, deserialize_with = "deserialize_opt_bool")]
            x: Option<bool>,
        }
        let parse = |v| serde_json::from_value::<R>(v).map(|r| r.x);
        // Native booleans
        assert_eq!(parse(serde_json::json!({"x": true})).unwrap(), Some(true));
        assert_eq!(parse(serde_json::json!({"x": false})).unwrap(), Some(false));
        // String forms (case-insensitive)
        assert_eq!(parse(serde_json::json!({"x": "true"})).unwrap(), Some(true));
        assert_eq!(
            parse(serde_json::json!({"x": "FALSE"})).unwrap(),
            Some(false)
        );
        assert_eq!(parse(serde_json::json!({"x": "yes"})).unwrap(), Some(true));
        assert_eq!(parse(serde_json::json!({"x": "no"})).unwrap(), Some(false));
        // Numeric forms (string and native)
        assert_eq!(parse(serde_json::json!({"x": "1"})).unwrap(), Some(true));
        assert_eq!(parse(serde_json::json!({"x": "0"})).unwrap(), Some(false));
        assert_eq!(parse(serde_json::json!({"x": 1})).unwrap(), Some(true));
        assert_eq!(parse(serde_json::json!({"x": 0})).unwrap(), Some(false));
        // Empty / null / missing → None
        assert_eq!(parse(serde_json::json!({"x": null})).unwrap(), None);
        assert_eq!(parse(serde_json::json!({"x": ""})).unwrap(), None);
        assert_eq!(parse(serde_json::json!({})).unwrap(), None);
        // Bad inputs
        assert!(parse(serde_json::json!({"x": "maybe"})).is_err());
        assert!(parse(serde_json::json!({"x": 2})).is_err());
    }
}
