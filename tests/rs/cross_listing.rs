//! Tests for cross-listing validation and helper methods

use nu_analytics::core::degree::{load_degree_from_yaml, parse_degree_yaml, DegreeProgram};
use nu_analytics::core::{validate_degree_program, ValidationError, ValidationWarning};

#[test]
fn test_cross_listing_validation_csu_degree() {
    // CSU degree has CS201 and PHIL201 cross-listed
    let program = load_degree_from_yaml("samples/degrees/csu-cs-bscs-general.yaml")
        .expect("Failed to load CSU degree");

    let result = validate_degree_program(&program);

    // Should not have any cross-listing errors since they're bidirectional
    let cross_listing_errors: Vec<_> = result
        .errors
        .iter()
        .filter(|e| matches!(e, ValidationError::UnidirectionalCrossListing { .. }))
        .collect();

    assert!(
        cross_listing_errors.is_empty(),
        "Should not have cross-listing errors in CSU degree: {cross_listing_errors:?}"
    );
}

#[test]
fn test_course_is_cross_listed() {
    let program = load_degree_from_yaml("samples/degrees/csu-cs-bscs-general.yaml")
        .expect("Failed to load CSU degree");

    // CS201 should be cross-listed with PHIL201
    let cs201 = program.courses.get("CS201").expect("CS201 should exist");
    assert!(cs201.is_cross_listed(), "CS201 should be cross-listed");
    assert!(
        cs201.is_cross_listed_with("PHIL201"),
        "CS201 should be cross-listed with PHIL201"
    );

    let cross_listed = cs201.cross_listed_courses();
    assert_eq!(cross_listed, vec!["PHIL201"]);
}

#[test]
fn test_degree_program_cross_listing_helpers() {
    let program = load_degree_from_yaml("samples/degrees/csu-cs-bscs-general.yaml")
        .expect("Failed to load CSU degree");

    // Test get_cross_listed_courses
    let cs201_equivalents = program.get_cross_listed_courses("CS201");
    assert_eq!(
        cs201_equivalents,
        vec!["PHIL201"],
        "CS201 should be cross-listed with PHIL201"
    );

    // Test are_cross_listed
    assert!(
        program.are_cross_listed("CS201", "PHIL201"),
        "CS201 and PHIL201 should be cross-listed"
    );
    assert!(
        program.are_cross_listed("PHIL201", "CS201"),
        "Cross-listing check should be symmetric"
    );
    assert!(
        !program.are_cross_listed("CS201", "CS165"),
        "CS201 and CS165 should not be cross-listed"
    );

    // Test get_equivalent_course_set
    let cs201_set = program.get_equivalent_course_set("CS201");
    assert!(
        cs201_set.contains(&"CS201".to_string()),
        "Set should include the original course"
    );
    assert!(
        cs201_set.contains(&"PHIL201".to_string()),
        "Set should include cross-listed courses"
    );
}

#[test]
fn test_unidirectional_cross_listing_error() {
    // Create a test degree with unidirectional cross-listing
    let yaml = r#"
degree:
  name: Test Degree
  degree_type: BS
  system_type: semester
  total_credits: 120
  allow_double_counting: false

requirements: {}

courses:
  CS101:
    subject: CS
    number: "101"
    title: Intro to CS
    credits: 3
    cross_listed_as: [PHIL101]  # Lists PHIL101 but...

  PHIL101:
    subject: PHIL
    number: "101"
    title: Intro to CS
    credits: 3
    # Does NOT list CS101 back - this is an error!
"#;

    let program: DegreeProgram = parse_degree_yaml(yaml).expect("Should parse YAML");

    let result = validate_degree_program(&program);

    // Should have an error for unidirectional cross-listing
    let cross_listing_errors: Vec<_> = result
        .errors
        .iter()
        .filter(|e| matches!(e, ValidationError::UnidirectionalCrossListing { .. }))
        .collect();

    assert!(
        !cross_listing_errors.is_empty(),
        "Should have error for unidirectional cross-listing"
    );

    // Check the specific error
    if let Some(ValidationError::UnidirectionalCrossListing {
        course_key,
        cross_listed_key,
    }) = cross_listing_errors.first()
    {
        assert_eq!(course_key, "CS101");
        assert_eq!(cross_listed_key, "PHIL101");
    } else {
        panic!("Expected UnidirectionalCrossListing error");
    }
}

#[test]
fn test_missing_cross_listed_course_warning() {
    // Create a test degree where cross-listed course doesn't exist
    let yaml = r#"
degree:
  name: Test Degree
  degree_type: BS
  system_type: semester
  total_credits: 120
  allow_double_counting: false

requirements: {}

courses:
  CS101:
    subject: CS
    number: "101"
    title: Intro to CS
    credits: 3
    cross_listed_as: [PHIL101]  # PHIL101 doesn't exist!
"#;

    let program: DegreeProgram = parse_degree_yaml(yaml).expect("Should parse YAML");

    let result = validate_degree_program(&program);

    // Should have a warning for missing cross-listed course
    assert!(
        result
            .warnings
            .iter()
            .any(|w| matches!(w, ValidationWarning::MissingCrossListedCourse { .. })),
        "Should have warning for missing cross-listed course"
    );
}
