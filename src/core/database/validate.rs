//! Compare a local IPEDS file against what the backend actually stores.
//!
//! `db doctor` answers "is this deployment set up correctly". This answers the next
//! question — "is the data in it *right*" — by reading the survey file the import came
//! from and diffing it column by column.
//!
//! ## Two checks, because one of them cannot catch importer bugs
//!
//! **Fidelity** parses the file with the importer's own code and diffs the result against
//! the stored rows. That catches a stale year, a partial import, a row that never landed,
//! and a value that changed between survey years. It **cannot** catch a parsing defect:
//! both sides would share it, so a mis-parsed column agrees with itself and reports clean.
//!
//! **Provenance** closes that gap for the columns where it has bitten. Several IPEDS
//! measures ship under more than one column name in the same file — HD2022 carries four
//! vintages of the Carnegie classification — and the importer picks by candidate order.
//! Pick the wrong one and every count still matches; only the *meaning* is wrong. So for
//! those columns this reports which source column the stored data actually agrees with,
//! and flags it when that is not the one the importer intends to read.
//!
//! The Carnegie column was imported from the 2018 vintage under a 2021 label for two
//! survey years before anyone noticed, which is why this check exists.

use std::collections::BTreeMap;

use std::path::Path;

use super::client::DbClient;
use super::error::{DatabaseError, DatabaseResult};
use super::ipeds::ingest::{
    build_institution, find_col, open_csv, parse_ipeds_code, read_file_or_zip, uppercase_headers,
    HdCols, HD_CARNEGIE_CANDIDATES,
};
use super::models::Institution;
use super::query::QueryFilters;
use super::tables;

/// How a single column compared between file and backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnDiff {
    /// Column name as the database knows it.
    pub column: &'static str,
    /// Rows where both sides had a row to compare.
    pub compared: usize,
    /// Rows whose values disagreed.
    pub mismatched: usize,
    /// A few disagreements, for the report. Never the whole list.
    pub examples: Vec<Mismatch>,
}

impl ColumnDiff {
    /// Share of compared rows that disagreed, as a percentage.
    #[must_use]
    pub fn mismatch_rate(&self) -> f64 {
        if self.compared == 0 {
            return 0.0;
        }
        #[allow(clippy::cast_precision_loss)]
        {
            self.mismatched as f64 * 100.0 / self.compared as f64
        }
    }
}

/// One disagreement, identified well enough to look up by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mismatch {
    /// Primary key of the row — `UNITID` for institutions.
    pub key: String,
    /// What the survey file says.
    pub in_file: String,
    /// What the backend stores.
    pub in_db: String,
}

/// How many examples to keep per column.
///
/// Enough to see a pattern — a systematic off-by-one vintage looks different from three
/// scattered typos — without turning the report into a data dump.
const MAX_EXAMPLES: usize = 3;

/// Rows present on one side only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Coverage {
    /// Rows in the file that the backend does not have.
    pub missing_from_db: usize,
    /// Rows in the backend that this file does not mention.
    ///
    /// Not a fault on its own: the `institutions` table accumulates across survey years,
    /// so an institution that closed before this file's year is expected to remain.
    pub absent_from_file: usize,
    /// Rows compared on both sides.
    pub in_both: usize,
}

/// Result of diffing one survey file against the backend.
#[derive(Debug, Clone)]
pub struct FidelityReport {
    /// Row-level presence.
    pub coverage: Coverage,
    /// Per-column disagreement, worst first.
    pub columns: Vec<ColumnDiff>,
}

impl FidelityReport {
    /// Columns with at least one disagreement.
    #[must_use]
    pub fn failing_columns(&self) -> usize {
        self.columns.iter().filter(|c| c.mismatched > 0).count()
    }

    /// Total disagreements across every column.
    #[must_use]
    pub fn total_mismatches(&self) -> usize {
        self.columns.iter().map(|c| c.mismatched).sum()
    }
}

/// Render an optional value the way the report shows it.
fn show<T: std::fmt::Display>(v: Option<&T>) -> String {
    v.map_or_else(|| "NULL".to_string(), ToString::to_string)
}

