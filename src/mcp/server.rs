//! MCP server implementation
//!
//! This module provides the MCP server that exposes `NuAnalytics` tools.

use serde::Serialize;
use std::fmt::Write as _;
use std::sync::Arc;

use crate::core::config::DatabaseConfig;
use crate::mcp::tools::{
    analyze, audit, convert, course_detail, match_courses, plan_graph, reference, report, samples,
    shared, trim, validate, AnalyzeDegreeRequest, AuditDegreeRequest, ConvertDegreeRequest,
    FindCoursesMatchingRequest, GetCourseDetailRequest, GetReferenceRequest,
    ListSampleDegreesRequest, RenderDegreeReportRequest, RenderPlanGraphRequest, TrimDegreeRequest,
    ValidateDegreeRequest,
};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
    ServerHandler, ServiceExt,
};

use crate::core::database::{DatabaseError, DbClient};
use crate::core::query::sql::QuerySqlRequest;
use crate::core::query::{
    cip_codes, completions, institutions, lookup, CompletionDemographicsRequest,
    GetDegreeMetricsRequest, GetDegreeRequest, GetLookupCodesRequest, SearchCipCodesRequest,
    SearchDegreesRequest, SearchInstitutionsRequest,
};
use crate::mcp::tools::view::{AnalysisView, StoredAnalysis};
use crate::mcp::tools::{degrees as compare, import, CompareDegreesRequest, ImportDegreeRequest};
// ============================================================================
// MCP Server Implementation
// ============================================================================

/// MCP server that exposes `NuAnalytics` tools via stdio transport.
///
/// Every read tool is always routed; the database tools report the reason recorded at
/// startup when there is no client. Tools that write to the database are routed only
/// with [`Self::with_writes`].
#[derive(Debug, Clone)]
pub struct NuAnalyticsMcpServer {
    // Read indirectly by the rmcp #[tool_handler] macro's generated trait impl
    // for dispatch; rustc's dead_code analysis under release/LTO can't trace
    // that read through the macro expansion, so silence the false positive.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
    /// Database client — `None` if database is not configured or disabled
    db: Option<Arc<DbClient>>,
    /// Why [`Self::db`] is `None`, when it is. Built once at startup alongside the
    /// client, so every tool can report the actual failure instead of guessing between
    /// "not configured" and "not logged in".
    db_unavailable: Option<DbUnavailable>,
}

/// Why the database client could not be built at startup.
#[derive(Debug, Clone)]
struct DbUnavailable {
    /// Backend the client was built for; empty when none was configured.
    endpoint: String,
    /// Machine-readable failure class, from [`DatabaseError::kind`].
    kind: &'static str,
    /// The error as the user would see it.
    detail: String,
    /// Variant-specific remediation, from [`DatabaseError::next_steps`].
    next_steps: Vec<String>,
}

impl DbUnavailable {
    fn from_error(error: &DatabaseError, endpoint: &str) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            kind: error.kind(),
            detail: error.to_string(),
            next_steps: error.next_steps(endpoint),
        }
    }
}

#[tool_router]
impl NuAnalyticsMcpServer {
    /// Create a new MCP server instance without database access
    #[must_use]
    pub fn new() -> Self {
        Self {
            tool_router: Self::read_only_router(),
            db: None,
            db_unavailable: None,
        }
    }

    /// The router without the tools that write — what every server starts with.
    fn read_only_router() -> ToolRouter<Self> {
        let mut router = Self::tool_router();
        for tool in WRITE_TOOLS {
            router.remove_route(tool);
        }
        router
    }

    /// Also serve the tools that write to the database (`nuanalytics mcp --allow-writes`).
    ///
    /// Off unless asked for: a model given a write tool it did not need can still call
    /// it, and the flag is visible in `.mcp.json` where a config key would not be.
    #[must_use]
    pub fn with_writes(mut self) -> Self {
        self.tool_router = Self::tool_router();
        self
    }

    /// Create a new MCP server instance with database access.
    ///
    /// Takes the client by value rather than `Option`: a missing client must come with the
    /// reason it is missing, or the response would blame a cause the code has not
    /// established. Use `with_db_error` when the client could not be built.
    #[must_use]
    pub fn with_db(db: Arc<DbClient>) -> Self {
        Self {
            tool_router: Self::read_only_router(),
            db: Some(db),
            db_unavailable: None,
        }
    }

    /// Create a server whose database is unavailable, carrying the reason so every
    /// DB-backed tool can report it.
    #[must_use]
    fn with_db_error(error: &DatabaseError, endpoint: &str) -> Self {
        Self {
            tool_router: Self::read_only_router(),
            db: None,
            db_unavailable: Some(DbUnavailable::from_error(error, endpoint)),
        }
    }

    // ── Degree tools (no DB required) ──────────────────────────────────────

