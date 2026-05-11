//! In-memory caches shared by the MCP tool layer.
//!
//! Two process-wide singletons:
//!
//! - [`YAML_CACHE`] — content-hashed inline YAML storage. The caller hands a
//!   YAML body to `cache_yaml` once; subsequent tools accept the returned
//!   `cache:{hex}` handle anywhere a `degree_id` is accepted. Removes the
//!   per-call repaste tax for hosted MCP clients whose filesystem isn't
//!   reachable by the server (`yaml_path` returns ENOENT).
//!
//! - [`ARTIFACT_CACHE`] — small LRU of [`AnalysisArtifacts`] keyed by the
//!   (yaml-hash, `max_plans`, `include_courses`) tuple. Three sequential
//!   `render_plan_graph` calls on the same YAML now run the plan-generation
//!   pipeline once instead of three times.
//!
//! Both caches live as `LazyLock<Mutex<…>>` statics because the analyze
//! pipeline reaches them from five tool modules (`analyze`, `audit`,
//! `report`, `plan_graph`, `course_detail`). Threading the state through
//! every call site would be invasive for what is effectively a process
//! singleton.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use crate::mcp::tools::analyze::AnalysisArtifacts;

/// Prefix that marks an in-memory YAML-cache handle.
///
/// `degree_id` resolution in `run_yaml_tool` matches against this prefix
/// before falling through to the sample registry or the DB lookup.
pub const YAML_CACHE_PREFIX: &str = "cache:";

const YAML_CACHE_TTL: Duration = Duration::from_secs(60 * 60);
const ARTIFACT_CACHE_CAPACITY: usize = 4;

// ============================================================================
// YAML cache
// ============================================================================

/// One cached YAML body with its insertion timestamp.
struct CachedYaml {
    body: Arc<str>,
    inserted_at: Instant,
}

/// Content-hashed YAML cache. Keyed by `cache:{16-hex}` handles.
#[derive(Default)]
pub struct YamlCache {
    entries: HashMap<String, CachedYaml>,
}

impl YamlCache {
    /// Compute the canonical handle for `body`. Deterministic — the same
    /// body always hashes to the same handle, so re-caching is idempotent.
    #[must_use]
    pub fn handle_for(body: &str) -> String {
        let mut hasher = DefaultHasher::new();
        body.hash(&mut hasher);
        format!("{YAML_CACHE_PREFIX}{:016x}", hasher.finish())
    }

    /// Insert a YAML body, returning its handle. Sweeps expired entries as
    /// a side-effect so the cache doesn't grow unboundedly under heavy use.
    pub fn insert(&mut self, body: String) -> String {
        self.sweep_expired();
        let handle = Self::handle_for(&body);
        self.entries.insert(
            handle.clone(),
            CachedYaml {
                body: Arc::from(body),
                inserted_at: Instant::now(),
            },
        );
        handle
    }

    /// Fetch by handle. Returns `None` if missing or expired.
    #[must_use]
    pub fn get(&self, handle: &str) -> Option<Arc<str>> {
        let entry = self.entries.get(handle)?;
        if entry.inserted_at.elapsed() > YAML_CACHE_TTL {
            return None;
        }
        Some(entry.body.clone())
    }

    /// Current entry count. Exposed for the `cache_yaml` response so callers
    /// can see the cache state alongside the freshly-issued handle.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache has no entries. Pairs with [`Self::len`].
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn sweep_expired(&mut self) {
        let now = Instant::now();
        self.entries
            .retain(|_, e| now.duration_since(e.inserted_at) < YAML_CACHE_TTL);
    }
}

/// Process-wide YAML cache.
pub static YAML_CACHE: LazyLock<Mutex<YamlCache>> =
    LazyLock::new(|| Mutex::new(YamlCache::default()));

// ============================================================================
// Artifact cache
// ============================================================================

/// Composite key — the inputs that uniquely determine an [`AnalysisArtifacts`].
///
/// `include_courses` is canonicalised (sorted) before hashing so different
/// orderings of the same set hit the same cache entry.
type ArtifactKey = u64;

fn make_artifact_key(
    yaml: &str,
    max_plans: Option<usize>,
    include_courses: Option<&[String]>,
) -> ArtifactKey {
    let mut hasher = DefaultHasher::new();
    yaml.hash(&mut hasher);
    max_plans.hash(&mut hasher);
    if let Some(courses) = include_courses {
        // Sort + hash each entry individually. clippy's `collection_is_never_read`
        // would fire on a "build the vec, hash the vec" pattern because it can't
        // see through sort_unstable to the subsequent read.
        let mut sorted: Vec<&str> = courses.iter().map(String::as_str).collect();
        sorted.sort_unstable();
        for entry in &sorted {
            entry.hash(&mut hasher);
        }
    } else {
        // Distinguish None from Some(vec![]) — both legitimate "no constraint" forms.
        b"__none_include_courses__".hash(&mut hasher);
    }
    hasher.finish()
}

struct ArtifactEntry {
    key: ArtifactKey,
    value: Arc<AnalysisArtifacts>,
    last_accessed: Instant,
}

/// Small LRU of analysis artifacts.
///
/// Bounded at [`ARTIFACT_CACHE_CAPACITY`] to keep memory usage predictable —
/// each entry retains the full `MetricsAggregator`, `School`, `DAG`, and
/// `SelectedPlans` for the run.
#[derive(Default)]
pub struct ArtifactCache {
    entries: Vec<ArtifactEntry>,
}

