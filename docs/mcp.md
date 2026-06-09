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

After saving the config, restart Claude Code. The `nuanalytics` tools (see
[Available Tools](#available-tools) below) will appear in your session. You
can verify with `/mcp` to list connected servers.

### Other MCP Clients

The server uses stdio transport, compatible with any MCP client. Configure your client to:
1. Run the command: `nuanalytics mcp`
2. Communicate via stdin/stdout
3. Use JSON-RPC 2.0 protocol

## Available Tools

> **Input formats.** The degree tools (`validate_degree`, `audit_degree`,
> `trim_degree`, `analyze_degree`, `get_course_detail`) accept YAML, unified
> JSON, or raw ai-landscape JSON in their `yaml_content` parameter — the
> format is auto-detected, and ai-landscape shapes are converted on the fly.
> They also accept a `cache:<hash>` handle (from `cache_yaml` or
> `convert_degree`) as `degree_id`. When the input is ai-landscape JSON, any
> assumptions made during conversion are reported as `conversion_warnings`.

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

### `get_degree_json_schema`

Returns the machine-validatable JSON Schema (draft 2020-12) for the unified
degree format — the structure `convert_degree` produces and that
`validate_degree` / `analyze_degree` / `trim_degree` accept. Use it to
validate a unified degree document or to understand the format
programmatically, including wildcard `from` pools (e.g. pattern `"CS:2500+"`
or `"*:*"`). It is the same schema the CLI emits via `degree schema`.

This is distinct from `get_degree_schema`, which returns the *human-readable
YAML* reference.

**Parameters:** none.

**Example Prompts:**
- "Give me the JSON Schema for a degree" → calls `get_degree_json_schema`
- "What does a valid unified degree document look like?" → calls `get_degree_json_schema`

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

### `trim_degree`

Collapse prerequisite alternatives and `Select` option lists down to a single
shared shortest entry path per course, except inside protected subjects.
Equivalents groups (`{A, B, C}`) record substitutions so downstream prereq
references to dropped courses get rewritten, and the dropped courses are then
orphan-pruned. Pattern-pool members (e.g. `ICS:400+` electives) survive the
prune even when no requirement names them explicitly.

**Parameters:**
| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `yaml_content` / `yaml_path` / `degree_id` | string | Yes (exactly one) | YAML source. `degree_id` accepts `cache:<hash>` handles from prior tool calls. |
| `keep_all` | array of strings | No | Subject prefixes to protect in addition to the degree's declared `major_subjects`. Case-insensitive. |
| `include` | array of strings | No | Course keys to pin as winners at any choice point that lists them. Overrides the shortest-path metric. |
| `output_path` | string | No | Optional disk write target. The trimmed content is also returned inline. Refuses to overwrite a `yaml_path` input. |

**Response Format:**
```json
{
  "success": true,
  "trimmed_yaml": "degree:\n  ...",
  "trimmed_cache_id": "cache:7f3a...",
  "output_path": null,
  "report": {
    "protected_subjects": ["CS"],
    "protected_subjects_derived": false,
    "orphan_courses_removed": ["MATH241", "MATH242"]
  },
  "tool_followups": [
    { "tool": "validate_degree", "reason": "...", "suggested_args": { "degree_id": "cache:7f3a..." } },
    { "tool": "audit_degree",    "reason": "...", "suggested_args": { "degree_id": "cache:7f3a..." } }
  ]
}
```

**Notes:**
- Comments in the source YAML are not preserved on serialisation.
- `trimmed_cache_id` is a fresh handle into the YAML cache; pipe it straight
  into `validate_degree`, `audit_degree`, etc. as a `degree_id`.

**Example Prompts:**
- "Trim this degree to a single path through alternatives" → calls `trim_degree`
- "Also keep all MATH alternatives" → calls with `keep_all: ["MATH"]`
- "Force MATH241 wherever possible" → calls with `include: ["MATH241"]`

### `convert_degree`

Converts an ai-landscape program JSON into the unified NuAnalytics degree
JSON (the same format `analyze_degree` / `validate_degree` accept). Maps
category lists and picklists to requirements, flips AND-of-OR prerequisites
into the internal expression tree, and defaults missing credits (reported as
`conversion_warnings`).

**Parameters:**
| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `json_content` | string | One of `json_content` / `json_path` | Inline ai-landscape (or already-unified) program JSON |
| `json_path` | string | One of `json_content` / `json_path` | Path to a JSON file on the MCP server's filesystem |
| `program` | string | No | For a multi-program *cluster* pipeline file: the program to convert. Omit to get the cluster's program inventory instead. |
| `pretty` | boolean | No | Pretty-print the `unified_json` body (default compact) |

**Response Format:**
```json
{
  "success": true,
  "kind": "single",
  "program_count": 1,
  "unified_json": "{ ... unified degree JSON ... }",
  "conversion_warnings": ["Course 'CS101' missing credits; assumed 3"],
  "cache_id": "cache:9f3a…",
  "note": "Pass cache_id as degree_id to validate_degree / analyze_degree to chain."
}
```

The `cache_id` is a `cache:<hash>` handle for the converted body — pass its
value to `validate_degree` / `analyze_degree` / `audit_degree` / `trim_degree`
(as their `degree_id`) to chain without re-pasting the JSON. For a cluster
pipeline file with no `program` set, `kind` is `"cluster"` and a `programs`
array lists the available program names (pass one back as `program`).

**Example Prompts:**
- "Convert this ai-landscape program to the unified format" → calls with `json_content: "…"`
- "What programs are in this cluster file?" → calls with `json_path: "…"`, no `program`
- "Convert the BSCS program from that cluster and analyze it" → `convert_degree` then `analyze_degree` with the returned `degree_id`

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

### `degree_pipeline`

Runs `validate_degree` → `audit_degree` → `analyze_degree` in one call.
Short-circuits on a YAML parse error and surfaces a combined
`{validate, audit?, analyze?}` response. Saves three round-trips for the
common "look at this degree" prompt.

**Key parameters:** same yaml-source modes as the individual tools, plus
`skip_audit` / `skip_analyze` to stop at validate or audit when that's
all you need.

### `get_course_detail`

Returns everything an LLM typically wants to know about one course:
title / credits / level, raw + direct + transitive prerequisites,
dependents, requirements that reference it, cross-listed equivalents,
and (when `include_analysis=true`, default) per-course metric medians +
term placement in every selected plan. Use this instead of
`analyze_degree` when the question is "tell me about CS370 in this
degree."

**Key parameters:** `course_id` (required), `include_analysis` (default
true), `max_plans` (analysis depth cap), plus the standard yaml-source
fields.

### `generate_degree_report`

Build the full HTML degree-analysis report — the same artifact the CLI
`degree analyze` command produces. Set `output_dir` to also write CSV /
JSONL / index companions to disk; otherwise the rendered HTML is
returned inline (~200–300 KB for a typical degree).

**Key parameters:** `output_dir` (optional), `write_plan_csvs` /
`write_jsonl_summary` / `write_index_csv` (default true in disk mode),
`return_html_inline` (override to keep inline output when `output_dir`
is set), plus the analyze knobs (`max_plans`, `include_courses`).

### `render_plan_graph`

Render the prerequisite-graph HTML for one selected plan in a single
call (analyze + extract spec + visualize). Pick a plan via
`plan_category` (`"shortest"` / `"longest"` / `"calc-ready-shortest"`
/ `"sample"`) or by 0-indexed `plan_index`. `format` defaults to
`"standalone"`; use `"fragment"` to embed in another document. Set
`dry_run=true` to skip the HTML payload and just probe size.

### `find_courses_matching`

Resolve a list of include/exclude patterns against the courses defined
in a YAML and return the matched courses with titles + levels. Uses the
same pattern grammar as `select` requirements (e.g. `"CS:300+"`,
`"MATH:300-499"`). Useful when sketching a new pattern-based
requirement and previewing the pool.

### `list_sample_degrees`

List the bundled sample degree YAMLs (CSU, NEU Khoury, UH Manoa). Default
is metadata-only; pass `include_yaml=true` to also receive the full
embedded YAML body so you can pipe it into another tool.

### `cache_yaml`

Cache an inline YAML body server-side and return a `cache:<hash>`
handle. Any tool that accepts `degree_id` will resolve the handle back
to the body — removes the per-call repaste tax for hosted MCP clients
whose filesystem the server can't see. Idempotent (same body → same
handle) with a ~1 h TTL.

### `get_curriculum_visualization`

Lower-level companion to `render_plan_graph` — renders a curriculum
graph from a pre-computed `graph_spec` (the one `analyze_degree`
returns when `include_graph_spec=true`). Use this when you've already
analysed the degree and want to render multiple plans without re-running
the pipeline.

## Database-backed tools

> **Every tool in this section requires `nuanalytics db login`.** With
> the v0.4.0 auth-required RLS model, the MCP server skips registering
> DB tools entirely when no valid session is available — they won't
> appear in the tool list until you sign in and restart the server.

These tools query the Supabase-backed IPEDS data and stored degrees.
Each accepts a structured request and returns JSON. Brief inventory:

| Tool | Purpose |
|---|---|
| `search_institutions` | Filter institutions by name / state / Carnegie class / control / HBCU / tribal / size |
| `get_institution` | Full details for one institution by UNITID |
| `search_cip_codes` | Look up CIP program codes by title keyword or code prefix |
| `get_lookup_codes` | Decode lookup tables (carnegie_class, award_levels, …) into label maps |
| `search_degrees` | Stored programs (the normalized `programs` table) filtered by UNITID / CIP prefix / catalog year / `degree_type` / `program_kind` / `discipline` |
| `get_degree` | Fetch one stored program by `program_key` → `degree_id` → natural key; returns the lossless unified-JSON `document` |
| `import_degree` | Import a report/degree JSON into the `programs` tables (the supported write path) |
| `compare_degrees` | Diff stored programs and/or inline YAMLs side-by-side with analyze metrics |
| `get_institution_completions` | Per-CIP completion counts for one institution / year, with representation ratios |
| `get_completion_demographics` | Aggregate completion demographics across institutions matching filters |
| `get_schools_completion_demographics` | Per-school demographics for a Carnegie / award-level cohort |
| `scaffold_degree_yaml` | Generate a minimal YAML skeleton from UNITID + CIP (writes nothing) |

Run any of these from Claude with natural-language prompts; the model
picks the right tool and call shape automatically. The descriptions on
each tool's `#[tool]` attribute in `src/mcp/server.rs` are the
authoritative source — Claude reads them at handshake time.

> **Resolving a UNITID.** `search_institutions(name=…)` returns the
> `unitid` to pass to `import_degree` (or the CLI `db import --unitid`). The
> name match is a case-insensitive substring, so narrow an ambiguous name with
> `state` (e.g. `search_institutions(name="Boston", state="MA")`).

### `import_degree`

Imports a degree-first analysis report (or a plain unified degree) into the
normalized program tables — the MCP counterpart of the CLI `db import` command.
One report populates the program projection (`programs`, `courses`,
`program_courses`, `program_requirements`) plus, when it carries an `analysis`
block, one analysis run with its course metrics and selected plans. The
institution is resolved against IPEDS (the report's `unitid`, then a name + CIP
lookup); an ambiguous institution name returns the candidate `(unitid, name)`
pairs so the agent can re-call with an explicit `unitid`.

**Parameters:**
| Parameter | Type | Required | Description |
|-----------|------|----------|-------------|
| `json_content` | string | One of `json_content` / `json_path` | Inline degree-first report (or unified degree) JSON |
| `json_path` | string | One of `json_content` / `json_path` | Path to a report JSON file on the MCP server's filesystem |
| `variant` | string | No | Analysis-run variant label (default `"full"`). `"full"` writes the program projection; a non-full variant only attaches an analysis run |
| `unitid` | integer | No | Override the resolved IPEDS UNITID — set this to disambiguate after an `institution_ambiguous` result |
| `institution` | string | No | Override the institution name used for resolution / display |
| `cip` | string | No | Override the CIP code (part of the natural program key) |
| `catalog` | string | No | Override the catalog year (part of the program identity) |
| `degree_id` | string | No | Override the degree id |
| `force` | boolean | No | Overwrite a verified program / skip the confirmation gate |
| `replace` | boolean | No | Replace an existing (unverified) program |
| `skip_existing` | boolean | No | Skip the program entirely if it already exists |
| `dry_run` | boolean | No | Preview the import (report counts) without writing anything |

**Response Format:**
```json
{
  "result": "created",
  "program_key": "prog:167358|11.0701|2025-2026|BS",
  "resolved_unitid": 167358,
  "institution": "Northeastern University",
  "variant": "full",
  "variations_run": 10000,
  "sample_type": "shuffled",
  "courses_written": 82,
  "requirements_written": 32,
  "run_written": true,
  "plans_written": 7,
  "course_metrics_written": 72,
  "conversion_warnings": [],
  "messages": ["..."]
}
```

`result` is a stable lowercase tag: `created` | `updated` | `skipped` |
`needs_confirmation` | `institution_ambiguous` | `rejected`. The blocked
variants attach an extra payload:

- `institution_ambiguous` → `institution_candidates: [{ "unitid": …, "name": … }]`
  (re-call with one as `unitid`)
- `needs_confirmation` → `reason` (the program exists / is verified; re-call
  with `replace` or `force`)
- `rejected` → `errors` (the report could not be turned into a valid plan)

**Notes:**
- DB-gated — registered only with a logged-in session, like the other
  database-backed tools.
- `dry_run: true` runs the full resolution + plan build and returns the row
  counts, but writes nothing.

**Example Prompts:**
- "Import this analysis report into the database" → calls with `json_path: "…"`
- "Preview importing this degree without writing" → calls with `dry_run: true`
- "That matched two schools — use UNITID 167358" → re-calls with `unitid: 167358`

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
cargo run -- degree validate samples/degrees/neu-khoury-bscs-boston.yaml

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
