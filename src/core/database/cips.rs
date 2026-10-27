//! Confirmed CIP codes — the `program_cips` registry.
//!
//! Most stored CIPs are guesses. As a school confirms its programs' CIPs, they are
//! recorded here against the *program* — `(institution_ref, degree_id)`, which spans
//! catalog years and dated versions — rather than against one `program_key`. Three
//! things read the registry: `db import`, which overrides a guess with the confirmed CIP
//! ([`super::import`]); `db cip set`, which corrects the programs already stored
//! ([`reapply`]); and the degree-level demographics, which need the CIP a program was
//! filed under in each IPEDS year. A row may name a program that is not built yet.
//!
//! **History.** Schools recode programs, so a program may have several rows. Each applies
//! from its `from_year` — the start of an academic year, 2024 meaning 2024-2025 — until
//! the next row's; a row without one applies to all years, or in a history to every year
//! before the first dated row ([`resolve`]). The two things resolved against it count
//! years differently: a catalog year by its first year ([`program_year`]), an IPEDS
//! survey year as the academic year before it ([`ipeds_academic_year`]).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::error::{DatabaseError, DatabaseResult};
use super::filters::QueryFilters;
use super::import::document_and_hash;
use super::{program_kinds, tables, DbClient};
use crate::core::degree::from_unified_value;
use crate::core::json::parse_json_array;
use crate::core::models::DegreeProgram;

/// The columns that identify one registry row: the upsert target.
const IDENTITY: &[&str] = &["institution_ref", "degree_id", "from_year"];

/// How a confirmation is recorded in a document's `conversion_warnings`. A later
/// confirmation replaces the note rather than adding another, so it starts with this.
pub const CONFIRMED_NOTE_PREFIX: &str = "cip_code confirmed:";

/// One confirmed CIP for one program, from one academic year (or for all years).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramCip {
    /// The school, as `programs.institution_ref`: the unitid as text, or `cc:slug`.
    pub institution_ref: String,
    /// The program, as `programs.degree_id`.
    pub degree_id: String,
    /// Start of the academic year this CIP applies from (2024 = 2024-2025); `None` for
    /// all years.
    #[serde(default)]
    pub from_year: Option<i32>,
    /// The IPEDS unit id, for a US school — what completions are joined on.
    #[serde(default)]
    pub unitid: Option<i32>,
    /// The CIP code, dot notation.
    pub cip_code: String,
    /// IPEDS award level (5 = bachelor's, …).
    #[serde(default)]
    pub award_level: Option<i32>,
    /// The program as the school names it.
    pub name: String,
    /// The degree type, e.g. `BS`.
    #[serde(default)]
    pub degree_type: Option<String>,
    /// Where the CIP came from.
    pub source: String,
    /// Anything else worth keeping with it.
    #[serde(default)]
    pub note: Option<String>,
}

// ============================================================================
// Years
// ============================================================================

/// The row in force in academic year `year`, among one program's rows.
///
/// The row with the latest `from_year` not after `year`, an undated row counting as the
/// earliest. `None` when every row starts later — a school saying "11.0102 from
/// 2024-2025" says nothing about 2022-2023, so that year has no confirmed CIP rather than
/// the earliest one. With no year at all (a program whose catalog year cannot be read),
/// the latest row: the CIP the program has now.
#[must_use]
pub fn resolve(rows: &[ProgramCip], year: Option<i32>) -> Option<&ProgramCip> {
    rows.iter()
        .filter(|r| match (year, r.from_year) {
            (Some(y), Some(from)) => from <= y,
            _ => true,
        })
        .max_by_key(|r| r.from_year)
}

/// The academic year a stored program belongs to: its catalog year's first year, else
/// its version date's academic year, else `None`.
///
/// `2026-2028` and `Fall 2026` resolve by the first four-digit year. `Spring/Summer 2027`
/// belongs to 2026-2027 but reads as 2027 — rare, and not worth a special case.
#[must_use]
pub fn program_year(catalog_year: Option<&str>, version_date: Option<&str>) -> Option<i32> {
    catalog_year
        .and_then(first_year_in)
        .or_else(|| version_date.and_then(academic_year_of_date))
}

/// The academic year an IPEDS survey year counts.
///
/// IPEDS `C{Y}_A` reports awards conferred from July 1 of Y-1 to June 30 of Y — the
/// 2023-2024 academic year for `C2024_A`. Getting this backwards would shift every
/// recode by a year, so it is pinned by a test.
#[must_use]
pub const fn ipeds_academic_year(survey_year: i32) -> i32 {
    survey_year - 1
}

/// Parse `--from 2024-2025` to its first year.
///
/// A bare `2024` is refused: it could mean the 2024-2025 catalog year or IPEDS 2024,
/// which is 2023-2024.
///
/// # Errors
/// A message naming the expected form.
pub fn parse_academic_year(s: &str) -> Result<i32, String> {
    let s = s.trim();
    let bad = || {
        format!(
            "an academic year is written YYYY-YYYY, e.g. 2024-2025 (a bare year is ambiguous \
             between a catalog year and an IPEDS survey year), got \"{s}\""
        )
    };
    let (first, second) = s.split_once('-').ok_or_else(bad)?;
    let first: i32 = four_digits(first).ok_or_else(bad)?;
    let second: i32 = four_digits(second).ok_or_else(bad)?;
    if second == first + 1 {
        Ok(first)
    } else {
        Err(bad())
    }
}

/// `2024` → `"2024-2025"`.
#[must_use]
pub fn format_academic_year(first: i32) -> String {
    format!("{first}-{}", first + 1)
}

