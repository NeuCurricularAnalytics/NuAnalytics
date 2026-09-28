//! Free-elective blocks that exist only to reach the graduation total.
//!
//! Many degrees encode "free electives to reach 120 credits" as a fixed-size requirement
//! — `credits: 20` drawn from `*:*` — because the author sized it for a typical path. Plan
//! generation expanded that into a fixed set of placeholders for *every* plan, so a plan
//! whose other choices ran heavier (a longest path pulling in extra prerequisites)
//! overshot the total by up to the whole block: Colorado State's CS longest path landed at
//! 132 credits against 120.
//!
//! The fix has two halves, deliberately kept apart:
//!
//! - [`mark_fill_to_total`] decides **once, at conversion**, which requirements are such
//!   blocks and records it as [`Requirement::fills_to_total`]. The decision is written into
//!   the degree document where it can be read and corrected per degree.
//! - [`shrink_fill_blocks`] runs **per plan** and reads only that flag. It never looks at
//!   requirement wording.
//!
//! Keeping the text heuristic out of plan generation is the point. Gen-ed distribution
//! blocks share the exact `credits` + `*:*` shape and must stay fixed; a rule that decided
//! at analysis time is what previously let the behaviour swing between over-counting and
//! under-counting as it was re-tuned.

use std::collections::{HashMap, HashSet};

use crate::core::models::degree::{Requirement, RequirementType};
use crate::core::models::DegreeProgram;

// ============================================================================
// Classification — conversion time only
// ============================================================================

/// Phrases saying a block exists to reach some total. Only counted when the total they
/// point at is the *degree's* — see [`says_fill_to_degree_total`].
const FILL_PHRASES: [&str; 12] = [
    "to reach",
    "to meet the",
    "to complete the degree",
    "to complete degree",
    "to complete requirements for graduation",
    "to bring the total",
    "toward graduation",
    "towards graduation",
    "graduation minimum",
    "toward the degree total",
    "to fulfill the",
    "to satisfy the",
];

/// How far past a fill phrase to look for what it is reaching.
const FILL_ANCHOR_WINDOW: usize = 60;

/// Words naming the block itself as free or unrestricted electives.
const FREE_WORDS: [&str; 4] = ["free", "unrestricted", "general elective", "open elective"];

/// Words a fill block's name uses to describe what it holds. A block named for a specific
/// program — "Jewish Studies / YU Israel Program", "Second Discipline" — is a requirement
/// in its own right even when its note says it also counts toward the total.
const ELECTIVE_NOUNS: [&str; 11] = [
    "elective",
    "electiva",
    "free",
    "unrestricted",
    "additional",
    "credit",
    "hour",
    "unit",
    "coursework",
    "open",
    "remaining",
];

/// Requirement kinds that, named alongside free electives, make the block a *combined*
/// bucket ("Foreign Language and Free Electives"). Shrinking one of those on a heavy path
/// would under-count the genuine half, so they stay fixed.
///
/// Checked against the name with parenthetical asides removed, so a qualifier like
/// "Free (Non-Major) Electives" or "(may apply toward a minor)" does not count.
const COMBINED_KINDS: [&str; 22] = [
    "foreign language",
    "language",
    "breadth",
    "oral comm",
    "communication",
    "general education",
    "gen ed",
    "gen-ed",
    "core",
    "liberal",
    "curricul",
    "distribution",
    "university studies",
    "general studies",
    "major",
    "minor",
    "concentration",
    "writing",
    "science",
    "math",
    "humanit",
    "social",
];

/// Categories whose wildcard blocks are genuine requirements, never fill.
const FIXED_CATEGORIES: [&str; 3] = ["gen_ed", "major", "supporting"];

/// Remove `(…)` asides so qualifiers inside them are not read as part of the name.
fn strip_parentheticals(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// True when `number` appears in `text` as a whole number, not inside a longer one.
fn contains_whole_number(text: &str, number: &str) -> bool {
    text.match_indices(number).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + number.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_digit()) && !after.is_some_and(|c| c.is_ascii_digit())
    })
}

