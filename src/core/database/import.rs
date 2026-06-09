//! Shared "degree import" core.
//!
//! Loads a degree-first analysis report (or a plain unified degree) into the
//! normalized program tables (`programs`, `courses`, `program_courses`,
//! `program_requirements`) plus, when the input carries an `analysis` block, one
//! analysis run (`analysis_runs`, `analysis_course_metrics`, `analysis_plans`).
//!
//! The flow is split into a **pure builder** ([`build_import_plan`]) that turns a
//! report text into row structs without touching the database (used by
//! `--dry-run`), and an **async executor** ([`execute_import`]) that resolves the
//! institution, performs the existence/verified decision, and upserts the rows
//! (children first, the `programs` commit marker last).
//!
//! See `docs/database/programs-schema.sql` and the approved design plan for the
//! exact semantics this module follows.

use serde_json::Value;
use sha2::{Digest, Sha256};

use super::error::{DatabaseError, DatabaseResult};
use super::models::{
    StoredAnalysisCourseMetric, StoredAnalysisPlan, StoredAnalysisRun, StoredCourse, StoredProgram,
    StoredProgramCourse, StoredProgramRequirement,
};
use super::query::QueryFilters;
use super::tables;
use super::DbClient;
use crate::core::degree::{parse_degree_auto, serialize_degree_json, to_unified_value};
use crate::core::models::degree::{Requirement, RequirementType};
use crate::core::models::{Course, DegreeProgram};
use crate::core::prerequisite_parser::parse_to_ast;

/// Stable sha256 content hash (lowercase hex).
///
/// Used for persisted hashes (`document_hash`, `run_key`, fingerprint program
/// keys) — `DefaultHasher` is intentionally avoided because it is not stable
/// across Rust versions.
#[must_use]
pub fn content_hash(s: &str) -> String {
    let digest = Sha256::digest(s.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        // Writing hex into a String never fails.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

// --- safe numeric conversions for projecting model fields into DB rows ------
// Degree/Course credit fields are small `u32`/`usize` counts and `f64` metric
// stats; these helpers convert into the DB column widths (`i32`/`f32`/`i64`)
// without lossy-cast lints, saturating on the (practically unreachable)
// out-of-range case.

/// `u32` count → `i32`, saturating at `i32::MAX`.
fn u32_to_i32(v: u32) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

/// `usize` count → `i32`, saturating at `i32::MAX`.
fn usize_to_i32(v: usize) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

/// JSON `i64` → `i32`, saturating into range.
fn i64_to_i32(v: i64) -> i32 {
    i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX })
}

/// `u128` epoch-ms → `i64`, saturating at `i64::MAX`.
fn u128_to_i64(v: u128) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// JSON `f64` metric stat → `f32` (the DB column width). The narrowing is
/// intentional; metric means fit comfortably in `f32`.
#[allow(clippy::cast_possible_truncation)]
const fn f64_to_f32(v: f64) -> f32 {
    v as f32
}

/// Options controlling a single degree import.
//
// The four boolean flags (`force`/`replace`/`skip_existing`/`dry_run`) map
// 1:1 onto the CLI/MCP flags this options struct mirrors; folding them into an
// enum would obscure that direct correspondence and break callers that set them
// independently, so the `struct_excessive_bools` lint is allowed here by design.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct ImportOptions {
    /// Analysis-run variant label (default `"full"`). `full` is the canonical
    /// upload that writes the program projection; non-`full` only attaches a run.
    pub variant: String,
    /// Override the resolved institution unit id.
    pub unitid: Option<i32>,
    /// Override the institution name.
    pub institution: Option<String>,
    /// Override the CIP code (part of the natural program key).
    pub cip_code: Option<String>,
    /// Override the catalog year (part of the program identity).
    pub catalog_year: Option<String>,
    /// Override the degree id.
    pub degree_id: Option<String>,
    /// Overwrite a verified program / skip confirmation.
    pub force: bool,
    /// Replace an existing (unverified) program.
    pub replace: bool,
    /// Skip the program entirely if it already exists.
    pub skip_existing: bool,
    /// Build the plan but write nothing.
    pub dry_run: bool,
}

impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            variant: "full".to_string(),
            unitid: None,
            institution: None,
            cip_code: None,
            catalog_year: None,
            degree_id: None,
            force: false,
            replace: false,
            skip_existing: false,
            dry_run: false,
        }
    }
}

/// Outcome class for one import.
#[derive(Debug, Clone)]
pub enum ImportResult {
    /// The program did not exist and was created.
    Created,
    /// The program existed and was overwritten.
    Updated,
    /// The program existed and was left untouched.
    Skipped,
    /// The program exists and overwriting requires explicit confirmation.
    /// Carries a human-readable reason.
    NeedsConfirmation(String),
    /// The institution name matched several institutions; the caller must pick
    /// one and re-run with an explicit `unitid`. Carries `(unitid, name)` pairs.
    InstitutionAmbiguous(Vec<(i32, String)>),
    /// The report could not be turned into a valid plan. Carries the errors.
    Rejected(Vec<String>),
}

/// The full result of one import (counts + diagnostics).
#[derive(Debug, Clone)]
pub struct ImportOutcome {
    /// The deterministic program key this import resolved to.
    pub program_key: String,
    /// What happened to the program.
    pub result: ImportResult,
    /// The resolved IPEDS unit id, when one was determined.
    pub resolved_unitid: Option<i32>,
    /// The analysis-run variant label.
    pub variant: String,
    /// Institution name from the degree (raw), for display.
    pub institution: Option<String>,
    /// Plan variations enumerated/sampled by the analysis run; `None` when the
    /// upload carried no metrics (a degree-only upload).
    pub variations_run: Option<i32>,
    /// Plan sampling strategy used by the analysis run, if any.
    pub sample_type: Option<String>,
    /// Number of `courses` rows written.
    pub courses_written: usize,
    /// Number of `program_requirements` rows written.
    pub requirements_written: usize,
    /// Whether an `analysis_runs` row was written.
    pub run_written: bool,
    /// Number of `analysis_plans` rows written.
    pub plans_written: usize,
    /// Number of `analysis_course_metrics` rows written.
    pub course_metrics_written: usize,
    /// Warnings surfaced while parsing/converting the degree.
    pub conversion_warnings: Vec<String>,
    /// Informational messages about decisions taken.
    pub messages: Vec<String>,
}

