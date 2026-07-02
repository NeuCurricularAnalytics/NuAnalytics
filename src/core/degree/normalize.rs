//! Normalize a [`DegreeProgram`] into a flat, format-agnostic course set.
//!
//! Used by the test-suite pipeline to produce a common representation that
//! can be diffed against a ground-truth `course_verifier` record regardless
//! of whether the source was a hand-authored YAML, a `degree convert` unified
//! JSON, or a raw ai-landscape cluster file.
//!
//! Output shape (per program):
//! ```jsonc
//! {
//!   "institution": "Northeastern University",
//!   "program":     "BS in Artificial Intelligence",
//!   "courses": {
//!     "CS3000": {
//!       "title":               "Algorithms and Data",
//!       "credit_hours":        4.0,
//!       "prerequisites":       [["CS2100", "DS2500"], ["CS1800"]],
//!       "corequisites":        [],
//!       "strict_corequisites": []
//!     }
//!   }
//! }
//! ```
//!
//! `prerequisites` uses **AND-of-OR** (outer = AND groups, inner = OR
//! alternatives within each group) to match the `course_verifier` convention.
//! Each alternative may be a compound `"A and B"` string when an AND clause
//! appeared inside an OR branch.

use std::collections::HashMap;

use serde::Serialize;

use crate::core::models::DegreeProgram;
use crate::core::prerequisite_parser::{parse_to_ast, PrereqExpr};

// ---------------------------------------------------------------------------
// Output types
// ---------------------------------------------------------------------------

/// Flat, format-agnostic representation of one course.
#[derive(Debug, Clone, Serialize)]
pub struct NormalizedCourse {
    /// Course title / name.
    pub title: String,
    /// Credit hours (defaults to 0.0 when unknown).
    pub credit_hours: f32,
    /// AND-of-OR prerequisite groups.  Outer list = AND; inner list = OR
    /// alternatives.  An empty outer list means no prerequisites.
    pub prerequisites: Vec<Vec<String>>,
    /// Flat list of co-requisite course codes (normalized, no spaces).
    pub corequisites: Vec<String>,
    /// Flat list of strict co-requisite course codes.
    pub strict_corequisites: Vec<String>,
}

/// Normalized representation of one degree program.
#[derive(Debug, Clone, Serialize)]
pub struct NormalizedProgram {
    /// Institution name (empty string if unknown).
    pub institution: String,
    /// Program / degree name (empty string if unknown).
    pub program: String,
    /// Courses keyed by normalized course code (no spaces, e.g. `"CS3000"`).
    pub courses: HashMap<String, NormalizedCourse>,
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Normalize a [`DegreeProgram`] into a [`NormalizedProgram`].
///
/// The institution and program name are taken from `program.degree`; pass
/// `institution_override` / `program_override` to supply them when the
/// loaded degree lacks metadata (e.g. a partially-converted cluster record).
#[must_use]
pub fn normalize_program(
    program: &DegreeProgram,
    institution_override: Option<&str>,
    program_override: Option<&str>,
) -> NormalizedProgram {
    let institution = institution_override
        .map(str::to_owned)
        .or_else(|| program.degree.institution.clone())
        .unwrap_or_default();

    let program_name = program_override
        .map(str::to_owned)
        .unwrap_or_else(|| program.degree.name.clone());

    let courses = program
        .courses
        .iter()
        .map(|(key, course)| {
            let normalized = NormalizedCourse {
                title: course.name.clone(),
                credit_hours: course.credit_hours,
                prerequisites: prereq_raw_to_and_of_or(course.prerequisites_raw.as_deref()),
                corequisites: normalize_codes(&course.corequisites),
                strict_corequisites: normalize_codes(&course.strict_corequisites),
            };
            (key.clone(), normalized)
        })
        .collect();

    NormalizedProgram {
        institution,
        program: program_name,
        courses,
    }
}

// ---------------------------------------------------------------------------
// Prerequisite conversion
// ---------------------------------------------------------------------------

/// Parse a raw prerequisite expression string and convert it to AND-of-OR.
///
/// Returns an empty `Vec` for a missing or empty expression.
fn prereq_raw_to_and_of_or(raw: Option<&str>) -> Vec<Vec<String>> {
    let Some(s) = raw else { return vec![] };
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return vec![];
    }
    match parse_to_ast(trimmed) {
        Some(expr) => expr_to_and_of_or(&expr),
        None => vec![],
    }
}

