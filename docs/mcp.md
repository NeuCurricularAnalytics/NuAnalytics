# MCP Server

NuAnalytics includes an MCP (Model Context Protocol) server that allows AI models to validate degree program YAML files interactively. This enables AI assistants like Claude to help build and validate curriculum definitions.

## Table of Contents

- [Quick Start](#quick-start)
- [Installation](#installation)
- [Configuration](#configuration)
- [Available Tools](#available-tools)
- [Testing](#testing)
- [Workflow Example](#workflow-example)
- [Error & Warning Reference](#error--warning-reference)
- [Architecture](#architecture)
- [Troubleshooting](#troubleshooting)

## Quick Start

```bash
# Install (MCP is included by default)
cargo install nu-analytics

# Start the MCP server
nuanalytics mcp
```

The server communicates via stdio (standard input/output) using the MCP JSON-RPC protocol.

## Installation

### Via Cargo (Recommended)

Install the CLI globally so `nuanalytics` is available on your PATH:

```bash
cargo install nu-analytics
```

MCP support is enabled by default. To build *without* MCP (smaller binary):

```bash
cargo install nu-analytics --no-default-features --features log-info,log-debug,verbose,file-logging
```

Verify the installation:

```bash
nuanalytics --help
```

### From Git (Latest)

```bash
cargo install --git https://github.com/NeuCurricularAnalytics/NuAnalytics --bin nuanalytics
```

### Local Development Setup

For development and testing, you can run the MCP server directly from a local checkout:

```bash
git clone https://github.com/NeuCurricularAnalytics/NuAnalytics.git
cd NuAnalytics

# Run directly (rebuilds as needed)
cargo run -- mcp

# Or build release and run
cargo build --release
./target/release/nuanalytics mcp
```

## Configuration

### Claude Desktop Integration

Add NuAnalytics to your Claude Desktop configuration:

**Linux/macOS**: `~/.config/claude-desktop/config.json`
**Windows**: `%APPDATA%\claude-desktop\config.json`

```json
{
  "mcpServers": {
    "nuanalytics": {
      "command": "nuanalytics",
      "args": ["mcp"]
    }
  }
}
```

> **Note**: If `nuanalytics` is not on your PATH, use the absolute path instead.
> Find it with `which nuanalytics` or `realpath ./target/release/nuanalytics`.

### Claude Code (CLI) Integration

Add NuAnalytics as an MCP server in your Claude Code settings. You can configure it at
the project level (`.claude/settings.json`) or user level (`~/.claude/settings.json`):

```json
{
  "mcpServers": {
    "nuanalytics": {
      "command": "nuanalytics",
      "args": ["mcp"]
    }
  }
}
```

For development, point to a local checkout:

```json
{
  "mcpServers": {
    "nuanalytics-dev": {
      "command": "cargo",
      "args": ["run", "--features", "mcp", "--", "mcp"],
      "cwd": "/path/to/NuAnalytics"
    }
  }
}
```

After saving the config, restart Claude Code. The `get_degree_schema`, `validate_degree`,
and `audit_degree` tools will be available in your session. You can verify with `/mcp` to
list connected servers.

### Other MCP Clients

The server uses stdio transport, compatible with any MCP client. Configure your client to:
1. Run the command: `nuanalytics mcp`
2. Communicate via stdin/stdout
3. Use JSON-RPC 2.0 protocol

## Available Tools

### `get_degree_schema`

Returns documentation about the degree YAML format.

**Parameters:**
| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `section` | string | No | Section filter: `"all"` (default), `"degree"`, `"requirements"`, `"courses"`, or `"examples"` |

**Example Prompts:**
- "Show me the degree schema" → calls with `section: "all"`
- "What fields go in the degree section?" → calls with `section: "degree"`
- "Give me an example degree YAML" → calls with `section: "examples"`

### `validate_degree`

Validates a degree program YAML string and returns detailed feedback.

**Parameters:**
| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `yaml_content` | string | Yes | Complete degree program YAML content |

**Response Format:**
```json
{
  "is_valid": false,
  "parse_error": null,
  "errors": [
    {
      "error_type": "MissingCourse",
      "message": "Course 'CS101' referenced in requirement 'intro' but not defined",
      "suggestion": "Add 'CS101' to the courses section, or remove it from requirement 'intro'."
    }
  ],
  "warnings": [
    {
      "warning_type": "IsolatedCourse",
      "message": "Course 'ELEC100' has no prerequisites and nothing depends on it"
    }
  ],
  "context": {
    "degree_name": "BS Computer Science",
    "institution": "Example University",
    "total_courses": 15,
    "total_requirements": 4,
    "defined_courses": ["CS101", "CS102", "..."],
    "defined_requirements": ["intro", "core", "..."]
  },
  "suggestions": [
    "Fix the errors above and re-validate.",
    "Currently defined courses: CS101, CS102, ..."
  ]
}
```

### `audit_degree`

Runs a comprehensive audit on a degree program YAML. Combines validation with
structural analysis: missing prerequisites on upper-level courses and deep prerequisite chains.

**Parameters:**
| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `yaml_content` | string | Yes | Complete degree program YAML content |
| `chain_threshold` | integer | No | Minimum chain depth to flag (default: 3) |

**Response Format:**
```json
{
  "passed": false,
  "validation_errors": 2,
  "validation_warnings": 1,
  "validation_report": "...",
  "missing_prerequisites": [
    { "course": "CS3500", "level": 3000 }
  ],
  "deep_chains": [
    {
      "course": "CS4500",
      "max_depth": 5,
      "branch_lengths": "5, 3",
      "chain": "CS1000 → CS2500 → CS2510 → CS3500 → CS4500"
    }
  ],
  "chain_threshold": 3,
  "degree_name": "BS Computer Science",
  "institution": "Example University",
  "total_courses": 25
}
```

**Example Prompts:**
- "Audit this degree for issues" → calls `audit_degree`
- "Find courses with long prerequisite chains" → calls with `chain_threshold: 2`
- "Are there any upper-level courses missing prerequisites?" → calls `audit_degree`

### `analyze_degree`

Runs full degree analysis: generates all possible course plans, computes aggregate
metrics, and identifies shortest/longest paths with term-by-term schedules.

**Parameters:**
| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `yaml_content` | string | Yes | Complete degree program YAML content |
| `max_plans` | integer | No | Maximum plans to generate (default: 500) |
| `include_courses` | string | No | Comma-separated course codes to include in all plans (e.g., "CS150B,MATH156") |

**Response Format:**
```json
{
  "success": true,
  "degree_name": "BS Computer Science",
  "institution": "Example University",
  "total_courses": 45,
  "total_requirements": 12,
  "plans_analyzed": 500,
  "was_truncated": true,
  "complexity": { "min": 120.0, "q1": 150.0, "median": 168.0, "q3": 190.0, "max": 240.0, "mean": 170.0, "std_dev": 25.0 },
  "longest_delay": { "min": 4.0, "q1": 5.0, "median": 5.0, "q3": 6.0, "max": 6.0, "mean": 5.2, "std_dev": 0.5 },
  "total_credits": { "min": 134.0, "q1": 138.0, "median": 140.0, "q3": 144.0, "max": 156.0, "mean": 141.0, "std_dev": 4.0 },
  "selected_plans": [
    {
      "category": "Shortest Path",
      "terms": 8,
      "complexity": 145,
      "longest_delay": 5,
      "critical_path": ["CS2000", "CS2100", "CS3500", "CS4500"],
      "credits": 138.0,
      "course_count": 38,
      "schedule": [
        { "term": 1, "courses": ["CS2000", "CS2001", "CS1800", "CS1802"], "credits": 14.0 }
      ]
    }
  ]
}
```

**Example Prompts:**
- "Analyze this degree program" → calls `analyze_degree`
- "What is the shortest path through this degree?" → calls `analyze_degree`
- "How complex is this curriculum?" → calls `analyze_degree`
- "Analyze this degree and write a full curricular analytics report with metrics, shortest/longest paths, and recommendations for improving the curriculum" → calls `analyze_degree`, then formats results into a detailed report

## Testing

### Using MCP Inspector (Recommended)

The MCP Inspector provides an interactive web UI for testing:

```bash
# Install and run the inspector
npx @modelcontextprotocol/inspector cargo run -- mcp
```

This opens a browser where you can:
- View available tools and their schemas
- Call tools with custom parameters
- Inspect JSON-RPC request/response pairs
- Debug issues interactively

### Manual Testing with JSON-RPC

You can test the server directly using stdin:

```bash
# Start the server
cargo run -- mcp

# In another terminal, send a JSON-RPC request:
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' | \
  cargo run -- mcp
```

### Unit Tests

Run the MCP module tests:

```bash
# Run all MCP tests
cargo test mcp::

# Run specific tool tests
cargo test mcp::tools::schema
cargo test mcp::tools::validate
```

### Integration Testing

Test with a sample degree file:

```bash
# Validate using the CLI (for comparison)
cargo run -- degree --validate samples/degrees/neu-khoury-bscs-boston.yaml

# The MCP validate_degree tool uses the same validation logic
```

## Workflow Example

A typical workflow for building a degree with an AI assistant:

### 1. Get Schema Documentation

```
User: "I want to create a degree YAML for a BS in Computer Science at Example University"

AI: [calls get_degree_schema with section="all"]
"Here's how to structure a degree YAML file..."
```

### 2. Create Initial Draft

```
AI: "Based on your requirements, here's an initial draft:

degree:
  id: example-bscs-2025
  institution: Example University
  program: Bachelor of Science in Computer Science
  total_credits: 120
  gpa_minimum: 2.0
  ...
"
```

### 3. Validate and Iterate

```
AI: [calls validate_degree with the YAML]

"The validation found 2 issues:
❌ Error: Course 'MATH151' is referenced in 'math_req' but not defined
⚠️ Warning: Course 'CS401' has no prerequisites (unusual for a 400-level course)

Let me fix the error by adding MATH151 to the courses section..."
```

### 4. Repeat Until Valid

```
AI: [calls validate_degree with fixed YAML]

"✓ Degree program is valid!

Summary:
- 45 courses defined
- 12 requirements
- No circular prerequisites
- 2 warnings to consider (isolated courses)

Would you like me to address the warnings or save this as your final degree.yaml?"
```

### 5. Audit for Structural Issues

```
User: "Run an audit on this degree"

AI: [calls audit_degree with the YAML]

"The audit found a few things to review:
- 3 upper-level courses have no prerequisites (CS301, CS302, CS401)
- CS401 has a deep prerequisite chain of 5 courses: CS101 → CS102 → CS201 → CS301 → CS401

These aren't errors, but you may want to verify CS301/CS302 intentionally have no prereqs."
```

### 6. Analyze and Generate Report

```
User: "Analyze this degree and give me a full curricular analytics report with
your findings, including the shortest and longest paths, metrics summary,
and any recommendations for improving the curriculum."

AI: [calls analyze_degree with the YAML]

"# Curricular Analytics Report: BS Computer Science — Example University

## Overview
- **Plans analyzed**: 500 (of ~2,400 possible)
- **Total courses**: 45 across 12 requirements

## Aggregate Metrics
| Metric | Min | Q1 | Median | Q3 | Max | Mean | Std Dev |
|--------|-----|-----|--------|-----|-----|------|---------|
| Complexity | 120 | 150 | 168 | 190 | 240 | 170 | 25 |
| Longest Delay | 4 | 5 | 5 | 6 | 6 | 5.2 | 0.5 |
| Total Credits | 134 | 138 | 140 | 144 | 156 | 141 | 4.0 |

## Shortest Path (8 terms, complexity 145)
Critical path: CS2000 → CS2100 → CS3500 → CS4500

| Term | Courses | Credits |
|------|---------|---------|
| 1 | CS2000, CS2001, CS1800, CS1802, DS2000 | 14 |
| 2 | CS2100, CS2101, MATH1341, DS2500 | 17 |
| ... | ... | ... |

## Longest Path (9 terms, complexity 220)
Critical path: CS2000 → CS2100 → CS3100 → CS3650 → CY4740 → CY4770

## Recommendations
1. **High-complexity bottleneck**: CS3100 appears on most critical paths — consider
   adding an alternative prerequisite path to reduce delay.
2. **Credit balance**: Median credits (140) are close to the minimum (134), meaning
   most students take near-minimum courseloads.
3. **Prerequisite chain depth**: The longest chain is 6 courses — students who fall
   behind on early courses face significant delays."
```

This workflow shows how the AI can use the structured JSON from `analyze_degree` to
produce a human-readable curriculum report with tables, recommendations, and insights.

## Error & Warning Reference

### Error Types

| Error Type | Description | Common Fix |
|------------|-------------|------------|
| `CircularPrerequisite` | Course prerequisites form a cycle (A→B→C→A) | Remove one prerequisite to break the cycle |
| `MissingCourse` | Course referenced in requirement doesn't exist | Add the course to `courses:` section |
| `MissingPrerequisite` | Prerequisite course not defined | Add prerequisite to `courses:` or fix the reference |
| `MissingCorequisite` | Corequisite course not defined | Add corequisite to `courses:` section |
| `InvalidPattern` | Pattern syntax is malformed | Use format like `CS:3000+` or `MATH:300-499` |
| `PatternMatchesNoCourses` | Pattern doesn't match any defined courses | Add courses matching the pattern or fix syntax |
| `InvalidRequirement` | Requirement has invalid configuration | Check requirement type and required fields |
| `UnidirectionalCrossListing` | Cross-listing is not bidirectional | Add reciprocal `cross_listed_as` entry |

### Warning Types

| Warning Type | Description | Recommendation |
|--------------|-------------|----------------|
| `UnreferencedCourse` | Course defined but never used | Remove if unneeded, or add to a requirement |
| `IsolatedCourse` | No prerequisites and nothing depends on it | Verify this is intentional (e.g., elective) |
| `BroadPattern` | Pattern matches many courses | Consider more specific pattern |
| `HiddenRequirement` | Course implicitly required via prerequisites | Consider adding to explicit requirements |
| `HiddenRequirementOption` | Prerequisite options not in degree | Add at least one option to requirements |
| `MissingCrossListedCourse` | Cross-listed course doesn't exist | Add the cross-listed course or remove reference |

## Architecture

The MCP server is organized as a library module that can be used independently:

```
src/mcp/
├── mod.rs              # Module exports and documentation
├── server.rs           # MCP server handler and entry points
├── schema_content.rs   # Static schema documentation
└── tools/
    ├── mod.rs          # Tool exports
    ├── schema.rs       # get_degree_schema implementation
    └── validate.rs     # validate_degree implementation
```

### Programmatic Usage

You can use the MCP module directly in Rust:

```rust
use nu_analytics::mcp;

// Run the server (blocking)
mcp::run()?;

// Or use the async version
mcp::run_server().await?;

// Use tools directly
use nu_analytics::mcp::tools::validate;
let response = validate::execute(yaml_content);
```

## Troubleshooting

### Server Won't Start

```
Error: Failed to create tokio runtime
```
**Solution**: MCP is enabled by default. If you built with `--no-default-features`, add `mcp` back: `cargo build --features mcp`

### Claude Desktop Doesn't See the Server

1. Check the config file path is correct for your OS
2. Verify the command path is absolute
3. Restart Claude Desktop after config changes
4. Check Claude Desktop logs for errors

### Validation Returns Parse Errors

```json
{"parse_error": "YAML syntax error: ..."}
```
**Solution**: Check YAML syntax. Common issues:
- Missing required fields (`prefix`, `number` for courses)
- Incorrect indentation
- Missing quotes around numbers in strings

### Tools Not Appearing

If tools don't appear in the MCP Inspector:
1. Verify server starts without errors
2. Check that initialization completes (look for "Waiting for requests..." message)
3. Try restarting the inspector

### Debug Mode

Enable debug logging for more information:

```bash
nuanalytics --log-level debug mcp 2>debug.log
```

## See Also

- [Degree Command Documentation](degree.md) - CLI-based degree validation
- [Schema Reference](../samples/claude-project-reference/schema-v5.2.yaml) - Full YAML schema
- [Sample Degrees](../samples/degrees/) - Example degree YAML files
- [MCP Specification](https://modelcontextprotocol.io/) - Official MCP documentation
