//! IPEDS CSV ingestion functions.
//!
//! Parses locally downloaded IPEDS CSV (or zip-compressed CSV) files and upserts
//! records into Supabase. Files must be downloaded manually from
//! <https://nces.ed.gov/ipeds/use-the-data>.
//!
//! ## Expected files
//! | Survey | File pattern | Required columns |
//! |--------|-------------|-----------------|
//! | HD (institutions) | `HD{year}.csv` | `UNITID`, `INSTNM` |
//! | `C_A` (completions) | `C{year}_A.csv` | `UNITID`, `CIPCODE`, `AWLEVEL` |
//!
//! The `C_A` file is read in a single pass that produces two outputs:
//! - `completions` table — **every** row of the file, all CIP codes and both
//!   `MAJORNUM` values. There is no CIP filter on ingest; callers filter at query time.
//!   Measured against `C2025_A.csv`: 313,566 rows in, 313,566 rows stored.
//! - `institution_completion_totals` table — totals across all CIP codes per institution,
//!   used as the denominator for demographic representation calculations
//!
//! ## IPEDS sentinel values
//! `"."` and empty mean "no value" in every column. Beyond that the two kinds of column
//! disagree, so they have separate parsers: `parse_ipeds_code` for categorical codes and
//! `parse_ipeds_count` for counts. A code column stores everything else, `99` and the
//! negatives included, because `lookup-seed.sql` gives each of them a label. A count
//! column stores `99` as ninety-nine and rejects negatives as impossible.
//!
//! ## Column name variants
//! IPEDS column names change across survey years. `find_col` (an internal
//! helper) accepts multiple
//! candidate names and matches case-insensitively, e.g. Carnegie class uses
//! `C21BASIC`, `C18BASIC`, `C15BASIC` or `CCBASIC` depending on the year.
//!
//! **Candidate order is load-bearing, newest first.** `find_col` returns the first
//! candidate present, and several HD years carry more than one vintage of the same
//! measure — HD2022 has all four Carnegie columns at once.

use std::io::{Cursor, Read};
use std::path::Path;

use crate::core::database::client::DbClient;
use crate::core::database::error::{DatabaseError, DatabaseResult};
use crate::core::database::models::{Completion, Institution, InstitutionCompletionTotal};
use crate::core::database::tables;

/// Statistics from a completed ingest operation.
#[derive(Debug, Default)]
pub struct IngestStats {
    /// Total rows read from the source file
    pub rows_read: usize,
    /// Rows with a parsable UNITID and a non-empty name. No CIP filter is applied.
    pub rows_filtered: usize,
    /// Rows successfully upserted to the database
    pub rows_upserted: usize,
    /// Rows skipped due to missing required fields or parse errors
    pub rows_skipped: usize,
}

/// Batch size for Supabase upsert operations — balances memory usage and network round-trips.
const UPSERT_BATCH_SIZE: usize = 500;

/// Conflict-resolution columns for the `completions` upsert.
const COMPLETIONS_CONFLICT: &[&str] = &["unitid", "cip_code", "award_level", "major_num", "year"];

/// Conflict-resolution columns for the `institution_completion_totals` upsert.
const INST_TOTALS_CONFLICT: &[&str] = &["unitid", "award_level", "year"];

/// Returns `true` if a CIP code is in scope for ingestion.
///
/// Relevant codes: CIP family 11 (Computer and Information Sciences),
/// `30.7099` (Multi/Interdisciplinary Studies, Other), and `30.7001` (Data Science).
///
/// Accepts both dot-notation (`"11.0101"`) and raw integer form (`"110101"`).
#[must_use]
pub fn is_relevant_cip(code: &str) -> bool {
    let normalized: String = code.chars().filter(char::is_ascii_digit).collect();
    normalized.starts_with("11") || normalized == "307099" || normalized == "307001"
}

/// Read a file, automatically extracting from a `.zip` archive if needed.
///
/// Returns the CSV content as a `String`. For zip files, the first `.csv` entry
/// in the archive is extracted.
/// Explain why importing `importing` would discard newer data, or `None` if it is safe.
///
/// The `institutions` table holds one row per institution, not one per year, and the
/// upsert is last-write-wins on `unitid` — so whichever HD file runs *last* decides every
/// attribute. `HD2022` having been imported after the later years left 5,784 of 5,985
/// institutions carrying 2022 values under a table the schema documents as current, and
/// `carnegie_class` wrong for 57% of them. Nothing detected it for months.
///
/// `updated_year` recorded enough to catch it and was never read. This reads it.
///
/// Completions are unaffected — `year` is part of their natural key, so years cannot
/// overwrite one another — which is why this guards only the HD half.
#[must_use]
pub fn downgrade_refusal(
    importing: u16,
    newest_stored: Option<i32>,
    force: bool,
) -> Option<String> {
    if force {
        return None;
    }
    let newest = newest_stored?;
    if i32::from(importing) >= newest {
        return None;
    }
    let mut msg = format!(
        "refusing to import HD{importing}: the institutions table already holds data from {newest},"
    );
    msg.push_str(" and this import would overwrite every shared institution with the");
    msg.push_str(" older year's values. Import oldest-year-first, or pass --force if");
    msg.push_str(" that is what you want.");
    Some(msg)
}

/// Choose which CSV to read from an IPEDS archive, preferring the **revised** release.
///
/// `C2022_A.zip` and `C2023_A.zip` each hold two: the provisional `c2022_a.csv` and the
/// revised `c2022_a_rv.csv`. IPEDS publishes the provisional file first and supersedes it
/// months later with corrections, shipping both in the same archive. Taking the first
/// entry — which is what this did — imported the superseded data: for 2022 that is 904
/// differing totals over the shared rows, 229 rows the revision adds, and 51 it retracts.
///
/// Matched on the `_rv` stem suffix, case-insensitively, since the archives are
/// inconsistent about case (`c2022_a_rv.csv` but `C2023_a_RV.csv`).
fn pick_csv_entry<R: std::io::Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Option<usize> {
    let names: Vec<(usize, String)> = (0..archive.len())
        .filter_map(|i| archive.by_index(i).ok().map(|f| (i, f.name().to_string())))
        .filter(|(_, name)| {
            std::path::Path::new(name)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
        })
        .collect();
    let is_revised = |name: &str| {
        std::path::Path::new(name)
            .file_stem()
            .and_then(|s| s.to_str())
            .is_some_and(|stem| stem.to_ascii_lowercase().ends_with("_rv"))
    };
    names
        .iter()
        .find(|(_, name)| is_revised(name))
        .or_else(|| names.first())
        .map(|(i, _)| *i)
}

pub(crate) fn read_file_or_zip(path: &Path) -> DatabaseResult<String> {
    let bytes = std::fs::read(path)
        .map_err(|e| DatabaseError::IngestError(format!("Cannot read {}: {e}", path.display())))?;

    if path.extension().and_then(|e| e.to_str()) == Some("zip") {
        let cursor = Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(cursor)
            .map_err(|e| DatabaseError::IngestError(format!("Cannot open zip: {e}")))?;
        let csv_index = pick_csv_entry(&mut archive).ok_or_else(|| {
            DatabaseError::IngestError(format!("No CSV entry found inside {}", path.display()))
        })?;
        let mut file = archive
            .by_index(csv_index)
            .map_err(|e| DatabaseError::IngestError(format!("Cannot read zip entry: {e}")))?;
        let entry_name = file.name().to_string();
        let mut entry_bytes = Vec::new();
        file.read_to_end(&mut entry_bytes).map_err(|e| {
            DatabaseError::IngestError(format!(
                "Cannot read {entry_name} inside {}: {e}",
                path.display()
            ))
        })?;
        Ok(decode_ipeds_bytes(
            &entry_bytes,
            &format!("{entry_name} inside {}", path.display()),
        ))
    } else {
        Ok(decode_ipeds_bytes(&bytes, &path.display().to_string()))
    }
}

