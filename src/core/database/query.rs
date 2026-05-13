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
    /// Greater than or equal (`gte`)
    Gte,
    /// Less than or equal (`lte`)
    Lte,
    /// In list (`in.(v1,v2,v3)`) — values stored comma-separated in the String slot.
    /// Values must not contain commas (safe for integers and CIP codes).
    In,
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
    /// Uses `*` as the wildcard (`PostgREST` translates it to SQL `%`). Avoids using
    /// `%` directly in the URL, which gets double-encoded by `reqwest`'s URL parser
    /// (`%` → `%25`) and then never decoded back by `PostgREST`, causing no matches.
    #[must_use]
    pub fn ilike<T: ToString>(mut self, col: &'static str, val: Option<T>) -> Self {
        if let Some(v) = val {
            self.entries
                .push((FilterKind::Ilike, col, format!("*{}*", v.to_string())));
        }
        self
    }

    /// Add a prefix filter using `LIKE 'value*'`. Skipped when `val` is `None`.
    ///
    /// Uses `*` as the wildcard (`PostgREST` translates it to SQL `%`). Avoids using
    /// `%` directly in the URL, which gets double-encoded by `reqwest`'s URL parser.
    ///
    /// Useful for CIP code family filtering (e.g. `starts_with("cip_code", Some("11."))`).
    #[must_use]
    pub fn starts_with<T: ToString>(mut self, col: &'static str, val: Option<T>) -> Self {
        if let Some(v) = val {
            self.entries
                .push((FilterKind::StartsWith, col, format!("{}*", v.to_string())));
        }
        self
    }

    /// Add a greater-than-or-equal filter. Skipped when `val` is `None`.
    #[must_use]
    pub fn gte<T: ToString>(mut self, col: &'static str, val: Option<T>) -> Self {
        if let Some(v) = val {
            self.entries.push((FilterKind::Gte, col, v.to_string()));
        }
        self
    }

    /// Add a less-than-or-equal filter. Skipped when `val` is `None`.
    #[must_use]
    pub fn lte<T: ToString>(mut self, col: &'static str, val: Option<T>) -> Self {
        if let Some(v) = val {
            self.entries.push((FilterKind::Lte, col, v.to_string()));
        }
        self
    }

    /// Add an `IN` list filter. Values are joined with commas and stored internally.
    ///
    /// Skipped when `vals` is empty. Values must not contain commas (safe for integers
    /// and dot-notation CIP codes).
    #[must_use]
    pub fn in_list<T: ToString>(mut self, col: &'static str, vals: &[T]) -> Self {
        if !vals.is_empty() {
            let joined = vals
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",");
            self.entries.push((FilterKind::In, col, joined));
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
        // Uses * not % — PostgREST maps * to SQL %, avoiding reqwest URL double-encoding
        let f = QueryFilters::new().ilike("name", Some("northeastern"));
        assert_eq!(f.entries[0].2, "*northeastern*");
    }

    #[test]
    fn test_starts_with_appends_wildcard() {
        // Uses * not % — avoids URL percent-encoding issue
        let f = QueryFilters::new().starts_with("cip_code", Some("11."));
        assert_eq!(f.entries[0].2, "11.*");
    }

    #[test]
    fn test_chaining() {
        let f = QueryFilters::new()
            .eq("state", Some("MA"))
            .eq::<i32>("carnegie_class", None)
            .eq("control", Some(1));
        assert_eq!(f.entries.len(), 2);
    }

    #[test]
    fn test_gte_some_adds_entry() {
        let f = QueryFilters::new().gte("inst_size", Some(2));
        assert_eq!(f.entries.len(), 1);
        assert!(matches!(f.entries[0].0, FilterKind::Gte));
        assert_eq!(f.entries[0].2, "2");
    }

    #[test]
    fn test_gte_none_skips() {
        let f = QueryFilters::new().gte::<i32>("inst_size", None);
        assert!(f.is_empty());
    }

    #[test]
    fn test_lte_some_adds_entry() {
        let f = QueryFilters::new().lte("year", Some(2024));
        assert!(matches!(f.entries[0].0, FilterKind::Lte));
        assert_eq!(f.entries[0].2, "2024");
    }

    #[test]
    fn test_lte_none_skips() {
        let f = QueryFilters::new().lte::<i32>("year", None);
        assert!(f.is_empty());
    }

    #[test]
    fn test_gte_lte_chaining() {
        let f = QueryFilters::new()
            .gte("year", Some(2020))
            .lte("year", Some(2024));
        assert_eq!(f.entries.len(), 2);
        assert!(matches!(f.entries[0].0, FilterKind::Gte));
        assert!(matches!(f.entries[1].0, FilterKind::Lte));
    }

    #[test]
    fn test_in_list_joins_with_commas() {
        let f = QueryFilters::new().in_list("unitid", &[167_358_i32, 166_629, 155_317]);
        assert_eq!(f.entries.len(), 1);
        assert!(matches!(f.entries[0].0, FilterKind::In));
        assert_eq!(f.entries[0].2, "167358,166629,155317");
    }

    #[test]
    fn test_in_list_empty_skips() {
        let f = QueryFilters::new().in_list::<i32>("unitid", &[]);
        assert!(f.is_empty());
    }

    #[test]
    fn test_in_list_strings() {
        let codes = vec!["11.0101".to_string(), "11.0201".to_string()];
        let f = QueryFilters::new().in_list("cip_code", &codes);
        assert_eq!(f.entries[0].2, "11.0101,11.0201");
    }
}
