//! `search_institutions` and `get_institution` MCP tools

use std::collections::HashMap;
use std::sync::Arc;

use crate::core::database::{tables, DbClient, QueryFilters};
use crate::core::json::{error_json, parse_first, parse_json_array, to_json_pretty};
use serde::{Deserialize, Serialize};

// ============================================================================
// Request types
// ============================================================================

/// Parameters for filtering and searching IPEDS institutions.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchInstitutionsRequest {
    /// Institution name substring to search for (case-insensitive)
    #[schemars(description = "Institution name substring (case-insensitive)")]
    pub name: Option<String>,
    /// Two-letter state abbreviation (e.g. `\"MA\"`, `\"CA\"`)
    #[schemars(description = "Two-letter state abbreviation")]
    pub state: Option<String>,
    /// Carnegie classification code (15=R1 doctoral, 16=R2 doctoral, 17=doctoral/professional). Use `get_lookup_codes` for full list.
    #[schemars(
        description = "Carnegie classification, 2021 Basic (15=R1, 16=R2, 17=Doctoral/Professional). Use get_lookup_codes(\"carnegie_class\") for full list."
    )]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_i32")]
    pub carnegie_class: Option<i32>,
    /// Control type (1=public, 2=private nonprofit, 3=for-profit)
    #[schemars(description = "Control type: 1=public, 2=private nonprofit, 3=for-profit")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_i32")]
    pub control: Option<i32>,
    /// If true, return only HBCUs
    #[schemars(description = "Filter to Historically Black Colleges and Universities")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_bool")]
    pub hbcu: Option<bool>,
    /// If true, return only Tribal colleges
    #[schemars(description = "Filter to Tribal colleges and universities")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_bool")]
    pub tribal: Option<bool>,
    /// Minimum institution size bucket (1=<1000, 2=1000-4999, 3=5000-9999, 4=10000-19999, 5=20000+)
    #[schemars(
        description = "Minimum size bucket: 1=<1000, 2=1000-4999, 3=5000-9999, 4=10000-19999, 5=20000+"
    )]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_i32")]
    pub inst_size_min: Option<i32>,
    /// Maximum results to return (default 25, max 100)
    #[schemars(description = "Maximum results (default 25, max 100)")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_usize")]
    pub limit: Option<usize>,
}

/// Request parameters for `get_institution`
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetInstitutionRequest {
    /// IPEDS Unit ID of the institution (from `search_institutions`)
    #[schemars(description = "IPEDS Unit ID of the institution")]
    pub unitid: i32,
}

// ============================================================================
// Response types
// ============================================================================

/// Summary fields returned by `search_institutions` — no sector, locale, or `updated_year`.
#[derive(Debug, Serialize, Deserialize)]
struct InstitutionSummary {
    unitid: i32,
    name: String,
    city: Option<String>,
    state: Option<String>,
    carnegie_class: Option<i32>,
    control: Option<i32>,
    iclevel: Option<i32>,
    hbcu: Option<bool>,
    tribal: Option<bool>,
    inst_size: Option<i32>,
}

/// Full institution record returned by `get_institution` (all columns).
#[derive(Debug, Serialize, Deserialize)]
struct InstitutionDetail {
    unitid: i32,
    name: String,
    city: Option<String>,
    state: Option<String>,
    sector: Option<i32>,
    control: Option<i32>,
    iclevel: Option<i32>,
    carnegie_class: Option<i32>,
    hbcu: Option<bool>,
    tribal: Option<bool>,
    locale: Option<i32>,
    inst_size: Option<i32>,
    updated_year: Option<i32>,
}

/// Response wrapper for `search_institutions`.
#[derive(Debug, Serialize)]
struct SearchInstitutionsResponse {
    count: usize,
    institutions: Vec<InstitutionSummary>,
}

/// One stored program, as reported alongside its institution.
#[derive(Debug, Serialize, Deserialize)]
struct ProgramBrief {
    program_key: String,
    name: Option<String>,
    degree_type: Option<String>,
    catalog_year: Option<String>,
    program_kind: Option<String>,
}

/// A program row as read from the backend, before grouping.
#[derive(Debug, Deserialize)]
struct ProgramJoinRow {
    unitid: Option<i32>,
    #[serde(flatten)]
    brief: ProgramBrief,
}

/// An institution together with the degree programs stored for it.
#[derive(Debug, Serialize)]
struct SchoolWithPrograms {
    #[serde(flatten)]
    institution: InstitutionSummary,
    program_count: usize,
    programs: Vec<ProgramBrief>,
}

