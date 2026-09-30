//! Institution search: `search_institutions` and `db query schools`. An exact `unitid`
//! returns the full record.

use std::sync::Arc;

use super::catalog;
use super::sql::SqlError;
use crate::core::database::{tables, DbClient, QueryFilters};
use crate::core::json::{parse_first, parse_json_array, to_json_pretty};
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
    /// One institution by unitid, with its full record; the other filters are ignored.
    #[schemars(
        description = "One institution by IPEDS unitid, returned with every field; other filters are ignored"
    )]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_i32")]
    pub unitid: Option<i32>,
    /// Only institutions with stored degree programs, each with its programs.
    #[schemars(
        description = "Only institutions holding stored degree programs, each listed with its programs (default false)"
    )]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_bool")]
    pub with_programs: Option<bool>,
}

/// An exact-unitid lookup, which `execute_json` runs when `unitid` is set.
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

/// The full institution record an exact-unitid lookup returns (all columns).
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

/// An institution together with the degree programs stored for it.
#[derive(Debug, Serialize, Deserialize)]
struct SchoolWithPrograms {
    #[serde(flatten)]
    institution: InstitutionSummary,
    program_count: usize,
    programs: Vec<ProgramBrief>,
}

/// The programs-joined school search, as the catalog query returns it.
#[derive(Debug, Serialize, Deserialize)]
pub struct SchoolsWithProgramsResponse {
    /// Schools shown.
    #[serde(default)]
    count: usize,
    /// Schools matching the filters that hold at least one stored program, before
    /// `limit` was applied.
    schools_with_programs: usize,
    schools: Vec<SchoolWithPrograms>,
}

/// What `institutions_with_programs.sql` reads from `$1`. Every key is always sent — a
/// `null` means "not filtered" — so no `skip_serializing_if`.
#[derive(Debug, Default, Serialize)]
struct WithProgramsParams<'a> {
    name: Option<&'a str>,
    state: Option<&'a str>,
    carnegie_class: Option<i32>,
    control: Option<i32>,
    hbcu: Option<bool>,
    tribal: Option<bool>,
    inst_size_min: Option<i32>,
    limit: usize,
}

impl<'a> From<&'a SearchInstitutionsRequest> for WithProgramsParams<'a> {
    /// Destructured with no `..`, as [`institution_filters`] is: a filter added to the
    /// request fails to compile here until the joined search honours it too.
    fn from(req: &'a SearchInstitutionsRequest) -> Self {
        let SearchInstitutionsRequest {
            name,
            state,
            carnegie_class,
            control,
            hbcu,
            tribal,
            inst_size_min,
            limit: _,
            unitid: _,
            with_programs: _,
        } = req;
        Self {
            name: name.as_deref(),
            state: state.as_deref(),
            carnegie_class: *carnegie_class,
            control: *control,
            hbcu: *hbcu,
            tribal: *tribal,
            inst_size_min: *inst_size_min,
            limit: school_limit(req),
        }
    }
}

/// Schools a search returns: the request's own limit, or 25, at most 100.
fn school_limit(req: &SearchInstitutionsRequest) -> usize {
    req.limit.unwrap_or(25).min(100)
}

// ============================================================================
// Execute functions
// ============================================================================

/// The institution filters of the plain school search.
///
/// The joined search applies the same filters in SQL (`institutions_with_programs.sql`).
/// Both destructure the request with no `..` — here and in `WithProgramsParams::from` — so
/// a filter honoured by `db query schools` but not by `--with-programs`, which would
/// quietly widen the second, does not compile.
fn institution_filters(req: &SearchInstitutionsRequest) -> QueryFilters {
    let SearchInstitutionsRequest {
        name,
        state,
        carnegie_class,
        control,
        hbcu,
        tribal,
        inst_size_min,
        limit: _,
        unitid: _,
        with_programs: _,
    } = req;
    QueryFilters::new()
        .eq("carnegie_class", *carnegie_class)
        .eq("control", *control)
        .eq("state", state.as_deref())
        .eq("hbcu", *hbcu)
        .eq("tribal", *tribal)
        .gte("inst_size", *inst_size_min)
        .ilike("name", name.as_deref())
}

/// One institution search, whichever form `req` asks for.
///
/// An exact `unitid` returns that institution's full record; `with_programs` limits the
/// search to institutions holding stored programs and lists them; otherwise a plain
/// filtered search.
pub async fn execute_json(client: &Arc<DbClient>, req: SearchInstitutionsRequest) -> String {
    if let Some(unitid) = req.unitid {
        return execute_get_json(client, GetInstitutionRequest { unitid }).await;
    }
    if req.with_programs.unwrap_or(false) {
        execute_search_with_programs_json(client, req).await
    } else {
        execute_search_json(client, req).await
    }
}

