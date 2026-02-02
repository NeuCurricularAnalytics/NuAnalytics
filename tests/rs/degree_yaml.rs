//! Degree YAML ingestion integration tests

use nu_analytics::core::degree::{load_degree_from_yaml, parse_degree_yaml, RequirementType};

#[test]
fn test_load_uhm_degree_yaml() {
    let result = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml");
    assert!(result.is_ok(), "Failed to load UHM degree YAML");

    let degree = result.unwrap();

    // Verify metadata
    assert_eq!(degree.degree.id, "uhm-ics-bscs-general");
    assert_eq!(degree.degree.institution, "University of Hawaiʻi at Mānoa");
    assert_eq!(degree.degree.total_credits, 120);
    #[allow(clippy::float_cmp)]
    {
        assert_eq!(degree.degree.gpa_minimum, 2.0);
    }
    assert!(!degree.degree.allow_double_counting);

    // Verify requirements exist
    assert!(
        !degree.requirements.is_empty(),
        "Requirements should not be empty"
    );

    // Verify courses exist
    assert!(!degree.courses.is_empty(), "Courses should not be empty");

    // Spot-check some known courses
    assert!(degree.courses.contains_key("ICS111"), "Should have ICS111");
    assert!(degree.courses.contains_key("ICS211"), "Should have ICS211");
    assert!(
        degree.courses.contains_key("MATH215"),
        "Should have MATH215"
    );
}

#[test]
fn test_load_csu_degree_yaml() {
    let result = load_degree_from_yaml("samples/degrees/csu-cs-bscs-general.yaml");
    assert!(result.is_ok(), "Failed to load CSU degree YAML");

    let degree = result.unwrap();

    // Verify metadata
    assert_eq!(degree.degree.id, "csu-cs-bscs-general");
    assert_eq!(degree.degree.institution, "Colorado State University");
    assert_eq!(degree.degree.total_credits, 120);

    // Verify requirements and courses exist
    assert!(
        !degree.requirements.is_empty(),
        "Requirements should not be empty"
    );
    assert!(!degree.courses.is_empty(), "Courses should not be empty");

    // Spot-check some known courses
    assert!(degree.courses.contains_key("CO150"), "Should have CO150");
    assert!(degree.courses.contains_key("CS162"), "Should have CS162");
}

#[test]
fn test_load_neu_degree_yaml() {
    let result = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml");
    assert!(result.is_ok(), "Failed to load NEU degree YAML");

    let degree = result.unwrap();

    // Verify metadata
    assert_eq!(degree.degree.id, "neu-khoury-bscs-boston");
    assert_eq!(degree.degree.institution, "Northeastern University");

    // Verify requirements and courses exist
    assert!(
        !degree.requirements.is_empty(),
        "Requirements should not be empty"
    );
    assert!(!degree.courses.is_empty(), "Courses should not be empty");
}

#[test]
fn test_degree_metadata_fields() {
    let result = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml");
    let degree = result.unwrap();

    // Verify optional fields are present or correctly set
    assert!(degree.degree.source_url.is_some());
    assert!(degree.degree.upper_division_credits.is_some());
    assert_eq!(degree.degree.upper_division_credits, Some(45));
    assert!(degree.degree.in_major_credits.is_some());
    assert_eq!(degree.degree.in_major_credits, Some(57));
    assert!(degree.degree.grade_minimum.is_some());
    assert!(degree.degree.major_subjects.is_some());
}

#[test]
fn test_course_key_generation() {
    let result = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml");
    let degree = result.unwrap();

    // Verify that courses can generate their keys
    if let Some(course) = degree.courses.get("ICS111") {
        assert_eq!(course.course_key(), "ICS111");
        assert_eq!(course.subject, "ICS");
        assert_eq!(course.number, "111");
    } else {
        panic!("ICS111 course not found");
    }
}

#[test]
fn test_requirement_types_parsed() {
    let result = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml");
    let degree = result.unwrap();

    // Count requirements by type
    let all_reqs = degree
        .requirements
        .values()
        .filter(|r| r.req_type == RequirementType::All)
        .count();

    let select_reqs = degree
        .requirements
        .values()
        .filter(|r| r.req_type == RequirementType::Select)
        .count();

    let one_of_reqs = degree
        .requirements
        .values()
        .filter(|r| r.req_type == RequirementType::OneOf)
        .count();

    // Should have at least some of each type
    assert!(all_reqs > 0, "Should have some 'all' type requirements");
    assert!(
        select_reqs > 0,
        "Should have some 'select' type requirements"
    );
    assert!(
        one_of_reqs > 0,
        "Should have some 'one_of' type requirements"
    );
}