/// A fully-built set of rows ready to write — produced without touching the DB.
#[derive(Debug, Clone)]
pub struct ImportPlan {
    /// The program row (always populated; the executor decides whether to write
    /// it based on [`Self::is_full`] and the existence decision).
    pub program: Option<StoredProgram>,
    /// Whether the variant is the canonical `full` upload.
    pub is_full: bool,
    /// Shared course catalog rows.
    pub courses: Vec<StoredCourse>,
    /// Program↔course junction rows.
    pub program_courses: Vec<StoredProgramCourse>,
    /// Flattened requirement-tree rows.
    pub requirements: Vec<StoredProgramRequirement>,
    /// The analysis run, when the report carried an `analysis` block.
    pub run: Option<StoredAnalysisRun>,
    /// Per-course analysis metric rows.
    pub course_metrics: Vec<StoredAnalysisCourseMetric>,
    /// Selected-plan rows.
    pub plans: Vec<StoredAnalysisPlan>,
}

/// Normalize an institution name into a stable slug:
/// lowercase, trimmed, non-alphanumeric → `-`, collapsed dashes, no leading/
/// trailing dash.
#[must_use]
pub fn institution_slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_dash = true; // suppress leading dashes
    for ch in name.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Compute the `institution_ref` (course-catalog partition key).
fn institution_ref(resolved_unitid: Option<i32>, institution: Option<&str>) -> String {
    resolved_unitid.map_or_else(
        || institution.map_or_else(|| "unknown".to_string(), institution_slug),
        |u| u.to_string(),
    )
}

/// Compute the deterministic `program_key` for a degree.
fn program_key(
    degree: &crate::core::models::Degree,
    opts: &ImportOptions,
    resolved_unitid: Option<i32>,
) -> String {
    let cip = opts
        .cip_code
        .clone()
        .or_else(|| degree.cip_code.clone())
        .unwrap_or_default();
    let cat = opts
        .catalog_year
        .clone()
        .or_else(|| degree.catalog_year.clone())
        .unwrap_or_default();
    let dtype = &degree.degree_type;
    if let Some(u) = resolved_unitid {
        format!("prog:{u}|{cip}|{cat}|{dtype}")
    } else if let Some(id) = opts.degree_id.clone().or_else(|| degree.id.clone()) {
        format!("prog:{id}|{cat}")
    } else {
        let institution = degree.institution.clone().unwrap_or_default();
        let source_url = degree.source_url.clone().unwrap_or_default();
        let fp = [
            institution,
            degree.name.clone(),
            dtype.clone(),
            cat,
            source_url,
        ]
        .join("|");
        format!("fp:{}", content_hash(&fp))
    }
}

/// Derive the normalized `program_kind` from degree type, tags, and name.
fn derive_program_kind(degree: &crate::core::models::Degree) -> String {
    let kinds = [
        "major",
        "minor",
        "concentration",
        "certificate",
        "track",
        "specialization",
        "emphasis",
        "micro",
    ];
    let haystack = format!(
        "{} {} {}",
        degree.degree_type.to_lowercase(),
        degree.name.to_lowercase(),
        degree
            .tags
            .as_ref()
            .map(|t| t.join(" ").to_lowercase())
            .unwrap_or_default()
    );
    for kind in kinds {
        if haystack.contains(kind) {
            return kind.to_string();
        }
    }
    "major".to_string()
}

/// Derive the primary `discipline` from the degree tags (first of ai/cs/ds/cy).
fn derive_discipline(degree: &crate::core::models::Degree) -> Option<String> {
    let tags = degree.tags.as_ref()?;
    let lower: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();
    for disc in ["ai", "cs", "ds", "cy"] {
        if lower.iter().any(|t| t == disc) {
            return Some(disc.to_string());
        }
    }
    None
}

/// Pull the `mean` of a named metric stat block out of a metrics `Value`.
fn metric_mean(metrics: &Value, key: &str) -> Option<f32> {
    metrics
        .get(key)
        .and_then(|m| m.get("mean"))
        .and_then(Value::as_f64)
        .map(f64_to_f32)
}

/// Build the structured prerequisite tree for a course from its raw expression.
fn course_prereq_value(course: &Course) -> Option<Value> {
    let raw = course.prerequisites_raw.as_deref()?;
    let expr = parse_to_ast(raw)?;
    serde_json::to_value(&expr).ok()
}

/// Build a [`StoredCourse`] from a parsed [`Course`].
fn build_course(
    course: &Course,
    course_code: &str,
    institution_ref: &str,
    resolved_unitid: Option<i32>,
    generation: i64,
) -> StoredCourse {
    let (credit_min, credit_max) = course.credit_range.as_ref().map_or((None, None), |r| {
        (Some(u32_to_i32(r.min)), Some(u32_to_i32(r.max)))
    });
    StoredCourse {
        id: None,
        institution_ref: institution_ref.to_string(),
        unitid: resolved_unitid,
        course_code: course_code.to_string(),
        prefix: (!course.prefix.is_empty()).then(|| course.prefix.clone()),
        number: (!course.number.is_empty()).then(|| course.number.clone()),
        name: (!course.name.is_empty()).then(|| course.name.clone()),
        credit_hours: Some(course.credit_hours),
        credit_min,
        credit_max,
        prerequisites: course_prereq_value(course),
        prerequisites_raw: course.prerequisites_raw.clone(),
        gen_ed_attributes: course.gen_ed_attributes.clone(),
        cross_listed_as: course.cross_listed_as.clone(),
        tags: course.tags.clone(),
        generation,
        created_at: None,
        updated_at: None,
    }
}

/// Derive the requirement's effective double-count flag.
///
/// `exclude_used = true`  → no double counting (`Some(false)`).
/// `exclude_used = false` → double counting allowed (`Some(true)`).
/// unset → fall back to the program's global `allow_double_counting` default.
fn allow_double_count(req: &Requirement, program_default: Option<bool>) -> Option<bool> {
    match req.constraints.as_ref().and_then(|c| c.exclude_used) {
        Some(true) => Some(false),
        Some(false) => Some(true),
        None => program_default,
    }
}