/// Execute the `search_institutions` tool and return JSON.
pub async fn execute_search_json(client: &Arc<DbClient>, req: SearchInstitutionsRequest) -> String {
    let limit = school_limit(&req);

    let filters = institution_filters(&req);

    let result = match client
        .select(tables::INSTITUTIONS, SUMMARY_COLS, &filters, Some(limit))
        .await
    {
        Ok(v) => v,
        Err(e) => return e.to_json("searching institutions"),
    };

    let institutions: Vec<InstitutionSummary> = parse_json_array(&result);

    to_json_pretty(&SearchInstitutionsResponse {
        count: institutions.len(),
        institutions,
    })
}

/// Institution columns of the plain school search; the joined one selects the same in SQL.
const SUMMARY_COLS: &str =
    "unitid,name,city,state,carnegie_class,control,iclevel,hbcu,tribal,inst_size";

/// Search institutions that hold stored degree programs, attaching those programs.
///
/// One catalog query, [`INSTITUTIONS_WITH_PROGRAMS`](super::catalog::INSTITUTIONS_WITH_PROGRAMS):
/// the join, the filters, the grouping and the limit all run in the database.
///
/// # Errors
/// [`SqlError::Backend`] when the database refuses the query, [`SqlError::Database`]
/// when the call does not reach it, and [`SqlError::Shape`] when the answer does not
/// decode — the query and [`SchoolsWithProgramsResponse`] disagree.
pub async fn search_with_programs(
    client: &DbClient,
    req: &SearchInstitutionsRequest,
) -> Result<SchoolsWithProgramsResponse, SqlError> {
    let params = WithProgramsParams::from(req);
    let mut found: SchoolsWithProgramsResponse =
        catalog::run_one(client, &catalog::INSTITUTIONS_WITH_PROGRAMS, &params).await?;
    found.count = found.schools.len();
    Ok(found)
}

/// [`search_with_programs`] as pretty JSON, or an error object.
pub async fn execute_search_with_programs_json(
    client: &Arc<DbClient>,
    req: SearchInstitutionsRequest,
) -> String {
    match search_with_programs(client, &req).await {
        Ok(found) => to_json_pretty(&found),
        Err(e) => e.to_json_value().to_string(),
    }
}

/// Look up one institution by `unitid` and return its full record as JSON.
pub async fn execute_get_json(client: &Arc<DbClient>, req: GetInstitutionRequest) -> String {
    let filters = QueryFilters::new().eq("unitid", Some(req.unitid));

    let result = match client
        .select(tables::INSTITUTIONS, "*", &filters, Some(1))
        .await
    {
        Ok(v) => v,
        Err(e) => return e.to_json(&format!("reading institution {}", req.unitid)),
    };

    let institution: Option<InstitutionDetail> = parse_first(&result);

    institution.map_or_else(
        || {
            serde_json::json!({
                "error": format!("no institution has unitid {}", req.unitid),
                "code": crate::core::json::error_code::SOURCE_NOT_FOUND,
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

    #[test]
    fn with_programs_params_send_exactly_the_keys_the_query_reads() {
        catalog::assert_params_match(
            &catalog::INSTITUTIONS_WITH_PROGRAMS,
            &WithProgramsParams::default(),
        );
    }

    #[test]
    fn the_with_programs_envelope_decodes_institution_and_programs_side_by_side() {
        // `SchoolWithPrograms` flattens the institution beside `programs`, so a field
        // landing on the wrong side would silently drop out of the report.
        let found: SchoolsWithProgramsResponse = serde_json::from_value(serde_json::json!({
            "schools_with_programs": 582,
            "schools": [{
                "unitid": 141_574, "name": "University of Hawaii at Manoa", "city": "Honolulu",
                "state": "HI", "carnegie_class": 15, "control": 1, "iclevel": 1,
                "hbcu": false, "tribal": false, "inst_size": 4,
                "program_count": 1,
                "programs": [{
                    "program_key": "prog:1", "name": "BS CS", "degree_type": "BS",
                    "catalog_year": "2024-2025", "program_kind": "major"
                }]
            }]
        }))
        .expect("envelope decodes");
        assert_eq!(found.schools_with_programs, 582);
        let school = &found.schools[0];
        assert_eq!(school.institution.unitid, 141_574);
        assert_eq!(school.program_count, 1);
        assert_eq!(
            school.programs[0].catalog_year.as_deref(),
            Some("2024-2025")
        );
        let out = serde_json::to_value(&found).expect("serialize");
        assert_eq!(
            out["schools"][0]["state"], "HI",
            "institution fields stay top-level"
        );
    }
}
