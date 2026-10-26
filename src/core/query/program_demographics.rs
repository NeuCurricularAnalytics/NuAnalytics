//! Completion demographics for one program — `degree_id` at a school.
//!
//! IPEDS has no programs: completions are counted by school, CIP and award level. This
//! answers for a program by asking the per-CIP query ([`super::completions`]) once per
//! IPEDS year, under the CIP the program was filed under *that* year: the confirmed one
//! in `program_cips` when there is one ([`cips::resolve`]), else the stored program's
//! guess — and every year says which. A program recoded in 2024-2025 therefore reads
//! one CIP through IPEDS 2024 and the other from IPEDS 2025, and the answer says where
//! it changed, so the jump is not mistaken for a change in graduates.
//!
//! Works for a program that is registered but not built: the registry alone names its
//! CIP. What it cannot do is separate programs IPEDS counts together — a BS and a BA
//! filed under one CIP and award level — so those are listed beside the numbers.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::completions::{self, CompletionDemographicsRequest, DemographicsGroupBy};
use crate::core::database::cips::{self, ProgramCip};
use crate::core::database::{tables, DbClient, QueryFilters};
use crate::core::json::{bad_arguments, coded_error, error_code, parse_json_array, to_json_pretty};

/// The stored-program columns this reads.
const PROGRAM_COLS: &str =
    "degree_id,name,cip_code,catalog_year,version_date,degree_type,program_kind";

/// Program kinds that are not IPEDS awards of their own.
use crate::core::database::program_kinds;

/// Whether a registered degree type is a minor, which IPEDS has no line for.
fn is_minor_type(degree_type: Option<&str>) -> bool {
    degree_type.is_some_and(|t| t.eq_ignore_ascii_case(program_kinds::MINOR))
}

#[derive(Debug, Clone, Deserialize)]
struct StoredProgram {
    #[serde(default)]
    degree_id: Option<String>,
    name: String,
    #[serde(default)]
    cip_code: Option<String>,
    #[serde(default)]
    catalog_year: Option<String>,
    #[serde(default)]
    version_date: Option<String>,
    #[serde(default)]
    degree_type: Option<String>,
    #[serde(default)]
    program_kind: Option<String>,
}

impl StoredProgram {
    fn year(&self) -> Option<i32> {
        cips::program_year(self.catalog_year.as_deref(), self.version_date.as_deref())
    }
}

/// Where a year's CIP came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CipSource {
    /// A `program_cips` row covers that year.
    Confirmed,
    /// No row does; the stored program's own CIP, usually a guess.
    Guessed,
}

/// One IPEDS year's CIP for the program.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct YearCip {
    year: i32,
    academic_year: String,
    cip_code: String,
    cip_source: CipSource,
}

/// The CIP for IPEDS survey year `year`: confirmed if a registry row covers its academic
/// year, else the guess of the stored program nearest that year — the latest one not
/// after it, else the earliest after it. Nearest-earlier first because a later catalog
/// may already carry a recode the earlier year did not have.
fn cip_for_year(rows: &[ProgramCip], stored: &[StoredProgram], year: i32) -> Option<YearCip> {
    let academic = cips::ipeds_academic_year(year);
    let (cip_code, cip_source) = if let Some(row) = cips::resolve(rows, Some(academic)) {
        (row.cip_code.clone(), CipSource::Confirmed)
    } else {
        let with_cip = || stored.iter().filter(|p| p.cip_code.is_some());
        let guess = with_cip()
            .filter(|p| p.year().is_some_and(|y| y <= academic))
            .max_by_key(|p| p.year())
            .or_else(|| {
                with_cip()
                    .filter(|p| p.year().is_some_and(|y| y > academic))
                    .min_by_key(|p| p.year())
            })
            .or_else(|| with_cip().next())?;
        (guess.cip_code.clone()?, CipSource::Guessed)
    };
    Some(YearCip {
        year,
        academic_year: cips::format_academic_year(academic),
        cip_code,
        cip_source,
    })
}