/// Whether a `select` requirement is logically impossible (`count` exceeds the
/// known explicit pool size). Only an explicit `from.courses` list yields a
/// known pool; patterns/groups → unknown → not impossible.
fn requirement_is_impossible(req: &Requirement) -> bool {
    if req.req_type != RequirementType::Select {
        return false;
    }
    let Some(count) = req.count else {
        return false;
    };
    req.from
        .as_ref()
        .and_then(|f| f.courses.as_ref())
        .is_some_and(|pool| u64::from(count) > pool.len() as u64)
}

/// Map a [`RequirementType`] to its DB string.
const fn req_type_str(t: &RequirementType) -> &'static str {
    match t {
        RequirementType::All => "all",
        RequirementType::Select => "select",
        RequirementType::OneOf => "one_of",
    }
}

/// Per-program context threaded through the requirement-tree walk (the bits
/// that stay constant across every node).
struct ReqContext<'a> {
    program_key: &'a str,
    /// Program-level `allow_double_counting` default.
    program_default: Option<bool>,
    generation: i64,
}

/// The address of one requirement node in the flattened tree.
struct ReqAddress<'a> {
    req_path: &'a str,
    parent_path: Option<&'a str>,
    map_key: Option<&'a str>,
    option_id: Option<&'a str>,
    option_name: Option<&'a str>,
}

/// Build a single [`StoredProgramRequirement`] row from a [`Requirement`].
fn build_requirement_row(
    req: &Requirement,
    ctx: &ReqContext,
    addr: &ReqAddress,
) -> StoredProgramRequirement {
    let (credit_min, credit_max) = req.credit_range.as_ref().map_or((None, None), |r| {
        (Some(u32_to_i32(r.min)), Some(u32_to_i32(r.max)))
    });
    StoredProgramRequirement {
        id: None,
        program_key: ctx.program_key.to_string(),
        req_path: addr.req_path.to_string(),
        parent_path: addr.parent_path.map(str::to_string),
        map_key: addr.map_key.map(str::to_string),
        option_id: addr.option_id.map(str::to_string),
        option_name: addr.option_name.map(str::to_string),
        name: req.name.clone(),
        req_type: req_type_str(&req.req_type).to_string(),
        category: req.category.clone(),
        count: req.count.map(u32_to_i32),
        credits: req.credits.map(u32_to_i32),
        credit_min,
        credit_max,
        tags: req.tags.clone(),
        courses: req
            .courses
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        selection_spec: req.from.as_ref().and_then(|f| serde_json::to_value(f).ok()),
        req_constraints: req
            .constraints
            .as_ref()
            .and_then(|c| serde_json::to_value(c).ok()),
        is_impossible: requirement_is_impossible(req),
        allow_double_count: allow_double_count(req, ctx.program_default),
        generation: ctx.generation,
    }
}

/// Recursively flatten a requirement (and any `one_of` options) into rows.
fn flatten_requirement(
    req: &Requirement,
    ctx: &ReqContext,
    addr: &ReqAddress,
    out: &mut Vec<StoredProgramRequirement>,
) {
    out.push(build_requirement_row(req, ctx, addr));

    if req.req_type == RequirementType::OneOf {
        if let Some(options) = req.options.as_ref() {
            for option in options {
                for (j, nested) in option.requirements.iter().enumerate() {
                    let nested_path = format!("{}#{}#{j}", addr.req_path, option.id);
                    flatten_requirement(
                        nested,
                        ctx,
                        &ReqAddress {
                            req_path: &nested_path,
                            parent_path: Some(addr.req_path),
                            map_key: None,
                            option_id: Some(&option.id),
                            option_name: Some(&option.name),
                        },
                        out,
                    );
                }
            }
        }
    }
}

