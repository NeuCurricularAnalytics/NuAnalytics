//! Vendored degree fixtures and the target-course analysis helper built on them.
//!
//! The JSON lives in `tests/assets/degrees/`; that directory's `Readme.md` records which
//! upstream corpus build each file is and how to refresh one. Fixtures are `include_str!`d
//! once here and shared, so a fixture used by two test modules is embedded in the test
//! binary once rather than per module.

use nu_analytics::mcp::tools::analyze::{execute, AnalysisResponse, TargetCourseStats};

/// Plan cap used by the target-course cases, so their figures stay comparable.
pub const MAX_PLANS: usize = 200;

pub const TULANE: &str = include_str!(
    "../assets/degrees/tulane-university-of-louisiana-computer-science-bs.unified.json"
);
pub const COC: &str =
    include_str!("../assets/degrees/college-of-charleston-computer-science-b-s.unified.json");
pub const BOWDOIN: &str =
    include_str!("../assets/degrees/bowdoin-college-computer-science.unified.json");
pub const NMSU: &str = include_str!(
    "../assets/degrees/new-mexico-state-university-main-campus-computer-science-bachelor-of-science.unified.json"
);
pub const LIBERTY: &str =
    include_str!("../assets/degrees/liberty-university-computer-science-b-s.unified.json");
pub const RIC: &str =
    include_str!("../assets/degrees/rhode-island-college-artificial-intelligence-bs.unified.json");
pub const CALSTATELA: &str = include_str!(
    "../assets/degrees/california-state-university-los-angeles-computer-science-b-s.unified.json"
);
pub const METRO: &str = include_str!(
    "../assets/degrees/metropolitan-state-university-of-denver-computer-science-major-b-s.unified.json"
);
pub const WKU: &str = include_str!(
    "../assets/degrees/western-kentucky-university-computer-science-bachelor-of-science.unified.json"
);
pub const TXSTATE: &str =
    include_str!("../assets/degrees/texas-state-university-computer-science-b-s.unified.json");
pub const ASU: &str =
    include_str!("../assets/degrees/arizona-state-university-computer-science-bs.unified.json");
pub const SYRACUSE: &str =
    include_str!("../assets/degrees/syracuse-university-computer-science-bs.unified.json");
pub const BELLEVUE: &str = include_str!(
    "../assets/degrees/bellevue-college-software-development-bas-artificial-intelligence-concentration.unified.json"
);

/// Analyze `degree_json` asking where `course` lands.
///
/// `label` identifies the degree in failure messages — without it a panic from one of the
/// thirteen fixtures does not say which one. Wrapping `execute` also keeps its five
/// type-identical filler arguments (three `bool`s, two `Option<u64>`s) in one place, where
/// they can only be mis-ordered once.
pub fn analyze_target(
    label: &str,
    degree_json: &str,
    max_plans: usize,
    course: &str,
) -> AnalysisResponse {
    let response = execute(
        degree_json,
        Some(max_plans),
        None,
        false,
        None,
        false,
        false,
        None,
        None,
        Some(course),
    );
    assert!(
        response.success,
        "{label}: analyzing for {course} failed: {:?}",
        response.error
    );
    response
}

/// `target_course_stats` for one degree/course pair.
///
/// Panics rather than returning an `Option`: every caller requests a target course, and
/// the field is populated whenever one is requested.
pub fn target_stats(
    label: &str,
    degree_json: &str,
    max_plans: usize,
    course: &str,
) -> TargetCourseStats {
    analyze_target(label, degree_json, max_plans, course)
        .target_course_stats
        .unwrap_or_else(|| {
            panic!("{label}: requesting target_course={course} must populate target_course_stats")
        })
}

/// Read one of the bundled sample degrees from `samples/degrees/`.
///
/// Read at runtime rather than `include_str!`d, matching the other integration tests
/// (`degree_yaml`, `planner`), which resolve sample paths against the crate root.
pub fn bundled_sample(name: &str) -> String {
    let path = format!("samples/degrees/{name}");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading bundled sample {path}: {e}"))
}