/// True when a fill phrase in `text` points at the degree total.
///
/// "Free electives to reach the 120-credit graduation minimum" does; "an additional course
/// must be taken to reach 21 credits" (Purdue Fort Wayne's second-discipline block) and
/// "3 advisor-approved credits to reach the 80-credit option total" (an Oregon State option
/// subtotal) do not — both reach something other than the degree, and both are real
/// requirements. A phrase is anchored by what follows it: the word graduation or degree,
/// or `total_credits` as a whole number. A phrase that names graduation or the degree
/// itself is anchored already.
fn says_fill_to_degree_total(text: &str, total_credits: Option<u32>) -> bool {
    let total = total_credits.map(|t| t.to_string());
    FILL_PHRASES.iter().any(|phrase| {
        let self_anchored = phrase.contains("graduat") || phrase.contains("degree");
        text.match_indices(phrase).any(|(i, _)| {
            if self_anchored {
                return true;
            }
            let window: String = text[i + phrase.len()..]
                .chars()
                .take(FILL_ANCHOR_WINDOW)
                .collect();
            window.contains("graduat")
                || window.contains("degree")
                || total
                    .as_deref()
                    .is_some_and(|t| contains_whole_number(&window, t))
        })
    })
}

/// True when the pool is every course at every level — `*:*` and nothing narrower.
///
/// A level- or subject-restricted wildcard (`*:300+`, `CS:*`) is a real constraint, so
/// only the fully unrestricted pool can be a free-elective block.
fn is_unrestricted_pool(req: &Requirement) -> bool {
    let Some(from) = &req.from else {
        return false;
    };
    if from.courses.is_some() || from.groups.is_some() {
        return false;
    }
    let pattern_open = from.pattern.as_deref().is_some_and(|p| p.trim() == "*:*");
    let include_open = from
        .include
        .as_ref()
        .is_some_and(|inc| !inc.is_empty() && inc.iter().all(|p| p.trim() == "*:*"));
    pattern_open || include_open
}

/// Decide whether a requirement is a block that exists to reach the degree total.
///
/// Conservative by construction, because a false positive silently under-counts a real
/// requirement while a false negative only leaves today's overshoot in place. All of these
/// must hold:
///
/// 1. a `select` sized by credits, not by course count;
/// 2. drawn from the fully unrestricted pool `*:*`;
/// 3. not a gen-ed, major or supporting block;
/// 4. not a combined bucket naming another requirement kind;
/// 5. named for electives or credits rather than a specific program; and
/// 6. either says it exists to reach the **degree** total, or names itself free or
///    unrestricted electives.
///
/// Called only when converting a degree. Plan generation never calls it — it reads the
/// flag this decision writes.
#[must_use]
pub fn is_fill_to_total(id: &str, req: &Requirement, total_credits: Option<u32>) -> bool {
    if req.req_type != RequirementType::Select || req.count.is_some() {
        return false;
    }
    if req.credits.is_none() && req.credit_range.is_none() {
        return false;
    }
    if !is_unrestricted_pool(req) {
        return false;
    }
    let category = req.category.as_deref().unwrap_or("").to_lowercase();
    if FIXED_CATEGORIES.contains(&category.as_str()) {
        return false;
    }

    let name = req.name.as_deref().unwrap_or("").to_lowercase();
    let id_words = id.to_lowercase().replace('_', " ");
    let bare_name = strip_parentheticals(&name)
        .replace("non-major", "")
        .replace("non major", "")
        .replace("nonmajor", "")
        .replace("outside the major", "")
        .replace("outside your major", "");
    if COMBINED_KINDS.iter().any(|k| bare_name.contains(k)) {
        return false;
    }
    // An unnamed block is described by its id instead.
    let described = if bare_name.trim().is_empty() {
        &id_words
    } else {
        &bare_name
    };
    if !ELECTIVE_NOUNS.iter().any(|n| described.contains(n)) {
        return false;
    }

    let note = req.external_note.as_deref().unwrap_or("").to_lowercase();
    let says_fill = says_fill_to_degree_total(&name, total_credits)
        || says_fill_to_degree_total(&note, total_credits);
    let says_free = FREE_WORDS
        .iter()
        .any(|w| name.contains(w) || id_words.contains(w));
    says_fill || says_free
}

/// Set [`Requirement::fills_to_total`] on every undecided requirement
/// [`is_fill_to_total`] accepts.
///
/// Returns the ids it newly flagged, sorted, so a conversion can report what changed.
/// Only `None` is ever written to: an explicit `Some(true)` or `Some(false)` is a decision
/// someone made in the document, and re-conversion must not undo it.
pub fn mark_fill_to_total(program: &mut DegreeProgram) -> Vec<String> {
    let total = program.degree.total_credits;
    let mut flagged: Vec<String> = program
        .requirements
        .iter_mut()
        .filter(|(id, req)| req.fills_to_total.is_none() && is_fill_to_total(id, req, total))
        .map(|(id, req)| {
            req.fills_to_total = Some(true);
            id.clone()
        })
        .collect();
    flagged.sort();
    flagged
}

