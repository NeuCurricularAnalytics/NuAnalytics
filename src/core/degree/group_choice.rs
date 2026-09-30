//! A choice between groups of courses in an `all` list: `"{[MATH141, MATH142], [MATH155]}"`.
//!
//! The catalog pattern is "(MATH 141 and 142) or MATH 155" inside a list of required
//! courses. The engine already models a choice of course sets as a `one_of`, so rather than
//! teach every consumer a new course-reference shape, the parser expands the string into that
//! `one_of` once, right after a degree is read. Everything downstream — validation, the
//! resolver, plan generation, trimming, reports, import — only ever sees `one_of`.
//!
//! Supported in the `courses` of an `all` requirement, at the top level or inside a `one_of`
//! option. Anywhere else (a `select` pool, a group), a malformed string, or more than
//! [`MAX_COMBINATIONS`] expansions is left as written, and validation reports it by name
//! (`describe_unexpanded`) rather than as a missing course.
//!
//! A degree with no such string is returned untouched: `expand_group_choices` scans
//! first and changes nothing unless a choice is present.

use crate::core::models::degree::{Requirement, RequirementOption, RequirementType};
use crate::core::models::DegreeProgram;

/// More combinations than this in one requirement or option are not expanded: a list with
/// several choices multiplies, and a result this large is more likely a mistake than a
/// catalog.
pub const MAX_COMBINATIONS: usize = 32;

/// The alternatives of a choice-of-groups string, each a list of course keys.
///
/// `None` when `s` is not one — a plain course, a `[A, B]` bundle, or a `{A, B}` set of
/// equivalents, which are all left to the existing course-reference syntax. `Some(Err)`
/// when it is braced and contains a bracketed group but is malformed.
#[must_use]
pub fn parse_group_choice(s: &str) -> Option<Result<Vec<Vec<String>>, String>> {
    let inner = s.trim().strip_prefix('{')?.strip_suffix('}')?;
    if !inner.contains('[') {
        return None;
    }
    Some(split_alternatives(inner))
}

