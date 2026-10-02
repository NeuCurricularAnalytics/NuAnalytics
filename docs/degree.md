# Degree Command

`nuanalytics degree` works with degree programs: the requirements, the courses and their
prerequisites, written in YAML or the unified JSON format. It validates and audits them,
analyzes every plan a student could take through them, and converts and trims them.

```bash
nuanalytics degree validate    degree.yaml     # is it well-formed?
nuanalytics degree audit       degree.yaml     # validation plus curriculum warnings
nuanalytics degree analyze     degree.yaml     # plans, metrics, report
nuanalytics degree print-graph degree.yaml     # the prerequisite graph as text
nuanalytics degree trim        degree.yaml     # one walkable path per course
nuanalytics degree convert     program.json    # scraped program JSON → unified JSON
nuanalytics degree schema -o degree.schema.json
```

Every subcommand reads YAML or unified JSON and detects which on load (content starting
with `{` or `[` is JSON). Raw ai-landscape program JSON is detected and converted too.

## Analyze (`analyze`)

```bash
nuanalytics degree analyze path/to/degree.yaml
```

`analyze` enumerates the degree's plans — each a set of courses that satisfies every
requirement — and for each plan:

1. adds the prerequisites the plan's courses need and do not already have (the shortest
   chain, preferring courses already in the plan);
2. schedules the plan into terms;
3. computes each course's delay, blocking, complexity, centrality and chain length.

It then reports statistics across the plans, picks out the shortest and longest plans, a
"calc-ready" plan where one applies, and random samples, and writes an HTML report and
metrics files.

**Which plans.** A degree with no more distinct plans than `--max-plans` is enumerated in
full. One with more is sampled: under the default `shuffled` strategy, a random sample of
`--max-plans` plans. The sample is seeded from the degree itself, so the same degree and
settings give the same plans and figures every run. A plan with the same set of courses
as one already analyzed is skipped unless `--full-run` is given.

**How prerequisites are read.** In `A & B | C`, `&` binds tighter, so it means
"A and B, or C". When an OR offers a group of courses — `(MATH124 & MATH126) | MATH127` —
a plan satisfies it with a whole branch: the plan's graph draws every course of the
branch the plan completes, and prerequisite expansion adds the branch needing the fewest
new courses, never one course of a group. An equivalence group in a requirement
(`{MATH156, MATH160}`) lets one member stand in for another as a prerequisite, but a
course never stands in for its own prerequisite: where a requirement offers
`{CS2800, CS4820}` and CS4820 requires CS2800, a plan taking CS4820 still gets CS2800.

| Option | Meaning | Default |
|---|---|---|
| `--max-plans <N>` | The most plans to analyze; above it, a seeded sample | 1000 (config) |
| `--sample-plans <N>` | Random plans to select and export in full | 5 (config) |
| `--calc-strategy <S>` | Summarize across plans by `median` or `mean` | `median` |
| `--sampling-strategy <S>` | `shuffled`, or `sequential` (favours the options listed first). `stratified` is accepted but behaves as `shuffled` | `shuffled` |
| `--full-run` | Analyze plans with identical course sets too | off |
| `--include <COURSES>` | Courses every plan must contain (comma-separated) | none |
| `--target-course <COURSE>` | Report where one course lands across the plans, as JSON on stdout | none |
| `--metrics-out <PATH>` | With `--target-course`, also write the report JSON here | none |
| `--no-report` | Skip the HTML report | off |
| `--no-csv` | Skip the metrics files: plan CSVs, summary, index row **and** the report JSON | off |
| `--report-dir <DIR>`, `--metrics-dir <DIR>` | Output directories | from config |
| `-j, --jobs <N>` | Degrees analyzed at once when several files are given | 8 |
| `--school <NAME>` | Also write a combined `<school>_school_report.json` across the files | none |
| `--from-db <NAME>` | Analyze a stored program instead of a file | none |

`FILES` may be omitted only with `--from-db`.

### Include courses

`--include` pins courses into every plan, including the shortest. It answers "what if a
student has already taken these", or narrows the plan space to one choice. A requirement
that an included course satisfies is locked to it:

```bash
nuanalytics -v degree analyze --include "CS370,STAT301" samples/degrees/csu-cs-bscs-general.yaml
# Plan Generation:
#   Included courses: CS370, STAT301
#   Estimated total plans: 1400            (28,154,110,320 without them)
#   Variable requirements: 5               (7 without them)
```