/// Compare institutions parsed from a survey file against the stored rows.
///
/// Keyed on `unitid`. Rows present on only one side are counted in [`Coverage`] and not
/// compared — an institution the file does not mention says nothing about the file.
///
/// `updated_year` is deliberately **not** compared: it records which import wrote the row,
/// not anything the survey file asserts.
#[must_use]
pub fn diff_institutions(
    from_file: &BTreeMap<i32, Institution>,
    from_db: &BTreeMap<i32, Institution>,
) -> FidelityReport {
    let mut coverage = Coverage::default();
    for unitid in from_file.keys() {
        if from_db.contains_key(unitid) {
            coverage.in_both += 1;
        } else {
            coverage.missing_from_db += 1;
        }
    }
    coverage.absent_from_file = from_db
        .keys()
        .filter(|u| !from_file.contains_key(u))
        .count();

    // Each entry pulls one field from both sides and renders it, so adding a column to
    // `Institution` means adding one line here rather than a new comparison loop.
    let mut columns = vec![
        compare(from_file, from_db, "name", |i| Some(i.name.clone())),
        compare(from_file, from_db, "city", |i| i.city.clone()),
        compare(from_file, from_db, "state", |i| i.state.clone()),
        compare(from_file, from_db, "sector", |i| i.sector),
        compare(from_file, from_db, "control", |i| i.control),
        compare(from_file, from_db, "iclevel", |i| i.iclevel),
        compare(from_file, from_db, "carnegie_class", |i| i.carnegie_class),
        compare(from_file, from_db, "hbcu", |i| i.hbcu),
        compare(from_file, from_db, "tribal", |i| i.tribal),
        compare(from_file, from_db, "locale", |i| i.locale),
        compare(from_file, from_db, "inst_size", |i| i.inst_size),
    ];
    columns.sort_by(|a, b| b.mismatched.cmp(&a.mismatched).then(a.column.cmp(b.column)));

    FidelityReport { coverage, columns }
}

/// Compare one field across every institution present on both sides.
fn compare<T, F>(
    from_file: &BTreeMap<i32, Institution>,
    from_db: &BTreeMap<i32, Institution>,
    column: &'static str,
    field: F,
) -> ColumnDiff
where
    T: PartialEq + std::fmt::Display,
    F: Fn(&Institution) -> Option<T>,
{
    let mut diff = ColumnDiff {
        column,
        compared: 0,
        mismatched: 0,
        examples: Vec::new(),
    };
    // `BTreeMap` iterates in key order, so the examples are the lowest unitids on every
    // run. With a `HashMap` the report would differ between runs over identical inputs,
    // which makes diffing two validate runs meaningless.
    for unitid in from_file.keys() {
        let (Some(file_row), Some(db_row)) = (from_file.get(unitid), from_db.get(unitid)) else {
            continue;
        };
        diff.compared += 1;
        let (in_file, in_db) = (field(file_row), field(db_row));
        if in_file != in_db {
            diff.mismatched += 1;
            if diff.examples.len() < MAX_EXAMPLES {
                diff.examples.push(Mismatch {
                    key: unitid.to_string(),
                    in_file: show(in_file.as_ref()),
                    in_db: show(in_db.as_ref()),
                });
            }
        }
    }
    diff
}

// ============================================================================
// Provenance
// ============================================================================

/// How well the stored column agrees with one candidate source column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateMatch {
    /// Source column name as it appears in the survey file, e.g. `C21BASIC`.
    pub source_column: String,
    /// Rows where the stored value equals this column's value.
    pub agreements: usize,
    /// Rows compared.
    pub compared: usize,
}

impl CandidateMatch {
    /// Whether every compared row agrees.
    #[must_use]
    pub const fn is_exact(&self) -> bool {
        self.agreements == self.compared
    }
}

/// Which source column the stored data actually came from.
#[derive(Debug, Clone)]
pub struct ProvenanceReport {
    /// Database column under examination.
    pub column: &'static str,
    /// The column the importer intends to read — the first candidate present in the file.
    pub expected: Option<String>,
    /// Every candidate present in this file, best agreement first.
    pub candidates: Vec<CandidateMatch>,
}

