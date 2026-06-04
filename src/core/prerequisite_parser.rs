//! Shared prerequisite expression parsing utilities
//!
//! This module provides functions to parse prerequisite expressions in various forms:
//! - DNF (Disjunctive Normal Form) - OR of ANDs
//! - Flat edge list - for graph traversal
//! - Strict prerequisites only - for validation
//!
//! # Expression Syntax
//!
//! Prerequisite expressions use:
//! - `&` for AND (all required)
//! - `|` for OR (choose one)
//! - `()` for grouping
//! - `[X]` for grade requirements (stripped during parsing)
//!
//! # Examples
//!
//! - `CS101` - Single prerequisite
//! - `CS101 & CS102` - Both required
//! - `CS101 | CS102` - Either one required
//! - `(CS101 & CS102) | CS103` - Both CS101 and CS102, OR just CS103
//! - `CS101[B] & CS102[C]` - With grade requirements (stripped)

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::HashSet;

/// AND/OR tree form of a prerequisite expression.
///
/// Mirrors the original expression structure (unlike [`parse_to_dnf`], which
/// loses the AND-of-OR shape). Used by `degree trim` to rewrite each
/// disjunct independently while keeping the rest of the expression intact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrereqExpr {
    /// A single course reference (e.g. "CS2510").
    Course(String),
    /// Conjunction — every child must be satisfied.
    All(Vec<Self>),
    /// Disjunction — at least one child must be satisfied.
    Any(Vec<Self>),
}

impl PrereqExpr {
    /// Render this AST back into the same boolean-expression syntax that
    /// [`parse_to_ast`] accepts (`A`, `A & B`, `A | B`, with parens where
    /// precedence requires).
    #[must_use]
    pub fn to_expression_string(&self) -> String {
        match self {
            Self::Course(c) => c.clone(),
            Self::All(xs) => {
                if xs.is_empty() {
                    return String::new();
                }
                if xs.len() == 1 {
                    return xs[0].to_expression_string();
                }
                xs.iter()
                    .map(|child| match child {
                        // `&` binds tighter than `|`, so an `Any` inside an
                        // `All` needs parens to preserve meaning.
                        Self::Any(inner) if inner.len() > 1 => {
                            format!("({})", child.to_expression_string())
                        }
                        _ => child.to_expression_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(" & ")
            }
            Self::Any(xs) => {
                if xs.is_empty() {
                    return String::new();
                }
                if xs.len() == 1 {
                    return xs[0].to_expression_string();
                }
                // Children of `Any` never need parens: `All` is higher
                // precedence and a nested `Any` flattens trivially since
                // OR is associative.
                xs.iter()
                    .map(Self::to_expression_string)
                    .collect::<Vec<_>>()
                    .join(" | ")
            }
        }
    }

    /// Returns true if this expression is empty (no courses referenced).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Course(c) => c.is_empty(),
            Self::All(xs) | Self::Any(xs) => xs.iter().all(Self::is_empty),
        }
    }
}

// Unified-JSON serialization for prerequisites (symmetric tagged form):
//   - a leaf course      -> a bare JSON string  ("CS101")
//   - All (AND)          -> {"and": [ <children> ]}
//   - Any (OR)           -> {"or":  [ <children> ]}
// This nests to any depth and round-trips losslessly with the AST. The node
// type encodes the operator, so the structure is unambiguous for consumers.
impl Serialize for PrereqExpr {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        use serde::ser::SerializeMap;
        match self {
            Self::Course(c) => serializer.serialize_str(c),
            Self::All(xs) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("and", xs)?;
                map.end()
            }
            Self::Any(xs) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("or", xs)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for PrereqExpr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PrereqExprVisitor;

        impl<'de> serde::de::Visitor<'de> for PrereqExprVisitor {
            type Value = PrereqExpr;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a course string or an object with a single `and`/`or` key")
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(PrereqExpr::Course(v.to_string()))
            }

            fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(PrereqExpr::Course(v))
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                let key: String = map
                    .next_key()?
                    .ok_or_else(|| serde::de::Error::custom("expected `and` or `or` key"))?;
                let children: Vec<PrereqExpr> = map.next_value()?;
                if map.next_key::<String>()?.is_some() {
                    return Err(serde::de::Error::custom(
                        "prerequisite object must have exactly one key (`and` or `or`)",
                    ));
                }
                match key.as_str() {
                    "and" => Ok(PrereqExpr::All(children)),
                    "or" => Ok(PrereqExpr::Any(children)),
                    other => Err(serde::de::Error::custom(format!(
                        "unknown prerequisite operator `{other}` (expected `and` or `or`)"
                    ))),
                }
            }
        }

        deserializer.deserialize_any(PrereqExprVisitor)
    }
}

