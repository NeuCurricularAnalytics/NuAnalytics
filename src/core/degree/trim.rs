//! Degree trim.
//!
//! Collapses alternative prerequisites and selection lists down to a
//! single shared shortest entry path per course, except for courses tied
//! to the major itself (and any user-protected subjects).
//!
//! See `nuanalytics degree trim --help` for user-facing semantics.

use std::collections::{HashMap, HashSet};

use crate::core::degree::{FromClause, Requirement, RequirementType};
use crate::core::models::DegreeProgram;
use crate::core::prerequisite_parser::{extract_all_courses, parse_to_ast, PrereqExpr};

/// Options governing how [`trim_program`] rewrites a degree.
#[derive(Debug, Clone, Default)]
pub struct TrimOptions {
    /// Subject prefixes whose alternatives are preserved even when the
    /// degree doesn't declare them as major subjects. Case-insensitive
    /// (canonicalised to uppercase internally).
    pub keep_all_subjects: HashSet<String>,
    /// Course keys to pin as winners at any choice point listing them.
    /// Overrides both the shortest-prereq-depth metric and the
    /// prefer-protected rule for the disjunct/list that contains them.
    pub include_courses: HashSet<String>,
}

/// Summary of what [`trim_program`] did. Returned alongside the trimmed
/// program for caller-side reporting.
#[derive(Debug, Clone, Default)]
pub struct TrimReport {
    /// Subject prefixes that were treated as protected.
    pub protected_subjects: Vec<String>,
    /// Whether `protected_subjects` was derived from requirement contents
    /// (true) or taken straight from `degree.major_subjects` (false).
    pub protected_subjects_derived: bool,
    /// Course keys removed from `program.courses` because nothing in the
    /// trimmed program still references them.
    pub orphan_courses_removed: Vec<String>,
}

/// Trim a degree program down to a single entry path per course.
///
/// See module docs and the user-facing `degree trim --help` text for the
/// full ruleset.
#[must_use]
pub fn trim_program(program: &DegreeProgram, opts: &TrimOptions) -> (DegreeProgram, TrimReport) {
    let (protected, derived) = build_protected_subjects(program, &opts.keep_all_subjects);
    let depths = build_prereq_depth_map(program);
    let include = &opts.include_courses;

    let mut out = program.clone();

    for course in out.courses.values_mut() {
        let Some(raw) = course.prerequisites_raw.as_ref() else {
            continue;
        };
        let Some(ast) = parse_to_ast(raw) else {
            continue;
        };
        let trimmed = trim_ast(ast, &protected, &depths, include);
        if trimmed.is_empty() {
            course.prerequisites_raw = None;
            course.prerequisites.clear();
        } else {
            let new_raw = trimmed.to_expression_string();
            course.prerequisites = unique_courses(&new_raw);
            course.prerequisites_raw = Some(new_raw);
        }
    }

    let credit_lookup = build_credit_lookup(&out);
    for req in out.requirements.values_mut() {
        trim_requirement_in_place(req, &protected, &depths, include, &credit_lookup);
    }

    let orphan_courses_removed = prune_orphan_courses(&mut out);

    let mut protected_sorted: Vec<String> = protected.into_iter().collect();
    protected_sorted.sort();
    (
        out,
        TrimReport {
            protected_subjects: protected_sorted,
            protected_subjects_derived: derived,
            orphan_courses_removed,
        },
    )
}

// ---------------------------------------------------------------------------
// Protected-subject derivation
// ---------------------------------------------------------------------------

/// Plurality threshold (in percent of the top subject's reference count)
/// for deriving the protected-subject set when `degree.major_subjects` is
/// absent. A subject counts as part of the major if its referenced-course
/// count is within this fraction of the top-ranked subject.
const PROTECTED_SUBJECT_THRESHOLD_PCT: u32 = 90;

fn build_protected_subjects(
    program: &DegreeProgram,
    extras: &HashSet<String>,
) -> (HashSet<String>, bool) {
    let mut set: HashSet<String> = extras.iter().map(|s| s.to_uppercase()).collect();

    if let Some(subjects) = &program.degree.major_subjects {
        for s in subjects {
            set.insert(s.to_uppercase());
        }
        return (set, false);
    }

    // Fallback: count subject prefixes across every course key referenced
    // in `requirements:` (top-level and nested). Treat any subject within
    // `PROTECTED_SUBJECT_THRESHOLD_PCT` of the top count as part of the
    // major. Integer arithmetic avoids f32 casts.
    let counts = count_subjects_in_requirements(&program.requirements);
    if let Some(&top) = counts.values().max() {
        let threshold = top.saturating_mul(PROTECTED_SUBJECT_THRESHOLD_PCT) / 100;
        for (subj, count) in &counts {
            if *count >= threshold {
                set.insert(subj.clone());
            }
        }
    }
    (set, true)
}

fn count_subjects_in_requirements(
    requirements: &HashMap<String, Requirement>,
) -> HashMap<String, u32> {
    let mut counts: HashMap<String, u32> = HashMap::new();
    for req in requirements.values() {
        walk_requirement_courses(req, &mut |key| {
            if let Some(subj) = subject_of(key) {
                *counts.entry(subj).or_insert(0) += 1;
            }
        });
    }
    counts
}

