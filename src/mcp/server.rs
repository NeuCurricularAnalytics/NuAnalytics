//! MCP server implementation
//!
//! This module provides the MCP server that exposes `NuAnalytics` tools.

use std::sync::Arc;

use crate::core::config::DatabaseConfig;
use crate::mcp::tools::{
    analyze, audit, cache, course_detail, match_courses, pipeline, plan_graph, report, samples,
    schema, shared, validate, visualize, AnalyzeDegreeRequest, AuditDegreeRequest,
    CacheYamlRequest, DegreePipelineRequest, FindCoursesMatchingRequest,
    GenerateDegreeReportRequest, GetCourseDetailRequest, GetCurriculumVisualizationRequest,
    GetSchemaRequest, ListSampleDegreesRequest, RenderPlanGraphRequest, ValidateDegreeRequest,
};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
    ServerHandler, ServiceExt,
};

#[cfg(feature = "database")]
use crate::core::database::DbClient;
#[cfg(feature = "database")]
use crate::mcp::tools::{
    cip_codes, completions, degrees, institutions, lookup, scaffold, CompareDegreesRequest,
    CompletionDemographicsRequest, GetDegreeRequest, GetInstitutionCompletionsRequest,
    GetInstitutionRequest, GetLookupCodesRequest, GetSchoolsCompletionDemographicsRequest,
    ScaffoldDegreeYamlRequest, SearchCipCodesRequest, SearchDegreesRequest,
    SearchInstitutionsRequest,
};
// `StoreDegreeRequest` import kept out of the active list while the
// store_degree MCP tool is shelved (see TODO in the impl block); the type
// itself still lives in `degrees::StoreDegreeRequest` for when the DB
// write path is provisioned.

// ============================================================================
// MCP Server Implementation
// ============================================================================

/// MCP server that exposes `NuAnalytics` tools via stdio transport.
///
/// Non-database tools (degree YAML validation and analysis) are always available.
/// Database tools (IPEDS queries) are enabled when `db` is `Some`.
#[derive(Debug, Clone)]
pub struct NuAnalyticsMcpServer {
    // Read indirectly by the rmcp #[tool_handler] macro's generated trait impl
    // for dispatch; rustc's dead_code analysis under release/LTO can't trace
    // that read through the macro expansion, so silence the false positive.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
    /// Database client — `None` if database is not configured or disabled
    #[cfg(feature = "database")]
    db: Option<Arc<DbClient>>,
}