/// Parse a prerequisite expression into a structural AND/OR tree.
///
/// Returns `None` for empty input. Operator precedence matches
/// [`parse_to_dnf`]: `&` binds tighter than `|`, parens override.
///
/// # Examples
/// ```
/// use nu_analytics::core::prerequisite_parser::{parse_to_ast, PrereqExpr};
///
/// let ast = parse_to_ast("(CS101 & CS102) | CS103").unwrap();
/// match ast {
///     PrereqExpr::Any(branches) => assert_eq!(branches.len(), 2),
///     _ => panic!("expected top-level OR"),
/// }
/// ```
#[must_use]
pub fn parse_to_ast(raw: &str) -> Option<PrereqExpr> {
    let cleaned = remove_grade_requirements(raw);
    parse_ast_recursive(&cleaned)
}

fn parse_ast_recursive(s: &str) -> Option<PrereqExpr> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }

    let unwrapped = unwrap_parens(trimmed);

    if contains_at_level(unwrapped, '|') {
        let parts: Vec<PrereqExpr> = split_at_level(unwrapped, '|')
            .into_iter()
            .filter_map(|p| parse_ast_recursive(&p))
            .collect();
        return match parts.len() {
            0 => None,
            1 => parts.into_iter().next(),
            _ => Some(PrereqExpr::Any(parts)),
        };
    }

    if contains_at_level(unwrapped, '&') {
        let parts: Vec<PrereqExpr> = split_at_level(unwrapped, '&')
            .into_iter()
            .filter_map(|p| parse_ast_recursive(&p))
            .collect();
        return match parts.len() {
            0 => None,
            1 => parts.into_iter().next(),
            _ => Some(PrereqExpr::All(parts)),
        };
    }

    let course = clean_course_key(unwrapped);
    if course.is_empty() {
        None
    } else {
        Some(PrereqExpr::Course(course))
    }
}

/// Parse a prerequisite expression into DNF form (OR of ANDs)
///
/// Each inner `Vec` represents a valid path (all courses must be taken).
/// The outer `Vec` represents alternatives (any one path satisfies the requirement).
///
/// # Arguments
/// * `raw` - The raw prerequisite expression string
///
/// # Returns
/// A vector of paths, where each path is a vector of course keys
///
/// # Examples
/// ```
/// use nu_analytics::core::prerequisite_parser::parse_to_dnf;
///
/// let result = parse_to_dnf("(CS101 & CS102) | CS103");
/// assert_eq!(result.len(), 2);
/// assert!(result.contains(&vec!["CS101".to_string(), "CS102".to_string()]));
/// assert!(result.contains(&vec!["CS103".to_string()]));
/// ```
#[must_use]
pub fn parse_to_dnf(raw: &str) -> Vec<Vec<String>> {
    let cleaned = remove_grade_requirements(raw);
    parse_dnf_recursive(&cleaned)
}

/// Parse a prerequisite expression into a flat list of edges for graph traversal
///
/// Returns tuples of (`course_key`, `is_optional`, `or_group`).
/// - `is_optional`: true if part of an OR group
/// - `or_group`: `Some(id)` if optional, where same id = alternatives
///
/// # Arguments
/// * `raw` - The raw prerequisite expression string
///
/// # Returns
/// A vector of (`course`, `is_optional`, `or_group`) tuples
#[must_use]
pub fn parse_to_edges(raw: &str) -> Vec<(String, bool, Option<usize>)> {
    let mut result = Vec::new();
    let mut or_group_counter = 0;

    let cleaned = remove_grade_requirements(raw);
    let and_parts = split_at_level(&cleaned, '&');

    for part in and_parts {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }

        if contains_at_level(trimmed, '|') {
            // OR group
            let or_parts = split_at_level(trimmed, '|');
            let current_group = or_group_counter;
            or_group_counter += 1;

            for or_part in or_parts {
                let or_trimmed = or_part.trim();
                if or_trimmed.is_empty() {
                    continue;
                }

                let unwrapped = unwrap_parens(or_trimmed);

                if contains_at_level(unwrapped, '&') || contains_at_level(unwrapped, '|') {
                    // Complex nested - extract all courses
                    for course in extract_all_courses(unwrapped) {
                        result.push((course, true, Some(current_group)));
                    }
                } else {
                    let course = clean_course_key(unwrapped);
                    if !course.is_empty() {
                        result.push((course, true, Some(current_group)));
                    }
                }
            }
        } else {
            // Required prerequisite
            let unwrapped = unwrap_parens(trimmed);

            if contains_at_level(unwrapped, '&') {
                for (course, is_opt, group) in parse_to_edges(unwrapped) {
                    result.push((course, is_opt, group.map(|g| g + or_group_counter)));
                }
            } else if contains_at_level(unwrapped, '|') {
                for (course, _, _) in parse_to_edges(unwrapped) {
                    result.push((course, true, Some(or_group_counter)));
                }
                or_group_counter += 1;
            } else {
                let course = clean_course_key(unwrapped);
                if !course.is_empty() {
                    result.push((course, false, None));
                }
            }
        }
    }

    result
}

