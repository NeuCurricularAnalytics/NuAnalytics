
use nu_analytics::mcp::tools::analyze::execute_json;

    #[test]
    fn tc_tulane_cmps2200__reasonable_true() {
        let content = include_str!("/tmp/first_sem_unified/Tulane_University_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CMPS2200"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "Tulane", "CMPS2200", "reasonable_true", earliest, 3, calc_earliest);
    }
    #[test]
    fn tc_tulane_cmps3340__reasonable_true() {
        let content = include_str!("/tmp/first_sem_unified/Tulane_University_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CMPS3340"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "Tulane", "CMPS3340", "reasonable_true", earliest, 2, calc_earliest);
    }
    #[test]
    fn tc_coc_csci218__reasonable_true() {
        let content = include_str!("/tmp/first_sem_unified/College_of_Charleston_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CSCI218"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "CoC", "CSCI218", "reasonable_true", earliest, 2, calc_earliest);
    }
    #[test]
    fn tc_coc_csci495__reasonable_true() {
        let content = include_str!("/tmp/first_sem_unified/College_of_Charleston_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CSCI495"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "CoC", "CSCI495", "reasonable_true", earliest, 4, calc_earliest);
    }
    #[test]
    fn tc_bowdoin_csci2101__reasonable_true() {
        let content = include_str!("/tmp/first_sem_unified/Bowdoin_College_degree_webpage__Major_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CSCI2101"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "Bowdoin", "CSCI2101", "reasonable_true", earliest, 3, calc_earliest);
    }
    #[test]
    fn tc_bowdoin_csci3465__reasonable_true() {
        let content = include_str!("/tmp/first_sem_unified/Bowdoin_College_degree_webpage__Major_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CSCI3465"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "Bowdoin", "CSCI3465", "reasonable_true", earliest, 5, calc_earliest);
    }
    #[test]
    fn tc_nmsu_csci2220__reasonable_true() {
        let content = include_str!("/tmp/first_sem_unified/New_Mexico_State_University_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CSCI2220"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "NMSU", "CSCI2220", "reasonable_true", earliest, 3, calc_earliest);
    }
    #[test]
    fn tc_nmsu_csci4270__reasonable_true() {
        let content = include_str!("/tmp/first_sem_unified/New_Mexico_State_University_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CSCI4270"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "NMSU", "CSCI4270", "reasonable_true", earliest, 6, calc_earliest);
    }
    #[test]
    fn tc_liberty_csis316__reasonable_true() {
        let content = include_str!("/tmp/first_sem_unified/Liberty_University_degree_webpage__Bachelor_of_Science_in_Computer_Science_(General).unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CSIS316"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "Liberty", "CSIS316", "reasonable_true", earliest, 2, calc_earliest);
    }
    #[test]
    fn tc_liberty_cscn354__reasonable_true() {
        let content = include_str!("/tmp/first_sem_unified/Liberty_University_degree_webpage__Bachelor_of_Science_in_Computer_Science_(General).unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CSCN354"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "Liberty", "CSCN354", "reasonable_true", earliest, 2, calc_earliest);
    }
    #[test]
    fn tc_ric_data245__reasonable_true() {
        let content = include_str!("/tmp/first_sem_unified/Rhode_Island_College_degree-BS_webpage__Bachelor_of_Science_in_Artificial_Intelligence.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("DATA245"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "RIC", "DATA245", "reasonable_true", earliest, 2, calc_earliest);
    }
    #[test]
    fn tc_ric_csci446__reasonable_true() {
        let content = include_str!("/tmp/first_sem_unified/Rhode_Island_College_degree-BS_webpage__Bachelor_of_Science_in_Artificial_Intelligence.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CSCI446"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "RIC", "CSCI446", "reasonable_true", earliest, 5, calc_earliest);
    }
    #[test]
    fn tc_calstatela_cs4963__reasonable_true() {
        let content = include_str!("/tmp/first_sem_unified/California_State_University-Los_Angeles_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CS4963"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "CalStateLA", "CS4963", "reasonable_true", earliest, 10, calc_earliest);
    }
    #[test]
    fn tc_tulane_cmps3340__reasonable_false() {
        let content = include_str!("/tmp/first_sem_unified/Tulane_University_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CMPS3340"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "Tulane", "CMPS3340", "reasonable_false", earliest, 2, calc_earliest);
    }
    #[test]
    fn tc_nmsu_csci4270__reasonable_false() {
        let content = include_str!("/tmp/first_sem_unified/New_Mexico_State_University_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CSCI4270"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "NMSU", "CSCI4270", "reasonable_false", earliest, 6, calc_earliest);
    }
    #[test]
    fn tc_calstatela_cs4963__reasonable_false() {
        let content = include_str!("/tmp/first_sem_unified/California_State_University-Los_Angeles_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CS4963"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "CalStateLA", "CS4963", "reasonable_false", earliest, 8, calc_earliest);
    }
    #[test]
    fn tc_metro_cs4050__reasonable_false() {
        let content = include_str!("/tmp/first_sem_unified/Metropolitan_State_University_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CS4050"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "Metro", "CS4050", "reasonable_false", earliest, 6, calc_earliest);
    }
    #[test]
    fn tc_ric_csci446__reasonable_false() {
        let content = include_str!("/tmp/first_sem_unified/Rhode_Island_College_degree-BS_webpage__Bachelor_of_Science_in_Artificial_Intelligence.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CSCI446"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "RIC", "CSCI446", "reasonable_false", earliest, 4, calc_earliest);
    }
    #[test]
    fn tc_wku_stat402__reasonable_false() {
        let content = include_str!("/tmp/first_sem_unified/Western_Kentucky_University_certificate_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("STAT402"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "WKU", "STAT402", "reasonable_false", earliest, 4, calc_earliest);
    }
    #[test]
    fn tc_txstate_cs4398__reasonable_false() {
        let content = include_str!("/tmp/first_sem_unified/Texas_State_University_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("CS4398"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "TxState", "CS4398", "reasonable_false", earliest, 7, calc_earliest);
    }
    #[test]
    fn tc_asu_mat266__calc_ready() {
        let content = include_str!("/tmp/first_sem_unified/Arizona_State_University_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("MAT266"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "ASU", "MAT266", "calc_ready", earliest, 2, calc_earliest);
    }
    #[test]
    fn tc_uaa_matha252f__calc_ready() {
        let content = include_str!("/tmp/first_sem_unified/University_of_Alaska_Anchorage_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("MATHA252F"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "UAA", "MATHA252F", "calc_ready", earliest, 2, calc_earliest);
    }
    #[test]
    fn tc_syracuse_mat397__calc_ready() {
        let content = include_str!("/tmp/first_sem_unified/Syracuse_University_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("MAT397"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "Syracuse", "MAT397", "calc_ready", earliest, 3, calc_earliest);
    }
    #[test]
    fn tc_asu_mat266__not_calc_ready() {
        let content = include_str!("/tmp/first_sem_unified/Arizona_State_University_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("MAT266"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "ASU", "MAT266", "not_calc_ready", earliest, 2, calc_earliest);
    }
    #[test]
    fn tc_uaa_matha252f__not_calc_ready() {
        let content = include_str!("/tmp/first_sem_unified/University_of_Alaska_Anchorage_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("MATHA252F"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "UAA", "MATHA252F", "not_calc_ready", earliest, 3, calc_earliest);
    }
    #[test]
    fn tc_syracuse_mat397__not_calc_ready() {
        let content = include_str!("/tmp/first_sem_unified/Syracuse_University_degree_webpage__Bachelor_of_Science_in_Computer_Science.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("MAT397"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "Syracuse", "MAT397", "not_calc_ready", earliest, 4, calc_earliest);
    }
    #[test]
    fn tc_bellevue_ai240__not_calc_ready() {
        let content = include_str!("/tmp/first_sem_unified/Bellevue_College_degree-BS_webpage__Bachelor_of_Applied_Science_in_Software_Development,_Artificial_Intelligence_Concentration.unified.json");
        let result = nu_analytics::mcp::tools::analyze::execute_json(
            content, Some(200), None, false, None, false, false, None, None, Some("AI240"),
        );
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        let earliest = v["target_course_stats"]["all_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        let calc_earliest = v["target_course_stats"]["calc_ready_plans"]["earliest_term"]
            .as_u64().unwrap_or(0) as usize;
        println!("{} | {} | group={} | got_all={} expected={} | got_calc={}",
            "Bellevue", "AI240", "not_calc_ready", earliest, 4, calc_earliest);
    }