#[test]
fn test_parse_yaml_from_string() {
    // Test string parsing (simulating network/database source)
    let yaml_content = std::fs::read_to_string("samples/degrees/uhm-ics-bscs-general.yaml")
        .expect("Failed to read file");

    let result = parse_degree_yaml(&yaml_content);
    assert!(result.is_ok(), "Failed to parse YAML from string");

    let degree = result.unwrap();
    assert_eq!(degree.degree.id, "uhm-ics-bscs-general");
    assert!(!degree.courses.is_empty());
}

#[test]
fn test_courses_have_expected_fields() {
    let result = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml");
    let degree = result.unwrap();

    // Check a course with prerequisites
    if let Some(course) = degree.courses.get("ICS311") {
        assert_eq!(course.subject, "ICS");
        assert_eq!(course.number, "311");
        assert!(course.title.contains("Algorithms") || !course.title.is_empty());
        assert!(course.credits.is_some() || course.credit_range.is_some());
        // ICS311 should have prerequisites
        assert!(
            course.prerequisites.is_some(),
            "ICS311 should have prerequisites"
        );
    } else {
        panic!("ICS311 not found in courses");
    }
}

#[test]
fn test_yaml_course_to_unified_course() {
    let result = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml");
    let degree = result.unwrap();

    // Convert a YAML course to unified Course model
    if let Some(yaml_course) = degree.courses.get("ICS311") {
        let course = yaml_course.to_course();

        // Verify unified model fields are properly populated
        assert_eq!(course.key(), "ICS311");
        assert_eq!(course.prefix, "ICS");
        assert_eq!(course.number, "311");
        assert!(!course.name.is_empty(), "Course name should be populated");

        // Check that prerequisites_raw is preserved
        assert!(
            course.prerequisites_raw.is_some(),
            "prerequisites_raw should be set"
        );

        // Prerequisites Vec is empty until parsed
        assert!(
            course.prerequisites.is_empty(),
            "prerequisites Vec should be empty until parsed"
        );

        // Credit hours should be set
        assert!(course.credit_hours > 0.0 || course.credit_range.is_some());
    } else {
        panic!("ICS311 not found");
    }
}

#[test]
fn test_degree_meta_to_unified_degree() {
    let result = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml");
    let degree_program = result.unwrap();

    // Convert DegreeMeta to unified Degree model
    let unified_degree = degree_program.degree.to_degree();

    // Verify unified model fields
    assert_eq!(unified_degree.degree_id(), "uhm-ics-bscs-general");
    assert_eq!(
        unified_degree.institution,
        Some("University of Hawaiʻi at Mānoa".to_string())
    );
    assert_eq!(unified_degree.total_credits, Some(120));
    assert_eq!(unified_degree.gpa_minimum, Some(2.0));
    assert_eq!(unified_degree.allow_double_counting, Some(false));
    assert_eq!(unified_degree.upper_division_credits, Some(45));

    // Core fields should be set appropriately
    assert!(unified_degree.name.contains("Computer Science"));
}

#[test]
fn test_all_courses_convert_to_unified_model() {
    let result = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml");
    let degree = result.unwrap();

    // Every YamlCourse should convert cleanly to Course
    for (key, yaml_course) in &degree.courses {
        let course = yaml_course.to_course();

        // Key should match
        assert_eq!(course.key(), *key, "Course key mismatch for {key}");

        // Required fields should be set
        assert!(!course.prefix.is_empty(), "prefix missing for {key}");
        assert!(!course.number.is_empty(), "number missing for {key}");
        assert!(!course.name.is_empty(), "name (title) missing for {key}");
    }
}

#[test]
fn test_convert_csu_degree_to_unified_models() {
    let result = load_degree_from_yaml("samples/degrees/csu-cs-bscs-general.yaml");
    let degree_program = result.unwrap();

    // Convert degree
    let unified_degree = degree_program.degree.to_degree();
    assert_eq!(unified_degree.degree_id(), "csu-cs-bscs-general");
    assert_eq!(unified_degree.total_credits, Some(120));

    // Convert a sample course
    if let Some(yaml_course) = degree_program.courses.get("CS162") {
        let course = yaml_course.to_course();
        assert_eq!(course.prefix, "CS");
        assert_eq!(course.number, "162");
    }
}

#[test]
fn test_convert_neu_degree_to_unified_models() {
    let result = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml");
    let degree_program = result.unwrap();

    // Convert degree
    let unified_degree = degree_program.degree.to_degree();
    assert_eq!(unified_degree.degree_id(), "neu-khoury-bscs-boston");
    assert_eq!(
        unified_degree.institution,
        Some("Northeastern University".to_string())
    );

    // All courses should convert
    for (key, yaml_course) in &degree_program.courses {
        let course = yaml_course.to_course();
        assert_eq!(course.key(), *key);
    }
}
