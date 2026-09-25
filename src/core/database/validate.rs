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

use std::collections::{BTreeMap, BTreeSet};

use std::path::Path;

use super::client::DbClient;
use super::error::{DatabaseError, DatabaseResult};
use super::filters::QueryFilters;
use super::ipeds::ingest::{
    build_completion, build_institution, find_col, open_csv, parse_ipeds_code, read_file_or_zip,
    uppercase_headers, DemoCols, HdCols, HD_CARNEGIE_CANDIDATES,
};
use super::models::{Completion, Institution};
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
    let coverage = coverage_of(from_file, from_db);

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

/// Compare one field across every row present on both sides.
///
/// Generic over the key and row types so institutions and completions share one
/// comparison rather than two that can drift apart.
fn compare<K, R, T, F>(
    from_file: &BTreeMap<K, R>,
    from_db: &BTreeMap<K, R>,
    column: &'static str,
    field: F,
) -> ColumnDiff
where
    K: Ord + std::fmt::Display,
    T: PartialEq + std::fmt::Display,
    F: Fn(&R) -> Option<T>,
{
    let mut diff = ColumnDiff {
        column,
        compared: 0,
        mismatched: 0,
        examples: Vec::new(),
    };
    // `BTreeMap` iterates in key order, so the examples are the same rows on every run.
    // With a `HashMap` the report would differ between runs over identical inputs, which
    // makes diffing two validate runs meaningless.
    for (key, file_row) in from_file {
        let Some(db_row) = from_db.get(key) else {
            continue;
        };
        diff.compared += 1;
        let (in_file, in_db) = (field(file_row), field(db_row));
        if in_file != in_db {
            diff.mismatched += 1;
            if diff.examples.len() < MAX_EXAMPLES {
                diff.examples.push(Mismatch {
                    key: key.to_string(),
                    in_file: show(in_file.as_ref()),
                    in_db: show(in_db.as_ref()),
                });
            }
        }
    }
    diff
}

/// Count rows present on one side only.
fn coverage_of<K: Ord, R>(from_file: &BTreeMap<K, R>, from_db: &BTreeMap<K, R>) -> Coverage {
    let in_both = from_file.keys().filter(|k| from_db.contains_key(k)).count();
    Coverage {
        in_both,
        missing_from_db: from_file.len() - in_both,
        absent_from_file: from_db
            .keys()
            .filter(|k| !from_file.contains_key(k))
            .count(),
    }
}

/// What a validation run concluded.
///
/// Separated from printing so the conclusion is testable. It was not, and three faults
/// hid in the formatting: a partial import printed "every column matches" and exited 0
/// because the verdict only looked at column mismatches; a run that compared **nothing**
/// reported clean for the same reason; and a failure caused by a row-count gap printed
/// "0 column(s) disagree" while exiting 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Everything checked agreed, and something was actually checked.
    Clean,
    /// Nothing could be concluded — no rows compared, or the read was truncated.
    ///
    /// Distinct from `Clean` on purpose: "I checked and found nothing wrong" and "I could
    /// not check" are different answers, and only one of them justifies a re-import.
    Inconclusive(Vec<String>),
    /// At least one real disagreement, each named.
    Failed(Vec<String>),
}

impl Verdict {
    /// Process exit code: 0 only for [`Verdict::Clean`].
    #[must_use]
    pub const fn exit_code(&self) -> i32 {
        match self {
            Self::Clean => 0,
            _ => 1,
        }
    }

    /// The reasons behind a non-clean verdict.
    #[must_use]
    pub fn reasons(&self) -> &[String] {
        match self {
            Self::Clean => &[],
            Self::Inconclusive(r) | Self::Failed(r) => r,
        }
    }
}

