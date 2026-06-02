//! Converter: ai-landscape program JSON -> unified [`DegreeProgram`].
//!
//! ai-landscape-tools (the sibling web-crawling/validation project) emits one
//! JSON file per program with a fixed shape:
//!
//! ```jsonc
//! {
//!   "university": "...", "school": null, "department": "...",
//!   "degree": "Bachelor's of Science Computer Science",
//!   "ai_program": "Minor" | "Major" | "Concentration" | "Certificate" | null,
//!   "curriculum_link": "...", "avg_classes_per_term": "5", "verified": true,
//!   "courses": {
//!     "cs_course_core": [ <course>, ... ],
//!     "ai_program_required_courses": [ ... ],
//!     "ai_program_unrestricted_electives": [ ... ],
//!     "unrestricted_elective": [ ... ]
//!   }
//! }
//! ```
//!
//! Each course carries `prerequisites` as `Vec<Vec<String>>` where the **outer
//! list is AND** and the **inner list is OR** (the opposite of our DNF), and an
//! inner `"X and Y"` string is itself an AND. We flip that into our
//! [`PrereqExpr`] (`All`/`Any`/`Course`) tree at the boundary.
//!
//! This converter is transitional: once ai-landscape emits the unified JSON
//! directly, it can be retired.

use std::collections::HashMap;

use serde::{Deserialize, Deserializer};

use crate::core::models::degree::{FromClause, Requirement, RequirementType};
use crate::core::models::{Course, Degree, DegreeProgram};
use crate::core::prerequisite_parser::PrereqExpr;

/// Deserialize a string field, mapping an explicit JSON `null` (which some
/// source files use for a missing title/code) to an empty string. `#[serde(default)]`
/// alone only covers absent keys, not an explicit `null`.
fn string_or_null<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

/// Default credit hours assumed when a source course is missing `course_hours`.
const DEFAULT_CREDIT_HOURS: f32 = 3.0;

// ai-landscape cluster pipeline-stage keys.
const STAGE_VERIFIER: &str = "course_verifier";
const STAGE_SCRAPER: &str = "course_scraper";
const STAGE_RESULTS: &str = "results";

// ai-landscape category names (their fixed vocabulary).
const CAT_CS_CORE: &str = "cs_course_core";
const CAT_AI_REQUIRED: &str = "ai_program_required_courses";
const CAT_AI_ELECTIVE: &str = "ai_program_unrestricted_electives";
const CAT_UNRESTRICTED: &str = "unrestricted_elective";

// Unified classification tag vocabulary (program-agnostic).
const TAG_AI: &str = "ai";
const TAG_CORE: &str = "core";
const TAG_ELECTIVE: &str = "elective";
const TAG_REQUIRED: &str = "required";
const TAG_OTHER: &str = "other";

// Unified `Degree`/`Requirement` category values.
const DEGREE_CAT_MAJOR: &str = "major";
const DEGREE_CAT_ELECTIVE: &str = "elective";

// ---------------------------------------------------------------------------
// Transitional input structs (ai-landscape shape)
// ---------------------------------------------------------------------------

/// Top-level ai-landscape program record.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct LandscapeProgram {
    /// Institution name.
    #[serde(default)]
    pub university: String,
    /// School/college within the institution, if recorded.
    #[serde(default)]
    pub school: Option<String>,
    /// Department, if recorded.
    #[serde(default)]
    pub department: Option<String>,
    /// Free-text degree title (e.g. "Bachelor's of Science Computer Science").
    #[serde(default)]
    pub degree: String,
    /// AI program flavor (`Major`/`Minor`/`Concentration`/…) or null.
    #[serde(default)]
    pub ai_program: Option<String>,
    /// Catalog/curriculum source URL.
    #[serde(default)]
    pub curriculum_link: Option<String>,
    /// Category name -> list of courses. Category names are not fixed in code
    /// so unknown categories degrade gracefully.
    #[serde(default)]
    pub courses: HashMap<String, Vec<LandscapeCourse>>,
}