fn walk_requirement_courses<F: FnMut(&str)>(req: &Requirement, visit: &mut F) {
    if let Some(courses) = &req.courses {
        for entry in courses {
            for c in expand_course_entry(entry) {
                visit(&c);
            }
        }
    }
    if let Some(from) = &req.from {
        if let Some(courses) = &from.courses {
            for entry in courses {
                for c in expand_course_entry(entry) {
                    visit(&c);
                }
            }
        }
    }
    if let Some(options) = &req.options {
        for opt in options {
            for nested in &opt.requirements {
                walk_requirement_courses(nested, visit);
            }
        }
    }
}

/// Expand a `courses:` entry, which may be a bare key, a bundle
/// `"[A, B]"`, or an equivalents group `"{A, B}"`, into its constituent
/// course keys.
fn expand_course_entry(entry: &str) -> Vec<String> {
    let trimmed = entry.trim();
    if (trimmed.starts_with('[') && trimmed.ends_with(']'))
        || (trimmed.starts_with('{') && trimmed.ends_with('}'))
    {
        let inner = &trimmed[1..trimmed.len() - 1];
        return inner
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    vec![trimmed.to_string()]
}

/// Extract leading alphabetic subject prefix (e.g. `"CS"` from `"CS3000"`).
fn subject_of(key: &str) -> Option<String> {
    let trimmed = key.trim();
    let subj: String = trimmed.chars().take_while(|c| c.is_alphabetic()).collect();
    if subj.is_empty() {
        None
    } else {
        Some(subj.to_uppercase())
    }
}

fn is_protected(key: &str, protected: &HashSet<String>) -> bool {
    subject_of(key).is_some_and(|s| protected.contains(&s))
}

// ---------------------------------------------------------------------------
// Shortest-prereq-depth metric
// ---------------------------------------------------------------------------

/// Compute the minimum prerequisite depth for every course in `program`.
///
/// Depth semantics — strictly upstream of the course (the metric we use to
/// rank alternatives by "shortest path into the course"):
///
/// - A course with no prerequisites has depth `0`.
/// - For a single course `c`, depth(c) = 1 + depth(c.prerequisites).
/// - For an `All` (AND) prereq subexpression, the depth is the **max** of
///   its children — every branch must be satisfied.
/// - For an `Any` (OR) prereq subexpression, the depth is the **min** of
///   its children — only the easiest branch is needed.
///
/// The recursion is memoised; cycles (the validator should have caught
/// them earlier) and dangling references both collapse to depth `0`.
fn build_prereq_depth_map(program: &DegreeProgram) -> HashMap<String, usize> {
    let mut depths: HashMap<String, usize> = HashMap::new();
    let mut visiting: HashSet<String> = HashSet::new();
    for key in program.courses.keys() {
        let _ = course_depth(key, program, &mut depths, &mut visiting);
    }
    depths
}

fn course_depth(
    key: &str,
    program: &DegreeProgram,
    depths: &mut HashMap<String, usize>,
    visiting: &mut HashSet<String>,
) -> usize {
    if let Some(&cached) = depths.get(key) {
        return cached;
    }
    // Treat cycles and unknown courses as roots — depth 0 — so they don't
    // poison the rest of the computation.
    if visiting.contains(key) {
        return 0;
    }
    let Some(course) = program.courses.get(key) else {
        return 0;
    };
    let Some(raw) = course.prerequisites_raw.as_ref() else {
        depths.insert(key.to_string(), 0);
        return 0;
    };
    let Some(ast) = parse_to_ast(raw) else {
        depths.insert(key.to_string(), 0);
        return 0;
    };

    visiting.insert(key.to_string());
    let prereq_depth = ast_depth(&ast, program, depths, visiting);
    visiting.remove(key);

    let depth = prereq_depth + 1;
    depths.insert(key.to_string(), depth);
    depth
}

fn ast_depth(
    ast: &PrereqExpr,
    program: &DegreeProgram,
    depths: &mut HashMap<String, usize>,
    visiting: &mut HashSet<String>,
) -> usize {
    match ast {
        PrereqExpr::Course(c) => course_depth(c, program, depths, visiting),
        PrereqExpr::All(xs) => xs
            .iter()
            .map(|x| ast_depth(x, program, depths, visiting))
            .max()
            .unwrap_or(0),
        PrereqExpr::Any(xs) => xs
            .iter()
            .map(|x| ast_depth(x, program, depths, visiting))
            .min()
            .unwrap_or(0),
    }
}

// ---------------------------------------------------------------------------
// Prerequisite AST trim
// ---------------------------------------------------------------------------

fn trim_ast(
    ast: PrereqExpr,
    protected: &HashSet<String>,
    depths: &HashMap<String, usize>,
    include: &HashSet<String>,
) -> PrereqExpr {
    match ast {
        PrereqExpr::Course(c) => PrereqExpr::Course(c),
        PrereqExpr::All(xs) => collapse(PrereqExpr::All(
            xs.into_iter()
                .map(|x| trim_ast(x, protected, depths, include))
                .filter(|x| !x.is_empty())
                .collect(),
        )),
        PrereqExpr::Any(xs) => trim_disjunction(xs, protected, depths, include),
    }
}

fn trim_disjunction(
    xs: Vec<PrereqExpr>,
    protected: &HashSet<String>,
    depths: &HashMap<String, usize>,
    include: &HashSet<String>,
) -> PrereqExpr {
    // --include override: if any disjunct mentions a forced course, drop
    // all others before applying the protection and shortest-path rules.
    let forced: Vec<PrereqExpr> = xs
        .iter()
        .filter(|d| disjunct_mentions_any(d, include))
        .cloned()
        .collect();
    let candidates = if forced.is_empty() { xs } else { forced };

    // If every disjunct is wholly protected, keep them all.
    if candidates
        .iter()
        .all(|d| disjunct_all_protected(d, protected))
    {
        let trimmed: Vec<PrereqExpr> = candidates
            .into_iter()
            .map(|x| trim_ast(x, protected, depths, include))
            .filter(|x| !x.is_empty())
            .collect();
        return collapse(PrereqExpr::Any(trimmed));
    }

    // Mixed: keep the disjuncts that contain at least one protected course.
    let with_protected: Vec<PrereqExpr> = candidates
        .iter()
        .filter(|d| disjunct_has_protected(d, protected))
        .cloned()
        .collect();
    if !with_protected.is_empty() {
        let trimmed: Vec<PrereqExpr> = with_protected
            .into_iter()
            .map(|x| trim_ast(x, protected, depths, include))
            .filter(|x| !x.is_empty())
            .collect();
        return collapse(PrereqExpr::Any(trimmed));
    }

    // No protection — pick the smallest-depths disjunct deterministically.
    let chosen = candidates
        .into_iter()
        .min_by(|a, b| {
            disjunct_cost(a, depths)
                .cmp(&disjunct_cost(b, depths))
                .then_with(|| disjunct_sort_key(a).cmp(&disjunct_sort_key(b)))
        })
        .expect("disjunction was non-empty before filtering");
    trim_ast(chosen, protected, depths, include)
}

/// Collapse no-op wrappers: `All[x] -> x`, `Any[x] -> x`, empty -> empty Any.
fn collapse(expr: PrereqExpr) -> PrereqExpr {
    match expr {
        PrereqExpr::All(mut xs) | PrereqExpr::Any(mut xs) if xs.len() == 1 => xs.remove(0),
        other => other,
    }
}

fn disjunct_courses(expr: &PrereqExpr) -> Vec<String> {
    let mut out = Vec::new();
    walk_courses(expr, &mut out);
    out
}

fn walk_courses(e: &PrereqExpr, out: &mut Vec<String>) {
    match e {
        PrereqExpr::Course(c) => out.push(c.clone()),
        PrereqExpr::All(xs) | PrereqExpr::Any(xs) => {
            for x in xs {
                walk_courses(x, out);
            }
        }
    }
}

fn disjunct_all_protected(expr: &PrereqExpr, protected: &HashSet<String>) -> bool {
    let courses = disjunct_courses(expr);
    !courses.is_empty() && courses.iter().all(|c| is_protected(c, protected))
}

fn disjunct_has_protected(expr: &PrereqExpr, protected: &HashSet<String>) -> bool {
    disjunct_courses(expr)
        .iter()
        .any(|c| is_protected(c, protected))
}

fn disjunct_mentions_any(expr: &PrereqExpr, target: &HashSet<String>) -> bool {
    disjunct_courses(expr).iter().any(|c| target.contains(c))
}

/// Structural prereq depth carried along if this disjunct is chosen.
///
/// Mirrors [`ast_depth`] one level up — leaves use the precomputed
/// per-course depth map, `All` children take the max (every branch
/// required), `Any` children take the min (only the easiest branch
/// needed). Flattening into `disjunct_courses` is wrong here because it
/// loses the structural distinction between OR-alternatives and
/// AND-conjunctions inside the disjunct.
fn disjunct_cost(expr: &PrereqExpr, depths: &HashMap<String, usize>) -> usize {
    match expr {
        PrereqExpr::Course(c) => depths.get(c).copied().unwrap_or(0),
        PrereqExpr::All(xs) => xs
            .iter()
            .map(|x| disjunct_cost(x, depths))
            .max()
            .unwrap_or(0),
        PrereqExpr::Any(xs) => xs
            .iter()
            .map(|x| disjunct_cost(x, depths))
            .min()
            .unwrap_or(0),
    }
}

/// Lexicographic tiebreak — sorted concatenation of the disjunct's courses.
fn disjunct_sort_key(expr: &PrereqExpr) -> String {
    let mut keys = disjunct_courses(expr);
    keys.sort();
    keys.join("|")
}

// ---------------------------------------------------------------------------
// Requirement trim
// ---------------------------------------------------------------------------

fn trim_requirement_in_place(
    req: &mut Requirement,
    protected: &HashSet<String>,
    depths: &HashMap<String, usize>,
    include: &HashSet<String>,
    credits: &HashMap<String, f32>,
) {
    match req.req_type {
        RequirementType::All => {
            if let Some(courses) = req.courses.as_mut() {
                let new_courses: Vec<String> = courses
                    .iter()
                    .map(|entry| trim_all_entry(entry, protected, depths, include))
                    .filter(|entry| !entry.is_empty())
                    .collect();
                *courses = new_courses;
            }
        }
        RequirementType::Select => {
            if let Some(from) = req.from.as_mut() {
                trim_select_from(
                    from,
                    req.count,
                    req.credits,
                    protected,
                    depths,
                    include,
                    credits,
                );
            }
        }
        RequirementType::OneOf => {
            if let Some(options) = req.options.as_mut() {
                for opt in options {
                    for nested in &mut opt.requirements {
                        trim_requirement_in_place(nested, protected, depths, include, credits);
                    }
                }
            }
        }
    }
}

/// Handle a single entry in a `type: all` `courses:` array. Bundles
/// (`[A, B]`) pass through unchanged; equivalents (`{A, B}`) get trimmed
/// like a prerequisite disjunction.
fn trim_all_entry(
    entry: &str,
    protected: &HashSet<String>,
    depths: &HashMap<String, usize>,
    include: &HashSet<String>,
) -> String {
    let trimmed = entry.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        let inner = &trimmed[1..trimmed.len() - 1];
        let courses: Vec<String> = inner
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        let kept = pick_equivalents(&courses, protected, depths, include);
        return match kept.len() {
            0 => String::new(),
            1 => kept.into_iter().next().unwrap_or_default(),
            _ => format!("{{{}}}", kept.join(", ")),
        };
    }
    // Bundles `[A, B]` and bare keys are returned verbatim.
    trimmed.to_string()
}