/// Conclude an institutions run.
///
/// `wrong_source` comes from [`ProvenanceReport::is_wrong_source`]; it is a failure in
/// its own right even when every value matches, because the values matching a column the
/// importer does not intend to read is the defect, not the absence of one.
#[must_use]
pub fn institutions_verdict(report: &FidelityReport, wrong_source: bool) -> Verdict {
    if report.coverage.in_both == 0 {
        return Verdict::Inconclusive(vec![
            "no rows could be compared — the file and the backend share no unitids".into(),
        ]);
    }
    let mut reasons = Vec::new();
    if report.coverage.missing_from_db > 0 {
        reasons.push(format!(
            "{} row(s) in the file are not in the backend",
            report.coverage.missing_from_db
        ));
    }
    let failing = report.failing_columns();
    if failing > 0 {
        reasons.push(format!(
            "{failing} column(s) disagree, {} value(s) total",
            report.total_mismatches()
        ));
    }
    if wrong_source {
        reasons.push("stored data came from a column the importer does not read".into());
    }
    if reasons.is_empty() {
        Verdict::Clean
    } else {
        Verdict::Failed(reasons)
    }
}

/// Conclude a completions run.
#[must_use]
pub fn completions_verdict(report: &CompletionsReport) -> Verdict {
    if report.coverage.in_both == 0 {
        return Verdict::Inconclusive(vec![
            "no rows could be compared — the sample matched nothing in the backend".into(),
        ]);
    }
    let mut reasons = Vec::new();
    if !report.counts_agree() {
        reasons.push(format!(
            "row counts differ: {} in the file, {} in the backend",
            report.rows_in_file, report.rows_in_db
        ));
    }
    if report.coverage.missing_from_db > 0 {
        reasons.push(format!(
            "{} sampled row(s) are not in the backend",
            report.coverage.missing_from_db
        ));
    }
    let failing = report.failing_columns();
    if failing > 0 {
        reasons.push(format!(
            "{failing} column(s) disagree in the sample, {} value(s) total",
            report.total_mismatches()
        ));
    }
    if report.dropped_from_file > 0 || report.dropped_from_db > 0 {
        reasons.push(format!(
            "{} file row(s) and {} stored row(s) could not be keyed and went unchecked",
            report.dropped_from_file, report.dropped_from_db
        ));
    }
    if reasons.is_empty() {
        Verdict::Clean
    } else {
        Verdict::Failed(reasons)
    }
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
    /// Whether every compared row agrees, over at least one row.
    ///
    /// The row count matters: with nothing compared, `0 == 0` would mark every candidate
    /// exact and the report would tick columns it never looked at.
    #[must_use]
    pub const fn is_exact(&self) -> bool {
        self.compared > 0 && self.agreements == self.compared
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
    // Stable sort with no secondary key, so equal scores keep their input order — which
    // is the importer's own priority. An alphabetical tie-break would list `C18BASIC`
    // above `C21BASIC` and visually promote the older vintage.
    matches.sort_by_key(|c| std::cmp::Reverse(c.agreements));

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

/// Identify which survey a file holds without parsing its rows.
///
/// # Errors
/// [`DatabaseError::ParseError`] if the file cannot be read, or its header matches
/// neither survey.
pub fn survey_kind_of(path: &Path) -> DatabaseResult<SurveyKind> {
    let content = read_file_or_zip(path)?;
    let mut reader = open_csv(&content);
    let raw = reader
        .headers()
        .map_err(|e| DatabaseError::ParseError(format!("CSV header error: {e}")))?
        .clone();
    let headers = uppercase_headers(&raw);
    SurveyKind::from_headers(&headers).ok_or_else(|| {
        DatabaseError::ParseError(format!(
            "{} is neither an HD nor a completions file — no INSTNM, and no CIPCODE+AWLEVEL",
            path.display()
        ))
    })
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
    let expected = client.count_rows(tables::INSTITUTIONS).await?;
    let body = client
        .select(tables::INSTITUTIONS, "*", &filters, Some(FETCH_LIMIT))
        .await?;
    let rows: Vec<Institution> = serde_json::from_value(body)
        .map_err(|e| DatabaseError::ParseError(format!("cannot read stored institutions: {e}")))?;
    // A capped read is an HTTP 200 with fewer rows and nothing to say so, and every
    // absent row would then be reported as "in the file, not in the backend" — a
    // confident wrong diagnosis. `count=exact` is not subject to the cap, so comparing
    // the two turns a silent truncation into a refusal to guess.
    if u64::try_from(rows.len()).is_ok_and(|got| got != expected) {
        return Err(DatabaseError::QueryError(format!(
            "read {} of {expected} institutions — the backend truncated the response. Run \
             `nuanalytics db doctor` and check its row-limit line (PGRST_DB_MAX_ROWS)",
            rows.len()
        )));
    }
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
// Completions
// ============================================================================

/// Natural key of a completions row — the table's own unique constraint.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CompletionKey {
    /// IPEDS Unit ID.
    pub unitid: i32,
    /// CIP code in dot notation.
    pub cip_code: String,
    /// Award level code.
    pub award_level: i32,
    /// 1 = primary major, 2 = second major.
    pub major_num: i32,
}

impl std::fmt::Display for CompletionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{}/{}/{}",
            self.unitid, self.cip_code, self.award_level, self.major_num
        )
    }
}