#[tool_router]
impl NuAnalyticsMcpServer {
    /// Create a new MCP server instance without database access
    #[must_use]
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
            #[cfg(feature = "database")]
            db: None,
        }
    }

    /// Create a new MCP server instance with database access
    #[cfg(feature = "database")]
    #[must_use]
    pub fn with_db(db: Option<Arc<DbClient>>) -> Self {
        Self {
            tool_router: Self::tool_router(),
            db,
        }
    }

    // ── Degree tools (no DB required) ──────────────────────────────────────

    /// Get the degree YAML schema documentation
    #[tool(
        description = "Get the degree YAML schema documentation. Returns structured information about how to create valid degree program YAML files, including field descriptions, requirement types, and examples."
    )]
    #[allow(clippy::unused_self)]
    fn get_degree_schema(&self, Parameters(req): Parameters<GetSchemaRequest>) -> String {
        schema::execute(req.section.as_deref())
    }

    /// Validate a degree program YAML
    #[tool(
        description = "Validate a degree program YAML. Returns detailed validation results including errors, warnings, and suggestions. Provide exactly ONE YAML source: yaml_content (inline string), yaml_path (file on the server's filesystem), or degree_id (DB lookup). Pass allow_unmatched_patterns=true to surface external gen-ed pool patterns (e.g. \"*:100+\") as warnings instead of errors when courses aren't enumerated locally."
    )]
    fn validate_degree(&self, Parameters(req): Parameters<ValidateDegreeRequest>) -> String {
        let allow_unmatched_patterns = req.allow_unmatched_patterns.unwrap_or(false);
        let source = match shared::parse_yaml_source(req.yaml_content, req.yaml_path, req.degree_id)
        {
            Ok(s) => s,
            Err(e) => return e,
        };
        self.run_yaml_tool("validate_degree", source, move |yaml| {
            validate::execute_json(yaml, allow_unmatched_patterns)
        })
    }

    /// Audit a degree program YAML
    #[tool(
        description = "Run a comprehensive audit on a degree program YAML: validation, missing prerequisites on upper-level courses, and deep prerequisite chains. Provide exactly ONE YAML source: yaml_content (inline), yaml_path (file path), or degree_id (DB lookup). Returns structured findings."
    )]
    fn audit_degree(&self, Parameters(req): Parameters<AuditDegreeRequest>) -> String {
        let chain_threshold = req.chain_threshold;
        let source = match shared::parse_yaml_source(req.yaml_content, req.yaml_path, req.degree_id)
        {
            Ok(s) => s,
            Err(e) => return e,
        };
        self.run_yaml_tool("audit_degree", source, move |yaml| {
            audit::execute_json(yaml, chain_threshold)
        })
    }

    /// Analyze a degree program YAML
    #[tool(
        description = "Run full degree analysis: generate all possible course plans, compute aggregate metrics (complexity, delay, credits), and identify shortest/longest paths. Provide exactly ONE YAML source: yaml_content (inline), yaml_path (file path), or degree_id (DB lookup). Returns aggregate stats across the analyzed plans plus a curated selected_plans list (always shortest + longest + optional calc-ready-shortest + 3 random samples — 5-6 plans typical, independent of max_plans). The complexity/longest_delay/total_credits objects use the standard 5-number summary (min/q1/median/q3/max + mean/std_dev) — to plot as a boxplot in Chart.js, add the chartjs-chart-boxplot plugin. The response also includes is_full_population: when true, plans_analyzed is the entire population for this YAML; when false, it's a sample capped at max_plans. Set include_graph_spec=true (default false) when you need to render visualizations — each spec is ~30 KB so omitting them keeps the response compact. Use after validate_degree confirms the YAML is valid. Optionally specify include_courses to constrain all plans to include specific courses."
    )]
    fn analyze_degree(&self, Parameters(req): Parameters<AnalyzeDegreeRequest>) -> String {
        let include_courses = req.include_courses.map(|s| shared::parse_comma_list(&s));
        let include_graph_spec = req.include_graph_spec.unwrap_or(false);
        let max_plans = req.max_plans;
        let plan_indices: Option<Vec<usize>> = req
            .plan_indices
            .as_deref()
            .map(shared::parse_comma_list_usize);
        let source = match shared::parse_yaml_source(req.yaml_content, req.yaml_path, req.degree_id)
        {
            Ok(s) => s,
            Err(e) => return e,
        };
        self.run_yaml_tool("analyze_degree", source, move |yaml| {
            analyze::execute_json(
                yaml,
                max_plans,
                include_courses.as_deref(),
                include_graph_spec,
                plan_indices.as_deref(),
            )
        })
    }

    /// Look up a single course's prerequisites, dependents, and stats
    #[tool(
        description = "Return everything an LLM typically wants to know about one course in a degree YAML: title/credits/level, raw + direct + transitive prerequisites, dependents, requirements that reference it, cross-listed equivalents, and (with include_analysis=true, default) per-course metric medians + term placement in every selected plan. Set include_analysis=false to skip the analysis pipeline for static-only data (~10x faster). Same yaml source modes as analyze_degree (yaml_content / yaml_path / degree_id). Use this instead of analyze_degree when the question is 'tell me about CS370 in this degree'."
    )]
    fn get_course_detail(&self, Parameters(req): Parameters<GetCourseDetailRequest>) -> String {
        let course_id = req.course_id.clone();
        let include_analysis = req.include_analysis.unwrap_or(true);
        let max_plans = req.max_plans;
        let source = match shared::parse_yaml_source(req.yaml_content, req.yaml_path, req.degree_id)
        {
            Ok(s) => s,
            Err(e) => return e,
        };
        self.run_yaml_tool("get_course_detail", source, move |yaml| {
            course_detail::execute_json(yaml, &course_id, include_analysis, max_plans)
        })
    }

    /// Run validate + audit + analyze in a single MCP call
    #[tool(
        description = "Combined pipeline: runs validate_degree, audit_degree, and analyze_degree on the same YAML in one call. Short-circuits when validate hits a YAML parse error (audit/analyze are then null). Use skip_audit / skip_analyze to stop earlier when you only need validate (or validate + audit). Same yaml source modes as the individual tools (yaml_content / yaml_path / degree_id). Forwards allow_unmatched_patterns to validate, chain_threshold to audit, max_plans + include_courses to analyze. Returns {validate, audit?, analyze?} — saves three round-trips for the common 'look at this degree' prompt."
    )]
    fn degree_pipeline(&self, Parameters(req): Parameters<DegreePipelineRequest>) -> String {
        let allow_unmatched_patterns = req.allow_unmatched_patterns.unwrap_or(false);
        let chain_threshold = req.chain_threshold;
        let max_plans = req.max_plans;
        let include_courses = req.include_courses.map(|s| shared::parse_comma_list(&s));
        let skip_audit = req.skip_audit.unwrap_or(false);
        let skip_analyze = req.skip_analyze.unwrap_or(false);
        let source = match shared::parse_yaml_source(req.yaml_content, req.yaml_path, req.degree_id)
        {
            Ok(s) => s,
            Err(e) => return e,
        };
        self.run_yaml_tool("degree_pipeline", source, move |yaml| {
            pipeline::execute_json(
                yaml,
                allow_unmatched_patterns,
                chain_threshold,
                max_plans,
                include_courses.as_deref(),
                skip_audit,
                skip_analyze,
            )
        })
    }

    /// Generate the full HTML degree report (plus optional CSV / JSONL / index artifacts)
    #[tool(
        description = "Build the full HTML degree analysis report — the same artifact the CLI `degree --analyze` command produces. Provide exactly ONE YAML source: yaml_content (inline), yaml_path (file path), or degree_id (DB lookup). Same analysis knobs as analyze_degree (max_plans, include_courses). Set output_dir to write the HTML report + optional per-plan CSVs + JSONL summary + index.csv into a directory; in that mode html_content is omitted from the response (override with return_html_inline=true). Without output_dir, the rendered HTML is returned inline (~200-300 KB for a typical degree). Companion outputs (write_plan_csvs / write_jsonl_summary / write_index_csv) default to true in disk mode and are ignored in inline mode."
    )]
    fn generate_degree_report(
        &self,
        Parameters(req): Parameters<GenerateDegreeReportRequest>,
    ) -> String {
        let max_plans = req.max_plans;
        let include_courses = req.include_courses.map(|s| shared::parse_comma_list(&s));
        let output_dir = req.output_dir;
        let write_plan_csvs = req.write_plan_csvs;
        let write_jsonl_summary = req.write_jsonl_summary;
        let write_index_csv = req.write_index_csv;
        let return_html_inline = req.return_html_inline;
        let source = match shared::parse_yaml_source(req.yaml_content, req.yaml_path, req.degree_id)
        {
            Ok(s) => s,
            Err(e) => return e,
        };
        self.run_yaml_tool("generate_degree_report", source, move |yaml| {
            report::execute_json(
                yaml,
                max_plans,
                include_courses.as_deref(),
                output_dir.as_deref(),
                write_plan_csvs,
                write_jsonl_summary,
                write_index_csv,
                return_html_inline,
            )
        })
    }

    /// Cache an inline YAML body and return a degree_id-style handle
    #[tool(
        description = "Cache an inline degree YAML body in the server and return a handle (\"cache:{hex}\") that any other tool accepts as a `degree_id`. Removes the per-call repaste tax for hosted MCP clients whose filesystem the server can't see (yaml_path returns ENOENT in that setup). Handle is content-hashed and idempotent — caching the same body twice returns the same handle. TTL is about 1 hour. After caching: pass the handle as `degree_id` on validate_degree / audit_degree / analyze_degree / generate_degree_report / get_course_detail / render_plan_graph / find_courses_matching / degree_pipeline / compare_degrees."
    )]
    #[allow(clippy::unused_self)]
    fn cache_yaml(&self, Parameters(req): Parameters<CacheYamlRequest>) -> String {
        cache::execute_json(req.yaml_content)
    }

    /// List the bundled sample degree YAMLs
    #[tool(
        description = "List the sample degree YAMLs bundled with this MCP server (three real curricula: CSU Fort Collins, Northeastern Khoury, UH Manoa). Default response is metadata-only (institution, program, total_credits, summary). Pass include_yaml=true to also receive the full embedded YAML body for each sample — pipe that body into validate_degree / audit_degree / analyze_degree / generate_degree_report. Use this to bootstrap exploration when the caller doesn't have a YAML in hand."
    )]
    #[allow(clippy::unused_self)]
    fn list_sample_degrees(&self, Parameters(req): Parameters<ListSampleDegreesRequest>) -> String {
        samples::execute_json(req.include_yaml.unwrap_or(false))
    }

    /// Preview which courses match a set of patterns in a degree YAML
    #[tool(
        description = "Resolve a set of include/exclude patterns against the courses defined in a YAML and return the matched course list with titles + levels. Same pattern grammar as `select` requirements (e.g. \"CS:300+\", \"MATH:300-499\"). Useful when sketching a new requirement and you want to preview the resulting pool before committing it to the YAML. Same yaml source modes as analyze_degree."
    )]
    fn find_courses_matching(
        &self,
        Parameters(req): Parameters<FindCoursesMatchingRequest>,
    ) -> String {
        let patterns = shared::parse_comma_list(&req.patterns);
        let exclude = req
            .exclude
            .as_deref()
            .map(shared::parse_comma_list)
            .unwrap_or_default();
        let source = match shared::parse_yaml_source(req.yaml_content, req.yaml_path, req.degree_id)
        {
            Ok(s) => s,
            Err(e) => return e,
        };
        self.run_yaml_tool("find_courses_matching", source, move |yaml| {
            match_courses::execute_json(yaml, patterns, exclude)
        })
    }

    /// Render the curriculum graph for one selected plan in a single call
    #[tool(
        description = "Render the curriculum graph HTML for one selected plan in a single call (analyze + extract graph_spec + visualize in one tool). Pick a plan via plan_category=\"shortest\" | \"longest\" | \"calc-ready-shortest\" | \"sample\" (with optional sample_index, 1-indexed) OR via plan_index (0-indexed offset into the analyze response's selected_plans). format defaults to \"standalone\" — pass \"fragment\" or \"fragment-no-library\" to embed in another HTML document. Same yaml source modes + analyze knobs (max_plans, include_courses) as analyze_degree."
    )]
    fn render_plan_graph(&self, Parameters(req): Parameters<RenderPlanGraphRequest>) -> String {
        let plan_category = req.plan_category;
        let sample_index = req.sample_index;
        let plan_index = req.plan_index;
        let format = req.format;
        let max_plans = req.max_plans;
        let include_courses = req.include_courses.map(|s| shared::parse_comma_list(&s));
        let source = match shared::parse_yaml_source(req.yaml_content, req.yaml_path, req.degree_id)
        {
            Ok(s) => s,
            Err(e) => return e,
        };
        self.run_yaml_tool("render_plan_graph", source, move |yaml| {
            plan_graph::execute_json(
                yaml,
                plan_category.as_deref(),
                sample_index,
                plan_index,
                format,
                max_plans,
                include_courses.as_deref(),
            )
        })
    }

    /// Render a curriculum graph visualization from an `analyze_degree` result
    #[tool(
        description = "Low-level: most callers should use `render_plan_graph` instead — that tool runs analyze + extracts the graph_spec + renders in one call. Use this tool only when you already have a graph_spec in hand and want full control over rendering. Renders an interactive HTML curriculum graph from a graph_spec produced by analyze_degree. The output shows course nodes arranged by term, prerequisite/corequisite edges drawn as Bezier curves, complexity badges, hover highlighting of prerequisite chains, and a click-to-open course detail modal. Set format=\"standalone\" (default) for a full HTML page openable in a browser, format=\"fragment\" for an embeddable snippet, or format=\"fragment-no-library\" when the page already loaded the shared library."
    )]
    #[allow(clippy::unused_self)]
    fn get_curriculum_visualization(
        &self,
        Parameters(req): Parameters<GetCurriculumVisualizationRequest>,
    ) -> String {
        visualize::execute_html(&req.graph_spec_json, req.format)
    }

    // ── Institution tools ───────────────────────────────────────────────────

    /// Search institutions from the IPEDS database
    #[cfg(feature = "database")]
    #[tool(
        description = "Search institutions from the IPEDS database. Filter by name, state, Carnegie classification (15=R1, 16=R2, 21=R1-2021), control (1=public, 2=private nonprofit), HBCU/tribal status, or minimum size (inst_size_min: 2=1000+ students). Returns UNITID and metadata. Use get_lookup_codes(\"carnegie_class\") for the full classification list."
    )]
    fn search_institutions(
        &self,
        Parameters(req): Parameters<SearchInstitutionsRequest>,
    ) -> String {
        self.call_db("search_institutions", |db| async move {
            institutions::execute_search_json(&db, req).await
        })
    }

    /// Get full details for a specific institution by UNITID
    #[cfg(feature = "database")]
    #[tool(
        description = "Get full institution details by IPEDS Unit ID (UNITID). Returns all fields including sector, locale, Carnegie class, HBCU/tribal status, and institution size. Use search_institutions to find the UNITID first."
    )]
    fn get_institution(&self, Parameters(req): Parameters<GetInstitutionRequest>) -> String {
        self.call_db("get_institution", |db| async move {
            institutions::execute_get_json(&db, req).await
        })
    }

    // ── CIP code tools ──────────────────────────────────────────────────────

    /// Search CIP program codes by title keyword or code prefix
    #[cfg(feature = "database")]
    #[tool(
        description = "Search CIP (Classification of Instructional Programs) codes. Use query for title keyword search (e.g. \"cybersecurity\"), prefix for code prefix (e.g. \"11.\" for all CS, \"30.70\" for Data Science). Use trailing dot for families: \"11.\" not \"11\". Results include cip_code (dot notation) and title."
    )]
    fn search_cip_codes(&self, Parameters(req): Parameters<SearchCipCodesRequest>) -> String {
        self.call_db("search_cip_codes", |db| async move {
            cip_codes::execute_json(&db, req).await
        })
    }

    /// Get the full contents of an IPEDS lookup table
    #[cfg(feature = "database")]
    #[tool(
        description = "Get all rows from an IPEDS lookup table to discover numeric code meanings. Tables: \"carnegie_class\" (R1/R2 codes), \"award_levels\" (bachelor's/master's codes), \"institution_control\", \"institution_level\", \"institution_sector\", \"institution_locale\", \"institution_size\". Call this before filtering to confirm the right code."
    )]
    fn get_lookup_codes(&self, Parameters(req): Parameters<GetLookupCodesRequest>) -> String {
        self.call_db("get_lookup_codes", |db| async move {
            lookup::execute_json(&db, req).await
        })
    }

    // ── Completion demographic tools ────────────────────────────────────────

    /// Get CS completion demographics for a single institution
    #[cfg(feature = "database")]
    #[tool(
        description = "Get completion demographics per CIP program for a single institution. Returns per-CIP rows with demographic counts + representation ratios, plus a cross_tab section showing the race×gender breakdown aggregated across all selected programs. cross_tab fields: women_pct_within_group (gender parity: % of race group that are women), women/men_pct_of_total (share of all CS completions), women/men_representation_ratio (vs institution baseline). Use cip_prefix=\"11.\" for CS. Provide year for accurate ratios. When the CIP filter matches no rows, the response includes nearby_cips_with_data — top CIPs at this institution that DO have completions — so you can see whether the school files the program under a different CIP code (the most common IPEDS user error)."
    )]
    fn get_institution_completions(
        &self,
        Parameters(req): Parameters<GetInstitutionCompletionsRequest>,
    ) -> String {
        self.call_db("get_institution_completions", |db| async move {
            completions::execute_institution_json(&db, req).await
        })
    }

    /// Get per-school CS completion demographics across many institutions in one call
    #[cfg(feature = "database")]
    #[tool(
        description = "Per-school CS completion demographics. Use unitid for a single school, or combine carnegie_class/control/state/hbcu/tribal/inst_size_min for a group. Returns demographics (Women%, race%, representation ratios) AND cross_tab (race×gender: women_pct_within_group, representation_ratio per cell). Omitting cip_prefix/cip_codes returns all CIPs; set cip_prefix=\"11.\" for CS. Provide year for accurate ratios."
    )]
    fn get_schools_completion_demographics(
        &self,
        Parameters(req): Parameters<GetSchoolsCompletionDemographicsRequest>,
    ) -> String {
        self.call_db("get_schools_completion_demographics", |db| async move {
            completions::execute_schools_json(&db, req).await
        })
    }

    /// Query completion demographics aggregated across institutions
    #[cfg(feature = "database")]
    #[tool(
        description = "Aggregate completion demographics across matching institutions. Returns demographics AND cross_tab (race×gender). Filter by unitid (single school), carnegie_class, control, state, cip_prefix (\"11.\" for CS — no default, omit for all CIPs), cip_codes (comma-separated exact codes), award level, year. For per-school breakdown use get_schools_completion_demographics."
    )]
    fn get_completion_demographics(
        &self,
        Parameters(req): Parameters<CompletionDemographicsRequest>,
    ) -> String {
        self.call_db("get_completion_demographics", |db| async move {
            completions::execute_json(&db, req).await
        })
    }

    /// Generate a starter degree YAML for a UNITID + CIP code
    #[cfg(feature = "database")]
    #[tool(
        description = "Generate a minimal degree YAML scaffold for a UNITID + CIP code: pulls institution name from IPEDS, CIP title from the cip_codes lookup, derives a slug like \"{inst-slug}-{cip-slug}-bscs-{year}\", and emits a `degree:` header with empty `requirements:` + `courses:` ready for the caller to fill in. Defaults system_type to \"semester\" (override with the parameter). Optional catalog_year populates both the slug suffix and the YAML field. Removes the cold-start barrier for building a new degree definition."
    )]
    fn scaffold_degree_yaml(
        &self,
        Parameters(req): Parameters<ScaffoldDegreeYamlRequest>,
    ) -> String {
        self.call_db("scaffold_degree_yaml", |db| async move {
            scaffold::execute_json(&db, req).await
        })
    }

    // ── Degree storage tools ────────────────────────────────────────────────

    /// Search stored degree programs in the database
    #[cfg(feature = "database")]
    #[tool(
        description = "Search stored degree programs in the database. Filter by institution UNITID, CIP code prefix (\"11.\" for CS), or catalog year. Returns matching degrees with metadata. Use get_degree to retrieve the full YAML content."
    )]
    fn search_degrees(&self, Parameters(req): Parameters<SearchDegreesRequest>) -> String {
        self.call_db("search_degrees", |db| async move {
            degrees::execute_search_json(&db, req).await
        })
    }

    /// Retrieve a stored degree program by ID or natural key
    #[cfg(feature = "database")]
    #[tool(
        description = "Retrieve a full degree program YAML. Lookup by degree_id (fastest) or by natural key: unitid + cip_code + catalog_year. If multiple degrees match the natural key, returns a list of summaries — narrow with more filters or use degree_id. Returns full YAML content usable with validate_degree / analyze_degree."
    )]
    fn get_degree(&self, Parameters(req): Parameters<GetDegreeRequest>) -> String {
        self.call_db("get_degree", |db| async move {
            degrees::execute_get_json(&db, req).await
        })
    }

    /// Compare multiple stored degree programs (and/or inline YAMLs)
    #[cfg(feature = "database")]
    #[tool(
        description = "Compare multiple degrees side-by-side. Provide `sources` (structured list of {label?, degree_id?|yaml_content?|yaml_path?}, exactly one source per entry) and/or `degree_ids` (legacy comma-separated stored-IDs form). `sources` lets you benchmark an in-progress inline YAML against a stored peer without storing it first. Returns metadata + YAML content per entry, plus (default) analyze-style metrics (complexity, longest_delay, total_credits, plans_analyzed). Set include_metrics=false to skip the analysis pass."
    )]
    fn compare_degrees(&self, Parameters(req): Parameters<CompareDegreesRequest>) -> String {
        self.call_db("compare_degrees", |db| async move {
            degrees::execute_compare_json(&db, req).await
        })
    }

    /// Run a degree-pipeline tool against a [`YamlSource`].
    ///
    /// Resolves the YAML from inline content, a filesystem path, or a stored
    /// degree id, then invokes `run` with the resolved string. Errors at any
    /// resolution step return a JSON error string.
    fn run_yaml_tool<F>(
        &self,
        #[cfg_attr(not(feature = "database"), allow(unused_variables))] tool: &'static str,
        source: shared::YamlSource,
        run: F,
    ) -> String
    where
        F: FnOnce(&str) -> String,
    {
        match source {
            shared::YamlSource::Content(yaml) => run(&yaml),
            shared::YamlSource::Path(p) => match shared::read_yaml_file(&p) {
                Ok(yaml) => run(&yaml),
                Err(e) => e,
            },
            shared::YamlSource::DegreeId(id) => match self.resolve_degree_id(tool, &id) {
                Ok(yaml) => run(&yaml),
                Err(e) => e,
            },
        }
    }

    /// Resolve a `degree_id` to a YAML body using the layered lookup:
    ///
    /// 1. **YAML cache** — handles minted by `cache_yaml` (prefix
    ///    `cache:`). In-memory; works regardless of DB availability.
    /// 2. **Bundled samples** — the three sample keys (`csu`, `neu-khoury`,
    ///    `uhm`) returned by `list_sample_degrees`. Embedded at compile time.
    /// 3. **Database** — stored degrees by id (requires the `database`
    ///    feature + a configured client).
    ///
    /// Returns a JSON error string on any failure path so the calling
    /// handler can return it verbatim.
    fn resolve_degree_id(
        &self,
        #[cfg_attr(not(feature = "database"), allow(unused_variables))] tool: &'static str,
        id: &str,
    ) -> Result<String, String> {
        if id.starts_with(crate::mcp::cache::YAML_CACHE_PREFIX) {
            let cache = crate::mcp::cache::YAML_CACHE
                .lock()
                .expect("yaml cache mutex poisoned");
            return cache.get(id).map_or_else(
                || {
                    Err(serde_json::json!({
                        "error": "Unknown or expired YAML cache handle",
                        "degree_id": id,
                        "hint": "Call cache_yaml(yaml_content=...) to mint a fresh handle. Cache TTL is ~1 hour.",
                    })
                    .to_string())
                },
                |arc| Ok((*arc).to_string()),
            );
        }

        if let Some(yaml) = crate::mcp::tools::samples::yaml_for_key(id) {
            return Ok(yaml.to_string());
        }

        #[cfg(feature = "database")]
        {
            let db = self.get_db(tool)?;
            let id_owned = id.to_string();
            run_db_async_result(move || async move {
                shared::fetch_yaml_by_degree_id(&db, &id_owned).await
            })
        }
        #[cfg(not(feature = "database"))]
        {
            Err(shared::error_json(&format!(
                "degree_id '{id}' did not match any cache handle or bundled sample key; database lookups require the nu-analytics 'database' feature"
            )))
        }
    }

    // TODO(database): re-enable `store_degree` once the deployed Supabase
    // schema provisions the `degrees` write path and the auth flow is set up
    // (`nuanalytics db login`). The implementation in
    // `degrees::execute_store_json` + `StoreDegreeRequest` is in place and
    // works against a locally-configured DB; only the MCP exposure is
    // shelved so callers don't see a tool that returns auth errors.
    //
    // #[cfg(feature = "database")]
    // #[tool(
    //     description = "Save a validated degree program YAML to the database. Requires authentication (run `nuanalytics db login` first). Uses upsert on degree_id — safe to re-run after updates. Provide unitid (from search_institutions) and cip_code for the program. The degree YAML should be validated with validate_degree before storing."
    // )]
    // fn store_degree(&self, Parameters(req): Parameters<StoreDegreeRequest>) -> String {
    //     self.call_db("store_degree", |db| async move {
    //         degrees::execute_store_json(&db, req).await
    //     })
    // }
}