/// Decode IPEDS bytes as UTF-8, falling back to CP1252.
///
/// IPEDS has historically shipped CP1252, and the two read paths used to disagree about
/// it: the zip path used a strict UTF-8 read, so one `\xe9` (the `é` in a trustee name)
/// aborted a 6,256-row import, while the loose-CSV path used `from_utf8_lossy` and
/// silently turned the same byte into `U+FFFD`, corrupting the institution's name with no
/// signal at all.
///
/// CP1252 rather than Latin-1 on purpose: they differ over `0x80..=0x9F`, where CP1252
/// has the smart quotes and em dash that appear in institution names and Latin-1 has C1
/// control codes. Decoding CP1252 cannot fail — every byte maps — so this returns a
/// `String` rather than a `Result`.
fn decode_ipeds_bytes(bytes: &[u8], source: &str) -> String {
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(e) => {
            // Reported, not silent: a file that needed the fallback is worth knowing
            // about, and the byte offset is what makes a bad row findable.
            let (text, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
            crate::info!(
                "{source} is not valid UTF-8 at byte {}; decoded as CP1252 instead",
                e.valid_up_to()
            );
            text.into_owned()
        }
    }
}

/// IPEDS "not applicable" — the only in-band marker meaning "there is no value here".
/// Every other code IPEDS emits is modelled in `lookup-seed.sql` and stored as-is.
const NOT_APPLICABLE: &str = ".";

/// Parse an IPEDS **categorical code** field, returning `None` only when there is no code.
///
/// Missing is [`NOT_APPLICABLE`] and empty. **Every other code is stored, including the
/// negatives and `99`**, because `lookup-seed.sql` models them as real rows with labels:
/// `institution_sector (99, 'Not classified')`, `institution_size (-2, 'Not applicable')`
/// and `(-1, 'Not reported')`, `institution_control (-3, 'Not available')`. Nulling them
/// would collapse "IPEDS told us it is not classified" into "we have no value", and leave
/// seed rows that no row could ever reference.
///
/// Verified against `HD2025.csv`: every value in all six categorical columns has a
/// matching lookup row, so this cannot produce an orphan. A future survey year that
/// introduces a code the seeds do not carry *would* — `db validate` is what catches that.
///
/// **Not for count columns.** See [`parse_ipeds_count`], where `99` is a number and
/// negatives are impossible.
pub(crate) fn parse_ipeds_code(val: &str) -> Option<i32> {
    let v = val.trim();
    if v == NOT_APPLICABLE || v.is_empty() {
        None
    } else {
        v.parse().ok()
    }
}

/// Parse an IPEDS **count** field, returning `None` for sentinels and impossible values.
///
/// Differs from [`parse_ipeds_code`] in both directions:
///
/// - **`99` is a number here, not a sentinel** — ninety-nine graduates. In `C2025_A.csv`
///   the 21 count columns hold `99` 566 times and every one carries the imputation flag
///   `R` (Reported); `IPEDS` marks suppression in the parallel `X`-prefixed flag columns,
///   never with an in-band value.
/// - **Every negative is missing.** A count cannot be negative, and
///   [`accumulate_demo_totals`] sums these into the institution totals that every
///   representation ratio divides by, so one negative would silently shrink a
///   denominator. `-1`, `-2` and `-3` are meaningful as *categorical* codes and
///   meaningless here.
fn parse_ipeds_count(val: &str) -> Option<i32> {
    let v = val.trim();
    if v == NOT_APPLICABLE || v.is_empty() {
        return None;
    }
    v.parse().ok().filter(|&n| n >= 0)
}

/// Carnegie classification source columns, **newest vintage first**.
///
/// Order is load-bearing and `find_col` takes the first match. HD2022 and HD2023 carry
/// `C21BASIC` *and* `C18BASIC`, and they disagree for 1,275 of 6,256 institutions — so
/// listing `C18BASIC` first imported 2018-vintage codes under the 2021 label that
/// `lookup-seed.sql` and `schema.sql` both apply to this column. `CCBASIC` is the
/// pre-2015 name; `CBASIC` never existed in any survey year.
///
/// Deliberately excludes `CARNEGIE`/`C00CARNEGIE`: their code space includes 40 and
/// 51-60, which the `carnegie_class` lookup table has no rows for.
///
/// Shared with `db validate`'s provenance check, which reports which of these the stored
/// data actually agrees with. It must be the same list, not a copy — a copy is how the
/// first guard for this silently stopped guarding anything.
pub(crate) const HD_CARNEGIE_CANDIDATES: &[&str] = &["C21BASIC", "C18BASIC", "C15BASIC", "CCBASIC"];

/// Column indexes for the HD (institution directory) survey.
///
/// Mirrors [`DemoCols`] for the completions survey. Every field is optional because IPEDS
/// renames and drops columns between survey years; a missing column yields `None` rather
/// than failing the import.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct HdCols {
    city: Option<usize>,
    state: Option<usize>,
    sector: Option<usize>,
    control: Option<usize>,
    iclevel: Option<usize>,
    carnegie: Option<usize>,
    hbcu: Option<usize>,
    tribal: Option<usize>,
    locale: Option<usize>,
    inst_size: Option<usize>,
}

impl HdCols {
    /// Build column indices from IPEDS HD (institution directory) survey headers.
    ///
    /// A function rather than an inline literal so the **candidate order** is testable.
    /// Order decides which column wins when a survey year ships several vintages of the
    /// same measure, and getting it wrong is silent — see the Carnegie note below.
    pub(crate) fn for_hd(headers: &[String]) -> Self {
        macro_rules! col {
            ($($n:expr),+) => { find_col(headers, &[$($n),+]) };
        }
        Self {
            city: col!("CITY"),
            state: col!("STABBR"),
            sector: col!("SECTOR"),
            control: col!("CONTROL"),
            iclevel: col!("ICLEVEL"),
            carnegie: find_col(headers, HD_CARNEGIE_CANDIDATES),
            hbcu: col!("HBCU"),
            tribal: col!("TRIBAL"),
            locale: col!("LOCALE"),
            inst_size: col!("INSTSIZE"),
        }
    }
}