fn pick_equivalents(
    courses: &[String],
    protected: &HashSet<String>,
    depths: &HashMap<String, usize>,
    include: &HashSet<String>,
) -> Vec<String> {
    let forced: Vec<String> = courses
        .iter()
        .filter(|c| include.contains(*c))
        .cloned()
        .collect();
    if !forced.is_empty() {
        return forced;
    }
    let protected_courses: Vec<String> = courses
        .iter()
        .filter(|c| is_protected(c, protected))
        .cloned()
        .collect();
    if !protected_courses.is_empty() {
        // If every member is protected, keep them all; if mixed, drop the
        // unprotected ones (prefer-protected rule).
        return protected_courses;
    }
    // None protected — pick smallest-depths course, lexicographic tiebreak.
    if let Some(pick) = courses.iter().min_by(|a, b| {
        depths
            .get(*a)
            .copied()
            .unwrap_or(0)
            .cmp(&depths.get(*b).copied().unwrap_or(0))
            .then_with(|| a.cmp(b))
    }) {
        return vec![pick.clone()];
    }
    Vec::new()
}

fn trim_select_from(
    from: &mut FromClause,
    count: Option<u32>,
    credits: Option<u32>,
    protected: &HashSet<String>,
    depths: &HashMap<String, usize>,
    include: &HashSet<String>,
    credit_lookup: &HashMap<String, f32>,
) {
    // Pattern- or group-based selections are out of scope for v1; leave
    // them as-is so the trimmed file still represents the same selection
    // space for those requirements.
    if from.pattern.is_some() || from.groups.is_some() {
        return;
    }
    let Some(courses) = from.courses.as_mut() else {
        return;
    };

    // If every option is protected, leave the list intact.
    if !courses.is_empty() && courses.iter().all(|c| is_protected(c, protected)) {
        return;
    }

    let target_count = count.unwrap_or(1);
    let target_credits = credits;

    let mut chosen: Vec<String> = Vec::new();
    let mut chosen_set: HashSet<String> = HashSet::new();

    // Pass 1: pinned by --include (in input order).
    for c in courses.iter() {
        if include.contains(c) && chosen_set.insert(c.clone()) {
            chosen.push(c.clone());
        }
    }
    // Pass 2: protected courses (in input order).
    for c in courses.iter() {
        if is_protected(c, protected) && chosen_set.insert(c.clone()) {
            chosen.push(c.clone());
        }
    }
    // Pass 3: fill from remaining courses, ordered by depths then lexicographically.
    let mut remaining: Vec<String> = courses
        .iter()
        .filter(|c| !chosen_set.contains(*c))
        .cloned()
        .collect();
    remaining.sort_by(|a, b| {
        depths
            .get(a)
            .copied()
            .unwrap_or(0)
            .cmp(&depths.get(b).copied().unwrap_or(0))
            .then_with(|| a.cmp(b))
    });

    if let Some(target) = target_credits {
        // Credits-driven selection. Greedy until total credits reaches the target.
        // `target` is a u32 from the YAML schema; degree-credit values are well
        // within f32 precision range so the cast is always lossless in practice.
        #[allow(clippy::cast_precision_loss)]
        let target_f = target as f32;
        let mut total: f32 = chosen.iter().map(|c| credit_of(c, credit_lookup)).sum();
        for c in remaining {
            if total >= target_f {
                break;
            }
            total += credit_of(&c, credit_lookup);
            chosen.push(c);
        }
    } else {
        // Count-driven selection.
        let target = target_count.max(1) as usize;
        for c in remaining {
            if chosen.len() >= target {
                break;
            }
            chosen.push(c);
        }
    }

    *courses = chosen;
}