/// Database access helpers.
#[cfg(feature = "database")]
impl NuAnalyticsMcpServer {
    /// Return the configured DB client, or a JSON error response naming `tool_name`
    /// when the database is not configured.
    fn get_db(&self, tool_name: &'static str) -> Result<Arc<DbClient>, String> {
        self.db
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| db_not_configured_response(tool_name))
    }

    /// Fetch the DB client, run an async tool function, and return JSON.
    ///
    /// Returns an error JSON string if the database is not configured.
    fn call_db<F, Fut>(&self, tool: &'static str, f: F) -> String
    where
        F: FnOnce(Arc<DbClient>) -> Fut,
        Fut: std::future::Future<Output = String>,
    {
        match self.get_db(tool) {
            Ok(db) => run_db_async(move || f(db)),
            Err(e) => e,
        }
    }
}

/// Run an async database operation from a synchronous MCP tool handler.
#[cfg(feature = "database")]
fn run_db_async<F, Fut>(f: F) -> String
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = String>,
{
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f()))
}

/// Run an async database operation that yields a `Result<T, String>`.
///
/// Used by tools that need to thread a fetched value back into a sync
/// pipeline (e.g. resolving `degree_id` to YAML before calling validate).
#[cfg(feature = "database")]
fn run_db_async_result<T, F, Fut>(f: F) -> Result<T, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f()))
}