/// Where consecutive years change CIP.
fn cip_changes(years: &[YearCip]) -> Vec<serde_json::Value> {
    years
        .windows(2)
        .filter(|w| w[0].cip_code != w[1].cip_code)
        .map(|w| {
            serde_json::json!({
                "from_year": w[1].year,
                "from": w[0].cip_code,
                "to": w[1].cip_code,
            })
        })
        .collect()
}

/// The program and school a degree-mode request names, or why it is refused. Checked
/// before anything is read.
fn refuse_arguments(req: &CompletionDemographicsRequest) -> Result<(&str, i32), String> {
    let degree_id = req
        .degree_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| bad_arguments("degree_id is empty"))?;
    let unitid = req.unitid.ok_or_else(|| {
        bad_arguments(
            "degree_id needs unitid: a program is found at one school (programs outside the \
             US have no IPEDS completions)",
        )
    })?;
    if req.cip_prefix.is_some() || req.cip_codes.is_some() {
        return Err(bad_arguments(
            "degree_id replaces cip_prefix / cip_codes: the program's CIP is looked up per year",
        ));
    }
    if !matches!(req.group_by, None | Some(DemographicsGroupBy::Cip)) {
        return Err(bad_arguments(
            "degree_id answers one program at one school; drop group_by",
        ));
    }
    // One school leaves an institution filter nothing to apply to: refused, not dropped.
    let filters: Vec<&str> = completions::set_filters(req)
        .into_iter()
        .map(completions::DemoFilter::field)
        .collect();
    if !filters.is_empty() {
        return Err(bad_arguments(format!(
            "degree_id is one program at one school, so it cannot apply {}",
            filters.join(", ")
        )));
    }
    Ok((degree_id, unitid))
}

/// What the program's kind means for the answer: `Err` with the whole answer when there
/// is nothing to count (a minor), else the notes to attach.
fn kind_notes(
    degree_id: &str,
    unitid: i32,
    rows: &[ProgramCip],
    stored: &[StoredProgram],
) -> Result<Vec<String>, String> {
    let kind = stored.iter().find_map(|p| p.program_kind.as_deref());
    let registered_minor = rows.iter().any(|r| is_minor_type(r.degree_type.as_deref()));
    if kind == Some(program_kinds::MINOR) || registered_minor {
        return Err(to_json_pretty(&serde_json::json!({
            "degree_id": degree_id,
            "unitid": unitid,
            "years": [],
            "notes": ["a minor is not an IPEDS award: there is no completions line for it"],
        })));
    }
    Ok(kind
        .filter(|k| program_kinds::COUNTED_UNDER_PARENT.contains(k))
        .map(|k| {
            format!(
                "a {k} is not an IPEDS award of its own: these are the numbers of the major it \
                 belongs to, filed under the same CIP"
            )
        })
        .into_iter()
        .collect())
}

/// The award level to count: the one asked for, else the confirmed row's, else what the
/// degree type implies.
fn award_level_for(
    req: &CompletionDemographicsRequest,
    rows: &[ProgramCip],
    stored: &[StoredProgram],
) -> Result<i32, String> {
    let degree_type = rows
        .iter()
        .find_map(|r| r.degree_type.clone())
        .or_else(|| stored.iter().find_map(|p| p.degree_type.clone()));
    req.award_level
        .or_else(|| cips::resolve(rows, None).and_then(|r| r.award_level))
        .or_else(|| degree_type.as_deref().and_then(cips::default_award_level))
        .ok_or_else(|| {
            bad_arguments(degree_type.map_or_else(
                || {
                    "no degree type is stored or registered for this program; pass \
                     award_level, or record it with `db cip set --award-level`"
                        .to_string()
                },
                |t| {
                    format!(
                        "the award level cannot be told from degree type \"{t}\" (a \
                         certificate spans several IPEDS levels); pass award_level, or record \
                         it with `db cip set --award-level`"
                    )
                },
            ))
        })
}

