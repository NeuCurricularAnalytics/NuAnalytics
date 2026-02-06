//! Integration tests for `CourseGraph`

use nu_analytics::core::degree::load_degree_from_yaml;
use nu_analytics::core::models::{CourseGraph, PrerequisiteType};

#[test]
fn test_build_course_graph_from_neu_degree() {
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load NEU degree");

    let result = CourseGraph::from_degree_program(&program);

    // Should have all courses from the degree
    assert!(!result.graph.is_empty());
    assert!(result.graph.len() >= program.courses.len());

    // Should not have cycles (we trust the sample is valid)
    assert!(
        result.cycles.is_empty(),
        "Unexpected cycles: {:?}",
        result.cycles
    );
    assert!(!result.graph.has_cycles());

    // Should have topological order
    assert!(result.graph.topological_order().is_some());
}

#[test]
fn test_course_graph_prerequisite_chains() {
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load NEU degree");

    let result = CourseGraph::from_degree_program(&program);
    let graph = &result.graph;

    // CS3650 (Computer Systems) requires CS3100
    let cs3650 = graph.get("CS3650").expect("CS3650 should exist");
    assert!(
        cs3650
            .prerequisites
            .iter()
            .any(|e| e.prerequisite == "CS3100"),
        "CS3650 should require CS3100"
    );

    // Get the full prerequisite chain for CS3650
    let chain = graph
        .prerequisite_chain("CS3650", true)
        .expect("Should get prerequisite chain");

    // Should include CS3100 and its prerequisites
    assert!(chain.contains("CS3100"), "Chain should include CS3100");
    assert!(
        chain.contains("CS2100"),
        "Chain should include CS2100 (prereq of CS3100)"
    );
}

#[test]
fn test_course_graph_or_prerequisites() {
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load NEU degree");

    let result = CourseGraph::from_degree_program(&program);
    let graph = &result.graph;

    // CS3000 has "(CS2100 | DS2500) & CS1800"
    let cs3000 = graph.get("CS3000").expect("CS3000 should exist");

    // Should have CS1800 as required
    let required = cs3000.required_prerequisites();
    assert!(required.contains(&"CS1800"), "CS3000 should require CS1800");

    // Should have CS2100 and DS2500 as optional (OR group)
    let optional_groups = cs3000.optional_prerequisite_groups();
    assert!(
        !optional_groups.is_empty(),
        "CS3000 should have optional prerequisites"
    );

    // Find the group with CS2100/DS2500
    let has_or_group = cs3000.prerequisites.iter().any(|e| {
        e.prereq_type == PrerequisiteType::Optional
            && (e.prerequisite == "CS2100" || e.prerequisite == "DS2500")
    });
    assert!(
        has_or_group,
        "CS3000 should have CS2100|DS2500 as optional prerequisites"
    );
}

#[test]
fn test_course_graph_dependents() {
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load NEU degree");

    let result = CourseGraph::from_degree_program(&program);
    let graph = &result.graph;

    // CS3100 should have dependents (courses that require it)
    let cs3100 = graph.get("CS3100").expect("CS3100 should exist");
    assert!(
        !cs3100.dependents.is_empty(),
        "CS3100 should have dependents"
    );

    // CS3650 should be one of them
    assert!(
        cs3100.dependents.contains(&"CS3650".to_string()),
        "CS3650 should depend on CS3100"
    );

    // Get full dependent chain
    let dep_chain = graph
        .dependent_chain("CS3100")
        .expect("Should get dependent chain");
    assert!(
        !dep_chain.is_empty(),
        "CS3100 should have dependent courses"
    );
}

#[test]
fn test_course_graph_leaf_and_terminal_courses() {
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load NEU degree");

    let result = CourseGraph::from_degree_program(&program);
    let graph = &result.graph;

    // Should have leaf courses (no prerequisites)
    let leaves = graph.leaf_courses();
    assert!(!leaves.is_empty(), "Should have leaf courses");

    // CS1200 (First Year Seminar) has no prerequisites
    assert!(leaves.contains(&"CS1200"), "CS1200 should be a leaf course");

    // Should have terminal courses (no dependents)
    let terminals = graph.terminal_courses();
    assert!(!terminals.is_empty(), "Should have terminal courses");
}

#[test]
fn test_course_graph_depth() {
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load NEU degree");

    let result = CourseGraph::from_degree_program(&program);
    let graph = &result.graph;

    // Leaf courses should have depth 0
    let cs1200_depth = graph.course_depth("CS1200");
    assert_eq!(cs1200_depth, Some(0), "CS1200 should have depth 0");

    // Courses with prerequisites should have depth > 0
    let cs3650_depth = graph.course_depth("CS3650");
    assert!(
        cs3650_depth.is_some() && cs3650_depth.unwrap() > 0,
        "CS3650 should have depth > 0"
    );
}

#[test]
fn test_course_graph_corequisites() {
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load NEU degree");

    let result = CourseGraph::from_degree_program(&program);
    let graph = &result.graph;

    // CS1800 has corequisite CS1802
    let cs1800 = graph.get("CS1800").expect("CS1800 should exist");
    let coreqs = cs1800.corequisites();
    assert!(
        coreqs.contains(&"CS1802"),
        "CS1800 should have CS1802 as corequisite"
    );
}

#[test]
fn test_course_graph_display() {
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load NEU degree");

    let result = CourseGraph::from_degree_program(&program);

    // Test display formatting
    let display = format!("{}", result.graph);
    assert!(
        display.contains("Course Graph"),
        "Display should have header"
    );
    assert!(display.contains("CS3650"), "Display should include CS3650");
}

