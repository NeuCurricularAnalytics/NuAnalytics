//! `cache_yaml` MCP tool — stash an inline YAML body in the server's
//! in-memory cache and get back a handle that any other tool accepts as a
//! `degree_id`.
//!
//! Motivation: hosted MCP clients (Claude Code, etc.) run in a sandbox whose
//! filesystem the server can't see, so `yaml_path` doesn't help them and
//! every validate / audit / analyze call re-pipes the same ~40 KB YAML
//! through the model's context. This tool lets the caller register the body
//! once and then refer to it via a short `cache:{hex}` handle on every
//! subsequent call.

use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::mcp::cache::{YAML_CACHE, YAML_CACHE_TTL};

// ============================================================================
// Request / Response
// ============================================================================

/// Request parameters for `cache_yaml`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CacheYamlRequest {
    /// Full YAML body to cache. Anything that would be valid input to
    /// `validate_degree(yaml_content=...)` is valid here.
    #[schemars(description = "Complete degree YAML content to cache.")]
    pub yaml_content: String,
}

/// Response for `cache_yaml`.
#[derive(Debug, Serialize)]
pub struct CacheYamlResponse {
    /// True when the body was stored. Currently always true on success
    /// because the cache has no rejection criteria — kept for shape
    /// consistency with the other tools.
    pub success: bool,
    /// Cache handle. Pass this as `degree_id` on any subsequent tool call.
    pub handle: String,
    /// Size of the cached body in bytes — useful for the caller to plan
    /// context budgets.
    pub bytes: usize,
    /// Number of entries currently in the cache after this insertion.
    pub cache_entries: usize,
    /// TTL in seconds — how long this handle stays valid from insertion.
    /// Surfaced explicitly so callers can plan re-caching before expiry.
    pub ttl_seconds: u64,
    /// Human-readable hint about how to use the handle.
    pub note: &'static str,
}

// ============================================================================
// Execution
// ============================================================================

/// Insert `yaml_content` into the in-memory YAML cache and return the
/// content-hashed handle.
///
/// # Panics
/// Panics if the process-wide `YAML_CACHE` mutex is poisoned. This indicates
/// a thread previously panicked while holding the cache lock and the cache
/// state is no longer trustworthy — there is no useful recovery from this
/// at the tool layer.
#[must_use]
pub fn execute(yaml_content: String) -> CacheYamlResponse {
    let bytes = yaml_content.len();
    let (handle, cache_entries) = {
        let mut cache = YAML_CACHE.lock().expect("yaml cache mutex poisoned");
        let handle = cache.insert(yaml_content);
        (handle, cache.len())
    };
    CacheYamlResponse {
        success: true,
        handle,
        bytes,
        cache_entries,
        ttl_seconds: YAML_CACHE_TTL.as_secs(),
        note: "Pass `handle` as `degree_id` to validate_degree / audit_degree / analyze_degree / generate_degree_report / get_course_detail / render_plan_graph / find_courses_matching / degree_pipeline. Subsequent responses surface cache_ttl_remaining_seconds so you can re-cache before expiry.",
    }
}

/// Execute and serialize as JSON.
#[must_use]
pub fn execute_json(yaml_content: String) -> String {
    let response = execute(yaml_content);
    serde_json::to_string_pretty(&response)
        .unwrap_or_else(|e| format!("{{\"error\": \"Failed to serialize response: {e}\"}}"))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_execute_returns_handle_with_expected_prefix_and_bytes() {
        let body = "degree:\n  id: test-cache-yaml\n  total_credits: 8\n".to_string();
        let expected_bytes = body.len();
        let response = execute(body);
        assert!(response.success);
        assert!(response.handle.starts_with("cache:"));
        assert_eq!(response.bytes, expected_bytes);
        assert!(response.cache_entries >= 1);
    }

    #[test]
    fn test_handle_is_idempotent_for_same_body() {
        let body = "degree:\n  id: t\n".to_string();
        let r1 = execute(body.clone());
        let r2 = execute(body);
        assert_eq!(
            r1.handle, r2.handle,
            "same body must produce the same handle on repeat insert"
        );
    }

    #[test]
    fn test_handle_matches_yamlcache_handle_for() {
        // The handle format is the implementation choice in YamlCache — keep
        // the tool's output aligned so callers can predict the handle from
        // the body without round-tripping through the tool.
        use crate::mcp::cache::YamlCache;
        let body = "degree:\n  id: predictable\n".to_string();
        let expected = YamlCache::handle_for(&body);
        let response = execute(body);
        assert_eq!(response.handle, expected);
    }
}