impl ProvenanceReport {
    /// The candidate the stored data agrees with exactly, if exactly one does.
    ///
    /// `None` when no candidate matches every row, or when several do — which happens
    /// when the vintages are identical in this file and the check simply cannot tell
    /// them apart. Reporting "cannot tell" is the point; guessing would be worse.
    #[must_use]
    pub fn sole_exact_match(&self) -> Option<&CandidateMatch> {
        let mut exact = self.candidates.iter().filter(|c| c.is_exact());
        let first = exact.next()?;
        if exact.next().is_some() {
            return None;
        }
        Some(first)
    }

    /// `true` when the data provably came from a column the importer did not intend.
    ///
    /// Requires a sole exact match that differs from [`Self::expected`]. Anything less
    /// certain is not reported as wrong.
    #[must_use]
    pub fn is_wrong_source(&self) -> bool {
        match (self.sole_exact_match(), self.expected.as_deref()) {
            (Some(actual), Some(expected)) => actual.source_column != expected,
            _ => false,
        }
    }

    /// The candidate the stored data agrees with most, when that beats the expected one.
    ///
    /// Weaker evidence than [`Self::sole_exact_match`] and reported differently: another
    /// discrepancy — a column the import nulled, a year of drift — can stop *any*
    /// candidate matching exactly while still leaving a clear signal about which one the
    /// data came from. Returns `(best, expected)` only when `best` strictly leads, so a
    /// tie says nothing.
    #[must_use]
    pub fn best_beats_expected(&self) -> Option<(&CandidateMatch, &CandidateMatch)> {
        let expected_name = self.expected.as_deref()?;
        let best = self.candidates.first()?;
        let expected = self
            .candidates
            .iter()
            .find(|c| c.source_column == expected_name)?;
        (best.source_column != expected.source_column && best.agreements > expected.agreements)
            .then_some((best, expected))
    }
}

/// Work out which candidate column the stored values actually match.
///
/// `stored` maps `unitid` to the value in the database. `candidates` are the source
/// column names in the importer's own priority order, each with that column's values by
/// `unitid`. Candidates absent from the file are simply not passed in.
///
/// This is the check that a row-count comparison cannot make: pick the wrong vintage of a
/// column and every count still ties, because the number of rows is unchanged.
#[must_use]
pub fn diff_provenance(
    column: &'static str,
    stored: &BTreeMap<i32, Option<i32>>,
    candidates: &[CandidateValues],
) -> ProvenanceReport {
    let expected = candidates.first().map(|(name, _)| name.clone());

    let mut matches: Vec<CandidateMatch> = candidates
        .iter()
        .map(|(name, values)| {
            let mut m = CandidateMatch {
                source_column: name.clone(),
                agreements: 0,
                compared: 0,
            };
            for (unitid, stored_value) in stored {
                let Some(file_value) = values.get(unitid) else {
                    continue;
                };
                m.compared += 1;
                if file_value == stored_value {
                    m.agreements += 1;
                }
            }
            m
        })
        .collect();
    matches.sort_by(|a, b| {
        b.agreements
            .cmp(&a.agreements)
            .then(a.source_column.cmp(&b.source_column))
    });

    ProvenanceReport {
        column,
        expected,
        candidates: matches,
    }
}

// ============================================================================
// Reading the survey file and the backend
// ============================================================================

/// Which IPEDS survey a file holds, decided from its header row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurveyKind {
    /// HD — the institution directory.
    Institutions,
    /// `C_A` — completions by award level.
    Completions,
}

impl SurveyKind {
    /// Identify a survey from its uppercased header names.
    ///
    /// Keys off columns unique to each survey rather than the file name, so a renamed
    /// download still works and a wrong `--year` cannot be masked by a plausible name.
    #[must_use]
    pub fn from_headers(headers: &[String]) -> Option<Self> {
        let has = |name: &str| headers.iter().any(|h| h == name);
        if has("CIPCODE") && has("AWLEVEL") {
            Some(Self::Completions)
        } else if has("INSTNM") {
            Some(Self::Institutions)
        } else {
            None
        }
    }

    /// Human-readable name for the report.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Institutions => "HD (institution directory)",
            Self::Completions => "C_A (completions by award level)",
        }
    }
}