/// The program's registry rows and stored programs; `Err` with the payload when neither
/// exists or the backend fails.
async fn load(
    client: &DbClient,
    unitid: i32,
    degree_id: &str,
) -> Result<(Vec<ProgramCip>, Vec<StoredProgram>), String> {
    let rows = cips::fetch(client, &unitid.to_string(), degree_id)
        .await
        .map_err(|e| e.to_json("reading program_cips"))?;
    let stored: Vec<StoredProgram> = client
        .select(
            tables::PROGRAMS,
            PROGRAM_COLS,
            &QueryFilters::new()
                .eq("unitid", Some(unitid))
                .eq("degree_id", Some(degree_id)),
            None,
        )
        .await
        .map(|v| parse_json_array(&v))
        .map_err(|e| e.to_json("reading stored programs"))?;
    if rows.is_empty() && stored.is_empty() {
        let mut payload = coded_error(
            error_code::SOURCE_NOT_FOUND,
            format!(
                "no stored or registered program has degree_id \"{degree_id}\" at unitid {unitid}"
            ),
        );
        payload["tip"] = "List the school's programs and their degree_ids with a degree \
                          search; register an unbuilt one with `db cip set`"
            .into();
        return Err(payload.to_string());
    }
    Ok((rows, stored))
}

/// One per-CIP answer per year, under that year's CIP.
async fn yearly_answers(
    client: &DbClient,
    req: &CompletionDemographicsRequest,
    unitid: i32,
    award_level: i32,
    years: &[YearCip],
) -> Result<Vec<serde_json::Value>, String> {
    let mut answers = Vec::with_capacity(years.len());
    for y in years {
        let per_cip = CompletionDemographicsRequest {
            group_by: Some(DemographicsGroupBy::Cip),
            unitid: Some(unitid),
            cip_codes: Some(y.cip_code.clone()),
            award_level: Some(award_level),
            year: Some(y.year),
            major_num: req.major_num,
            include_representation: req.include_representation,
            ..CompletionDemographicsRequest::default()
        };
        let answer = completions::execute(client, &per_cip)
            .await
            .map_err(|e| e.to_json_value().to_string())?;
        let mut entry = serde_json::to_value(y).unwrap_or_default();
        entry["demographics"] = serde_json::to_value(answer).unwrap_or_default();
        answers.push(entry);
    }
    Ok(answers)
}

/// Answer `req` for the program `req.degree_id` names at `req.unitid`.
///
/// Called by [`completions::execute_json`] whenever `degree_id` is set.
pub async fn execute_json(client: &DbClient, req: &CompletionDemographicsRequest) -> String {
    match answer(client, req).await {
        Ok(json) | Err(json) => json,
    }
}

async fn answer(client: &DbClient, req: &CompletionDemographicsRequest) -> Result<String, String> {
    let (degree_id, unitid) = refuse_arguments(req)?;
    let (rows, stored) = load(client, unitid, degree_id).await?;
    let mut notes = kind_notes(degree_id, unitid, &rows, &stored)?;
    let award_level = award_level_for(req, &rows, &stored)?;

    let survey_years = match req.year {
        Some(y) => vec![y],
        None => ipeds_years(client, unitid)
            .await
            .map_err(|e| e.to_json("reading the IPEDS years on file"))?,
    };
    let years = years_with_a_cip(&rows, &stored, &survey_years, &mut notes);
    let answers = yearly_answers(client, req, unitid, award_level, &years).await?;

    let used: BTreeSet<&str> = years.iter().map(|y| y.cip_code.as_str()).collect();
    let shared = shared_with(client, unitid, degree_id, &used, award_level)
        .await
        .map_err(|e| e.to_json("reading the school's other programs"))?;
    if !shared.is_empty() {
        notes.push(
            "IPEDS counts every program filed under one CIP and award level together; the \
             programs in shared_cip are inside these numbers too"
                .to_string(),
        );
    }

    Ok(to_json_pretty(&serde_json::json!({
        "degree_id": degree_id,
        "unitid": unitid,
        "award_level": award_level,
        "major_num": req.major_num,
        "major_num_note": if req.major_num.is_none() {
            "first and second majors counted (major_num 1 and 2)"
        } else {
            "only the major_num given"
        },
        "registered": !rows.is_empty(),
        "built": !stored.is_empty(),
        "years": answers,
        "cip_changes": cip_changes(&years),
        "shared_cip": shared,
        "notes": notes,
    })))
}

