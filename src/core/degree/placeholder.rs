//! Placeholder course names and the credits they stand for.
//!
//! A placeholder is a synthetic course standing in for credits no specific course was
//! chosen for: `ELEC001` from the generic filler, `FE01` from a wildcard requirement such
//! as free electives. It has no catalog entry, so **its name is the only record of its
//! credits**, and this module is the single place that reads or writes that encoding.
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

/// A remainder at or below this is not worth a placeholder. Preserved from the fillers
/// this replaces, which never added one for a fractional sliver.
const IGNORED_REMAINDER: f32 = 0.5;

/// Credits a placeholder name stands for.
///
/// `…S<n>` is `n` credits, bare `…S` is 2, anything else 3. Only meaningful for names that
/// are placeholders; a real course's credits come from the catalog.
#[must_use]
pub fn placeholder_credits(name: &str) -> f32 {
    if let Some(pos) = name.rfind('S') {
        let tail = &name[pos + 1..];
        if tail.is_empty() {
            return SHORT_PLACEHOLDER_CREDITS;
        }
        if tail.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(n) = tail.parse::<u16>() {
                return f32::from(n);
            }
        }
    }
    FULL_PLACEHOLDER_CREDITS
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
/// A remainder of [`IGNORED_REMAINDER`] or less adds nothing, as before; anything larger is
/// covered in whole credits, so an integral need is met exactly and a fractional one is
/// overshot by less than a credit.
#[must_use]
pub fn placeholder_names_for(prefix: &str, credits_needed: f32, width: usize) -> Vec<String> {
    if credits_needed <= IGNORED_REMAINDER {
        return Vec::new();
    }
    let whole = credits_needed.floor();
    let fraction = credits_needed - whole;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let credits = if fraction > IGNORED_REMAINDER {
        whole as u32 + 1
    } else {
        whole as u32
    };
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
    fn every_whole_amount_round_trips_exactly() {
        for credits in 0..=40 {
            let total: f32 = placeholder_names("FE", credits, 2)
                .iter()
                .map(|n| placeholder_credits(n))
                .sum();
            assert!(
                (total - f32::from(u16::try_from(credits).unwrap())).abs() < f32::EPSILON,
                "{credits} -> {total}"
            );
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
    fn fractional_needs_round_up_but_ignore_a_sliver() {
        assert!(elective_placeholders(0.4).is_empty());
        assert!(elective_placeholders(0.0).is_empty());
        assert!(elective_placeholders(-3.0).is_empty());
        assert_eq!(elective_placeholders(1.0), ["ELEC001S1"]);
        assert_eq!(elective_placeholders(4.0), ["ELEC001", "ELEC002S1"]);
        // 4.3: the sliver is ignored, matching the fillers this replaced.
        assert_eq!(elective_placeholders(4.3), ["ELEC001", "ELEC002S1"]);
        // 4.6: covered in whole credits, so 5.
        assert_eq!(elective_placeholders(4.6), ["ELEC001", "ELEC002S"]);
    }
}