fn credit_of(key: &str, lookup: &HashMap<String, f32>) -> f32 {
    let val = lookup.get(key).copied().unwrap_or(0.0);
    if val > 0.0 {
        val
    } else {
        // Missing credits would stall the credit-greedy loop; assume 1 so
        // the loop terminates. Caller-side credit totals on trimmed output
        // will reflect whatever is actually in Course.credit_hours.
        1.0
    }
}

fn build_credit_lookup(program: &DegreeProgram) -> HashMap<String, f32> {
    program
        .courses
        .iter()
        .map(|(k, c)| (k.clone(), c.credit_hours))
        .collect()
}

// ---------------------------------------------------------------------------
// Orphan pruning
// ---------------------------------------------------------------------------

fn prune_orphan_courses(program: &mut DegreeProgram) -> Vec<String> {
    let mut referenced: HashSet<String> = HashSet::new();

    for req in program.requirements.values() {
        walk_requirement_courses(req, &mut |k| {
            referenced.insert(k.to_string());
        });
    }
    // Transitively include every prereq still mentioned by a retained
    // course, so the trimmed file remains internally consistent.
    let mut frontier: Vec<String> = referenced.iter().cloned().collect();
    while let Some(key) = frontier.pop() {
        if let Some(course) = program.courses.get(&key) {
            if let Some(raw) = &course.prerequisites_raw {
                for prereq in extract_all_courses(raw) {
                    if referenced.insert(prereq.clone()) {
                        frontier.push(prereq);
                    }
                }
            }
            for coreq in &course.corequisites {
                if referenced.insert(coreq.clone()) {
                    frontier.push(coreq.clone());
                }
            }
        }
    }

    let removed: Vec<String> = program
        .courses
        .keys()
        .filter(|k| !referenced.contains(*k))
        .cloned()
        .collect();
    for key in &removed {
        program.courses.remove(key);
    }
    let mut sorted = removed;
    sorted.sort();
    sorted
}