/// Extract only strict (required) prerequisites from an expression
///
/// This extracts courses that must be taken (not alternatives).
/// Used for validation and strict prerequisite checking.
///
/// # Arguments
/// * `raw` - The raw prerequisite expression string
///
/// # Returns
/// A set of course keys that are strictly required
#[must_use]
pub fn extract_strict_prerequisites(raw: &str) -> HashSet<String> {
    let mut strict = HashSet::new();
    let edges = parse_to_edges(raw);

    for (course, is_optional, _) in edges {
        if !is_optional {
            strict.insert(course);
        }
    }

    strict
}

/// Extract all course keys mentioned in a prerequisite expression
///
/// This returns all courses, regardless of AND/OR logic.
///
/// # Arguments
/// * `raw` - The raw prerequisite expression string
///
/// # Returns
/// A set of all course keys found in the expression
#[must_use]
pub fn extract_all_courses(raw: &str) -> Vec<String> {
    let mut courses = Vec::new();
    let cleaned = raw.replace(['(', ')', '&', '|', '[', ']'], " ");

    for part in cleaned.split_whitespace() {
        let key = part.trim();
        // Filter out grade requirements (single letters)
        if !key.is_empty() && key.len() > 1 {
            courses.push(key.to_string());
        }
    }

    courses
}

// ============================================================================
// Private Helper Functions
// ============================================================================

/// Recursively parse prerequisite expression into DNF
fn parse_dnf_recursive(s: &str) -> Vec<Vec<String>> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let unwrapped = unwrap_parens(trimmed);

    // Check for top-level OR
    if contains_at_level(unwrapped, '|') {
        let or_parts = split_at_level(unwrapped, '|');
        let mut result = Vec::new();
        for part in or_parts {
            let part_dnf = parse_dnf_recursive(part.trim());
            result.extend(part_dnf);
        }
        return result;
    }

    // Check for top-level AND
    if contains_at_level(unwrapped, '&') {
        let and_parts = split_at_level(unwrapped, '&');
        let mut current_paths: Vec<Vec<String>> = vec![vec![]];

        for part in and_parts {
            let part_dnf = parse_dnf_recursive(part.trim());
            if part_dnf.is_empty() {
                continue;
            }

            // Cartesian product
            let mut new_paths = Vec::new();
            for existing in &current_paths {
                for new_part in &part_dnf {
                    let mut combined = existing.clone();
                    combined.extend(new_part.clone());
                    new_paths.push(combined);
                }
            }
            current_paths = new_paths;
        }

        return current_paths;
    }

    // Single course
    let course = clean_course_key(unwrapped);
    if course.is_empty() {
        Vec::new()
    } else {
        vec![vec![course]]
    }
}

/// Remove grade requirements like `[B]` or `[C]` from an expression
fn remove_grade_requirements(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_bracket = false;

    for c in s.chars() {
        if c == '[' {
            in_bracket = true;
        } else if c == ']' {
            in_bracket = false;
        } else if !in_bracket {
            result.push(c);
        }
    }

    result
}

