//! MCP server implementation
//!
//! This module provides the MCP server that exposes `NuAnalytics` tools.

use std::sync::Arc;

use crate::core::config::DatabaseConfig;
use crate::mcp::tools::{
    analyze, audit, schema, validate, AnalyzeDegreeRequest, AuditDegreeRequest, GetSchemaRequest,
    ValidateDegreeRequest,
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
    completions, degrees, institutions, CompareDegreesRequest, CompletionDemographicsRequest,
    GetDegreeRequest, SearchDegreesRequest, SearchInstitutionsRequest,
};

// ============================================================================
// MCP Server Implementation
// ============================================================================

/// `NuAnalytics` MCP Server
///
/// Provides tools for validating degree program YAML files and retrieving schema documentation.
/// When database integration is enabled, also provides tools for querying IPEDS data.
#[derive(Debug, Clone)]
pub struct NuAnalyticsMcpServer {
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

    // ---- Degree tools (no DB required) ----

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
        description = "Validate a degree program YAML string. Returns detailed validation results including errors, warnings, and suggestions for fixing issues. Use this iteratively to build a valid degree.yaml file."
    )]
    #[allow(clippy::unused_self)]
    fn validate_degree(&self, Parameters(req): Parameters<ValidateDegreeRequest>) -> String {
        validate::execute_json(&req.yaml_content)
    }

    /// Audit a degree program YAML
    #[tool(
        description = "Run a comprehensive audit on a degree program YAML. Includes validation, detection of upper-level courses missing prerequisites, and identification of deep prerequisite chains. Returns structured results with actionable findings."
    )]
    #[allow(clippy::unused_self)]
    fn audit_degree(&self, Parameters(req): Parameters<AuditDegreeRequest>) -> String {
        audit::execute_json(&req.yaml_content, req.chain_threshold)
    }

    /// Analyze a degree program YAML
    #[tool(
        description = "Run full degree analysis: generate all possible course plans, compute aggregate metrics (complexity, delay, credits), and identify shortest/longest paths. Returns statistics across plans and term-by-term schedules for selected plans. Use after validate_degree confirms the YAML is valid. Optionally specify include_courses to constrain all plans to include specific courses."
    )]
    #[allow(clippy::unused_self)]
    fn analyze_degree(&self, Parameters(req): Parameters<AnalyzeDegreeRequest>) -> String {
        let include_courses = req.include_courses.map(|s| {
            s.split(',')
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect()
        });
        analyze::execute_json(&req.yaml_content, req.max_plans, include_courses)
    }

    // ---- Database tools (require DB) ----

    /// Search institutions from the IPEDS database
    #[cfg(feature = "database")]
    #[tool(
        description = "Search institutions from the IPEDS database. Filter by name, state, Carnegie classification (e.g. 15=R1, 16=R2), control type (1=public, 2=private nonprofit), HBCU status, or institution size. Returns matching institutions with their UNITID and metadata."
    )]
    fn search_institutions(
        &self,
        Parameters(req): Parameters<SearchInstitutionsRequest>,
    ) -> String {
        let db = match self.get_db("search_institutions") {
            Ok(db) => db,
            Err(e) => return e,
        };
        run_db_async(|| async move { institutions::execute_json(&db, req).await })
    }

    /// Search stored degree programs in the database
    #[cfg(feature = "database")]
    #[tool(
        description = "Search stored degree programs in the database. Filter by institution UNITID, CIP code, catalog year, or degree ID prefix. Returns matching degrees with metadata. Use get_degree to retrieve the full YAML content."
    )]
    fn search_degrees(&self, Parameters(req): Parameters<SearchDegreesRequest>) -> String {
        let db = match self.get_db("search_degrees") {
            Ok(db) => db,
            Err(e) => return e,
        };
        run_db_async(|| async move { degrees::execute_search_json(&db, req).await })
    }

    /// Retrieve a stored degree program by ID
    #[cfg(feature = "database")]
    #[tool(
        description = "Retrieve a full degree program YAML by its degree ID from the database. Returns the complete YAML content that can be passed to validate_degree or analyze_degree."
    )]
    fn get_degree(&self, Parameters(req): Parameters<GetDegreeRequest>) -> String {
        let db = match self.get_db("get_degree") {
            Ok(db) => db,
            Err(e) => return e,
        };
        run_db_async(|| async move { degrees::execute_get_json(&db, req).await })
    }

    /// Compare multiple stored degree programs
    #[cfg(feature = "database")]
    #[tool(
        description = "Compare multiple stored degree programs by their IDs. Returns side-by-side metadata and metrics for each degree. Provide a comma-separated list of degree IDs."
    )]
    fn compare_degrees(&self, Parameters(req): Parameters<CompareDegreesRequest>) -> String {
        let db = match self.get_db("compare_degrees") {
            Ok(db) => db,
            Err(e) => return e,
        };
        run_db_async(|| async move { degrees::execute_compare_json(&db, req).await })
    }

    /// Query completion demographics from IPEDS data
    #[cfg(feature = "database")]
    #[tool(
        description = "Query CS degree completion demographics from IPEDS data. Filter by Carnegie classification, control type (public/private), state, CIP code family, award level (5=bachelors, 7=masters, 9=doctoral), and year. Returns completion counts and representation ratios by demographic group (gender, race/ethnicity). Example: completions of women from R1 public institutions."
    )]
    fn get_completion_demographics(
        &self,
        Parameters(req): Parameters<CompletionDemographicsRequest>,
    ) -> String {
        let db = match self.get_db("get_completion_demographics") {
            Ok(db) => db,
            Err(e) => return e,
        };
        run_db_async(|| async move { completions::execute_json(&db, req).await })
    }
}

