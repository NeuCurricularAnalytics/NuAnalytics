//! Integration tests for planner CSV parsing

use nu_analytics::core::planner::csv_parser::parse_curriculum_csv;

#[test]
fn test_parse_colostate_cs_curriculum() {
    let csv_path = "samples/plans/Colostate_CSDegree.csv";

    let result = parse_curriculum_csv(csv_path);
    assert!(
        result.is_ok(),
        "Failed to parse curriculum CSV: {:?}",
        result.err()
    );

    let school = result.unwrap();

    // Verify school name
    assert_eq!(school.name, "Colorado State University");

    // Verify degree was created
    assert_eq!(school.degrees.len(), 1);
    assert_eq!(school.degrees[0].name, "Colostate_CS Degree");
    assert_eq!(school.degrees[0].degree_type, "BS");
    assert_eq!(school.degrees[0].cip_code, "11.0701");

    // Verify courses were loaded (should have multiple courses)
    let courses = school.courses();
    assert!(
        courses.len() > 20,
        "Expected more than 20 courses, got {}",
        courses.len()
    );

    // Verify specific courses exist
    assert!(school.get_course("CS165").is_some(), "CS165 should exist");
    assert!(school.get_course("CS220").is_some(), "CS220 should exist");
    assert!(
        school.get_course("MATH156").is_some(),
        "MATH156 should exist"
    );

    // Verify course details for CS220
    let cs220 = school.get_course("CS220").unwrap();
    assert_eq!(cs220.name, "Discrete Structures and their Applications");
    assert_eq!(cs220.prefix, "CS");
    assert_eq!(cs220.number, "220");
    assert!((cs220.credit_hours - 4.0).abs() < f32::EPSILON);

    // Verify prerequisites were resolved correctly
    assert!(
        cs220.prerequisites.contains(&"CS150B".to_string()),
        "CS220 should have CS150B as prerequisite"
    );
    assert!(
        cs220.prerequisites.contains(&"MATH156".to_string()),
        "CS220 should have MATH156 as prerequisite"
    );

    // Verify a course with prerequisites
    let cs165 = school.get_course("CS165").unwrap();
    assert_eq!(cs165.name, "CS2--Data Structures");
    assert_eq!(
        cs165.prerequisites.len(),
        1,
        "CS165 should have 1 prerequisite"
    );
    assert!(
        cs165.prerequisites.contains(&"CS164".to_string()),
        "CS165 should have CS164 as prerequisite"
    );

    // Verify a course with no prerequisites
    let math156 = school.get_course("MATH156").unwrap();
    assert_eq!(
        math156.prerequisites.len(),
        0,
        "MATH156 should have no prerequisites"
    );

    // Verify course dependency validation passes
    let validation = school.validate_course_dependencies();
    assert!(
        validation.is_ok(),
        "Course dependencies should be valid: {:?}",
        validation.err()
    );
}

#[test]
fn test_parse_nonexistent_file() {
    let csv_path = "samples/plans/nonexistent.csv";

    let result = parse_curriculum_csv(csv_path);
    assert!(result.is_err(), "Should fail for nonexistent file");
}
