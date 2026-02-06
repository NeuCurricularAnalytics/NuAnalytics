//! Tests specifically for bundle and equivalent course syntax

use nu_analytics::core::degree::{load_degree_from_yaml, CourseReference};
use nu_analytics::core::{validate_degree_program, ValidationError};

#[test]
fn test_parse_course_bundle() {
    // Test parsing course bundles (lecture + lab)
    let bundle = CourseReference::parse("[CHEM161, CHEM161L]").unwrap();
    assert!(bundle.is_bundle());
    assert_eq!(bundle.courses(), vec!["CHEM161", "CHEM161L"]);
}

#[test]
fn test_parse_equivalent_courses() {
    // Test parsing equivalent/cross-listed courses
    let equiv = CourseReference::parse("{CS201, PHIL201}").unwrap();
    assert!(equiv.is_equivalent());
    assert_eq!(equiv.courses(), vec!["CS201", "PHIL201"]);
}

#[test]
fn test_validate_bundles_in_csu_degree() {
    // Load CSU degree which has many course bundles
    let program = load_degree_from_yaml("samples/degrees/csu-cs-bscs-general.yaml")
        .expect("Failed to load CSU degree");

    let result = validate_degree_program(&program);

    // Find the natural sciences requirement which has bundles
    let nat_sci_req = program.requirements.get("natural_sciences");
    assert!(
        nat_sci_req.is_some(),
        "Should have natural_sciences requirement"
    );

    if let Some(req) = nat_sci_req {
        if let Some(from) = &req.from {
            if let Some(courses) = &from.courses {
                // Check that we have bundle syntax
                let has_bundles = courses.iter().any(|c| c.starts_with('['));
                assert!(has_bundles, "Should have course bundles");

                // Parse each bundle and verify they're valid
                for course_ref_str in courses {
                    if course_ref_str.starts_with('[') {
                        let parsed = CourseReference::parse(course_ref_str);
                        assert!(parsed.is_ok(), "Should parse bundle: {course_ref_str}");

                        if let Ok(course_ref) = parsed {
                            assert!(course_ref.is_bundle());
                            // Each course in the bundle should exist
                            for course_key in course_ref.courses() {
                                assert!(
                                    program.courses.contains_key(course_key),
                                    "Bundle course {course_key} should exist"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    // Verify no missing course errors for bundles
    let bundle_errors: Vec<_> = result
        .errors
        .iter()
        .filter(|e| {
            matches!(e, ValidationError::MissingCourse { course_key, .. }
                if course_key.starts_with('['))
        })
        .collect();

    assert!(
        bundle_errors.is_empty(),
        "Should not have missing course errors for bundles: {bundle_errors:?}"
    );
}

#[test]
fn test_validate_equivalents_in_csu_degree() {
    // Load CSU degree which has equivalent courses
    let program = load_degree_from_yaml("samples/degrees/csu-cs-bscs-general.yaml")
        .expect("Failed to load CSU degree");

    let result = validate_degree_program(&program);

    // Find the ethics requirement which has equivalent courses
    let ethics_req = program.requirements.get("ethics_computing");
    assert!(
        ethics_req.is_some(),
        "Should have ethics_computing requirement"
    );

    if let Some(req) = ethics_req {
        if let Some(courses) = &req.courses {
            // Check that we have equivalent syntax
            let has_equivalents = courses.iter().any(|c| c.starts_with('{'));
            assert!(has_equivalents, "Should have equivalent courses");

            // Parse each equivalent set and verify they're valid
            for course_ref_str in courses {
                if course_ref_str.starts_with('{') {
                    let parsed = CourseReference::parse(course_ref_str);
                    assert!(parsed.is_ok(), "Should parse equivalent: {course_ref_str}");

                    if let Ok(course_ref) = parsed {
                        assert!(course_ref.is_equivalent());
                        // Each course in the equivalent set should exist
                        for course_key in course_ref.courses() {
                            assert!(
                                program.courses.contains_key(course_key),
                                "Equivalent course {course_key} should exist"
                            );
                        }
                    }
                }
            }
        }
    }

    // Verify no missing course errors for equivalents
    let equiv_errors: Vec<_> = result
        .errors
        .iter()
        .filter(|e| {
            matches!(e, ValidationError::MissingCourse { course_key, .. }
                if course_key.starts_with('{'))
        })
        .collect();

    assert!(
        equiv_errors.is_empty(),
        "Should not have missing course errors for equivalents: {equiv_errors:?}"
    );
}

#[test]
fn test_validate_equivalents_in_neu_degree() {
    // Load NEU degree which also has equivalent courses
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load NEU degree");

    let result = validate_degree_program(&program);

    // Check for equivalent course syntax in requirements
    let has_equivalents = program.requirements.values().any(|req| {
        req.courses
            .as_ref()
            .is_some_and(|courses| courses.iter().any(|c| c.starts_with('{')))
    });

    if has_equivalents {
        // Verify no missing course errors for equivalents
        let equiv_errors: Vec<_> = result
            .errors
            .iter()
            .filter(|e| {
                matches!(e, ValidationError::MissingCourse { course_key, .. }
                    if course_key.starts_with('{'))
            })
            .collect();

        assert!(
            equiv_errors.is_empty(),
            "Should not have missing course errors for equivalents: {equiv_errors:?}"
        );
    }
}
