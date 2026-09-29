//! Placeholder course names and the credits they stand for.
//!
//! A placeholder is a synthetic course standing in for credits no specific course was
//! chosen for: `ELEC001` from the generic filler, `FE01` from a wildcard requirement such
//! as free electives. It has no catalog entry, so **its name is the only record of its
//! credits**, and this module is the single place that turns credits into names and names
//! into credits.
//!
//! | name | credits | |
//! |---|---|---|
//! | `FE03`, `ELEC001` | 3 | a full placeholder |
//! | `FE07S`, `ELEC002S` | 2 | the original short form; still written for 2 |
//! | `FE05S1`, `ELEC004S1` | 1 | explicit partial credit |
//!
//! Before this module the rule was "ends in `S` → 2, otherwise 3", re-implemented in seven
//! places that had already drifted (one also accepted an `SM` suffix nothing produced).
//! Worse, with no way to say 1 credit, every remainder of 1 was written as a 2-credit `S`:
//! a wildcard block stating 10 credits counted as 11, and 687 such blocks in the stored
//! corpus each over-counted by one credit in every plan that contained them.

/// Credits a full placeholder stands for.
pub const FULL_PLACEHOLDER_CREDITS: f32 = 3.0;

/// Credits of the historical short form, `…S`.
pub const SHORT_PLACEHOLDER_CREDITS: f32 = 2.0;

/// Shortfalls this small are float noise, not a real gap.
const CREDIT_EPSILON: f32 = 1e-3;

/// Credits a placeholder name stands for.
///
/// Parsed by structure — letters (the prefix), an optional `_`, digits (the number), then
/// the credit marker: nothing for 3, `S` for 2, `S<n>` for `n`. Only meaningful for names
/// that are placeholders; a real course's credits come from the catalog.
///
/// The marker is found *after the number*, never by searching for the last `S`: many
/// prefixes end in one (`NS` for natural sciences, `PSS`, `GES`). The last-`S` reading
/// credited `NS04` with 4, and a course key that missed the catalog, such as `METCS201`,
/// with 201 — which is how whole plans reached the thousands.
#[must_use]
pub fn placeholder_credits(name: &str) -> f32 {
    let after_prefix = name.trim_start_matches(|c: char| c.is_ascii_alphabetic());
    let after_prefix = after_prefix.strip_prefix('_').unwrap_or(after_prefix);
    let marker = after_prefix.trim_start_matches(|c: char| c.is_ascii_digit());
    match marker.strip_prefix('S') {
        Some("") => SHORT_PLACEHOLDER_CREDITS,
        Some(n) if n.chars().all(|c| c.is_ascii_digit()) => {
            n.parse::<u16>().map_or(FULL_PLACEHOLDER_CREDITS, f32::from)
        }
        _ => FULL_PLACEHOLDER_CREDITS,
    }
}

/// Placeholder names covering exactly `credits`, as full 3-credit placeholders plus one
/// partial for any remainder: `S` for 2, `S1` for 1.
///
/// Numbered from 1 with `width` digits (`FE01`, `ELEC001`), which is what the resolver and
/// the filler have always produced, so names for whole-3 amounts are unchanged.
#[must_use]
pub fn placeholder_names(prefix: &str, credits: u32, width: usize) -> Vec<String> {
    let full = credits / 3;
    let remainder = credits % 3;
    let mut names: Vec<String> = (1..=full).map(|i| format!("{prefix}{i:0width$}")).collect();
    match remainder {
        2 => names.push(format!("{prefix}{:0width$}S", full + 1)),
        1 => names.push(format!("{prefix}{:0width$}S1", full + 1)),
        _ => {}
    }
    names
}