impl CompletionKey {
    /// Build a key from a stored row, if it carries every key part.
    fn from_row(row: &Completion) -> Option<Self> {
        Some(Self {
            unitid: row.unitid?,
            cip_code: row.cip_code.clone()?,
            award_level: row.award_level?,
            major_num: row.major_num?,
        })
    }
}

/// How many institutions to draw into the value-comparison sample.
///
/// A completions file is ~313k rows per year, far too many to fetch. The count check
/// below covers every row; this bounds only the *value* comparison, and the report says
/// so rather than implying full coverage.
const SAMPLE_INSTITUTIONS: usize = 150;

/// Pick a spread of institutions from those present, deterministically.
///
/// Every k-th unitid rather than the first N: the lowest `UNITID`s cluster by state, so
/// taking a prefix would check Alabama thoroughly and nothing else. Deterministic so two
/// runs over the same file sample the same rows and their reports can be compared.
fn sample_unitids(all: &BTreeSet<i32>, wanted: usize) -> BTreeSet<i32> {
    // `wanted == 0` returns everything rather than nothing: it is unreachable today
    // (`SAMPLE_INSTITUTIONS` is a constant) and exists to keep the division below safe,
    // so the harmless answer is better than an empty sample that would read as "checked".
    if all.len() <= wanted || wanted == 0 {
        return all.clone();
    }
    let values: Vec<i32> = all.iter().copied().collect();
    let last = values.len() - 1;
    if wanted == 1 {
        return std::iter::once(values[0]).collect();
    }
    // Positions interpolated across the whole range, not a fixed stride. `len / wanted`
    // collapses to 1 whenever the population is less than twice the sample — 299 rows
    // and a 150 sample took the first 150, covering the bottom half, which is exactly
    // the prefix this is supposed to avoid.
    (0..wanted)
        .map(|i| values[i * last / (wanted - 1)])
        .collect()
}

/// What a completions comparison looked at.
#[derive(Debug, Clone)]
pub struct CompletionsReport {
    /// Rows in the survey file.
    pub rows_in_file: usize,
    /// Rows the backend holds for this year, from an exact count.
    pub rows_in_db: u64,
    /// Institutions drawn into the value sample.
    pub sampled_institutions: usize,
    /// Sampled file rows discarded because part of the natural key would not parse.
    ///
    /// Reported rather than silently skipped: a row that cannot be keyed is a row
    /// nothing checked, which is different from a row that matched.
    pub dropped_from_file: usize,
    /// Stored rows discarded for the same reason.
    pub dropped_from_db: usize,
    /// Row-level presence within the sample.
    pub coverage: Coverage,
    /// Per-column disagreement within the sample, worst first.
    pub columns: Vec<ColumnDiff>,
}

impl CompletionsReport {
    /// Columns with at least one disagreement.
    #[must_use]
    pub fn failing_columns(&self) -> usize {
        self.columns.iter().filter(|c| c.mismatched > 0).count()
    }

    /// Total disagreements across every column in the sample.
    #[must_use]
    pub fn total_mismatches(&self) -> usize {
        self.columns.iter().map(|c| c.mismatched).sum()
    }

    /// Whether the file and the backend hold the same number of rows for this year.
    #[must_use]
    pub fn counts_agree(&self) -> bool {
        u64::try_from(self.rows_in_file).is_ok_and(|n| n == self.rows_in_db)
    }
}