/// Split a string by delimiter at top level, respecting parentheses
fn split_at_level(s: &str, delimiter: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut level: i32 = 0;

    for c in s.chars() {
        if c == '(' {
            level += 1;
            current.push(c);
        } else if c == ')' {
            level = level.saturating_sub(1);
            current.push(c);
        } else if c == delimiter && level == 0 {
            parts.push(current.clone());
            current.clear();
        } else {
            current.push(c);
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

/// Check if a delimiter exists at top level (not in parentheses)
fn contains_at_level(s: &str, delimiter: char) -> bool {
    let mut level: i32 = 0;

    for c in s.chars() {
        if c == '(' {
            level += 1;
        } else if c == ')' {
            level = level.saturating_sub(1);
        } else if c == delimiter && level == 0 {
            return true;
        }
    }

    false
}

/// Unwrap outer parentheses if they wrap the entire string
fn unwrap_parens(s: &str) -> &str {
    let trimmed = s.trim();
    if trimmed.starts_with('(') && trimmed.ends_with(')') {
        let mut level = 0;
        for (i, c) in trimmed.chars().enumerate() {
            if c == '(' {
                level += 1;
            } else if c == ')' {
                level -= 1;
                if level == 0 && i < trimmed.len() - 1 {
                    return trimmed;
                }
            }
        }
        return &trimmed[1..trimmed.len() - 1];
    }
    trimmed
}

/// Clean a course key (remove parens, trim whitespace)
fn clean_course_key(s: &str) -> String {
    s.replace(['(', ')'], "").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_to_dnf_simple() {
        let result = parse_to_dnf("CS101");
        assert_eq!(result, vec![vec!["CS101".to_string()]]);
    }

    #[test]
    fn test_prereq_expr_json_tagged_form() {
        // Leaf -> bare string
        let leaf = PrereqExpr::Course("CS101".into());
        assert_eq!(serde_json::to_string(&leaf).unwrap(), "\"CS101\"");

        // (X & Y) | Z  ->  {"or":[{"and":["X","Y"]},"Z"]}
        let expr = parse_to_ast("(X & Y) | Z").unwrap();
        let json = serde_json::to_string(&expr).unwrap();
        assert_eq!(json, r#"{"or":[{"and":["X","Y"]},"Z"]}"#);

        // Round-trips losslessly
        let back: PrereqExpr = serde_json::from_str(&json).unwrap();
        assert_eq!(back, expr);
    }

    #[test]
    fn test_prereq_expr_json_rejects_bad_operator() {
        let err = serde_json::from_str::<PrereqExpr>(r#"{"xor":["A","B"]}"#);
        assert!(err.is_err());
    }

    #[test]
    fn test_prereq_expr_deserialize_bare_string_leaf() {
        let leaf: PrereqExpr = serde_json::from_str("\"CS101\"").unwrap();
        assert_eq!(leaf, PrereqExpr::Course("CS101".to_string()));
    }

    #[test]
    fn test_prereq_expr_deserialize_empty_and_or() {
        let and: PrereqExpr = serde_json::from_str(r#"{"and":[]}"#).unwrap();
        assert_eq!(and, PrereqExpr::All(vec![]));
        let or: PrereqExpr = serde_json::from_str(r#"{"or":[]}"#).unwrap();
        assert_eq!(or, PrereqExpr::Any(vec![]));
    }

    #[test]
    fn test_prereq_expr_deserialize_rejects_two_key_object() {
        let err = serde_json::from_str::<PrereqExpr>(r#"{"and":["A"],"or":["B"]}"#);
        assert!(err.is_err(), "object with two keys must be rejected");
    }

    #[test]
    fn test_prereq_expr_deserialize_nested_depth_roundtrips() {
        // (A & (B | C)) | D — recursion through both node types.
        let json = r#"{"or":[{"and":["A",{"or":["B","C"]}]},"D"]}"#;
        let expr: PrereqExpr = serde_json::from_str(json).unwrap();
        assert_eq!(
            expr,
            PrereqExpr::Any(vec![
                PrereqExpr::All(vec![
                    PrereqExpr::Course("A".into()),
                    PrereqExpr::Any(vec![
                        PrereqExpr::Course("B".into()),
                        PrereqExpr::Course("C".into()),
                    ]),
                ]),
                PrereqExpr::Course("D".into()),
            ])
        );
        assert_eq!(serde_json::to_string(&expr).unwrap(), json);
    }

    #[test]
    fn test_parse_to_dnf_and() {
        let result = parse_to_dnf("CS101 & CS102");
        assert_eq!(result, vec![vec!["CS101".to_string(), "CS102".to_string()]]);
    }

    #[test]
    fn test_parse_to_dnf_or() {
        let result = parse_to_dnf("CS101 | CS102");
        assert_eq!(
            result,
            vec![vec!["CS101".to_string()], vec!["CS102".to_string()]]
        );
    }

    #[test]
    fn test_parse_to_dnf_mixed() {
        let result = parse_to_dnf("(CS101 & CS102) | CS103");
        assert_eq!(result.len(), 2);
        assert!(result.contains(&vec!["CS101".to_string(), "CS102".to_string()]));
        assert!(result.contains(&vec!["CS103".to_string()]));
    }

    #[test]
    fn test_parse_to_edges_simple() {
        let result = parse_to_edges("CS101");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], ("CS101".to_string(), false, None));
    }

    #[test]
    fn test_parse_to_edges_or() {
        let result = parse_to_edges("CS101 | CS102");
        assert_eq!(result.len(), 2);
        assert!(result
            .iter()
            .any(|r| r.0 == "CS101" && r.1 && r.2 == Some(0)));
        assert!(result
            .iter()
            .any(|r| r.0 == "CS102" && r.1 && r.2 == Some(0)));
    }

    #[test]
    fn test_extract_strict() {
        let result = extract_strict_prerequisites("CS101 & (CS102 | CS103)");
        assert!(result.contains("CS101"));
        assert!(!result.contains("CS102"));
        assert!(!result.contains("CS103"));
    }

    #[test]
    fn test_extract_all() {
        let result = extract_all_courses("(CS101 & CS102) | CS103");
        assert_eq!(result.len(), 3);
        assert!(result.contains(&"CS101".to_string()));
        assert!(result.contains(&"CS102".to_string()));
        assert!(result.contains(&"CS103".to_string()));
    }

    // Edge case tests
    #[test]
    fn test_parse_empty_string() {
        let result = parse_to_dnf("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_whitespace_only() {
        let result = parse_to_dnf("   ");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_deeply_nested() {
        let result = parse_to_dnf("((A & B) | (C & D)) & ((E & F) | (G & H))");
        // Should produce 4 paths: A&B&E&F, A&B&G&H, C&D&E&F, C&D&G&H
        assert_eq!(result.len(), 4);
        assert!(result.contains(&vec![
            "A".to_string(),
            "B".to_string(),
            "E".to_string(),
            "F".to_string()
        ]));
        assert!(result.contains(&vec![
            "A".to_string(),
            "B".to_string(),
            "G".to_string(),
            "H".to_string()
        ]));
        assert!(result.contains(&vec![
            "C".to_string(),
            "D".to_string(),
            "E".to_string(),
            "F".to_string()
        ]));
        assert!(result.contains(&vec![
            "C".to_string(),
            "D".to_string(),
            "G".to_string(),
            "H".to_string()
        ]));
    }

    #[test]
    fn test_parse_malformed_parens() {
        // Extra closing paren - should still parse what it can
        let result = parse_to_dnf("CS101)");
        assert!(!result.is_empty());
        assert!(result[0].contains(&"CS101".to_string()));
    }

    #[test]
    fn test_parse_only_operators() {
        let result = parse_to_dnf("& | &");
        // Should attempt to parse but result in empty or minimal output
        // The parser extracts tokens, so this might create empty paths
        // Just verify it doesn't crash
        assert!(
            result.is_empty()
                || result
                    .iter()
                    .all(|path| path.is_empty() || path.iter().all(String::is_empty))
        );
    }

    #[test]
    fn test_parse_with_numbers_only() {
        let result = parse_to_dnf("CS101 & 123");
        // Parser will attempt to parse "123" as a course
        assert!(!result.is_empty());
        assert!(result[0].contains(&"CS101".to_string()));
        // The number "123" might be included since it's > 1 char
        // This is expected behavior - parser doesn't validate course format
    }

    #[test]
    fn test_extract_strict_all_optional() {
        let result = extract_strict_prerequisites("CS101 | CS102 | CS103");
        assert!(result.is_empty());
    }

    #[test]
    fn test_extract_strict_mixed() {
        let result = extract_strict_prerequisites("(A & B) & (C | D)");
        assert!(result.contains("A"));
        assert!(result.contains("B"));
        assert!(!result.contains("C"));
        assert!(!result.contains("D"));
    }

    #[test]
    fn test_parse_to_edges_complex_nested() {
        let result = parse_to_edges("A & (B | (C & D))");
        // A should be required
        assert!(result.iter().any(|r| r.0 == "A" && !r.1));
        // B, C, D should be in OR groups
        let has_optional = result.iter().any(|r| r.1);
        assert!(has_optional);
    }

    #[test]
    fn test_grade_requirements_with_plus_minus() {
        let result = parse_to_dnf("CS101[A+] & CS102[B-]");
        assert_eq!(result, vec![vec!["CS101".to_string(), "CS102".to_string()]]);
    }

    // -----------------------------------------------------------------
    // AST tests
    // -----------------------------------------------------------------

    #[test]
    fn test_ast_single_course() {
        let ast = parse_to_ast("CS101").unwrap();
        assert_eq!(ast, PrereqExpr::Course("CS101".to_string()));
        assert_eq!(ast.to_expression_string(), "CS101");
    }

    #[test]
    fn test_ast_and() {
        let ast = parse_to_ast("CS101 & CS102").unwrap();
        assert_eq!(
            ast,
            PrereqExpr::All(vec![
                PrereqExpr::Course("CS101".to_string()),
                PrereqExpr::Course("CS102".to_string()),
            ])
        );
        assert_eq!(ast.to_expression_string(), "CS101 & CS102");
    }

    #[test]
    fn test_ast_or() {
        let ast = parse_to_ast("CS101 | CS102").unwrap();
        assert_eq!(
            ast,
            PrereqExpr::Any(vec![
                PrereqExpr::Course("CS101".to_string()),
                PrereqExpr::Course("CS102".to_string()),
            ])
        );
        assert_eq!(ast.to_expression_string(), "CS101 | CS102");
    }

    #[test]
    fn test_ast_and_of_or_keeps_parens() {
        // AND of OR needs parens to preserve precedence on emit.
        let ast = parse_to_ast("CS101 & (CS102 | CS103)").unwrap();
        let s = ast.to_expression_string();
        let reparsed = parse_to_ast(&s).unwrap();
        assert_eq!(reparsed, ast, "round-trip mismatch: {s}");
        assert!(
            s.contains('('),
            "expected parens around OR child of AND, got {s}"
        );
    }

    #[test]
    fn test_ast_or_of_and_no_parens_needed() {
        let ast = parse_to_ast("(CS101 & CS102) | CS103").unwrap();
        let s = ast.to_expression_string();
        // OR is the lower-precedence operator, so its AND children don't
        // need parens. Round-trip must still preserve the structure.
        let reparsed = parse_to_ast(&s).unwrap();
        assert_eq!(reparsed, ast, "round-trip mismatch: {s}");
    }

    #[test]
    fn test_ast_round_trip_real_world_examples() {
        // Drawn from samples/degrees/neu-khoury-bscs-boston.yaml.
        for expr in [
            "CS2000",
            "CS2100 | DS2500",
            "CS3100 & CS3520",
            "(CS2100 | DS2500) & CS1800",
            "(CS1800 | MATH1365) & CS2100",
            "CS3100 & (DS3000 | MATH2331)",
        ] {
            let ast = parse_to_ast(expr).expect("parse failed");
            let emitted = ast.to_expression_string();
            let reparsed = parse_to_ast(&emitted).expect("re-parse failed");
            assert_eq!(reparsed, ast, "round-trip mismatch for {expr}: {emitted}");
        }
    }

    #[test]
    fn test_ast_strips_grade_requirements() {
        let ast = parse_to_ast("CS101[B-] & CS102[A]").unwrap();
        assert_eq!(ast.to_expression_string(), "CS101 & CS102");
    }

    #[test]
    fn test_ast_empty_returns_none() {
        assert!(parse_to_ast("").is_none());
        assert!(parse_to_ast("   ").is_none());
    }

    #[test]
    fn test_ast_is_empty_recurses_through_aggregates() {
        // Course with empty string is "empty" (no course referenced).
        assert!(PrereqExpr::Course(String::new()).is_empty());
        assert!(!PrereqExpr::Course("CS101".to_string()).is_empty());

        // All/Any with only empty children are themselves empty.
        let all_empty = PrereqExpr::All(vec![
            PrereqExpr::Course(String::new()),
            PrereqExpr::Course(String::new()),
        ]);
        assert!(all_empty.is_empty());

        // A single non-empty leaf is enough to make the aggregate non-empty.
        let any_one_real = PrereqExpr::Any(vec![
            PrereqExpr::Course(String::new()),
            PrereqExpr::Course("CS101".to_string()),
        ]);
        assert!(!any_one_real.is_empty());

        // Recursion through nested aggregates.
        let nested_empty = PrereqExpr::All(vec![PrereqExpr::Any(vec![PrereqExpr::Course(
            String::new(),
        )])]);
        assert!(nested_empty.is_empty());
    }

    #[test]
    fn test_extract_all_with_special_chars() {
        let result = extract_all_courses("CS-101 & CS_102[A] | CS.103");
        // Should extract courses even with special chars
        assert!(!result.is_empty());
    }
}
