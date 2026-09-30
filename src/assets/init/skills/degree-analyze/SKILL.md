---
name: degree-analyze
description: Analyze a NuAnalytics degree, computing its plans, complexity and delay metrics, and explain the results, render its report or plan graphs, or compare degrees. Use when asked for a degree's metrics, how hard or long a program is, a report or graph, or how two degrees compare.
allowed-tools: mcp__nuanalytics__analyze_degree mcp__nuanalytics__get_course_detail mcp__nuanalytics__compare_degrees mcp__nuanalytics__list_sample_degrees mcp__nuanalytics__validate_degree
---

# Degree analyze

**Goal:** a degree's metrics, explained in terms of which courses drive them.

**Where files go:** reports go in `reports/`, graphs in `reports/graphs/`.

## Tools

- `analyze_degree(path=…)` enumerates the degree's plans now and returns JSON. It
  **writes nothing**.
- `get_course_detail(path=…, course_id=…)` shows one course's metrics and the term it
  lands in, in each selected plan. Use it to explain why a metric is high.
- `render_degree_report(path=…, output_dir="reports/")` writes the HTML report.
  `render_plan_graph(path=…, plan_category="shortest", output_path="reports/graphs/…")`
  writes one plan's graph. Both refuse to replace a file unless `overwrite=true`.
- `compare_degrees(sources=[{degree: …}, {path: …}])` compares degrees side by side.
- `list_sample_degrees` lists bundled degrees to try things on (`degree="sample:csu"`).

## What the numbers mean

- **Complexity** is structural complexity: each course's *delay* (the longest
  prerequisite chain through it) plus its *blocking* (how many courses depend on it),
  summed over the plan. Each figure is a five-number summary across the plans
  enumerated.
- **`is_full_population`** says whether every possible plan was seen. When it is false,
  the figures describe a sample. `was_truncated` and `time_limit_reached` say why.
  `seed_used` reproduces the sample; pass it back as `random_seed`.
- `selected_plans` holds the shortest and longest plans plus random samples, each with a
  term-by-term schedule.
- **Placeholders:** `FE01`, `ELEC001` and similar courses stand in for electives no
  specific course was chosen for. They are not real courses; say so when one shows up
  in a schedule.
- `tool_followups` suggests the next call, such as a steadier `max_plans` for a large
  degree.

## What you would otherwise get wrong

- **Fresh is not stored.** `analyze_degree` re-enumerates every time, so sampled
  figures move between runs. For a degree that is stored in the database, the
  reproducible figures are its stored run (the stored-programs skill).
- **A curriculum CSV** (`plans/*.csv`, the CurricularAnalytics.org format) is not a
  degree file. Run `nuanalytics planner plans/<file>.csv` in the shell. `--no-report`
  gives metrics only, and `--term-credits` changes the term load.
- **Validate before analyzing.** An invalid degree gives metrics that mean nothing.

## Done when

- The metrics are reported with whether they cover the whole population, and the seed
  when they don't.
- The courses that drive complexity or delay are named, with the reason.
- The paths of any files written are given.