/// Ids of flagged requirements, sorted so every per-plan walk visits them in one order.
#[must_use]
pub fn fill_requirement_ids<S: std::hash::BuildHasher>(
    requirements: &HashMap<String, Requirement, S>,
) -> Vec<String> {
    let mut ids: Vec<String> = requirements
        .iter()
        .filter(|(_, req)| req.fills_to_total == Some(true))
        .map(|(id, _)| id.clone())
        .collect();
    ids.sort();
    ids
}

// ============================================================================
// Per-plan sizing — reads only the flag
// ============================================================================

/// The result of sizing a plan's flagged blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FillResize {
    /// The plan's courses after resizing.
    pub courses: Vec<String>,
    /// Placeholders no longer in the plan.
    pub removed: HashSet<String>,
    /// Placeholders newly in the plan, by the requirement they belong to. Non-empty when a
    /// block is rebuilt at a size its old placeholders could not reach, e.g. 13 credits
    /// from six 3s and one 2.
    pub added: HashMap<String, Vec<String>>,
}

/// Leading letters of a placeholder name — its prefix, e.g. `FE` of `FE07S`.
fn placeholder_prefix(name: &str) -> &str {
    let end = name
        .char_indices()
        .find(|(_, c)| !c.is_ascii_alphabetic())
        .map_or(name.len(), |(i, _)| i);
    &name[..end]
}

/// Digits in a placeholder's number, e.g. 2 for `FE07S`.
fn placeholder_width(name: &str) -> usize {
    name[placeholder_prefix(name).len()..]
        .chars()
        .take_while(char::is_ascii_digit)
        .count()
        .max(1)
}