    /// Reference material: the degree format, its JSON Schema, the database
    #[tool(
        description = "Reference material, offline: topic \"degree-yaml\" is the degree format (optionally one section), \"degree-json-schema\" the JSON Schema a unified degree validates against, \"database\" the tables and columns query_sql can read and how they join (optionally one table in full).",
        annotations(read_only_hint = true)
    )]
    #[allow(clippy::unused_self)]
    fn get_reference(&self, Parameters(req): Parameters<GetReferenceRequest>) -> String {
        reference::execute(&req)
    }

    /// Convert a degree between formats
    #[tool(
        description = "Convert a degree between formats: any source — YAML, unified JSON, or an ai-landscape program or cluster JSON — out as unified JSON (default) or YAML. For a cluster file, omit program to list its programs. output_path writes the result instead of returning it, which is how a stored program is exported.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    fn convert_degree(&self, Parameters(req): Parameters<ConvertDegreeRequest>) -> String {
        let format = req.format.unwrap_or_default();
        let program = req.program;
        let output_path = req.output_path;
        let overwrite = req.overwrite.unwrap_or(false);
        let pretty = req.pretty.unwrap_or(true);
        self.run_yaml_tool("convert_degree", req.source, move |text, _| {
            convert::execute_json(
                text,
                &convert::ConvertOptions {
                    program: program.as_deref(),
                    format,
                    pretty,
                    output_path: output_path.as_deref(),
                    overwrite,
                },
            )
        })
    }

    /// Validate a degree program YAML
    #[tool(
        description = "Check a degree for structural errors. Returns is_valid with errors, warnings and suggestions — findings about the degree, not a failed call. allow_unmatched_patterns=true downgrades patterns that match no listed course (external gen-ed pools such as \"*:100+\") to warnings.",
        annotations(read_only_hint = true)
    )]
    fn validate_degree(&self, Parameters(req): Parameters<ValidateDegreeRequest>) -> String {
        let allow_unmatched_patterns = req.allow_unmatched_patterns.unwrap_or(false);
        let include_hidden_prereq_warnings = req.include_hidden_prereq_warnings.unwrap_or(true);
        self.run_yaml_tool("validate_degree", req.source, move |yaml, _| {
            validate::execute_json(
                yaml,
                allow_unmatched_patterns,
                include_hidden_prereq_warnings,
            )
        })
    }

    /// Audit a degree program YAML
    #[tool(
        description = "Audit a degree beyond validity: upper-level courses missing prerequisites, and deep prerequisite chains, each given as structured branches. chain_threshold sets the depth that counts as deep.",
        annotations(read_only_hint = true)
    )]
    fn audit_degree(&self, Parameters(req): Parameters<AuditDegreeRequest>) -> String {
        let chain_threshold = req.chain_threshold;
        let include_missing_intermediate = req.include_missing_intermediate_prereqs.unwrap_or(true);
        self.run_yaml_tool("audit_degree", req.source, move |yaml, _| {
            audit::execute_json(yaml, chain_threshold, include_missing_intermediate)
        })
    }

    /// Analyze a degree program YAML
    #[tool(
        description = "A degree's plan metrics: complexity, delay and credits as five-number summaries, plus curated selected_plans (shortest, longest, random samples). A stored program is read from its stored run (variant, default full) and nothing is enumerated; any other source, or fresh=true, enumerates plans afresh, bounded by max_plans and analysis_timeout_seconds. is_full_population says whether every plan was seen, and tool_followups suggest a stable cutoff for large degrees.",
        annotations(read_only_hint = true)
    )]
    fn analyze_degree(&self, Parameters(req): Parameters<AnalyzeDegreeRequest>) -> String {
        let include_courses = req
            .include_courses
            .map(|s| crate::core::json::parse_comma_list(&s));
        let include_per_course_metrics = req.include_per_course_metrics.unwrap_or(false);
        let include_placeholder_metrics = req.include_placeholder_metrics.unwrap_or(false);
        let random_seed = req.random_seed;
        let analysis_timeout_seconds = req.analysis_timeout_seconds;
        let max_plans = req.max_plans;
        let target_course = req.target_course;
        let generation = [
            ("max_plans", max_plans.is_some()),
            ("include_courses", include_courses.is_some()),
            ("random_seed", random_seed.is_some()),
            (
                "analysis_timeout_seconds",
                analysis_timeout_seconds.is_some(),
            ),
            // Never stored: where one course lands is computed during enumeration.
            ("target_course", target_course.is_some()),
        ];
        self.run_analysis_tool(
            "analyze_degree",
            req.source,
            &req.run,
            &generation,
            |view| {
                analyze::view_json(
                    view,
                    include_per_course_metrics,
                    include_placeholder_metrics,
                )
            },
            move |yaml, _| {
                analyze::execute_json(
                    yaml,
                    &analyze::AnalyzeOptions {
                        max_plans,
                        include_courses: include_courses.as_deref(),
                        include_per_course_metrics,
                        include_placeholder_metrics,
                        random_seed,
                        analysis_timeout_seconds,
                        target_course: target_course.as_deref(),
                    },
                )
            },
        )
    }

    /// Look up a single course's prerequisites, dependents, and stats
    #[tool(
        description = "Everything about one course in a degree: credits and level, direct and transitive prerequisites, dependents, requirements naming it, equivalents, and — with include_analysis (default) — its metrics and term in each selected plan, from a stored program's stored run unless fresh=true. include_analysis=false skips the analysis and is about ten times faster.",
        annotations(read_only_hint = true)
    )]
    fn get_course_detail(&self, Parameters(req): Parameters<GetCourseDetailRequest>) -> String {
        let course_id = req.course_id.clone();
        let include_analysis = req.include_analysis.unwrap_or(true);
        let max_plans = req.max_plans;
        // Without the analysis nothing is enumerated, so the stored run has nothing to add.
        if !include_analysis {
            if req.run.variant.is_some() || req.run.fresh.is_some() {
                return shared::bad_arguments(
                    "variant and fresh choose the analysis, which include_analysis=false leaves out",
                );
            }
            return self.run_yaml_tool("get_course_detail", req.source, move |yaml, _| {
                course_detail::execute_json(yaml, &course_id, false, max_plans)
            });
        }
        let stored_course = course_id.clone();
        self.run_analysis_tool(
            "get_course_detail",
            req.source,
            &req.run,
            &[("max_plans", max_plans.is_some())],
            |view| course_detail::present_json(&stored_course, view),
            move |yaml, _| course_detail::execute_json(yaml, &course_id, true, max_plans),
        )
    }

    /// Trim a degree program YAML to a single shortest-entry-path variant
    #[tool(
        description = "Collapse a degree's prerequisite alternatives and select lists to one shortest entry path per course, except inside protected subjects (its major_subjects plus keep_all). Returns the trimmed YAML and trimmed_degree, a reference later calls take as degree; output_path also writes it, never over the input.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    fn trim_degree(&self, Parameters(req): Parameters<TrimDegreeRequest>) -> String {
        let keep_all = req.keep_all.unwrap_or_default();
        let include = req.include.unwrap_or_default();
        let output_path = req.output_path;
        let overwrite = req.overwrite.unwrap_or(false);
        // The origin's path, when it has one, so trimming can refuse to overwrite its input.
        self.run_yaml_tool("trim_degree", req.source, move |yaml, origin| {
            trim::execute_json(
                yaml,
                &keep_all,
                &include,
                &trim::TrimOutput {
                    path: output_path.as_deref(),
                    overwrite,
                    source_path: origin.path.as_deref(),
                },
            )
        })
    }

    /// Generate the full HTML degree report (plus optional CSV / JSONL / index artifacts)
    #[tool(
        description = "Render the full HTML degree report — the report `degree analyze` writes. A stored program's comes from its stored run, the page render_stored_report renders, unless fresh=true; any other source is computed afresh. output_dir writes it, with plan CSVs, a JSONL summary and index.csv, and refuses to replace an existing report unless overwrite=true; without it the HTML (a few hundred KB) is returned inline.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    fn render_degree_report(
        &self,
        Parameters(req): Parameters<RenderDegreeReportRequest>,
    ) -> String {
        let max_plans = req.max_plans;
        let include_courses = req
            .include_courses
            .map(|s| crate::core::json::parse_comma_list(&s));
        let output_dir = req.output_dir;
        let write_plan_csvs = req.write_plan_csvs;
        let write_jsonl_summary = req.write_jsonl_summary;
        let write_index_csv = req.write_index_csv;
        let return_html_inline = req.return_html_inline;
        let overwrite = req.overwrite.unwrap_or(false);
        let generation = [
            ("max_plans", max_plans.is_some()),
            ("include_courses", include_courses.is_some()),
        ];
        let out = report::ReportOutput {
            output_dir: output_dir.as_deref(),
            write_plan_csvs,
            write_jsonl_summary,
            write_index_csv,
            return_html_inline,
            overwrite,
        };
        self.run_analysis_tool(
            "render_degree_report",
            req.source,
            &req.run,
            &generation,
            |view| report::present_json(view, &out),
            |yaml, _| report::execute_json(yaml, max_plans, include_courses.as_deref(), &out),
        )
    }

    /// List the bundled sample degree YAMLs
    #[tool(
        description = "The degrees bundled with the server, each with a degree reference such as \"sample:csu\" that every degree tool accepts. include_yaml=true also returns their bodies.",
        annotations(read_only_hint = true)
    )]
    #[allow(clippy::unused_self)]
    fn list_sample_degrees(&self, Parameters(req): Parameters<ListSampleDegreesRequest>) -> String {
        samples::execute_json(req.include_yaml.unwrap_or(false))
    }

    /// Preview which courses match a set of patterns in a degree YAML
    #[tool(
        description = "Preview which of a degree's courses a requirement pattern selects — patterns such as \"CS:300+\", with optional exclusions — before putting the pattern in a requirement.",
        annotations(read_only_hint = true)
    )]
    fn find_courses_matching(
        &self,
        Parameters(req): Parameters<FindCoursesMatchingRequest>,
    ) -> String {
        let patterns = crate::core::json::parse_comma_list(&req.patterns);
        let exclude = req
            .exclude
            .as_deref()
            .map(crate::core::json::parse_comma_list)
            .unwrap_or_default();
        self.run_yaml_tool("find_courses_matching", req.source, move |yaml, _| {
            match_courses::execute_json(yaml, patterns, exclude)
        })
    }

    /// Render the curriculum graph for one selected plan in a single call
    #[tool(
        description = "Render one selected plan of a degree as a curriculum graph — from a stored program's stored run unless fresh=true, afresh for any other source: choose it with plan_category, sample_index or plan_index. output_path writes the HTML to a file (refusing to replace one unless overwrite=true) instead of returning about 100 KB inline.",
        annotations(read_only_hint = false, destructive_hint = false)
    )]
    fn render_plan_graph(&self, Parameters(req): Parameters<RenderPlanGraphRequest>) -> String {
        let plan_category = req.plan_category;
        let sample_index = req.sample_index;
        let plan_index = req.plan_index;
        let format = req.format;
        let max_plans = req.max_plans;
        let include_courses = req
            .include_courses
            .map(|s| crate::core::json::parse_comma_list(&s));
        let dry_run = req.dry_run.unwrap_or(false);
        let output_path = req.output_path;
        let overwrite = req.overwrite.unwrap_or(false);
        let generation = [
            ("max_plans", max_plans.is_some()),
            ("include_courses", include_courses.is_some()),
        ];
        let opts = plan_graph::PlanGraphOptions {
            plan_category: plan_category.as_deref(),
            sample_index,
            plan_index,
            format,
            max_plans,
            include_courses: include_courses.as_deref(),
            dry_run,
        };
        let output = plan_graph::GraphOutput {
            path: output_path.as_deref(),
            overwrite,
        };
        self.run_analysis_tool(
            "render_plan_graph",
            req.source,
            &req.run,
            &generation,
            |view| plan_graph::present_json(view, &opts, output),
            |yaml, _| plan_graph::execute_json(yaml, &opts, output),
        )
    }

    // ── Institution tools ───────────────────────────────────────────────────

    /// Search institutions from the IPEDS database
    #[tool(
        description = "Find IPEDS institutions by name, state, Carnegie class (15=R1, 16=R2), control (1 public, 2 private non-profit), HBCU or tribal status, or minimum size. Returns the unitids other tools take; get_lookup_codes decodes the codes.",
        annotations(read_only_hint = true)
    )]
    fn search_institutions(
        &self,
        Parameters(req): Parameters<SearchInstitutionsRequest>,
    ) -> String {
        self.call_db("search_institutions", |db| async move {
            institutions::execute_json(&db, req).await
        })
    }

    /// Search CIP program codes by title or code prefix
    #[tool(
        description = "Find CIP program codes by title words (query=\"cybersecurity\") or code prefix (prefix=\"11.\" for computing; keep the trailing dot for a family).",
        annotations(read_only_hint = true)
    )]
    fn search_cip_codes(&self, Parameters(req): Parameters<SearchCipCodesRequest>) -> String {
        self.call_db("search_cip_codes", |db| async move {
            cip_codes::execute_json(&db, req).await
        })
    }

    /// Get the full contents of an IPEDS lookup table
    #[tool(
        description = "Decode an IPEDS lookup table: carnegie_class, award_levels, institution_control, institution_level, institution_sector, institution_locale or institution_size.",
        annotations(read_only_hint = true)
    )]
    fn get_lookup_codes(&self, Parameters(req): Parameters<GetLookupCodesRequest>) -> String {
        self.call_db("get_lookup_codes", |db| async move {
            lookup::execute_json(&db, req).await
        })
    }

    // ── Completion demographic tools ────────────────────────────────────────

    /// Completion demographics at the grouping the caller names
    #[tool(
        description = "IPEDS completion demographics (race and gender of graduates) with representation ratios, 1.0 = parity, against the whole graduating population — pooled for total, each school's own otherwise. group_by: total aggregates the matched schools, school ranks them, cip lists one school's CIP codes (needs unitid). cip_prefix=\"11.\" is computing; the year defaults to the latest with data.",
        annotations(read_only_hint = true)
    )]
    fn get_completion_demographics(
        &self,
        Parameters(req): Parameters<CompletionDemographicsRequest>,
    ) -> String {
        self.call_db("get_completion_demographics", |db| async move {
            completions::execute_json(&db, req).await
        })
    }

    /// Search the degree programs stored in the database
    #[tool(
        description = "Find the degree programs stored in the database by unitid, CIP prefix, catalog year, degree type, program kind or discipline. Returns program_keys, which every degree tool accepts as degree.",
        annotations(read_only_hint = true)
    )]
    fn search_degrees(&self, Parameters(req): Parameters<SearchDegreesRequest>) -> String {
        self.call_db("search_degrees", |db| async move {
            crate::core::query::degrees::execute_search_json(&db, req).await
        })
    }

    /// Retrieve a stored degree program by ID or natural key
    #[tool(
        description = "One stored program — by program_key, degree_id, or unitid with cip_code and catalog_year — with the analysis runs stored for it; include_document adds its lossless unified-JSON document. Several matches return summaries to narrow down.",
        annotations(read_only_hint = true)
    )]
    fn get_degree(&self, Parameters(req): Parameters<GetDegreeRequest>) -> String {
        self.call_db("get_degree", |db| async move {
            crate::core::query::degrees::execute_get_json(&db, req).await
        })
    }

    /// Compare degrees side by side
    #[tool(
        description = "Compare degrees side by side: each source is {label?, degree | content | path}. By default a stored program's metrics are its newest stored run of variant and any other source's are enumerated afresh; each entry's metrics_from says which. metrics=\"fresh\" enumerates every degree afresh, \"stored\" reads stored runs only, \"none\" gives identity only. The degrees themselves are not echoed back.",
        annotations(read_only_hint = true)
    )]
    fn compare_degrees(&self, Parameters(req): Parameters<CompareDegreesRequest>) -> String {
        if req.sources.is_empty() {
            return shared::bad_arguments("sources is empty; give at least one degree to compare");
        }
        let loaded = match self.load_sources("compare_degrees", req.sources) {
            Ok(loaded) => loaded,
            Err(e) => return e,
        };
        // The rule the other analysis tools follow: a fresh-run setting is refused where
        // nothing would be enumerated with it, rather than silently ignored.
        if let Some(refusal) = compare::refuse_fresh_settings(req.max_plans, req.metrics, &loaded) {
            return refusal;
        }
        let stored = self.stored_run_lookup(
            req.variant
                .unwrap_or_else(|| crate::core::database::variants::FULL.to_string()),
        );
        let (mode, max_plans) = (req.metrics, req.max_plans);
        guard_panics("compare_degrees", || {
            compare::compare_json(loaded, mode, max_plans, &stored)
        })
    }

    /// Read a stored program's analysis runs
    #[tool(
        description = "A stored program's analysis as imported: the newest run per variant (full, trimmed) with its degree metrics, or the history with latest=false. include_plans adds the selected plans and their schedules, include_course_metrics the per-course figures. analyze_degree reads the same newest run, in its own shape.",
        annotations(read_only_hint = true)
    )]
    fn get_stored_analysis(&self, Parameters(req): Parameters<GetDegreeMetricsRequest>) -> String {
        self.call_db("get_stored_analysis", |db| async move {
            crate::core::query::metrics::execute_json(&db, req).await
        })
    }

    /// Render the HTML report for a stored analysis run
    #[tool(
        description = "Render the HTML report for a stored program's newest analysis run, or one variant's — the page `db report` writes, from the stored run rather than a fresh enumeration, so it is reproducible. output_path writes it, refusing to replace a file unless overwrite=true; without it the HTML (a few hundred KB) is returned inline.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    fn render_stored_report(
        &self,
        Parameters(req): Parameters<report::RenderStoredReportRequest>,
    ) -> String {
        self.call_db("render_stored_report", |db| async move {
            let rendered = match crate::core::query::report_source::render_reference(
                &db,
                &req.degree,
                req.variant.as_deref(),
            )
            .await
            {
                Ok(r) => r,
                Err(payload) => return payload,
            };
            let raw = report::stored_report_json(
                &rendered,
                req.output_path.as_deref(),
                req.overwrite.unwrap_or(false),
            );
            inject_source(
                &raw,
                &SourceInfo {
                    program_key: Some(rendered.program_key),
                    ..SourceInfo::of("stored")
                },
            )
        })
    }

    /// Run a read-only SQL query
    #[tool(
        description = "Run one read-only SELECT or WITH against the database, with inputs in params read as $1 (($1->>'unitid')::int). Read get_reference(topic=\"database\") first for the tables and how they join. Returns up to max_rows rows (200 by default, 2,000 at most) and says when that cap truncated them; statements stop after 30 seconds.",
        annotations(read_only_hint = true)
    )]
    fn query_sql(&self, Parameters(req): Parameters<QuerySqlRequest>) -> String {
        self.call_db("query_sql", |db| async move {
            use crate::core::query::sql;
            match sql::execute_request(&db, &req, sql::RowLimits::AGENT).await {
                Ok(rows) => crate::core::json::to_json_pretty(&rows),
                Err(e) => e.to_json_value().to_string(),
            }
        })
    }

    /// Write a degree into the stored-programs tables
    #[tool(
        description = "Write a degree — a unified JSON, or an analysis report with its run — into the stored-programs tables. dry_run=true previews the row counts. An ambiguous institution returns candidates to pick a unitid from; force or replace overwrite an existing program.",
        annotations(read_only_hint = false, destructive_hint = true)
    )]
    fn import_degree(&self, Parameters(req): Parameters<ImportDegreeRequest>) -> String {
        self.call_db("import_degree", |db| async move {
            import::execute_json(&db, req).await
        })
    }

    /// A stored program's stored run, when the source is a stored program and `fresh` is
    /// not set; `None` when the tool should enumerate afresh.
    ///
    /// A degree pulled from the database means its stored run: its plans were enumerated
    /// at import, and nothing already generated is generated again unless asked.
    /// `generation` pairs each argument that shapes an enumeration with whether it was
    /// given. A stored run's were fixed at import, so they are refused rather than
    /// silently ignored, and a missing run is reported rather than replaced by a fresh one.
    fn stored_analysis(
        &self,
        tool: &'static str,
        source: &shared::DegreeSourceArgs,
        run: &shared::StoredRunArgs,
        generation: &[(&str, bool)],
    ) -> Result<Option<(StoredAnalysis, SourceInfo)>, String> {
        source.check()?;
        let Some(reference) = source.stored_reference() else {
            return match &run.variant {
                Some(_) => Err(shared::bad_arguments(format_args!(
                    "variant selects a stored program's run, and {} is always analyzed afresh",
                    source.describe()
                ))),
                None => Ok(None),
            };
        };
        if run.fresh.unwrap_or(false) {
            return match &run.variant {
                Some(_) => Err(shared::bad_arguments(
                    "variant reads a stored run, and fresh=true enumerates the program's degree instead: pass one or the other",
                )),
                None => Ok(None),
            };
        }
        refuse_fresh_run_settings(reference, generation)?;

        let db = self.get_db(tool)?;
        let reference = reference.to_string();
        let variant = run
            .variant
            .clone()
            .unwrap_or_else(|| crate::core::database::variants::FULL.to_string());
        let (program_key, report) = block_on(move || async move {
            crate::core::query::report_source::load_reference(&db, &reference, Some(&variant)).await
        })
        .map_err(stored_load_refusal)?;
        let stored = StoredAnalysis::new(report);
        let origin = SourceInfo {
            program_key: Some(program_key),
            run: Some(StoredRunInfo::from(stored.run())),
            ..SourceInfo::of("stored")
        };
        Ok(Some((stored, origin)))
    }

    /// Run `present` on a stored program's stored run, when [`Self::stored_analysis`]
    /// finds one; otherwise `fresh` on the degree's text, as [`Self::run_yaml_tool`] does.
    fn run_analysis_tool(
        &self,
        tool: &'static str,
        source: shared::DegreeSourceArgs,
        run: &shared::StoredRunArgs,
        generation: &[(&str, bool)],
        present: impl FnOnce(&AnalysisView<'_>) -> String,
        fresh: impl FnOnce(&str, &SourceInfo) -> String,
    ) -> String {
        match self.stored_analysis(tool, &source, run, generation) {
            Ok(Some((stored, origin))) => {
                let raw = guard_panics(tool, || present(&stored.view()));
                inject_source(&raw, &origin)
            }
            Ok(None) => self.run_yaml_tool(tool, source, fresh),
            Err(refusal) => refusal,
        }
    }

    /// Run a degree tool against the degree its arguments name.
    ///
    /// Loads the degree, invokes `run` on its text and origin, and adds a `source` object to
    /// the response saying where the degree came from — and, for inline content, the
    /// `cache:` handle later calls can pass as `degree` instead of sending it again.
    fn run_yaml_tool<F>(&self, tool: &'static str, args: shared::DegreeSourceArgs, run: F) -> String
    where
        F: FnOnce(&str, &SourceInfo) -> String,
    {
        let (yaml, origin) = match args.into_source().and_then(|s| self.load_source(tool, s)) {
            Ok(loaded) => loaded,
            Err(e) => return e,
        };
        let raw = guard_panics(tool, || run(&yaml, &origin));
        inject_source(&raw, &origin)
    }

    /// Load every comparison entry's degree, stopping at the first that fails.
    fn load_sources(
        &self,
        tool: &'static str,
        sources: Vec<compare::CompareSource>,
    ) -> Result<Vec<compare::LoadedDegree>, String> {
        sources
            .into_iter()
            .map(|entry| {
                let (text, origin) = self.load_source(tool, entry.source.into_source()?)?;
                Ok(compare::LoadedDegree {
                    label: entry.label,
                    text,
                    program_key: origin.program_key.clone(),
                    source: serde_json::to_value(&origin).unwrap_or_default(),
                })
            })
            .collect()
    }

    /// Look up a stored program's newest `variant` run, for `compare_degrees(metrics="stored")`.
    fn stored_run_lookup(
        &self,
        variant: String,
    ) -> impl Fn(&str) -> Result<serde_json::Value, String> + '_ {
        move |key: &str| {
            let Some(db) = self.db.clone() else {
                let why = self
                    .db_unavailable
                    .as_ref()
                    .map_or("this server has no database client", |r| r.detail.as_str());
                return Err(format!(
                    "stored metrics need the database, which is unavailable: {why}"
                ));
            };
            let (key, variant) = (key.to_string(), variant.clone());
            block_on(move || async move {
                crate::core::query::metrics::latest_run_summary(&db, &key, &variant)
                    .await
                    .map_err(|e| e.to_string())?
                    .ok_or_else(|| format!("no stored `{variant}` run for {key}"))
            })
        }
    }

    /// Load a degree's text, and say where it came from.
    ///
    /// Inline content is cached as it passes through, so the response can hand back a
    /// handle: the same degree need not cross the model's context twice.
    fn load_source(
        &self,
        tool: &'static str,
        source: shared::DegreeSource,
    ) -> Result<(String, SourceInfo), String> {
        match source {
            shared::DegreeSource::Content(body) => {
                let handle = crate::mcp::cache::yaml_cache().insert(body.clone());
                Ok((
                    body,
                    SourceInfo {
                        handle: Some(handle),
                        ttl_remaining_seconds: Some(crate::mcp::cache::YAML_CACHE_TTL.as_secs()),
                        ..SourceInfo::of("inline")
                    },
                ))
            }
            shared::DegreeSource::Path(path) => shared::read_degree_file(&path).map(|body| {
                (
                    body,
                    SourceInfo {
                        path: Some(path),
                        ..SourceInfo::of("file")
                    },
                )
            }),
            shared::DegreeSource::Reference(reference) => {
                self.resolve_reference(tool, reference.trim())
            }
        }
    }

    /// Resolve a `degree` reference to a degree's text.
    ///
    /// `cache:<hash>` is an earlier call's handle, `sample:<key>` a bundled sample, and
    /// anything else a stored program — its `program_key` or `degree_id`. Errors are JSON
    /// strings the handler returns as is.
    fn resolve_reference(
        &self,
        tool: &'static str,
        reference: &str,
    ) -> Result<(String, SourceInfo), String> {
        match shared::ReferenceKind::of(reference) {
            shared::ReferenceKind::Cache(handle) => resolve_cache_handle(handle),
            shared::ReferenceKind::Sample(key) => resolve_sample(key, reference),
            shared::ReferenceKind::Stored(stored) => self.resolve_stored(tool, stored),
        }
    }

    /// A stored program's lossless document, read from `programs`.
    fn resolve_stored(
        &self,
        tool: &'static str,
        reference: &str,
    ) -> Result<(String, SourceInfo), String> {
        let db = self.get_db(tool)?;
        let reference_owned = reference.to_string();
        let (program_key, document) = block_on(move || async move {
            crate::core::query::degrees::fetch_document(&db, &reference_owned).await
        })?;
        let text = serde_json::to_string(&document).map_err(crate::core::json::error_json)?;
        Ok((
            text,
            SourceInfo {
                program_key: Some(program_key),
                ..SourceInfo::of("stored")
            },
        ))
    }
}