/// A single course inside an ai-landscape category list.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct LandscapeCourse {
    /// Source course code (e.g. "CSCE 3201").
    #[serde(default, deserialize_with = "string_or_null")]
    pub course_code: String,
    /// Course title.
    #[serde(default, deserialize_with = "string_or_null")]
    pub title: String,
    /// Credit hours (numeric string, occasionally missing).
    #[serde(default)]
    pub course_hours: Option<Hours>,
    /// Catalog URL for the course.
    #[serde(default)]
    pub course_url: Option<String>,
    /// Picklist tags (`"Name [N]"` = choose N from group `Name`).
    #[serde(default)]
    pub picklist: Vec<String>,
    /// Prerequisites: outer list = AND, inner list = OR (their convention).
    #[serde(default)]
    pub prerequisites: Vec<Vec<String>>,
    /// Corequisites in the same AND-of-OR shape.
    #[serde(default)]
    pub corequisites: Vec<Vec<String>>,
    /// Strict corequisites (must be taken together) in the same shape.
    #[serde(default)]
    pub strict_corequisites: Vec<Vec<String>>,
}

/// `course_hours` arrives as a numeric string ("3") but tolerate raw numbers.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(untagged)]
pub enum Hours {
    /// Numeric string form, e.g. `"3"`.
    Str(String),
    /// Raw JSON number form.
    Num(f64),
}

impl Hours {
    fn as_credits(&self) -> Option<f32> {
        match self {
            #[allow(clippy::cast_possible_truncation)]
            Self::Num(n) => Some(*n as f32),
            Self::Str(s) => {
                let t = s.trim();
                if t.is_empty() {
                    None
                } else {
                    t.parse::<f32>().ok()
                }
            }
        }
    }
}