fn unique_courses(raw: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for c in extract_all_courses(raw) {
        if seen.insert(c.clone()) {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::Course;
    use crate::core::models::Degree;

    fn course(prefix: &str, number: &str, credits: f32, prereqs: Option<&str>) -> Course {
        Course {
            name: format!("{prefix}{number}"),
            prefix: prefix.to_string(),
            number: number.to_string(),
            credit_hours: credits,
            prerequisites_raw: prereqs.map(str::to_string),
            ..Course::default()
        }
    }

    fn base_degree(major_subjects: Option<Vec<&str>>) -> Degree {
        let mut deg = Degree::new(
            "Test BS".to_string(),
            "BS".to_string(),
            None,
            "semester".to_string(),
        );
        deg.id = Some("test".to_string());
        deg.major_subjects = major_subjects.map(|v| v.into_iter().map(str::to_string).collect());
        deg
    }

    fn program_with(
        major_subjects: Option<Vec<&str>>,
        courses: Vec<(&str, Course)>,
        requirements: Vec<(&str, Requirement)>,
    ) -> DegreeProgram {
        let mut program = DegreeProgram {
            degree: base_degree(major_subjects),
            requirements: HashMap::new(),
            courses: HashMap::new(),
        };
        for (key, c) in courses {
            program.courses.insert(key.to_string(), c);
        }
        for (key, r) in requirements {
            program.requirements.insert(key.to_string(), r);
        }
        program
    }

    fn empty_from_with_courses(courses: Vec<&str>) -> FromClause {
        FromClause {
            courses: Some(courses.into_iter().map(str::to_string).collect()),
            pattern: None,
            include: None,
            exclude: None,
            groups: None,
            groups_required: None,
            per_group: None,
        }
    }

    /// Build a minimal `type: all` requirement that lists the given courses
    /// — anchors them so the orphan pruner keeps them around in the trimmed
    /// output. Real degrees always have such anchors; the helper just spares
    /// us from writing them out longhand in every test.
    fn all_req(courses: Vec<&str>) -> Requirement {
        Requirement {
            name: None,
            req_type: RequirementType::All,
            category: None,
            courses: Some(courses.into_iter().map(str::to_string).collect()),
            from: None,
            count: None,
            credits: None,
            credit_range: None,
            constraints: None,
            options: None,
        }
    }

    #[test]
    fn shared_shortest_path_picks_same_alternative_for_every_consumer() {
        // MATH_A has depths 1 (no prereqs). MATH_B requires MATH_PRE (depths 2).
        // Two CS courses each require "MATH_A | MATH_B" — both must pick MATH_A.
        let courses = vec![
            ("MATHA", course("MATH", "A", 3.0, None)),
            ("MATHPRE", course("MATH", "PRE", 3.0, None)),
            ("MATHB", course("MATH", "B", 3.0, Some("MATHPRE"))),
            ("CS1", course("CS", "1", 4.0, Some("MATHA | MATHB"))),
            ("CS2", course("CS", "2", 4.0, Some("MATHA | MATHB"))),
        ];
        let program = program_with(
            Some(vec!["CS"]),
            courses,
            vec![("core", all_req(vec!["CS1", "CS2"]))],
        );
        let (out, _report) = trim_program(&program, &TrimOptions::default());
        assert_eq!(
            out.courses["CS1"].prerequisites_raw.as_deref(),
            Some("MATHA")
        );
        assert_eq!(
            out.courses["CS2"].prerequisites_raw.as_deref(),
            Some("MATHA")
        );
    }

    #[test]
    fn preserves_alternatives_among_protected_subjects() {
        // Both alternatives are CS — major subject — so the disjunct stays.
        let courses = vec![
            ("CS163", course("CS", "163", 4.0, None)),
            ("CS164", course("CS", "164", 4.0, None)),
            ("CS300", course("CS", "300", 4.0, Some("CS163 | CS164"))),
        ];
        let program = program_with(
            Some(vec!["CS"]),
            courses,
            vec![("core", all_req(vec!["CS300"]))],
        );
        let (out, _report) = trim_program(&program, &TrimOptions::default());
        assert_eq!(
            out.courses["CS300"].prerequisites_raw.as_deref(),
            Some("CS163 | CS164")
        );
    }

    #[test]
    fn mixed_disjunct_prefers_protected_drops_unprotected() {
        // CS163 (protected) | MATH156 (not) → CS163 only.
        let courses = vec![
            ("CS163", course("CS", "163", 4.0, None)),
            ("MATH156", course("MATH", "156", 4.0, None)),
            ("CS300", course("CS", "300", 4.0, Some("CS163 | MATH156"))),
        ];
        let program = program_with(
            Some(vec!["CS"]),
            courses,
            vec![("core", all_req(vec!["CS300"]))],
        );
        let (out, _report) = trim_program(&program, &TrimOptions::default());
        assert_eq!(
            out.courses["CS300"].prerequisites_raw.as_deref(),
            Some("CS163")
        );
    }

    #[test]
    fn keep_all_subject_protects_additional_prefix() {
        // MATH156 vs MATH160 — with --keep-all MATH, both survive.
        let courses = vec![
            ("MATH156", course("MATH", "156", 4.0, None)),
            ("MATH160", course("MATH", "160", 4.0, None)),
            ("CS300", course("CS", "300", 4.0, Some("MATH156 | MATH160"))),
        ];
        let program = program_with(
            Some(vec!["CS"]),
            courses,
            vec![("core", all_req(vec!["CS300"]))],
        );
        let opts = TrimOptions {
            keep_all_subjects: HashSet::from(["MATH".to_string()]),
            ..TrimOptions::default()
        };
        let (out, _report) = trim_program(&program, &opts);
        assert_eq!(
            out.courses["CS300"].prerequisites_raw.as_deref(),
            Some("MATH156 | MATH160")
        );
    }

    #[test]
    fn include_forces_specific_pick_over_depth() {
        // Same setup as shared_shortest_path test but --include MATHB.
        let courses = vec![
            ("MATHA", course("MATH", "A", 3.0, None)),
            ("MATHPRE", course("MATH", "PRE", 3.0, None)),
            ("MATHB", course("MATH", "B", 3.0, Some("MATHPRE"))),
            ("CS1", course("CS", "1", 4.0, Some("MATHA | MATHB"))),
        ];
        let program = program_with(
            Some(vec!["CS"]),
            courses,
            vec![("core", all_req(vec!["CS1"]))],
        );
        let opts = TrimOptions {
            include_courses: HashSet::from(["MATHB".to_string()]),
            ..TrimOptions::default()
        };
        let (out, _report) = trim_program(&program, &opts);
        assert_eq!(
            out.courses["CS1"].prerequisites_raw.as_deref(),
            Some("MATHB")
        );
    }

    #[test]
    fn select_count_trims_to_n_choices() {
        let select_req = Requirement {
            name: Some("pick 2".to_string()),
            req_type: RequirementType::Select,
            category: None,
            courses: None,
            from: Some(empty_from_with_courses(vec![
                "MATH100", "MATH200", "MATH300",
            ])),
            count: Some(2),
            credits: None,
            credit_range: None,
            constraints: None,
            options: None,
        };
        let courses = vec![
            ("MATH100", course("MATH", "100", 3.0, None)),
            ("MATH200", course("MATH", "200", 3.0, Some("MATH100"))),
            ("MATH300", course("MATH", "300", 3.0, Some("MATH200"))),
        ];
        let program = program_with(Some(vec!["CS"]), courses, vec![("req", select_req)]);
        let (out, _report) = trim_program(&program, &TrimOptions::default());
        let trimmed = out.requirements["req"]
            .from
            .as_ref()
            .and_then(|f| f.courses.as_ref())
            .unwrap();
        // Should keep the 2 lowest-depths courses: MATH100, MATH200.
        assert_eq!(trimmed, &vec!["MATH100".to_string(), "MATH200".to_string()]);
    }

    #[test]
    fn equivalents_group_with_no_protection_trims_to_smallest_depth() {
        let req = Requirement {
            name: None,
            req_type: RequirementType::All,
            category: None,
            courses: Some(vec!["{BIO100, BIO200}".to_string()]),
            from: None,
            count: None,
            credits: None,
            credit_range: None,
            constraints: None,
            options: None,
        };
        let courses = vec![
            ("BIO100", course("BIO", "100", 4.0, None)),
            ("BIO200", course("BIO", "200", 4.0, Some("BIO100"))),
        ];
        let program = program_with(Some(vec!["CS"]), courses, vec![("req", req)]);
        let (out, _report) = trim_program(&program, &TrimOptions::default());
        let entries = out.requirements["req"].courses.as_ref().unwrap();
        // BIO100 has depths 1, BIO200 has depths 2 → BIO100 wins; group collapses to a bare key.
        assert_eq!(entries, &vec!["BIO100".to_string()]);
    }

    #[test]
    fn equivalents_all_protected_kept_verbatim() {
        let req = Requirement {
            name: None,
            req_type: RequirementType::All,
            category: None,
            courses: Some(vec!["{CS530, CS535}".to_string()]),
            from: None,
            count: None,
            credits: None,
            credit_range: None,
            constraints: None,
            options: None,
        };
        let courses = vec![
            ("CS530", course("CS", "530", 4.0, None)),
            ("CS535", course("CS", "535", 4.0, None)),
        ];
        let program = program_with(Some(vec!["CS"]), courses, vec![("req", req)]);
        let (out, _report) = trim_program(&program, &TrimOptions::default());
        assert_eq!(
            out.requirements["req"].courses.as_ref().unwrap(),
            &vec!["{CS530, CS535}".to_string()]
        );
    }

    #[test]
    fn orphan_courses_pruned() {
        // BIO200 is no longer referenced after trim → must be removed.
        let req = Requirement {
            name: None,
            req_type: RequirementType::All,
            category: None,
            courses: Some(vec!["{BIO100, BIO200}".to_string()]),
            from: None,
            count: None,
            credits: None,
            credit_range: None,
            constraints: None,
            options: None,
        };
        let courses = vec![
            ("BIO100", course("BIO", "100", 4.0, None)),
            ("BIO200", course("BIO", "200", 4.0, None)),
        ];
        let program = program_with(Some(vec!["CS"]), courses, vec![("req", req)]);
        let (out, report) = trim_program(&program, &TrimOptions::default());
        assert!(out.courses.contains_key("BIO100"));
        assert!(!out.courses.contains_key("BIO200"));
        assert_eq!(report.orphan_courses_removed, vec!["BIO200".to_string()]);
    }

    #[test]
    fn protected_subjects_derived_when_major_subjects_missing() {
        // No `major_subjects` declared, but most courses are CS — should derive CS.
        let req = Requirement {
            name: None,
            req_type: RequirementType::All,
            category: None,
            courses: Some(vec![
                "CS100".to_string(),
                "CS200".to_string(),
                "CS300".to_string(),
                "MATH100".to_string(),
            ]),
            from: None,
            count: None,
            credits: None,
            credit_range: None,
            constraints: None,
            options: None,
        };
        let courses = vec![
            ("CS100", course("CS", "100", 3.0, None)),
            ("CS200", course("CS", "200", 3.0, None)),
            ("CS300", course("CS", "300", 3.0, None)),
            ("MATH100", course("MATH", "100", 3.0, None)),
        ];
        let program = program_with(None, courses, vec![("req", req)]);
        let (_out, report) = trim_program(&program, &TrimOptions::default());
        assert!(report.protected_subjects_derived);
        assert!(report.protected_subjects.contains(&"CS".to_string()));
    }

    // -----------------------------------------------------------------
    // Helper-level coverage
    // -----------------------------------------------------------------

    #[test]
    fn subject_of_handles_edges() {
        assert_eq!(subject_of(""), None);
        assert_eq!(subject_of("   "), None);
        assert_eq!(subject_of("123"), None, "no alphabetic prefix");
        assert_eq!(subject_of("CS3000"), Some("CS".to_string()));
        assert_eq!(
            subject_of("  cs101"),
            Some("CS".to_string()),
            "lowercase normalised, leading whitespace trimmed"
        );
        assert_eq!(subject_of("MATH2331"), Some("MATH".to_string()));
    }

    #[test]
    fn expand_course_entry_covers_bundle_equiv_and_bare() {
        assert_eq!(
            expand_course_entry("[CS1800, CS1802]"),
            vec!["CS1800".to_string(), "CS1802".to_string()],
            "bundle [A, B] expands to its members"
        );
        assert_eq!(
            expand_course_entry("{CS4530, CS4535}"),
            vec!["CS4530".to_string(), "CS4535".to_string()],
            "equivalents {{A, B}} expand to their members"
        );
        assert_eq!(
            expand_course_entry("CS3000"),
            vec!["CS3000".to_string()],
            "bare key passes through unchanged"
        );
    }

    #[test]
    fn pick_equivalents_honours_include_override() {
        let protected: HashSet<String> = HashSet::new();
        let depths = HashMap::from([("BIO100".to_string(), 1), ("BIO200".to_string(), 5)]);
        let include = HashSet::from(["BIO200".to_string()]);
        let courses = vec!["BIO100".to_string(), "BIO200".to_string()];

        // BIO200 has a larger depths but --include pins it as the winner.
        let result = pick_equivalents(&courses, &protected, &depths, &include);
        assert_eq!(result, vec!["BIO200".to_string()]);
    }

    #[test]
    fn select_credits_fills_until_target_reached() {
        // Greedy by depths (lowest first); stop once cumulative credit total
        // meets `credits`. Here depths are MATH100<MATH200<MATH300, credits
        // are 4 each, target is 7 → must pick MATH100 (4) + MATH200 (8 ≥ 7).
        let select_req = Requirement {
            name: None,
            req_type: RequirementType::Select,
            category: None,
            courses: None,
            from: Some(empty_from_with_courses(vec![
                "MATH100", "MATH200", "MATH300",
            ])),
            count: None,
            credits: Some(7),
            credit_range: None,
            constraints: None,
            options: None,
        };
        let courses = vec![
            ("MATH100", course("MATH", "100", 4.0, None)),
            ("MATH200", course("MATH", "200", 4.0, Some("MATH100"))),
            ("MATH300", course("MATH", "300", 4.0, Some("MATH200"))),
        ];
        let program = program_with(Some(vec!["CS"]), courses, vec![("req", select_req)]);
        let (out, _report) = trim_program(&program, &TrimOptions::default());
        let trimmed = out.requirements["req"]
            .from
            .as_ref()
            .and_then(|f| f.courses.as_ref())
            .unwrap();
        assert_eq!(trimmed, &vec!["MATH100".to_string(), "MATH200".to_string()]);
    }

    #[test]
    fn metric_uses_upstream_depth_only_not_downstream_blocking() {
        // Regression: an earlier version of this module ranked alternatives by
        // `compute_delay`, which combines longest-path-to + longest-path-from.
        // That picked the *less-used* (longer-upstream) alternative because it
        // had a smaller downstream count. The metric must only consider the
        // candidate's own prereq chain.
        //
        //   MATH_SHALLOW: no prereqs                              → depth 0
        //   MATH_DEEP:    requires MATH_PRE_A & MATH_PRE_B (root) → depth 1
        //   CS_USES_SHALLOW_1..5 all require MATH_SHALLOW         → load it up
        //   CS_TARGET:    requires MATH_SHALLOW | MATH_DEEP
        //
        // With the buggy bidirectional metric, MATH_SHALLOW's heavy downstream
        // blocking would have made it score *higher* than MATH_DEEP and the
        // wrong alternative would win. The upstream-only metric correctly
        // ranks MATH_SHALLOW (depth 0) below MATH_DEEP (depth 1).
        let mut courses = vec![
            ("MATHSHALLOW", course("MATH", "SHALLOW", 3.0, None)),
            ("MATHPREA", course("MATH", "PREA", 3.0, None)),
            ("MATHPREB", course("MATH", "PREB", 3.0, None)),
            (
                "MATHDEEP",
                course("MATH", "DEEP", 3.0, Some("MATHPREA & MATHPREB")),
            ),
            (
                "CSTARGET",
                course("CS", "TARGET", 4.0, Some("MATHSHALLOW | MATHDEEP")),
            ),
        ];
        for i in 1..=5 {
            courses.push((
                Box::leak(format!("CSUSER{i}").into_boxed_str()),
                course("CS", &format!("USER{i}"), 4.0, Some("MATHSHALLOW")),
            ));
        }
        let mut all = vec!["CSTARGET"];
        for i in 1..=5 {
            all.push(Box::leak(format!("CSUSER{i}").into_boxed_str()));
        }
        let program = program_with(Some(vec!["CS"]), courses, vec![("core", all_req(all))]);
        let (out, _report) = trim_program(&program, &TrimOptions::default());
        assert_eq!(
            out.courses["CSTARGET"].prerequisites_raw.as_deref(),
            Some("MATHSHALLOW"),
            "trim must pick the shallowest-upstream alternative regardless of how many downstream courses depend on it"
        );
    }

    #[test]
    fn mixed_disjunct_keeps_every_branch_that_has_a_protected_course() {
        // (CS163 & MATH156) | (CS164 & MATH157): every disjunct contains a
        // CS course (protected). The "mixed but every branch contributes"
        // rule keeps both branches verbatim.
        let courses = vec![
            ("CS163", course("CS", "163", 4.0, None)),
            ("CS164", course("CS", "164", 4.0, None)),
            ("MATH156", course("MATH", "156", 4.0, None)),
            ("MATH157", course("MATH", "157", 4.0, None)),
            (
                "CS300",
                course(
                    "CS",
                    "300",
                    4.0,
                    Some("(CS163 & MATH156) | (CS164 & MATH157)"),
                ),
            ),
        ];
        let program = program_with(
            Some(vec!["CS"]),
            courses,
            vec![("core", all_req(vec!["CS300"]))],
        );
        let (out, _report) = trim_program(&program, &TrimOptions::default());
        let trimmed = out.courses["CS300"].prerequisites_raw.as_deref().unwrap();
        for required in ["CS163", "CS164", "MATH156", "MATH157"] {
            assert!(
                trimmed.contains(required),
                "{required} missing from trimmed disjunct: {trimmed}"
            );
        }
    }
}