/// Refuse the settings that shape a fresh run, when a stored program would be read from
/// its stored run instead. `generation` pairs each such argument with whether it was given.
fn refuse_fresh_run_settings(reference: &str, generation: &[(&str, bool)]) -> Result<(), String> {
    let given: Vec<&str> = generation
        .iter()
        .filter_map(|&(name, set)| set.then_some(name))
        .collect();
    if given.is_empty() {
        return Ok(());
    }
    Err(shared::bad_arguments(format_args!(
        "fresh-run settings ({}) do not apply: `{reference}` is a stored program, read from its \
         stored run. Pass fresh=true to enumerate it afresh with them.",
        given.join(", ")
    )))
}

/// The payload for a stored run that could not be read. A program with no run of the
/// variant is `source_not_found` and names `fresh=true`; any other failure is reported as
/// it is, with the program it was for.
fn stored_load_refusal(error: crate::core::query::report_source::ReferenceError) -> String {
    use crate::core::query::report_source::{LoadError, ReferenceError};
    match error {
        ReferenceError::Load {
            error: LoadError::NoRun(message),
            ..
        } => crate::core::json::coded_error(
            crate::core::json::error_code::SOURCE_NOT_FOUND,
            format_args!("{message}. fresh=true enumerates the program's degree afresh instead."),
        )
        .to_string(),
        other => other.into_payload(),
    }
}

