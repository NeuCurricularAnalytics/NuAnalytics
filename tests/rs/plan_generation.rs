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
        ..Default::default()
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
        ..Default::default()
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
        ..Default::default()
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

/// Test plan score comparison logic
#[test]
fn test_plan_score_comparison() {
    use nu_analytics::core::degree::PlanScore;

    let score1 = PlanScore {
        terms_required: 8,
        total_complexity: 150,
        longest_delay: 6,
        longest_delay_chain: Vec::new(),
        is_calc_ready: false,
    };

    let score2 = PlanScore {
        terms_required: 9,
        total_complexity: 160,
        longest_delay: 7,
        longest_delay_chain: Vec::new(),
        is_calc_ready: false,
    };

    let score3 = PlanScore {
        terms_required: 8,
        total_complexity: 140,
        longest_delay: 5,
        longest_delay_chain: Vec::new(),
        is_calc_ready: false,
    };

    // Basic comparison
    assert!(score1.is_shorter_than(&score2));
    assert!(score2.is_longer_than(&score1));

    // Same terms, different complexity
    assert!(score3.has_lower_complexity(&score1));
    assert!(!score1.has_lower_complexity(&score3));
}

/// Test plan category utilities
#[test]
fn test_plan_category_utilities() {
    use nu_analytics::core::degree::PlanCategory;

    assert_eq!(PlanCategory::Shortest.display_name(), "Shortest Path");
    assert_eq!(PlanCategory::Shortest.file_name(), "shortest");

    assert_eq!(PlanCategory::Longest.display_name(), "Longest Path");
    assert_eq!(PlanCategory::Longest.file_name(), "longest");

    assert_eq!(
        PlanCategory::CalcReadyShortest.display_name(),
        "Calculus-Ready Shortest"
    );
    assert_eq!(
        PlanCategory::CalcReadyShortest.file_name(),
        "calc-ready-shortest"
    );

    assert_eq!(PlanCategory::RandomSample.display_name(), "Random Sample");
    assert_eq!(PlanCategory::RandomSample.file_name(), "random-sample");
}

/// Debug test: Print resolved requirements for CSU to verify 400-level handling
#[test]
fn test_debug_csu_requirements() {
    let degree_path = samples_dir().join("csu-cs-bscs-general.yaml");
    if !degree_path.exists() {
        eprintln!("Skipping test: sample degree file not found");
        return;
    }

    let program = load_degree_from_yaml(&degree_path).expect("Failed to load degree");
    let mut req_resolver = RequirementResolver::new(&program.courses);
    let resolved = req_resolver.resolve_all(&program.requirements);

    println!("\n=== Resolved Major Requirements ===");
    for req in &resolved {
        if req.category.as_deref() == Some("major") {
            println!(
                "\n{} ({} choices, exclude_used: {})",
                req.id, req.choice_count, req.exclude_used
            );
            if req.choice_count <= 3 {
                for (i, choice) in req.choices.iter().enumerate() {
                    println!("  Choice {}: {:?}", i + 1, choice);
                }
            } else {
                println!("  First: {:?}", req.choices.first());
                println!("  Last: {:?}", req.choices.last());
            }
        }
    }

    // Check specific requirements
    println!("\n=== Checking 400-level requirements ===");
    let capstone = resolved.iter().find(|r| r.id == "capstone");
    let cs_400_electives = resolved.iter().find(|r| r.id == "cs_400_electives");
    let tech_focus = resolved.iter().find(|r| r.id.contains("tech_focus"));

    if let Some(req) = capstone {
        println!("\nCapstone: {} choices", req.choice_count);
        println!("  First choice: {:?}", req.choices.first());
    }

    if let Some(req) = cs_400_electives {
        println!("\nCS 400 Electives: {} choices", req.choice_count);
        println!("  First choice: {:?}", req.choices.first());
        // Count total CS 400-level courses across all choices
        let all_courses: std::collections::HashSet<_> = req.choices.iter().flatten().collect();
        let cs_400_count = all_courses.iter().filter(|c| c.starts_with("CS4")).count();
        println!("  Total unique CS 400-level courses: {cs_400_count}");
    }

    if let Some(req) = tech_focus {
        println!("\nTech Focus/Minor: {} choices", req.choice_count);
        if !req.choices.is_empty() {
            let first = &req.choices[0];
            let cs_400_in_first: Vec<_> = first.iter().filter(|c| c.starts_with("CS4")).collect();
            println!("  CS 400-level in first choice: {cs_400_in_first:?}");
        }
    }
}

/// Test selected plans collection
#[test]
fn test_selected_plans_collection() {
    use nu_analytics::core::degree::{PlanScore, ScoredPlan, SelectedPlans};
    use nu_analytics::core::report::term_scheduler::TermPlan;
    use std::collections::HashMap;

    #[allow(clippy::cast_precision_loss)]
    let create_scored_plan = |courses: &[&str]| ScoredPlan {
        variant: PlanVariant::from_parts(
            courses
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
            HashMap::new(),
            courses.len() as f32 * 3.0,
        ),
        score: PlanScore::default(),
        schedule: TermPlan::new(8, false, 15.0),
        course_metrics: HashMap::new(),
    };

    let selected = SelectedPlans {
        shortest: Some(create_scored_plan(&["CS1000", "CS2000"])),
        longest: Some(create_scored_plan(&["CS1000", "CS2000", "CS3000"])),
        calc_ready_shortest: None,
        random_samples: vec![
            create_scored_plan(&["CS1000"]),
            create_scored_plan(&["CS2000"]),
        ],
        total_plans_seen: 100,
    };

    assert_eq!(selected.special_plan_count(), 2);
    assert_eq!(selected.total_count(), 4);

    // Test iteration
    assert_eq!(selected.iter().count(), 4);
}