/// Build the full set of rows for an import without touching the database.
///
/// `resolved_unitid` and `generation` are supplied by the executor (or a test).
///
/// # Errors
/// Returns [`DatabaseError::ParseError`] when the report text cannot be parsed
/// as a degree or serialized back to the canonical document.
// Top-level sequential assembly of one `ImportPlan`. The sub-steps already
// delegate to helpers (`build_course`, `flatten_requirement`, `build_analysis_run`);
// the remaining length is the flat field-by-field `StoredProgram` mapping, which
// reads clearer inline than behind a 9-field args struct — so the line-count lint
// is allowed here by design.
#[allow(clippy::too_many_lines)]
pub fn build_import_plan(
    report_text: &str,
    opts: &ImportOptions,
    resolved_unitid: Option<i32>,
    generation: i64,
) -> Result<(ImportPlan, Vec<String>), DatabaseError> {
    // (a) canonical degree (ignores analysis / metrics / selected_plans blocks)
    let (program, conversion_warnings) = parse_degree_auto(report_text)
        .map_err(|e| DatabaseError::ParseError(format!("failed to parse degree: {e}")))?;
    // (b) raw report value for the report-only blocks
    let raw: Value = serde_json::from_str(report_text).unwrap_or(Value::Null);

    let degree = &program.degree;
    let variant = opts.variant.clone();
    let is_full = variant.eq_ignore_ascii_case("full");

    let inst_ref = institution_ref(resolved_unitid, degree.institution.as_deref());
    let program_key = program_key(degree, opts, resolved_unitid);

    let document = to_unified_value(&program)
        .map_err(|e| DatabaseError::ParseError(format!("failed to build document: {e}")))?;
    let document_hash = content_hash(
        &serialize_degree_json(&program, false)
            .map_err(|e| DatabaseError::ParseError(format!("failed to serialize degree: {e}")))?,
    );

    // ---- courses + program_courses --------------------------------------
    let mut courses = Vec::with_capacity(program.courses.len());
    let mut program_courses = Vec::with_capacity(program.courses.len());
    let mut course_keys: Vec<&String> = program.courses.keys().collect();
    course_keys.sort(); // deterministic order
    for key in &course_keys {
        let course = &program.courses[*key];
        courses.push(build_course(
            course,
            key,
            &inst_ref,
            resolved_unitid,
            generation,
        ));
        program_courses.push(StoredProgramCourse {
            id: None,
            program_key: program_key.clone(),
            institution_ref: inst_ref.clone(),
            course_code: (*key).clone(),
            credit_hours_override: None,
            name_as_listed: None,
            generation,
        });
    }

    // ---- requirements (recursive, deterministic by map key) -------------
    let mut requirements = Vec::new();
    let mut req_keys: Vec<&String> = program.requirements.keys().collect();
    req_keys.sort();
    let req_ctx = ReqContext {
        program_key: &program_key,
        program_default: degree.allow_double_counting,
        generation,
    };
    for key in &req_keys {
        let req = &program.requirements[*key];
        flatten_requirement(
            req,
            &req_ctx,
            &ReqAddress {
                req_path: key,
                parent_path: None,
                map_key: Some(key),
                option_id: None,
                option_name: None,
            },
            &mut requirements,
        );
    }

    let has_impossible = requirements.iter().any(|r| r.is_impossible);

    // ---- program row (always built; executor decides whether to write) --
    let stored_program = StoredProgram {
        id: None,
        program_key: program_key.clone(),
        degree_id: opts.degree_id.clone().or_else(|| degree.id.clone()),
        unitid: resolved_unitid,
        institution_ref: inst_ref,
        institution_raw: opts
            .institution
            .clone()
            .or_else(|| degree.institution.clone()),
        cip_code: opts.cip_code.clone().or_else(|| degree.cip_code.clone()),
        name: degree.name.clone(),
        degree_type: (!degree.degree_type.is_empty()).then(|| degree.degree_type.clone()),
        program_kind: Some(derive_program_kind(degree)),
        discipline: derive_discipline(degree),
        system_type: degree.system_type.clone(),
        tags: degree.tags.clone(),
        catalog_year: opts
            .catalog_year
            .clone()
            .or_else(|| degree.catalog_year.clone()),
        source_url: degree.source_url.clone(),
        total_credits: degree.total_credits.map(u32_to_i32),
        upper_division_credits: degree.upper_division_credits.map(u32_to_i32),
        in_major_credits: degree.in_major_credits.map(u32_to_i32),
        gpa_minimum: degree.gpa_minimum,
        gpa_major: degree.gpa_major,
        grade_minimum: degree.grade_minimum.clone(),
        major_subjects: degree.major_subjects.clone(),
        allow_double_counting: degree.allow_double_counting,
        document: document.clone(),
        document_hash: document_hash.clone(),
        verified: false,
        institution_resolved: resolved_unitid.is_some(),
        has_impossible_requirements: has_impossible,
        generation,
        created_at: None,
        updated_at: None,
    };

    // ---- analysis run + children (only when report has `analysis`) ------
    let (run, course_metrics, plans) = match raw.get("analysis").filter(|v| !v.is_null()) {
        Some(analysis) => {
            let run = build_analysis_run(BuildRunArgs {
                analysis,
                program: &program,
                program_key: &program_key,
                variant: &variant,
                is_full,
                document,
                document_hash,
                generation,
            })?;
            let course_metrics = build_course_metrics(&raw, &run.run_key, &program_key, generation);
            let plans = build_plans(&raw, &run.run_key, &program_key, generation);
            (Some(run), course_metrics, plans)
        }
        None => (None, Vec::new(), Vec::new()),
    };

    let plan = ImportPlan {
        program: Some(stored_program),
        is_full,
        courses,
        program_courses,
        requirements,
        run,
        course_metrics,
        plans,
    };
    Ok((plan, conversion_warnings))
}

/// Inputs for [`build_analysis_run`] (grouped so the builder stays under the
/// argument-count limit).
struct BuildRunArgs<'a> {
    analysis: &'a Value,
    program: &'a DegreeProgram,
    program_key: &'a str,
    variant: &'a str,
    is_full: bool,
    /// The canonical document (consumed for a non-full variant's `analyzed_document`).
    document: Value,
    /// The canonical document hash (reused for a full run).
    document_hash: String,
    generation: i64,
}

/// Build the [`StoredAnalysisRun`] from a report's `analysis` block.
fn build_analysis_run(args: BuildRunArgs) -> Result<StoredAnalysisRun, DatabaseError> {
    let BuildRunArgs {
        analysis,
        program,
        program_key,
        variant,
        is_full,
        document,
        document_hash,
        generation,
    } = args;

    let variations_run = analysis
        .get("variations_run")
        .and_then(Value::as_i64)
        .map(i64_to_i32);
    let sample_type = analysis
        .get("sample_type")
        .and_then(Value::as_str)
        .map(str::to_string);
    let degree_metrics = analysis.get("metrics").cloned();

    // For non-full variants the analyzed artifact differs from the program
    // document, so we record this variant's own hash + the document value.
    let analyzed_document_hash =
        if is_full {
            document_hash
        } else {
            content_hash(&serialize_degree_json(program, false).map_err(|e| {
                DatabaseError::ParseError(format!("failed to serialize variant: {e}"))
            })?)
        };
    let analyzed_document = if is_full { None } else { Some(document) };

    let run_key = content_hash(&format!(
        "{program_key}|{analyzed_document_hash}|{variant}|{}|{}|{}|{}|{}|{}",
        variations_run.map(|v| v.to_string()).unwrap_or_default(),
        sample_type.clone().unwrap_or_default(),
        "", // calc_strategy (None for now)
        "", // max_plans (None for now)
        "", // full_run (None for now)
        "", // include_joined (None for now)
    ));

    let (complexity_mean, delay_mean, credits_mean) =
        degree_metrics.as_ref().map_or((None, None, None), |m| {
            (
                metric_mean(m, "complexity"),
                metric_mean(m, "delay"),
                metric_mean(m, "credits"),
            )
        });

    Ok(StoredAnalysisRun {
        id: None,
        run_key,
        program_key: program_key.to_string(),
        analyzed_document_hash,
        variant: variant.to_string(),
        trimmed: !is_full && variant.eq_ignore_ascii_case("trimmed"),
        analyzed_document,
        variations_run,
        sample_type,
        calc_strategy: None,
        sampling_strategy: None,
        max_plans: None,
        full_run: None,
        included_courses: None,
        degree_metrics,
        complexity_mean,
        delay_mean,
        credits_mean,
        generation,
        created_at: None,
        updated_at: None,
    })
}

