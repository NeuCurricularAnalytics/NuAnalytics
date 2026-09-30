//! Tool failures as protocol errors.
//!
//! Tools answer with a JSON string, and a failure is one with an `error` key. To the
//! protocol that is still an ordinary result, so a client could not tell a refused call
//! from an answer without reading the text. [`finish`] runs over every call's result: a
//! failure comes back with `isError: true` and one envelope,
//!
//! ```json
//! {"error": {"code": "db_unavailable", "message": "...", "next_steps": ["..."], "details": {...}}}
//! ```
//!
//! and everything else is left exactly as the tool wrote it. A degree that fails validation
//! is a finding, not a failure: validation reports it under `parse_error` / `errors`, never
//! `error`, so it stays an ordinary result.
//!
//! Done once here, over the text, rather than in each of the tools: the engines keep their
//! `{"error": ...}` payloads, which the CLI renders too.

use rmcp::model::{CallToolResult, Content};
use serde_json::{Map, Value};

/// Keys the envelope takes over from a tool's payload; the rest go under `details`.
const CONSUMED: [&str; 8] = [
    "error",
    "code",
    "kind",
    "next_steps",
    "tip",
    "hint",
    "suggestion",
    "success",
];

/// A tool's result, with a failure turned into a protocol error carrying the envelope.
#[must_use]
pub fn finish(mut result: CallToolResult) -> CallToolResult {
    if result.is_error == Some(true) || result.content.len() != 1 {
        return result;
    }
    let Some(envelope) = result.content[0]
        .as_text()
        .and_then(|text| envelope_for(&text.text))
    else {
        return result;
    };
    result.content = vec![Content::text(envelope)];
    result.is_error = Some(true);
    result
}

/// The envelope for `raw`, or `None` when `raw` is not a failure.
///
/// A failure is a JSON object whose `error` is present and not null — some tools always
/// emit the key, `null` on success.
fn envelope_for(raw: &str) -> Option<String> {
    let Value::Object(body) = serde_json::from_str::<Value>(raw).ok()? else {
        return None;
    };
    let error = body.get("error").filter(|e| !e.is_null())?;
    // Already an envelope: pass it through.
    if error.get("code").is_some() && error.get("message").is_some() {
        return Some(raw.to_string());
    }
    let message = error
        .as_str()
        .map_or_else(|| error.to_string(), str::to_string);
    let mut envelope = Map::new();
    envelope.insert("code".into(), Value::String(code_for(&body).to_string()));
    envelope.insert("message".into(), Value::String(message));
    let next_steps = next_steps_for(&body);
    if !next_steps.is_empty() {
        envelope.insert("next_steps".into(), Value::from(next_steps));
    }
    let mut details: Map<String, Value> = body
        .iter()
        .filter(|(k, v)| !CONSUMED.contains(&k.as_str()) && !is_empty(v))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    // A database refusal names its own SQLSTATE; the envelope's code stays the stable
    // `sql_backend`, and the SQLSTATE goes with the details.
    if let (Some(_), Some(sqlstate)) = (body.get("kind"), body.get("code")) {
        details.insert("sqlstate".into(), sqlstate.clone());
    }
    if !details.is_empty() {
        envelope.insert("details".into(), Value::Object(details));
    }
    serde_json::to_string_pretty(&serde_json::json!({ "error": envelope })).ok()
}