fn four_digits(s: &str) -> Option<i32> {
    (s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

fn first_year_in(s: &str) -> Option<i32> {
    let bytes = s.as_bytes();
    (0..bytes.len().saturating_sub(3)).find_map(|i| {
        let run = &bytes[i..i + 4];
        let bounded = i == 0 || !bytes[i - 1].is_ascii_digit();
        let ends = bytes.get(i + 4).is_none_or(|b| !b.is_ascii_digit());
        (bounded && ends && run.iter().all(u8::is_ascii_digit))
            .then(|| std::str::from_utf8(run).ok()?.parse().ok())
            .flatten()
    })
}

/// `YYYY-MM-DD` → its academic year: July onward starts the next one.
fn academic_year_of_date(date: &str) -> Option<i32> {
    let d = chrono::NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").ok()?;
    let year = chrono::Datelike::year(&d);
    Some(if chrono::Datelike::month(&d) >= 7 {
        year
    } else {
        year - 1
    })
}

/// The IPEDS award level a degree type implies, when it implies one.
///
/// Only the unambiguous ones. A certificate spans IPEDS levels 1, 2, 4, 6 and 8, and a
/// minor is not an IPEDS award at all, so those need an explicit level (or have none).
#[must_use]
pub fn default_award_level(degree_type: &str) -> Option<i32> {
    match degree_type.trim().to_ascii_uppercase().as_str() {
        "AA" | "AS" | "AAS" => Some(3),
        "BS" | "BA" | "BSE" | "BAS" | "BFA" | "BBA" | "AB" | "BSC" | "BASC" => Some(5),
        "MS" | "MA" | "MENG" | "MBA" | "MSC" | "MPS" => Some(7),
        _ => None,
    }
}

// ============================================================================
// The document
// ============================================================================

/// How a row's span reads: `all years`, or `from 2024-2025`.
#[must_use]
pub fn span_label(from_year: Option<i32>) -> String {
    from_year.map_or_else(
        || "all years".to_string(),
        |y| format!("from {}", format_academic_year(y)),
    )
}

/// The note a confirmed CIP leaves in a document's `conversion_warnings`.
#[must_use]
pub fn confirmed_note(row: &ProgramCip) -> String {
    format!(
        "{CONFIRMED_NOTE_PREFIX} {} ({}) — {}",
        row.cip_code,
        span_label(row.from_year),
        row.source
    )
}

/// Remove a confirmation note; `true` when there was one.
fn strip_confirmed(program: &mut DegreeProgram) -> bool {
    let before = program.conversion_warnings.len();
    program
        .conversion_warnings
        .retain(|w| !w.starts_with(CONFIRMED_NOTE_PREFIX));
    program.conversion_warnings.len() != before
}

/// Write a confirmed CIP into a program: its `cip_code`, and a note saying where it came
/// from.
///
/// The note replaces an earlier confirmation's, so re-confirming does not pile notes up
/// and a correction re-imported unchanged hashes the same. The original warnings — often
/// "`cip_code` … inferred from program name" — are kept: many also say other things
/// ("`degree_type` confirmed BS; `cip_code` …"), so deleting them would lose those; the note
/// after them is what supersedes the guess.
pub fn apply_confirmed(program: &mut DegreeProgram, row: &ProgramCip) {
    program.degree.cip_code = Some(row.cip_code.clone());
    strip_confirmed(program);
    program.conversion_warnings.push(confirmed_note(row));
}

// ============================================================================
// The database
// ============================================================================

/// The filter for one program's rows, in `program_cips` or `programs` (both carry
/// `institution_ref` and `degree_id`).
fn program_filter(institution_ref: &str, degree_id: &str) -> QueryFilters {
    QueryFilters::new()
        .eq("institution_ref", Some(institution_ref))
        .eq("degree_id", Some(degree_id))
}

/// One program's registry rows.
///
/// # Errors
/// The backend's error.
pub async fn fetch(
    client: &DbClient,
    institution_ref: &str,
    degree_id: &str,
) -> DatabaseResult<Vec<ProgramCip>> {
    let rows = client
        .select(
            tables::PROGRAM_CIPS,
            "*",
            &program_filter(institution_ref, degree_id),
            None,
        )
        .await?;
    Ok(parse_json_array(&rows))
}

/// Every registry row for the given schools (`institution_ref`s), or for every school
/// when `refs` is empty.
///
/// # Errors
/// The backend's error.
pub async fn fetch_for_schools(
    client: &DbClient,
    refs: &[String],
) -> DatabaseResult<Vec<ProgramCip>> {
    let filters = QueryFilters::new().in_list("institution_ref", refs);
    let rows = client
        .select(tables::PROGRAM_CIPS, "*", &filters, None)
        .await?;
    Ok(parse_json_array(&rows))
}

/// Add a row, or replace the one for the same program and `from_year`.
///
/// # Errors
/// The backend's error.
pub async fn upsert(client: &DbClient, row: &ProgramCip) -> DatabaseResult<()> {
    let mut value = serde_json::to_value(row)
        .map_err(|e| DatabaseError::ParseError(format!("could not serialize the CIP row: {e}")))?;
    value["updated_at"] = serde_json::Value::String(chrono::Utc::now().to_rfc3339());
    client
        .upsert_batch(tables::PROGRAM_CIPS, vec![value], IDENTITY)
        .await
}

/// Which of a program's rows [`remove`] takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowSelection {
    /// Every row.
    All,
    /// The all-years row (no `from_year`).
    AllYearsRow,
    /// The row from this academic year.
    FromYear(i32),
}

impl RowSelection {
    /// From `db cip remove`'s flags: `--all-years` for the all-years row, `--from` for
    /// one dated row, neither for every row.
    #[must_use]
    pub const fn from_flags(all_years: bool, from: Option<i32>) -> Self {
        match (all_years, from) {
            (true, _) => Self::AllYearsRow,
            (false, Some(year)) => Self::FromYear(year),
            (false, None) => Self::All,
        }
    }

    /// How it reads in a message: `all rows`, `the all-years row`, `the row from …`.
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::All => "all rows".to_string(),
            Self::AllYearsRow => "the all-years row".to_string(),
            Self::FromYear(y) => format!("the row from {}", format_academic_year(y)),
        }
    }
}

/// The filter [`remove`] deletes by.
fn selection_filters(institution_ref: &str, degree_id: &str, which: RowSelection) -> QueryFilters {
    let filters = program_filter(institution_ref, degree_id);
    match which {
        RowSelection::FromYear(year) => filters.eq("from_year", Some(year)),
        RowSelection::AllYearsRow => filters.is_null("from_year"),
        RowSelection::All => filters,
    }
}

/// Remove some or all of one program's rows, then bring its stored programs back in line.
///
/// Through [`reapply`]: a program a remaining row covers is corrected to it; one no row
/// covers any more loses its confirmation note and keeps the CIP it has — the guess it
/// replaced is not known. Returns how many rows went and what changed; with nothing
/// removed, nothing is reapplied.
///
/// # Errors
/// The backend's error.
pub async fn remove(
    client: &DbClient,
    institution_ref: &str,
    degree_id: &str,
    which: RowSelection,
) -> DatabaseResult<(u64, ReapplyReport)> {
    let removed = client
        .delete_where(
            tables::PROGRAM_CIPS,
            &selection_filters(institution_ref, degree_id, which),
        )
        .await?;
    if removed == 0 {
        return Ok((0, ReapplyReport::default()));
    }
    let report = reapply(client, institution_ref, degree_id, false).await?;
    Ok((removed, report))
}