/// Parse an HD survey file into institutions, keyed by `unitid`.
///
/// Uses the importer's own [`build_institution`], so the comparison answers "does the
/// backend hold what importing this file would produce". It therefore cannot detect a
/// parsing defect — [`diff_provenance`] covers the case where that has actually bitten.
///
/// # Errors
/// [`DatabaseError::ParseError`] if the file cannot be read or has no usable header.
pub fn read_institutions(path: &Path, year: u16) -> DatabaseResult<BTreeMap<i32, Institution>> {
    let content = read_file_or_zip(path)?;
    let mut reader = open_csv(&content);
    let raw = reader
        .headers()
        .map_err(|e| DatabaseError::ParseError(format!("CSV header error: {e}")))?
        .clone();
    let headers = uppercase_headers(&raw);

    let col_unitid = find_col(&headers, &["UNITID"]).ok_or_else(|| {
        DatabaseError::ParseError(format!("{} has no UNITID column", path.display()))
    })?;
    let col_name = find_col(&headers, &["INSTNM"]).ok_or_else(|| {
        DatabaseError::ParseError(format!("{} has no INSTNM column", path.display()))
    })?;
    let cols = HdCols::for_hd(&headers);

    let mut out = BTreeMap::new();
    for record in reader.records() {
        let record =
            record.map_err(|e| DatabaseError::ParseError(format!("CSV parse error: {e}")))?;
        let Ok(unitid) = record.get(col_unitid).unwrap_or("").trim().parse::<i32>() else {
            continue;
        };
        let name = record.get(col_name).unwrap_or("").trim().to_string();
        if name.is_empty() {
            continue;
        }
        out.insert(
            unitid,
            build_institution(unitid, name, year, &cols, &record),
        );
    }
    Ok(out)
}

/// One candidate source column and the value it holds for each `unitid`.
pub type CandidateValues = (String, BTreeMap<i32, Option<i32>>);

/// Read every candidate Carnegie column present in an HD file, in importer priority order.
///
/// Returns `(column name, values by unitid)` for each candidate the file actually
/// carries, so [`diff_provenance`] can report which one the backend agrees with.
///
/// # Errors
/// [`DatabaseError::ParseError`] if the file cannot be read or parsed.
pub fn read_carnegie_candidates(path: &Path) -> DatabaseResult<Vec<CandidateValues>> {
    let content = read_file_or_zip(path)?;
    let mut reader = open_csv(&content);
    let raw = reader
        .headers()
        .map_err(|e| DatabaseError::ParseError(format!("CSV header error: {e}")))?
        .clone();
    let headers = uppercase_headers(&raw);

    let col_unitid = find_col(&headers, &["UNITID"]).ok_or_else(|| {
        DatabaseError::ParseError(format!("{} has no UNITID column", path.display()))
    })?;
    // Same constant the importer selects from, not a copy of it.
    let present: Vec<(String, usize)> = HD_CARNEGIE_CANDIDATES
        .iter()
        .filter_map(|name| find_col(&headers, &[name]).map(|idx| ((*name).to_string(), idx)))
        .collect();
    if present.is_empty() {
        return Ok(Vec::new());
    }

    let mut out: Vec<(String, BTreeMap<i32, Option<i32>>)> = present
        .iter()
        .map(|(name, _)| (name.clone(), BTreeMap::new()))
        .collect();
    for record in reader.records() {
        let record =
            record.map_err(|e| DatabaseError::ParseError(format!("CSV parse error: {e}")))?;
        let Ok(unitid) = record.get(col_unitid).unwrap_or("").trim().parse::<i32>() else {
            continue;
        };
        for (slot, (_, idx)) in out.iter_mut().zip(present.iter()) {
            let value = record.get(*idx).and_then(parse_ipeds_code);
            slot.1.insert(unitid, value);
        }
    }
    Ok(out)
}

/// Fetch every stored institution, keyed by `unitid`.
///
/// # Errors
/// Whatever [`DbClient::select`] reports.
pub async fn fetch_institutions(client: &DbClient) -> DatabaseResult<BTreeMap<i32, Institution>> {
    let filters = QueryFilters::new();
    let body = client
        .select(tables::INSTITUTIONS, "*", &filters, Some(FETCH_LIMIT))
        .await?;
    let rows: Vec<Institution> = serde_json::from_value(body)
        .map_err(|e| DatabaseError::ParseError(format!("cannot read stored institutions: {e}")))?;
    Ok(rows.into_iter().map(|i| (i.unitid, i)).collect())
}