/// Split the inside of `{…}` into alternatives: `[A, B]` groups or single courses.
fn split_alternatives(inner: &str) -> Result<Vec<Vec<String>>, String> {
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for c in inner.chars() {
        match c {
            '[' => {
                if depth > 0 {
                    return Err("a group cannot contain another group".to_string());
                }
                depth = 1;
                current.push(c);
            }
            ']' => {
                if depth == 0 {
                    return Err("']' without a matching '['".to_string());
                }
                depth = 0;
                current.push(c);
            }
            '{' | '}' => return Err("a choice cannot contain another choice".to_string()),
            ',' if depth == 0 => items.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    if depth > 0 {
        return Err("'[' without a matching ']'".to_string());
    }
    items.push(current);

    let alternatives = items
        .iter()
        .map(|item| alternative(item.trim()))
        .collect::<Result<Vec<_>, _>>()?;
    if alternatives.len() < 2 {
        return Err("a choice needs at least two alternatives".to_string());
    }
    Ok(alternatives)
}

/// One alternative: `[A, B]` or a single course key.
fn alternative(item: &str) -> Result<Vec<String>, String> {
    let members: Vec<String> = match item.strip_prefix('[') {
        Some(rest) => rest
            .strip_suffix(']')
            .ok_or_else(|| format!("text after a group: '{item}'"))?
            .split(',')
            .map(|m| m.trim().to_string())
            .collect(),
        None => vec![item.to_string()],
    };
    if members.iter().any(String::is_empty) {
        return Err(format!("an empty course in '{item}'"));
    }
    if let Some(bad) = members.iter().find(|m| m.contains(char::is_whitespace)) {
        return Err(format!("'{bad}' is not a course key"));
    }
    Ok(members)
}

/// Expand every supported choice-of-groups string in `program` into a `one_of`.
///
/// Changes nothing when the program contains none, which is checked before anything is
/// touched.
pub(crate) fn expand_group_choices(program: &mut DegreeProgram) {
    if !program.requirements.values().any(has_expandable_choice) {
        return;
    }
    for (key, req) in &mut program.requirements {
        if req.req_type == RequirementType::All {
            if let Some(options) = expand_all(req, key) {
                req.req_type = RequirementType::OneOf;
                req.courses = None;
                req.constraints = None;
                req.options = Some(options);
            }
        } else if req.req_type == RequirementType::OneOf {
            if let Some(options) = &mut req.options {
                expand_options(options);
            }
        }
    }
}

/// Whether `req` holds a choice this module expands: a well-formed one, within the cap, in
/// an `all` list — its own, or one inside its options.
fn has_expandable_choice(req: &Requirement) -> bool {
    match req.req_type {
        RequirementType::All => combinations(req).is_some_and(|c| c.len() > 1),
        RequirementType::OneOf => req.options.iter().flatten().any(|option| {
            option.requirements.iter().any(|nested| {
                nested.req_type == RequirementType::All && has_expandable_choice(nested)
            })
        }),
        RequirementType::Select => false,
    }
}

/// Every course list `req`'s choices expand to, in order; `None` when it has no choice, a
/// malformed one, or more than [`MAX_COMBINATIONS`].
fn combinations(req: &Requirement) -> Option<Vec<Vec<String>>> {
    let courses = req.courses.as_ref()?;
    let mut lists: Vec<Vec<String>> = vec![Vec::new()];
    let mut any_choice = false;
    for item in courses {
        match parse_group_choice(item) {
            None => lists.iter_mut().for_each(|l| l.push(item.clone())),
            Some(Err(_)) => return None,
            Some(Ok(alternatives)) => {
                any_choice = true;
                if lists.len() * alternatives.len() > MAX_COMBINATIONS {
                    return None;
                }
                lists = lists
                    .iter()
                    .flat_map(|l| {
                        alternatives.iter().map(move |alt| {
                            let mut next = l.clone();
                            next.extend(alt.iter().cloned());
                            next
                        })
                    })
                    .collect();
            }
        }
    }
    any_choice.then_some(lists)
}

/// The chosen courses of each combination, for option names: `"MATH141 + MATH142"`.
fn combination_names(req: &Requirement) -> Vec<String> {
    let mut names: Vec<Vec<String>> = vec![Vec::new()];
    for item in req.courses.iter().flatten() {
        if let Some(Ok(alternatives)) = parse_group_choice(item) {
            names = names
                .iter()
                .flat_map(|n| {
                    alternatives.iter().map(move |alt| {
                        let mut next = n.clone();
                        next.push(alt.join(" + "));
                        next
                    })
                })
                .collect();
        }
    }
    names.into_iter().map(|n| n.join("; ")).collect()
}

/// A top-level `all` requirement's options, one per combination; `None` when it has none
/// to expand.
fn expand_all(req: &Requirement, key: &str) -> Option<Vec<RequirementOption>> {
    let lists = combinations(req)?;
    let names = combination_names(req);
    Some(
        lists
            .into_iter()
            .zip(names)
            .enumerate()
            .map(|(i, (courses, name))| RequirementOption {
                id: format!("{key}__{}", i + 1),
                name,
                requirements: vec![nested_all(req, courses)],
            })
            .collect(),
    )
}

/// The `all` requirement inside one option: the original's list with a choice made.
fn nested_all(req: &Requirement, courses: Vec<String>) -> Requirement {
    Requirement {
        name: req.name.clone(),
        req_type: RequirementType::All,
        category: req.category.clone(),
        courses: Some(courses),
        from: None,
        count: None,
        credits: None,
        credit_range: None,
        constraints: req.constraints.clone(),
        options: None,
        external_requirement: None,
        external_credits: None,
        external_note: None,
        tags: None,
        fills_to_total: None,
    }
}

/// Replace each option whose `all` requirements hold choices with one option per
/// combination, in place and in order.
fn expand_options(options: &mut Vec<RequirementOption>) {
    let mut expanded = Vec::with_capacity(options.len());
    for option in options.drain(..) {
        match option_combinations(&option) {
            Some(combos) => {
                for (i, (requirements, name)) in combos.into_iter().enumerate() {
                    expanded.push(RequirementOption {
                        id: format!("{}__{}", option.id, i + 1),
                        name: format!("{} ({name})", option.name),
                        requirements,
                    });
                }
            }
            None => expanded.push(option),
        }
    }
    *options = expanded;
}

/// The requirement lists an option expands to, each with a name for its choices; `None`
/// when it has nothing to expand or would exceed [`MAX_COMBINATIONS`].
fn option_combinations(option: &RequirementOption) -> Option<Vec<(Vec<Requirement>, String)>> {
    let mut combos: Vec<(Vec<Requirement>, Vec<String>)> = vec![(Vec::new(), Vec::new())];
    let mut any_choice = false;
    for nested in &option.requirements {
        let expansion = (nested.req_type == RequirementType::All)
            .then(|| combinations(nested))
            .flatten();
        let Some(lists) = expansion else {
            for (reqs, _) in &mut combos {
                reqs.push(nested.clone());
            }
            continue;
        };
        any_choice = true;
        if combos.len() * lists.len() > MAX_COMBINATIONS {
            return None;
        }
        let names = combination_names(nested);
        combos = combos
            .iter()
            .flat_map(|(reqs, labels)| {
                lists.iter().zip(&names).map(move |(courses, name)| {
                    let mut next_reqs = reqs.clone();
                    let mut chosen = nested.clone();
                    chosen.courses = Some(courses.clone());
                    next_reqs.push(chosen);
                    let mut next_labels = labels.clone();
                    next_labels.push(name.clone());
                    (next_reqs, next_labels)
                })
            })
            .collect();
    }
    any_choice.then(|| {
        combos
            .into_iter()
            .map(|(reqs, labels)| (reqs, labels.join("; ")))
            .collect()
    })
}

/// Why a choice-of-groups string was left as written, for validation to report; `None`
/// when `s` is not one.
#[must_use]
pub(crate) fn describe_unexpanded(s: &str) -> Option<String> {
    let parsed = parse_group_choice(s)?;
    Some(match parsed {
        Err(why) => format!("malformed choice of course groups '{s}': {why}"),
        Ok(_) => format!(
            "a choice of course groups ('{s}') is supported only in the course list of an \
             `all` requirement, top-level or inside a `one_of` option, with at most \
             {MAX_COMBINATIONS} combinations; write a `one_of` here instead"
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::degree::{parse_degree_auto, validate_degree_program, ValidationError};

    fn alternatives(s: &str) -> Vec<Vec<String>> {
        parse_group_choice(s)
            .unwrap_or_else(|| panic!("{s:?} is a choice"))
            .unwrap_or_else(|e| panic!("{s:?}: {e}"))
    }

    fn strings(xs: &[&str]) -> Vec<String> {
        xs.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn test_parse_group_choice_reads_groups_and_single_courses() {
        assert_eq!(
            alternatives("{[MATH141, MATH142], [MATH151, MATH152]}"),
            vec![
                strings(&["MATH141", "MATH142"]),
                strings(&["MATH151", "MATH152"])
            ]
        );
        assert_eq!(
            alternatives(" {[A,B],C} "),
            vec![strings(&["A", "B"]), strings(&["C"])]
        );
    }

    #[test]
    fn test_parse_group_choice_leaves_the_existing_syntax_alone() {
        for s in ["CS101", "[CHEM111, CHEM111L]", "{CS201, PHIL201}", "", "{}"] {
            assert!(
                parse_group_choice(s).is_none(),
                "{s:?} is not a choice of groups"
            );
        }
    }

    #[test]
    fn test_parse_group_choice_rejects_malformed_choices() {
        for s in [
            "{[A, B], [C}",
            "{[A, [B]], C}",
            "{[A, B]}",
            "{[A, ], C}",
            "{[A B], C}",
            "{[A, B] x, C}",
            "{[A, B], {C, D}}",
        ] {
            assert!(
                matches!(parse_group_choice(s), Some(Err(_))),
                "{s:?} must be malformed, got {:?}",
                parse_group_choice(s)
            );
        }
    }

    fn program(yaml: &str) -> DegreeProgram {
        parse_degree_auto(yaml).expect("parses").0
    }

    const HEAD: &str =
        "degree: {id: t, institution: T, program: T, total_credits: 30, gpa_minimum: 2.0}\n";
    const COURSES: &str = r#"
courses:
  CS101: {title: A, prefix: CS, number: "101", credits: 3}
  MATH141: {title: B, prefix: MATH, number: "141", credits: 4}
  MATH142: {title: C, prefix: MATH, number: "142", credits: 4}
  MATH155: {title: D, prefix: MATH, number: "155", credits: 5}
  PHYS101: {title: E, prefix: PHYS, number: "101", credits: 4}
  CHEM101: {title: F, prefix: CHEM, number: "101", credits: 4}
"#;

    fn degree(requirements: &str) -> DegreeProgram {
        program(&format!("{HEAD}requirements:\n{requirements}{COURSES}"))
    }

    #[test]
    fn test_a_choice_in_an_all_list_becomes_a_one_of() {
        let p = degree(
            "  core: {name: Core, type: all, category: major, courses: [CS101, \"{[MATH141, MATH142], [MATH155]}\"]}\n",
        );
        let core = &p.requirements["core"];
        assert_eq!(core.req_type, RequirementType::OneOf);
        assert!(core.courses.is_none());
        assert_eq!(core.name.as_deref(), Some("Core"));
        let options = core.options.as_ref().expect("options");
        let summary: Vec<(&str, &str, Vec<String>)> = options
            .iter()
            .map(|o| {
                assert_eq!(o.requirements.len(), 1);
                let r = &o.requirements[0];
                assert_eq!(r.req_type, RequirementType::All);
                assert_eq!(r.category.as_deref(), Some("major"));
                (o.id.as_str(), o.name.as_str(), r.courses.clone().unwrap())
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                (
                    "core__1",
                    "MATH141 + MATH142",
                    strings(&["CS101", "MATH141", "MATH142"])
                ),
                ("core__2", "MATH155", strings(&["CS101", "MATH155"])),
            ]
        );
        assert!(
            validate_degree_program(&p).errors.is_empty(),
            "{:?}",
            validate_degree_program(&p).errors
        );
    }

    #[test]
    fn test_two_choices_in_one_list_multiply_in_order() {
        let p = degree(
            "  core: {name: Core, type: all, courses: [\"{[MATH141, MATH142], MATH155}\", \"{PHYS101, [CHEM101]}\"]}\n",
        );
        let lists: Vec<Vec<String>> = p.requirements["core"]
            .options
            .as_ref()
            .unwrap()
            .iter()
            .map(|o| o.requirements[0].courses.clone().unwrap())
            .collect();
        assert_eq!(
            lists,
            vec![
                strings(&["MATH141", "MATH142", "PHYS101"]),
                strings(&["MATH141", "MATH142", "CHEM101"]),
                strings(&["MATH155", "PHYS101"]),
                strings(&["MATH155", "CHEM101"]),
            ]
        );
    }

    #[test]
    fn test_a_choice_inside_a_one_of_option_multiplies_that_option() {
        let p = degree(
            r#"  track:
    name: Track
    type: one_of
    options:
      - id: a
        name: A
        requirements:
          - {name: A1, type: all, courses: [CS101, "{[MATH141, MATH142], [MATH155]}"]}
          - {name: A2, type: all, courses: [PHYS101]}
      - id: b
        name: B
        requirements:
          - {name: B1, type: all, courses: [CHEM101]}
"#,
        );
        let options = p.requirements["track"].options.as_ref().unwrap();
        let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["a__1", "a__2", "b"]);
        assert_eq!(options[0].name, "A (MATH141 + MATH142)");
        assert_eq!(
            options[1].requirements[0].courses.clone().unwrap(),
            strings(&["CS101", "MATH155"])
        );
        assert_eq!(
            options[1].requirements[1].courses.clone().unwrap(),
            strings(&["PHYS101"])
        );
        assert!(
            validate_degree_program(&p).errors.is_empty(),
            "{:?}",
            validate_degree_program(&p).errors
        );
    }

    fn reasons(p: &DegreeProgram) -> Vec<String> {
        validate_degree_program(p)
            .errors
            .iter()
            .map(|e| match e {
                ValidationError::InvalidRequirement { reason, .. } => reason.clone(),
                other => format!("{other:?}"),
            })
            .collect()
    }

    #[test]
    fn test_an_unsupported_position_is_named_by_validation_not_called_a_missing_course() {
        let pool = degree(
            "  pick: {name: Pick, type: select, count: 1, from: {courses: [\"{[MATH141, MATH142], [MATH155]}\"]}}\n",
        );
        assert_eq!(
            pool.requirements["pick"].req_type,
            RequirementType::Select,
            "left as written"
        );
        let r = reasons(&pool);
        assert!(r.iter().any(|m| m.contains("supported only")), "{r:?}");
        assert!(!r.iter().any(|m| m.contains("MissingCourse")), "{r:?}");

        let malformed = degree(
            "  core: {name: Core, type: all, courses: [\"{[MATH141, MATH142], [MATH155}\"]}\n",
        );
        assert_eq!(
            malformed.requirements["core"].req_type,
            RequirementType::All,
            "left as written"
        );
        let r = reasons(&malformed);
        assert!(r.iter().any(|m| m.starts_with("malformed choice")), "{r:?}");
        assert!(!r.iter().any(|m| m.contains("MissingCourse")), "{r:?}");
    }

    #[test]
    fn test_more_than_the_cap_is_left_for_validation() {
        let choice = "\"{[MATH141, MATH142], MATH155}\"";
        let six = [choice; 6].join(", ");
        let p = degree(&format!(
            "  core: {{name: Core, type: all, courses: [{six}]}}\n"
        ));
        assert_eq!(
            p.requirements["core"].req_type,
            RequirementType::All,
            "64 > {MAX_COMBINATIONS}"
        );
        assert!(reasons(&p).iter().any(|m| m.contains("at most")));
    }

    #[test]
    fn test_expansion_round_trips_through_unified_json() {
        let p = degree(
            "  core: {name: Core, type: all, courses: [CS101, \"{[MATH141, MATH142], [MATH155]}\"]}\n",
        );
        let json = crate::core::degree::serialize_degree_json(&p, false).expect("serializes");
        assert!(
            !json.contains("{["),
            "the choice string is gone once expanded"
        );
        let back = parse_degree_auto(&json).expect("re-parses").0;
        assert_eq!(
            crate::core::degree::serialize_degree_json(&back, false).unwrap(),
            json,
            "expanding is idempotent"
        );
    }

    /// Every degree the repo ships, deserialized *without* the parser's expansion, then
    /// expanded: none contains the syntax, so each must come out exactly as it went in.
    #[test]
    fn test_expansion_changes_nothing_in_any_shipped_degree() {
        let json_fixtures = [
            include_str!("../../../tests/assets/degrees/arizona-state-university-computer-science-bs.unified.json"),
            include_str!("../../../tests/assets/degrees/bellevue-college-software-development-bas-artificial-intelligence-concentration.unified.json"),
            include_str!("../../../tests/assets/degrees/bowdoin-college-computer-science.unified.json"),
            include_str!("../../../tests/assets/degrees/california-state-university-los-angeles-computer-science-b-s.unified.json"),
            include_str!("../../../tests/assets/degrees/college-of-charleston-computer-science-b-s.unified.json"),
            include_str!("../../../tests/assets/degrees/liberty-university-computer-science-b-s.unified.json"),
            include_str!("../../../tests/assets/degrees/metropolitan-state-university-of-denver-computer-science-major-b-s.unified.json"),
            include_str!("../../../tests/assets/degrees/new-mexico-state-university-main-campus-computer-science-bachelor-of-science.unified.json"),
            include_str!("../../../tests/assets/degrees/rhode-island-college-artificial-intelligence-bs.unified.json"),
            include_str!("../../../tests/assets/degrees/syracuse-university-computer-science-bs.unified.json"),
            include_str!("../../../tests/assets/degrees/texas-state-university-computer-science-b-s.unified.json"),
            include_str!("../../../tests/assets/degrees/tulane-university-of-louisiana-computer-science-bs.unified.json"),
            include_str!("../../../tests/assets/degrees/western-kentucky-university-computer-science-bachelor-of-science.unified.json"),
        ];
        let yaml_samples = [
            include_str!("../../../samples/degrees/csu-cs-bscs-general.yaml"),
            include_str!("../../../samples/degrees/neu-khoury-bscs-boston.yaml"),
            include_str!("../../../samples/degrees/uhm-ics-bscs-general.yaml"),
            include_str!("../../assets/init/skills/degree-author/example.yaml"),
        ];
        let raw: Vec<DegreeProgram> = json_fixtures
            .iter()
            .map(|t| serde_json::from_str(t).expect("fixture deserializes"))
            .chain(
                yaml_samples
                    .iter()
                    .map(|t| serde_yaml::from_str(t).expect("sample deserializes")),
            )
            .collect();
        assert_eq!(raw.len(), 17);
        for mut p in raw {
            crate::core::degree::yaml_parser::resolve_prerequisites(&mut p);
            let before = serde_json::to_string(&p).unwrap();
            assert!(
                !p.requirements.values().any(has_expandable_choice),
                "{:?} uses the syntax",
                p.degree.id
            );
            expand_group_choices(&mut p);
            assert_eq!(
                serde_json::to_string(&p).unwrap(),
                before,
                "{:?} changed",
                p.degree.id
            );
        }
    }
}