/// Point a program's rows at another `degree_id`, then correct its stored programs.
///
/// For when a build arrives under a new id: the stored programs under the new id are
/// reapplied. Returns how many rows moved and what changed; with nothing moved, nothing
/// is reapplied.
///
/// # Errors
/// The backend's error.
pub async fn relink(
    client: &DbClient,
    institution_ref: &str,
    old_id: &str,
    new_id: &str,
) -> DatabaseResult<(u64, ReapplyReport)> {
    let moved = client
        .update_where(
            tables::PROGRAM_CIPS,
            &program_filter(institution_ref, old_id),
            &serde_json::json!({ "degree_id": new_id }),
        )
        .await?;
    if moved == 0 {
        return Ok((0, ReapplyReport::default()));
    }
    let report = reapply(client, institution_ref, new_id, false).await?;
    Ok((moved, report))
}

/// One registry row as `db cip list` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListedCip {
    /// The school (`institution_ref`).
    pub school: String,
    /// The program.
    pub degree_id: String,
    /// `all years`, or `from 2024-2025`.
    pub applies: String,
    /// The confirmed CIP.
    pub cip_code: String,
    /// Its IPEDS award level, when known.
    pub award_level: Option<i32>,
    /// Whether a stored program has this school and `degree_id`.
    pub built: bool,
    /// The program's name.
    pub name: String,
    /// Where the CIP came from.
    pub source: String,
}

#[derive(Debug, Deserialize)]
struct BuiltKey {
    institution_ref: String,
    #[serde(default)]
    degree_id: Option<String>,
}