/// An earlier call's cached degree, by its `cache:` handle.
fn resolve_cache_handle(reference: &str) -> Result<(String, SourceInfo), String> {
    use crate::core::json::error_code;
    use crate::mcp::cache::CacheLookup;
    // Bound first: the guard is dropped at the end of the statement, not held while the
    // result is built.
    let lookup = crate::mcp::cache::yaml_cache().lookup(reference);
    match lookup {
        CacheLookup::Expired => Err(serde_json::json!({
            "error": "cache handle expired",
            "code": error_code::CACHE_EXPIRED,
            "degree": reference,
            "hint": "Handles last 24 hours. Pass the degree again as content, which caches it afresh.",
        })
        .to_string()),
        CacheLookup::Unknown => Err(serde_json::json!({
            "error": "unknown cache handle",
            "code": error_code::SOURCE_NOT_FOUND,
            "degree": reference,
            "hint": "No such handle in this server process (a typo, or the server restarted). Pass the degree as content to cache it.",
        })
        .to_string()),
        CacheLookup::Hit(body, remaining) => Ok((
            (*body).to_string(),
            SourceInfo {
                handle: Some(reference.to_string()),
                ttl_remaining_seconds: Some(remaining.as_secs()),
                ..SourceInfo::of("cache")
            },
        )),
    }
}

/// A bundled sample, by the key after `sample:`.
fn resolve_sample(key: &str, reference: &str) -> Result<(String, SourceInfo), String> {
    crate::mcp::tools::samples::yaml_for_key(key).map_or_else(
        || {
            Err(serde_json::json!({
                "error": format!("no bundled sample `{key}`"),
                "code": crate::core::json::error_code::SOURCE_NOT_FOUND,
                "degree": reference,
                "hint": "list_sample_degrees gives each sample's reference.",
            })
            .to_string())
        },
        |yaml| {
            Ok((
                yaml.to_string(),
                SourceInfo {
                    sample: Some(key.to_string()),
                    ..SourceInfo::of("sample")
                },
            ))
        },
    )
}

/// Run a tool body inside `catch_unwind` so an unexpected panic in the
/// analysis pipeline surfaces as a structured JSON error instead of taking
/// down the MCP server. The tool body is treated as unwind-safe — every
/// MCP tool body is pure aside from process-wide `Arc`-shared caches.
fn guard_panics<F>(tool: &'static str, f: F) -> String
where
    F: FnOnce() -> String,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(s) => s,
        Err(payload) => {
            let message = panic_payload_to_string(&*payload);
            serde_json::json!({
                "success": false,
                "code": "internal_error",
                "error": format!("{tool} panicked: {message}"),
                "hint": "This is a bug in the MCP server — please report the YAML that triggered it. The server is still running.",
            })
            .to_string()
        }
    }
}

/// Pull a human-readable message out of a panic payload. Panics started by
/// `panic!("literal")` carry a `&'static str`; panics with formatting carry
/// a `String`; anything else falls back to a generic placeholder.
fn panic_payload_to_string(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&'static str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "non-string panic payload".to_string())
}

/// Where a tool's degree came from, reported as the response's `source` object.
#[derive(Debug, Clone, Serialize)]
struct SourceInfo {
    /// `inline`, `file`, `cache`, `sample` or `stored`.
    kind: &'static str,
    /// A `cache:` handle later calls can pass as `degree`.
    #[serde(skip_serializing_if = "Option::is_none")]
    handle: Option<String>,
    /// Seconds the handle has left.
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl_remaining_seconds: Option<u64>,
    /// The stored program read, for a `stored` source.
    #[serde(skip_serializing_if = "Option::is_none")]
    program_key: Option<String>,
    /// The bundled sample read, for a `sample` source.
    #[serde(skip_serializing_if = "Option::is_none")]
    sample: Option<String>,
    /// The file read, for a `file` source.
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    /// The stored run read, when a stored program's analysis was read rather than run.
    #[serde(skip_serializing_if = "Option::is_none")]
    run: Option<StoredRunInfo>,
}

impl SourceInfo {
    const fn of(kind: &'static str) -> Self {
        Self {
            kind,
            handle: None,
            ttl_remaining_seconds: None,
            program_key: None,
            sample: None,
            path: None,
            run: None,
        }
    }
}

/// Which stored run a response was read from.
#[derive(Debug, Clone, Serialize)]
struct StoredRunInfo {
    run_key: String,
    variant: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    analyzer_version: Option<String>,
}

impl From<&crate::core::query::report_source::StoredRun> for StoredRunInfo {
    fn from(run: &crate::core::query::report_source::StoredRun) -> Self {
        Self {
            run_key: run.run_key.clone(),
            variant: run.variant.clone(),
            created_at: run.created_at.clone(),
            analyzer_version: run.analyzer_version.clone(),
        }
    }
}

/// Add `source` to a tool's JSON response.
///
/// One injection point rather than threading the origin through every tool's
/// `execute()`. A response that is not a JSON object is returned unchanged, so a malformed
/// one — a separate bug — surfaces as it was.
fn inject_source(raw: &str, origin: &SourceInfo) -> String {
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(raw) else {
        return raw.to_string();
    };
    let serde_json::Value::Object(map) = &mut value else {
        return raw.to_string();
    };
    let Ok(source) = serde_json::to_value(origin) else {
        return raw.to_string();
    };
    map.insert("source".to_string(), source);
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| raw.to_string())
}

impl NuAnalyticsMcpServer {
    /// A refusal naming the arguments `request` passes that its tool does not take.
    ///
    /// Serde ignores a field it does not know, so without this a misspelt filter
    /// (`school` for `unitid`) returns unfiltered results that look like an answer. Checked
    /// against the tool's own input schema, which includes the flattened degree source.
    fn refuse_unknown_arguments(
        &self,
        request: &rmcp::model::CallToolRequestParams,
    ) -> Option<String> {
        let tool = self.tool_router.get(&request.name)?;
        let accepted = tool.input_schema.get("properties")?.as_object()?;
        let mut unknown: Vec<&str> = request
            .arguments
            .as_ref()?
            .keys()
            .map(String::as_str)
            .filter(|k| !accepted.contains_key(*k))
            .collect();
        if unknown.is_empty() {
            return None;
        }
        unknown.sort_unstable();
        let mut payload = crate::core::json::coded_error(
            crate::core::json::error_code::BAD_ARGUMENTS,
            format_args!("{} does not take {}", request.name, unknown.join(", ")),
        );
        payload["accepted"] = accepted.keys().cloned().collect::<Vec<_>>().into();
        Some(payload.to_string())
    }
}

/// Database access helpers.
impl NuAnalyticsMcpServer {
    /// Return the DB client, or a JSON error response naming `tool_name` and the reason
    /// recorded at startup when it is unavailable.
    fn get_db(&self, tool_name: &'static str) -> Result<Arc<DbClient>, String> {
        self.db
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| db_not_configured_response(tool_name, self.db_unavailable.as_ref()))
    }

    /// Fetch the DB client, run an async tool function, and return JSON.
    ///
    /// Returns [`Self::get_db`]'s error JSON when the database is unavailable.
    fn call_db<F, Fut>(&self, tool: &'static str, f: F) -> String
    where
        F: FnOnce(Arc<DbClient>) -> Fut,
        Fut: std::future::Future<Output = String>,
    {
        match self.get_db(tool) {
            Ok(db) => block_on(move || f(db)),
            Err(e) => e,
        }
    }
}