/// Size flagged blocks so a plan lands on its target rather than above it.
///
/// `plan_courses` is the plan after prerequisite expansion; `choices` maps requirement ids
/// to the courses chosen for them. Only placeholder members (`is_real` false) of flagged
/// requirements are resized — a real course chosen for a flagged block is left alone.
///
/// The flagged blocks are allowed `min(stated, max(0, target − rest))` credits, where
/// `rest` excludes their placeholders and any `ELEC` filler (which the caller recomputes
/// afterwards). The allowance is shared across flagged blocks in id order, and each block
/// is **rebuilt** at its share with [`placeholder_names`](crate::core::degree::placeholder::placeholder_names),
/// so an integral allowance is met exactly: Colorado State's 13 credits become four 3s and
/// a 1-credit `FE05S1`, where removing whole placeholders could only reach 14. A fractional
/// allowance rounds up, so the plan never lands below target.
///
/// Returns `None` when nothing needs resizing, which is the case for every plan with no
/// flagged requirement and for every plan already at or under target — those are
/// untouched by construction.
#[must_use]
pub fn shrink_fill_blocks<S: std::hash::BuildHasher>(
    plan_courses: &[String],
    choices: &HashMap<String, Vec<String>, S>,
    fill_ids: &[String],
    target: f32,
    credits_of: impl Fn(&str) -> f32,
    is_real: impl Fn(&str) -> bool,
    is_elec_filler: impl Fn(&str) -> bool,
) -> Option<FillResize> {
    if fill_ids.is_empty() {
        return None;
    }
    let in_plan: HashSet<&str> = plan_courses.iter().map(String::as_str).collect();

    // Each flagged requirement's placeholders in this plan, in id order.
    let mut claimed: HashSet<&str> = HashSet::new();
    let blocks: Vec<(&String, Vec<&str>)> = fill_ids
        .iter()
        .filter_map(|id| {
            let mut own: Vec<&str> = choices
                .get(id)?
                .iter()
                .map(String::as_str)
                .filter(|c| in_plan.contains(c) && !is_real(c) && claimed.insert(c))
                .collect();
            own.sort_unstable();
            (!own.is_empty()).then_some((id, own))
        })
        .collect();
    if blocks.is_empty() {
        return None;
    }

    let rest: f32 = plan_courses
        .iter()
        .map(String::as_str)
        .filter(|c| !claimed.contains(c) && !is_elec_filler(c))
        .map(&credits_of)
        .sum();
    let stated: f32 = claimed.iter().map(|c| credits_of(c)).sum();
    let allowance = (target - rest).clamp(0.0, stated);
    if allowance >= stated - f32::EPSILON {
        return None;
    }

    // Whole credits, rounded up so a fractional allowance never leaves the plan short.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let mut remaining = (allowance - 1e-4).ceil().max(0.0) as u32;

    let mut removed: HashSet<String> = HashSet::new();
    let mut added: HashMap<String, Vec<String>> = HashMap::new();
    for (id, own) in &blocks {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let own_credits = own.iter().map(|c| credits_of(c)).sum::<f32>().round() as u32;
        let share = own_credits.min(remaining);
        remaining -= share;

        let rebuilt = crate::core::degree::placeholder::placeholder_names(
            placeholder_prefix(own[0]),
            share,
            placeholder_width(own[0]),
        );
        let old: HashSet<&str> = own.iter().copied().collect();
        let new: HashSet<&str> = rebuilt.iter().map(String::as_str).collect();
        removed.extend(old.difference(&new).map(|c| (*c).to_string()));
        let fresh: Vec<String> = rebuilt
            .iter()
            .filter(|c| !old.contains(c.as_str()))
            .cloned()
            .collect();
        if !fresh.is_empty() {
            added.insert((*id).clone(), fresh);
        }
    }

    let mut courses: Vec<String> = plan_courses
        .iter()
        .filter(|c| !removed.contains(c.as_str()))
        .cloned()
        .collect();
    let mut ids: Vec<&String> = added.keys().collect();
    ids.sort();
    for id in ids {
        courses.extend(added[id].iter().cloned());
    }
    Some(FillResize {
        courses,
        removed,
        added,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::degree::FromClause;

    fn wildcard(name: &str, category: Option<&str>, note: Option<&str>) -> Requirement {
        Requirement {
            fills_to_total: None,
            name: Some(name.to_string()),
            req_type: RequirementType::Select,
            category: category.map(ToString::to_string),
            courses: None,
            from: Some(FromClause {
                courses: None,
                pattern: None,
                include: Some(vec!["*:*".to_string()]),
                exclude: None,
                groups: None,
                groups_required: None,
                per_group: None,
            }),
            count: None,
            credits: Some(20),
            credit_range: None,
            constraints: None,
            options: None,
            external_requirement: None,
            external_credits: None,
            external_note: note.map(ToString::to_string),
            tags: None,
        }
    }

    // --- classification: names taken verbatim from the stored corpus -------------------

    #[test]
    fn blocks_that_exist_to_reach_the_total_are_flagged() {
        for (id, name, note) in [
            // Colorado State CS — the report that surfaced this.
            (
                "free_electives",
                "Unrestricted Electives",
                Some("Free/unrestricted electives to reach the 120-credit graduation minimum"),
            ),
            (
                "additional",
                "Additional Coursework to Reach 120 Hours",
                None,
            ),
            (
                "x",
                "Additional Credit Hours to Complete Degree (general electives)",
                None,
            ),
            ("x", "Electives (to bring the total to 120 credits)", None),
            (
                "x",
                "Free / General Electives (to reach 120 total credits)",
                None,
            ),
            ("x", "Additional Units Toward Graduation", None),
        ] {
            assert!(
                is_fill_to_total(id, &wildcard(name, Some("elective"), note), Some(120)),
                "not flagged: {name}"
            );
        }
    }

    #[test]
    fn stated_amount_free_electives_are_flagged_too() {
        // The 233 "Free Electives (12 cr)" blocks: treated as fill by decision, since a
        // standalone free-elective block has no content of its own to under-count.
        for name in [
            "Free Electives (12 credits)",
            "Free Electives",
            "Block 6: Free Electives (12 or 14 credits)",
        ] {
            assert!(
                is_fill_to_total("free", &wildcard(name, Some("elective"), None), Some(120)),
                "not flagged: {name}"
            );
        }
    }

    #[test]
    fn a_qualifier_in_parentheses_does_not_make_a_block_combined() {
        for name in [
            "Free (Non-Major) Electives",
            "Elective Credits (to reach 120; may apply toward a second major or minor)",
        ] {
            assert!(
                is_fill_to_total("x", &wildcard(name, Some("elective"), None), Some(120)),
                "wrongly treated as combined: {name}"
            );
        }
    }

    #[test]
    fn combined_buckets_stay_fixed_even_when_they_mention_reaching_the_total() {
        // Shrinking one of these on a heavy path would under-count the genuine half —
        // the direction this fix must never move in.
        for name in [
            "Foreign Language (B.A. requirement) and Free Electives",
            "Bachelor of Science Breadth Requirement + Free Electives",
            "Foreign Language / Oral Communication / Free Electives",
            "Exploratory Curriculum and Free Electives (to reach 180 credits)",
            "Distribution and Free Electives (to reach 32 units)",
            "Free / General Education Electives",
            "Additional University Studies / Free Electives (to reach 120 hrs)",
        ] {
            assert!(
                !is_fill_to_total("x", &wildcard(name, Some("elective"), None), Some(120)),
                "combined bucket flagged: {name}"
            );
        }
    }

    #[test]
    fn gen_ed_major_and_supporting_wildcards_are_never_flagged() {
        // 2,874 gen-ed blocks in the corpus have this exact shape and are real.
        for category in ["gen_ed", "major", "supporting"] {
            let req = wildcard(
                "Free Electives (to reach 120)",
                Some(category),
                Some("to reach 120"),
            );
            assert!(
                !is_fill_to_total("free", &req, Some(120)),
                "{category} flagged"
            );
        }
    }

    #[test]
    fn an_elective_with_neither_fill_wording_nor_free_naming_stays_fixed() {
        assert!(!is_fill_to_total(
            "upper",
            &wildcard(
                "300-400 Level Electives (15 credits, any subject)",
                Some("elective"),
                None
            ),
            Some(120)
        ));
    }

    #[test]
    fn a_restricted_pool_is_a_real_constraint_not_a_free_block() {
        for pattern in ["*:300+", "CS:*", "*:*,CS:*"] {
            let mut req = wildcard("Free Electives", Some("elective"), None);
            req.from.as_mut().unwrap().include =
                Some(pattern.split(',').map(ToString::to_string).collect());
            assert!(
                !is_fill_to_total("free", &req, Some(120)),
                "pool {pattern} flagged"
            );
        }
    }

    #[test]
    fn count_based_and_non_select_requirements_are_never_flagged() {
        let mut by_count = wildcard("Free Electives", Some("elective"), None);
        by_count.count = Some(4);
        assert!(!is_fill_to_total("free", &by_count, Some(120)));

        let mut all = wildcard("Free Electives", Some("elective"), None);
        all.req_type = RequirementType::All;
        assert!(!is_fill_to_total("free", &all, Some(120)));

        let mut no_credits = wildcard("Free Electives", Some("elective"), None);
        no_credits.credits = None;
        assert!(!is_fill_to_total("free", &no_credits, Some(120)));
    }

    #[test]
    fn marking_never_unflags_a_hand_correction() {
        let mut program = DegreeProgram {
            degree: crate::core::models::Degree::new(
                "Computer Science".to_string(),
                "BS".to_string(),
                None,
                "semester".to_string(),
            ),
            requirements: HashMap::new(),
            courses: HashMap::new(),
            conversion_warnings: Vec::new(),
            corrections_applied: Vec::new(),
        };
        // Would not pass the rule, but a person set it — it must survive re-conversion.
        let mut hand = wildcard("Advanced Studio", Some("elective"), None);
        hand.fills_to_total = Some(true);
        program.requirements.insert("studio".to_string(), hand);
        program.requirements.insert(
            "free_electives".to_string(),
            wildcard("Free Electives", Some("elective"), None),
        );
        let flagged = mark_fill_to_total(&mut program);
        assert_eq!(flagged, ["free_electives"], "reported a pre-existing flag");
        assert_eq!(
            program.requirements["studio"].fills_to_total,
            Some(true),
            "hand flag lost"
        );
        assert_eq!(
            program.requirements["free_electives"].fills_to_total,
            Some(true)
        );
    }

    // --- false positives found by dry-running the rule over the whole corpus --------------

    #[test]
    fn reaching_a_blocks_own_credits_is_not_reaching_the_degree_total() {
        // Purdue Fort Wayne BA CS: a real 21-credit second-discipline requirement whose note
        // happens to contain "to reach".
        let req = wildcard(
            "Second Discipline (approved courses from a non-CS discipline; >=12 cr at 20000+)",
            Some("elective"),
            Some(
                "If a discipline course is applied to another BA CS requirement, an \
                  additional course must be taken to reach 21 credits.",
            ),
        );
        assert!(!is_fill_to_total("second_discipline", &req, Some(120)));
    }

    #[test]
    fn reaching_an_option_subtotal_is_not_reaching_the_degree_total() {
        // Oregon State: fills an 80-credit option, inside a 180-credit degree.
        let req = wildcard(
            "Approved Electives",
            Some("elective"),
            Some("3 advisor-approved credits to reach the 80-credit option total."),
        );
        assert!(!is_fill_to_total("approved_electives", &req, Some(180)));
    }

    #[test]
    fn a_block_named_for_a_program_is_a_requirement_even_if_it_counts_toward_the_total() {
        // Yeshiva's mandatory dual curriculum. Its note does say "to reach the 128-credit
        // graduation minimum", but the block is named for what it requires.
        let req = wildcard(
            "Jewish Studies / YU Israel Program (dual-curriculum, taken outside Yeshiva College credits)",
            Some("elective"),
            Some("students complete ~31-32 credits of Jewish Studies to reach the 128-credit \
                  graduation minimum."),
        );
        assert!(!is_fill_to_total("jewish_studies_israel", &req, Some(128)));
    }

    #[test]
    fn the_degree_total_anchors_a_fill_phrase_only_as_a_whole_number() {
        let fill = |note: &str, total| {
            is_fill_to_total(
                "x",
                &wildcard("Electives", Some("elective"), Some(note)),
                total,
            )
        };
        assert!(fill("electives to reach 120 credits", Some(120)));
        assert!(fill(
            "to reach the 120-credit graduation minimum",
            Some(120)
        ));
        assert!(
            fill("to reach the graduation minimum", None),
            "graduation anchors alone"
        );
        assert!(
            !fill("to reach 1200 contact hours", Some(120)),
            "matched inside 1200"
        );
        assert!(!fill("to reach 21 credits", Some(120)));
        assert!(!fill("to meet the major requirement", Some(120)));
    }

    #[test]
    fn an_explicit_not_fill_decision_survives_re_conversion() {
        // Tulsa's `free_electives` is "Electives (14 hours; CS or CYB, advisor-approved)" —
        // the rule accepts it on the id, but it is really major electives. A person marking
        // it `false` must not be overruled the next time the degree is converted.
        let mut program = DegreeProgram {
            degree: crate::core::models::Degree::new(
                "Computer Science".to_string(),
                "BS".to_string(),
                None,
                "semester".to_string(),
            ),
            requirements: HashMap::new(),
            courses: HashMap::new(),
            conversion_warnings: Vec::new(),
            corrections_applied: Vec::new(),
        };
        let mut tulsa = wildcard(
            "Electives (14 hours; CS or CYB, advisor-approved)",
            Some("elective"),
            None,
        );
        assert!(
            is_fill_to_total("free_electives", &tulsa, Some(120)),
            "premise"
        );
        tulsa.fills_to_total = Some(false);
        program
            .requirements
            .insert("free_electives".to_string(), tulsa);
        assert!(mark_fill_to_total(&mut program).is_empty());
        assert_eq!(
            program.requirements["free_electives"].fills_to_total,
            Some(false)
        );
        assert!(fill_requirement_ids(&program.requirements).is_empty());
    }

    // --- per-plan sizing -----------------------------------------------------------------

    /// Colorado State's block: six 3-credit placeholders and one 2-credit remainder.
    const FE: [&str; 7] = ["FE01", "FE02", "FE03", "FE04", "FE05", "FE06", "FE07S"];

    fn cr(c: &str) -> f32 {
        // REALnn carries nn credits, so a test can dial `rest` exactly; everything else is
        // a placeholder, credited by the one shared rule rather than a local copy of it.
        c.strip_prefix("REAL").map_or_else(
            || crate::core::degree::placeholder::placeholder_credits(c),
            |n| n.parse().unwrap(),
        )
    }
    fn real(c: &str) -> bool {
        c.starts_with("REAL")
    }
    fn elec(c: &str) -> bool {
        c.starts_with("ELEC")
    }

    fn plan_with_rest(rest: u32) -> (Vec<String>, HashMap<String, Vec<String>>) {
        let mut courses = vec![format!("REAL{rest}")];
        courses.extend(FE.iter().map(ToString::to_string));
        let mut choices = HashMap::new();
        choices.insert(
            "free_electives".to_string(),
            FE.iter().map(ToString::to_string).collect(),
        );
        (courses, choices)
    }

    fn shrink(rest: u32) -> Option<FillResize> {
        let (courses, choices) = plan_with_rest(rest);
        shrink_fill_blocks(
            &courses,
            &choices,
            &["free_electives".to_string()],
            120.0,
            cr,
            real,
            elec,
        )
    }

    fn total(courses: &[String]) -> f32 {
        courses.iter().map(|c| cr(c)).sum()
    }

    fn fe(courses: &[String]) -> Vec<&str> {
        let mut out: Vec<&str> = courses
            .iter()
            .map(String::as_str)
            .filter(|c| c.starts_with("FE"))
            .collect();
        out.sort_unstable();
        out
    }

    #[test]
    fn the_colorado_state_trimmed_longest_path_lands_exactly_on_the_target() {
        // 112 credits of real courses + the 20-credit block = 132, the reported case.
        let resize = shrink(112).expect("must shrink");
        assert!(
            (total(&resize.courses) - 120.0).abs() < f32::EPSILON,
            "{}",
            total(&resize.courses)
        );
        assert_eq!(fe(&resize.courses), ["FE01", "FE02", "FE03S"]);
    }

    #[test]
    fn the_colorado_state_full_longest_path_lands_exactly_on_the_target() {
        // The 121 in your report: 107 + a 13-credit allowance. Whole 3s and a 2 could only
        // reach 14; rebuilding the block reaches 13 with a 1-credit remainder.
        let resize = shrink(107).expect("must shrink");
        assert!(
            (total(&resize.courses) - 120.0).abs() < f32::EPSILON,
            "{}",
            total(&resize.courses)
        );
        assert_eq!(
            fe(&resize.courses),
            ["FE01", "FE02", "FE03", "FE04", "FE05S1"]
        );
        assert_eq!(resize.added["free_electives"], ["FE05S1"]);
    }

    #[test]
    fn a_light_plan_keeps_the_whole_block() {
        // The shortest path: 95 real + 20 = 115, still under target. The block must stay
        // at its stated size and the ELEC filler tops up — today's correct behaviour.
        assert!(shrink(95).is_none());
        assert!(shrink(100).is_none(), "exactly on target must be untouched");
    }

    #[test]
    fn a_plan_already_over_target_on_real_courses_drops_the_whole_block() {
        let resize = shrink(125).expect("must shrink");
        assert_eq!(resize.removed.len(), FE.len());
        assert!(resize.added.is_empty());
        assert!((total(&resize.courses) - 125.0).abs() < f32::EPSILON);
    }

    #[test]
    fn every_plan_lands_exactly_on_target_or_is_left_alone() {
        // The property that stops this fix trading one error for the other: never below
        // target, never above what it was, and exact whenever the block can reach it.
        for rest in 60..=140 {
            let (courses, _) = plan_with_rest(rest);
            let after = shrink(rest).map_or_else(|| total(&courses), |r| total(&r.courses));
            let before = f32::from(u16::try_from(rest).unwrap()) + 20.0;
            if before >= 120.0 {
                assert!(
                    after >= 120.0 - f32::EPSILON,
                    "rest {rest}: fell to {after}"
                );
            }
            assert!(
                after <= before + f32::EPSILON,
                "rest {rest}: grew to {after}"
            );
            match rest {
                100..=119 => assert!((after - 120.0).abs() < f32::EPSILON, "rest {rest}: {after}"),
                120.. => assert!(
                    (after - before.min(f32::from(u16::try_from(rest).unwrap()))).abs()
                        < f32::EPSILON
                ),
                _ => assert!(
                    (after - before).abs() < f32::EPSILON,
                    "rest {rest}: touched a light plan"
                ),
            }
        }
    }

    #[test]
    fn nothing_changes_without_a_flagged_requirement() {
        let (courses, choices) = plan_with_rest(112);
        assert!(shrink_fill_blocks(&courses, &choices, &[], 120.0, cr, real, elec).is_none());
    }

    #[test]
    fn a_real_course_chosen_for_a_flagged_block_is_never_removed() {
        let (mut courses, mut choices) = plan_with_rest(112);
        courses.push("REAL3".to_string());
        choices
            .get_mut("free_electives")
            .unwrap()
            .push("REAL3".to_string());
        let resize = shrink_fill_blocks(
            &courses,
            &choices,
            &["free_electives".to_string()],
            120.0,
            cr,
            real,
            elec,
        )
        .expect("shrinks");
        assert!(resize.courses.contains(&"REAL3".to_string()));
        assert!(!resize.removed.contains("REAL3"));
    }

    #[test]
    fn elec_filler_does_not_count_toward_the_rest() {
        // The caller recomputes ELEC afterwards; counting stale filler here would shrink
        // the block too far and let the plan fall under target.
        let (mut courses, choices) = plan_with_rest(95);
        courses.push("ELEC001".to_string());
        courses.push("ELEC002S".to_string());
        assert!(shrink_fill_blocks(
            &courses,
            &choices,
            &["free_electives".to_string()],
            120.0,
            cr,
            real,
            elec
        )
        .is_none());
    }

    #[test]
    fn a_fractional_allowance_rounds_up_so_the_plan_is_never_short() {
        // Real credits of 107.5 leave 12.5 to find; 13 whole credits covers it.
        let mut courses: Vec<String> = FE.iter().map(ToString::to_string).collect();
        courses.push("HALF".to_string());
        let mut choices = HashMap::new();
        choices.insert(
            "free_electives".to_string(),
            FE.iter().map(ToString::to_string).collect(),
        );
        let credits = |c: &str| if c == "HALF" { 107.5 } else { cr(c) };
        let resize = shrink_fill_blocks(
            &courses,
            &choices,
            &["free_electives".to_string()],
            120.0,
            credits,
            |c| c == "HALF",
            elec,
        )
        .expect("shrinks");
        let after: f32 = resize.courses.iter().map(|c| credits(c)).sum();
        assert!((after - 120.5).abs() < f32::EPSILON, "{after}");
    }
}

#[cfg(test)]
mod round_trip_tests {
    //! The flag is only useful if it survives every path a degree document travels:
    //! the unified JSON stored as `programs.document`, and `degree trim`, which derives
    //! the corpus's trimmed tree from the full one.

    use crate::core::degree::{parse_degree_auto, to_unified_value};

    /// Colorado State's actual block, embedded in a minimal unified degree.
    const DOC: &str = r#"{
      "degree": {"id": "t", "name": "T", "degree_type": "BS", "institution": "X",
                 "system": "semester", "total_credits": 120},
      "requirements": {
        "core": {"type": "all", "courses": ["CS101"]},
        "free_electives": {"type": "select", "credits": 20, "category": "elective",
                           "name": "Unrestricted Electives", "from": {"include": ["*:*"]}}
      },
      "courses": {"CS101": {"name": "Intro", "prefix": "CS", "number": "101",
                            "credit_hours": 3.0, "prerequisites": null}}
    }"#;

    fn load() -> crate::core::models::DegreeProgram {
        parse_degree_auto(DOC).expect("parses").0
    }

    #[test]
    fn an_unflagged_document_serializes_with_no_trace_of_the_field() {
        // `document_hash` is computed over this JSON, so a stray `"fills_to_total": false`
        // would change the hash of every stored program.
        let json = to_unified_value(&load()).expect("serializes").to_string();
        assert!(!json.contains("fills_to_total"), "{json}");
    }

    #[test]
    fn the_flag_survives_the_stored_document_round_trip() {
        let mut program = load();
        super::mark_fill_to_total(&mut program);
        let stored = to_unified_value(&program).expect("serializes").to_string();
        assert!(
            stored.contains("\"fills_to_total\":true"),
            "not written: {stored}"
        );
        let back = parse_degree_auto(&stored).expect("re-parses").0;
        assert_eq!(
            back.requirements["free_electives"].fills_to_total,
            Some(true),
            "lost on read"
        );
        assert_eq!(
            back.requirements["core"].fills_to_total, None,
            "spread to another block"
        );
    }

    #[test]
    fn setting_the_flag_does_not_re_seed_plan_enumeration() {
        // Otherwise every flagged degree re-samples its plans on re-import and its metrics
        // move for reasons unrelated to the flag.
        let plain = load();
        let mut flagged = load();
        super::mark_fill_to_total(&mut flagged);
        assert_eq!(
            flagged.requirements["free_electives"].fills_to_total,
            Some(true)
        );
        assert_eq!(
            crate::core::degree::default_seed_for_program(&plain),
            crate::core::degree::default_seed_for_program(&flagged),
        );
    }

    #[test]
    fn an_unflagged_program_keeps_the_seed_it_had_before_the_flag_existed() {
        // The stored corpus was enumerated with this exact derivation; re-analysis must
        // reproduce it.
        let program = load();
        let before = crate::core::degree::default_seed_for_document(
            &crate::core::degree::serialize_degree_json(&program, false).unwrap_or_default(),
        );
        assert_eq!(
            crate::core::degree::default_seed_for_program(&program),
            before
        );
    }

    #[test]
    fn trimming_keeps_the_flag() {
        let mut program = load();
        super::mark_fill_to_total(&mut program);
        let (trimmed, _) = crate::core::degree::trim::trim_program(
            &program,
            &crate::core::degree::trim::TrimOptions::default(),
        );
        assert_eq!(
            trimmed.requirements["free_electives"].fills_to_total,
            Some(true),
            "trim dropped the flag — the trimmed corpus tree would overshoot again"
        );
    }
}