/// The registry for the given schools (every school when `refs` is empty), each row
/// marked built or not, by school, `degree_id`, then `from_year`. With `unbuilt_only`,
/// only the programs not built yet.
///
/// Reads `programs` only for the registered schools and ids, never the whole table: an
/// unfiltered select is subject to the backend's row cap, which truncates silently and
/// would show built programs as unbuilt.
///
/// # Errors
/// The backend's error.
pub async fn list(
    client: &DbClient,
    refs: &[String],
    unbuilt_only: bool,
) -> DatabaseResult<Vec<ListedCip>> {
    let mut rows = fetch_for_schools(client, refs).await?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    rows.sort_by(|a, b| {
        (&a.institution_ref, &a.degree_id, a.from_year).cmp(&(
            &b.institution_ref,
            &b.degree_id,
            b.from_year,
        ))
    });
    let schools: Vec<&str> = rows
        .iter()
        .map(|r| r.institution_ref.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let ids: Vec<&str> = rows
        .iter()
        .map(|r| r.degree_id.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let stored: Vec<BuiltKey> = parse_json_array(
        &client
            .select(
                tables::PROGRAMS,
                "institution_ref,degree_id",
                &QueryFilters::new()
                    .in_list("institution_ref", &schools)
                    .in_list("degree_id", &ids),
                None,
            )
            .await?,
    );
    let built: BTreeSet<(String, String)> = stored
        .into_iter()
        .filter_map(|b| Some((b.institution_ref, b.degree_id?)))
        .collect();
    Ok(rows
        .into_iter()
        .map(|r| ListedCip {
            built: built.contains(&(r.institution_ref.clone(), r.degree_id.clone())),
            applies: span_label(r.from_year),
            school: r.institution_ref,
            degree_id: r.degree_id,
            cip_code: r.cip_code,
            award_level: r.award_level,
            name: r.name,
            source: r.source,
        })
        .filter(|c| !unbuilt_only || !c.built)
        .collect())
}

/// A stored program's CIP before and after [`reapply`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CipChange {
    /// The program.
    pub program_key: String,
    /// Its CIP before.
    pub from: Option<String>,
    /// Its CIP now.
    pub to: String,
}

/// What [`reapply`] did to one program's stored rows.
#[derive(Debug, Default, Serialize)]
pub struct ReapplyReport {
    /// Stored programs with this school and `degree_id`.
    pub programs: usize,
    /// Those whose CIP (or confirmation note) changed.
    pub changed: Vec<CipChange>,
    /// Those no registry row covers — their year precedes every dated row, or no row is
    /// left — so they keep the CIP they have.
    pub uncovered: Vec<String>,
    /// Of those, the ones whose earlier confirmation note was removed.
    pub unconfirmed: Vec<String>,
    /// Those whose stored document could not be read; left as they are.
    pub unreadable: Vec<String>,
}

/// The `programs` columns [`reapply`] reads.
const REAPPLY_COLS: &str = "program_key,catalog_year,version_date,cip_code,document";

#[derive(Debug, Deserialize)]
struct StoredRow {
    program_key: String,
    #[serde(default)]
    catalog_year: Option<String>,
    #[serde(default)]
    version_date: Option<String>,
    #[serde(default)]
    cip_code: Option<String>,
    document: serde_json::Value,
}

/// What [`reapply`] does to one stored program.
#[derive(Debug)]
enum Action {
    /// Its document could not be read.
    Unreadable,
    /// No row covers it; `Some` when a stale confirmation note is to be removed.
    Uncovered(Option<DegreeProgram>),
    /// It already carries the row's CIP and note.
    Unchanged,
    /// It takes this row's CIP.
    Confirm(DegreeProgram, String),
}

/// Decide what [`reapply`] does to one stored program — no backend involved.
fn reapply_action(program: &StoredRow, rows: &[ProgramCip]) -> Action {
    let Ok(mut parsed) = from_unified_value(&program.document) else {
        return Action::Unreadable;
    };
    let year = program_year(
        program.catalog_year.as_deref(),
        program.version_date.as_deref(),
    );
    let Some(row) = resolve(rows, year) else {
        return Action::Uncovered(strip_confirmed(&mut parsed).then_some(parsed));
    };
    let current = |cip: Option<&str>| cip == Some(row.cip_code.as_str());
    if current(program.cip_code.as_deref())
        && current(parsed.degree.cip_code.as_deref())
        && parsed.conversion_warnings.last() == Some(&confirmed_note(row))
    {
        return Action::Unchanged;
    }
    apply_confirmed(&mut parsed, row);
    Action::Confirm(parsed, row.cip_code.clone())
}

/// Correct the stored programs of `(institution_ref, degree_id)` to the registry, each
/// catalog year and version against its own year.
///
/// Only the `cip_code` column, the document and its hash change — never the key, and
/// never a stored run (the CIP is not an analysis input). A program the registry does
/// not cover keeps its CIP, but loses a confirmation note left by a row since removed.
/// With `dry_run`, reports what would change and writes nothing.
///
/// Applies to verified programs too: verification vouches for the requirements, and a
/// CIP the school confirmed is not a change to those.
///
/// # Errors
/// The backend's error.
pub async fn reapply(
    client: &DbClient,
    institution_ref: &str,
    degree_id: &str,
    dry_run: bool,
) -> DatabaseResult<ReapplyReport> {
    let rows = fetch(client, institution_ref, degree_id).await?;
    reapply_rows(client, institution_ref, degree_id, &rows, dry_run).await
}

/// [`reapply`] against the given registry rows rather than the stored ones — what a
/// dry run of a registration uses, before the row exists.
async fn reapply_rows(
    client: &DbClient,
    institution_ref: &str,
    degree_id: &str,
    rows: &[ProgramCip],
    dry_run: bool,
) -> DatabaseResult<ReapplyReport> {
    let stored: Vec<StoredRow> = parse_json_array(
        &client
            .select(
                tables::PROGRAMS,
                REAPPLY_COLS,
                &program_filter(institution_ref, degree_id),
                None,
            )
            .await?,
    );
    let mut report = ReapplyReport {
        programs: stored.len(),
        ..ReapplyReport::default()
    };
    for program in stored {
        let key = program.program_key.clone();
        match reapply_action(&program, rows) {
            Action::Unreadable => report.unreadable.push(key),
            Action::Unchanged => {}
            Action::Uncovered(stripped) => {
                if let Some(parsed) = stripped {
                    write_document(client, &key, None, &parsed, dry_run).await?;
                    report.unconfirmed.push(key.clone());
                }
                report.uncovered.push(key);
            }
            Action::Confirm(parsed, cip) => {
                write_document(client, &key, Some(&cip), &parsed, dry_run).await?;
                report.changed.push(CipChange {
                    program_key: key,
                    from: program.cip_code,
                    to: cip,
                });
            }
        }
    }
    Ok(report)
}

/// Store a program's rebuilt document and its hash (and its `cip_code` column, when
/// given), through [`document_and_hash`] — the pair an import builds — so re-importing
/// the source afterwards reports it unchanged.
async fn write_document(
    client: &DbClient,
    program_key: &str,
    cip_code: Option<&str>,
    program: &DegreeProgram,
    dry_run: bool,
) -> DatabaseResult<()> {
    let (document, hash) = document_and_hash(program)?;
    if dry_run {
        return Ok(());
    }
    let mut fields = serde_json::json!({ "document": document, "document_hash": hash });
    if let Some(cip) = cip_code {
        fields["cip_code"] = serde_json::Value::String(cip.to_string());
    }
    client
        .update_where(
            tables::PROGRAMS,
            &QueryFilters::new().eq("program_key", Some(program_key)),
            &fields,
        )
        .await?;
    Ok(())
}

// ============================================================================
// Registering a CIP
// ============================================================================

/// A school as the CLI takes it: an IPEDS unitid, or a non-US `cc:slug` ref.
///
/// Returns the `institution_ref` the registry and `programs` key on, and the unitid when
/// there is one. A program whose school never resolved — its `institution_ref` is a name
/// slug — cannot be named this way; import it with `--unitid` first.
///
/// # Errors
/// A message when the value is neither.
pub fn parse_school(s: &str) -> Result<(String, Option<i32>), String> {
    let s = s.trim();
    if let Ok(unitid) = s.parse::<i32>() {
        if unitid <= 0 {
            return Err(format!(
                "an IPEDS unitid is a positive number, got {unitid}"
            ));
        }
        return Ok((unitid.to_string(), Some(unitid)));
    }
    super::import::parse_institution_ref(s)
        .map(|r| (r.reference, None))
        .map_err(|e| format!("a school is an IPEDS unitid or a non-US ref: {e}"))
}

/// A CIP to register for one program.
#[derive(Debug, Clone)]
pub struct NewCip {
    /// The school: an IPEDS unitid or a `cc:slug` ref ([`parse_school`]).
    pub school: String,
    /// The program's `degree_id`.
    pub degree_id: String,
    /// The CIP the school gave.
    pub cip_code: String,
    /// The academic year it applies from; `None` for all years.
    pub from_year: Option<i32>,
    /// Where it came from.
    pub source: String,
    /// The program's name; required when it is not stored, else taken from it.
    pub name: Option<String>,
    /// Its degree type; overrides the stored program's, which is used when absent.
    pub degree_type: Option<String>,
    /// Its IPEDS award level; derived from the degree type when absent and unambiguous.
    pub award_level: Option<i32>,
    /// Anything else to keep with it.
    pub note: Option<String>,
}

/// What [`register`] did.
#[derive(Debug, Serialize)]
pub struct Registration {
    /// The row as written (or, with `dry_run`, as it would be).
    pub row: ProgramCip,
    /// Whether a stored program has this school and `degree_id`.
    pub built: bool,
    /// When it is not built: the school's stored `degree_id`s, to catch a typo.
    pub school_degree_ids: Vec<String>,
    /// The stored programs corrected.
    pub reapplied: ReapplyReport,
    /// Things worth saying that did not stop it.
    pub warnings: Vec<String>,
}

/// Why [`register`] refused.
#[derive(Debug)]
pub enum RegisterError {
    /// The request itself is wrong; the message says how.
    Invalid(String),
    /// The backend failed.
    Database(DatabaseError),
}

impl From<DatabaseError> for RegisterError {
    fn from(e: DatabaseError) -> Self {
        Self::Database(e)
    }
}

impl std::fmt::Display for RegisterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(m) => f.write_str(m),
            Self::Database(e) => write!(f, "{e}"),
        }
    }
}

/// A stored program at the school, as [`register`] reads it.
#[derive(Debug, Clone, Deserialize)]
struct NameRow {
    #[serde(default)]
    degree_id: Option<String>,
    name: String,
    #[serde(default)]
    degree_type: Option<String>,
}

/// A registration worked out from the request and the school's stored programs, before
/// anything is written.
#[derive(Debug)]
struct Draft {
    row: ProgramCip,
    built: bool,
    school_degree_ids: Vec<String>,
    warnings: Vec<String>,
}