/// Response wrapper for the programs-joined school search.
#[derive(Debug, Serialize)]
struct SchoolsWithProgramsResponse {
    count: usize,
    /// Schools that hold at least one stored program, before `limit` was applied.
    schools_with_programs: usize,
    /// True when the program scan hit its cap, so `schools_with_programs` is a floor.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    truncated: bool,
    schools: Vec<SchoolWithPrograms>,
}

// ============================================================================
// Execute functions
// ============================================================================

/// The institution filters common to both school searches.
///
/// Shared so the joined view cannot drift from the plain one — a filter honoured by
/// `db query schools` but not by `--with-programs` would quietly widen the second.
fn institution_filters(req: &SearchInstitutionsRequest) -> QueryFilters {
    QueryFilters::new()
        .eq("carnegie_class", req.carnegie_class)
        .eq("control", req.control)
        .eq("state", req.state.as_deref())
        .eq("hbcu", req.hbcu)
        .eq("tribal", req.tribal)
        .gte("inst_size", req.inst_size_min)
        .ilike("name", req.name.as_deref())
}

/// Execute the `search_institutions` tool and return JSON.
pub async fn execute_search_json(client: &Arc<DbClient>, req: SearchInstitutionsRequest) -> String {
    let limit = req.limit.unwrap_or(25).min(100);

    let filters = institution_filters(&req);

    let result = match client
        .select(tables::INSTITUTIONS, SUMMARY_COLS, &filters, Some(limit))
        .await
    {
        Ok(v) => v,
        Err(e) => return error_json(e),
    };

    let institutions: Vec<InstitutionSummary> = parse_json_array(&result);

    to_json_pretty(&SearchInstitutionsResponse {
        count: institutions.len(),
        institutions,
    })
}

/// Institution columns shared by both school searches.
const SUMMARY_COLS: &str =
    "unitid,name,city,state,carnegie_class,control,iclevel,hbcu,tribal,inst_size";

/// Program columns needed to say what a school offers.
const PROGRAM_JOIN_COLS: &str = "unitid,program_key,name,degree_type,catalog_year,program_kind";

/// Unitids per institution `IN(...)` batch.
///
/// Every school with a program goes into these lists, so a single `IN(...)` would grow
/// with the corpus — 582 unitids is already a 4 KB URL. Batching bounds it.
const SCHOOL_JOIN_BATCH_SIZE: usize = 100;

/// Program rows read when building the school to programs map.
///
/// Below `PGRST_DB_MAX_ROWS` (5,000, checked by `db doctor`) so the cap that bites is this
/// one, which is reported, rather than the backend's, which is silent.
const PROGRAM_SCAN_LIMIT: usize = 4_000;

/// Group program rows by institution, dropping any that name no institution.
///
/// A `programs` row may have a null `unitid` — the column carries no FK and import can
/// fail to resolve one. Such a row belongs to no school, so it is skipped rather than
/// collected under a placeholder. Each school's list is sorted by `program_key` so the
/// output does not depend on the order the backend returned rows in.
fn group_programs_by_unitid(rows: Vec<ProgramJoinRow>) -> HashMap<i32, Vec<ProgramBrief>> {
    let mut by_unitid: HashMap<i32, Vec<ProgramBrief>> = HashMap::new();
    for row in rows {
        if let Some(uid) = row.unitid {
            by_unitid.entry(uid).or_default().push(row.brief);
        }
    }
    for programs in by_unitid.values_mut() {
        programs.sort_by(|a, b| a.program_key.cmp(&b.program_key));
    }
    by_unitid
}