impl ArtifactCache {
    /// Look up by composite key. Updates the entry's last-accessed timestamp
    /// on hit so the eviction order tracks recency.
    fn get(&mut self, key: ArtifactKey) -> Option<Arc<AnalysisArtifacts>> {
        let idx = self.entries.iter().position(|e| e.key == key)?;
        self.entries[idx].last_accessed = Instant::now();
        Some(self.entries[idx].value.clone())
    }

    /// Insert. Evicts the oldest entry when at capacity.
    fn insert(&mut self, key: ArtifactKey, value: Arc<AnalysisArtifacts>) {
        if self.entries.len() >= ARTIFACT_CACHE_CAPACITY {
            if let Some(oldest_idx) = self
                .entries
                .iter()
                .enumerate()
                .min_by_key(|(_, e)| e.last_accessed)
                .map(|(i, _)| i)
            {
                self.entries.remove(oldest_idx);
            }
        }
        self.entries.push(ArtifactEntry {
            key,
            value,
            last_accessed: Instant::now(),
        });
    }

    /// Current entry count. Exposed for diagnostics.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache has no entries. Pairs with [`Self::len`].
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Process-wide artifact cache.
pub static ARTIFACT_CACHE: LazyLock<Mutex<ArtifactCache>> =
    LazyLock::new(|| Mutex::new(ArtifactCache::default()));

/// Fetch a cached `AnalysisArtifacts` for the given inputs, building +
/// inserting on miss. Returns the same `Arc` on subsequent calls so
/// `render_plan_graph` + `analyze_degree` + `course_detail` on the same
/// YAML share one expensive pipeline run.
///
/// # Errors
/// Forwards the parse-error string from [`crate::mcp::tools::analyze::build_artifacts`].
pub(crate) fn cached_artifacts(
    yaml: &str,
    max_plans: Option<usize>,
    include_courses: Option<&[String]>,
) -> Result<Arc<AnalysisArtifacts>, String> {
    let key = make_artifact_key(yaml, max_plans, include_courses);

    // Cache-hit path: bind the lock guard to a local so it drops before we
    // return rather than living for the full `if let` (clippy's
    // `significant_drop_in_scrutinee`).
    let hit = ARTIFACT_CACHE
        .lock()
        .expect("artifact cache mutex poisoned")
        .get(key);
    if let Some(arc) = hit {
        return Ok(arc);
    }

    let owned_includes = include_courses.map(<[String]>::to_vec);
    let artifacts = crate::mcp::tools::analyze::build_artifacts(yaml, max_plans, owned_includes)?;
    let arc = Arc::new(artifacts);
    ARTIFACT_CACHE
        .lock()
        .expect("artifact cache mutex poisoned")
        .insert(key, arc.clone());
    Ok(arc)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yaml_cache_handle_is_content_hashed_and_idempotent() {
        let body = "degree:\n  id: x\n".to_string();
        let h1 = YamlCache::handle_for(&body);
        let h2 = YamlCache::handle_for(&body);
        assert_eq!(h1, h2, "same body must hash to the same handle");
        assert!(h1.starts_with(YAML_CACHE_PREFIX));

        let different = YamlCache::handle_for("degree:\n  id: y\n");
        assert_ne!(h1, different, "different bodies must hash differently");
    }

    #[test]
    fn test_yaml_cache_round_trips_through_insert_get() {
        let mut cache = YamlCache::default();
        let body = "degree:\n  id: round-trip\n".to_string();
        let handle = cache.insert(body.clone());
        let retrieved = cache.get(&handle).expect("present after insert");
        assert_eq!(&*retrieved, body);
    }

    #[test]
    fn test_yaml_cache_unknown_handle_returns_none() {
        let cache = YamlCache::default();
        assert!(cache.get("cache:deadbeefdeadbeef").is_none());
    }

    #[test]
    fn test_make_artifact_key_order_independent_for_include_courses() {
        // Same set of courses in different orders → same cache key.
        let a = make_artifact_key("yaml", Some(10), Some(&["CS101".into(), "CS201".into()]));
        let b = make_artifact_key("yaml", Some(10), Some(&["CS201".into(), "CS101".into()]));
        assert_eq!(a, b);
    }

    #[test]
    fn test_cached_artifacts_returns_same_arc_on_repeated_lookup() {
        // P0 artifact cache: two sequential cached_artifacts calls with the
        // same inputs must return the same Arc, so downstream tools share
        // work instead of running build_artifacts twice. Use the embedded
        // CSU sample (cheap to parse, deterministic across runs).
        let yaml = crate::mcp::tools::samples::yaml_for_key("csu")
            .expect("csu sample key must resolve to embedded YAML");
        let first = cached_artifacts(yaml, Some(50), None).expect("first build");
        let second = cached_artifacts(yaml, Some(50), None).expect("cache hit");
        assert!(
            Arc::ptr_eq(&first, &second),
            "second call must hit the cache and return the same Arc"
        );
    }

    #[test]
    fn test_make_artifact_key_distinguishes_none_from_empty_includes() {
        // None and Some(vec![]) are both "no constraint" semantically, but
        // the cache key separates them so swapping forms doesn't accidentally
        // collide (build_artifacts treats them identically; we err on safety).
        let none = make_artifact_key("yaml", Some(10), None);
        let empty = make_artifact_key("yaml", Some(10), Some(&[]));
        assert_ne!(none, empty);
    }
}
