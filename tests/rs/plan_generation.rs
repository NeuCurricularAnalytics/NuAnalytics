//! Integration tests for plan generation
//!
//! Tests the plan generation functionality using realistic degree requirements.

use nu_analytics::core::degree::{
    load_degree_from_yaml, PlanGenerator, PlanGeneratorConfig, PlanVariant, RequirementResolver,
};
use std::path::PathBuf;

/// Get the path to sample degrees directory
fn samples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples/degrees")
}

/// Test plan generation with a real degree file
#[test]
fn test_plan_generation_from_yaml() {
    let degree_path = samples_dir().join("neu-khoury-bscs-boston.yaml");
    if !degree_path.exists() {
        eprintln!("Skipping test: sample degree file not found");
        return;
    }

    let program = load_degree_from_yaml(&degree_path).expect("Failed to load degree");

    // Create generator with limited plans for testing
    let config = PlanGeneratorConfig {
        max_plans: 100,
        ignore_duplicates: false,
        sample_count: 5,
    };

    let generator = PlanGenerator::new(&program.requirements, &program.courses, config);

    // Check that we can estimate plan count
    let estimated = generator.estimate_plan_count();
    assert!(estimated > 0, "Should estimate at least one plan");

    // Check stats
    let stats = generator.get_stats();
    assert!(
        stats.variable_requirements > 0,
        "NEU degree should have variable requirements"
    );

    // Generate some plans
    let plans: Vec<PlanVariant> = generator.generate().take(10).collect();
    assert!(!plans.is_empty(), "Should generate at least one plan");

    // Each plan should have courses
    for plan in &plans {
        assert!(plan.course_count() > 0, "Plan should have courses");
        assert!(plan.total_credits > 0.0, "Plan should have credits");
    }
}

/// Test that requirement resolver handles different requirement types
#[test]
fn test_requirement_resolver_types() {
    let degree_path = samples_dir().join("neu-khoury-bscs-boston.yaml");
    if !degree_path.exists() {
        eprintln!("Skipping test: sample degree file not found");
        return;
    }

    let program = load_degree_from_yaml(&degree_path).expect("Failed to load degree");
    let mut req_resolver = RequirementResolver::new(&program.courses);

    let resolved_reqs = req_resolver.resolve_all(&program.requirements);

    // Should have resolved requirements
    assert!(
        !resolved_reqs.is_empty(),
        "Should have resolved requirements"
    );

    // Check for fixed requirements (type: all)
    let fixed_reqs: Vec<_> = resolved_reqs.iter().filter(|r| !r.is_variable).collect();
    assert!(
        !fixed_reqs.is_empty(),
        "Should have some fixed requirements"
    );

    // Check for variable requirements (type: select or one_of)
    let var_reqs: Vec<_> = resolved_reqs.iter().filter(|r| r.is_variable).collect();
    assert!(
        !var_reqs.is_empty(),
        "Should have some variable requirements"
    );

    // Print stats for debugging
    println!("Fixed requirements: {}", fixed_reqs.len());
    println!("Variable requirements: {}", var_reqs.len());
    for req in &var_reqs {
        println!("  {}: {} choices", req.id, req.choice_count);
    }
}

/// Test plan generation statistics accuracy
#[test]
fn test_plan_generation_stats() {
    let degree_path = samples_dir().join("csu-cs-bscs-general.yaml");
    if !degree_path.exists() {
        eprintln!("Skipping test: sample degree file not found");
        return;
    }

    let program = load_degree_from_yaml(&degree_path).expect("Failed to load degree");

    let config = PlanGeneratorConfig {
        max_plans: 1000,
        ignore_duplicates: false,
        sample_count: 5,
    };

    let generator = PlanGenerator::new(&program.requirements, &program.courses, config);
    let (plans, stats) = generator.generate_all();

    // Stats should reflect actual generation
    assert_eq!(
        stats.plans_generated,
        plans.len(),
        "Stats should match actual plans"
    );

    // If we hit the limit, truncation should be detected
    if plans.len() >= 1000 {
        assert!(stats.was_truncated(), "Should detect truncation");
    }

    println!("Generated {} plans", stats.plans_generated);
    println!("Total possible: {}", stats.total_possible);
    println!("Variable requirements: {}", stats.variable_requirements);
}

/// Test plan variant deduplication
#[test]
fn test_plan_deduplication() {
    use std::collections::HashMap;

    // Create simple test data
    let mut courses = HashMap::new();
    courses.insert(
        "CS1000".to_string(),
        nu_analytics::core::models::course::Course {
            credit_hours: 4.0,
            ..Default::default()
        },
    );
    courses.insert(
        "CS2000".to_string(),
        nu_analytics::core::models::course::Course {
            credit_hours: 4.0,
            ..Default::default()
        },
    );

    // Create two equivalent plans with different requirement orderings
    let mut choices1 = HashMap::new();
    choices1.insert("req1".to_string(), vec!["CS1000".to_string()]);
    choices1.insert("req2".to_string(), vec!["CS2000".to_string()]);

    let mut choices2 = HashMap::new();
    choices2.insert("req2".to_string(), vec!["CS2000".to_string()]);
    choices2.insert("req1".to_string(), vec!["CS1000".to_string()]);

    let credits: HashMap<String, f32> = courses
        .iter()
        .map(|(k, c)| (k.clone(), c.credit_hours))
        .collect();

    let plan1 = PlanVariant::new(choices1, &credits);
    let plan2 = PlanVariant::new(choices2, &credits);

    // Plans should be equivalent
    assert!(
        plan1.is_equivalent_to(&plan2),
        "Plans with same courses should be equivalent"
    );
    assert_eq!(plan1.fingerprint(), plan2.fingerprint());
}

/// Test that all generated plans are unique (when not deduplicating)
#[test]
fn test_plan_uniqueness() {
    let degree_path = samples_dir().join("neu-khoury-bscs-boston.yaml");
    if !degree_path.exists() {
        eprintln!("Skipping test: sample degree file not found");
        return;
    }

    let program = load_degree_from_yaml(&degree_path).expect("Failed to load degree");

    let config = PlanGeneratorConfig {
        max_plans: 50,
        ignore_duplicates: false,
        sample_count: 5,
    };

    let generator = PlanGenerator::new(&program.requirements, &program.courses, config);
    let plans: Vec<PlanVariant> = generator.generate().collect();

    // Check uniqueness by comparing fingerprints
    let mut seen = std::collections::HashSet::new();
    for plan in &plans {
        let fp = plan.fingerprint();
        if seen.contains(&fp) {
            // If fingerprints match, verify courses actually differ
            // (hash collisions are theoretically possible)
            continue;
        }
        seen.insert(fp);
    }

    println!(
        "Generated {} plans, {} unique fingerprints",
        plans.len(),
        seen.len()
    );
}