Prerequisite expansion also avoids alternatives to an included course, so the plan does
not take a second course for a slot the included one already fills. `degree trim` takes
`--include` too, with its own meaning — see [Trim](#trim-trim).

### Target course

`--target-course CSE475` reports which term the course lands in across the analyzed
plans, as JSON on stdout, instead of writing reports. It always runs in one process.

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

### Analyze a stored program (`--from-db`)

`--from-db <NAME>` analyzes a program already imported into the database
(via `db import` or the `import_degree` MCP tool — see
[Database Setup](database/setup.md)) instead of a local file. The stored
program's lossless `document` is parsed back into a degree and analyzed with
exactly the same options as the file-based path. This is single-program only —
no worker pool, so `-j/--jobs` doesn't apply. Asking `degree analyze` for a stored program
always enumerates it afresh; the MCP tools instead read the program's stored run unless
passed `fresh=true` (see [docs/mcp.md](mcp.md#stored-or-fresh)).

`<NAME>` is resolved through a ladder; the first tier that matches wins:

1. exact `program_key` (e.g. `prog:167358|11.0701|2025-2026|BS`)
2. exact `degree_id`
3. a `name` substring (case-insensitive `ILIKE`)

Exactly one match is analyzed; zero matches is an error; multiple matches list
the candidates as `program_key · name · institution · catalog_year` so you can
re-run with a more specific name or the exact `program_key`.

```bash
# Analyze a stored program by name
nuanalytics degree analyze --from-db "Computer Science (Boston)"

# Or pin the exact program by key, with the usual analyze options
nuanalytics degree analyze --from-db "prog:167358|11.0701|2025-2026|BS" --sample-plans 20
```

On a single match the loaded-program banner is printed before analysis:

```
✓ Loaded stored program: Bachelor of Science in Computer Science (Boston) (program_key prog:167358|11.0701|2025-2026|BS · unitid 167358)
```

`--from-db` requires a configured, logged-in database session
(`nuanalytics db login`). It is mutually exclusive with positional `FILES`.

### Output

The default console output is a short summary:

```
Degree Analysis Complete
========================
Degree: BS Bachelor of Science in Computer Science - Computer Science Concentration
Plans analyzed: 200

Degree Statistics (across all plans):
  Complexity: median 219.0, range 168.0-441.0
  Longest Delay: median 6.0, range 5.0-10.0
```

With `-v` it also shows loading, cycle breaking, plan generation, the selected plans and
a check of the shortest plan:

```
✓ Loaded degree: BS Bachelor of Science in Computer Science - Computer Science Concentration
  Courses: 145
  Requirements: 21
⚠ Detected 1 circular prerequisite(s), breaking cycles...
  Removed edge: CS163 → CS152

Plan Generation:
  Estimated total plans: 28154110320
  Variable requirements: 7
  ⚠ Will cap at 200 plans (use --max-plans to adjust)

Selected Plans:
  Shortest: 8 terms
  Longest: 10 terms
  Calc-Ready: N/A
  Random Samples: 5
```

Files written, for a degree whose id is `csu-cs-bscs-general`:

| Where | File |
|---|---|
| metrics directory | `csu-cs-bscs-general_report.json` — the full analysis: degree, statistics, selected plans |
| | `csu-cs-bscs-general_summary.jsonl` — one summary line |
| | `index.csv` — one row per degree, for a batch |
| | `plans/csu-cs-bscs-general/shortest.csv`, `longest.csv`, `random-sample-N.csv` |
| reports directory | `csu-cs-bscs-general-analysis.html` |

The HTML report has the degree's statistics with box plots for each metric, and each
selected plan's term-by-term schedule and curriculum graph.

## Validate (`validate`)

```bash
nuanalytics degree validate path/to/degree.yaml
```

Checks that the file parses, that every course a requirement or prerequisite names
exists, that requirements are well-formed for their type, that prerequisites form no
cycle, and that cross-listings are two-way. Errors make it fail; warnings — unreferenced
courses, courses required only through a prerequisite chain, electives whose
prerequisite is itself only optional, credit totals that cannot add up — do not:

```
✓ Degree program is valid

Warnings (246):

  Unreferenced Courses:
    - Course 'CS155' is defined but never referenced in requirements

  Hidden Requirements (Implicitly Required):
    - Course 'MATH125' is implicitly required: MATH160 -> MATH126 -> MATH125
```

A pattern that matches no listed course is an error. For a pool the degree does not
enumerate — a gen-ed `select` over `"HUM:100+"` or `"*:100+"` — pass
`--allow-unmatched-patterns` to report it as a warning instead.

[docs/mcp.md](mcp.md#validation-reference) lists every error and warning type.

## Audit (`audit`)

```bash
nuanalytics degree audit path/to/degree.yaml
```

Validation, plus two curriculum checks: upper-level courses that declare no
prerequisites, and major courses whose prerequisite chain is at least
`prerequisite_chain_threshold` long (default 4; set it with
`nuanalytics config set prerequisite_chain_threshold 5`). The report ends with a summary:

```
Audit Summary
-------------
  ⚠ Validation warnings: 246
  ⚠ Upper-level courses without prerequisites: 9
  ⚠ Courses with deep chains (≥4): 26
```

## Print the graph (`print-graph`)

```bash
nuanalytics degree print-graph path/to/degree.yaml
```

Prints any prerequisite cycles, the entry and terminal course counts, and each course's
prerequisites (`co:` marks corequisites):

```
Prerequisite Map (course → prerequisites):
------------------------------------------
  BZ120 → (none)
  BZ350 → BZ110 | BZ120 | LIFE102 + co: STAT301, STAT307, STAT315
```

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

Conversion also marks free-elective blocks that exist only to reach the degree total with
`fills_to_total: true`, and prints which ones — see
[Free Electives That Fill to the Total](#free-electives-that-fill-to-the-total-fills_to_total).
It never changes a value already set in the input.

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

Prints the JSON Schema of the unified degree format, for validating unified JSON in other
tools — the same schema the MCP server returns from
`get_reference(topic="degree-json-schema")`.

```bash
nuanalytics degree schema                       # to stdout
nuanalytics degree schema -o degree.schema.json
```

## The degree format

The full reference is `src/assets/Degree-schema.yaml`, which the MCP server serves as
`get_reference(topic="degree-yaml")`; the files in [samples/degrees/](../samples/degrees/)
are complete real examples. In outline:

```yaml
degree:
  id: example-bs-cs-2024
  institution: Example University
  program: B.S. Computer Science
  catalog_year: "2024-2025"
  total_credits: 120
  gpa_minimum: 2.0
  major_subjects: [CS, MATH]          # what `trim` protects, and what counts as major

requirements:
  intro:
    name: Introductory CS
    type: all                          # every listed course
    category: major
    courses: [CS1100, CS2100, "{MATH1341, MATH1241}"]
  cs_electives:
    name: Upper-division CS electives
    type: select                       # `count` courses, or `credits`, from a pool
    category: major
    count: 3
    from:
      pattern: "CS:3000+"
      exclude: [CS5000]

courses:
  CS2100:
    title: Data Structures
    prefix: CS
    number: "2100"
    credits: 4
    prerequisites_raw: "CS1100[C] & (MATH1341 | MATH1241)"
```

**Requirement types:** `all` (every course), `select` (`count` courses or `credits` from a
`from` pool of listed `courses`, a `pattern`, several `include` patterns, minus
`exclude`), and `one_of` (exactly one of several `options`, each with its own
requirements). **Categories:** `major`, `supporting`, `gen_ed`, `elective`.

**Inside a course list:**

| Written | Means |
|---|---|
| `CS101` | that course |
| `[CHEM111, CHEM112]` | a bundle — all of them, together |
| `{MATH156, MATH160}` | equivalents — any one |
| `{[CHEM111, CHEM112], CHEM107}` | a choice of groups — the bundle, or CHEM107. Supported in `all` lists, and expanded on load into the `one_of` it means, so the file re-serializes in that form |
| `CS:300+`, `MATH:300-499`, `*:100+` | a pattern, in a `select` pool |

**Prerequisite expressions** (`prerequisites_raw`) use `&` (and), `|` (or), parentheses,
and a `[GRADE]` suffix for a minimum grade (`ICS111[B]`). `&` binds tighter than `|`. In
unified JSON the same expression is a tree: `{"and": [...]}` / `{"or": [...]}`, a bare
string being one course.

**Course fields** beyond the four above include `corequisites`, `strict_corequisites`
(same term), `cross_listed_as`, `typically_offered`, `gen_ed_attributes` and
`grade_minimum`.

### Free Electives That Fill to the Total (`fills_to_total`)

Many degrees end with a block like "free electives to reach 120 credits". Written as a
fixed amount, it over-counts on any plan whose other choices run heavier:

```yaml
  free_electives:
    name: "Unrestricted Electives"
    type: select
    category: elective
    from:
      include: ["*:*"]
    credits: 20
    fills_to_total: true   # size per plan, up to 20, to reach total_credits
```

With `fills_to_total: true` the block is sized **per plan** as
`min(credits, max(0, total_credits − everything else))`. A plan whose other courses already
reach the total takes none of it; a light plan still takes all 20, and the generic `ELEC`
filler tops up any remaining gap as before. It never grows past `credits`, never takes a
plan below `total_credits`, and lands on the total exactly whenever the credits involved are
whole numbers — the block is rebuilt at its share rather than trimmed a placeholder at a
time (see [Placeholder courses](#placeholder-courses)).

Colorado State's CS concentration is the case that prompted it. Its longest path pulls in
extra prerequisite courses and used to land at 132 credits against 120; flagged, it lands on
120 exactly, with the block rebuilt as `FE01 FE02 FE03S`.

**You rarely set it by hand.** `degree convert` sets it on requirements it recognises and
prints what it flagged (`• fills_to_total set on: free_electives`). The rule is deliberately
conservative, because a wrong `true` silently under-counts a real requirement while a missed
one only leaves the old overshoot. A block qualifies only when it is:

- a credit-sized `select` from the fully unrestricted pool `*:*`
- not `gen_ed`, `major` or `supporting`
- not a combined bucket such as "Foreign Language and Free Electives"
- named for electives or credits, not a program ("Jewish Studies", "Second Discipline")
- described as reaching the **degree** total — graduation, the degree, or `total_credits`
  itself, not "to reach 21 credits" or "the 80-credit option total" — or named as free /
  unrestricted electives

**Correcting it.** The field has three states. Leave it unset and conversion decides. Set
`true` or `false` and conversion never overrides you, so a correction survives the next
`degree convert`. Use `false` for a block the rule gets wrong: Tulsa's `free_electives` is
really "Electives (14 hours; CS or CYB, advisor-approved)" and is marked `false` in the
corpus for that reason.

Setting the flag does **not** re-sample the plans. The plan-enumeration seed ignores it, so
flagging a degree changes only how that block is sized; every other metric is as before.

### Placeholder courses

Where a plan needs credits that no specific course supplies, it uses a **placeholder**: a
synthetic course with no catalog entry. A wildcard requirement such as `credits: 20` from
`*:*` becomes `FE01`, `FE02`, …; the generic filler that tops a plan up to `total_credits`
uses `ELEC001`, `ELEC002`, …. A placeholder's credits are carried in its name:

| name | credits |
|---|---|
| `FE03`, `ELEC001` | 3 — a full placeholder |
| `FE07S`, `ELEC002S` | 2 |
| `FE05S1`, `ELEC004S1` | 1 |

Amounts are written as full 3-credit placeholders plus one for the remainder, so every whole
amount is exact: 20 is `FE01`–`FE06` + `FE07S`, 10 is `FE01`–`FE03` + `FE04S1`. A fractional
shortfall — half-unit courses at course-unit schools — is rounded **up** to whole credits,
so a plan is never left below its total.

The credit marker is read *after* the number, never by searching for the last `S`: many
prefixes end in one (`NS` for natural sciences, `PSS`, `GES`), and `NS04` is a 3-credit
placeholder numbered 4, not "`S`, then 4 credits".

### Gen-ed attributes

A course's `gen_ed_attributes` (`["AUCC-1A", "GT-CO2"]`) name the general-education
requirements it satisfies, so a major course that also counts for gen-ed is not counted
twice.

## Configuration

`[degree_analysis]` and `[audit]` in the configuration set the defaults these commands
use; see [docs/config.md](config.md).

## Troubleshooting

- **A parse error.** The message names the line. In YAML, check indentation (spaces, not
  tabs) and quote strings that contain `:`, `{` or `[`.
- **A course does not exist.** Every course a requirement or prerequisite names must be
  under `courses:`. `validate` names each one.
- **A prerequisite cycle.** `validate` reports it as an error. `analyze` breaks it to
  proceed and says which edge it removed (`-v`).
- **Every plan is huge.** Narrow the plan space with `--include`, or raise `--max-plans`
  and accept a sample.

## See also

- [MCP server](mcp.md) — the same operations as tools for a model
- [Config command](config.md)
- [Planner command](planner.md) — analyzing a fixed curriculum CSV