/// Build per-course metric rows from the raw report's `courses.*.metrics`.
fn build_course_metrics(
    raw: &Value,
    run_key: &str,
    program_key: &str,
    generation: i64,
) -> Vec<StoredAnalysisCourseMetric> {
    let Some(courses_obj) = raw.get("courses").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut metric_keys: Vec<&String> = courses_obj.keys().collect();
    metric_keys.sort();
    let mut out = Vec::new();
    for code in metric_keys {
        let Some(metrics) = courses_obj[code].get("metrics").filter(|v| !v.is_null()) else {
            continue;
        };
        let plan_count = metrics
            .get("plan_count")
            .and_then(Value::as_i64)
            .map(i64_to_i32);
        // Drop the redundant course_id before storing.
        let mut metrics_value = metrics.clone();
        if let Some(obj) = metrics_value.as_object_mut() {
            obj.remove("course_id");
        }
        out.push(StoredAnalysisCourseMetric {
            id: None,
            run_key: run_key.to_string(),
            program_key: program_key.to_string(),
            course_code: code.clone(),
            plan_count,
            complexity_mean: metric_mean(metrics, "complexity"),
            centrality_mean: metric_mean(metrics, "centrality"),
            delay_mean: metric_mean(metrics, "delay"),
            blocking_mean: metric_mean(metrics, "blocking"),
            metrics: Some(metrics_value),
            generation,
        });
    }
    out
}

/// Build selected-plan rows from the raw report's `selected_plans[]`.
fn build_plans(
    raw: &Value,
    run_key: &str,
    program_key: &str,
    generation: i64,
) -> Vec<StoredAnalysisPlan> {
    let Some(selected) = raw.get("selected_plans").and_then(Value::as_array) else {
        return Vec::new();
    };
    selected
        .iter()
        .enumerate()
        .map(|(i, plan)| StoredAnalysisPlan {
            id: None,
            run_key: run_key.to_string(),
            program_key: program_key.to_string(),
            plan_index: usize_to_i32(i),
            category: plan
                .get("category")
                .and_then(Value::as_str)
                .map(str::to_string),
            terms_required: plan
                .get("terms_required")
                .and_then(Value::as_i64)
                .map(i64_to_i32),
            total_complexity: plan
                .get("total_complexity")
                .and_then(Value::as_f64)
                .map(f64_to_f32),
            longest_delay: plan
                .get("longest_delay")
                .and_then(Value::as_f64)
                .map(f64_to_f32),
            credits: plan.get("credits").and_then(Value::as_f64).map(f64_to_f32),
            course_count: plan
                .get("course_count")
                .and_then(Value::as_i64)
                .map(i64_to_i32),
            is_calc_ready: plan.get("is_calc_ready").and_then(Value::as_bool),
            critical_path: plan.get("critical_path").cloned(),
            schedule: plan.get("schedule").cloned(),
            generation,
        })
        .collect()
}

/// Current epoch time in milliseconds (the per-import `generation` stamp).
fn now_generation() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u128_to_i64(d.as_millis()))
}

/// Snapshot of an existing program's idempotency fields.
#[derive(Debug, serde::Deserialize)]
struct ExistingProgram {
    document_hash: String,
    verified: bool,
}

/// Resolve the institution → unit id.
///
/// Returns `Ok(Some(unitid))` on a confident resolution, `Ok(None)` when the
/// institution stays unresolved, and `Err(candidates)` when the name matched
/// several institutions (the caller must disambiguate).
async fn resolve_institution(
    client: &DbClient,
    opts: &ImportOptions,
    degree: &crate::core::models::Degree,
) -> DatabaseResult<Result<Option<i32>, Vec<(i32, String)>>> {
    if let Some(u) = opts.unitid {
        return Ok(Ok(Some(u)));
    }
    if let Some(u) = degree.unitid {
        return Ok(Ok(Some(u)));
    }
    let Some(name) = opts
        .institution
        .as_deref()
        .or(degree.institution.as_deref())
    else {
        return Ok(Ok(None));
    };
    let filters = QueryFilters::new().ilike("name", Some(name));
    let value = client
        .select(
            tables::INSTITUTIONS,
            "unitid,name,state",
            &filters,
            Some(10),
        )
        .await?;
    let rows = value.as_array().cloned().unwrap_or_default();
    let candidates: Vec<(i32, String)> = rows
        .iter()
        .filter_map(|r| {
            let u = i64_to_i32(r.get("unitid").and_then(Value::as_i64)?);
            let n = r
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            Some((u, n))
        })
        .collect();
    match candidates.len() {
        0 => Ok(Ok(None)),
        1 => Ok(Ok(Some(candidates[0].0))),
        _ => Ok(Err(candidates)),
    }
}

/// Import a degree report into the database.
///
/// Resolves the institution, computes the existence/verified decision, builds
/// the row plan, and (unless `dry_run`) upserts the rows — children first, the
/// `programs` commit marker last.
///
/// Domain results (skipped, needs-confirmation, ambiguous, rejected) are
/// returned as `Ok(ImportOutcome)`; only transport/parse failures are `Err`.
///
/// # Errors
/// Returns [`DatabaseError`] when a query/upsert fails or the report cannot be
/// parsed.
pub async fn execute_import(
    client: &DbClient,
    report_text: &str,
    opts: &ImportOptions,
) -> DatabaseResult<ImportOutcome> {
    // Parse once up front so we can resolve the institution before building.
    let (program, _warn) = parse_degree_auto(report_text)
        .map_err(|e| DatabaseError::ParseError(format!("failed to parse degree: {e}")))?;
    let degree = program.degree.clone();

    // 1. institution resolution.
    let resolved_unitid = match resolve_institution(client, opts, &degree).await? {
        Ok(u) => u,
        Err(candidates) => {
            let key = program_key(&degree, opts, None);
            return Ok(ImportOutcome {
                program_key: key,
                result: ImportResult::InstitutionAmbiguous(candidates),
                resolved_unitid: None,
                variant: opts.variant.clone(),
                institution: opts
                    .institution
                    .clone()
                    .or_else(|| degree.institution.clone()),
                variations_run: None,
                sample_type: None,
                courses_written: 0,
                requirements_written: 0,
                run_written: false,
                plans_written: 0,
                course_metrics_written: 0,
                conversion_warnings: Vec::new(),
                messages: vec!["institution name matched multiple institutions".to_string()],
            });
        }
    };

    // 2. generation.
    let generation = now_generation();

    // 3. build the plan.
    let (plan, conversion_warnings) =
        build_import_plan(report_text, opts, resolved_unitid, generation)?;
    let program_key = plan
        .program
        .as_ref()
        .map(|p| p.program_key.clone())
        .unwrap_or_default();

    // 4. existence decision.
    let document_hash = plan
        .program
        .as_ref()
        .map(|p| p.document_hash.clone())
        .unwrap_or_default();
    let existing = fetch_existing_program(client, &program_key).await?;
    let decision = decide_existence(existing.as_ref(), &document_hash, plan.is_full, opts);

    let mut outcome = ImportOutcome {
        program_key: program_key.clone(),
        result: decision.result,
        resolved_unitid,
        variant: opts.variant.clone(),
        institution: opts
            .institution
            .clone()
            .or_else(|| degree.institution.clone()),
        variations_run: plan.run.as_ref().and_then(|r| r.variations_run),
        sample_type: plan.run.as_ref().and_then(|r| r.sample_type.clone()),
        courses_written: plan.courses.len(),
        requirements_written: plan.requirements.len(),
        run_written: plan.run.is_some(),
        plans_written: plan.plans.len(),
        course_metrics_written: plan.course_metrics.len(),
        conversion_warnings,
        messages: decision.messages,
    };

    // 5. dry-run: report counts, write nothing.
    if opts.dry_run {
        outcome
            .messages
            .push("dry-run: nothing written".to_string());
        return Ok(outcome);
    }

    // 6. writes — children first, run-parents as commit markers, program last.
    write_import_plan(
        client,
        plan,
        decision.write_program,
        decision.write_minimal_program,
        &mut outcome,
    )
    .await?;

    Ok(outcome)
}