/// Extract the database client from an MCP server, returning an error JSON string if absent.
#[cfg(feature = "database")]
impl NuAnalyticsMcpServer {
    fn get_db(&self, tool_name: &'static str) -> Result<Arc<DbClient>, String> {
        self.db
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| db_not_configured_response(tool_name))
    }
}

/// Run an async database operation from a synchronous MCP tool handler.
///
/// MCP tool methods are sync, but the MCP server runs inside a multi-thread
/// tokio runtime. `block_in_place` parks the current thread as "blocking" so
/// the executor can schedule other tasks while we wait.
#[cfg(feature = "database")]
fn run_db_async<F, Fut>(f: F) -> String
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = String>,
{
    tokio::task::block_in_place(|| tokio::runtime::Handle::current().block_on(f()))
}

/// Return a standardized error JSON when the database is not configured.
#[cfg(feature = "database")]
fn db_not_configured_response(tool: &str) -> String {
    serde_json::json!({
        "error": "Database not configured",
        "tool": tool,
        "suggestion": "Set `endpoint`, `token`, and `enabled = true` in the [database] section of your nuanalytics.toml config file."
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
        let db_tools = if self.db.is_some() {
            "\n\nDatabase tools (IPEDS data available):\n\
            - search_institutions: Search institutions by name, state, Carnegie class, control type\n\
            - search_degrees: Search stored degree programs in the database\n\
            - get_degree: Retrieve a full degree YAML by degree ID\n\
            - compare_degrees: Compare multiple degree programs side-by-side\n\
            - get_completion_demographics: Query CS completion demographics and representation metrics"
        } else {
            "\n\nDatabase tools: Not available (database not configured)"
        };

        #[cfg(not(feature = "database"))]
        let db_tools = "";

        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            format!(
                "NuAnalytics MCP Server - Tools for building and validating degree program YAML files.\n\n\
                Available tools:\n\
                - get_degree_schema: Get documentation about the degree YAML format\n\
                - validate_degree: Validate a degree YAML and get detailed feedback\n\
                - audit_degree: Comprehensive audit (validation + missing prereqs + deep chains)\n\
                - analyze_degree: Full plan analysis with aggregate metrics and schedules\
                {db_tools}\n\n\
                Typical workflow:\n\
                1. Call get_degree_schema to understand the format\n\
                2. Build a degree YAML\n\
                3. Call validate_degree to check for errors\n\
                4. Fix issues based on feedback\n\
                5. Repeat steps 3-4 until valid\n\
                6. Call audit_degree for a comprehensive quality check\n\
                7. Call analyze_degree for plan generation and metrics"
            ),
        )
    }
}

// ============================================================================
// Server Entry Points
// ============================================================================

/// Run the MCP server (async)
///
/// This function starts the MCP server using stdio transport.
/// It blocks until the server is shut down.
///
/// # Errors
///
/// Returns an error if the server fails to start or encounters a fatal error.
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
/// Creates a tokio runtime and runs the async server.
/// Use this from the CLI command handler.
///
/// # Errors
///
/// Returns an error string if the server fails.
pub fn run(db_config: &DatabaseConfig) -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| format!("Failed to create tokio runtime: {e}"))?;

    rt.block_on(run_server(db_config))
        .map_err(|e| format!("MCP server error: {e}"))
}