/// Result of a conversion: the unified program plus any data-quality warnings.
#[derive(Debug, Clone)]
pub struct ConversionResult {
    /// The converted unified program.
    pub program: DegreeProgram,
    /// Data-quality warnings raised during conversion.
    pub warnings: Vec<String>,
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Convert an ai-landscape JSON string into a unified [`DegreeProgram`].
///
/// # Errors
/// Returns an error string if the JSON does not match the ai-landscape shape.
pub fn convert_landscape_str(json: &str) -> Result<ConversionResult, String> {
    let program: LandscapeProgram = serde_json::from_str(json)
        .map_err(|e| format!("Failed to parse ai-landscape JSON: {e}"))?;
    Ok(convert_landscape(&program))
}

/// Extract convertible programs from an ai-landscape *cluster* pipeline file.
///
/// Each program's data lives at `course_verifier.<program>.results` or
/// `course_scraper.<program>.results`; the verifier stage is preferred, falling
/// back to the scraper. Programs whose results are absent, unparseable, or have
/// no courses are skipped. Returns `None` if the value is not a cluster file
/// (no `course_*` stage).
#[must_use]
pub fn extract_cluster_programs(
    value: &serde_json::Value,
) -> Option<Vec<(String, LandscapeProgram)>> {
    let verifier = value
        .get(STAGE_VERIFIER)
        .and_then(serde_json::Value::as_object);
    let scraper = value
        .get(STAGE_SCRAPER)
        .and_then(serde_json::Value::as_object);
    if verifier.is_none() && scraper.is_none() {
        return None;
    }

    // Deterministic, de-duplicated program order across both stages.
    let mut names: Vec<&String> = verifier
        .into_iter()
        .chain(scraper)
        .flat_map(serde_json::Map::keys)
        .collect();
    names.sort();
    names.dedup();

    let mut programs = Vec::new();
    for name in names {
        if let Some(prog) = stage_program(verifier, name).or_else(|| stage_program(scraper, name)) {
            programs.push((name.clone(), prog));
        }
    }
    Some(programs)
}

/// Deserialize `<stage>.<name>.results` into a [`LandscapeProgram`], returning
/// `None` unless it parses and has at least one non-empty course category.
fn stage_program(
    stage: Option<&serde_json::Map<String, serde_json::Value>>,
    name: &str,
) -> Option<LandscapeProgram> {
    let results = stage?.get(name)?.get(STAGE_RESULTS)?;
    let program: LandscapeProgram = serde_json::from_value(results.clone()).ok()?;
    program
        .courses
        .values()
        .any(|list| !list.is_empty())
        .then_some(program)
}

/// Canonical category ordering so requirement IDs / output are deterministic.
const KNOWN_CATEGORY_ORDER: [&str; 4] = [
    CAT_CS_CORE,
    CAT_AI_REQUIRED,
    CAT_AI_ELECTIVE,
    CAT_UNRESTRICTED,
];

/// Convert a parsed [`LandscapeProgram`] into a unified [`DegreeProgram`].
#[must_use]
pub fn convert_landscape(src: &LandscapeProgram) -> ConversionResult {
    let mut warnings = Vec::new();
    let mut courses: HashMap<String, Course> = HashMap::new();

    // picklist name -> (count, member course keys, is_ai)
    let mut picklists: HashMap<String, PicklistAcc> = HashMap::new();
    // category -> non-picklisted course keys (for the "all" requirement)
    let mut category_required: Vec<(String, Vec<String>)> = Vec::new();

    for category in ordered_categories(src) {
        let meta = category_meta(&category);
        let Some(list) = src.courses.get(&category) else {
            continue;
        };
        let mut plain_keys: Vec<String> = Vec::new();
        for lc in list {
            classify_course(
                lc,
                &meta,
                &mut courses,
                &mut picklists,
                &mut plain_keys,
                &mut warnings,
            );
        }
        if !plain_keys.is_empty() {
            category_required.push((category.clone(), plain_keys));
        }
    }

    let requirements = assemble_requirements(category_required, picklists);
    let degree = build_degree(src);

    ConversionResult {
        program: DegreeProgram {
            degree,
            requirements,
            courses,
        },
        warnings,
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct PicklistAcc {
    count: u32,
    keys: Vec<String>,
    is_ai: bool,
}

/// Process one source course within a category: create/merge its unified course
/// entry, then record it as a plain required course or as picklist member(s).
fn classify_course(
    lc: &LandscapeCourse,
    meta: &CategoryMeta,
    courses: &mut HashMap<String, Course>,
    picklists: &mut HashMap<String, PicklistAcc>,
    plain_keys: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    let key = normalize_key(&lc.course_code);
    if key.is_empty() {
        return;
    }
    build_or_merge_course(courses, &key, lc, meta.course_ai, warnings);

    if lc.picklist.is_empty() {
        if !plain_keys.contains(&key) {
            plain_keys.push(key);
        }
        return;
    }
    for entry in &lc.picklist {
        record_picklist(entry, &key, meta.course_ai, picklists, warnings);
    }
}

/// Record one `"Name [N]"` picklist membership for `key`, accumulating the
/// choose-N count and AI flag for that group.
fn record_picklist(
    entry: &str,
    key: &str,
    is_ai: bool,
    picklists: &mut HashMap<String, PicklistAcc>,
    warnings: &mut Vec<String>,
) {
    let Some((name, count)) = parse_picklist(entry) else {
        warnings.push(format!("Unparseable picklist tag `{entry}` on {key}"));
        return;
    };
    let acc = picklists.entry(name).or_insert_with(|| PicklistAcc {
        count,
        keys: Vec::new(),
        is_ai: false,
    });
    acc.count = acc.count.max(count);
    acc.is_ai |= is_ai;
    if !acc.keys.iter().any(|k| k == key) {
        acc.keys.push(key.to_string());
    }
}

/// Assemble requirements: one `all` requirement per category (for its
/// non-picklisted courses) and one `select` requirement per picklist group.
fn assemble_requirements(
    category_required: Vec<(String, Vec<String>)>,
    picklists: HashMap<String, PicklistAcc>,
) -> HashMap<String, Requirement> {
    let mut requirements: HashMap<String, Requirement> = HashMap::new();

    for (category, keys) in category_required {
        let meta = category_meta(&category);
        requirements.insert(
            category.clone(),
            Requirement {
                name: Some(meta.display.to_string()),
                req_type: RequirementType::All,
                category: Some(meta.degree_category.to_string()),
                courses: Some(keys),
                from: None,
                count: None,
                credits: None,
                credit_range: None,
                constraints: None,
                options: None,
                tags: Some(meta.req_tags.iter().map(|s| (*s).to_string()).collect()),
            },
        );
    }

    for (name, acc) in picklists {
        let mut tags = vec![TAG_ELECTIVE.to_string()];
        if acc.is_ai {
            tags.insert(0, TAG_AI.to_string());
        }
        requirements.insert(
            format!("picklist_{}", slug(&name)),
            Requirement {
                name: Some(name),
                req_type: RequirementType::Select,
                category: Some(DEGREE_CAT_ELECTIVE.to_string()),
                courses: None,
                from: Some(FromClause {
                    courses: Some(acc.keys),
                    pattern: None,
                    include: None,
                    exclude: None,
                    groups: None,
                    groups_required: None,
                    per_group: None,
                }),
                count: Some(acc.count),
                credits: None,
                credit_range: None,
                constraints: None,
                options: None,
                tags: Some(tags),
            },
        );
    }

    requirements
}

struct CategoryMeta {
    req_tags: &'static [&'static str],
    course_ai: bool,
    degree_category: &'static str,
    display: &'static str,
}

fn category_meta(category: &str) -> CategoryMeta {
    match category {
        CAT_CS_CORE => CategoryMeta {
            req_tags: &[TAG_CORE],
            course_ai: false,
            degree_category: DEGREE_CAT_MAJOR,
            display: "Core Courses",
        },
        CAT_AI_REQUIRED => CategoryMeta {
            req_tags: &[TAG_AI, TAG_REQUIRED],
            course_ai: true,
            degree_category: DEGREE_CAT_MAJOR,
            display: "AI Required Courses",
        },
        CAT_AI_ELECTIVE => CategoryMeta {
            req_tags: &[TAG_AI, TAG_ELECTIVE],
            course_ai: true,
            degree_category: DEGREE_CAT_ELECTIVE,
            display: "AI Electives",
        },
        CAT_UNRESTRICTED => CategoryMeta {
            req_tags: &[TAG_ELECTIVE],
            course_ai: false,
            degree_category: DEGREE_CAT_ELECTIVE,
            display: "Unrestricted Electives",
        },
        _ => CategoryMeta {
            req_tags: &[TAG_OTHER],
            course_ai: false,
            degree_category: DEGREE_CAT_MAJOR,
            display: "Other Courses",
        },
    }
}

/// Known categories first (in canonical order), then any extras the file
/// happens to contain, sorted for determinism.
fn ordered_categories(src: &LandscapeProgram) -> Vec<String> {
    let mut out: Vec<String> = KNOWN_CATEGORY_ORDER
        .iter()
        .filter(|c| src.courses.contains_key(**c))
        .map(|c| (*c).to_string())
        .collect();
    let mut extras: Vec<String> = src
        .courses
        .keys()
        .filter(|k| !KNOWN_CATEGORY_ORDER.contains(&k.as_str()))
        .cloned()
        .collect();
    extras.sort();
    out.extend(extras);
    out
}

fn build_or_merge_course(
    courses: &mut HashMap<String, Course>,
    key: &str,
    lc: &LandscapeCourse,
    is_ai: bool,
    warnings: &mut Vec<String>,
) {
    if let Some(existing) = courses.get_mut(key) {
        // Cross-category duplicate: just union the ai tag.
        if is_ai {
            existing.add_tag(TAG_AI);
        }
        return;
    }

    let (prefix, number) = split_code(&lc.course_code);
    let credits = lc.course_hours.as_ref().and_then(Hours::as_credits);
    let credit_hours = credits.unwrap_or_else(|| {
        warnings.push(format!(
            "Course {key} missing course_hours; assuming {DEFAULT_CREDIT_HOURS} credits"
        ));
        DEFAULT_CREDIT_HOURS
    });

    let prereq_expr = convert_groups_and_of_or(&lc.prerequisites);
    let prerequisites_raw = prereq_expr.map(|e| e.to_expression_string());

    let mut course = Course::new(lc.title.clone(), prefix, number, credit_hours);
    course.prerequisites_raw = prerequisites_raw;
    course.corequisites = flatten_coreqs(&lc.corequisites, key, "corequisite", warnings);
    course.strict_corequisites =
        flatten_coreqs(&lc.strict_corequisites, key, "strict corequisite", warnings);
    if is_ai {
        course.add_tag(TAG_AI);
    }
    courses.insert(key.to_string(), course);
}

/// Their corequisites are `Vec<Vec<String>>` (AND-of-OR) but our model stores a
/// flat required list. Take the first alternative of each OR group; warn when an
/// alternative is dropped.
fn flatten_coreqs(
    groups: &[Vec<String>],
    key: &str,
    label: &str,
    warnings: &mut Vec<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    for group in groups {
        if group.len() > 1 {
            warnings.push(format!(
                "{key}: collapsed a {label} choice ({} alternatives) to the first option",
                group.len()
            ));
        }
        if let Some(first) = group.first() {
            for code in split_on_and(first) {
                let k = normalize_key(&code);
                if !k.is_empty() && !out.contains(&k) {
                    out.push(k);
                }
            }
        }
    }
    out
}

/// Convert ai-landscape `Vec<Vec<String>>` (outer AND, inner OR, inner `"X and Y"`
/// = nested AND) into our [`PrereqExpr`]. Returns `None` for empty input.
#[must_use]
pub fn convert_groups_and_of_or(groups: &[Vec<String>]) -> Option<PrereqExpr> {
    let parts: Vec<PrereqExpr> = groups.iter().filter_map(|g| or_group_to_expr(g)).collect();
    collapse(parts, true)
}

fn or_group_to_expr(group: &[String]) -> Option<PrereqExpr> {
    let alts: Vec<PrereqExpr> = group
        .iter()
        .filter_map(|item| {
            let leaves: Vec<PrereqExpr> = split_on_and(item)
                .into_iter()
                .filter_map(|code| {
                    let k = normalize_key(&code);
                    if k.is_empty() {
                        None
                    } else {
                        Some(PrereqExpr::Course(k))
                    }
                })
                .collect();
            collapse(leaves, true)
        })
        .collect();
    collapse(alts, false)
}

/// Collapse a child list into `All` (`and = true`) or `Any` (`and = false`),
/// flattening the single-child and empty cases.
fn collapse(mut parts: Vec<PrereqExpr>, and: bool) -> Option<PrereqExpr> {
    match parts.len() {
        0 => None,
        1 => parts.pop(),
        _ => Some(if and {
            PrereqExpr::All(parts)
        } else {
            PrereqExpr::Any(parts)
        }),
    }
}

/// Split a course-code string on the word "and" (case-insensitive, whitespace
/// delimited), e.g. `"MAC 2312 and MAD 2104"` -> `["MAC 2312", "MAD 2104"]`.
fn split_on_and(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = s.trim();
    loop {
        let lower = rest.to_lowercase();
        if let Some(pos) = lower.find(" and ") {
            out.push(rest[..pos].trim().to_string());
            rest = rest[pos + 5..].trim_start();
        } else {
            let t = rest.trim();
            if !t.is_empty() {
                out.push(t.to_string());
            }
            break;
        }
    }
    out
}

/// Normalize a course code to a graph key: split into prefix+number, drop
/// parser-hostile characters (`()`, grade `[..]`, whitespace) so the key is safe
/// inside a boolean prerequisite string.
fn normalize_key(code: &str) -> String {
    let (prefix, number) = split_code(code);
    format!("{prefix}{number}")
}

/// Split a raw course code into `(prefix, number)`, stripping `()`, `[..]` and
/// collapsing whitespace. e.g. `"CSCE 3201"` -> `("CSCE","3201")`,
/// `"M 362K"` -> `("M","362K")`, `"COT3100"` -> `("COT","3100")`.
#[allow(clippy::option_if_let_else)]
fn split_code(code: &str) -> (String, String) {
    // Strip grade brackets and parentheses (e.g. "CS 429(H)" -> "CS 429H").
    let mut cleaned = String::with_capacity(code.len());
    let mut in_bracket = false;
    for c in code.chars() {
        match c {
            '[' => in_bracket = true,
            ']' => in_bracket = false,
            '(' | ')' => {}
            _ if !in_bracket => cleaned.push(c),
            _ => {}
        }
    }
    let cleaned = cleaned.trim();

    if let Some(idx) = cleaned.find(char::is_whitespace) {
        let prefix = cleaned[..idx].trim().to_string();
        let number: String = cleaned[idx..].split_whitespace().collect();
        (prefix, number)
    } else if let Some(idx) = cleaned.find(|c: char| c.is_ascii_digit()) {
        (cleaned[..idx].to_string(), cleaned[idx..].to_string())
    } else {
        (cleaned.to_string(), String::new())
    }
}

/// Parse a picklist tag `"Name [N]"` (spacing is inconsistent, e.g.
/// `"Electives[4]"`) into `(name, count)`.
fn parse_picklist(entry: &str) -> Option<(String, u32)> {
    let open = entry.find('[')?;
    let close = entry[open..].find(']')? + open;
    let name = entry[..open].trim();
    let count: u32 = entry[open + 1..close].trim().parse().ok()?;
    if name.is_empty() {
        None
    } else {
        Some((name.to_string(), count))
    }
}

fn build_degree(src: &LandscapeProgram) -> Degree {
    let mut degree = Degree::new(
        if src.degree.is_empty() {
            "Computer Science".to_string()
        } else {
            src.degree.clone()
        },
        infer_degree_type(&src.degree),
        None,
        "semester".to_string(),
    );
    if !src.university.is_empty() {
        degree.institution = Some(src.university.clone());
    }
    degree.source_url.clone_from(&src.curriculum_link);
    degree.id = Some(slug(&format!(
        "{}-{}-{}",
        src.university,
        src.degree,
        src.ai_program.as_deref().unwrap_or("program")
    )));

    // Program-level tags generalize the ai_program enum.
    if let Some(flavor) = ai_flavor(src.ai_program.as_deref()) {
        let mut tags = vec!["ai".to_string()];
        tags.push(format!("ai-{}", slug(flavor)));
        degree.tags = Some(tags);
    }
    degree
}

/// Returns the `ai_program` flavor when it represents a real AI program.
fn ai_flavor(ai_program: Option<&str>) -> Option<&str> {
    match ai_program.map(str::trim) {
        Some(s)
            if !s.is_empty()
                && !s.eq_ignore_ascii_case("null")
                && !s.eq_ignore_ascii_case("none") =>
        {
            Some(s)
        }
        _ => None,
    }
}

fn infer_degree_type(degree: &str) -> String {
    let d = degree.to_lowercase();
    if d.contains("bachelor") && (d.contains("art") || d.contains(" ba")) {
        "BA".to_string()
    } else if d.contains("master") {
        "MS".to_string()
    } else {
        "BS".to_string()
    }
}

/// kebab-case slug from arbitrary text.
fn slug(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_dash = false;
    for c in s.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash && !out.is_empty() {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_cluster_programs_falls_back_to_scraper_only() {
        // Program lives only under course_scraper: verifier branch is None, so
        // the scraper branch supplies it.
        let value: serde_json::Value = serde_json::from_str(
            r#"{
            "course_scraper": {
                "DS BS": {"results": {"university":"U","degree":"BS DS","courses":{
                    "cs_course_core":[{"course_code":"DS 200","title":"Data","course_hours":"3","prerequisites":[]}]}}}
            },
            "course_verifier": {}
        }"#,
        )
        .unwrap();
        let progs = extract_cluster_programs(&value).unwrap();
        assert_eq!(progs.len(), 1);
        assert_eq!(progs[0].0, "DS BS");
        assert!(convert_landscape(&progs[0].1)
            .program
            .courses
            .contains_key("DS200"));
    }

    #[test]
    fn test_extract_cluster_programs_skips_missing_unparseable_and_empty() {
        // NoResults: no `results` key. BadResults: `results` not a program object.
        // EmptyCats: parses but no courses. Good: survives.
        let value: serde_json::Value = serde_json::from_str(
            r#"{
            "course_verifier": {
                "NoResults":  {"status": "ok"},
                "BadResults": {"results": 42},
                "EmptyCats":  {"results": {"university":"U","degree":"X","courses":{"cs_course_core":[]}}},
                "Good":       {"results": {"university":"U","degree":"BS","courses":{
                    "cs_course_core":[{"course_code":"CS 101","title":"Intro","course_hours":"3","prerequisites":[]}]}}}
            }
        }"#,
        )
        .unwrap();
        let progs = extract_cluster_programs(&value).unwrap();
        assert_eq!(progs.len(), 1, "only `Good` has non-empty courses");
        assert_eq!(progs[0].0, "Good");
    }

    #[test]
    fn test_extract_cluster_programs_stage_not_object_is_none() {
        // `course_verifier` present but not an object (and no scraper) -> non-cluster.
        let value: serde_json::Value =
            serde_json::from_str(r#"{"course_verifier": "running"}"#).unwrap();
        assert!(extract_cluster_programs(&value).is_none());
    }

    #[test]
    fn test_extract_cluster_programs_prefers_verifier_and_skips_empty() {
        let value: serde_json::Value = serde_json::from_str(
            r#"{
            "university": "Test U",
            "course_scraper": {
                "CS BS": {"results": {"university":"Test U","degree":"BS CS","courses":{
                    "cs_course_core":[{"course_code":"CS 101","title":"Intro","course_hours":"4","prerequisites":[]}]}}},
                "Empty Prog": {"results": {"university":"Test U","degree":"X","courses":{"cs_course_core":[]}}}
            },
            "course_verifier": {
                "CS BS": {"results": {"university":"Test U","degree":"BS CS","courses":{
                    "cs_course_core":[
                        {"course_code":"CS 101","title":"Intro V","course_hours":"4","prerequisites":[]},
                        {"course_code":"CS 201","title":"DS","course_hours":"4","prerequisites":[["CS 101"]]}]}}}
            }
        }"#,
        )
        .unwrap();

        let progs = extract_cluster_programs(&value).unwrap();
        // "CS BS" resolves from the verifier (2 courses); "Empty Prog" is skipped.
        assert_eq!(progs.len(), 1);
        assert_eq!(progs[0].0, "CS BS");
        let conv = convert_landscape(&progs[0].1);
        assert!(conv.program.courses.contains_key("CS201"));
    }

    #[test]
    fn test_extract_cluster_programs_none_for_non_cluster() {
        let value: serde_json::Value =
            serde_json::from_str(r#"{"courses":{"cs_course_core":[]}}"#).unwrap();
        assert!(extract_cluster_programs(&value).is_none());
    }

    #[test]
    fn test_convert_tolerates_null_title_and_code() {
        // Real source files sometimes carry `"title": null` / `"course_code": null`.
        let json = r#"{
            "university": "U", "degree": "BS CS",
            "courses": { "cs_course_core": [
                {"course_code":"CS 101","title":null,"course_hours":null,"prerequisites":[]},
                {"course_code":null,"title":"Ghost","prerequisites":[]}
            ]}
        }"#;
        let result = convert_landscape_str(json).expect("null fields must not fail conversion");
        // Null title -> empty name; null code -> dropped (empty key).
        assert!(result.program.courses.contains_key("CS101"));
        assert_eq!(result.program.courses["CS101"].name, "");
        assert_eq!(result.program.courses.len(), 1);
    }

    #[test]
    fn test_split_on_and_variants() {
        // Case-insensitive " and ", trims, drops empties; no-and returns whole.
        assert_eq!(
            split_on_and("MAC 2312 and MAD 2104"),
            vec!["MAC 2312", "MAD 2104"]
        );
        assert_eq!(split_on_and("A aNd B AND C"), vec!["A", "B", "C"]);
        assert_eq!(split_on_and("CS 101"), vec!["CS 101"]);
        assert_eq!(split_on_and("  CS 101  "), vec!["CS 101"]);
        assert!(split_on_and("").is_empty());
        assert!(split_on_and("   ").is_empty());
        // "and" inside a token (no surrounding spaces) is not a split point.
        assert_eq!(split_on_and("ANDREW 100"), vec!["ANDREW 100"]);
    }

    #[test]
    fn test_flatten_coreqs_first_alternative_split_and_dedup() {
        let mut warnings = Vec::new();
        // Group 1: OR with 2 alternatives -> keep first, warn.
        // Group 2: single "X and Y" -> split into two keys.
        let groups = vec![
            vec!["CS 101".to_string(), "CS 102".to_string()],
            vec!["MA 100 and MA 200".to_string()],
        ];
        let out = flatten_coreqs(&groups, "CS500", "corequisite", &mut warnings);
        assert_eq!(out, vec!["CS101", "MA100", "MA200"]);
        assert!(warnings
            .iter()
            .any(|w| w.contains("CS500") && w.contains("2 alternatives")));

        // Empty input -> empty, no warning. Duplicates collapse.
        let mut w2 = Vec::new();
        assert!(flatten_coreqs(&[], "X", "corequisite", &mut w2).is_empty());
        assert!(w2.is_empty());
        let dup = vec![vec!["CS 101".to_string()], vec!["CS 101".to_string()]];
        assert_eq!(
            flatten_coreqs(&dup, "X", "corequisite", &mut w2),
            vec!["CS101"]
        );
    }

    #[test]
    fn test_ai_flavor_recognizes_real_and_rejects_sentinels() {
        assert_eq!(ai_flavor(Some("Minor")), Some("Minor"));
        assert_eq!(ai_flavor(Some("  Concentration  ")), Some("Concentration"));
        assert_eq!(ai_flavor(None), None);
        assert_eq!(ai_flavor(Some("")), None);
        assert_eq!(ai_flavor(Some("   ")), None);
        assert_eq!(ai_flavor(Some("null")), None);
        assert_eq!(ai_flavor(Some("NONE")), None);
    }

    #[test]
    fn test_hours_as_credits_str_num_and_garbage() {
        assert_eq!(Hours::Num(3.0).as_credits(), Some(3.0));
        assert_eq!(Hours::Str("4".to_string()).as_credits(), Some(4.0));
        assert_eq!(Hours::Str(" 3.5 ".to_string()).as_credits(), Some(3.5));
        assert_eq!(Hours::Str(String::new()).as_credits(), None);
        assert_eq!(Hours::Str("   ".to_string()).as_credits(), None);
        assert_eq!(Hours::Str("three".to_string()).as_credits(), None);
    }

    #[test]
    fn test_slug_and_infer_degree_type() {
        assert_eq!(slug("AI  Minor!"), "ai-minor");
        assert_eq!(slug("  --x--  "), "x");
        assert_eq!(slug(""), "");

        assert_eq!(infer_degree_type("Bachelor of Arts in CS"), "BA");
        assert_eq!(infer_degree_type("Master of Science"), "MS");
        assert_eq!(
            infer_degree_type("Bachelor's of Science Computer Science"),
            "BS"
        );
        assert_eq!(infer_degree_type(""), "BS");
    }

    #[test]
    fn test_prereq_flip_and_of_or() {
        // their: outer AND, inner OR -> (CS429H|CS429) & (M362K|SDS321)
        let groups = vec![
            vec!["CS 429(H)".to_string(), "CS 429".to_string()],
            vec!["M 362K".to_string(), "SDS 321".to_string()],
        ];
        let expr = convert_groups_and_of_or(&groups).unwrap();
        // Top level must be AND (All)
        assert!(matches!(expr, PrereqExpr::All(_)));
        let json = serde_json::to_string(&expr).unwrap();
        assert_eq!(
            json,
            r#"{"and":[{"or":["CS429H","CS429"]},{"or":["M362K","SDS321"]}]}"#
        );
    }

    #[test]
    fn test_inner_and_inside_or() {
        // [["MAC 2312 and MAD 2104", "MAC 2313"]] -> (MAC2312 & MAD2104) | MAC2313
        let groups = vec![vec![
            "MAC 2312 and MAD 2104".to_string(),
            "MAC 2313".to_string(),
        ]];
        let expr = convert_groups_and_of_or(&groups).unwrap();
        let json = serde_json::to_string(&expr).unwrap();
        assert_eq!(json, r#"{"or":[{"and":["MAC2312","MAD2104"]},"MAC2313"]}"#);
    }

    #[test]
    fn test_single_group_single_course() {
        let groups = vec![vec!["MA 26100".to_string()]];
        let expr = convert_groups_and_of_or(&groups).unwrap();
        assert_eq!(expr, PrereqExpr::Course("MA26100".to_string()));
    }

    #[test]
    fn test_split_code_variants() {
        assert_eq!(split_code("CSCE 3201"), ("CSCE".into(), "3201".into()));
        assert_eq!(split_code("M 362K"), ("M".into(), "362K".into()));
        assert_eq!(split_code("COT3100"), ("COT".into(), "3100".into()));
        assert_eq!(split_code("CS 429(H)"), ("CS".into(), "429H".into()));
    }

    #[test]
    fn test_parse_picklist() {
        assert_eq!(
            parse_picklist("Foundations [1]"),
            Some(("Foundations".into(), 1))
        );
        assert_eq!(
            parse_picklist("Electives[4]"),
            Some(("Electives".into(), 4))
        );
        assert_eq!(parse_picklist("no brackets"), None);
    }

    #[test]
    fn test_full_conversion_dedup_and_credits() {
        let json = r#"{
            "university": "Test U",
            "degree": "Bachelor's of Science Computer Science",
            "ai_program": "Minor",
            "courses": {
                "cs_course_core": [
                    {"course_code":"CS 101","title":"Intro","course_hours":"4","prerequisites":[]},
                    {"course_code":"CS 201","title":"DS","prerequisites":[["CS 101"]]}
                ],
                "unrestricted_elective": [
                    {"course_code":"CS 101","title":"Intro","course_hours":"4","prerequisites":[]}
                ],
                "ai_program_required_courses": [
                    {"course_code":"CS 440","title":"AI","course_hours":"3","picklist":["AI Pick [1]"],"prerequisites":[["CS 201"]]}
                ]
            }
        }"#;
        let result = convert_landscape_str(json).unwrap();
        let p = &result.program;

        // CS101 appears in two categories but collapses to one course entry.
        assert!(p.courses.contains_key("CS101"));
        assert_eq!(p.courses.len(), 3);

        // CS201 missing course_hours -> defaulted + warning.
        assert!((p.courses["CS201"].credit_hours - DEFAULT_CREDIT_HOURS).abs() < f32::EPSILON);
        assert!(result.warnings.iter().any(|w| w.contains("CS201")));

        // AI course tagged + picklist became a select requirement.
        assert_eq!(
            p.courses["CS440"].tags.as_deref(),
            Some(&["ai".to_string()][..])
        );
        assert!(p
            .requirements
            .values()
            .any(|r| r.req_type == RequirementType::Select));

        // Program tagged as AI minor.
        let tags = p.degree.tags.as_ref().unwrap();
        assert!(tags.contains(&"ai".to_string()));
        assert!(tags.contains(&"ai-minor".to_string()));
    }
}
