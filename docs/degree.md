# Degree Command

The `degree` command validates and analyzes degree program definitions from YAML files. It can validate structure, analyze prerequisite graphs, and generate comprehensive plan analysis reports with curriculum metrics.

## Overview

The `degree` command provides several modes of operation:

- **Analysis** (default): Generate all possible degree plans, compute curriculum metrics, and produce HTML reports with statistics
- **Validation**: Check course data, prerequisite structures, and requirement definitions
- **Audit**: Run validation plus identify hidden requirements and prerequisite chain issues
- **Graph Display**: Print the course prerequisite graph structure

## Quick Start

The `degree` command is a subcommand dispatcher. Each action is its own
subcommand:

```bash
# Run full degree analysis
nuanalytics degree analyze samples/degrees/csu-cs-bscs-general.yaml

# Validate a degree program
nuanalytics degree validate samples/degrees/csu-cs-bscs-general.yaml

# Run an audit report
nuanalytics degree audit samples/degrees/csu-cs-bscs-general.yaml

# Print the prerequisite graph
nuanalytics degree print-graph samples/degrees/csu-cs-bscs-general.yaml

# Trim alternatives down to a single shared shortest path
nuanalytics degree trim samples/degrees/csu-cs-bscs-general.yaml

# Convert ai-landscape program JSON to the unified degree JSON
nuanalytics degree convert program.json -o converted/

# Emit the unified-degree JSON Schema
nuanalytics degree schema -o degree.schema.json
```