/// Run an async database operation to completion from a synchronous MCP tool handler.
fn block_on<T, F, Fut>(f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f()))
}

/// Return an error JSON naming why the database is unavailable.
///
/// `DbClient::from_config` runs once at startup, so this is reported for the whole
/// process lifetime. It therefore names the failure recorded then rather than guessing
/// one: a guess of "not signed in" would report an outage of a working backend as user
/// error for as long as the server ran.
///
/// `reason` is `None` only for [`NuAnalyticsMcpServer::new`] — a server built without
/// database support. [`NuAnalyticsMcpServer::with_db`] always has a client, and
/// [`NuAnalyticsMcpServer::with_db_error`] always has a reason.
fn db_not_configured_response(tool: &str, reason: Option<&DbUnavailable>) -> String {
    let Some(reason) = reason else {
        return serde_json::json!({
            "error": "Database not available: this MCP server was started without a database client",
            "code": crate::core::json::error_code::DB_UNAVAILABLE,
            "tool": tool,
            "reason": "no_client",
            "detail": "this MCP server was started without a database client",
            "next_steps": ["Restart the MCP server with a configured [database] section."],
        })
        .to_string();
    };

    let backend = crate::core::config::endpoint_label(&reason.endpoint);
    let mut next_steps = reason.next_steps.clone();
    // The client is built once at startup, so fixing config or logging in is not enough
    // on its own — say so rather than leaving the model to guess.
    next_steps.push(
        "The database client is created at MCP server startup; restart the server after \
         fixing the above."
            .to_string(),
    );
    serde_json::json!({
        "error": format!("Database not available: {}", reason.detail),
        "code": crate::core::json::error_code::DB_UNAVAILABLE,
        "tool": tool,
        "backend": backend,
        "reason": reason.kind,
        "detail": reason.detail,
        "next_steps": next_steps,
    })
    .to_string()
}

impl Default for NuAnalyticsMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Tools that change the database, served only with `--allow-writes`.
const WRITE_TOOLS: &[&str] = &["import_degree"];

/// The tools a server would serve, with their descriptions, for `mcp --list-tools`.
#[must_use]
pub fn tool_list(allow_writes: bool) -> Vec<(String, String)> {
    let server = NuAnalyticsMcpServer::new();
    let server = if allow_writes {
        server.with_writes()
    } else {
        server
    };
    let mut tools: Vec<(String, String)> = server
        .tool_router
        .list_all()
        .into_iter()
        .map(|t| {
            (
                t.name.into_owned(),
                t.description
                    .map(std::borrow::Cow::into_owned)
                    .unwrap_or_default(),
            )
        })
        .collect();
    tools.sort();
    tools
}

/// One group of tools, for the server instructions.
struct Capability {
    /// Short name of the group.
    group: &'static str,
    /// What the group is for, in a clause.
    summary: &'static str,
    /// Every tool in it.
    tools: &'static [&'static str],
    /// Whether the group needs the database.
    needs_db: bool,
}

/// Every tool the server serves, grouped by what it is for.
///
/// The instructions are rendered from this, and a test checks it names exactly the tools
/// the router serves — so a tool added, renamed or removed cannot leave the instructions
/// describing a server that no longer exists.
const CAPABILITIES: &[Capability] = &[
    Capability {
        group: "Reference",
        summary: "the degree format, its JSON Schema, and the database's tables",
        tools: &["get_reference"],
        needs_db: false,
    },
    Capability {
        group: "Check",
        summary: "find errors and quality problems in a degree, or inspect one course or pattern",
        tools: &[
            "validate_degree",
            "audit_degree",
            "find_courses_matching",
            "get_course_detail",
        ],
        needs_db: false,
    },
    Capability {
        group: "Transform",
        summary: "convert a degree between YAML and JSON, or trim it to one entry path",
        tools: &["convert_degree", "trim_degree"],
        needs_db: false,
    },
    Capability {
        group: "Analyze",
        summary: "plan metrics as JSON, a report or a graph: a stored program's from its stored run, anything else enumerated afresh",
        tools: &[
            "analyze_degree",
            "render_degree_report",
            "render_plan_graph",
        ],
        needs_db: false,
    },
    Capability {
        group: "Samples",
        summary: "degrees bundled with the server",
        tools: &["list_sample_degrees"],
        needs_db: false,
    },
    Capability {
        group: "IPEDS",
        summary: "institutions, CIP and lookup codes, and completion demographics",
        tools: &[
            "search_institutions",
            "search_cip_codes",
            "get_lookup_codes",
            "get_completion_demographics",
        ],
        needs_db: true,
    },
    Capability {
        group: "Stored programs",
        summary: "the degree programs in the database and the analysis stored with them",
        tools: &[
            "search_degrees",
            "get_degree",
            "get_stored_analysis",
            "render_stored_report",
            "compare_degrees",
        ],
        needs_db: true,
    },
    Capability {
        group: "SQL",
        summary: "ad-hoc read-only queries, for what the typed tools do not answer",
        tools: &["query_sql"],
        needs_db: true,
    },
    Capability {
        group: "Writes",
        summary: "add a program and its analysis to the database (only with --allow-writes)",
        tools: &["import_degree"],
        needs_db: true,
    },
];

/// The server instructions: a capability map, not a script.
///
/// Every client puts these in the model's context on every turn, so they stay short and
/// say what the server can do and how to pass things to it; the order to do things in is
/// the model's call. `db_status` is one line on whether the database tools will work.
fn render_instructions(db_status: &str, serves: &dyn Fn(&str) -> bool) -> String {
    let mut out = String::from(
        "NuAnalytics: curricular analytics over degree programs, and IPEDS completion data.\n\n\
         A degree is passed as exactly one of degree (\"sample:<key>\", a \"cache:\" \
         handle, or a stored program_key), content (inline YAML or JSON) or path (a file); \
         the response's source says which, with a handle for reuse.\n\
         A stored program's analysis is its stored run, read and never re-run unless a tool \
         is given fresh=true; files, content and samples are always enumerated afresh.\n\
         A failure is a protocol error whose JSON says {error: {code, message, next_steps}}.\n\
         \nTools:\n",
    );
    for cap in CAPABILITIES {
        let tools: Vec<&str> = cap.tools.iter().copied().filter(|t| serves(t)).collect();
        if tools.is_empty() {
            continue;
        }
        let db = if cap.needs_db { " [database]" } else { "" };
        let _ = writeln!(
            out,
            "- {}{db}: {} — {}",
            cap.group,
            cap.summary,
            tools.join(", ")
        );
    }
    out.push('\n');
    out.push_str(db_status);
    out
}

// `router = self.tool_router`, not the macro's default `Self::tool_router()`: the default
// builds the full router on every call, so `tools/list` would advertise the write tools a
// read-only server has removed from its own router.
#[tool_handler(router = self.tool_router)]
impl ServerHandler for NuAnalyticsMcpServer {
    /// Dispatch as the generated handler would, then turn a failed tool's JSON into a
    /// protocol error with one envelope (see [`crate::mcp::envelope`]).
    async fn call_tool(
        &self,
        request: rmcp::model::CallToolRequestParams,
        context: rmcp::service::RequestContext<rmcp::RoleServer>,
    ) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
        if let Some(refusal) = self.refuse_unknown_arguments(&request) {
            return Ok(crate::mcp::envelope::finish(
                rmcp::model::CallToolResult::success(vec![rmcp::model::Content::text(refusal)]),
            ));
        }
        let call = rmcp::handler::server::tool::ToolCallContext::new(self, request, context);
        self.tool_router
            .call(call)
            .await
            .map(crate::mcp::envelope::finish)
    }

    fn get_info(&self) -> ServerInfo {
        let db_status = match (&self.db, &self.db_unavailable) {
            (Some(_), _) => "Database: connected.".to_string(),
            (None, Some(reason)) => format!(
                "Database: unavailable ({}: {}). Every database tool returns this with the \
                 steps to fix it; the server must be restarted afterwards.",
                reason.kind, reason.detail
            ),
            (None, None) => "Database: not configured for this server.".to_string(),
        };

        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            render_instructions(&db_status, &|tool| self.tool_router.has_route(tool)),
        )
    }
}

// ============================================================================
// Server Entry Points
// ============================================================================

/// Run the MCP server (async)
///
/// # Errors
///
/// Returns an error if the server fails to start or encounters a fatal transport error.
pub async fn run_server(
    db_config: &DatabaseConfig,
    allow_writes: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Starting NuAnalytics MCP server...");

    let server = {
        match DbClient::from_config(db_config).await {
            Ok(client) => {
                eprintln!("Database client initialized for {}.", db_config.endpoint);
                NuAnalyticsMcpServer::with_db(Arc::new(client))
            }
            Err(e) => {
                let backend = db_config.endpoint_label();
                eprintln!("Database unavailable for {backend}: {e}");
                for step in e.next_steps(&db_config.endpoint) {
                    eprintln!("  {step}");
                }
                eprintln!(
                    "  DB-backed MCP tools will report this reason until the server is \
                     restarted."
                );
                NuAnalyticsMcpServer::with_db_error(&e, &db_config.endpoint)
            }
        }
    };

    let server = if allow_writes {
        server.with_writes()
    } else {
        server
    };
    let service = server
        .serve(stdio())
        .await
        .map_err(|e| format!("Failed to start MCP server: {e}"))?;

    eprintln!("NuAnalytics MCP server running. Waiting for requests...");

    service.waiting().await?;

    eprintln!("NuAnalytics MCP server shut down.");
    Ok(())
}

