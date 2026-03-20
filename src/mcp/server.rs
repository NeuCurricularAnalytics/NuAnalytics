//! MCP server implementation
//!
//! This module provides the MCP server that exposes `NuAnalytics` tools.

use crate::mcp::tools::{
    audit, schema, validate, AuditDegreeRequest, GetSchemaRequest, ValidateDegreeRequest,
};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
    ServerHandler, ServiceExt,
};

// ============================================================================
// MCP Server Implementation
// ============================================================================

/// `NuAnalytics` MCP Server
///
/// Provides tools for validating degree program YAML files and retrieving schema documentation.
#[derive(Debug, Clone)]
pub struct NuAnalyticsMcpServer {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl NuAnalyticsMcpServer {
    /// Create a new MCP server instance
    #[must_use]
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

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
}

impl Default for NuAnalyticsMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

#[tool_handler]
impl ServerHandler for NuAnalyticsMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "NuAnalytics MCP Server - Tools for building and validating degree program YAML files.\n\n\
            Available tools:\n\
            - get_degree_schema: Get documentation about the degree YAML format\n\
            - validate_degree: Validate a degree YAML and get detailed feedback\n\
            - audit_degree: Comprehensive audit (validation + missing prereqs + deep chains)\n\n\
            Typical workflow:\n\
            1. Call get_degree_schema to understand the format\n\
            2. Build a degree YAML\n\
            3. Call validate_degree to check for errors\n\
            4. Fix issues based on feedback\n\
            5. Repeat steps 3-4 until valid\n\
            6. Call audit_degree for a comprehensive quality check"
                .to_string(),
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
pub async fn run_server() -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("Starting NuAnalytics MCP server...");

    let service = NuAnalyticsMcpServer::new()
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
pub fn run() -> Result<(), String> {
    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| format!("Failed to create tokio runtime: {e}"))?;

    rt.block_on(run_server())
        .map_err(|e| format!("MCP server error: {e}"))
}
