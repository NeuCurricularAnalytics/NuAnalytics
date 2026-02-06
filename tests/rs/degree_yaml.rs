//! Degree YAML ingestion integration tests

use nu_analytics::core::degree::{
    load_degree_from_yaml, parse_degree_yaml, save_degree_to_yaml, RequirementType,
};
use std::path::Path;

#[test]
fn test_load_uhm_degree_yaml() {
    let result = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml");
    assert!(result.is_ok(), "Failed to load UHM degree YAML");

    let program = result.unwrap();

    // Verify metadata
    assert_eq!(program.degree.id, Some("uhm-ics-bscs-general".to_string()));
    assert_eq!(
        program.degree.institution,
        Some("University of Hawaiʻi at Mānoa".to_string())
    );
    assert_eq!(program.degree.total_credits, Some(120));
    #[allow(clippy::float_cmp)]
    {
        assert_eq!(program.degree.gpa_minimum, Some(2.0));
    }
    assert_eq!(program.degree.allow_double_counting, Some(false));

    // Verify requirements exist
    assert!(
        !program.requirements.is_empty(),
        "Requirements should not be empty"
    );

    // Verify courses exist
    assert!(!program.courses.is_empty(), "Courses should not be empty");

    // Spot-check some known courses
    assert!(program.courses.contains_key("ICS111"), "Should have ICS111");
    assert!(program.courses.contains_key("ICS211"), "Should have ICS211");
    assert!(
        program.courses.contains_key("MATH215"),
        "Should have MATH215"
    );
}

#[test]
fn test_load_csu_degree_yaml() {
    let result = load_degree_from_yaml("samples/degrees/csu-cs-bscs-general.yaml");
    assert!(result.is_ok(), "Failed to load CSU degree YAML");

    let program = result.unwrap();

    // Verify metadata
    assert_eq!(program.degree.id, Some("csu-cs-bscs-general".to_string()));
    assert_eq!(
        program.degree.institution,
        Some("Colorado State University".to_string())
    );
    assert_eq!(program.degree.total_credits, Some(120));

    // Verify requirements and courses exist
    assert!(
        !program.requirements.is_empty(),
        "Requirements should not be empty"
    );
    assert!(!program.courses.is_empty(), "Courses should not be empty");

    // Spot-check some known courses
    assert!(program.courses.contains_key("CO150"), "Should have CO150");
    assert!(program.courses.contains_key("CS162"), "Should have CS162");
}

#[test]
fn test_load_neu_degree_yaml() {
    let result = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml");
    assert!(result.is_ok(), "Failed to load NEU degree YAML");

    let program = result.unwrap();

    // Verify metadata
    assert_eq!(
        program.degree.id,
        Some("neu-khoury-bscs-boston".to_string())
    );
    assert_eq!(
        program.degree.institution,
        Some("Northeastern University".to_string())
    );

    // Verify requirements and courses exist
    assert!(
        !program.requirements.is_empty(),
        "Requirements should not be empty"
    );
    assert!(!program.courses.is_empty(), "Courses should not be empty");
}

#[test]
fn test_degree_metadata_fields() {
    let result = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml");
    let program = result.unwrap();

    // Verify optional fields are present or correctly set
    assert!(program.degree.source_url.is_some());
    assert!(program.degree.upper_division_credits.is_some());
    assert_eq!(program.degree.upper_division_credits, Some(45));
    assert!(program.degree.in_major_credits.is_some());
    assert_eq!(program.degree.in_major_credits, Some(57));
    assert!(program.degree.gpa_minimum.is_some());
    assert!(program.degree.major_subjects.is_some());
}

#[test]
fn test_course_key_generation() {
    let result = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml");
    let program = result.unwrap();

    // Verify that courses can generate their keys
    if let Some(course) = program.courses.get("ICS111") {
        assert_eq!(course.key(), "ICS111");
        assert_eq!(course.prefix, "ICS");
        assert_eq!(course.number, "111");
    } else {
        panic!("ICS111 course not found");
    }
}

#[test]
fn test_requirement_types_parsed() {
    let result = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml");
    let program = result.unwrap();

    // Count requirements by type
    let all_reqs = program
        .requirements
        .values()
        .filter(|r| r.req_type == RequirementType::All)
        .count();

    let select_reqs = program
        .requirements
        .values()
        .filter(|r| r.req_type == RequirementType::Select)
        .count();

    let one_of_reqs = program
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

    let program = result.unwrap();
    assert_eq!(program.degree.id, Some("uhm-ics-bscs-general".to_string()));
    assert!(!program.courses.is_empty());
}

#[test]
fn test_round_trip_degree_yaml_exports() -> Result<(), Box<dyn std::error::Error>> {
    let degrees_dir = Path::new("samples/degrees");
    let temp_dir = tempfile::TempDir::new()?;

    for entry in std::fs::read_dir(degrees_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|ext| ext.to_str()) != Some("yaml") {
            continue;
        }

        let program = load_degree_from_yaml(&path)?;
        let output_path = temp_dir.path().join(path.file_name().unwrap_or_default());

        save_degree_to_yaml(&program, &output_path)?;
        let reloaded = load_degree_from_yaml(&output_path)?;

        let original = serde_json::to_value(&program)?;
        let roundtrip = serde_json::to_value(&reloaded)?;

        assert_eq!(
            original,
            roundtrip,
            "Round-trip YAML mismatch for {}",
            path.display()
        );
    }

    Ok(())
}

#[test]
fn test_courses_have_expected_fields() {
    let result = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml");
    let program = result.unwrap();

    // Check a course with prerequisites
    if let Some(course) = program.courses.get("ICS311") {
        assert_eq!(course.prefix, "ICS");
        assert_eq!(course.number, "311");
        assert!(course.name.contains("Algorithms") || !course.name.is_empty());
        assert!(course.credit_hours > 0.0 || course.credit_range.is_some());
        // ICS311 should have prerequisites
        assert!(
            !course.prerequisites.is_empty() || course.prerequisites_raw.is_some(),
            "ICS311 should have prerequisites"
        );
    } else {
        panic!("ICS311 not found in courses");
    }
}

#[test]
fn test_yaml_course_to_unified_course() {
    let result = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml");
    let program = result.unwrap();

    // Courses are already in unified model
    if let Some(course) = program.courses.get("ICS311") {
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

        // Prerequisites Vec contains resolved prerequisites
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

    // Degree is already in unified model - no conversion needed
    let unified_degree = &degree_program.degree;

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
fn test_all_courses_in_unified_model() {
    let result = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml");
    let program = result.unwrap();

    // Every course should be in unified model
    for (key, course) in &program.courses {
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

    // Degree is already in unified model
    let unified_degree = &degree_program.degree;
    assert_eq!(unified_degree.degree_id(), "csu-cs-bscs-general");
    assert_eq!(unified_degree.total_credits, Some(120));

    // Course is already in unified model
    if let Some(course) = degree_program.courses.get("CS162") {
        assert_eq!(course.prefix, "CS");
        assert_eq!(course.number, "162");
    }
}

#[test]
fn test_convert_neu_degree_to_unified_models() {
    let result = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml");
    let degree_program = result.unwrap();

    // Degree is already in unified model
    let unified_degree = &degree_program.degree;
    assert_eq!(unified_degree.degree_id(), "neu-khoury-bscs-boston");
    assert_eq!(
        unified_degree.institution,
        Some("Northeastern University".to_string())
    );

    // All courses should already be in unified model
    for (key, course) in &degree_program.courses {
        assert_eq!(course.key(), *key);
    }
}