/// Each survey year that has a CIP — confirmed or guessed — and a note when some do not.
fn years_with_a_cip(
    rows: &[ProgramCip],
    stored: &[StoredProgram],
    survey_years: &[i32],
    notes: &mut Vec<String>,
) -> Vec<YearCip> {
    let years: Vec<YearCip> = survey_years
        .iter()
        .filter_map(|&y| cip_for_year(rows, stored, y))
        .collect();
    if years.len() < survey_years.len() {
        notes.push(
            "years with neither a confirmed CIP nor a stored program are left out".to_string(),
        );
    }
    years
}

/// The IPEDS survey years on file for a school, oldest first.
async fn ipeds_years(
    client: &DbClient,
    unitid: i32,
) -> crate::core::database::DatabaseResult<Vec<i32>> {
    #[derive(Deserialize)]
    struct YearRow {
        year: i32,
    }
    let rows: Vec<YearRow> = parse_json_array(
        &client
            .select(
                tables::INSTITUTION_COMPLETION_TOTALS,
                "year",
                &QueryFilters::new().eq("unitid", Some(unitid)),
                None,
            )
            .await?,
    );
    let years: BTreeSet<i32> = rows.into_iter().map(|r| r.year).collect();
    Ok(years.into_iter().collect())
}

/// The school's other programs filed under one of `cips` at `award_level` — stored or
/// registered — which IPEDS counts in the same numbers.
async fn shared_with(
    client: &DbClient,
    unitid: i32,
    degree_id: &str,
    cips_used: &BTreeSet<&str>,
    award_level: i32,
) -> crate::core::database::DatabaseResult<Vec<serde_json::Value>> {
    // An empty `in_list` adds no filter, which would read every program at the school.
    if cips_used.is_empty() {
        return Ok(Vec::new());
    }
    let list: Vec<&str> = cips_used.iter().copied().collect();
    let stored: Vec<StoredProgram> = parse_json_array(
        &client
            .select(
                tables::PROGRAMS,
                PROGRAM_COLS,
                &QueryFilters::new()
                    .eq("unitid", Some(unitid))
                    .in_list("cip_code", &list),
                None,
            )
            .await?,
    );
    let registered = cips::fetch_for_schools(client, &[unitid.to_string()]).await?;
    Ok(shared_programs(
        &stored,
        &registered,
        degree_id,
        cips_used,
        award_level,
    ))
}