/// The request's own fields, checked: its `degree_id`, trimmed.
fn validate_request(new: &NewCip) -> Result<&str, RegisterError> {
    let degree_id = new.degree_id.trim();
    if degree_id.is_empty() {
        return Err(RegisterError::Invalid("degree_id is empty".to_string()));
    }
    if new.cip_code.trim().is_empty() {
        return Err(RegisterError::Invalid(
            "cip_code is empty: give the CIP the school confirmed, e.g. 11.0701".to_string(),
        ));
    }
    if new.source.trim().is_empty() {
        return Err(RegisterError::Invalid(
            "--source is empty: say where the CIP came from".to_string(),
        ));
    }
    Ok(degree_id)
}

/// The school's stored `degree_id`s, sorted and unique — shown when a registration names
/// none of them, to catch a typo.
fn school_degree_ids(at_school: &[NameRow]) -> Vec<String> {
    at_school
        .iter()
        .filter_map(|p| p.degree_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// The row a registration writes, from the request and the school's stored programs —
/// no backend involved, so every refusal and warning is testable.
fn build_row(
    new: &NewCip,
    institution_ref: &str,
    unitid: Option<i32>,
    at_school: &[NameRow],
) -> Result<Draft, RegisterError> {
    let degree_id = validate_request(new)?;
    let stored = at_school
        .iter()
        .find(|p| p.degree_id.as_deref() == Some(degree_id));
    let built = stored.is_some();
    let Some(name) = new.name.clone().or_else(|| stored.map(|p| p.name.clone())) else {
        return Err(RegisterError::Invalid(format!(
            "no stored program at {institution_ref} has degree_id \"{degree_id}\"; to register \
             one not built yet, pass --name (and --degree-type)"
        )));
    };
    let degree_type = new
        .degree_type
        .clone()
        .or_else(|| stored.and_then(|p| p.degree_type.clone()));
    let award_level = new
        .award_level
        .or_else(|| degree_type.as_deref().and_then(default_award_level));

    let mut warnings = Vec::new();
    // A minor has no IPEDS line, so demographics refuses it whatever its award level.
    if award_level.is_none() && !program_kinds::is_minor(degree_type.as_deref()) {
        warnings.push(degree_type.as_deref().map_or_else(
            || {
                "no award level: no degree type is given or stored; pass --award-level \
                 (or --degree-type)"
                    .to_string()
            },
            |t| {
                format!(
                    "no award level: degree type \"{t}\" does not imply one, so demographics \
                     for this program will need award_level passed (or set it with \
                     --award-level)"
                )
            },
        ));
    }
    let school_degree_ids = if built {
        Vec::new()
    } else {
        warnings.push(format!(
            "not built: it links when a build is imported with degree_id \"{degree_id}\" \
             (`db import --degree-id`)"
        ));
        school_degree_ids(at_school)
    };
    let row = ProgramCip {
        institution_ref: institution_ref.to_string(),
        degree_id: degree_id.to_string(),
        from_year: new.from_year,
        unitid,
        cip_code: new.cip_code.trim().to_string(),
        award_level,
        name,
        degree_type,
        source: new.source.trim().to_string(),
        note: new.note.clone(),
    };
    Ok(Draft {
        row,
        built,
        school_degree_ids,
        warnings,
    })
}

/// Refuse a CIP that is not in `cip_codes`.
async fn ensure_known_cip(client: &DbClient, cip_code: &str) -> Result<(), RegisterError> {
    let known = client
        .select(
            tables::CIP_CODES,
            "cip_code",
            &QueryFilters::new().eq("cip_code", Some(cip_code)),
            Some(1),
        )
        .await?;
    if known.as_array().is_none_or(Vec::is_empty) {
        return Err(RegisterError::Invalid(format!(
            "\"{cip_code}\" is not in cip_codes; CIP codes are dot notation, e.g. 11.0701 \
             (`db query cip --search <words>` finds one)"
        )));
    }
    Ok(())
}

/// Record a confirmed CIP, then correct the stored programs it covers ([`reapply`]).
/// With `dry_run`, checks everything and reports what would change, writing nothing.
///
/// # Errors
/// [`RegisterError::Invalid`] for a bad school, an empty `degree_id`, CIP or source, an
/// unknown CIP, or an unbuilt program with no name; [`RegisterError::Database`] when the
/// backend fails.
pub async fn register(
    client: &DbClient,
    new: NewCip,
    dry_run: bool,
) -> Result<Registration, RegisterError> {
    let (institution_ref, unitid) = parse_school(&new.school).map_err(RegisterError::Invalid)?;
    validate_request(&new)?;
    ensure_known_cip(client, new.cip_code.trim()).await?;
    let at_school: Vec<NameRow> = parse_json_array(
        &client
            .select(
                tables::PROGRAMS,
                "degree_id,name,degree_type",
                &QueryFilters::new().eq("institution_ref", Some(institution_ref.as_str())),
                None,
            )
            .await?,
    );
    let draft = build_row(&new, &institution_ref, unitid, &at_school)?;
    let row = draft.row;
    let reapplied = if dry_run {
        // Reapply against the registry as it would be, without writing it.
        let mut rows = fetch(client, &institution_ref, &row.degree_id).await?;
        rows.retain(|r| r.from_year != row.from_year);
        rows.push(row.clone());
        reapply_rows(client, &institution_ref, &row.degree_id, &rows, true).await?
    } else {
        upsert(client, &row).await?;
        reapply(client, &institution_ref, &row.degree_id, false).await?
    };
    Ok(Registration {
        row,
        built: draft.built,
        school_degree_ids: draft.school_degree_ids,
        reapplied,
        warnings: draft.warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::database::test_backend;
    use crate::core::degree::to_unified_value;

    fn row(from_year: Option<i32>, cip: &str) -> ProgramCip {
        ProgramCip {
            institution_ref: "167358".into(),
            degree_id: "neu-cs".into(),
            from_year,
            unitid: Some(167_358),
            cip_code: cip.into(),
            award_level: Some(5),
            name: "Computer Science BS".into(),
            degree_type: Some("BS".into()),
            source: "registrar email 2026-11-02".into(),
            note: None,
        }
    }

    #[test]
    fn test_parse_school_takes_a_unitid_or_a_non_us_ref() {
        assert_eq!(
            parse_school(" 167358 "),
            Ok(("167358".to_string(), Some(167_358)))
        );
        assert_eq!(
            parse_school("CA:university-of-toronto"),
            Ok(("ca:university-of-toronto".to_string(), None))
        );
        for bad in [
            "",
            "northeastern",
            "ca:",
            "ca:Bad Slug",
            "16735x",
            "-5",
            "0",
        ] {
            assert!(parse_school(bad).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn test_first_year_in_needs_four_bounded_digits() {
        assert_eq!(first_year_in("AY2024-25"), Some(2024));
        assert_eq!(first_year_in("Été 2026"), Some(2026));
        for none in ["12345", "123", "20242025", ""] {
            assert_eq!(first_year_in(none), None, "{none:?}");
        }
        // The academic year turns over on July 1.
        assert_eq!(program_year(None, Some("2026-06-30")), Some(2025));
        assert_eq!(program_year(None, Some("2026-07-01")), Some(2026));
        assert_eq!(
            program_year(Some("TBD"), Some("2026-07-01")),
            Some(2026),
            "an unreadable catalog year falls back to the version date"
        );
        assert_eq!(program_year(None, Some("not a date")), None);
    }

    #[test]
    fn test_confirmed_note_names_the_span_and_the_source() {
        assert_eq!(
            confirmed_note(&row(None, "11.0701")),
            "cip_code confirmed: 11.0701 (all years) — registrar email 2026-11-02"
        );
        assert!(confirmed_note(&row(Some(2024), "11.0102")).starts_with(CONFIRMED_NOTE_PREFIX));
    }

    #[test]
    fn test_row_selection_narrows_the_delete() {
        let cols = |w| {
            selection_filters("167358", "neu-cs", w)
                .entries
                .iter()
                .map(|(k, c, v)| format!("{c}={k:?}:{v}"))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            cols(RowSelection::All),
            ["institution_ref=Eq:167358", "degree_id=Eq:neu-cs"]
        );
        assert_eq!(cols(RowSelection::AllYearsRow)[2], "from_year=IsNull:");
        assert_eq!(cols(RowSelection::FromYear(2024))[2], "from_year=Eq:2024");
    }

    fn new_cip() -> NewCip {
        NewCip {
            school: "167358".into(),
            degree_id: "neu-ai".into(),
            cip_code: "11.0102".into(),
            from_year: None,
            source: "dept".into(),
            name: None,
            degree_type: None,
            award_level: None,
            note: None,
        }
    }

    fn at_school() -> Vec<NameRow> {
        vec![NameRow {
            degree_id: Some("neu-cs".into()),
            name: "Computer Science".into(),
            degree_type: Some("BS".into()),
        }]
    }

    #[test]
    fn test_build_row_takes_a_stored_programs_name_and_type() {
        let stored = NewCip {
            degree_id: " neu-cs ".into(),
            ..new_cip()
        };
        let Draft {
            row,
            built,
            school_degree_ids: ids,
            warnings,
        } = build_row(&stored, "167358", Some(167_358), &at_school()).unwrap();
        assert!(
            built && ids.is_empty() && warnings.is_empty(),
            "{warnings:?}"
        );
        assert_eq!(row.degree_id, "neu-cs");
        assert_eq!(row.name, "Computer Science");
        assert_eq!(row.award_level, Some(5));
    }

    #[test]
    fn test_build_row_needs_a_name_for_an_unbuilt_program() {
        let err = build_row(&new_cip(), "167358", Some(167_358), &at_school()).unwrap_err();
        assert!(err.to_string().contains("--name"), "{err}");
        let named = NewCip {
            name: Some("BS in AI".into()),
            degree_type: Some("BS".into()),
            ..new_cip()
        };
        let Draft {
            row,
            built,
            school_degree_ids: ids,
            warnings,
        } = build_row(&named, "167358", Some(167_358), &at_school()).unwrap();
        assert!(!built);
        assert_eq!(ids, ["neu-cs"]);
        assert_eq!(row.award_level, Some(5));
        assert!(
            warnings.iter().any(|w| w.starts_with("not built")),
            "{warnings:?}"
        );
        let cert = NewCip {
            degree_type: Some("CERT".into()),
            ..named
        };
        let draft = build_row(&cert, "167358", Some(167_358), &at_school()).unwrap();
        assert!(draft
            .warnings
            .iter()
            .any(|w| w.starts_with("no award level") && w.contains("\"CERT\"")));
        let untyped = NewCip {
            degree_type: None,
            ..cert
        };
        let draft = build_row(&untyped, "167358", Some(167_358), &at_school()).unwrap();
        assert!(draft.warnings.iter().any(|w| w.contains("no degree type")));
        for minor in ["minor", "Minor", " MINOR "] {
            let typed = NewCip {
                degree_type: Some(minor.into()),
                ..untyped.clone()
            };
            let draft = build_row(&typed, "167358", Some(167_358), &at_school()).unwrap();
            assert_eq!(draft.row.award_level, None);
            assert!(
                !draft
                    .warnings
                    .iter()
                    .any(|w| w.starts_with("no award level")),
                "{minor}: {:?}",
                draft.warnings
            );
        }
        for bad in [
            NewCip {
                degree_id: "  ".into(),
                ..new_cip()
            },
            NewCip {
                source: " ".into(),
                ..new_cip()
            },
            NewCip {
                cip_code: String::new(),
                ..new_cip()
            },
        ] {
            assert!(build_row(&bad, "167358", None, &at_school()).is_err());
        }
    }

    /// A stored-programs row as `reapply` reads it.
    fn stored_json(
        key: &str,
        catalog: &str,
        cip: &str,
        confirmed: Option<&ProgramCip>,
    ) -> serde_json::Value {
        let mut p = DegreeProgram {
            degree: crate::core::models::Degree::new(
                "Computer Science".into(),
                "BS".into(),
                Some(cip.into()),
                "semester".into(),
            ),
            requirements: std::collections::HashMap::new(),
            courses: std::collections::HashMap::new(),
            conversion_warnings: Vec::new(),
            corrections_applied: Vec::new(),
        };
        if let Some(r) = confirmed {
            apply_confirmed(&mut p, r);
        }
        serde_json::json!({
            "program_key": key, "catalog_year": catalog, "version_date": null,
            "cip_code": cip, "document": to_unified_value(&p).unwrap(),
        })
    }

    #[tokio::test]
    async fn test_reapply_corrects_each_year_and_a_dry_run_writes_nothing() {
        let reg = row(Some(2024), "11.0102");
        let programs = serde_json::json!([
            stored_json("p:2023", "2023-2024", "11.0701", None),
            stored_json("p:2024", "2024-2025", "11.0701", None),
            stored_json("p:done", "2025-2026", "11.0102", Some(&reg)),
        ])
        .to_string();
        let backend = test_backend::routed(vec![
            (
                "/rest/v1/program_cips",
                serde_json::to_string(&[reg]).unwrap(),
            ),
            ("/rest/v1/programs", programs),
        ])
        .await;
        let dry = reapply(&backend.client, "167358", "neu-cs", true)
            .await
            .unwrap();
        assert_eq!(dry.programs, 3);
        assert_eq!(dry.uncovered, ["p:2023"], "it precedes the dated row");
        let changed: Vec<&str> = dry.changed.iter().map(|c| c.program_key.as_str()).collect();
        assert_eq!(changed, ["p:2024"], "p:done is already confirmed");
        assert_eq!(backend.write_count(), 0, "a dry run writes nothing");
        reapply(&backend.client, "167358", "neu-cs", false)
            .await
            .unwrap();
        assert_eq!(
            backend.write_count(),
            1,
            "only the changed program is patched"
        );
    }

    #[tokio::test]
    async fn test_reapply_strips_the_note_a_removed_row_left() {
        // The registry is now empty: the program keeps its CIP but not the note.
        let gone = row(None, "11.0102");
        let programs = serde_json::json!([stored_json("p:1", "2024-2025", "11.0102", Some(&gone))])
            .to_string();
        let backend = test_backend::routed(vec![
            ("/rest/v1/program_cips", "[]".to_string()),
            ("/rest/v1/programs", programs),
        ])
        .await;
        let report = reapply(&backend.client, "167358", "neu-cs", false)
            .await
            .unwrap();
        assert_eq!(report.unconfirmed, ["p:1"]);
        assert!(report.changed.is_empty());
        assert_eq!(backend.write_count(), 1);
    }

    /// A `programs` row both `register`'s school read and `reapply` can parse.
    fn stored_at_school(
        key: &str,
        catalog: &str,
        cip: &str,
        confirmed: Option<&ProgramCip>,
    ) -> serde_json::Value {
        let mut v = stored_json(key, catalog, cip, confirmed);
        v["degree_id"] = "neu-cs".into();
        v["name"] = "Computer Science".into();
        v["degree_type"] = "BS".into();
        v
    }

    #[tokio::test]
    async fn test_register_dry_run_reapplies_against_the_merged_history() {
        // A dated row exists; the dry run proposes an all-years row. 2023 becomes covered
        // by the proposal; 2024 stays on the stored row.
        let existing = row(Some(2024), "11.0102");
        let programs = serde_json::json!([
            stored_at_school("p:2023", "2023-2024", "11.0101", None),
            stored_at_school("p:2024", "2024-2025", "11.0102", Some(&existing)),
        ])
        .to_string();
        let backend = test_backend::routed(vec![
            (
                "/rest/v1/cip_codes",
                r#"[{"cip_code":"11.0701"}]"#.to_string(),
            ),
            (
                "/rest/v1/program_cips",
                serde_json::to_string(&[existing]).unwrap(),
            ),
            ("/rest/v1/programs", programs),
        ])
        .await;
        let new = NewCip {
            degree_id: "neu-cs".into(),
            cip_code: "11.0701".into(),
            ..new_cip()
        };
        let r = register(&backend.client, new, true).await.unwrap();
        assert!(r.built);
        let changed: Vec<&str> = r
            .reapplied
            .changed
            .iter()
            .map(|c| c.program_key.as_str())
            .collect();
        assert_eq!(changed, ["p:2023"], "{:?}", r.reapplied);
        assert!(r.reapplied.uncovered.is_empty());
        assert_eq!(
            backend.write_count(),
            0,
            "a dry run writes nothing, not even the row"
        );
    }

    #[tokio::test]
    async fn test_register_writes_the_row_then_patches_what_it_covers() {
        let backend = test_backend::routed(vec![
            (
                "/rest/v1/cip_codes",
                r#"[{"cip_code":"11.0701"}]"#.to_string(),
            ),
            (
                "/rest/v1/program_cips",
                serde_json::to_string(&[row(None, "11.0701")]).unwrap(),
            ),
            (
                "/rest/v1/programs",
                serde_json::json!([stored_at_school("p:1", "2024-2025", "11.0102", None)])
                    .to_string(),
            ),
        ])
        .await;
        let new = NewCip {
            degree_id: "neu-cs".into(),
            cip_code: "11.0701".into(),
            ..new_cip()
        };
        let r = register(&backend.client, new, false).await.unwrap();
        assert_eq!(r.reapplied.changed.len(), 1);
        assert_eq!(backend.write_count(), 2, "one upsert, one PATCH");
    }

    #[tokio::test]
    async fn test_reapply_leaves_an_unreadable_document_alone() {
        let mut bad = stored_json("p:bad", "2024-2025", "11.0701", None);
        bad["document"] = serde_json::json!("not a document");
        let backend = test_backend::routed(vec![
            (
                "/rest/v1/program_cips",
                serde_json::to_string(&[row(None, "11.0102")]).unwrap(),
            ),
            ("/rest/v1/programs", serde_json::json!([bad]).to_string()),
        ])
        .await;
        let report = reapply(&backend.client, "167358", "neu-cs", false)
            .await
            .unwrap();
        assert_eq!(report.programs, 1);
        assert_eq!(report.unreadable, ["p:bad"]);
        assert!(report.changed.is_empty() && report.uncovered.is_empty());
        assert_eq!(backend.write_count(), 0);
    }

    #[tokio::test]
    async fn test_relink_and_remove_report_the_rows_they_touched() {
        // The stub answers every write `*/1` and holds no stored programs.
        let backend = test_backend::routed(Vec::new()).await;
        let (moved, report) = relink(&backend.client, "167358", "old", "neu-cs")
            .await
            .unwrap();
        assert_eq!((moved, report.programs), (1, 0));
        let (n, report) = remove(&backend.client, "167358", "neu-cs", RowSelection::All)
            .await
            .unwrap();
        assert_eq!((n, report.programs), (1, 0));
        assert_eq!(backend.write_count(), 2, "one PATCH, one DELETE");
    }

    #[test]
    fn test_row_selection_from_flags_and_its_label() {
        assert_eq!(RowSelection::from_flags(false, None), RowSelection::All);
        assert_eq!(
            RowSelection::from_flags(true, None),
            RowSelection::AllYearsRow
        );
        assert_eq!(
            RowSelection::from_flags(true, Some(2024)),
            RowSelection::AllYearsRow
        );
        assert_eq!(
            RowSelection::from_flags(false, Some(2024)),
            RowSelection::FromYear(2024)
        );
        assert_eq!(
            RowSelection::FromYear(2024).label(),
            "the row from 2024-2025"
        );
    }

    #[tokio::test]
    async fn test_list_marks_built_programs_and_filters_unbuilt() {
        let mut unbuilt = row(None, "11.0102");
        unbuilt.degree_id = "neu-ai".into();
        let dated = row(Some(2024), "11.0103");
        let backend = test_backend::routed(vec![
            (
                "/rest/v1/program_cips",
                serde_json::to_string(&[dated, row(None, "11.0701"), unbuilt]).unwrap(),
            ),
            (
                "/rest/v1/programs",
                r#"[{"institution_ref":"167358","degree_id":"neu-cs"}]"#.to_string(),
            ),
        ])
        .await;
        let all = list(&backend.client, &[], false).await.unwrap();
        let got: Vec<(&str, &str, bool)> = all
            .iter()
            .map(|c| (c.degree_id.as_str(), c.applies.as_str(), c.built))
            .collect();
        assert_eq!(
            got,
            [
                ("neu-ai", "all years", false),
                ("neu-cs", "all years", true),
                ("neu-cs", "from 2024-2025", true),
            ],
            "by school, degree_id, then from_year"
        );
        let unbuilt = list(&backend.client, &[], true).await.unwrap();
        assert_eq!(unbuilt.len(), 1);
        // An empty registry reads nothing more.
        let empty = test_backend::routed(Vec::new()).await;
        assert!(list(&empty.client, &[], false).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_register_refuses_an_unknown_cip_before_writing() {
        let backend = test_backend::routed(vec![("/rest/v1/cip_codes", "[]".to_string())]).await;
        let err = register(&backend.client, new_cip(), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not in cip_codes"), "{err}");
        assert_eq!(backend.write_count(), 0);
    }

    #[test]
    fn test_resolve_follows_the_history() {
        // all years → 11.0701, then 11.0102 from 2024-2025.
        let rows = [row(None, "11.0701"), row(Some(2024), "11.0102")];
        let cip = |y| resolve(&rows, y).map(|r| r.cip_code.as_str());
        assert_eq!(cip(Some(2022)), Some("11.0701"));
        assert_eq!(cip(Some(2023)), Some("11.0701"));
        assert_eq!(cip(Some(2024)), Some("11.0102"));
        assert_eq!(cip(Some(2030)), Some("11.0102"));
        assert_eq!(cip(None), Some("11.0102"), "no year: the current CIP");
    }

    #[test]
    fn test_resolve_leaves_years_before_every_dated_row_unconfirmed() {
        let rows = [row(Some(2024), "11.0102"), row(Some(2026), "11.0103")];
        assert_eq!(resolve(&rows, Some(2023)), None);
        assert_eq!(resolve(&rows, Some(2025)).unwrap().cip_code, "11.0102");
        assert_eq!(resolve(&rows, Some(2026)).unwrap().cip_code, "11.0103");
        assert_eq!(resolve(&[], Some(2024)), None);
        assert_eq!(resolve(&[], None), None);
    }

    #[test]
    fn test_program_year_reads_the_stored_catalog_years() {
        for (catalog, want) in [
            ("2026-2027", Some(2026)),
            ("2026-2028", Some(2026)),
            ("2025", Some(2025)),
            ("Fall 2026", Some(2026)),
            ("Spring/Summer 2027", Some(2027)),
            ("", None),
            ("catalog 12345", None),
        ] {
            assert_eq!(program_year(Some(catalog), None), want, "{catalog:?}");
        }
        // A blank catalog year falls back to the version date's academic year.
        assert_eq!(program_year(Some(""), Some("2026-10-09")), Some(2026));
        assert_eq!(program_year(None, Some("2026-03-15")), Some(2025));
        assert_eq!(program_year(None, None), None);
    }

    #[test]
    fn test_ipeds_survey_year_is_the_academic_year_before() {
        // C2024_A counts July 2023 - June 2024: academic year 2023-2024.
        assert_eq!(ipeds_academic_year(2024), 2023);
        let rows = [row(None, "11.0701"), row(Some(2024), "11.0102")];
        // A recode from 2024-2025 first shows in IPEDS 2025, not 2024.
        let at = |survey| resolve(&rows, Some(ipeds_academic_year(survey))).unwrap();
        assert_eq!(at(2024).cip_code, "11.0701");
        assert_eq!(at(2025).cip_code, "11.0102");
    }

    #[test]
    fn test_parse_academic_year_refuses_a_bare_year() {
        assert_eq!(parse_academic_year("2024-2025"), Ok(2024));
        assert_eq!(parse_academic_year(" 2024-2025 "), Ok(2024));
        for bad in [
            "2024",
            "2024-2026",
            "2024-25",
            "24-25",
            "2025-2024",
            "",
            "abcd-efgh",
        ] {
            assert!(parse_academic_year(bad).is_err(), "{bad:?} accepted");
        }
        assert_eq!(format_academic_year(2024), "2024-2025");
    }

    #[test]
    fn test_default_award_level_only_where_unambiguous() {
        for (t, want) in [
            ("BS", Some(5)),
            ("ba", Some(5)),
            ("MS", Some(7)),
            ("AS", Some(3)),
            ("CERT", None),
            ("MINOR", None),
            ("certificate", None),
            ("", None),
        ] {
            assert_eq!(default_award_level(t), want, "{t:?}");
        }
    }

    #[test]
    fn test_apply_confirmed_replaces_its_own_note_and_keeps_the_rest() {
        let mut program = DegreeProgram {
            degree: crate::core::models::Degree::new(
                "Computer Science".into(),
                "BS".into(),
                Some("11.0701".into()),
                "semester".into(),
            ),
            requirements: std::collections::HashMap::new(),
            courses: std::collections::HashMap::new(),
            conversion_warnings: vec![
                "degree_type confirmed BS; cip_code 11.0701 inferred from name".into(),
            ],
            corrections_applied: Vec::new(),
        };
        apply_confirmed(&mut program, &row(None, "11.0102"));
        apply_confirmed(&mut program, &row(Some(2024), "11.0103"));
        assert_eq!(program.degree.cip_code.as_deref(), Some("11.0103"));
        assert_eq!(
            program.conversion_warnings.len(),
            2,
            "{:?}",
            program.conversion_warnings
        );
        assert!(program.conversion_warnings[0].starts_with("degree_type confirmed"));
        assert_eq!(
            program.conversion_warnings[1],
            "cip_code confirmed: 11.0103 (from 2024-2025) — registrar email 2026-11-02"
        );
    }
}