/// Return a standardized error JSON when the database is not configured.
#[cfg(feature = "database")]
fn db_not_configured_response(tool: &str) -> String {
    serde_json::json!({
        "error": "Database not configured",
        "tool": tool,
        "suggestion": "Set `endpoint`, `anon_key`, and `enabled = true` in the [database] section of your nuanalytics config file."
    })
    .to_string()
}

impl Default for NuAnalyticsMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_handler]
impl ServerHandler for NuAnalyticsMcpServer {
    fn get_info(&self) -> ServerInfo {
        #[cfg(feature = "database")]
        let db_section = if self.db.is_some() {
            "\n\n## Database Query Workflow (IPEDS data available)\n\
            \n\
            Discover codes first (optional):\n\
            - get_lookup_codes(\"carnegie_class\") → R1=15, R2=16, R1-2021=21\n\
            - get_lookup_codes(\"award_levels\")   → associate=3, bachelors=5, masters=7, doctoral=9\n\
            - search_cip_codes(\"computer science\") or prefix=\"11.\" → find CIP codes\n\
            \n\
            Find institutions:\n\
            - search_institutions(carnegie_class=15, state=\"MA\") → list with UNITIDs\n\
            - get_institution(unitid=167358) → full institution details\n\
            \n\
            Query completion demographics:\n\
            - get_institution_completions(unitid=167358, year=2024, cip_prefix=\"11.\") → per-CIP rows + representation ratios\n\
            - get_schools_completion_demographics(carnegie_class=15, inst_size_min=2, cip_prefix=\"11.\", year=2024)\n\
            \t→ per-school CS demographics for all R1 schools >1000 students (3 DB calls, not 130)\n\
            - get_completion_demographics(carnegie_class=15, award_level=5, year=2024) → aggregate across all matched schools\n\
            \n\
            Degree programs:\n\
            - search_degrees(unitid=167358) / get_degree(unitid=167358, cip_code=\"11.0101\") → retrieve stored degrees\n\
            - search_degrees + get_degree → read-only DB access; storage of new degrees is not yet provisioned"
        } else {
            "\n\nDatabase tools: Not available (database not configured or disabled)"
        };

        #[cfg(not(feature = "database"))]
        let db_section = "";

        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            format!(
                "NuAnalytics MCP Server\n\
                \n\
                ## Degree YAML Workflow\n\
                1. get_degree_schema — understand the YAML format\n\
                2. Build a degree YAML\n\
                3. validate_degree — check for structural errors\n\
                4. Fix issues; repeat until valid\n\
                5. audit_degree — comprehensive quality check (missing prereqs, deep chains)\n\
                6. analyze_degree — plan generation and metrics (JSON)\n\
                7. generate_degree_report — full HTML report (same artifact as `degree --analyze`); optional CSV / JSONL outputs via output_dir\
                {db_section}"
            ),
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
pub async fn run_server(db_config: &DatabaseConfig) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Starting NuAnalytics MCP server...");

    #[cfg(feature = "database")]
    let server = {
        let db = match DbClient::from_config(db_config) {
            Ok(client) => {
                eprintln!("Database client initialized.");
                Some(Arc::new(client))
            }
            Err(e) => {
                eprintln!("Database unavailable: {e}");
                None
            }
        };
        NuAnalyticsMcpServer::with_db(db)
    };

    #[cfg(not(feature = "database"))]
    let server = {
        let _ = db_config;
        NuAnalyticsMcpServer::new()
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
pub fn run(db_config: &DatabaseConfig) -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| format!("Failed to create tokio runtime: {e}"))?;

    rt.block_on(run_server(db_config))
        .map_err(|e| format!("MCP server error: {e}"))
}