/// Search institutions that hold stored degree programs, attaching those programs.
///
/// Driven from `programs` rather than `institutions` because only 582 of 6,515
/// institutions have any: filtering after an institution search would make `--limit 25`
/// return a handful of schools, and scanning all 6,515 to intersect would cross
/// `PGRST_DB_MAX_ROWS` and truncate silently.
pub async fn execute_search_with_programs_json(
    client: &Arc<DbClient>,
    req: SearchInstitutionsRequest,
) -> String {
    let limit = req.limit.unwrap_or(25).min(100);

    let scanned = match client
        .select(
            tables::PROGRAMS,
            PROGRAM_JOIN_COLS,
            &QueryFilters::new(),
            Some(PROGRAM_SCAN_LIMIT),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => return error_json(e),
    };
    let rows: Vec<ProgramJoinRow> = parse_json_array(&scanned);
    let truncated = rows.len() >= PROGRAM_SCAN_LIMIT;

    let mut by_unitid = group_programs_by_unitid(rows);
    let schools_with_programs = by_unitid.len();

    // Sorted so the batches — and therefore which schools `limit` keeps — are the same
    // on every run rather than following HashMap iteration order.
    let mut unitids: Vec<i32> = by_unitid.keys().copied().collect();
    unitids.sort_unstable();

    let mut schools: Vec<SchoolWithPrograms> = Vec::new();
    for chunk in unitids.chunks(SCHOOL_JOIN_BATCH_SIZE) {
        let remaining = limit - schools.len();
        if remaining == 0 {
            break;
        }
        let filters = institution_filters(&req).in_list("unitid", chunk);
        let found = match client
            .select(
                tables::INSTITUTIONS,
                SUMMARY_COLS,
                &filters,
                Some(remaining),
            )
            .await
        {
            Ok(v) => v,
            Err(e) => return error_json(e),
        };
        for institution in parse_json_array::<InstitutionSummary>(&found) {
            let programs = by_unitid.remove(&institution.unitid).unwrap_or_default();
            schools.push(SchoolWithPrograms {
                institution,
                program_count: programs.len(),
                programs,
            });
        }
    }
    schools.sort_by_key(|s| s.institution.unitid);

    to_json_pretty(&SchoolsWithProgramsResponse {
        count: schools.len(),
        schools_with_programs,
        truncated,
        schools,
    })
}

/// Execute the `get_institution` tool and return JSON.
pub async fn execute_get_json(client: &Arc<DbClient>, req: GetInstitutionRequest) -> String {
    let filters = QueryFilters::new().eq("unitid", Some(req.unitid));

    let result = match client
        .select(tables::INSTITUTIONS, "*", &filters, Some(1))
        .await
    {
        Ok(v) => v,
        Err(e) => return error_json(e),
    };

    let institution: Option<InstitutionDetail> = parse_first(&result);

    institution.map_or_else(
        || {
            serde_json::json!({
                "error": "Institution not found",
                "unitid": req.unitid
            })
            .to_string()
        },
        |inst| to_json_pretty(&inst),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prog(unitid: Option<i32>, key: &str) -> ProgramJoinRow {
        ProgramJoinRow {
            unitid,
            brief: ProgramBrief {
                program_key: key.to_string(),
                name: None,
                degree_type: None,
                catalog_year: None,
                program_kind: None,
            },
        }
    }

    #[test]
    fn grouping_collects_every_program_under_its_school() {
        let by_uid = group_programs_by_unitid(vec![
            prog(Some(1), "b"),
            prog(Some(2), "c"),
            prog(Some(1), "a"),
        ]);
        assert_eq!(by_uid.len(), 2, "one entry per school");
        assert_eq!(by_uid[&1].len(), 2);
        assert_eq!(by_uid[&2].len(), 1);
    }

    #[test]
    fn grouping_sorts_each_school_so_output_does_not_track_backend_row_order() {
        let by_uid = group_programs_by_unitid(vec![
            prog(Some(1), "zeta"),
            prog(Some(1), "alpha"),
            prog(Some(1), "mid"),
        ]);
        let keys: Vec<&str> = by_uid[&1].iter().map(|p| p.program_key.as_str()).collect();
        assert_eq!(keys, ["alpha", "mid", "zeta"]);
    }

    #[test]
    fn grouping_drops_programs_with_no_institution_rather_than_inventing_one() {
        // `programs.unitid` has no FK and import can fail to resolve it. Such a row
        // belongs to no school, and bucketing it under a placeholder would inflate
        // `schools_with_programs`.
        let by_uid = group_programs_by_unitid(vec![prog(None, "orphan"), prog(Some(7), "real")]);
        assert_eq!(by_uid.len(), 1);
        assert!(by_uid.contains_key(&7));
    }

    #[test]
    fn grouping_nothing_yields_no_schools() {
        assert!(group_programs_by_unitid(Vec::new()).is_empty());
    }

    #[test]
    fn a_program_row_deserialises_with_its_institution_split_out() {
        // `ProgramJoinRow` flattens `ProgramBrief`, so a field landing on the wrong side
        // of that split would silently drop out of the report.
        let row: ProgramJoinRow = serde_json::from_value(serde_json::json!({
            "unitid": 141_574,
            "program_key": "prog:1",
            "name": "BS CS",
            "degree_type": "BS",
            "catalog_year": "2024-2025",
            "program_kind": "major",
        }))
        .expect("deserialize");
        assert_eq!(row.unitid, Some(141_574));
        assert_eq!(row.brief.program_key, "prog:1");
        assert_eq!(row.brief.degree_type.as_deref(), Some("BS"));
        assert_eq!(row.brief.catalog_year.as_deref(), Some("2024-2025"));
        assert_eq!(row.brief.program_kind.as_deref(), Some("major"));
    }
}