/// Read a completions file, keeping only rows for the sampled institutions.
///
/// Returns `(total rows in the file, sampled rows)`. The total counts every data row so
/// the count check covers the whole file even though the values do not.
///
/// # Errors
/// [`DatabaseError::ParseError`] if the file cannot be read or parsed.
pub fn read_completions(
    path: &Path,
    year: u16,
) -> DatabaseResult<(usize, usize, BTreeMap<CompletionKey, Completion>)> {
    let content = read_file_or_zip(path)?;
    let mut reader = open_csv(&content);
    let raw = reader
        .headers()
        .map_err(|e| DatabaseError::ParseError(format!("CSV header error: {e}")))?
        .clone();
    let headers = uppercase_headers(&raw);

    let col = |name: &str| {
        find_col(&headers, &[name]).ok_or_else(|| {
            DatabaseError::ParseError(format!("{} has no {name} column", path.display()))
        })
    };
    let (col_unitid, col_cip, col_awlevel) = (col("UNITID")?, col("CIPCODE")?, col("AWLEVEL")?);
    let col_majornum = find_col(&headers, &["MAJORNUM"]);
    let demo = DemoCols::for_completions(&headers);

    // Two passes: the first learns which institutions exist so the sample can be spread
    // across the whole file rather than its opening rows.
    let mut unitids = BTreeSet::new();
    let mut total = 0usize;
    for record in reader.records() {
        let record =
            record.map_err(|e| DatabaseError::ParseError(format!("CSV parse error: {e}")))?;
        if let Ok(u) = record.get(col_unitid).unwrap_or("").trim().parse::<i32>() {
            total += 1;
            unitids.insert(u);
        }
    }
    let sample = sample_unitids(&unitids, SAMPLE_INSTITUTIONS);

    let mut rows = BTreeMap::new();
    let mut dropped = 0usize;
    let mut reader = open_csv(&content);
    for record in reader.records() {
        let record =
            record.map_err(|e| DatabaseError::ParseError(format!("CSV parse error: {e}")))?;
        let Ok(unitid) = record.get(col_unitid).unwrap_or("").trim().parse::<i32>() else {
            continue;
        };
        if !sample.contains(&unitid) {
            continue;
        }
        let raw_cip = record.get(col_cip).unwrap_or("").trim().to_string();
        let award_level: Option<i32> = record.get(col_awlevel).and_then(|v| v.trim().parse().ok());
        let major_num: Option<i32> = col_majornum
            .and_then(|i| record.get(i))
            .and_then(|v| v.trim().parse().ok());
        let row = build_completion(
            unitid,
            &raw_cip,
            award_level,
            major_num,
            year,
            &demo,
            &record,
        );
        if let Some(key) = CompletionKey::from_row(&row) {
            rows.insert(key, row);
        } else {
            dropped += 1;
        }
    }
    Ok((total, dropped, rows))
}

/// Fetch the stored completions for a set of institutions in one year.
///
/// # Errors
/// Whatever [`DbClient::select`] reports.
pub async fn fetch_completions(
    client: &DbClient,
    year: u16,
    unitids: &BTreeSet<i32>,
) -> DatabaseResult<(usize, BTreeMap<CompletionKey, Completion>)> {
    let list: Vec<i32> = unitids.iter().copied().collect();
    let filters = QueryFilters::new()
        .eq("year", Some(year))
        .in_list("unitid", &list);

    let body = client
        .select(tables::COMPLETIONS, "*", &filters, Some(FETCH_LIMIT))
        .await?;
    let rows: Vec<Completion> = serde_json::from_value(body)
        .map_err(|e| DatabaseError::ParseError(format!("cannot read stored completions: {e}")))?;
    let total = rows.len();
    let keyed: BTreeMap<CompletionKey, Completion> = rows
        .into_iter()
        .filter_map(|r| CompletionKey::from_row(&r).map(|k| (k, r)))
        .collect();
    Ok((total - keyed.len(), keyed))
}

