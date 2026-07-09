use nu_analytics::mcp::tools::analyze::execute_json;

#[test]
fn asu_bscs_cse475_target_course() {
    let json_content = include_str!("/tmp/asu_unified.json/Arizona_State_University__Bachelor_of_Science_Computer_Science.unified.json");
    let result = execute_json(
        json_content,
        Some(200),
        None,
        false,
        None,
        false,
        false,
        None,
        None,
        Some("DAT402"),
    );
    println!("{result}");
}

#[test]
fn calstatela_cs4963_schedule() {
    let json_content = include_str!("/tmp/first_sem_unified/California_State_University-Los_Angeles_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
    let result = execute_json(
        json_content,
        Some(10),
        None,
        false,
        None,
        false,
        false,
        None,
        None,
        Some("CS4963"),
    );
    let v: serde_json::Value = serde_json::from_str(&result).unwrap();
    let shortest = v["selected_plans"].as_array().unwrap()
        .iter().find(|p| p["category"] == "Shortest Path").unwrap();
    println!("Shortest path: {} terms, {} credits", shortest["terms"], shortest["credits"]);
    for term in shortest["schedule"].as_array().unwrap() {
        let courses: Vec<&str> = term["courses"].as_array().unwrap()
            .iter().map(|c| c.as_str().unwrap()).collect();
        let marker = if courses.contains(&"CS4963") { " ← CS4963" } else { "" };
        println!("  Term {:2} ({:5.1}cr): {:?}{}", term["term"], term["credits"], courses, marker);
    }
    println!("\ntarget_course_stats:\n{}", serde_json::to_string_pretty(&v["target_course_stats"]).unwrap());
}