/// Fetch an existing program's idempotency snapshot, if any.
async fn fetch_existing_program(
    client: &DbClient,
    program_key: &str,
) -> DatabaseResult<Option<ExistingProgram>> {
    let filters = QueryFilters::new().eq("program_key", Some(program_key));
    let value = client
        .select(
            tables::PROGRAMS,
            "program_key,document_hash,verified,generation",
            &filters,
            Some(1),
        )
        .await?;
    Ok(value
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| serde_json::from_value(v.clone()).ok()))
}

/// The outcome of the existence/verified decision.
struct ExistenceDecision {
    result: ImportResult,
    /// Whether to upsert the canonical (full) programs row.
    write_program: bool,
    /// Whether to upsert a minimal flagged programs row (non-full, missing).
    write_minimal_program: bool,
    messages: Vec<String>,
}

/// Decide what to do with an existing program. `write_program` gates whether we
/// upsert the canonical programs row; the run/metrics are always allowed even
/// when the program itself is skipped.
fn decide_existence(
    existing: Option<&ExistingProgram>,
    document_hash: &str,
    is_full: bool,
    opts: &ImportOptions,
) -> ExistenceDecision {
    let mut messages = Vec::new();
    let (result, write_program) = match existing {
        None => (ImportResult::Created, is_full),
        Some(_) if opts.skip_existing => {
            messages.push("program exists; --skip-existing set".to_string());
            (ImportResult::Skipped, false)
        }
        Some(e) if e.document_hash == document_hash => {
            messages.push("document unchanged; skipping program".to_string());
            (ImportResult::Skipped, false)
        }
        Some(e) if e.verified && !opts.force => (
            ImportResult::NeedsConfirmation(
                "verified program; pass --force to overwrite".to_string(),
            ),
            false,
        ),
        Some(_) if !opts.force && !opts.replace => (
            ImportResult::NeedsConfirmation(
                "program exists; pass --replace/--force to overwrite".to_string(),
            ),
            false,
        ),
        Some(_) => (ImportResult::Updated, is_full),
    };
    // For a non-full variant whose program is missing, write a minimal flagged
    // program row so the run isn't orphaned.
    let write_minimal_program = !is_full && existing.is_none();
    ExistenceDecision {
        result,
        write_program,
        write_minimal_program,
        messages,
    }
}