/// Whether a detail carries nothing: null, or an empty array or object. A failed call's
/// response struct still serialises its unused fields (`tool_followups: []`), which would
/// only bury the ones that matter.
fn is_empty(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// A machine-readable code, from what the payload already says about itself.
fn code_for(body: &Map<String, Value>) -> &str {
    // `SqlError` and `DatabaseError` name their own kind; a SQL refusal's `code` beside
    // it is the SQLSTATE, which goes in the details instead.
    if let Some(kind) = body.get("kind").and_then(Value::as_str) {
        return kind;
    }
    body.get("code")
        .and_then(Value::as_str)
        .unwrap_or("tool_error")
}

/// The remediation a payload carries, in whatever key it used.
fn next_steps_for(body: &Map<String, Value>) -> Vec<String> {
    if let Some(steps) = body.get("next_steps").and_then(Value::as_array) {
        return steps
            .iter()
            .filter_map(|s| s.as_str().map(str::to_string))
            .collect();
    }
    ["tip", "hint", "suggestion"]
        .iter()
        .filter_map(|k| body.get(*k).and_then(Value::as_str).map(str::to_string))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(raw: &Value) -> Value {
        serde_json::from_str(&envelope_for(&raw.to_string()).expect("a failure")).expect("json")
    }

    #[test]
    fn an_answer_or_a_finding_is_not_a_failure() {
        for raw in [
            serde_json::json!({"success": true, "error": null, "plans": 3}),
            serde_json::json!({"is_valid": false, "parse_error": "bad indent", "errors": []}),
            serde_json::json!([1, 2]),
        ] {
            assert_eq!(envelope_for(&raw.to_string()), None, "{raw}");
        }
        assert_eq!(envelope_for("not json"), None);
    }

    #[test]
    fn a_legacy_error_becomes_an_envelope_keeping_its_details_and_advice() {
        let e = envelope(&serde_json::json!({
            "error": "No institutions matched the given filters",
            "suggestion": "Try broadening carnegie_class",
            "success": false,
            "unitid": 1,
            "tool_followups": [],
            "plans": null,
        }));
        assert_eq!(e["error"]["code"], "tool_error");
        assert_eq!(
            e["error"]["message"],
            "No institutions matched the given filters"
        );
        assert_eq!(
            e["error"]["next_steps"],
            serde_json::json!(["Try broadening carnegie_class"])
        );
        assert_eq!(e["error"]["details"], serde_json::json!({"unitid": 1}));
    }

    #[test]
    fn the_code_comes_from_what_the_payload_says_about_itself() {
        let db = envelope(&serde_json::json!({
            "error": "Database not available: timed out", "code": "db_unavailable",
            "reason": "unreachable", "next_steps": ["check the endpoint"], "tool": "search_degrees",
        }));
        assert_eq!(db["error"]["code"], "db_unavailable");
        assert_eq!(
            db["error"]["next_steps"],
            serde_json::json!(["check the endpoint"])
        );
        assert_eq!(
            db["error"]["details"]["reason"], "unreachable",
            "the failure class survives the envelope"
        );
        let query =
            envelope(&serde_json::json!({"error": "reading x: timed out", "kind": "unreachable"}));
        assert_eq!(
            query["error"]["code"], "unreachable",
            "a DatabaseError's kind is the code"
        );
        let sql = envelope(
            &serde_json::json!({"error": "no such column", "kind": "sql_backend", "code": "42703"}),
        );
        assert_eq!(
            sql["error"]["code"], "sql_backend",
            "a stable code, not the SQLSTATE"
        );
        assert_eq!(sql["error"]["details"]["sqlstate"], "42703");
        let coded =
            envelope(&serde_json::json!({"error": "no such sample", "code": "source_not_found"}));
        assert_eq!(
            coded["error"]["code"], "source_not_found",
            "an explicit code wins"
        );
        let kind = envelope(&serde_json::json!({"error": "refused", "kind": "sql_rejected"}));
        assert_eq!(kind["error"]["code"], "sql_rejected");
        let uncoded = envelope(&serde_json::json!({"error": "matches 2", "matches": ["a", "b"]}));
        assert_eq!(
            uncoded["error"]["code"], "tool_error",
            "no code is guessed from the payload's shape"
        );
    }

    #[test]
    fn an_existing_envelope_or_an_error_result_passes_through_unchanged() {
        let already = r#"{"error":{"code":"cache_expired","message":"expired"}}"#;
        assert_eq!(envelope_for(already).as_deref(), Some(already));

        let mut flagged = CallToolResult::success(vec![Content::text(r#"{"error":"x"}"#)]);
        flagged.is_error = Some(true);
        assert_eq!(
            finish(flagged).content[0].as_text().expect("text").text,
            r#"{"error":"x"}"#
        );

        let two = finish(CallToolResult::success(vec![
            Content::text(r#"{"error":"x"}"#),
            Content::text("more"),
        ]));
        assert_ne!(
            two.is_error,
            Some(true),
            "only a single-text result is read"
        );

        let null_error = finish(CallToolResult::success(vec![Content::text(
            r#"{"error":null,"ok":1}"#,
        )]));
        assert_ne!(
            null_error.is_error,
            Some(true),
            "an error key of null is success"
        );
    }

    #[test]
    fn a_structured_error_becomes_its_json_text_and_every_advice_key_is_kept() {
        let e =
            envelope(&serde_json::json!({"error": {"detail": "bad"}, "tip": "one", "hint": "two"}));
        assert_eq!(e["error"]["message"], r#"{"detail":"bad"}"#);
        assert_eq!(e["error"]["next_steps"], serde_json::json!(["one", "two"]));
        assert!(e["error"].get("details").is_none());
    }

    #[test]
    fn finish_marks_the_result_an_error_only_for_a_failure() {
        let failed = finish(CallToolResult::success(vec![Content::text(
            r#"{"error": "Must provide exactly one of: degree, content, or path", "code": "bad_arguments"}"#,
        )]));
        assert_eq!(failed.is_error, Some(true));
        let text = &failed.content[0].as_text().expect("text").text;
        assert!(text.contains("\"bad_arguments\""), "{text}");

        let ok = finish(CallToolResult::success(vec![Content::text(
            r#"{"is_valid": true}"#,
        )]));
        assert_ne!(ok.is_error, Some(true));
    }
}