/// Synchronous wrapper to run the MCP server
///
/// # Errors
///
/// Returns an error string if the tokio runtime cannot be created or the server fails.
pub fn run(db_config: &DatabaseConfig, allow_writes: bool) -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| format!("Failed to create tokio runtime: {e}"))?;

    rt.block_on(run_server(db_config, allow_writes))
        .map_err(|e| format!("MCP server error: {e}"))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_guard_panics_returns_value_on_normal_completion() {
        let result = guard_panics("noop", || "{\"success\":true}".to_string());
        assert_eq!(result, "{\"success\":true}");
    }

    #[test]
    fn test_guard_panics_translates_string_panic_into_json_error() {
        let result = guard_panics("explode", || panic!("intentional test panic"));
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["success"].as_bool(), Some(false));
        let error = parsed["error"].as_str().expect("error string");
        assert!(
            error.contains("explode panicked"),
            "error must name the tool: got {error:?}"
        );
        assert!(
            error.contains("intentional test panic"),
            "error must include the panic message: got {error:?}"
        );
        assert!(parsed["hint"].is_string(), "hint must be populated");
    }

    #[test]
    fn test_guard_panics_extracts_formatted_string_panic_message() {
        // panic! with formatting allocates a String payload — exercises the
        // downcast_ref::<String>() branch in panic_payload_to_string.
        let result = guard_panics("fmt", || panic!("dynamic {} value", 42));
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        let error = parsed["error"].as_str().unwrap();
        assert!(
            error.contains("dynamic 42 value"),
            "formatted panic message must reach the response: got {error:?}"
        );
    }

    #[test]
    fn test_guard_panics_handles_non_string_payload() {
        let result = guard_panics("weird", || {
            std::panic::panic_any(42_i32);
        });
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["success"].as_bool(), Some(false));
        assert!(parsed["error"]
            .as_str()
            .unwrap()
            .contains("non-string panic payload"));
    }

    #[test]
    fn inject_source_adds_where_the_degree_came_from() {
        let raw = r#"{"success": true, "is_valid": true}"#;
        let origin = SourceInfo {
            handle: Some("cache:abc123".to_string()),
            ttl_remaining_seconds: Some(12_345),
            ..SourceInfo::of("inline")
        };
        let parsed: serde_json::Value =
            serde_json::from_str(&inject_source(raw, &origin)).expect("valid JSON");
        assert_eq!(parsed["source"]["kind"], "inline");
        assert_eq!(parsed["source"]["handle"], "cache:abc123");
        assert_eq!(parsed["source"]["ttl_remaining_seconds"], 12_345);
        assert!(
            parsed["source"].get("program_key").is_none(),
            "absent fields are omitted"
        );
        // Existing fields preserved.
        assert_eq!(parsed["success"], true);
        assert_eq!(parsed["is_valid"], true);
    }

    #[test]
    fn inject_source_leaves_a_non_object_response_as_it_was() {
        // Tool bodies always emit objects; a parser bug that does not must not be
        // swallowed by the injection.
        let raw = r#"["just", "an", "array"]"#;
        assert_eq!(inject_source(raw, &SourceInfo::of("file")), raw);
    }

    #[test]
    fn a_sample_reference_resolves_without_a_database() {
        let server = NuAnalyticsMcpServer::new();
        let (yaml, origin) = server
            .resolve_reference("validate_degree", "sample:csu")
            .expect("bundled sample resolves");
        assert!(yaml.contains("degree:"), "sample body");
        assert_eq!(
            (origin.kind, origin.sample.as_deref()),
            ("sample", Some("csu"))
        );
        let unknown = server
            .resolve_reference("validate_degree", "sample:nope")
            .expect_err("unknown sample");
        assert!(unknown.contains("list_sample_degrees"), "{unknown}");
    }

    #[test]
    fn inline_content_is_cached_and_its_handle_resolves_back_to_it() {
        let server = NuAnalyticsMcpServer::new();
        let body = "degree:\n  id: handle-round-trip\n".to_string();
        let (_, origin) = server
            .load_source(
                "validate_degree",
                shared::DegreeSource::Content(body.clone()),
            )
            .expect("inline loads");
        let handle = origin.handle.expect("inline content gets a handle");
        let (resolved, again) = server
            .resolve_reference("validate_degree", &handle)
            .expect("the handle resolves");
        assert_eq!(resolved, body);
        assert_eq!(again.kind, "cache");
    }

    /// The envelope code a failed call would carry.
    fn envelope_code(raw: &str) -> String {
        let done = crate::mcp::envelope::finish(rmcp::model::CallToolResult::success(vec![
            rmcp::model::Content::text(raw),
        ]));
        assert_eq!(done.is_error, Some(true), "{raw}");
        let v: serde_json::Value =
            serde_json::from_str(&done.content[0].as_text().expect("text").text).expect("json");
        v["error"]["code"].as_str().unwrap_or_default().to_string()
    }

    #[test]
    fn an_unresolvable_source_names_why_in_a_stable_code() {
        let server = NuAnalyticsMcpServer::new();
        let unknown = server
            .resolve_reference("validate_degree", "cache:0000000000000000")
            .expect_err("no such handle");
        assert_eq!(envelope_code(&unknown), "source_not_found");
        let stored = server
            .resolve_reference("validate_degree", "prog:1|x")
            .expect_err("no database client");
        assert_eq!(envelope_code(&stored), "db_unavailable", "{stored}");
        let missing = server
            .load_source(
                "validate_degree",
                shared::DegreeSource::Path("/nonexistent/nuanalytics/x.yaml".into()),
            )
            .expect_err("no such file");
        assert_eq!(envelope_code(&missing), "source_not_found", "{missing}");
    }

    #[test]
    fn a_path_source_reads_the_file_and_says_so() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("d.yaml");
        std::fs::write(&file, "degree:\n  id: from-file\n").expect("write");
        let path = file.to_string_lossy().into_owned();
        let (body, origin) = NuAnalyticsMcpServer::new()
            .load_source("validate_degree", shared::DegreeSource::Path(path.clone()))
            .expect("reads");
        assert_eq!(body, "degree:\n  id: from-file\n");
        assert_eq!(
            (
                origin.kind,
                origin.path.as_deref(),
                origin.handle.as_deref()
            ),
            ("file", Some(path.as_str()), None)
        );
    }

    #[test]
    fn a_reference_is_resolved_without_its_surrounding_whitespace() {
        let (_, origin) = NuAnalyticsMcpServer::new()
            .load_source(
                "validate_degree",
                shared::DegreeSource::Reference("  sample:csu\n".into()),
            )
            .expect("resolves as the sample, not as a stored program");
        assert_eq!(origin.kind, "sample");
    }

    fn call(name: &str, arguments: &serde_json::Value) -> rmcp::model::CallToolRequestParams {
        serde_json::from_value(serde_json::json!({ "name": name, "arguments": arguments }))
            .expect("request decodes")
    }

    #[test]
    fn an_argument_the_tool_does_not_take_is_refused_by_name() {
        let server = NuAnalyticsMcpServer::new();
        let refusal = server
            .refuse_unknown_arguments(&call(
                "search_degrees",
                &serde_json::json!({"school": 141_574, "limit": 5, "zzz": 1}),
            ))
            .expect("school and zzz are not search_degrees arguments");
        let v: serde_json::Value = serde_json::from_str(&refusal).unwrap();
        assert_eq!(v["code"], "bad_arguments");
        assert_eq!(v["error"], "search_degrees does not take school, zzz");
        assert!(v["accepted"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a == "unitid"));

        for fine in [
            call(
                "search_degrees",
                &serde_json::json!({"unitid": 1, "name": "cs"}),
            ),
            call(
                "validate_degree",
                &serde_json::json!({"degree": "sample:csu"}),
            ),
            call("list_sample_degrees", &serde_json::json!({})),
        ] {
            assert_eq!(
                server.refuse_unknown_arguments(&fine),
                None,
                "{}",
                fine.name
            );
        }
    }

    #[test]
    fn compare_degrees_refuses_no_sources_and_compares_samples_without_a_database() {
        let server = NuAnalyticsMcpServer::new();
        let req =
            |v| Parameters(serde_json::from_value::<CompareDegreesRequest>(v).expect("decodes"));
        let empty: serde_json::Value =
            serde_json::from_str(&server.compare_degrees(req(serde_json::json!({"sources": []}))))
                .unwrap();
        assert_eq!(empty["code"], "bad_arguments");

        let out: serde_json::Value =
            serde_json::from_str(&server.compare_degrees(req(serde_json::json!({
                "sources": [{"label": "CSU", "degree": "sample:csu"}, {"degree": "sample:uhm"}],
                "metrics": "stored"
            }))))
            .unwrap();
        assert_eq!(out["count"], 2);
        assert_eq!(out["degrees"][0]["label"], "CSU");
        assert_eq!(out["degrees"][0]["source"]["kind"], "sample");
        assert!(
            out["degrees"][1]["metrics"]["error"]
                .as_str()
                .is_some_and(|e| e.contains("stored program")),
            "{out}"
        );
    }

    /// One tool handler, taking its request as JSON.
    type ToolCall<'s> = Box<dyn Fn(serde_json::Value) -> String + 's>;

    /// A stored program means its stored run. Settings that shape a fresh run are refused
    /// by name unless fresh=true — before anything is read, so no database is needed here.
    #[test]
    fn a_stored_program_refuses_fresh_run_settings_in_every_analysis_tool() {
        let server = NuAnalyticsMcpServer::new();
        let stored = "prog:1|11.0701|2025-2026|BS|test";
        let calls: [(&str, ToolCall<'_>); 4] = [
            (
                "analyze_degree",
                Box::new(|v| server.analyze_degree(Parameters(serde_json::from_value(v).unwrap()))),
            ),
            (
                "render_degree_report",
                Box::new(|v| {
                    server.render_degree_report(Parameters(serde_json::from_value(v).unwrap()))
                }),
            ),
            (
                "render_plan_graph",
                Box::new(|v| {
                    server.render_plan_graph(Parameters(serde_json::from_value(v).unwrap()))
                }),
            ),
            (
                "get_course_detail",
                Box::new(|v| {
                    server.get_course_detail(Parameters(serde_json::from_value(v).unwrap()))
                }),
            ),
        ];
        for (tool, call) in &calls {
            let args = |extra: serde_json::Value| {
                let mut v = serde_json::json!({
                    "degree": stored, "plan_category": "shortest", "course_id": "CS101"
                });
                v.as_object_mut()
                    .unwrap()
                    .extend(extra.as_object().unwrap().clone());
                if *tool != "render_plan_graph" {
                    v.as_object_mut().unwrap().remove("plan_category");
                }
                if *tool != "get_course_detail" {
                    v.as_object_mut().unwrap().remove("course_id");
                }
                v
            };
            let out: serde_json::Value =
                serde_json::from_str(&call(args(serde_json::json!({"max_plans": 50})))).unwrap();
            assert_eq!(out["code"], "bad_arguments", "{tool}: {out}");
            let message = out["error"].as_str().unwrap_or_default();
            assert!(
                message.contains("max_plans") && message.contains("fresh=true"),
                "{tool}: {message}"
            );

            // No database: the stored run cannot be read, and is not replaced by a fresh one.
            let unread: serde_json::Value =
                serde_json::from_str(&call(args(serde_json::json!({})))).unwrap();
            assert_eq!(unread["code"], "db_unavailable", "{tool}: {unread}");

            // variant chooses a stored run: not for other sources, and not with fresh=true.
            for extra in [
                serde_json::json!({"degree": "sample:csu", "variant": "full"}),
                serde_json::json!({"fresh": true, "variant": "trimmed"}),
            ] {
                let out: serde_json::Value = serde_json::from_str(&call(args(extra))).unwrap();
                assert_eq!(out["code"], "bad_arguments", "{tool}: {out}");
            }

            // A blank degree is refused as blank, not described as a stored program.
            let blank: serde_json::Value = serde_json::from_str(&call(args(
                serde_json::json!({"degree": "  ", "variant": "full"}),
            )))
            .unwrap();
            let message = blank["error"].as_str().unwrap_or_default();
            assert!(
                message.contains("blank") && !message.contains("stored program's run"),
                "{tool}: {blank}"
            );
        }

        // target_course is never stored, so it too needs fresh=true.
        let out: serde_json::Value = serde_json::from_str(
            &server.analyze_degree(Parameters(
                serde_json::from_value(
                    serde_json::json!({"degree": stored, "target_course": "CS101"}),
                )
                .unwrap(),
            )),
        )
        .unwrap();
        assert!(
            out["error"]
                .as_str()
                .is_some_and(|m| m.contains("target_course")),
            "{out}"
        );
    }

    /// `fresh=true` lifts the refusal: the settings are taken and the program's document is
    /// what is read next (unavailable here, not refused). Other sources take them freely.
    #[test]
    fn fresh_true_lifts_the_refusal_and_other_sources_take_run_settings() {
        let server = NuAnalyticsMcpServer::new();
        let analyze = |v: serde_json::Value| -> serde_json::Value {
            serde_json::from_str(
                &server.analyze_degree(Parameters(serde_json::from_value(v).unwrap())),
            )
            .unwrap()
        };
        let lifted = analyze(serde_json::json!({
            "degree": "prog:1|x", "fresh": true, "max_plans": 5, "random_seed": 7
        }));
        assert_eq!(lifted["code"], "db_unavailable", "{lifted}");

        let refused = analyze(serde_json::json!({
            "degree": "prog:1|x", "max_plans": 5, "random_seed": 7
        }));
        assert!(
            refused["error"]
                .as_str()
                .is_some_and(|m| m.contains("max_plans, random_seed")),
            "every refused setting is named: {refused}"
        );

        for fresh in [None, Some(true)] {
            let sample = analyze(serde_json::json!({
                "degree": "sample:csu", "max_plans": 5, "fresh": fresh
            }));
            assert_eq!(sample["success"], true, "{sample}");
            assert!(sample["plans_analyzed"].as_u64().is_some_and(|n| n <= 5));
        }

        let detail: serde_json::Value = serde_json::from_str(
            &server.get_course_detail(Parameters(
                serde_json::from_value(serde_json::json!({
                    "degree": "sample:csu", "course_id": "CS165",
                    "include_analysis": false, "fresh": true
                }))
                .unwrap(),
            )),
        )
        .unwrap();
        assert_eq!(detail["code"], "bad_arguments", "{detail}");
    }

    /// A program with no run of the variant is `source_not_found` and names `fresh=true`;
    /// a failed read is reported as itself, with the program it was for.
    #[test]
    fn a_missing_stored_run_is_source_not_found_and_a_failed_read_is_not() {
        use crate::core::query::report_source::{LoadError, ReferenceError};
        let json = |s: String| serde_json::from_str::<serde_json::Value>(&s).unwrap();
        let load = |error| ReferenceError::Load {
            program_key: "prog:1".to_string(),
            error,
        };

        let missing = json(stored_load_refusal(load(LoadError::NoRun(
            "no `trimmed` analysis run stored for prog:1".to_string(),
        ))));
        assert_eq!(missing["code"], "source_not_found");
        assert!(missing["error"]
            .as_str()
            .is_some_and(|m| m.contains("trimmed") && m.contains("fresh=true")));

        let failed = json(stored_load_refusal(load(LoadError::Failed(
            "timed out".to_string(),
        ))));
        assert_ne!(failed["code"], "source_not_found");
        assert_eq!(failed["program_key"], "prog:1");
        assert_eq!(failed["error"], "timed out");
        assert!(
            failed.get("kind").is_none(),
            "a code nothing established: {failed}"
        );

        // A backend failure keeps its kind, which the envelope makes the error code
        // (`envelope::tests` pins that step).
        let unreachable = json(stored_load_refusal(load(LoadError::Backend {
            op: "reading the stored runs of prog:1".to_string(),
            error: crate::core::database::DatabaseError::ConnectionError("timed out".to_string()),
        })));
        assert_eq!(unreachable["kind"], "unreachable", "{unreachable}");
        assert_eq!(unreachable["program_key"], "prog:1");
        assert!(unreachable["error"]
            .as_str()
            .is_some_and(|m| m.starts_with("reading the stored runs of prog:1: ")));

        let unresolved = r#"{"error":"no stored program","code":"source_not_found"}"#;
        assert_eq!(
            stored_load_refusal(ReferenceError::Unresolved(unresolved.to_string())),
            unresolved,
            "the resolution's own payload is returned as is"
        );
    }

    /// Fresh-run settings are refused only when some were given.
    #[test]
    fn refuse_fresh_run_settings_names_only_what_was_given() {
        assert!(refuse_fresh_run_settings("p", &[]).is_ok());
        assert!(refuse_fresh_run_settings("p", &[("max_plans", false)]).is_ok());
        let refused = refuse_fresh_run_settings(
            "prog:1",
            &[
                ("max_plans", true),
                ("random_seed", false),
                ("target_course", true),
            ],
        )
        .unwrap_err();
        assert!(refused.contains("max_plans, target_course"), "{refused}");
        assert!(!refused.contains("random_seed"), "{refused}");
        assert!(refused.contains("prog:1") && refused.contains("fresh=true"));
    }
}