/// Recursively convert a [`PrereqExpr`] to AND-of-OR lists.
///
/// - `Course(c)` → `[[c]]`
/// - `All([A, B])` → flatten: one OR-group per child
/// - `Any([A, B])` → one OR-group with all alternatives
///
/// An `All` nested inside an `Any` alternative is joined with `" and "` to
/// produce a compound alternative string, matching the `course_verifier`
/// convention for inner AND expressions.
fn expr_to_and_of_or(expr: &PrereqExpr) -> Vec<Vec<String>> {
    match expr {
        PrereqExpr::Course(c) => vec![vec![normalize_code(c)]],
        PrereqExpr::All(parts) => parts.iter().flat_map(expr_to_and_of_or).collect(),
        PrereqExpr::Any(alts) => {
            let group: Vec<String> = alts.iter().map(expr_to_or_alt).collect();
            if group.is_empty() {
                vec![]
            } else {
                vec![group]
            }
        }
    }
}

/// Render one OR-alternative, collapsing any nested AND into a `"A and B"` string.
fn expr_to_or_alt(expr: &PrereqExpr) -> String {
    match expr {
        PrereqExpr::Course(c) => normalize_code(c),
        PrereqExpr::All(parts) => parts
            .iter()
            .map(expr_to_or_alt)
            .collect::<Vec<_>>()
            .join(" and "),
        PrereqExpr::Any(alts) => alts
            .first()
            .map_or_else(String::new, expr_to_or_alt),
    }
}

// ---------------------------------------------------------------------------
// Code normalization
// ---------------------------------------------------------------------------

/// Strip spaces and brackets from a course code, e.g. `"CS 3000"` → `"CS3000"`.
fn normalize_code(code: &str) -> String {
    code.chars()
        .filter(|c| !c.is_whitespace() && *c != '[' && *c != ']')
        .collect()
}

/// Normalize a slice of course codes.
fn normalize_codes(codes: &[String]) -> Vec<String> {
    codes.iter().map(|c| normalize_code(c)).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_course() {
        let expr = PrereqExpr::Course("CS3000".into());
        assert_eq!(expr_to_and_of_or(&expr), vec![vec!["CS3000"]]);
    }

    #[test]
    fn and_of_courses() {
        // CS2100 & CS1800  →  [["CS2100"], ["CS1800"]]
        let expr = PrereqExpr::All(vec![
            PrereqExpr::Course("CS2100".into()),
            PrereqExpr::Course("CS1800".into()),
        ]);
        assert_eq!(
            expr_to_and_of_or(&expr),
            vec![vec!["CS2100"], vec!["CS1800"]]
        );
    }

    #[test]
    fn or_of_courses() {
        // CS2100 | DS2500  →  [["CS2100", "DS2500"]]
        let expr = PrereqExpr::Any(vec![
            PrereqExpr::Course("CS2100".into()),
            PrereqExpr::Course("DS2500".into()),
        ]);
        assert_eq!(
            expr_to_and_of_or(&expr),
            vec![vec!["CS2100", "DS2500"]]
        );
    }

    #[test]
    fn and_of_or() {
        // (CS2100 | DS2500) & CS1800  →  [["CS2100","DS2500"], ["CS1800"]]
        let expr = PrereqExpr::All(vec![
            PrereqExpr::Any(vec![
                PrereqExpr::Course("CS2100".into()),
                PrereqExpr::Course("DS2500".into()),
            ]),
            PrereqExpr::Course("CS1800".into()),
        ]);
        assert_eq!(
            expr_to_and_of_or(&expr),
            vec![vec!["CS2100", "DS2500"], vec!["CS1800"]]
        );
    }

    #[test]
    fn and_inside_or_becomes_compound_string() {
        // (A & B) | C  →  [["A and B", "C"]]
        let expr = PrereqExpr::Any(vec![
            PrereqExpr::All(vec![
                PrereqExpr::Course("MAC2312".into()),
                PrereqExpr::Course("MAD2104".into()),
            ]),
            PrereqExpr::Course("MAC2313".into()),
        ]);
        assert_eq!(
            expr_to_and_of_or(&expr),
            vec![vec!["MAC2312 and MAD2104", "MAC2313"]]
        );
    }

    #[test]
    fn empty_prereq_raw() {
        assert!(prereq_raw_to_and_of_or(None).is_empty());
        assert!(prereq_raw_to_and_of_or(Some("")).is_empty());
        assert!(prereq_raw_to_and_of_or(Some("   ")).is_empty());
    }

    #[test]
    fn parse_expression_string() {
        // Round-trip through the real parser
        let result = prereq_raw_to_and_of_or(Some("(CS2100 | DS2500) & CS1800"));
        assert_eq!(result, vec![vec!["CS2100", "DS2500"], vec!["CS1800"]]);
    }

    #[test]
    fn normalize_code_strips_spaces_and_brackets() {
        assert_eq!(normalize_code("CS 3000"), "CS3000");
        assert_eq!(normalize_code("CS3000[B]"), "CS3000B");
        assert_eq!(normalize_code("CS3000"), "CS3000");
    }
}