/// Build one [`Institution`] row from an HD record.
///
/// Split out of `ingest_institutions` so the parser choice is testable: every column here
/// is categorical and must use [`parse_ipeds_code`], and nothing else in the suite would
/// catch this being switched to [`parse_ipeds_count`] — which would store `SECTOR = 99`
/// as sector ninety-nine.
pub(crate) fn build_institution(
    unitid: i32,
    name: String,
    year: u16,
    cols: &HdCols,
    record: &csv::StringRecord,
) -> Institution {
    let code = |col: Option<usize>| col.and_then(|i| record.get(i)).and_then(parse_ipeds_code);
    let text = |col: Option<usize>| {
        col.and_then(|i| record.get(i))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let flag = |col: Option<usize>| col.and_then(|i| record.get(i)).and_then(parse_ipeds_bool);

    Institution {
        unitid,
        name,
        city: text(cols.city),
        state: text(cols.state),
        sector: code(cols.sector),
        control: code(cols.control),
        iclevel: code(cols.iclevel),
        carnegie_class: code(cols.carnegie),
        hbcu: flag(cols.hbcu),
        tribal: flag(cols.tribal),
        locale: code(cols.locale),
        inst_size: code(cols.inst_size),
        updated_year: Some(i32::from(year)),
    }
}

/// Parse an IPEDS boolean flag: `"1"` → `true`, `"2"` → `false`, anything else → `None`.
fn parse_ipeds_bool(val: &str) -> Option<bool> {
    match val.trim() {
        "1" => Some(true),
        "2" => Some(false),
        _ => None,
    }
}

/// Look up a column index by trying candidate names against pre-uppercased headers.
///
/// IPEDS column names change between survey years; pass multiple candidates in
/// priority order (most recent first) and the first match wins.
pub(crate) fn find_col(headers_upper: &[String], candidates: &[&str]) -> Option<usize> {
    candidates
        .iter()
        .find_map(|&name| headers_upper.iter().position(|h| h == name))
}

/// [`find_col`] that returns a `ParseError` naming the first candidate and the
/// source file when no candidate matches — used during column-resolution at the
/// top of each ingest pass.
fn require_col(
    headers_upper: &[String],
    candidates: &[&str],
    path: &Path,
) -> DatabaseResult<usize> {
    find_col(headers_upper, candidates).ok_or_else(|| {
        DatabaseError::ParseError(format!(
            "{} column not found in {}",
            candidates[0],
            path.display()
        ))
    })
}

/// Uppercase all CSV headers once, for reuse across all [`find_col`] calls.
pub(crate) fn uppercase_headers(record: &csv::StringRecord) -> Vec<String> {
    record.iter().map(str::to_uppercase).collect()
}

/// Open a CSV reader from file content.
pub(crate) fn open_csv(content: &str) -> csv::Reader<&[u8]> {
    csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(content.as_bytes())
}

/// Flush `batch` to the database and add the count to `stats.rows_upserted`.
async fn flush_batch<T: serde::Serialize>(
    client: &DbClient,
    batch: &mut Vec<T>,
    table: &str,
    conflict_cols: &[&str],
    rows_upserted: &mut usize,
) -> DatabaseResult<()> {
    if batch.is_empty() {
        return Ok(());
    }
    let n = batch.len();
    client
        .upsert_batch(table, std::mem::take(batch), conflict_cols)
        .await?;
    *rows_upserted += n;
    Ok(())
}

/// Ingest IPEDS HD (institution directory) CSV into the `institutions` table.
///
/// # Errors
///
/// Returns `DatabaseError` variants on file read, parse, or upload failures.
pub async fn ingest_institutions(
    client: &DbClient,
    path: &Path,
    year: u16,
) -> DatabaseResult<IngestStats> {
    let content = read_file_or_zip(path)?;
    let mut reader = open_csv(&content);

    let raw_headers = reader
        .headers()
        .map_err(|e| DatabaseError::ParseError(format!("CSV header error: {e}")))?
        .clone();
    let headers = uppercase_headers(&raw_headers);

    let col_unitid = require_col(&headers, &["UNITID"], path)?;
    let col_name = require_col(&headers, &["INSTNM"], path)?;
    let cols = HdCols::for_hd(&headers);

    let mut batch: Vec<Institution> = Vec::with_capacity(UPSERT_BATCH_SIZE);
    let mut stats = IngestStats::default();

    for record in reader.records() {
        let record =
            record.map_err(|e| DatabaseError::ParseError(format!("CSV parse error: {e}")))?;
        stats.rows_read += 1;

        let Ok(unitid): Result<i32, _> = record.get(col_unitid).unwrap_or("").trim().parse() else {
            stats.rows_skipped += 1;
            continue;
        };

        let name = record.get(col_name).unwrap_or("").trim().to_string();
        if name.is_empty() {
            stats.rows_skipped += 1;
            continue;
        }

        stats.rows_filtered += 1;

        batch.push(build_institution(unitid, name, year, &cols, &record));

        if batch.len() >= UPSERT_BATCH_SIZE {
            flush_batch(
                client,
                &mut batch,
                tables::INSTITUTIONS,
                &["unitid"],
                &mut stats.rows_upserted,
            )
            .await?;
        }
    }

    flush_batch(
        client,
        &mut batch,
        tables::INSTITUTIONS,
        &["unitid"],
        &mut stats.rows_upserted,
    )
    .await?;
    Ok(stats)
}

/// Running i64 demographic sums for the institution totals cache.
#[derive(Default)]
struct DemoAccum {
    total: i64,
    total_men: i64,
    total_women: i64,
    nonresident_alien_men: i64,
    nonresident_alien_women: i64,
    hispanic_men: i64,
    hispanic_women: i64,
    american_indian_men: i64,
    american_indian_women: i64,
    asian_men: i64,
    asian_women: i64,
    black_men: i64,
    black_women: i64,
    native_hawaiian_men: i64,
    native_hawaiian_women: i64,
    white_men: i64,
    white_women: i64,
    two_or_more_men: i64,
    two_or_more_women: i64,
    unknown_race_men: i64,
    unknown_race_women: i64,
}

/// Column indices for IPEDS demographic breakdown fields.
pub(crate) struct DemoCols {
    total: Option<usize>,
    total_men: Option<usize>,
    total_women: Option<usize>,
    nonresident_alien_men: Option<usize>,
    nonresident_alien_women: Option<usize>,
    hispanic_men: Option<usize>,
    hispanic_women: Option<usize>,
    american_indian_men: Option<usize>,
    american_indian_women: Option<usize>,
    asian_men: Option<usize>,
    asian_women: Option<usize>,
    black_men: Option<usize>,
    black_women: Option<usize>,
    native_hawaiian_men: Option<usize>,
    native_hawaiian_women: Option<usize>,
    white_men: Option<usize>,
    white_women: Option<usize>,
    two_or_more_men: Option<usize>,
    two_or_more_women: Option<usize>,
    unknown_race_men: Option<usize>,
    unknown_race_women: Option<usize>,
}

impl DemoCols {
    /// Build column indices from IPEDS C (Completions) survey headers.
    pub(crate) fn for_completions(headers: &[String]) -> Self {
        macro_rules! col {
            ($($n:expr),+) => { find_col(headers, &[$($n),+]) };
        }
        Self {
            total: col!("CTOTALT"),
            total_men: col!("CTOTALM"),
            total_women: col!("CTOTALW"),
            // No CNRALT fallback: the T suffix is the men+women total, so were CNRALM
            // ever absent this field would silently receive the combined figure.
            nonresident_alien_men: col!("CNRALM"),
            nonresident_alien_women: col!("CNRALW"),
            hispanic_men: col!("CHISPM", "CHISPAM"),
            hispanic_women: col!("CHISPW", "CHISPAW"),
            american_indian_men: col!("CAIANM"),
            american_indian_women: col!("CAIANW"),
            asian_men: col!("CASIAM"),
            asian_women: col!("CASIAW"),
            black_men: col!("CBKAAM"),
            black_women: col!("CBKAAW"),
            native_hawaiian_men: col!("CNHPIM"),
            native_hawaiian_women: col!("CNHPIW"),
            white_men: col!("CWHITM"),
            white_women: col!("CWHITW"),
            two_or_more_men: col!("C2MORM"),
            two_or_more_women: col!("C2MORW"),
            unknown_race_men: col!("CUNKM", "CUNKNM"),
            unknown_race_women: col!("CUNKW", "CUNKNW"),
        }
    }
}

/// Ingest IPEDS C (completions by award level) CSV into the `completions` table.
///
/// **Every row is ingested** — all CIP codes, both `MAJORNUM` values. CIP filtering is a
/// query-time concern, so nothing is discarded here.
///
/// # Errors
///
/// Returns `DatabaseError` variants on file read, parse, or upload failures.
pub async fn ingest_completions(
    client: &DbClient,
    path: &Path,
    year: u16,
) -> DatabaseResult<IngestStats> {
    let content = read_file_or_zip(path)?;
    let mut reader = open_csv(&content);

    let raw_headers = reader
        .headers()
        .map_err(|e| DatabaseError::ParseError(format!("CSV header error: {e}")))?
        .clone();
    let headers = uppercase_headers(&raw_headers);

    macro_rules! col {
        ($($name:expr),+) => { find_col(&headers, &[$($name),+]) };
    }

    let col_unitid = require_col(&headers, &["UNITID"], path)?;
    let col_cipcode = require_col(&headers, &["CIPCODE"], path)?;
    let col_awlevel = require_col(&headers, &["AWLEVEL"], path)?;
    // Include both MAJORNUM=1 (primary) and MAJORNUM=2 (double-major) so CS completions
    // are counted even when CS is the student's second major. majornum is stored on each
    // row and included in the unique constraint, preventing duplicate conflicts.
    let col_majornum = col!("MAJORNUM");

    let demo = DemoCols::for_completions(&headers);

    // All completions stored in one table; query-time CIP filtering handles
    // CS-specific vs all-programs distinction.
    let mut batch: Vec<Completion> = Vec::with_capacity(UPSERT_BATCH_SIZE);
    // Accumulator for institution_completion_totals cache.
    // Key: (unitid, award_level). Built in one pass; written after the loop.
    let mut totals: std::collections::HashMap<(i32, i32), DemoAccum> =
        std::collections::HashMap::new();
    let mut stats = IngestStats::default();

    for record in reader.records() {
        let record =
            record.map_err(|e| DatabaseError::ParseError(format!("CSV parse error: {e}")))?;
        stats.rows_read += 1;

        let Ok(unitid): Result<i32, _> = record.get(col_unitid).unwrap_or("").trim().parse() else {
            stats.rows_skipped += 1;
            continue;
        };
        stats.rows_filtered += 1;

        let raw_cip = record.get(col_cipcode).unwrap_or("").trim().to_string();
        // Parsed raw rather than through `parse_ipeds_code`: these two are part of the
        // `completions` unique key, and IPEDS does not use sentinels in them — AWLEVEL is
        // 1..=21 and MAJORNUM is 1 or 2. Verified across all 313,566 rows of C2025_A.
        // A sentinel here would not be caught by swapping in `parse_ipeds_code` — it
        // returns `None` for "." exactly as this does. The row would land in the
        // `award_level = 0` bucket that `flush_institution_totals` writes as NULL
        // ("all levels combined"), quietly inflating that total. If one ever appears the
        // fix is to skip the row and count it in `rows_skipped`, not to change parsers.
        let award_level: Option<i32> = record.get(col_awlevel).and_then(|v| v.trim().parse().ok());
        let major_num: Option<i32> = col_majornum
            .and_then(|i| record.get(i))
            .and_then(|v| v.trim().parse().ok());

        accumulate_demo_totals(&mut totals, unitid, award_level, &demo, &record);

        batch.push(build_completion(
            unitid,
            &raw_cip,
            award_level,
            major_num,
            year,
            &demo,
            &record,
        ));

        if batch.len() >= UPSERT_BATCH_SIZE {
            flush_batch(
                client,
                &mut batch,
                tables::COMPLETIONS,
                COMPLETIONS_CONFLICT,
                &mut stats.rows_upserted,
            )
            .await?;
        }
    }

    flush_batch(
        client,
        &mut batch,
        tables::COMPLETIONS,
        COMPLETIONS_CONFLICT,
        &mut stats.rows_upserted,
    )
    .await?;

    flush_institution_totals(client, totals, year).await?;
    Ok(stats)
}

/// Accumulate demographic values from one CSV record into the institution totals map.
fn accumulate_demo_totals(
    totals: &mut std::collections::HashMap<(i32, i32), DemoAccum>,
    unitid: i32,
    award_level: Option<i32>,
    demo: &DemoCols,
    record: &csv::StringRecord,
) {
    let acc = totals
        .entry((unitid, award_level.unwrap_or(0)))
        .or_default();
    macro_rules! add {
        ($f:ident, $col:expr) => {
            if let Some(v) = $col.and_then(|i| record.get(i)).and_then(parse_ipeds_count) {
                acc.$f += i64::from(v);
            }
        };
    }
    add!(total, demo.total);
    add!(total_men, demo.total_men);
    add!(total_women, demo.total_women);
    add!(nonresident_alien_men, demo.nonresident_alien_men);
    add!(nonresident_alien_women, demo.nonresident_alien_women);
    add!(hispanic_men, demo.hispanic_men);
    add!(hispanic_women, demo.hispanic_women);
    add!(american_indian_men, demo.american_indian_men);
    add!(american_indian_women, demo.american_indian_women);
    add!(asian_men, demo.asian_men);
    add!(asian_women, demo.asian_women);
    add!(black_men, demo.black_men);
    add!(black_women, demo.black_women);
    add!(native_hawaiian_men, demo.native_hawaiian_men);
    add!(native_hawaiian_women, demo.native_hawaiian_women);
    add!(white_men, demo.white_men);
    add!(white_women, demo.white_women);
    add!(two_or_more_men, demo.two_or_more_men);
    add!(two_or_more_women, demo.two_or_more_women);
    add!(unknown_race_men, demo.unknown_race_men);
    add!(unknown_race_women, demo.unknown_race_women);
}

/// Build a [`Completion`] row from a single parsed CSV record.
pub(crate) fn build_completion(
    unitid: i32,
    raw_cip: &str,
    award_level: Option<i32>,
    major_num: Option<i32>,
    year: u16,
    demo: &DemoCols,
    record: &csv::StringRecord,
) -> Completion {
    macro_rules! get_count {
        ($col:expr) => {
            $col.and_then(|i| record.get(i)).and_then(parse_ipeds_count)
        };
    }
    Completion {
        id: None,
        unitid: Some(unitid),
        cip_code: Some(normalize_cip(raw_cip)),
        award_level,
        major_num,
        year: Some(i32::from(year)),
        total: get_count!(demo.total),
        total_men: get_count!(demo.total_men),
        total_women: get_count!(demo.total_women),
        nonresident_alien_men: get_count!(demo.nonresident_alien_men),
        nonresident_alien_women: get_count!(demo.nonresident_alien_women),
        hispanic_men: get_count!(demo.hispanic_men),
        hispanic_women: get_count!(demo.hispanic_women),
        american_indian_men: get_count!(demo.american_indian_men),
        american_indian_women: get_count!(demo.american_indian_women),
        asian_men: get_count!(demo.asian_men),
        asian_women: get_count!(demo.asian_women),
        black_men: get_count!(demo.black_men),
        black_women: get_count!(demo.black_women),
        native_hawaiian_men: get_count!(demo.native_hawaiian_men),
        native_hawaiian_women: get_count!(demo.native_hawaiian_women),
        white_men: get_count!(demo.white_men),
        white_women: get_count!(demo.white_women),
        two_or_more_men: get_count!(demo.two_or_more_men),
        two_or_more_women: get_count!(demo.two_or_more_women),
        unknown_race_men: get_count!(demo.unknown_race_men),
        unknown_race_women: get_count!(demo.unknown_race_women),
    }
}

/// Convert the in-memory totals accumulator into [`InstitutionCompletionTotal`] rows
/// and upsert them to the `institution_completion_totals` cache table.
async fn flush_institution_totals(
    client: &DbClient,
    totals: std::collections::HashMap<(i32, i32), DemoAccum>,
    year: u16,
) -> DatabaseResult<()> {
    let rows: Vec<InstitutionCompletionTotal> = totals
        .into_iter()
        .map(|((unitid, award_lv), a)| InstitutionCompletionTotal {
            id: None,
            unitid: Some(unitid),
            award_level: if award_lv == 0 { None } else { Some(award_lv) },
            year: Some(i32::from(year)),
            total: a.total.try_into().ok(),
            total_men: a.total_men.try_into().ok(),
            total_women: a.total_women.try_into().ok(),
            nonresident_alien_men: a.nonresident_alien_men.try_into().ok(),
            nonresident_alien_women: a.nonresident_alien_women.try_into().ok(),
            hispanic_men: a.hispanic_men.try_into().ok(),
            hispanic_women: a.hispanic_women.try_into().ok(),
            american_indian_men: a.american_indian_men.try_into().ok(),
            american_indian_women: a.american_indian_women.try_into().ok(),
            asian_men: a.asian_men.try_into().ok(),
            asian_women: a.asian_women.try_into().ok(),
            black_men: a.black_men.try_into().ok(),
            black_women: a.black_women.try_into().ok(),
            native_hawaiian_men: a.native_hawaiian_men.try_into().ok(),
            native_hawaiian_women: a.native_hawaiian_women.try_into().ok(),
            white_men: a.white_men.try_into().ok(),
            white_women: a.white_women.try_into().ok(),
            two_or_more_men: a.two_or_more_men.try_into().ok(),
            two_or_more_women: a.two_or_more_women.try_into().ok(),
            unknown_race_men: a.unknown_race_men.try_into().ok(),
            unknown_race_women: a.unknown_race_women.try_into().ok(),
        })
        .collect();

    for chunk in rows.chunks(UPSERT_BATCH_SIZE) {
        client
            .upsert_batch(
                tables::INSTITUTION_COMPLETION_TOTALS,
                chunk.to_vec(),
                INST_TOTALS_CONFLICT,
            )
            .await?;
    }
    Ok(())
}

/// Normalize a CIP code to standard dot notation (`"11.0101"` form).
///
/// IPEDS CSV files use dot notation (e.g. `"11.0101"`), but this function also
/// handles plain integer format (`"110101"`) for robustness. Both inputs produce
/// the same output.
pub(crate) fn normalize_cip(raw: &str) -> String {
    let digits: String = raw.chars().filter(char::is_ascii_digit).collect();
    if digits.len() == 6 {
        format!("{}.{}", &digits[..2], &digits[2..])
    } else {
        raw.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_read_file_or_zip_returns_plain_csv_unchanged() {
        // Sanity-check the non-zip branch: a file without a .zip extension is
        // read verbatim. Acts as a control for the zip-archive test below so a
        // regression that swaps the branches still surfaces here.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ipeds.csv");
        let body = "year,unitid,cip\n2024,167358,11.0101\n";
        std::fs::write(&path, body).expect("write csv");
        let read = read_file_or_zip(&path).expect("read");
        assert_eq!(read, body);
    }

    #[test]
    fn test_read_file_or_zip_extracts_first_csv_from_archive() {
        // End-to-end exercise of the zip API surface (`ZipArchive::new`,
        // `archive.len()`, `archive.by_index`, `f.name()`, `read_to_string`)
        // against the version of zip pinned in Cargo.toml. The zip 2→8 bump
        // didn't break these calls; this test pins the behaviour so a future
        // major bump that does break them fails fast.
        use std::io::Write;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ipeds.zip");
        let csv_body = "year,unitid,cip\n2024,167358,11.0101\n";
        {
            let file = std::fs::File::create(&path).expect("create zip");
            let mut zip = zip::ZipWriter::new(file);
            let options: zip::write::SimpleFileOptions = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            // A non-CSV companion entry verifies the .csv discovery loop.
            zip.start_file("README.txt", options).expect("start readme");
            zip.write_all(b"meta").expect("write readme");
            zip.start_file("data.csv", options).expect("start csv");
            zip.write_all(csv_body.as_bytes()).expect("write csv");
            zip.finish().expect("finalise zip");
        }

        let read = read_file_or_zip(&path).expect("read zip");
        assert_eq!(read, csv_body);
    }

    #[test]
    fn test_is_relevant_cip_family_11() {
        assert!(is_relevant_cip("110101"));
        assert!(is_relevant_cip("11.0101"));
        assert!(is_relevant_cip("110201"));
        assert!(is_relevant_cip("119999"));
    }

    #[test]
    fn test_is_relevant_cip_data_science() {
        assert!(is_relevant_cip("307099"));
        assert!(is_relevant_cip("30.7099"));
        assert!(is_relevant_cip("307001"));
        assert!(is_relevant_cip("30.7001"));
    }

    #[test]
    fn test_is_relevant_cip_excluded() {
        assert!(!is_relevant_cip("140101")); // Engineering
        assert!(!is_relevant_cip("270101")); // Mathematics
        assert!(!is_relevant_cip("520201")); // Business
    }

    #[test]
    fn test_normalize_cip() {
        assert_eq!(normalize_cip("110101"), "11.0101");
        assert_eq!(normalize_cip("307099"), "30.7099");
        assert_eq!(normalize_cip("11.0101"), "11.0101"); // already dot-notation
    }

    #[test]
    fn parse_ipeds_code_stores_every_code_the_lookup_tables_label() {
        // `lookup-seed.sql` gives 99 and -2 labels, so nulling them would collapse
        // "IPEDS said not classified" into "we have no value" and strand seed rows that
        // nothing could ever reference.
        assert_eq!(parse_ipeds_code("99"), Some(99)); // institution_sector 'Not classified'
        assert_eq!(parse_ipeds_code("-2"), Some(-2)); // institution_size 'Not applicable'
        assert_eq!(parse_ipeds_code("42"), Some(42));
        assert_eq!(parse_ipeds_code("0"), Some(0));

        // Only a literal "no value" is missing.
        assert_eq!(parse_ipeds_code("."), None);
        assert_eq!(parse_ipeds_code(""), None);
    }

    #[test]
    fn parse_ipeds_count_treats_99_as_a_number() {
        assert_eq!(parse_ipeds_count("99"), Some(99));
        assert_eq!(parse_ipeds_count("."), None);
        assert_eq!(parse_ipeds_count(""), None);
        assert_eq!(parse_ipeds_count("42"), Some(42));
        assert_eq!(parse_ipeds_count("0"), Some(0));
    }

    #[test]
    fn parse_ipeds_count_rejects_every_negative_not_just_minus_two() {
        // `accumulate_demo_totals` sums these into the institution totals that every
        // representation ratio divides by, so a negative shrinks a denominator rather
        // than failing. -1 and -3 are real *categorical* codes and impossible counts.
        for impossible in ["-1", "-2", "-3", "-100"] {
            assert_eq!(
                parse_ipeds_count(impossible),
                None,
                "{impossible} is not a possible number of graduates"
            );
        }
    }

    #[test]
    fn the_parsers_differ_only_over_negatives() {
        // Guards the split: drift anywhere outside the one documented difference means
        // a parser has grown a rule the other needs. Note 99 is NOT a difference — both
        // keep it, for different reasons (a labelled code; ninety-nine graduates).
        // Driven from expected values, not just parser-vs-parser: asserting only that
        // the two agree would still pass if both regressed the same way — e.g. if each
        // started nulling "99" again.
        for (input, expected) in [
            (".", None),
            ("", None),
            ("0", Some(0)),
            ("42", Some(42)),
            ("99", Some(99)),
            ("1000", Some(1000)),
            ("not-a-number", None),
        ] {
            assert_eq!(
                parse_ipeds_code(input),
                expected,
                "code parser on {input:?}"
            );
            assert_eq!(
                parse_ipeds_count(input),
                expected,
                "count parser on {input:?}"
            );
        }
        // The difference: negatives are labelled lookup codes, and impossible counts.
        for negative in ["-1", "-2", "-3"] {
            assert!(parse_ipeds_code(negative).is_some(), "{negative} is a code");
            assert_eq!(
                parse_ipeds_count(negative),
                None,
                "{negative} is not a count"
            );
        }
    }

    #[test]
    fn parse_ipeds_code_keeps_the_negative_codes_the_lookup_tables_model() {
        // `lookup-seed.sql` has real rows for -1 ("Not reported"), -2 ("Not applicable")
        // and -3 ("Not available").
        assert_eq!(parse_ipeds_code("-1"), Some(-1));
        assert_eq!(parse_ipeds_code("-2"), Some(-2));
        assert_eq!(parse_ipeds_code("-3"), Some(-3));
    }

    /// Build an in-memory zip holding the named entries.
    fn zip_with(entries: &[(&str, &str)]) -> Vec<u8> {
        use std::io::Write as _;
        let mut buf = Vec::new();
        {
            let mut w = zip::ZipWriter::new(Cursor::new(&mut buf));
            for (name, body) in entries {
                w.start_file(*name, zip::write::SimpleFileOptions::default())
                    .expect("start entry");
                w.write_all(body.as_bytes()).expect("write entry");
            }
            w.finish().expect("finish zip");
        }
        buf
    }

    #[test]
    fn importing_an_older_hd_year_is_refused() {
        // The exact situation that left 5,784 of 5,985 institutions on 2022 data.
        let refusal = downgrade_refusal(2022, Some(2025), false).expect("must refuse");
        assert!(refusal.contains("HD2022"), "{refusal}");
        assert!(
            refusal.contains("2025"),
            "must name what it would overwrite: {refusal}"
        );
        assert!(
            refusal.contains("--force"),
            "must say how to proceed: {refusal}"
        );
    }

    #[test]
    fn importing_the_same_or_a_newer_year_is_allowed() {
        // Re-importing the newest year is the ordinary repair path and must not be
        // blocked; a newer year is the whole point of the table.
        assert!(downgrade_refusal(2025, Some(2025), false).is_none());
        assert!(downgrade_refusal(2026, Some(2025), false).is_none());
    }

    #[test]
    fn an_empty_table_never_refuses() {
        // A first import has nothing to overwrite, whichever year it is.
        assert!(downgrade_refusal(2022, None, false).is_none());
    }

    #[test]
    fn force_overrides_the_refusal() {
        assert!(downgrade_refusal(2022, Some(2025), true).is_none());
    }

    #[test]
    fn a_revised_csv_wins_over_the_provisional_one() {
        // IPEDS ships both in one archive and the revised file supersedes the other.
        // Taking the first entry imported 904 superseded totals for 2022 alone.
        for entries in [
            // provisional first, as the real C2022_A.zip is ordered
            vec![
                ("c2022_a.csv", "PROVISIONAL"),
                ("c2022_a_rv.csv", "REVISED"),
            ],
            // and the other way round, so entry order is not what makes this pass
            vec![
                ("c2022_a_rv.csv", "REVISED"),
                ("c2022_a.csv", "PROVISIONAL"),
            ],
            // the 2023 archive spells it differently
            vec![
                ("C2023_a.csv", "PROVISIONAL"),
                ("C2023_a_RV.csv", "REVISED"),
            ],
        ] {
            let bytes = zip_with(&entries);
            let mut archive =
                zip::ZipArchive::new(Cursor::new(bytes.as_slice())).expect("open zip");
            let idx = pick_csv_entry(&mut archive).expect("a csv");
            let name = archive.by_index(idx).expect("entry").name().to_string();
            assert!(
                name.to_ascii_lowercase().contains("_rv"),
                "picked {name} instead of the revised file"
            );
        }
    }

    #[test]
    fn the_only_csv_is_used_when_there_is_no_revision() {
        // HD files and the newer C files ship a single entry.
        let bytes = zip_with(&[("hd2025.csv", "ONLY")]);
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes.as_slice())).expect("open zip");
        let idx = pick_csv_entry(&mut archive).expect("a csv");
        assert_eq!(archive.by_index(idx).expect("entry").name(), "hd2025.csv");
    }

    #[test]
    fn non_csv_entries_are_ignored() {
        let bytes = zip_with(&[("readme.txt", "notes"), ("data.CSV", "rows")]);
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes.as_slice())).expect("open zip");
        let idx = pick_csv_entry(&mut archive).expect("a csv");
        assert_eq!(archive.by_index(idx).expect("entry").name(), "data.CSV");

        let none = zip_with(&[("readme.txt", "notes")]);
        let mut archive = zip::ZipArchive::new(Cursor::new(none.as_slice())).expect("open zip");
        assert!(pick_csv_entry(&mut archive).is_none());
    }

    #[test]
    fn test_find_col_matches_uppercase_headers() {
        // Headers are pre-uppercased by uppercase_headers() before find_col is called
        let headers = vec!["UNITID".to_string(), "INSTNM".to_string()];
        assert_eq!(find_col(&headers, &["UNITID"]), Some(0));
        assert_eq!(find_col(&headers, &["INSTNM"]), Some(1));
        assert_eq!(find_col(&headers, &["MISSING"]), None);
    }

    #[test]
    fn test_uppercase_headers_normalizes_case() {
        use csv::StringRecord;
        let record = StringRecord::from(vec!["unitid", "InstnM", "CITY"]);
        let upper = uppercase_headers(&record);
        assert_eq!(upper, vec!["UNITID", "INSTNM", "CITY"]);
    }

    #[test]
    fn test_find_col_tries_candidates_in_order() {
        // Candidate order decides, not header order — which is why the Carnegie
        // candidates must be listed newest first. HD2022 and HD2023 really do carry
        // both of these columns, and they disagree for 1,275 of 6,256 institutions,
        // so listing C18BASIC first imported the 2018 vintage under a 2021 label.
        let headers = vec!["C21BASIC".to_string(), "C18BASIC".to_string()];
        assert_eq!(find_col(&headers, &["C18BASIC", "C21BASIC"]), Some(1));
        assert_eq!(find_col(&headers, &["C21BASIC", "C18BASIC"]), Some(0));

        // Reversing the headers must not change the answer; only candidate order does.
        let swapped = vec!["C18BASIC".to_string(), "C21BASIC".to_string()];
        assert_eq!(find_col(&swapped, &["C21BASIC", "C18BASIC"]), Some(1));
    }

    #[test]
    fn hd_carnegie_resolves_to_the_newest_vintage_present() {
        // Drives `HdCols::for_hd` itself, not a copy of its candidate list — an earlier
        // version of this test asserted against a hand-written duplicate and therefore
        // passed even with the production order reverted.
        //
        // These are the real HD2022/HD2023 headers: all four vintages at once. C18BASIC
        // is placed first so header order cannot be what makes the test pass.
        let headers: Vec<String> = ["C18BASIC", "C21BASIC", "C15BASIC", "CCBASIC"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        assert_eq!(
            HdCols::for_hd(&headers).carnegie,
            Some(1),
            "C21BASIC is at index 1; picking index 0 means the 2018 vintage won"
        );

        // HD2024/HD2025 ship only the 2021 column.
        let only_2021 = vec!["C21BASIC".to_string()];
        assert_eq!(HdCols::for_hd(&only_2021).carnegie, Some(0));

        // A year with neither must yield None rather than a wrong column.
        let neither = vec!["INSTNM".to_string(), "CARNEGIE".to_string()];
        assert_eq!(
            HdCols::for_hd(&neither).carnegie,
            None,
            "CARNEGIE uses a different code space and must not be adopted"
        );
    }

    #[test]
    fn demo_cols_resolves_each_field_to_its_own_column() {
        // The completions counterpart of the HD test. Nothing drove
        // `DemoCols::for_completions` before, so a transposed candidate — `hispanic_men:
        // col!("CHISPW")` — would have swapped two demographics across 1.2M rows with
        // nothing to notice. Real IPEDS names, deliberately not in struct order.
        let names = [
            "CUNKNW", "CTOTALT", "CWHITM", "CNRALM", "CAIANW", "CTOTALM", "C2MORW", "CHISPM",
            "CBKAAW", "CASIAM", "CNHPIW", "CTOTALW", "CNRALW", "CHISPW", "CAIANM", "CASIAW",
            "CBKAAM", "CNHPIM", "CWHITW", "C2MORM", "CUNKNM",
        ];
        let headers: Vec<String> = names.iter().map(|s| (*s).to_string()).collect();
        let c = DemoCols::for_completions(&headers);
        let at = |name: &str| Some(names.iter().position(|n| *n == name).expect("present"));

        assert_eq!(c.total, at("CTOTALT"));
        assert_eq!(c.total_men, at("CTOTALM"));
        assert_eq!(c.total_women, at("CTOTALW"));
        assert_eq!(c.nonresident_alien_men, at("CNRALM"));
        assert_eq!(c.nonresident_alien_women, at("CNRALW"));
        assert_eq!(c.hispanic_men, at("CHISPM"));
        assert_eq!(c.hispanic_women, at("CHISPW"));
        assert_eq!(c.american_indian_men, at("CAIANM"));
        assert_eq!(c.american_indian_women, at("CAIANW"));
        assert_eq!(c.asian_men, at("CASIAM"));
        assert_eq!(c.asian_women, at("CASIAW"));
        assert_eq!(c.black_men, at("CBKAAM"));
        assert_eq!(c.black_women, at("CBKAAW"));
        assert_eq!(c.native_hawaiian_men, at("CNHPIM"));
        assert_eq!(c.native_hawaiian_women, at("CNHPIW"));
        assert_eq!(c.white_men, at("CWHITM"));
        assert_eq!(c.white_women, at("CWHITW"));
        assert_eq!(c.two_or_more_men, at("C2MORM"));
        assert_eq!(c.two_or_more_women, at("C2MORW"));
        assert_eq!(c.unknown_race_men, at("CUNKNM"));
        assert_eq!(c.unknown_race_women, at("CUNKNW"));
    }

    #[test]
    fn demo_cols_never_adopts_a_total_column_for_a_gendered_field() {
        // CNRALT is men+women. If CNRALM is absent the field must stay None rather than
        // quietly receive the combined figure, which would inflate a denominator.
        let headers = vec!["CNRALT".to_string(), "CNRALW".to_string()];
        let c = DemoCols::for_completions(&headers);
        assert_eq!(c.nonresident_alien_men, None);
        assert_eq!(c.nonresident_alien_women, Some(1));
    }

    #[test]
    fn hd_cols_resolves_each_field_to_its_own_column() {
        // Guards against a transposed candidate, the HD counterpart of the completions
        // column test. Uses the real IPEDS names in a deliberately shuffled order.
        let headers: Vec<String> = [
            "INSTSIZE", "STABBR", "TRIBAL", "SECTOR", "LOCALE", "CITY", "HBCU", "ICLEVEL",
            "C21BASIC", "CONTROL",
        ]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
        let cols = HdCols::for_hd(&headers);
        assert_eq!(cols.inst_size, Some(0));
        assert_eq!(cols.state, Some(1));
        assert_eq!(cols.tribal, Some(2));
        assert_eq!(cols.sector, Some(3));
        assert_eq!(cols.locale, Some(4));
        assert_eq!(cols.city, Some(5));
        assert_eq!(cols.hbcu, Some(6));
        assert_eq!(cols.iclevel, Some(7));
        assert_eq!(cols.carnegie, Some(8));
        assert_eq!(cols.control, Some(9));
    }

    #[test]
    fn test_parse_ipeds_bool_yes() {
        assert_eq!(parse_ipeds_bool("1"), Some(true));
        assert_eq!(parse_ipeds_bool(" 1 "), Some(true)); // with whitespace
    }

    #[test]
    fn test_parse_ipeds_bool_no() {
        assert_eq!(parse_ipeds_bool("2"), Some(false));
    }

    #[test]
    fn test_parse_ipeds_bool_unknown() {
        assert_eq!(parse_ipeds_bool(""), None);
        assert_eq!(parse_ipeds_bool("0"), None);
        assert_eq!(parse_ipeds_bool("."), None);
        assert_eq!(parse_ipeds_bool("99"), None);
    }

    #[test]
    fn test_normalize_cip_already_dot_notation() {
        assert_eq!(normalize_cip("11.0101"), "11.0101");
    }

    #[test]
    fn test_normalize_cip_short_passthrough() {
        // Non-6-digit strings are returned unchanged
        assert_eq!(normalize_cip("11010"), "11010"); // 5 digits
        assert_eq!(normalize_cip("1101010"), "1101010"); // 7 digits
    }

    fn empty_demo_cols() -> DemoCols {
        DemoCols {
            total: None,
            total_men: None,
            total_women: None,
            nonresident_alien_men: None,
            nonresident_alien_women: None,
            hispanic_men: None,
            hispanic_women: None,
            american_indian_men: None,
            american_indian_women: None,
            asian_men: None,
            asian_women: None,
            black_men: None,
            black_women: None,
            native_hawaiian_men: None,
            native_hawaiian_women: None,
            white_men: None,
            white_women: None,
            two_or_more_men: None,
            two_or_more_women: None,
            unknown_race_men: None,
            unknown_race_women: None,
        }
    }

    #[test]
    fn test_build_completion_basic_fields() {
        use csv::StringRecord;
        let record = StringRecord::from(vec!["", "", ""]);
        let demo = empty_demo_cols();
        let c = build_completion(123, "11.0101", Some(5), Some(1), 2024, &demo, &record);
        assert_eq!(c.unitid, Some(123));
        assert_eq!(c.cip_code, Some("11.0101".to_string()));
        assert_eq!(c.award_level, Some(5));
        assert_eq!(c.major_num, Some(1));
        assert_eq!(c.year, Some(2024));
    }

    #[test]
    fn test_build_completion_normalises_cip() {
        use csv::StringRecord;
        let record = StringRecord::from(vec![""; 0]);
        let demo = empty_demo_cols();
        let c = build_completion(1, "110101", None, None, 2024, &demo, &record);
        assert_eq!(c.cip_code, Some("11.0101".to_string()));
    }

    #[test]
    fn build_institution_keeps_the_codes_the_lookup_tables_label() {
        use csv::StringRecord;
        // The guard in the opposite direction to the count fix: nothing else in the
        // suite fails if this call site is switched to parse_ipeds_count, which would
        // null every negative code and strand the lookup rows that label them.
        let record = StringRecord::from(vec!["99", "1", "Boston", "MA", "-2", ".", "-2"]);
        let cols = HdCols {
            sector: Some(0),
            iclevel: Some(1),
            city: Some(2),
            state: Some(3),
            inst_size: Some(4),
            locale: Some(5),
            carnegie: Some(6),
            ..HdCols::default()
        };
        let inst = build_institution(100_654, "Test College".to_string(), 2025, &cols, &record);

        assert_eq!(inst.sector, Some(99), "sector 99 is 'Not classified'");
        assert_eq!(
            inst.inst_size,
            Some(-2),
            "-2 is 'Not applicable', a labelled code — parse_ipeds_count would null it"
        );
        assert_eq!(
            inst.carnegie_class,
            Some(-2),
            "C21BASIC=-2 is 'not in the Carnegie universe' — 2,262 of 5,985 HD2025 rows, \
             the single largest effect of storing these codes rather than nulling them"
        );
        assert_eq!(inst.iclevel, Some(1), "a plain code must survive");
        assert_eq!(inst.locale, None, "\".\" is the one value meaning no code");
        assert_eq!(inst.city.as_deref(), Some("Boston"));
        assert_eq!(inst.updated_year, Some(2025));
    }

    #[test]
    fn build_institution_leaves_absent_columns_none() {
        use csv::StringRecord;
        // IPEDS drops and renames columns between years, so every HdCols field is
        // optional. A missing column must not read whatever is at index 0.
        let record = StringRecord::from(vec!["7"]);
        let inst = build_institution(1, "X".to_string(), 2024, &HdCols::default(), &record);
        assert_eq!(inst.sector, None);
        assert_eq!(inst.city, None);
        assert_eq!(inst.hbcu, None);
    }

    #[test]
    fn build_completion_maps_every_demographic_column_to_its_own_field() {
        use csv::StringRecord;
        // 21 distinct values, so a transposed get_count! pair fails loudly instead of
        // silently swapping two demographics. Every other build_completion test drives
        // only the first three columns, so the remaining 18 were never executed — and
        // the corpus is about to be re-imported on this code.
        let record = StringRecord::from((1..=21).map(|i| i.to_string()).collect::<Vec<_>>());
        let demo = DemoCols {
            total: Some(0),
            total_men: Some(1),
            total_women: Some(2),
            nonresident_alien_men: Some(3),
            nonresident_alien_women: Some(4),
            hispanic_men: Some(5),
            hispanic_women: Some(6),
            american_indian_men: Some(7),
            american_indian_women: Some(8),
            asian_men: Some(9),
            asian_women: Some(10),
            black_men: Some(11),
            black_women: Some(12),
            native_hawaiian_men: Some(13),
            native_hawaiian_women: Some(14),
            white_men: Some(15),
            white_women: Some(16),
            two_or_more_men: Some(17),
            two_or_more_women: Some(18),
            unknown_race_men: Some(19),
            unknown_race_women: Some(20),
        };
        let c = build_completion(1, "11.0101", None, None, 2024, &demo, &record);

        // Column index i holds the value i+1.
        assert_eq!(c.total, Some(1));
        assert_eq!(c.total_men, Some(2));
        assert_eq!(c.total_women, Some(3));
        assert_eq!(c.nonresident_alien_men, Some(4));
        assert_eq!(c.nonresident_alien_women, Some(5));
        assert_eq!(c.hispanic_men, Some(6));
        assert_eq!(c.hispanic_women, Some(7));
        assert_eq!(c.american_indian_men, Some(8));
        assert_eq!(c.american_indian_women, Some(9));
        assert_eq!(c.asian_men, Some(10));
        assert_eq!(c.asian_women, Some(11));
        assert_eq!(c.black_men, Some(12));
        assert_eq!(c.black_women, Some(13));
        assert_eq!(c.native_hawaiian_men, Some(14));
        assert_eq!(c.native_hawaiian_women, Some(15));
        assert_eq!(c.white_men, Some(16));
        assert_eq!(c.white_women, Some(17));
        assert_eq!(c.two_or_more_men, Some(18));
        assert_eq!(c.two_or_more_women, Some(19));
        assert_eq!(c.unknown_race_men, Some(20));
        assert_eq!(c.unknown_race_women, Some(21));
    }

    #[test]
    fn build_completion_keeps_99_and_nulls_only_the_real_sentinels() {
        use csv::StringRecord;
        // Column 0 is a real count of 99; columns 1 and 2 are genuine sentinels.
        let record = StringRecord::from(vec!["99", ".", "-2"]);
        let demo = DemoCols {
            total: Some(0),
            total_men: Some(1),
            total_women: Some(2),
            ..empty_demo_cols()
        };
        let c = build_completion(1, "11.0101", None, None, 2024, &demo, &record);
        assert_eq!(
            c.total,
            Some(99),
            "99 completions is a count, not a sentinel"
        );
        assert_eq!(c.total_men, None);
        assert_eq!(c.total_women, None);
    }

    #[test]
    fn accumulate_demo_totals_counts_a_value_of_99() {
        use csv::StringRecord;
        // The same rule one level up: institution totals are the denominator for every
        // representation ratio, so a dropped 99 skews the ratio, not just the count.
        let mut totals = std::collections::HashMap::new();
        let demo = DemoCols {
            total: Some(0),
            ..empty_demo_cols()
        };
        accumulate_demo_totals(
            &mut totals,
            10,
            Some(5),
            &demo,
            &StringRecord::from(vec!["99"]),
        );
        assert_eq!(totals[&(10, 5)].total, 99);
    }

    #[test]
    fn test_accumulate_demo_totals_sums_across_records() {
        use csv::StringRecord;
        let mut totals = std::collections::HashMap::new();
        let demo = DemoCols {
            total_men: Some(0),
            ..empty_demo_cols()
        };
        let r1 = StringRecord::from(vec!["40"]);
        let r2 = StringRecord::from(vec!["25"]);
        accumulate_demo_totals(&mut totals, 10, Some(5), &demo, &r1);
        accumulate_demo_totals(&mut totals, 10, Some(5), &demo, &r2);
        assert_eq!(totals[&(10, 5)].total_men, 65);
    }

    #[test]
    fn test_accumulate_demo_totals_none_award_level_uses_zero_key() {
        use csv::StringRecord;
        let mut totals = std::collections::HashMap::new();
        let demo = empty_demo_cols();
        let r = StringRecord::from(vec![""; 0]);
        accumulate_demo_totals(&mut totals, 99, None, &demo, &r);
        assert!(totals.contains_key(&(99, 0)));
    }

    #[test]
    fn test_accumulate_demo_totals_sentinels_not_added() {
        use csv::StringRecord;
        let mut totals = std::collections::HashMap::new();
        let demo = DemoCols {
            total_men: Some(0),
            ..empty_demo_cols()
        };
        // "." and "-2" are genuine sentinels for a count column; "99" is not.
        for sentinel in [".", "-2", ""] {
            let record = StringRecord::from(vec![sentinel]);
            accumulate_demo_totals(&mut totals, 1, Some(5), &demo, &record);
        }
        assert_eq!(totals[&(1, 5)].total_men, 0);
    }

    // ---- CP1252 source files ------------------------------------------------

    /// `Ren\xe9 Smith` — the shape of the row that aborted the HD2022 import: a CP1252
    /// `é` (0xE9), which is not valid UTF-8 on its own.
    const CP1252_ROW: &[u8] = b"UNITID,INSTNM\n100654,Ren\xe9 Smith College\n";

    #[test]
    fn test_decode_ipeds_bytes_prefers_utf8() {
        let utf8 = "UNITID,INSTNM\n100654,René Smith College\n";
        assert_eq!(decode_ipeds_bytes(utf8.as_bytes(), "t"), utf8);
    }

    #[test]
    fn test_decode_ipeds_bytes_falls_back_to_cp1252() {
        let decoded = decode_ipeds_bytes(CP1252_ROW, "HD2022.csv");
        assert!(
            decoded.contains("René Smith College"),
            "the accented character must survive, got: {decoded}"
        );
        assert!(
            !decoded.contains('\u{fffd}'),
            "the byte must be decoded, not replaced with U+FFFD: {decoded}"
        );
    }

    #[test]
    fn test_decode_ipeds_bytes_maps_the_cp1252_specific_range() {
        // 0x80..=0x9F is where CP1252 and Latin-1 disagree. Decoding these as Latin-1
        // would yield C1 control codes instead of the punctuation that appears in
        // institution names.
        let cases: &[(&[u8], char)] = &[
            (b"\x91", '\u{2018}'), // left single quote
            (b"\x92", '\u{2019}'), // right single quote — apostrophes in names
            (b"\x93", '\u{201C}'), // left double quote
            (b"\x97", '\u{2014}'), // em dash
            (b"\x80", '\u{20AC}'), // euro sign
        ];
        for (bytes, expected) in cases {
            let decoded = decode_ipeds_bytes(bytes, "t");
            assert_eq!(
                decoded.chars().next(),
                Some(*expected),
                "CP1252 {bytes:02x?} must decode to {expected:?}, not a control code"
            );
        }
        // And the upper half agrees with Latin-1, so those must be unchanged.
        assert_eq!(decode_ipeds_bytes(b"\xe9", "t"), "é");
        assert_eq!(decode_ipeds_bytes(b"\xfc", "t"), "ü");
    }

    #[test]
    fn test_read_file_or_zip_decodes_a_cp1252_plain_csv() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("hd2022.csv");
        std::fs::write(&path, CP1252_ROW).expect("write");

        let read = read_file_or_zip(&path).expect("a CP1252 csv must be readable");
        assert!(read.contains("René Smith College"), "got: {read}");
    }

    #[test]
    fn test_read_file_or_zip_decodes_a_cp1252_zip_entry() {
        // The reported failure: a strict UTF-8 read aborted the whole import here.
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("HD2022.zip");
        let file = std::fs::File::create(&path).expect("create zip");
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file::<_, ()>("hd2022.csv", zip::write::SimpleFileOptions::default())
            .expect("start entry");
        std::io::Write::write_all(&mut zip, CP1252_ROW).expect("write entry");
        zip.finish().expect("finish zip");

        let read = read_file_or_zip(&path).expect("a CP1252 zip entry must be readable");
        assert!(
            read.contains("René Smith College"),
            "one accented character must not abort the import, got: {read}"
        );
    }
}