/// Upsert the plan's rows: children first, the analysis run after its children,
/// then the programs row as the commit marker. Updates `outcome` counts when a
/// projection/program write is skipped.
async fn write_import_plan(
    client: &DbClient,
    plan: ImportPlan,
    write_program: bool,
    write_minimal_program: bool,
    outcome: &mut ImportOutcome,
) -> DatabaseResult<()> {
    // Projection rows: only on a canonical (full) create/update.
    if plan.is_full
        && matches!(
            outcome.result,
            ImportResult::Created | ImportResult::Updated
        )
    {
        client
            .upsert_batch(
                tables::COURSES,
                plan.courses,
                &["institution_ref", "course_code"],
            )
            .await?;
        client
            .upsert_batch(
                tables::PROGRAM_COURSES,
                plan.program_courses,
                &["program_key", "course_code"],
            )
            .await?;
        client
            .upsert_batch(
                tables::PROGRAM_REQUIREMENTS,
                plan.requirements,
                &["program_key", "req_path"],
            )
            .await?;
    } else {
        // Non-full variant or skipped program: don't touch the projection rows.
        outcome.courses_written = 0;
        outcome.requirements_written = 0;
    }

    // Analysis run + children (always allowed when present): children first.
    if let Some(run) = plan.run {
        if !plan.course_metrics.is_empty() {
            client
                .upsert_batch(
                    tables::ANALYSIS_COURSE_METRICS,
                    plan.course_metrics,
                    &["run_key", "course_code"],
                )
                .await?;
        }
        if !plan.plans.is_empty() {
            client
                .upsert_batch(
                    tables::ANALYSIS_PLANS,
                    plan.plans,
                    &["run_key", "plan_index"],
                )
                .await?;
        }
        client
            .upsert_batch(tables::ANALYSIS_RUNS, vec![run], &["run_key"])
            .await?;
    }

    // Program row (commit marker) LAST. Full canonical write, or a minimal
    // flagged row for a non-full variant whose program does not yet exist.
    if let Some(program_row) = plan.program {
        if write_program {
            client
                .upsert_batch(tables::PROGRAMS, vec![program_row], &["program_key"])
                .await?;
        } else if write_minimal_program {
            client
                .upsert_batch(tables::PROGRAMS, vec![program_row], &["program_key"])
                .await?;
            outcome
                .messages
                .push("created minimal program row for non-full variant".to_string());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> ImportOptions {
        ImportOptions::default()
    }

    /// A minimal full report with `one_of` options, analysis, course metrics, and
    /// `selected_plans` — exercises every builder path.
    fn sample_report() -> String {
        r#"{
            "degree": {
                "name": "Computer Science",
                "degree_type": "BS",
                "system_type": "semester",
                "institution": "Test University",
                "catalog_year": "2024-2025",
                "cip_code": "11.0701",
                "total_credits": 120,
                "tags": ["cs"]
            },
            "requirements": {
                "core": {
                    "type": "all",
                    "category": "major",
                    "courses": ["CS101", "CS201"]
                },
                "track": {
                    "type": "one_of",
                    "name": "Track",
                    "options": [
                        {
                            "id": "ai",
                            "name": "AI Track",
                            "requirements": [
                                {"type": "all", "courses": ["CS300"]},
                                {"type": "select", "count": 1, "from": {"courses": ["CS400", "CS401"]}}
                            ]
                        }
                    ]
                }
            },
            "courses": {
                "CS101": {"name": "Intro", "prefix": "CS", "number": "101", "credit_hours": 4.0},
                "CS201": {"name": "Data Structures", "prefix": "CS", "number": "201", "credit_hours": 4.0, "prerequisites": "CS101"},
                "CS300": {"name": "AI", "prefix": "CS", "number": "300", "credit_hours": 3.0},
                "CS400": {"name": "ML", "prefix": "CS", "number": "400", "credit_hours": 3.0},
                "CS401": {"name": "NLP", "prefix": "CS", "number": "401", "credit_hours": 3.0}
            },
            "analysis": {
                "metrics": {
                    "complexity": {"mean": 12.5},
                    "delay": {"mean": 3.0},
                    "credits": {"mean": 120.0}
                },
                "sample_type": "shuffled",
                "variations_run": 137
            },
            "selected_plans": [
                {"category": "Shortest Path", "course_count": 5, "credits": 120.0,
                 "critical_path": ["CS101", "CS201"], "is_calc_ready": false,
                 "longest_delay": 3, "schedule": [{"term": 1, "courses": ["CS101"], "credits": 4.0}],
                 "terms_required": 8, "total_complexity": 12}
            ]
        }"#
        .to_string()
    }

    #[test]
    fn test_program_key_unitid_tier() {
        let report = sample_report();
        let (plan, _) = build_import_plan(&report, &opts(), Some(167_358), 1).unwrap();
        let key = &plan.program.as_ref().unwrap().program_key;
        assert_eq!(key, "prog:167358|11.0701|2024-2025|BS");
    }

    #[test]
    fn test_program_key_id_catalog_tier() {
        // No unitid resolved, but degree carries an id.
        let report = r#"{
            "degree": {"name": "CS", "degree_type": "BS", "system_type": "semester",
                       "id": "test-cs-2024", "catalog_year": "2024-2025"},
            "requirements": {}, "courses": {}
        }"#;
        let (plan, _) = build_import_plan(report, &opts(), None, 1).unwrap();
        let key = &plan.program.as_ref().unwrap().program_key;
        assert_eq!(key, "prog:test-cs-2024|2024-2025");
    }

    #[test]
    fn test_program_key_fingerprint_tier_and_catalog_year_matters() {
        // No unitid, no id → fingerprint. Different catalog_year ⇒ different key.
        let base = r#"{
            "degree": {"name": "CS", "degree_type": "BS", "system_type": "semester",
                       "institution": "Test University", "catalog_year": "CATYEAR"},
            "requirements": {}, "courses": {}
        }"#;
        let r2024 = base.replace("CATYEAR", "2024-2025");
        let r2025 = base.replace("CATYEAR", "2025-2026");
        let (p2024, _) = build_import_plan(&r2024, &opts(), None, 1).unwrap();
        let (p2025, _) = build_import_plan(&r2025, &opts(), None, 1).unwrap();
        let k24 = &p2024.program.as_ref().unwrap().program_key;
        let k25 = &p2025.program.as_ref().unwrap().program_key;
        assert!(
            k24.starts_with("fp:"),
            "expected fingerprint key, got {k24}"
        );
        assert!(k25.starts_with("fp:"));
        assert_ne!(k24, k25, "different catalog_year must yield different keys");
    }

    #[test]
    fn test_institution_slug() {
        assert_eq!(
            institution_slug("  Northeastern University "),
            "northeastern-university"
        );
        assert_eq!(
            institution_slug("Texas A&M (College Station)"),
            "texas-a-m-college-station"
        );
        assert_eq!(institution_slug("UC—Berkeley!!!"), "uc-berkeley");
    }

    #[test]
    fn test_institution_ref_prefers_unitid() {
        assert_eq!(
            institution_ref(Some(167_358), Some("Test University")),
            "167358"
        );
        assert_eq!(
            institution_ref(None, Some("Test University")),
            "test-university"
        );
        assert_eq!(institution_ref(None, None), "unknown");
    }

    #[test]
    fn test_req_path_for_nested_one_of() {
        let report = sample_report();
        let (plan, _) = build_import_plan(&report, &opts(), Some(1), 1).unwrap();
        // Top-level `track` is a one_of; its option `ai` has two nested reqs.
        let paths: Vec<&str> = plan
            .requirements
            .iter()
            .map(|r| r.req_path.as_str())
            .collect();
        assert!(paths.contains(&"core"));
        assert!(paths.contains(&"track"));
        assert!(paths.contains(&"track#ai#0"), "nested req 0: {paths:?}");
        assert!(paths.contains(&"track#ai#1"), "nested req 1: {paths:?}");

        let nested = plan
            .requirements
            .iter()
            .find(|r| r.req_path == "track#ai#1")
            .unwrap();
        assert_eq!(nested.parent_path.as_deref(), Some("track"));
        assert_eq!(nested.option_id.as_deref(), Some("ai"));
        assert_eq!(nested.option_name.as_deref(), Some("AI Track"));
        assert!(nested.map_key.is_none());
        assert_eq!(nested.req_type, "select");

        // Top-level `core` carries map_key, no parent/option.
        let core = plan
            .requirements
            .iter()
            .find(|r| r.req_path == "core")
            .unwrap();
        assert_eq!(core.map_key.as_deref(), Some("core"));
        assert!(core.parent_path.is_none());
        assert!(core.option_id.is_none());
    }

    #[test]
    fn test_allow_double_count_derivation() {
        use crate::core::models::degree::{Requirement, RequirementConstraints, RequirementType};
        let mk = |exclude_used: Option<bool>| Requirement {
            name: None,
            req_type: RequirementType::All,
            category: None,
            courses: None,
            from: None,
            count: None,
            credits: None,
            credit_range: None,
            constraints: exclude_used.map(|eu| RequirementConstraints {
                exclude_used: Some(eu),
                distinct_subjects: None,
                min_upper_division: None,
                max_from_subject: None,
                max_from_pattern: None,
                max_from_pattern_credits: None,
                grade_minimum: None,
            }),
            options: None,
            tags: None,
        };
        // exclude_used = true → no double counting.
        assert_eq!(allow_double_count(&mk(Some(true)), Some(true)), Some(false));
        // exclude_used = false → double counting allowed.
        assert_eq!(allow_double_count(&mk(Some(false)), None), Some(true));
        // unset → falls back to the program default.
        assert_eq!(allow_double_count(&mk(None), Some(true)), Some(true));
        assert_eq!(allow_double_count(&mk(None), None), None);
    }

    #[test]
    fn test_is_impossible_count_exceeds_pool() {
        use crate::core::models::degree::{FromClause, Requirement, RequirementType};
        let req = Requirement {
            name: None,
            req_type: RequirementType::Select,
            category: None,
            courses: None,
            from: Some(FromClause {
                courses: Some(vec!["CS1".into(), "CS2".into()]),
                pattern: None,
                include: None,
                exclude: None,
                groups: None,
                groups_required: None,
                per_group: None,
            }),
            count: Some(3), // 3 > pool of 2 → impossible
            credits: None,
            credit_range: None,
            constraints: None,
            options: None,
            tags: None,
        };
        assert!(requirement_is_impossible(&req));
    }

    #[test]
    fn test_build_plan_counts() {
        let report = sample_report();
        let (plan, warnings) = build_import_plan(&report, &opts(), Some(1), 7).unwrap();
        assert!(warnings.is_empty(), "unified JSON path emits no warnings");
        assert!(plan.is_full);
        // 5 courses → 5 course rows + 5 junction rows.
        assert_eq!(plan.courses.len(), 5);
        assert_eq!(plan.program_courses.len(), 5);
        // Requirements: core + track + 2 nested = 4.
        assert_eq!(plan.requirements.len(), 4);
        // Analysis present → run + 1 plan. No per-course metrics in this report.
        assert!(plan.run.is_some());
        assert_eq!(plan.plans.len(), 1);
        assert_eq!(plan.course_metrics.len(), 0);
        // Generation propagated.
        assert_eq!(plan.program.as_ref().unwrap().generation, 7);
        // Degree-level means promoted off analysis.metrics.
        let run = plan.run.as_ref().unwrap();
        assert!((run.complexity_mean.unwrap() - 12.5).abs() < f32::EPSILON);
        assert!((run.credits_mean.unwrap() - 120.0).abs() < f32::EPSILON);
        assert_eq!(run.variations_run, Some(137));
        assert_eq!(run.sample_type.as_deref(), Some("shuffled"));
        // Full run: analyzed_document not stored, not trimmed.
        assert!(run.analyzed_document.is_none());
        assert!(!run.trimmed);
        assert_eq!(
            run.analyzed_document_hash,
            plan.program.as_ref().unwrap().document_hash
        );
        // Structured prereq tree built for CS201.
        let cs201 = plan
            .courses
            .iter()
            .find(|c| c.course_code == "CS201")
            .unwrap();
        assert!(cs201.prerequisites.is_some());
    }

    #[test]
    fn test_course_metrics_built_from_report() {
        // A report whose courses carry a `metrics` object → course-metric rows.
        let report = r#"{
            "degree": {"name": "CS", "degree_type": "BS", "system_type": "semester", "institution": "U"},
            "requirements": {"core": {"type": "all", "courses": ["CS101"]}},
            "courses": {
                "CS101": {"name": "Intro", "prefix": "CS", "number": "101", "credit_hours": 4.0,
                          "metrics": {"complexity": {"mean": 1.0}, "centrality": {"mean": 0.0},
                                      "delay": {"mean": 1.0}, "blocking": {"mean": 0.0},
                                      "course_id": "CS101", "plan_count": 42}}
            },
            "analysis": {"metrics": {"complexity": {"mean": 1.0}}, "sample_type": "sequential", "variations_run": 1}
        }"#;
        let (plan, _) = build_import_plan(report, &opts(), Some(1), 1).unwrap();
        assert_eq!(plan.course_metrics.len(), 1);
        let m = &plan.course_metrics[0];
        assert_eq!(m.course_code, "CS101");
        assert_eq!(m.plan_count, Some(42));
        assert!((m.complexity_mean.unwrap() - 1.0).abs() < f32::EPSILON);
        // course_id dropped from the stored metrics blob.
        assert!(m.metrics.as_ref().unwrap().get("course_id").is_none());
    }

    #[test]
    fn test_non_full_variant_sets_analyzed_document_and_trimmed() {
        let report = sample_report();
        let mut o = opts();
        o.variant = "trimmed".to_string();
        let (plan, _) = build_import_plan(&report, &o, Some(1), 1).unwrap();
        assert!(!plan.is_full);
        let run = plan.run.as_ref().unwrap();
        assert!(run.trimmed, "trimmed variant must set trimmed=true");
        assert!(
            run.analyzed_document.is_some(),
            "non-full variant must store the analyzed_document"
        );
        assert_eq!(run.variant, "trimmed");
    }

    #[test]
    fn test_content_hash_is_stable_hex_sha256() {
        // Known sha256 of the empty string.
        assert_eq!(
            content_hash(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        // Deterministic and hex-only.
        let h = content_hash("hello");
        assert_eq!(h.len(), 64);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_no_analysis_block_yields_no_run() {
        let report = r#"{
            "degree": {"name": "CS", "degree_type": "BS", "system_type": "semester", "institution": "U"},
            "requirements": {"core": {"type": "all", "courses": ["CS101"]}},
            "courses": {"CS101": {"name": "Intro", "prefix": "CS", "number": "101", "credit_hours": 4.0}}
        }"#;
        let (plan, _) = build_import_plan(report, &opts(), Some(1), 1).unwrap();
        assert!(plan.run.is_none());
        assert!(plan.course_metrics.is_empty());
        assert!(plan.plans.is_empty());
    }
}