/// Placeholder names for a fractional credit need, rounded up to whole credits.
///
/// Any real shortfall is covered, so a plan is never left below its total. The fillers this
/// replaced dropped a remainder of half a credit or less, which left half-unit plans at
/// course-unit schools (Penn, Colorado College, TCNJ) short of their total. An integral
/// need is met exactly; a fractional one is overshot by less than a credit.
#[must_use]
pub fn placeholder_names_for(prefix: &str, credits_needed: f32, width: usize) -> Vec<String> {
    if credits_needed <= CREDIT_EPSILON {
        return Vec::new();
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let credits = (credits_needed - CREDIT_EPSILON).ceil() as u32;
    placeholder_names(prefix, credits, width)
}

/// The generic free-elective filler, `ELEC001`…, for `credits_needed`.
#[must_use]
pub fn elective_placeholders(credits_needed: f32) -> Vec<String> {
    placeholder_names_for("ELEC", credits_needed, 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credits_are_read_from_every_name_form() {
        for (name, want) in [
            ("FE03", 3.0),
            ("ELEC001", 3.0),
            ("FE07S", 2.0),
            ("ELEC002S", 2.0),
            ("FE05S1", 1.0),
            ("ELEC004S1", 1.0),
            ("FE05S2", 2.0),
            // Numbered 99 must not merge into 991 — the recogniser had that bug.
            ("FE99S1", 1.0),
            // Prefixes ending in S: the marker comes after the number, not at the last S.
            ("NS04", 3.0),
            ("PSS01", 3.0),
            ("PSS02S1", 1.0),
            ("GES03S", 2.0),
            ("HSSI04", 3.0),
            ("WS12", 3.0),
            ("S01", 3.0),
            // The drifted MCP form, still readable.
            ("ELEC_03S", 2.0),
            ("ELEC_01", 3.0),
        ] {
            assert!(
                (placeholder_credits(name) - want).abs() < f32::EPSILON,
                "{name}: {}",
                placeholder_credits(name)
            );
        }
    }

    #[test]
    fn a_remainder_of_one_is_one_credit_not_two() {
        // The accounting error this module exists to fix: 10 used to become 3+3+3+2.
        let names = placeholder_names("FE", 10, 2);
        assert_eq!(names, ["FE01", "FE02", "FE03", "FE04S1"]);
        let total: f32 = names.iter().map(|n| placeholder_credits(n)).sum();
        assert!((total - 10.0).abs() < f32::EPSILON, "{total}");
    }

    #[test]
    fn every_whole_amount_round_trips_exactly_for_every_prefix_shape() {
        // Prefix shapes the requirement resolver and the `ELEC` filler produce — including
        // the ones ending in S that a last-S parser misread — plus a bare `S` as an edge
        // case. A single `FE` here is what let that bug through.
        for (prefix, width) in [
            ("FE", 2),
            ("NS", 2),
            ("PSS", 2),
            ("GES", 2),
            ("HSSI", 2),
            ("S", 2),
            ("SS", 2),
            ("ELEC", 3),
        ] {
            for credits in 0..=60 {
                let total: f32 = placeholder_names(prefix, credits, width)
                    .iter()
                    .map(|n| placeholder_credits(n))
                    .sum();
                assert!(
                    (total - f32::from(u16::try_from(credits).unwrap())).abs() < f32::EPSILON,
                    "{prefix} {credits} -> {total}"
                );
            }
        }
    }

    #[test]
    fn names_for_multiples_of_three_and_remainder_two_are_unchanged() {
        // Stored runs carry these names; nothing that was already exact may be renamed.
        assert_eq!(
            placeholder_names("FE", 20, 2),
            ["FE01", "FE02", "FE03", "FE04", "FE05", "FE06", "FE07S"]
        );
        assert_eq!(
            placeholder_names("ELEC", 9, 3),
            ["ELEC001", "ELEC002", "ELEC003"]
        );
    }

    #[test]
    fn any_real_shortfall_is_covered_so_a_plan_is_never_left_short() {
        assert!(elective_placeholders(0.0).is_empty());
        assert!(elective_placeholders(-3.0).is_empty());
        assert!(
            elective_placeholders(0.0005).is_empty(),
            "float noise is not a gap"
        );
        // A half-unit gap at a course-unit school used to be ignored.
        assert_eq!(elective_placeholders(0.5), ["ELEC001S1"]);
        assert_eq!(elective_placeholders(1.0), ["ELEC001S1"]);
        assert_eq!(elective_placeholders(4.0), ["ELEC001", "ELEC002S1"]);
        assert_eq!(elective_placeholders(4.3), ["ELEC001", "ELEC002S"]);
        for need in [0.5_f32, 1.0, 2.5, 4.3, 12.5, 13.0] {
            let got: f32 = elective_placeholders(need)
                .iter()
                .map(|n| placeholder_credits(n))
                .sum();
            assert!(
                got >= need - f32::EPSILON && got < need + 1.0,
                "{need} -> {got}"
            );
        }
    }
}