#[cfg(test)]
mod db_unavailable_tests {
    use super::*;

    #[test]
    fn response_names_the_backend_the_reason_and_the_restart_requirement() {
        // A correctly-configured backend that was simply unreachable at startup used to
        // be reported as "config is incomplete or you're not signed in" for the whole
        // process lifetime. It must now say what actually happened.
        let error = DatabaseError::ConnectionError("connection refused".to_string());
        let reason = DbUnavailable::from_error(&error, "https://nu.example.com");
        let json = db_not_configured_response("query_institutions", Some(&reason));
        let v: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

        assert_eq!(v["backend"], "https://nu.example.com");
        assert_eq!(v["reason"], "unreachable");
        assert_eq!(v["tool"], "query_institutions");
        assert!(
            v["detail"]
                .as_str()
                .is_some_and(|d| d.contains("connection refused")),
            "detail must carry the underlying error: {v}"
        );

        let steps = v["next_steps"].as_array().expect("next_steps is an array");
        let joined = steps
            .iter()
            .filter_map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            joined.contains("not a login problem"),
            "an unreachable backend must not be reported as a login problem: {joined}"
        );
        assert!(
            joined.contains("restart the server"),
            "the client is built once at startup, so the restart requirement must be \
             stated: {joined}"
        );
    }

    #[test]
    fn response_distinguishes_not_configured_from_not_authenticated() {
        let not_configured = DbUnavailable::from_error(&DatabaseError::NotConfigured, "");
        let v: serde_json::Value =
            serde_json::from_str(&db_not_configured_response("t", Some(&not_configured)))
                .expect("valid JSON");
        assert_eq!(v["reason"], "not_configured");
        assert_eq!(v["backend"], "(no endpoint configured)");
        let steps = v["next_steps"].to_string();
        assert!(
            steps.contains("config set database.endpoint"),
            "a not-configured install has no backend to log in to: {steps}"
        );
        assert!(
            !steps.contains("db login"),
            "must not tell a not-configured user to log in: {steps}"
        );

        let not_authed = DbUnavailable::from_error(
            &DatabaseError::NotAuthenticated("auth file missing".to_string()),
            "https://nu.example.com",
        );
        let v2: serde_json::Value =
            serde_json::from_str(&db_not_configured_response("t", Some(&not_authed)))
                .expect("valid JSON");
        assert_eq!(v2["reason"], "not_authenticated");
        let steps2 = v2["next_steps"].to_string();
        assert!(steps2.contains("db login"), "got: {steps2}");
        assert!(
            steps2.contains("nu.example.com"),
            "must name which backend to log in to: {steps2}"
        );
    }

    #[test]
    fn response_without_a_reason_says_there_is_no_client() {
        let v: serde_json::Value =
            serde_json::from_str(&db_not_configured_response("t", None)).expect("valid JSON");
        assert_eq!(v["reason"], "no_client");
        assert!(v["next_steps"].as_array().is_some_and(|a| !a.is_empty()));
    }

    #[test]
    fn a_tool_on_a_db_less_server_reports_the_startup_reason() {
        // The three tests above call the formatter directly, so none of them would
        // notice if `get_db` stopped forwarding the recorded reason — every tool would
        // silently fall back to "no_client" and still pass.
        let server = NuAnalyticsMcpServer::with_db_error(
            &DatabaseError::ConnectionError("connection refused".to_string()),
            "https://nu.example.com",
        );
        let response = server
            .get_db("query_institutions")
            .expect_err("a server built from a startup failure has no client");
        let v: serde_json::Value = serde_json::from_str(&response).expect("valid JSON");
        assert_eq!(
            v["reason"], "unreachable",
            "the startup reason must reach the tool, not be replaced by no_client: {v}"
        );
        assert_eq!(v["backend"], "https://nu.example.com");
        assert_eq!(v["tool"], "query_institutions");
    }

    #[test]
    fn the_restart_requirement_is_stated_by_text_not_merely_present() {
        let reason = DbUnavailable::from_error(&DatabaseError::NotConfigured, "");
        let v: serde_json::Value =
            serde_json::from_str(&db_not_configured_response("t", Some(&reason)))
                .expect("valid JSON");
        let steps = v["next_steps"].to_string();
        assert!(
            steps.contains("restart the server"),
            "the client is built once at startup, so this must be said: {steps}"
        );
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    use crate::core::init_assets;
    use std::collections::BTreeSet;

    fn served() -> BTreeSet<String> {
        NuAnalyticsMcpServer::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.into_owned())
            .collect()
    }

    #[test]
    fn capabilities_name_exactly_the_tools_the_router_serves() {
        let listed: BTreeSet<String> = CAPABILITIES
            .iter()
            .flat_map(|c| c.tools.iter().map(|t| (*t).to_string()))
            .collect();
        let entries: usize = CAPABILITIES.iter().map(|c| c.tools.len()).sum();
        assert_eq!(entries, listed.len(), "a tool is listed in two groups");
        assert_eq!(listed, served(), "CAPABILITIES vs the router");
    }

    /// Tools `text` names, with the parameters it passes them.
    ///
    /// Two forms count: `mcp__nuanalytics__tool`, as `allowed-tools` writes it, and a
    /// backticked call, `` `tool(arg=…)` ``, which is how shipped text writes a tool. Fenced
    /// code blocks are skipped — SQL examples call functions like `jsonb_agg(` — and a call
    /// only counts when its name has an underscore, as every tool name does.
    fn referenced_tools(text: &str) -> Vec<(String, Vec<String>)> {
        let prose: String = text.split("```").step_by(2).collect::<Vec<_>>().join("\n");
        let ident = |s: &str| -> String {
            s.chars()
                .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
                .collect()
        };
        let prefix = format!("mcp__{}__", init_assets::MCP_SERVER_NAME);
        let mut found = Vec::new();
        for (at, _) in prose.match_indices(&prefix) {
            let name = ident(&prose[at + prefix.len()..]);
            if !name.is_empty() {
                found.push((name, Vec::new()));
            }
        }
        for (at, _) in prose.match_indices('`') {
            let rest = &prose[at + 1..];
            let name = ident(rest);
            let Some(inner) = rest[name.len()..].strip_prefix('(') else {
                continue;
            };
            if !name.contains('_') {
                continue;
            }
            let inner = inner.split(')').next().unwrap_or_default();
            let args = inner
                .split(',')
                .filter_map(|part| part.split_once('=').map(|(k, _)| k.trim().to_string()))
                .collect();
            found.push((name, args));
        }
        found
    }

    #[test]
    fn shipped_skills_and_instructions_name_only_real_tools_and_parameters() {
        let router = NuAnalyticsMcpServer::tool_router();
        let mut texts: Vec<(String, String)> = init_assets::markdown_files()
            .map(|(path, text)| (path.to_string(), text.to_string()))
            .collect();
        texts.push((
            "the server instructions".into(),
            render_instructions("", &|_| true),
        ));
        for (source, text) in &texts {
            for (tool, args) in referenced_tools(text) {
                let served = router.get(&tool).unwrap_or_else(|| {
                    panic!("{source} names `{tool}`, which the server does not serve")
                });
                let properties = served
                    .input_schema
                    .get("properties")
                    .and_then(serde_json::Value::as_object);
                for arg in args {
                    assert!(
                        properties.is_some_and(|p| p.contains_key(&arg)),
                        "{source}: `{tool}` has no parameter `{arg}`"
                    );
                }
            }
        }
    }

    #[test]
    fn referenced_tools_reads_both_forms_and_ignores_code_and_plain_calls() {
        let text = "Use `validate_degree(path=x, allow_unmatched_patterns=true)` then \
                    mcp__nuanalytics__audit_degree. Not `max(year)`.\n```sql\nSELECT jsonb_agg(x)\n```";
        let found = referenced_tools(text);
        assert_eq!(
            found,
            [
                ("audit_degree".to_string(), Vec::new()),
                (
                    "validate_degree".to_string(),
                    vec!["path".to_string(), "allow_unmatched_patterns".to_string()]
                ),
            ]
        );
    }

    #[test]
    fn skills_pre_approve_only_tools_that_only_read() {
        // A skill's allowed-tools skips the approval prompt, so it must never name a tool
        // that writes a file or the database.
        let router = NuAnalyticsMcpServer::tool_router();
        let prefix = format!("mcp__{}__", init_assets::MCP_SERVER_NAME);
        for (path, text) in init_assets::markdown_files().filter(|(p, _)| p.ends_with("/SKILL.md"))
        {
            let allowed = text
                .lines()
                .find_map(|l| l.strip_prefix("allowed-tools:"))
                .unwrap_or_else(|| panic!("{path}: no allowed-tools"));
            for entry in allowed.split_whitespace() {
                let name = entry.strip_prefix(&prefix).unwrap_or(entry);
                let tool = router
                    .get(name)
                    .unwrap_or_else(|| panic!("{path}: allowed-tools names `{name}`, not served"));
                let read_only = tool.annotations.as_ref().and_then(|a| a.read_only_hint);
                assert_eq!(
                    read_only,
                    Some(true),
                    "{path}: `{name}` writes; ask instead"
                );
            }
        }
    }

    #[test]
    fn no_shipped_text_names_a_retired_tool_or_parameter() {
        const RETIRED: &[&str] = &[
            "get_degree_schema",
            "get_degree_json_schema",
            "degree_pipeline",
            "cache_yaml",
            "generate_degree_report",
            "get_curriculum_visualization",
            "get_institution_completions",
            "get_schools_completion_demographics",
            "scaffold_degree_yaml",
            "store_degree",
            "yaml_content",
            "yaml_path",
            "trimmed_cache_id",
            "include_graph_spec",
            "plan_indices",
            "enrolled",
            "enrollment_pct",
            "school_pct",
        ];
        let mut texts: Vec<(String, String)> = init_assets::markdown_files()
            .map(|(p, t)| (p.to_string(), t.to_string()))
            .collect();
        texts.push((
            "list_sample_degrees note".into(),
            crate::mcp::tools::samples::execute(false).note,
        ));
        texts.push(("instructions".into(), render_instructions("", &|_| true)));
        for tool in NuAnalyticsMcpServer::tool_router().list_all() {
            texts.push((
                tool.name.to_string(),
                format!(
                    "{} {}",
                    tool.description.as_deref().unwrap_or_default(),
                    serde_json::to_string(&*tool.input_schema).unwrap_or_default()
                ),
            ));
        }
        for (source, text) in &texts {
            for name in RETIRED {
                assert!(!text.contains(name), "{source} names retired `{name}`");
            }
        }
    }

    #[test]
    fn every_tool_says_whether_it_only_reads() {
        // Clients use the hint to decide what needs the user's approval; a tool that
        // omits it leaves them guessing.
        for tool in NuAnalyticsMcpServer::tool_router().list_all() {
            let hints = tool
                .annotations
                .as_ref()
                .unwrap_or_else(|| panic!("{} has no annotations", tool.name));
            assert!(
                hints.read_only_hint.is_some(),
                "{} must set read_only_hint",
                tool.name
            );
        }
    }

    #[test]
    fn every_description_says_what_the_tool_returns_in_under_500_characters() {
        // Clients show descriptions to the model on every turn a tool is in view; parameter
        // detail belongs in the input schema, not here.
        for tool in NuAnalyticsMcpServer::tool_router().list_all() {
            let description = tool.description.as_deref().unwrap_or_default();
            assert!(!description.is_empty(), "{} has no description", tool.name);
            assert!(
                description.chars().count() <= 500,
                "{}: {} characters",
                tool.name,
                description.chars().count()
            );
        }
    }

    #[test]
    fn every_followup_names_a_tool_the_server_serves() {
        let served = served();
        for tool in shared::FOLLOWUP_TOOLS {
            assert!(
                served.contains(tool),
                "follow-ups name `{tool}`, which is not served"
            );
        }
    }

    #[test]
    fn the_write_tools_are_served_only_when_asked_for() {
        let reading = NuAnalyticsMcpServer::new();
        for tool in WRITE_TOOLS {
            assert!(
                !reading.tool_router.has_route(tool),
                "{tool} served without --allow-writes"
            );
        }
        let writing = NuAnalyticsMcpServer::new().with_writes();
        for tool in WRITE_TOOLS {
            assert!(
                writing.tool_router.has_route(tool),
                "{tool} missing with --allow-writes"
            );
        }
        // What the protocol handler advertises, which is not the same question: the
        // `#[tool_handler]` default reads a fresh full router, not the server's own.
        for tool in WRITE_TOOLS {
            assert!(
                ServerHandler::get_tool(&reading, tool).is_none(),
                "{tool} advertised without --allow-writes"
            );
            assert!(
                ServerHandler::get_tool(&writing, tool).is_some(),
                "{tool} not advertised with --allow-writes"
            );
        }
        let text = render_instructions("", &|t| reading.tool_router.has_route(t));
        assert!(
            !text.contains("import_degree"),
            "instructions offer a tool not served"
        );
        assert!(!tool_list(false)
            .iter()
            .any(|(name, _)| name == "import_degree"));
        assert!(tool_list(true)
            .iter()
            .any(|(name, _)| name == "import_degree"));
    }

    #[test]
    fn instructions_stay_a_short_capability_map() {
        // They go into the model's context on every turn.
        let text = render_instructions("Database: connected.", &|_| true);
        assert!(
            text.lines().count() <= 20,
            "{} lines:\n{text}",
            text.lines().count()
        );
        assert!(
            !text.contains("1. "),
            "a numbered workflow crept back in:\n{text}"
        );
    }
}