/// Compare sampled completions rows column by column.
#[must_use]
pub fn diff_completions(
    counts: (usize, u64),
    sampled_institutions: usize,
    dropped: (usize, usize),
    from_file: &BTreeMap<CompletionKey, Completion>,
    from_db: &BTreeMap<CompletionKey, Completion>,
) -> CompletionsReport {
    let (rows_in_file, rows_in_db) = counts;
    let (dropped_from_file, dropped_from_db) = dropped;
    let coverage = coverage_of(from_file, from_db);
    let mut columns = vec![
        compare(from_file, from_db, "total", |c| c.total),
        compare(from_file, from_db, "total_men", |c| c.total_men),
        compare(from_file, from_db, "total_women", |c| c.total_women),
        compare(from_file, from_db, "nonresident_alien_men", |c| {
            c.nonresident_alien_men
        }),
        compare(from_file, from_db, "nonresident_alien_women", |c| {
            c.nonresident_alien_women
        }),
        compare(from_file, from_db, "hispanic_men", |c| c.hispanic_men),
        compare(from_file, from_db, "hispanic_women", |c| c.hispanic_women),
        compare(from_file, from_db, "american_indian_men", |c| {
            c.american_indian_men
        }),
        compare(from_file, from_db, "american_indian_women", |c| {
            c.american_indian_women
        }),
        compare(from_file, from_db, "asian_men", |c| c.asian_men),
        compare(from_file, from_db, "asian_women", |c| c.asian_women),
        compare(from_file, from_db, "black_men", |c| c.black_men),
        compare(from_file, from_db, "black_women", |c| c.black_women),
        compare(from_file, from_db, "native_hawaiian_men", |c| {
            c.native_hawaiian_men
        }),
        compare(from_file, from_db, "native_hawaiian_women", |c| {
            c.native_hawaiian_women
        }),
        compare(from_file, from_db, "white_men", |c| c.white_men),
        compare(from_file, from_db, "white_women", |c| c.white_women),
        compare(from_file, from_db, "two_or_more_men", |c| c.two_or_more_men),
        compare(from_file, from_db, "two_or_more_women", |c| {
            c.two_or_more_women
        }),
        compare(from_file, from_db, "unknown_race_men", |c| {
            c.unknown_race_men
        }),
        compare(from_file, from_db, "unknown_race_women", |c| {
            c.unknown_race_women
        }),
    ];
    columns.sort_by(|a, b| b.mismatched.cmp(&a.mismatched).then(a.column.cmp(b.column)));

    CompletionsReport {
        rows_in_file,
        rows_in_db,
        sampled_institutions,
        dropped_from_file,
        dropped_from_db,
        coverage,
        columns,
    }
}

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
        // Asymmetric on purpose: with one row missing on each side the two counters
        // could be transposed and the test would still pass.
        let file = map(vec![inst(1), inst(2), inst(3)]);
        let db = map(vec![inst(3)]);
        let report = diff_institutions(&file, &db);

        assert_eq!(report.coverage.in_both, 1, "only unitid 3 is on both sides");
        assert_eq!(report.coverage.missing_from_db, 2, "unitids 1 and 2");
        assert_eq!(report.coverage.absent_from_file, 0);
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
        // 2 of 10 is asymmetric, so an inverted rate (compared/mismatched) fails here
        // where the 100% case above would not.
        assert!((report.columns[1].mismatch_rate() - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn examples_are_the_lowest_keys() {
        // Pins that `compare` walks the file map in key order and takes examples from
        // the front. Comparing two runs of the same pure function proves nothing.
        let file: BTreeMap<i32, Institution> = map((1..=50).map(inst).collect());
        let db: BTreeMap<i32, Institution> = map((1..=50)
            .map(|i| {
                let mut row = inst(i);
                row.city = Some("Elsewhere".into());
                row
            })
            .collect());

        let report = diff_institutions(&file, &db);
        let city = report.columns.iter().find(|c| c.column == "city").unwrap();
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

    // --- completions sampling -----------------------------------------------

    #[test]
    fn sampling_spreads_across_the_range_rather_than_taking_a_prefix() {
        // UNITIDs cluster by state, so a prefix would check Alabama thoroughly and
        // nothing else. The sample must reach the top of the range.
        let all: BTreeSet<i32> = (100_000..100_500).collect();
        let picked = sample_unitids(&all, 10);

        assert_eq!(picked.len(), 10);
        assert!(
            picked.iter().copied().max().unwrap() > 100_400,
            "sample stops at {:?} — it is a prefix, not a spread",
            picked.iter().copied().max()
        );
        assert!(picked.contains(&100_000), "should start at the lowest");
    }

    #[test]
    fn sampling_picks_evenly_spaced_positions() {
        // Pins the interpolation itself. Calling the function twice and comparing the
        // results — which this test used to do — passes for *any* pure implementation,
        // including the prefix bug it was supposed to guard.
        let all: BTreeSet<i32> = (1..=1000).collect();
        let picked: Vec<i32> = sample_unitids(&all, 11).into_iter().collect();
        assert_eq!(
            picked,
            vec![1, 100, 200, 300, 400, 500, 600, 700, 800, 900, 1000],
            "evenly spaced with both endpoints included"
        );
    }

    #[test]
    fn sampling_still_spreads_when_the_population_barely_exceeds_the_sample() {
        // The case the old stride formula got wrong: `len / wanted` collapses to 1
        // whenever the population is under twice the sample, yielding a prefix over the
        // bottom half of the range.
        let all: BTreeSet<i32> = (1..=299).collect();
        let picked = sample_unitids(&all, 150);
        assert_eq!(picked.len(), 150);
        assert_eq!(
            picked.iter().copied().max(),
            Some(299),
            "a 150-sample of 299 must still reach the top, not stop at 150"
        );
    }

    #[test]
    fn sampling_returns_everything_when_the_population_is_small() {
        let all: BTreeSet<i32> = (1..=20).collect();
        assert_eq!(sample_unitids(&all, 150), all, "no need to sample 20 rows");
        assert_eq!(
            sample_unitids(&all, 0),
            all,
            "a zero request is not an empty sample"
        );
    }

    fn completion(unitid: i32, total: Option<i32>) -> Completion {
        Completion {
            id: None,
            unitid: Some(unitid),
            cip_code: Some("11.0101".into()),
            award_level: Some(5),
            major_num: Some(1),
            year: Some(2025),
            total,
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

    fn keyed(rows: Vec<Completion>) -> BTreeMap<CompletionKey, Completion> {
        rows.into_iter()
            .filter_map(|r| CompletionKey::from_row(&r).map(|k| (k, r)))
            .collect()
    }

    #[test]
    fn completions_report_separates_the_exact_count_from_the_sampled_values() {
        // The count covers every row; the value comparison covers a sample. Conflating
        // them would let a clean sample imply a clean import.
        let file = keyed(vec![completion(1, Some(99)), completion(2, Some(5))]);
        let db = keyed(vec![completion(1, None), completion(2, Some(5))]);

        let report = diff_completions((313_566, 313_566), 2, (0, 0), &file, &db);
        assert!(report.counts_agree(), "every row is accounted for");
        assert_eq!(report.total_mismatches(), 1, "but a sampled value is wrong");
        assert_eq!(report.failing_columns(), 1);
        assert_eq!(report.coverage.in_both, 2);

        let total = report.columns.iter().find(|c| c.column == "total").unwrap();
        assert_eq!(total.examples[0].in_file, "99");
        assert_eq!(total.examples[0].in_db, "NULL");
        assert_eq!(
            total.examples[0].key, "1/11.0101/5/1",
            "the key must identify the row well enough to look up by hand"
        );
    }

    #[test]
    fn completions_counts_disagree_when_rows_are_missing() {
        let file = keyed(vec![completion(1, Some(1))]);
        let report = diff_completions((313_566, 300_000), 1, (0, 0), &file, &file);
        assert!(
            !report.counts_agree(),
            "a short import must fail even when every sampled value matches"
        );
        assert_eq!(report.total_mismatches(), 0);
    }

    /// A named key part and the way to blank it, for the key-completeness test.
    type ClearPart = (&'static str, fn(&mut Completion));

    #[test]
    fn completion_key_needs_every_part() {
        assert!(CompletionKey::from_row(&completion(1, Some(1))).is_some());

        // Every part, one at a time: any one of them becoming lenient would let a row
        // be matched against the wrong row instead of reported as unkeyable.
        let clear: [ClearPart; 4] = [
            ("unitid", |c| c.unitid = None),
            ("cip_code", |c| c.cip_code = None),
            ("award_level", |c| c.award_level = None),
            ("major_num", |c| c.major_num = None),
        ];
        for (part, clear_it) in clear {
            let mut row = completion(1, Some(1));
            clear_it(&mut row);
            assert!(
                CompletionKey::from_row(&row).is_none(),
                "a row with no {part} must not be keyed"
            );
        }
    }

    // --- reading real files -------------------------------------------------

    /// Write a CSV to a temp file and hand back the path plus its guard.
    fn csv_file(body: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("survey.csv");
        std::fs::write(&path, body).expect("write csv");
        (dir, path)
    }

    #[test]
    fn carnegie_candidates_come_back_in_importer_priority_not_file_order() {
        // Load-bearing: `diff_provenance` treats the first entry as the column the
        // importer would pick, so if this returned file order instead, `expected` would
        // name the wrong column and the whole check would invert. The header puts
        // C18BASIC first precisely so file order cannot be what makes this pass.
        let (_dir, path) = csv_file(
            "UNITID,C18BASIC,C21BASIC\n             1,10,11\n             2,18,17\n             3,-2,20\n",
        );
        let got = read_carnegie_candidates(&path).expect("read");

        assert_eq!(
            got.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
            ["C21BASIC", "C18BASIC"],
            "order must follow HD_CARNEGIE_CANDIDATES, not the header"
        );
        // And each slot must hold its own column's values — the zip pairing is a
        // classic transposition site.
        assert_eq!(got[0].1[&1], Some(11), "C21BASIC row 1");
        assert_eq!(got[1].1[&1], Some(10), "C18BASIC row 1");
        assert_eq!(got[0].1[&3], Some(20));
        assert_eq!(
            got[1].1[&3],
            Some(-2),
            "-2 is a labelled code, not a sentinel"
        );
    }

    #[test]
    fn carnegie_candidates_are_empty_when_the_file_has_none() {
        let (_dir, path) = csv_file("UNITID,INSTNM\n1,A College\n");
        assert!(read_carnegie_candidates(&path).expect("read").is_empty());
    }

    #[test]
    fn reading_completions_counts_every_row_but_samples_values() {
        // `total` must count every data row — it is compared against an exact backend
        // count — while the keyed map holds only sampled rows.
        let (_dir, path) = csv_file(
            "UNITID,CIPCODE,AWLEVEL,MAJORNUM,CTOTALT\n             1,11.0101,5,1,10\n             1,11.0701,5,1,99\n             2,11.0101,5,1,20\n             not-a-number,11.0101,5,1,30\n             3,11.0101,5,1,40\n",
        );
        let (total, dropped, rows) = read_completions(&path, 2025).expect("read");

        assert_eq!(
            total, 4,
            "the header and the unparsable UNITID are not rows"
        );
        assert_eq!(dropped, 0);
        assert_eq!(rows.len(), 4, "3 institutions is under the sample size");
        let key = CompletionKey {
            unitid: 1,
            cip_code: "11.0701".into(),
            award_level: 5,
            major_num: 1,
        };
        assert_eq!(
            rows[&key].total,
            Some(99),
            "99 is ninety-nine graduates, which is the defect this command exists to find"
        );
    }

    #[test]
    fn reading_completions_reports_rows_it_could_not_key() {
        // A missing MAJORNUM column makes every row unkeyable. That must be counted,
        // not silently skipped, or the run would look clean having checked nothing.
        let (_dir, path) =
            csv_file("UNITID,CIPCODE,AWLEVEL,CTOTALT\n1,11.0101,5,10\n2,11.0101,5,20\n");
        let (total, dropped, rows) = read_completions(&path, 2025).expect("read");
        assert_eq!(total, 2);
        assert_eq!(dropped, 2, "both rows lack MAJORNUM");
        assert!(rows.is_empty());
    }

    #[test]
    fn survey_kind_of_rejects_a_file_that_is_neither() {
        let (_dir, path) = csv_file("FOO,BAR\n1,2\n");
        let err = survey_kind_of(&path).expect_err("not a survey file");
        let msg = err.to_string();
        assert!(msg.contains("INSTNM"), "must say what it looked for: {msg}");
        assert!(msg.contains("CIPCODE"), "{msg}");
    }

    // --- verdicts -----------------------------------------------------------

    #[test]
    fn a_partial_import_fails_even_when_every_column_matches() {
        // The worst of the reporting faults: the verdict used to look only at column
        // mismatches, so rows that never landed printed "every column matches" and
        // exited 0 — three lines under a "412 in file, not stored" line.
        let file = map(vec![inst(1), inst(2), inst(3)]);
        let db = map(vec![inst(1)]);
        let report = diff_institutions(&file, &db);
        assert_eq!(report.failing_columns(), 0, "every compared value agrees");

        let verdict = institutions_verdict(&report, false);
        assert!(matches!(verdict, Verdict::Failed(_)), "{verdict:?}");
        assert_eq!(verdict.exit_code(), 1);
        assert!(
            verdict.reasons()[0].contains("2 row(s) in the file are not in the backend"),
            "{:?}",
            verdict.reasons()
        );
    }

    #[test]
    fn comparing_nothing_is_inconclusive_not_clean() {
        // "I checked and found nothing wrong" and "I could not check" are different
        // answers, and only one of them justifies trusting the data.
        let file = map(vec![inst(1)]);
        let db = map(vec![inst(2)]);
        let verdict = institutions_verdict(&diff_institutions(&file, &db), false);

        assert!(matches!(verdict, Verdict::Inconclusive(_)), "{verdict:?}");
        assert_eq!(verdict.exit_code(), 1, "an unchecked backend is not a pass");
    }

    #[test]
    fn a_wrong_provenance_source_fails_on_its_own() {
        // Values matching a column the importer does not read *is* the defect; every
        // value agreeing is what makes it invisible.
        let rows = map(vec![inst(1)]);
        let report = diff_institutions(&rows, &rows);
        assert_eq!(report.total_mismatches(), 0);

        let verdict = institutions_verdict(&report, true);
        assert!(matches!(verdict, Verdict::Failed(_)), "{verdict:?}");
        assert!(verdict.reasons()[0].contains("column the importer does not read"));
        assert_eq!(institutions_verdict(&report, false), Verdict::Clean);
    }

    #[test]
    fn completions_verdict_names_every_reason_it_failed() {
        // The summary used to be built from `failing_columns()` alone, so a run that
        // failed purely on row counts printed "0 column(s) disagree" while exiting 1.
        let file = keyed(vec![completion(1, Some(1))]);
        let report = diff_completions((313_566, 300_000), 1, (4, 2), &file, &file);
        let verdict = completions_verdict(&report);

        let reasons = verdict.reasons().join(" | ");
        assert!(reasons.contains("row counts differ"), "{reasons}");
        assert!(reasons.contains("could not be keyed"), "{reasons}");
        assert!(
            !reasons.contains("column(s) disagree"),
            "no column disagreed; saying so would be noise: {reasons}"
        );
        assert_eq!(verdict.exit_code(), 1);
    }

    #[test]
    fn a_clean_run_is_clean() {
        let rows = keyed(vec![completion(1, Some(7))]);
        let report = diff_completions((1, 1), 1, (0, 0), &rows, &rows);
        assert_eq!(completions_verdict(&report), Verdict::Clean);
        assert_eq!(completions_verdict(&report).exit_code(), 0);
    }

    #[test]
    fn an_empty_candidate_is_not_an_exact_match() {
        // `0 == 0` would tick every candidate on an empty backend.
        let empty = CandidateMatch {
            source_column: "C21BASIC".into(),
            agreements: 0,
            compared: 0,
        };
        assert!(!empty.is_exact(), "nothing compared is not a match");
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
