//! Query filter builder for Supabase `PostgREST` queries.

/// The comparison operator to apply to a filter.
#[derive(Debug)]
pub(super) enum FilterKind {
    /// Exact equality (`eq`)
    Eq,
    /// Case-insensitive substring match (`ilike '%value%'`)
    Ilike,
    /// Prefix match (`like 'value%'`)
    StartsWith,
}

/// Builder for `PostgREST` query filters.
///
/// Collects optional filter conditions and applies them to a Supabase query.
/// Eliminates the boilerplate of managing intermediate `String` lifetimes when
/// passing `Option<T>` fields as equality conditions.
///
/// # Example
/// ```ignore
/// let filters = QueryFilters::new()
///     .eq("state", Some("MA"))
///     .eq("carnegie_class", Some(15))
///     .ilike("name", Some("northeastern"));
/// client.select(tables::INSTITUTIONS, "*", &filters, None).await?;
/// ```
#[derive(Debug, Default)]
pub struct QueryFilters {
    pub(super) entries: Vec<(FilterKind, &'static str, String)>,
}

impl QueryFilters {
    /// Create an empty filter set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add an equality filter. Skipped when `val` is `None`.
    #[must_use]
    pub fn eq<T: ToString>(mut self, col: &'static str, val: Option<T>) -> Self {
        if let Some(v) = val {
            self.entries.push((FilterKind::Eq, col, v.to_string()));
        }
        self
    }

    /// Add a case-insensitive substring filter. Skipped when `val` is `None`.
    ///
    /// The value is automatically wrapped with `%` wildcards for substring matching.
    #[must_use]
    pub fn ilike<T: ToString>(mut self, col: &'static str, val: Option<T>) -> Self {
        if let Some(v) = val {
            self.entries
                .push((FilterKind::Ilike, col, format!("%{}%", v.to_string())));
        }
        self
    }

    /// Add a prefix filter using `LIKE 'value%'`. Skipped when `val` is `None`.
    ///
    /// Useful for CIP code family filtering (e.g. `starts_with("cip_code", Some("11."))`).
    #[must_use]
    pub fn starts_with<T: ToString>(mut self, col: &'static str, val: Option<T>) -> Self {
        if let Some(v) = val {
            self.entries
                .push((FilterKind::StartsWith, col, format!("{}%", v.to_string())));
        }
        self
    }

    /// Returns `true` if no filters have been added.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_filters() {
        let f = QueryFilters::new();
        assert!(f.is_empty());
        assert!(f.entries.is_empty());
    }

    #[test]
    fn test_eq_some_adds_entry() {
        let f = QueryFilters::new().eq("state", Some("MA"));
        assert_eq!(f.entries.len(), 1);
        assert_eq!(f.entries[0].1, "state");
        assert_eq!(f.entries[0].2, "MA");
    }

    #[test]
    fn test_eq_none_skips() {
        let f = QueryFilters::new().eq::<&str>("state", None);
        assert!(f.is_empty());
    }

    #[test]
    fn test_ilike_wraps_wildcards() {
        let f = QueryFilters::new().ilike("name", Some("northeastern"));
        assert_eq!(f.entries[0].2, "%northeastern%");
    }

    #[test]
    fn test_starts_with_appends_wildcard() {
        let f = QueryFilters::new().starts_with("cip_code", Some("11."));
        assert_eq!(f.entries[0].2, "11.%");
    }

    #[test]
    fn test_chaining() {
        let f = QueryFilters::new()
            .eq("state", Some("MA"))
            .eq::<i32>("carnegie_class", None)
            .eq("control", Some(1));
        assert_eq!(f.entries.len(), 2);
    }
}