#[test]
fn test_build_course_graph_from_csu_degree() {
    let program = load_degree_from_yaml("samples/degrees/csu-cs-bscs-general.yaml")
        .expect("Failed to load CSU degree");

    let result = CourseGraph::from_degree_program(&program);

    // Should have all courses from the degree
    assert!(!result.graph.is_empty());

    // CSU has a known "cycle" between CS152 and CS163 where each can be a prereq for the other
    // This is actually valid (alternative entry points) but detected as a cycle from graph perspective
    // We accept this specific pattern
    if !result.cycles.is_empty() {
        // Verify it's only the expected "cycle"
        for cycle in &result.cycles {
            let contains_cs152_cs163 =
                cycle.contains(&"CS152".to_string()) && cycle.contains(&"CS163".to_string());
            assert!(
                contains_cs152_cs163,
                "Unexpected cycle (not CS152/CS163): {cycle:?}"
            );
        }
    }
}

#[test]
fn test_build_course_graph_from_uhm_degree() {
    let program = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml")
        .expect("Failed to load UHM degree");

    let result = CourseGraph::from_degree_program(&program);

    // Should have all courses from the degree
    assert!(!result.graph.is_empty());

    // Should not have cycles
    assert!(
        result.cycles.is_empty(),
        "Unexpected cycles: {:?}",
        result.cycles
    );
}

#[test]
fn test_course_graph_with_missing_prerequisites() {
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load NEU degree");

    let result = CourseGraph::from_degree_program(&program);

    // The graph should still build even if some prerequisites are missing
    assert!(!result.graph.is_empty());

    // Check that missing courses are tracked
    if !result.missing_courses.is_empty() {
        // Missing courses should still be in the graph as placeholder nodes
        for missing in &result.missing_courses {
            assert!(
                result.graph.get(missing).is_some(),
                "Missing course {missing} should be in graph"
            );
        }
    }
}

#[test]
fn test_course_graph_cycle_detection() {
    let program = load_degree_from_yaml("samples/degrees/csu-cs-bscs-general.yaml")
        .expect("Failed to load CSU degree");

    let result = CourseGraph::from_degree_program(&program);

    // CSU has CS152 <-> CS163 cycle
    if !result.cycles.is_empty() {
        // Each cycle should be a valid Vec of courses
        for cycle in &result.cycles {
            assert!(!cycle.is_empty(), "Cycle should not be empty");

            // All courses in cycle should exist in graph
            for course in cycle {
                assert!(
                    result.graph.get(course).is_some(),
                    "Course in cycle should exist in graph"
                );
            }
        }
    }
}

#[test]
fn test_course_graph_entry_points() {
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load NEU degree");

    let result = CourseGraph::from_degree_program(&program);
    let graph = &result.graph;

    // Entry points are courses with no prerequisites
    let entry_points: Vec<&str> = graph
        .course_keys()
        .into_iter()
        .filter(|key| {
            graph
                .get(key)
                .is_some_and(|node| node.prerequisites.is_empty())
        })
        .collect();

    // Should have multiple entry points
    assert!(
        !entry_points.is_empty(),
        "Degree should have entry point courses"
    );

    // Entry points should have no prerequisites
    for ep in entry_points {
        let node = graph.get(ep).unwrap();
        assert!(
            node.prerequisites.is_empty(),
            "{ep} should have no prerequisites"
        );
    }
}

#[test]
fn test_course_graph_terminal_courses() {
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load NEU degree");

    let result = CourseGraph::from_degree_program(&program);
    let graph = &result.graph;

    // Terminal courses are those with no dependents
    // Just check if any exist
    let has_terminal = graph.course_keys().into_iter().any(|key| {
        graph
            .get(key)
            .is_some_and(|node| node.dependents.is_empty())
    });

    // Should have terminal courses
    assert!(
        has_terminal,
        "Degree should have terminal courses (no dependents)"
    );
}

#[test]
fn test_course_graph_prerequisite_chain_depth() {
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load NEU degree");

    let result = CourseGraph::from_degree_program(&program);
    let graph = &result.graph;

    // CS3650 (Computer Systems) has a deep prerequisite chain
    let chain = graph
        .prerequisite_chain("CS3650", true)
        .expect("Should get chain for CS3650");

    // Chain should include:
    // CS3650 -> CS3100 -> CS2100 -> CS2000
    assert!(
        chain.len() >= 3,
        "CS3650 should have deep prerequisite chain"
    );

    // All courses in chain should exist
    for course in &chain {
        assert!(
            graph.get(course).is_some(),
            "Course {course} in chain should exist"
        );
    }
}

#[test]
fn test_course_graph_with_corequisites() {
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load NEU degree");

    let result = CourseGraph::from_degree_program(&program);
    let graph = &result.graph;

    // CS1800 has corequisite CS1802
    let cs1800 = graph.get("CS1800").expect("CS1800 should exist");

    // Should have corequisite edge
    let has_coreq = cs1800.prerequisites.iter().any(|e| {
        e.prerequisite == "CS1802"
            && (e.prereq_type == PrerequisiteType::Corequisite
                || e.prereq_type == PrerequisiteType::StrictCorequisite)
    });

    assert!(has_coreq, "CS1800 should have CS1802 as corequisite");

    // Corequisite should be bidirectional
    let cs1802 = graph.get("CS1802").expect("CS1802 should exist");
    let has_reverse_coreq = cs1802.prerequisites.iter().any(|e| {
        e.prerequisite == "CS1800"
            && (e.prereq_type == PrerequisiteType::Corequisite
                || e.prereq_type == PrerequisiteType::StrictCorequisite)
    });

    assert!(
        has_reverse_coreq,
        "CS1802 should have CS1800 as corequisite (bidirectional)"
    );
}