> **Input formats:** every subcommand accepts both YAML and the unified
> JSON degree format, and auto-detects which on load (content starting
> with `{` or `[` is parsed as JSON, otherwise YAML). Raw ai-landscape
> program JSON is detected and converted automatically. See
> [Input File Format](#input-file-format).

> **Migration note:** prior versions exposed these as flags
> (`degree --validate`, `degree --analyze`, etc.). Every flag has moved
> to a subcommand of the same name. There is no implicit default action;
> pick one explicitly.

## Commands and Options

### Full Analysis (`analyze`)

```bash
nuanalytics degree analyze path/to/degree.yaml
```

This generates:
- All possible degree plans (respecting course choices and requirements)
- Curriculum metrics (complexity, blocking factor, delay factor, centrality)
- HTML report with box plots and statistics
- CSV exports of selected plans (shortest, longest, random samples)

**Analysis Options:**

| Option | Description | Default |
|--------|-------------|---------|
| `--max-plans <N>` | Maximum plans to generate (safety cap) | 1000 |
| `--sample-plans <N>` | Number of random plans to export | 5 |
| `--calc-strategy <S>` | Aggregation strategy: `median` or `mean` | median |
| `--sampling-strategy <S>` | Plan enumeration: `sequential`, `shuffled`, or `stratified` | shuffled |
| `--full-run` | Generate all combinations without deduplication | false |
| `--no-csv` | Skip CSV plan export | false |
| `--no-report` | Skip HTML report generation | false |
| `--report-dir <DIR>` | Override reports output directory | from config |
| `--metrics-dir <DIR>` | Override metrics output directory | from config |
| `--include <COURSES>` | Courses to always include in all plans (comma-separated) | none |
| `-j, --jobs <N>` | Files to analyze concurrently when multiple are given, each in its own process (see [Parallel analysis](#parallel-analysis)) | 8 |
| `--school <NAME>` | Treat all inputs as programs of one school and also emit a combined `<school>_school_report.json` rollup | none |

**Examples:**

```bash
# Analyze with more samples
nuanalytics degree analyze --sample-plans 20 degree.yaml

# Use mean instead of median for aggregation
nuanalytics degree analyze --calc-strategy mean degree.yaml

# Generate up to 5000 plans
nuanalytics degree analyze --max-plans 5000 degree.yaml

# Skip report generation, only export CSVs
nuanalytics degree analyze --no-report degree.yaml

# Use stratified sampling for better coverage
nuanalytics degree analyze --sampling-strategy stratified degree.yaml

# Always include specific courses in all plans
nuanalytics degree analyze --include "CS3500,MATH2331" degree.yaml
```

### Include Courses

The `--include` option allows you to specify courses that must be included in every generated plan. This is useful when:

- A student has already taken certain courses and you want to analyze plans including those courses
- You want to see how including a specific elective affects the shortest path
- You want to reduce the number of plan combinations by locking in certain choices

When an included course satisfies a requirement (e.g., it's one option in a picklist), that requirement is locked to include that course, and other options are not considered. This reduces the total number of possible plans.

```bash
# Example: Include two specific courses
nuanalytics degree analyze --include "CS3500,STAT301" samples/degrees/csu-cs-bscs-general.yaml

# The output will show:
# Plan Generation:
#   Included courses: CS3500, STAT301
#   Estimated total plans: 174579816600  (reduced from ~350 billion)
#   Variable requirements: 6  (reduced from 7)
```

`degree trim` reuses the same flag — see the **Trim** section below for trim-specific semantics.

### Parallel Analysis

When you pass **multiple** files to `analyze`, the command runs a rolling
pool of worker processes — one file per OS process, `-j/--jobs` at a time
(default 8). Process isolation matters: a pathological degree (for example a
full-catalog scrape with thousands of courses) is contained to its own
process, so if it exhausts memory or crashes, the kernel kills only that
child. The parent records the failure in `<metrics-dir>/failures.log` — one
`path<TAB>status` line, so an OOM kill shows as `signal: 9 (SIGKILL)` and is
distinguishable from a non-zero exit — and the remaining files carry on. The
parent prints a progress line and a summary; per-worker stdout/stderr is
suppressed.

```bash
# Analyze a directory of degrees, 12 at a time
nuanalytics degree analyze samples/degrees/*.yaml -j 12 --metrics-dir out/

# Roll the per-degree metrics up into one school report
nuanalytics degree analyze cs-programs/*.json --school "Example University"
```

Single-file runs, `--school`, and `-j 1` run in-process with full per-degree
console output. For an externally throttled variant (a per-process
`ulimit -v` memory cap and a timeout), see `scripts/analyze-batch.sh`; the
in-process pool deliberately imposes no ulimit or timeout.

### Validation (`validate`)

```bash
nuanalytics degree validate path/to/degree.yaml
```

Validates:
- YAML syntax and structure
- Course reference validity (courses mentioned in prerequisites exist)
- Circular prerequisite detection
- Requirement structure validation
- Cross-listing bidirectionality

### Audit (`audit`)

```bash
nuanalytics degree audit path/to/degree.yaml
```

Includes validation plus:
- Identification of upper-level courses without prerequisites
- Analysis of prerequisite chain depth
- Hidden requirements detection (courses required by prerequisites but not in requirements)
- Subject-area filtering for degree-relevant courses

### Print Graph (`print-graph`)

```bash
nuanalytics degree print-graph path/to/degree.yaml
```

Displays the prerequisite graph showing:
- All courses in the degree
- Prerequisite relationships (AND/OR)
- Expanded prerequisite options

### Trim (`trim`)

```bash
nuanalytics degree trim path/to/degree.yaml [-o <out>] [--keep-all <SUBJ>]... [--include <COURSE,...>]
```

Produces a reduced copy of the input where prerequisite alternatives and
`type: select` option lists are collapsed to a single shared shortest
entry path per course. Useful for visualization and for downstream tools
that don't enumerate alternatives.

**Default protection:** every course whose subject appears in the
degree's `major_subjects` list keeps its full set of alternatives. So on
a CS degree (`major_subjects: [CS, CY, DS, IS]`), a prereq disjunct like
`CS163 | CS164` survives unchanged, while `MATH156 | MATH160` collapses
to whichever course has the smaller graph-delay metric. If
`major_subjects` is missing from the YAML, the trim derives a protected
set from the most-referenced subject prefix in the requirements.

**Mixed disjuncts** (some protected, some not) trim to the protected
options only. Example: `CS163 | MATH156` on a CS degree → `CS163`.

**Shared shortest path:** the depth metric is upstream-only (each
candidate's own prereq tree). It's a global property of the graph, so
two courses that both list `MATH156 | MATH160` independently pick the
same alternative — no joint optimization needed. Downstream
"blocking" (how many other courses depend on a candidate) deliberately
does *not* influence the choice.

**Equivalents propagation:** when an equivalents group `{A, B, C}`
inside a `type: all` requirement collapses, dropped equivalents are
substituted into every downstream prereq expression that named them.
The dropped courses then have nothing referencing them and get pruned
in the orphan-pruning pass.

**Pattern pools survive pruning:** courses matched by a Select
requirement's `from.pattern` (or `from.include` patterns) — e.g. the
`ICS:400+` electives — are never orphan-pruned, even when no
requirement lists them by name.

**`--keep-all <SUBJ>`** protects an additional subject prefix.
Repeatable, also accepts comma-separated values:

```bash
nuanalytics degree trim degree.yaml --keep-all MATH --keep-all PHIL
nuanalytics degree trim degree.yaml --keep-all MATH,PHIL
```

**`--include <COURSE,...>`** pins specific picks at any choice point
that lists them — overrides the shortest-path metric and the
prefer-protected rule. Same semantics as the `analyze --include` flag,
repurposed for trim:

```bash
# Force MATH2331 to win wherever it's offered as an alternative
nuanalytics degree trim degree.yaml --include MATH2331
```

**Output:** defaults to `<input-stem>_trimmed.<ext>` next to each input
file; `-o`/`--out` overrides:

- `-o <FILE>` writes the single trimmed output to that file
  (only valid when exactly one input file is given).
- `-o <DIR>` (an existing directory, or any path ending with `/`) places
  each trimmed output as `<DIR>/<input-stem>_trimmed.<ext>`. The
  directory is created on demand if it doesn't exist. Required when
  multiple inputs are given (shell wildcards expand to many files).
- The command refuses to overwrite the input file, regardless of mode.

**Side effects:** courses no longer referenced anywhere (alternatives
dropped, with no other path leading to them) are pruned from the
`courses:` map. The verbose flag (`--verbose` at the global level)
prints the protected-subject set and the list of removed orphans.

**Caveats:**

- Comments in the source YAML are not preserved (serializer limitation).
- `type: select` requirements that use a `pattern:` or `groups:`
  (rather than an explicit `courses:` list) are left untouched, since
  v1 doesn't enumerate patterns.
- `type: one_of` concentrations are preserved as a whole; trim recurses
  into each concentration's nested requirements.

### Convert (`convert`)

Convert program file(s) into the unified degree JSON format.

```bash
nuanalytics degree convert path/to/program.json
```

The input may be raw ai-landscape program JSON (auto-detected and
converted), an existing unified JSON file, or YAML; the output is always
unified JSON with structured prerequisites. The converter maps ai-landscape
category lists and picklists to requirements, flips their AND-of-OR
prerequisites into the internal expression tree, and defaults missing course
credits to 3. Data-quality issues (such as assumed credits) are reported and
embedded as a `conversion_warnings` array in the output.

ai-landscape *cluster* pipeline files (`course_verifier` /
`course_scraper.<program>.results`) are expanded into **one unified file per
program**, using collision-safe `<school>__<program>.unified.json` names.

| Option | Description | Default |
|--------|-------------|---------|
| `-o, --out <PATH>` | Output file (single input) or directory (one `<stem>.unified.json` per input) | next to each input as `<stem>.unified.json` |
| `--pretty` | Pretty-print the JSON (default is compact, one line) | false |

```bash
# Convert a directory of ai-landscape programs into ./converted/
nuanalytics degree convert ai-landscape-tools/validation_jsons/*.json -o converted/

# Convert one program, pretty-printed
nuanalytics degree convert program.json --pretty -o program.unified.json
```

> This converter is transitional — once upstream emits unified JSON
> directly it is no longer needed.

### Schema (`schema`)

Print the JSON Schema for the unified degree format — useful for validating
unified JSON files in other tools or pipelines. This is the same
`degree.schema.json` the MCP server serves via `get_degree_json_schema`.

```bash
# Print to stdout
nuanalytics degree schema

# Write to a file
nuanalytics degree schema -o degree.schema.json
```

| Option | Description | Default |
|--------|-------------|---------|
| `-o, --out <PATH>` | Write the schema to this file instead of stdout | stdout |

## Analysis Output

### Console Output

```
Starting degree analysis...
Loading degree program from: samples/degrees/csu-cs-bscs-general.yaml
✓ Loaded degree: BS Bachelor of Science in Computer Science
  Courses: 145
  Requirements: 15

Plan Generation:
  Estimated total plans: 72576
  Variable requirements: 4
  ⚠ Will cap at 1000 plans (use --max-plans to adjust)

Processing plans...
✓ Processed 1000 plans

Selected Plans:
  Shortest: 8 terms
  Longest: 10 terms
  Calc-Ready: 6 terms
  Random Samples: 5

Plan Validation (Shortest Path):
  Courses: 38
  Credits: 120.0
  Placeholders: 12
  ⚠ Warnings: 8

✓ Plan is valid

✓ Analysis complete. Reports saved to: .debug/reports/
```

### Generated Files

The analysis generates several output files:

**Reports Directory** (default: `.debug/reports/`):
- `degree_name.html` - Interactive HTML report with box plots and statistics
- `index.csv` - Summary of all analyzed plans

**Metrics Directory** (default: `.debug/metrics/`):
- `plans/degree_name/shortest.csv` - Shortest path plan with metrics
- `plans/degree_name/longest.csv` - Longest path plan with metrics
- `plans/degree_name/calc_ready.csv` - Calc-ready plan (if applicable)
- `plans/degree_name/random_N.csv` - Random sample plans
- `summary.jsonl` - JSON Lines file with summary statistics

### HTML Report Contents

The HTML report includes:
- Degree overview (name, credits, course count)
- Plan statistics (shortest/longest/median term counts)
- Box plots for curriculum metrics:
  - Complexity scores
  - Blocking factors
  - Delay factors
  - Centrality measures
- Course-level breakdowns by category
- Validation warnings and suggestions

## Configuration

Analysis behavior can be configured via the config file or command-line options:

```bash
# View current settings
nuanalytics config get

# Set default max plans
nuanalytics config set degree_analysis.max_plans 5000

# Set default sampling strategy
nuanalytics config set degree_analysis.sampling_strategy stratified

# Set default sample count
nuanalytics config set degree_analysis.sample_plan_count 10

# Set output directories
nuanalytics config set reports_dir "./reports"
nuanalytics config set metrics_dir "./metrics"
```

**Configuration Options:**

| Key | Description | Default |
|-----|-------------|---------|
| `degree_analysis.max_plans` | Maximum plans to generate | 1000 |
| `degree_analysis.sample_plan_count` | Random plans to export | 5 |
| `degree_analysis.sampling_strategy` | Enumeration strategy | shuffled |
| `degree_analysis.ignore_duplicates` | Skip duplicate plan combinations | true |
| `reports_dir` | HTML reports output directory | .debug/reports |
| `metrics_dir` | CSV/metrics output directory | .debug/metrics |

## Input File Format

Degree programs are defined in YAML files with three main sections:

> **Unified JSON.** The same three-section model can also be expressed as
> *unified JSON* — the degree model serialized directly to JSON, with
> prerequisites written as a symmetric tagged structure
> (`{"and"|"or": [...]}`, a bare string being a single prerequisite) and an
> optional `tags` array on degrees, requirements, and courses. Every
> subcommand auto-detects YAML vs. JSON on load, and `degree schema` emits
> the JSON Schema for the format. The sections below describe the YAML form;
> the JSON form mirrors it field-for-field.

### Degree Metadata

```yaml
degree:
  name: "Bachelor of Science in Computer Science"
  abbreviation: "BS CS"
  institution: "University Name"
  department: "Khoury College of Computer Sciences"
  credits_required: 128
  catalog_year: "2024-2025"
```

### Course Definitions

```yaml
courses:
  CS101:
    name: "Introduction to Computer Science"
    credits: 4
    prerequisites_raw: ""

  CS220:
    name: "Data Structures"
    credits: 4
    prerequisites_raw: "CS101[C]"

  CS320:
    name: "Algorithms"
    credits: 4
    prerequisites_raw: "(CS220[C] & CS165[C]) & (MATH155[C] | MATH156[C])"
```

**Course Fields:**

- `name` - Full course name
- `credits` - Number of credit hours
- `prerequisites_raw` - Prerequisite expression (see syntax below)
- `corequisites_raw` (optional) - Corequisite expression
- `strict_corequisites_raw` (optional) - Must be taken in same term

### Prerequisite Expression Syntax

Prerequisites use a logical expression syntax:

- `&` - AND operator (all courses required)
- `|` - OR operator (choose one)
- `()` - Grouping for precedence
- `[C]` - Grade requirement suffix (e.g., "C" grade or better)

**Examples:**

```yaml
# Single prerequisite
prerequisites_raw: "CS101[C]"

# Multiple prerequisites (AND)
prerequisites_raw: "CS101[C] & MATH156[C]"

# Alternative prerequisites (OR)
prerequisites_raw: "MATH124[C] | MATH127[C]"

# Complex expression
prerequisites_raw: "(CS220[C] & CS165[C]) & (MATH155[C] | MATH156[C] | MATH160[C])"
```

The expression `(CS220 & CS165) & (MATH155 | MATH156 | MATH160)` means:
- BOTH CS220 AND CS165 are required
- AND one of MATH155, MATH156, or MATH160

### Requirements Section

Requirements define what courses students must complete. They support categories for proper gen-ed tracking.

```yaml
requirements:
  # Major core courses (enumerable in plan generation)
  core_cs:
    name: "Computer Science Core"
    type: all
    category: major
    courses:
      - CS101
      - CS220
      - CS320

  # Supporting courses (math, science requirements)
  math_foundation:
    name: "Mathematics Foundation"
    type: all
    category: supporting
    courses:
      - "{MATH156, MATH160}"  # Choose one calculus
      - MATH200

  # Gen-ed requirements (may be satisfied by major courses)
  gen_ed_quantitative:
    name: "Quantitative Reasoning (FQ)"
    type: select
    category: gen_ed
    from:
      courses: [MATH156, MATH160, CS101]
    count: 1

  # Electives with credit-based selection
  cs_electives:
    name: "CS Electives"
    type: select
    category: elective
    from:
      pattern: "CS4*"
    credits: 12
```

**Requirement Categories:**

| Category | Description | Gen-Ed Tracking |
|----------|-------------|-----------------|
| `major` | Core major courses | Courses may satisfy gen-ed |
| `supporting` | Math, science, supporting courses | Courses may satisfy gen-ed |
| `gen_ed` | General education requirements | Reduced by major/supporting |
| `elective` | Free or restricted electives | Added after gen-ed |

**Requirement Types:**

| Type | Description |
|------|-------------|
| `all` | All listed courses required |
| `select` | Choose courses by `count` or `credits` |
| `one_of` | Choose one option from `options` list |

**Course Syntax in Requirements:**

- `CS101` - Single course
- `[CS101, CS102, CS103]` - Bundle (all required together)
- `{CS101, CS102}` - Equivalents (choose one)
- `CS4*` - Pattern matching (all 400-level CS courses)

### Gen-Ed Attributes

Courses can have gen-ed attributes that satisfy university requirements:

```yaml
courses:
  CS1800:
    name: "Discrete Structures"
    credits: 4
    gen_ed_attributes: ["FQ", "ND"]  # Formal/Quant, Natural/Designed

  MATH241:
    name: "Calculus I"
    credits: 4
    gen_ed_attributes: ["FQ"]
```

When major courses have gen-ed attributes, they automatically satisfy corresponding gen-ed requirements, reducing duplicate course counting.

### Example Degree File

```yaml
degree:
  name: "Bachelor of Science in Computer Science"
  abbreviation: "BS CS"
  institution: "Northeastern University"
  credits_required: 128
  catalog_year: "2024-2025"

courses:
  CS101:
    name: "Fundamentals of Computer Science"
    credits: 4
    prerequisites_raw: ""

  MATH156:
    name: "Calculus for Scientists/Engineers I"
    credits: 4
    prerequisites_raw: ""

  CS220:
    name: "Discrete Structures"
    credits: 4
    prerequisites_raw: "CS101[C]"

  CS320:
    name: "Introduction to Algorithms"
    credits: 4
    prerequisites_raw: "CS220[C] & MATH156[C]"

requirements:
  core:
    name: "CS Core"
    required_courses:
      - CS101
      - CS220
      - CS320
    min_credits: 12
```

## Validation Output

Basic validation displays any errors or warnings:

```
=== Validation Report ===

✓ No circular dependencies detected
✓ All course references are valid
✓ All requirements reference existing courses

Validation: PASSED
```

If issues are found:

```
=== Validation Report ===

✗ Error: Circular dependency detected: CS101 → CS220 → CS101
✗ Warning: Course CS999 referenced in requirements does not exist
✗ Warning: Course CS320 has prerequisite CS999 which doesn't exist

Validation: FAILED (2 errors, 1 warning)
```

## Audit Output

The audit report provides detailed analysis:

```
=== Degree Audit Report ===

Degree: Bachelor of Science in Computer Science
Institution: Northeastern University
Catalog Year: 2024-2025
Total Courses: 45
Total Credits Required: 128

=== Validation Results ===
✓ No circular dependencies detected
✓ All course references valid

=== Upper-Level Courses Without Prerequisites ===

The following courses are 300-level or above but have no prerequisites:
  • CS350 - Introduction to Databases
  • CS425 - Software Engineering
  • MATH350 - Abstract Algebra

=== Courses with Deep Prerequisite Chains ===

Courses requiring multiple prerequisite chains (threshold: 3+):

  CS425 - Software Engineering (3 chains)
    Chain 1: CS220 → CS101
    Chain 2: CS320 → CS220 → CS101 & CS320 → MATH156
    Chain 3: CS165 → CS162 → CS150B

  CS460 - Database Systems (2 chains)
    Chain 1: CS320 → CS220 → CS101
    Chain 2: CS320 → MATH156

=== Summary ===
  Total courses analyzed: 45
  Courses in requirements: 38
  Upper-level without prereqs: 3
  Courses with complex chains: 2
```

### Audit Configuration

The audit behavior can be configured:

```bash
# Set the prerequisite chain threshold (default: 3)
nuanalytics config set prerequisite_chain_threshold 4

# View current threshold
nuanalytics config get prerequisite_chain_threshold
```

## Graph Output

The `--print-graph` option displays prerequisite relationships:

```
=== Course Prerequisite Graph ===

CS101 → (no prerequisites)

CS220 → CS101

CS320 → CS220 & (MATH155 | MATH156 | MATH160)

CS425 → CS320 & CS345 & STAT302A
  Expanded paths:
    - CS320 → CS220 → CS101
    - CS320 → (MATH155 | MATH156 | MATH160)
    - CS345 → CS220 → CS101
    - STAT302A → MATH156

MATH156 → MATH127 | (MATH124 & MATH126)
  Expanded paths:
    Option 1: MATH127
    Option 2: MATH124 & MATH126
```

## Command Examples

### Basic Validation

```bash
# Validate a degree file
nuanalytics degree validate degrees/cs_2024.yaml

# Validate with verbose output
nuanalytics --verbose degree validate degrees/cs_2024.yaml
```

### Comprehensive Analysis

```bash
# Full audit report
nuanalytics degree audit degrees/cs_2024.yaml

# Audit with custom threshold for prerequisite chains
nuanalytics config set prerequisite_chain_threshold 5
nuanalytics degree audit degrees/cs_2024.yaml
```

### Graph Visualization

```bash
# View prerequisite structure
nuanalytics degree print-graph degrees/cs_2024.yaml
```

### Batch Analysis

```bash
# Validate multiple degree files
nuanalytics degree validate degrees/*.yaml

# Audit all degree files in directory
for file in degrees/*.yaml; do
  echo "Auditing $file"
  nuanalytics degree audit "$file"
done
```

### With Logging

```bash
# Enable debug logging
nuanalytics --debug degree audit degrees/cs_2024.yaml

# Log to file
nuanalytics --log-file degree_audit.log degree audit degrees/cs_2024.yaml
```

## Workflow: Creating a New Degree Program

1. **Create YAML file** with degree metadata, courses, and requirements

2. **Validate structure**:
   ```bash
   nuanalytics degree validate my_degree.yaml
   ```

3. **Review validation results** and fix any errors

4. **Run comprehensive audit**:
   ```bash
   nuanalytics degree audit my_degree.yaml
   ```

5. **Analyze audit findings**:
   - Review upper-level courses without prerequisites (might need to add prereqs)
   - Check courses with deep prerequisite chains (might indicate curriculum bottlenecks)
   - Verify subject area filtering is capturing relevant courses

6. **Visualize structure**:
   ```bash
   nuanalytics degree print-graph my_degree.yaml
   ```

7. **Iterate**: Refine the degree structure based on findings and rerun audit

8. *(Optional)* **Produce a single-path view** for visualization or
   downstream tools that don't reason about alternatives:
   ```bash
   nuanalytics degree trim my_degree.yaml
   ```
   See the **Trim** section above for `--keep-all` / `--include` /
   directory-output semantics.

## Troubleshooting

### YAML Syntax Errors

```
Error: Failed to parse YAML: invalid type at line 45
```

**Solution**: Use a YAML validator to check syntax. Common issues:
- Incorrect indentation (use spaces, not tabs)
- Missing colons after keys
- Unquoted strings with special characters

### Invalid Course References

```
Error: Course CS999 referenced in CS425 prerequisites does not exist
```

**Solution**: Ensure all courses referenced in prerequisites are defined in the `courses` section.

### Circular Dependencies

```
Error: Circular dependency detected: CS101 → CS220 → CS330 → CS101
```

**Solution**: Review and break the circular prerequisite chain. A course cannot (directly or indirectly) require itself.

### Missing Prerequisites

```
Warning: CS425 (400-level) has no prerequisites
```

**Solution**: Consider if this upper-level course should have prerequisites. If intentional, this is just a warning.

## Configuration

Audit behavior is controlled by configuration settings:

```bash
# View current settings
nuanalytics config get

# Set prerequisite chain threshold (default: 3)
nuanalytics config set prerequisite_chain_threshold 4

# Reset to defaults
nuanalytics config reset
```

Available configuration options:
- `prerequisite_chain_threshold` - Minimum chain depth to report in audit (default: 3)

## Advanced Features

### Subject Area Filtering

The audit automatically filters courses to those relevant to the degree's primary subject area:
- Determined from courses listed in requirements
- Falls back to all courses if subject cannot be determined
- Ensures audit focuses on degree-relevant courses

### Prerequisite Chain Analysis

The audit computes prerequisite chains using:
- **Shortest path** when OR alternatives exist
- **Same-subject preference** when choosing between alternatives
- **Chain merging** to eliminate redundant prerequisites
- **Proper ordering** to show dependency direction

Example: For `CS320` with prerequisites `(CS220 & CS165) & (MATH155 | MATH156)`:
- Chooses shortest path among OR alternatives
- Prefers same subject code (CS over CIS if both options exist)
- Merges overlapping chains
- Orders from foundation courses to advanced courses

## See Also

- [Config Command](config.md) - Manage configuration settings
- [Planner Command](planner.md) - Analyze curriculum metrics
- [Development Guide](../Development.md) - Contributing to NuAnalytics