/// Upper bound on rows fetched for a comparison.
///
/// `institutions` is ~6.5k rows, so one request covers it. A backend capping responses
/// below this would make the comparison wrong rather than slow — which is exactly what
/// `db doctor`'s row-limit check is for, and why `db validate` reports coverage counts
/// instead of assuming it saw everything.
const FETCH_LIMIT: usize = 50_000;

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn inst(unitid: i32) -> Institution {
        Institution {
            unitid,
            name: format!("College {unitid}"),
            city: Some("Boston".into()),
            state: Some("MA".into()),
            sector: Some(1),
            control: Some(1),
            iclevel: Some(1),
            carnegie_class: Some(15),
            hbcu: Some(false),
            tribal: Some(false),
            locale: Some(11),
            inst_size: Some(3),
            updated_year: Some(2025),
        }
    }

    fn map(rows: Vec<Institution>) -> BTreeMap<i32, Institution> {
        rows.into_iter().map(|i| (i.unitid, i)).collect()
    }

    #[test]
    fn identical_sides_report_no_mismatches() {
        let file = map(vec![inst(1), inst(2)]);
        let db = map(vec![inst(1), inst(2)]);
        let report = diff_institutions(&file, &db);

        assert_eq!(report.total_mismatches(), 0);
        assert_eq!(report.failing_columns(), 0);
        assert_eq!(report.coverage.in_both, 2);
        assert_eq!(report.coverage.missing_from_db, 0);
        assert_eq!(report.coverage.absent_from_file, 0);
    }

    #[test]
    fn a_differing_column_is_counted_with_both_values_shown() {
        let file = map(vec![inst(1)]);
        let mut stale = inst(1);
        stale.carnegie_class = Some(17);
        let db = map(vec![stale]);

        let report = diff_institutions(&file, &db);
        let carnegie = report
            .columns
            .iter()
            .find(|c| c.column == "carnegie_class")
            .expect("column present");

        assert_eq!(carnegie.mismatched, 1);
        assert_eq!(carnegie.compared, 1);
        assert_eq!(
            carnegie.examples[0],
            Mismatch {
                key: "1".into(),
                in_file: "15".into(),
                in_db: "17".into(),
            },
            "a report that does not show both values cannot be acted on"
        );
        assert_eq!(report.total_mismatches(), 1, "only that one column differs");
    }

    #[test]
    fn a_null_on_either_side_is_a_mismatch_and_reads_as_null() {
        // The shape of the `99`-nulling defect: the file has a value, the backend has
        // NULL. It must not be silently skipped as "nothing to compare".
        let file = map(vec![inst(1)]);
        let mut nulled = inst(1);
        nulled.inst_size = None;
        let db = map(vec![nulled]);

        let diff = diff_institutions(&file, &db)
            .columns
            .into_iter()
            .find(|c| c.column == "inst_size")
            .expect("column present");
        assert_eq!(diff.mismatched, 1);
        assert_eq!(diff.examples[0].in_db, "NULL");
        assert_eq!(diff.examples[0].in_file, "3");
    }

    #[test]
    fn rows_on_only_one_side_are_counted_not_compared() {
        // An institution the backend holds but this file does not mention is expected —
        // `institutions` accumulates across survey years — so it must not inflate the
        // mismatch count.
        let file = map(vec![inst(1), inst(2)]);
        let db = map(vec![inst(2), inst(3)]);
        let report = diff_institutions(&file, &db);

        assert_eq!(report.coverage.in_both, 1);
        assert_eq!(report.coverage.missing_from_db, 1, "unitid 1");
        assert_eq!(report.coverage.absent_from_file, 1, "unitid 3");
        assert_eq!(report.total_mismatches(), 0);
        for c in &report.columns {
            assert_eq!(c.compared, 1, "{} compared the wrong row count", c.column);
        }
    }

    #[test]
    fn updated_year_is_not_compared() {
        // It records which import wrote the row, not anything the survey file claims, so
        // diffing it would report every row of an older import as wrong.
        let file = map(vec![inst(1)]);
        let mut older = inst(1);
        older.updated_year = Some(2022);
        let db = map(vec![older]);

        let report = diff_institutions(&file, &db);
        assert_eq!(report.total_mismatches(), 0);
        assert!(!report.columns.iter().any(|c| c.column == "updated_year"));
    }

    #[test]
    fn columns_are_ordered_worst_first_and_examples_are_capped() {
        let file: BTreeMap<i32, Institution> = map((1..=10).map(inst).collect());
        let db: BTreeMap<i32, Institution> = map((1..=10)
            .map(|i| {
                let mut row = inst(i);
                row.locale = Some(99); // every row differs
                if i <= 2 {
                    row.sector = Some(9); // only two differ
                }
                row
            })
            .collect());

        let report = diff_institutions(&file, &db);
        assert_eq!(
            report.columns[0].column, "locale",
            "worst column comes first"
        );
        assert_eq!(report.columns[0].mismatched, 10);
        assert_eq!(report.columns[1].column, "sector");
        assert_eq!(report.columns[1].mismatched, 2);
        assert_eq!(
            report.columns[0].examples.len(),
            MAX_EXAMPLES,
            "a report is not a data dump"
        );
        assert!((report.columns[0].mismatch_rate() - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn examples_are_stable_across_runs() {
        // Two runs over the same inputs must produce the same report, or a diff between
        // two validate runs is meaningless.
        let file: BTreeMap<i32, Institution> = map((1..=50).map(inst).collect());
        let db: BTreeMap<i32, Institution> = map((1..=50)
            .map(|i| {
                let mut row = inst(i);
                row.city = Some("Elsewhere".into());
                row
            })
            .collect());

        let first = diff_institutions(&file, &db);
        let second = diff_institutions(&file, &db);
        assert_eq!(first.columns, second.columns);
        let city = first.columns.iter().find(|c| c.column == "city").unwrap();
        assert_eq!(
            city.examples
                .iter()
                .map(|m| m.key.as_str())
                .collect::<Vec<_>>(),
            ["1", "2", "3"],
            "examples must be the lowest unitids, deterministically"
        );
    }

    // --- provenance ---------------------------------------------------------

    fn values(pairs: &[(i32, i32)]) -> BTreeMap<i32, Option<i32>> {
        pairs.iter().map(|(k, v)| (*k, Some(*v))).collect()
    }

    #[test]
    fn provenance_names_the_column_the_data_actually_came_from() {
        // The Carnegie defect in miniature: the importer intends C21BASIC, the stored
        // values agree with C18BASIC exactly, and every row count ties either way.
        let stored = values(&[(1, 10), (2, 18), (3, 22)]);
        let candidates = vec![
            ("C21BASIC".to_string(), values(&[(1, 11), (2, 17), (3, 20)])),
            ("C18BASIC".to_string(), values(&[(1, 10), (2, 18), (3, 22)])),
        ];

        let report = diff_provenance("carnegie_class", &stored, &candidates);
        assert_eq!(report.expected.as_deref(), Some("C21BASIC"));
        assert_eq!(
            report.sole_exact_match().map(|m| m.source_column.as_str()),
            Some("C18BASIC")
        );
        assert!(
            report.is_wrong_source(),
            "data from a column the importer does not read must be flagged"
        );
    }

    #[test]
    fn provenance_is_quiet_when_the_expected_column_matches() {
        let stored = values(&[(1, 11), (2, 17)]);
        let candidates = vec![
            ("C21BASIC".to_string(), values(&[(1, 11), (2, 17)])),
            ("C18BASIC".to_string(), values(&[(1, 10), (2, 18)])),
        ];
        let report = diff_provenance("carnegie_class", &stored, &candidates);

        assert!(!report.is_wrong_source());
        assert_eq!(
            report.sole_exact_match().map(|m| m.source_column.as_str()),
            Some("C21BASIC")
        );
    }

    #[test]
    fn provenance_declines_to_guess_when_two_candidates_are_identical() {
        // If the vintages agree in this file there is no evidence either way, and
        // claiming the data came from one of them would be invention.
        let stored = values(&[(1, 11), (2, 17)]);
        let candidates = vec![
            ("C21BASIC".to_string(), values(&[(1, 11), (2, 17)])),
            ("C18BASIC".to_string(), values(&[(1, 11), (2, 17)])),
        ];
        let report = diff_provenance("carnegie_class", &stored, &candidates);

        assert!(report.sole_exact_match().is_none(), "two exact matches");
        assert!(
            !report.is_wrong_source(),
            "indistinguishable is not the same as wrong"
        );
    }

    #[test]
    fn provenance_reports_the_leading_candidate_when_none_is_exact() {
        // The live situation: a second discrepancy (values the old importer nulled)
        // stops any candidate matching exactly, but one still clearly leads. Withholding
        // that would hide the only evidence there is.
        let stored = values(&[(1, 10), (2, 18), (3, 22)]);
        let mut with_a_hole = values(&[(1, 10), (2, 18)]);
        with_a_hole.insert(3, None); // the row the old parser nulled
        let candidates = vec![
            ("C21BASIC".to_string(), values(&[(1, 11), (2, 17), (3, 20)])),
            ("C18BASIC".to_string(), with_a_hole),
        ];
        let report = diff_provenance("carnegie_class", &stored, &candidates);

        assert!(report.sole_exact_match().is_none(), "nothing is exact");
        assert!(!report.is_wrong_source(), "not proven, so not asserted");
        let (best, expected) = report
            .best_beats_expected()
            .expect("C18BASIC leads C21BASIC");
        assert_eq!(best.source_column, "C18BASIC");
        assert_eq!(best.agreements, 2);
        assert_eq!(expected.source_column, "C21BASIC");
        assert_eq!(expected.agreements, 0);
    }

    #[test]
    fn provenance_says_nothing_when_the_expected_column_already_leads() {
        let stored = values(&[(1, 11), (2, 17)]);
        let candidates = vec![
            ("C21BASIC".to_string(), values(&[(1, 11), (2, 99)])),
            ("C18BASIC".to_string(), values(&[(1, 10), (2, 18)])),
        ];
        let report = diff_provenance("carnegie_class", &stored, &candidates);
        assert!(
            report.best_beats_expected().is_none(),
            "the expected column leading is the healthy case, not a finding"
        );
    }

    #[test]
    fn provenance_says_nothing_on_a_tie() {
        // Equal agreement is no evidence either way.
        let stored = values(&[(1, 1), (2, 2)]);
        let candidates = vec![
            ("C21BASIC".to_string(), values(&[(1, 1), (2, 9)])),
            ("C18BASIC".to_string(), values(&[(1, 9), (2, 2)])),
        ];
        let report = diff_provenance("carnegie_class", &stored, &candidates);
        assert!(report.best_beats_expected().is_none());
    }

    // --- survey identification ----------------------------------------------

    #[test]
    fn survey_kind_is_read_from_columns_not_the_file_name() {
        let hd = ["UNITID", "INSTNM", "CITY"].map(str::to_string).to_vec();
        assert_eq!(
            SurveyKind::from_headers(&hd),
            Some(SurveyKind::Institutions)
        );

        let c = ["UNITID", "CIPCODE", "AWLEVEL", "CTOTALT"]
            .map(str::to_string)
            .to_vec();
        assert_eq!(SurveyKind::from_headers(&c), Some(SurveyKind::Completions));

        // Completions also carries UNITID, so the check must not stop at that.
        let ambiguous = ["UNITID", "INSTNM", "CIPCODE", "AWLEVEL"]
            .map(str::to_string)
            .to_vec();
        assert_eq!(
            SurveyKind::from_headers(&ambiguous),
            Some(SurveyKind::Completions),
            "CIPCODE+AWLEVEL is the more specific signal"
        );

        let neither = ["FOO", "BAR"].map(str::to_string).to_vec();
        assert_eq!(SurveyKind::from_headers(&neither), None);
    }

    #[test]
    fn provenance_declines_to_guess_when_nothing_matches_exactly() {
        // Stale data matches no candidate. That is a fidelity problem, which the other
        // half of this module reports; provenance must not claim a wrong source.
        let stored = values(&[(1, 5), (2, 5)]);
        let candidates = vec![
            ("C21BASIC".to_string(), values(&[(1, 11), (2, 17)])),
            ("C18BASIC".to_string(), values(&[(1, 10), (2, 18)])),
        ];
        let report = diff_provenance("carnegie_class", &stored, &candidates);

        assert!(report.sole_exact_match().is_none());
        assert!(!report.is_wrong_source());
        assert_eq!(report.candidates[0].agreements, 0);
    }
}