/// Of the school's stored and registered programs, those IPEDS counts with this one: the
/// same CIP and award level, not itself, not a minor, each once.
fn shared_programs(
    stored: &[StoredProgram],
    registered: &[ProgramCip],
    degree_id: &str,
    cips_used: &BTreeSet<&str>,
    award_level: i32,
) -> Vec<serde_json::Value> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for p in stored {
        let Some(id) = p.degree_id.as_deref() else {
            continue;
        };
        let level = p.degree_type.as_deref().and_then(cips::default_award_level);
        let minor = p.program_kind.as_deref() == Some(program_kinds::MINOR);
        if id == degree_id || minor || level.is_some_and(|l| l != award_level) || !seen.insert(id) {
            continue;
        }
        out.push(serde_json::json!({
            "degree_id": id, "name": p.name, "cip_code": p.cip_code, "stored": true,
        }));
    }
    for r in registered {
        if r.degree_id == degree_id
            || is_minor_type(r.degree_type.as_deref())
            || !cips_used.contains(r.cip_code.as_str())
            || r.award_level.is_some_and(|l| l != award_level)
            || !seen.insert(r.degree_id.as_str())
        {
            continue;
        }
        out.push(serde_json::json!({
            "degree_id": r.degree_id, "name": r.name, "cip_code": r.cip_code, "stored": false,
        }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registered(from_year: Option<i32>, cip: &str) -> ProgramCip {
        ProgramCip {
            institution_ref: "1".into(),
            degree_id: "cs".into(),
            from_year,
            unitid: Some(1),
            cip_code: cip.into(),
            award_level: Some(5),
            name: "CS".into(),
            degree_type: Some("BS".into()),
            source: "registrar".into(),
            note: None,
        }
    }

    fn stored(catalog: &str, cip: &str) -> StoredProgram {
        StoredProgram {
            degree_id: Some("cs".into()),
            name: "CS".into(),
            cip_code: Some(cip.into()),
            catalog_year: Some(catalog.into()),
            version_date: None,
            degree_type: Some("BS".into()),
            program_kind: Some("major".into()),
        }
    }

    #[tokio::test]
    async fn test_bad_arguments_are_refused_before_any_request() {
        let client = crate::core::database::test_backend::unreachable();
        let base = || CompletionDemographicsRequest {
            degree_id: Some("cs".into()),
            unitid: Some(1),
            ..Default::default()
        };
        for (req, needle) in [
            (
                CompletionDemographicsRequest {
                    unitid: None,
                    ..base()
                },
                "needs unitid",
            ),
            (
                CompletionDemographicsRequest {
                    cip_prefix: Some("11.".into()),
                    ..base()
                },
                "replaces cip_prefix",
            ),
            (
                CompletionDemographicsRequest {
                    cip_codes: Some("11.0701".into()),
                    ..base()
                },
                "replaces cip_prefix",
            ),
            (
                CompletionDemographicsRequest {
                    group_by: Some(DemographicsGroupBy::School),
                    ..base()
                },
                "group_by",
            ),
            (
                CompletionDemographicsRequest {
                    state: Some("MA".into()),
                    ..base()
                },
                "state",
            ),
            (
                CompletionDemographicsRequest {
                    hbcu: Some(true),
                    limit: Some(5),
                    ..base()
                },
                "hbcu, limit",
            ),
            (
                CompletionDemographicsRequest {
                    degree_id: Some("  ".into()),
                    ..base()
                },
                "degree_id is empty",
            ),
        ] {
            let v: serde_json::Value =
                serde_json::from_str(&execute_json(&client, &req).await).unwrap();
            assert_eq!(v["code"], error_code::BAD_ARGUMENTS, "{v}");
            assert!(
                v["error"].as_str().unwrap().contains(needle),
                "{needle}: {v}"
            );
        }
        // group_by=cip is the grouping degree mode uses, so it is accepted.
        let cip = CompletionDemographicsRequest {
            group_by: Some(DemographicsGroupBy::Cip),
            ..base()
        };
        assert!(refuse_arguments(&cip).is_ok());
    }

    #[tokio::test]
    async fn test_ipeds_years_are_distinct_and_oldest_first() {
        let backend = crate::core::database::test_backend::routed(vec![(
            "/rest/v1/institution_completion_totals",
            r#"[{"year":2024},{"year":2022},{"year":2024}]"#.to_string(),
        )])
        .await;
        assert_eq!(ipeds_years(&backend.client, 1).await.unwrap(), [2022, 2024]);
    }

    #[tokio::test]
    async fn test_an_unknown_program_is_source_not_found() {
        let backend = crate::core::database::test_backend::routed(Vec::new()).await;
        let req = CompletionDemographicsRequest {
            degree_id: Some("cs".into()),
            unitid: Some(1),
            ..Default::default()
        };
        let v: serde_json::Value =
            serde_json::from_str(&execute_json(&backend.client, &req).await).unwrap();
        assert_eq!(v["code"], error_code::SOURCE_NOT_FOUND, "{v}");
        assert!(v["tip"].as_str().unwrap().contains("db cip set"));
    }

    #[tokio::test]
    async fn test_a_registered_unbuilt_program_answers_without_a_build() {
        // No IPEDS years on file, so no per-CIP query is made.
        let backend = crate::core::database::test_backend::routed(vec![(
            "/rest/v1/program_cips",
            serde_json::to_string(&[registered(None, "11.0102")]).unwrap(),
        )])
        .await;
        let req = CompletionDemographicsRequest {
            degree_id: Some("cs".into()),
            unitid: Some(1),
            ..Default::default()
        };
        let v: serde_json::Value =
            serde_json::from_str(&execute_json(&backend.client, &req).await).unwrap();
        assert_eq!(
            (v["registered"].as_bool(), v["built"].as_bool()),
            (Some(true), Some(false)),
            "{v}"
        );
        assert_eq!(v["award_level"], 5);
        assert!(v["years"].as_array().unwrap().is_empty());
        assert!(v["shared_cip"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_a_guess_skips_programs_without_a_cip_and_falls_back_to_an_undated_one() {
        let mut no_cip = stored("2024-2025", "x");
        no_cip.cip_code = None;
        let undated = stored("TBD", "11.0701");
        let y = cip_for_year(&[], &[no_cip.clone(), undated], 2025).unwrap();
        assert_eq!(
            (y.cip_code.as_str(), y.cip_source),
            ("11.0701", CipSource::Guessed)
        );
        assert_eq!(cip_for_year(&[], &[no_cip], 2025), None);
        let mut notes = Vec::new();
        let years = years_with_a_cip(&[], &[], &[2024, 2025], &mut notes);
        assert!(years.is_empty());
        assert!(notes[0].contains("left out"), "{notes:?}");
    }

    #[test]
    fn test_shared_programs_lists_each_program_once() {
        let mut ba = stored("2024-2025", "11.0701");
        (ba.degree_id, ba.degree_type) = (Some("cs-ba".into()), Some("BA".into()));
        let mut ba_later = ba.clone();
        ba_later.catalog_year = Some("2025-2026".into());
        let mut registered_ba = registered(None, "11.0701");
        registered_ba.degree_id = "cs-ba".into();
        let used = BTreeSet::from(["11.0701"]);
        let shared = shared_programs(&[ba, ba_later], &[registered_ba], "cs", &used, 5);
        assert_eq!(shared.len(), 1, "{shared:?}");
        assert_eq!(shared[0]["degree_id"], "cs-ba");
    }

    #[test]
    fn test_a_registered_minor_has_no_ipeds_line() {
        let mut minor = registered(None, "11.0701");
        minor.degree_type = Some("MINOR".into());
        let answer = kind_notes("cs", 1, &[minor], &[]).unwrap_err();
        assert!(answer.contains("not an IPEDS award"), "{answer}");
        let mut track = stored("2024-2025", "11.0701");
        track.program_kind = Some("track".into());
        let notes = kind_notes("cs", 1, &[], &[track]).unwrap();
        assert!(notes[0].contains("numbers of the major"), "{notes:?}");
    }

    #[test]
    fn test_award_level_comes_from_the_request_then_the_row_then_the_type() {
        let req = CompletionDemographicsRequest::default();
        assert_eq!(
            award_level_for(&req, &[registered(None, "11.0701")], &[]),
            Ok(5)
        );
        let mut cert = registered(None, "11.0701");
        (cert.award_level, cert.degree_type) = (None, Some("CERT".into()));
        assert!(award_level_for(&req, std::slice::from_ref(&cert), &[]).is_err());
        let given = CompletionDemographicsRequest {
            award_level: Some(4),
            ..Default::default()
        };
        assert_eq!(award_level_for(&given, &[cert], &[]), Ok(4));
        assert_eq!(
            award_level_for(&req, &[], &[stored("2024-2025", "11.0701")]),
            Ok(5),
            "the stored program's degree type when nothing is registered"
        );
        let mut untyped = registered(None, "11.0701");
        (untyped.award_level, untyped.degree_type) = (None, None);
        let err = award_level_for(&req, &[untyped], &[]).unwrap_err();
        assert!(err.contains("no degree type"), "{err}");
    }

    #[tokio::test]
    async fn test_shared_with_lists_only_programs_inside_the_same_numbers() {
        let programs = serde_json::json!([
            {"degree_id": "cs", "name": "CS BS", "cip_code": "11.0701", "degree_type": "BS"},
            {"degree_id": "cs-ba", "name": "CS BA", "cip_code": "11.0701", "degree_type": "BA"},
            {"degree_id": "cs-minor", "name": "CS minor", "cip_code": "11.0701",
             "degree_type": "MINOR", "program_kind": "minor"},
            {"degree_id": "cs-ms", "name": "CS MS", "cip_code": "11.0701", "degree_type": "MS"},
        ])
        .to_string();
        let mut unbuilt = registered(None, "11.0701");
        unbuilt.degree_id = "cs-new".into();
        let mut elsewhere = registered(None, "11.0102");
        elsewhere.degree_id = "ai".into();
        let backend = crate::core::database::test_backend::routed(vec![
            ("/rest/v1/programs", programs),
            (
                "/rest/v1/program_cips",
                serde_json::to_string(&[unbuilt, elsewhere]).unwrap(),
            ),
        ])
        .await;
        let used = BTreeSet::from(["11.0701"]);
        let shared = shared_with(&backend.client, 1, "cs", &used, 5)
            .await
            .unwrap();
        let ids: Vec<&str> = shared
            .iter()
            .map(|v| v["degree_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["cs-ba", "cs-new"]);
        // With no CIP in use there is nothing to share — and no request that would read
        // every program at the school.
        let none = shared_with(
            &crate::core::database::test_backend::unreachable(),
            1,
            "cs",
            &BTreeSet::new(),
            5,
        )
        .await
        .unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn test_a_recode_splits_the_series_at_the_right_ipeds_year() {
        // Confirmed from 2024-2025 only; earlier years fall back to the stored guess.
        let rows = [registered(Some(2024), "11.0102")];
        let programs = [
            stored("2023-2024", "11.0701"),
            stored("2025-2026", "11.0102"),
        ];
        let years: Vec<YearCip> = [2023, 2024, 2025, 2026]
            .iter()
            .filter_map(|&y| cip_for_year(&rows, &programs, y))
            .collect();
        let got: Vec<(i32, &str, CipSource)> = years
            .iter()
            .map(|y| (y.year, y.cip_code.as_str(), y.cip_source))
            .collect();
        assert_eq!(
            got,
            [
                (2023, "11.0701", CipSource::Guessed),
                (2024, "11.0701", CipSource::Guessed),
                (2025, "11.0102", CipSource::Confirmed),
                (2026, "11.0102", CipSource::Confirmed),
            ]
        );
        assert_eq!(years[2].academic_year, "2024-2025");
        let changes = cip_changes(&years);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["from_year"], 2025);
    }

    #[test]
    fn test_an_unbuilt_program_needs_only_the_registry() {
        let rows = [registered(None, "11.0102")];
        let y = cip_for_year(&rows, &[], 2024).unwrap();
        assert_eq!(
            (y.cip_code.as_str(), y.cip_source),
            ("11.0102", CipSource::Confirmed)
        );
        assert_eq!(cip_for_year(&[], &[], 2024), None);
    }

    #[test]
    fn test_a_guess_comes_from_the_nearest_stored_program() {
        let programs = [
            stored("2022-2023", "11.0701"),
            stored("2025-2026", "11.0103"),
        ];
        let guess = |y| cip_for_year(&[], &programs, y).unwrap().cip_code;
        assert_eq!(guess(2023), "11.0701", "its own year");
        assert_eq!(
            guess(2025),
            "11.0701",
            "nearest earlier, not the newer recode"
        );
        assert_eq!(guess(2026), "11.0103");
        assert_eq!(guess(2020), "11.0701", "before every program: the earliest");
    }
}
