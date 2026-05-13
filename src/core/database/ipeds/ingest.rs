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
//! - `completions` table — CS-relevant rows (CIP 11.*, 30.7001, 30.7099)
//! - `institution_completions` table — totals across **all** CIP codes per institution,
//!   used as the denominator for demographic representation calculations
//!
//! ## IPEDS sentinel values
//! `"."`, `"-2"`, and `"99"` are treated as missing/inapplicable and mapped to `None`.
//!
//! ## Column name variants
//! IPEDS column names change across survey years. `find_col` (an internal
//! helper) accepts multiple
//! candidate names and matches case-insensitively, e.g. Carnegie class uses
//! `C18BASIC`, `C21BASIC`, or `C15BASIC` depending on the year.

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
    /// Rows that passed CIP or validity filters
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
fn read_file_or_zip(path: &Path) -> DatabaseResult<String> {
    let bytes = std::fs::read(path)
        .map_err(|e| DatabaseError::IngestError(format!("Cannot read {}: {e}", path.display())))?;

    if path.extension().and_then(|e| e.to_str()) == Some("zip") {
        let cursor = Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(cursor)
            .map_err(|e| DatabaseError::IngestError(format!("Cannot open zip: {e}")))?;
        let csv_index = (0..archive.len())
            .find(|&i| {
                archive.by_index(i).is_ok_and(|f| {
                    std::path::Path::new(f.name())
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
                })
            })
            .ok_or_else(|| {
                DatabaseError::IngestError(format!("No CSV entry found inside {}", path.display()))
            })?;
        let mut file = archive
            .by_index(csv_index)
            .map_err(|e| DatabaseError::IngestError(format!("Cannot read zip entry: {e}")))?;
        let mut content = String::new();
        file.read_to_string(&mut content)
            .map_err(|e| DatabaseError::IngestError(format!("Cannot decode zip entry: {e}")))?;
        Ok(content)
    } else {
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// Parse an IPEDS integer field, returning `None` for sentinel values.
///
/// Sentinels: `"."` (not applicable), `"-2"` (not reported), `"99"` (privacy-suppressed).
fn parse_ipeds_int(val: &str) -> Option<i32> {
    let v = val.trim();
    if v == "." || v == "-2" || v == "99" || v.is_empty() {
        None
    } else {
        v.parse().ok()
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
fn find_col(headers_upper: &[String], candidates: &[&str]) -> Option<usize> {
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
fn uppercase_headers(record: &csv::StringRecord) -> Vec<String> {
    record.iter().map(str::to_uppercase).collect()
}

/// Open a CSV reader from file content.
fn open_csv(content: &str) -> csv::Reader<&[u8]> {
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

    // Column index lookups — multiple candidate names per field for cross-year compatibility
    macro_rules! col {
        ($($name:expr),+) => { find_col(&headers, &[$($name),+]) };
    }

    let col_unitid = require_col(&headers, &["UNITID"], path)?;
    let col_name = require_col(&headers, &["INSTNM"], path)?;
    let col_city = col!("CITY");
    let col_state = col!("STABBR");
    let col_sector = col!("SECTOR");
    let col_control = col!("CONTROL");
    let col_iclevel = col!("ICLEVEL");
    // Carnegie classification column changed names across survey cycles
    let col_carnegie = col!("C18BASIC", "C21BASIC", "C15BASIC", "CBASIC");
    let col_hbcu = col!("HBCU");
    let col_tribal = col!("TRIBAL");
    let col_locale = col!("LOCALE");
    let col_inst_size = col!("INSTSIZE");

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

        macro_rules! get_int {
            ($col:expr) => {
                $col.and_then(|i| record.get(i)).and_then(parse_ipeds_int)
            };
        }
        macro_rules! get_str {
            ($col:expr) => {
                $col.and_then(|i| record.get(i))
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
            };
        }

        batch.push(Institution {
            unitid,
            name,
            city: get_str!(col_city),
            state: get_str!(col_state),
            sector: get_int!(col_sector),
            control: get_int!(col_control),
            iclevel: get_int!(col_iclevel),
            carnegie_class: get_int!(col_carnegie),
            hbcu: col_hbcu
                .and_then(|i| record.get(i))
                .and_then(parse_ipeds_bool),
            tribal: col_tribal
                .and_then(|i| record.get(i))
                .and_then(parse_ipeds_bool),
            locale: get_int!(col_locale),
            inst_size: get_int!(col_inst_size),
            updated_year: Some(i32::from(year)),
        });

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
struct DemoCols {
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
    fn for_completions(headers: &[String]) -> Self {
        macro_rules! col {
            ($($n:expr),+) => { find_col(headers, &[$($n),+]) };
        }
        Self {
            total: col!("CTOTALT"),
            total_men: col!("CTOTALM"),
            total_women: col!("CTOTALW"),
            nonresident_alien_men: col!("CNRALM", "CNRALT"),
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
/// Only rows whose CIP code matches [`is_relevant_cip`] are ingested.
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
            if let Some(v) = $col.and_then(|i| record.get(i)).and_then(parse_ipeds_int) {
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
fn build_completion(
    unitid: i32,
    raw_cip: &str,
    award_level: Option<i32>,
    major_num: Option<i32>,
    year: u16,
    demo: &DemoCols,
    record: &csv::StringRecord,
) -> Completion {
    macro_rules! get_int {
        ($col:expr) => {
            $col.and_then(|i| record.get(i)).and_then(parse_ipeds_int)
        };
    }
    Completion {
        id: None,
        unitid: Some(unitid),
        cip_code: Some(normalize_cip(raw_cip)),
        award_level,
        major_num,
        year: Some(i32::from(year)),
        total: get_int!(demo.total),
        total_men: get_int!(demo.total_men),
        total_women: get_int!(demo.total_women),
        nonresident_alien_men: get_int!(demo.nonresident_alien_men),
        nonresident_alien_women: get_int!(demo.nonresident_alien_women),
        hispanic_men: get_int!(demo.hispanic_men),
        hispanic_women: get_int!(demo.hispanic_women),
        american_indian_men: get_int!(demo.american_indian_men),
        american_indian_women: get_int!(demo.american_indian_women),
        asian_men: get_int!(demo.asian_men),
        asian_women: get_int!(demo.asian_women),
        black_men: get_int!(demo.black_men),
        black_women: get_int!(demo.black_women),
        native_hawaiian_men: get_int!(demo.native_hawaiian_men),
        native_hawaiian_women: get_int!(demo.native_hawaiian_women),
        white_men: get_int!(demo.white_men),
        white_women: get_int!(demo.white_women),
        two_or_more_men: get_int!(demo.two_or_more_men),
        two_or_more_women: get_int!(demo.two_or_more_women),
        unknown_race_men: get_int!(demo.unknown_race_men),
        unknown_race_women: get_int!(demo.unknown_race_women),
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
fn normalize_cip(raw: &str) -> String {
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
    fn test_parse_ipeds_int_sentinels() {
        assert_eq!(parse_ipeds_int("."), None);
        assert_eq!(parse_ipeds_int("-2"), None);
        assert_eq!(parse_ipeds_int("99"), None);
        assert_eq!(parse_ipeds_int(""), None);
        assert_eq!(parse_ipeds_int("42"), Some(42));
        assert_eq!(parse_ipeds_int("0"), Some(0));
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
        let headers = vec!["C21BASIC".to_string(), "C18BASIC".to_string()];
        assert_eq!(
            find_col(&headers, &["C18BASIC", "C21BASIC"]),
            Some(1) // C18BASIC is at index 1
        );
        assert_eq!(
            find_col(&headers, &["C21BASIC", "C18BASIC"]),
            Some(0) // C21BASIC is at index 0, wins regardless of candidate order
        );
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
    fn test_build_completion_sentinel_demo_becomes_none() {
        use csv::StringRecord;
        // Column 0 has sentinel "99" — should parse to None via parse_ipeds_int
        let record = StringRecord::from(vec!["99", ".", "-2"]);
        let demo = DemoCols {
            total: Some(0),
            total_men: Some(1),
            total_women: Some(2),
            ..empty_demo_cols()
        };
        let c = build_completion(1, "11.0101", None, None, 2024, &demo, &record);
        assert_eq!(c.total, None);
        assert_eq!(c.total_men, None);
        assert_eq!(c.total_women, None);
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
        let record = StringRecord::from(vec!["99"]); // sentinel → None → 0 added
        accumulate_demo_totals(&mut totals, 1, Some(5), &demo, &record);
        assert_eq!(totals[&(1, 5)].total_men, 0);
    }
}
