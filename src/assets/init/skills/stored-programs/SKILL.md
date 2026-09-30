---
name: stored-programs
description: Find, read, export, analyze and compare the degree programs stored in the NuAnalytics database, and the analysis stored with them. Use when asked which programs a school has, to pull a stored degree into a file, for a stored program's metrics or report, or to compare programs across schools.
allowed-tools: mcp__nuanalytics__search_degrees mcp__nuanalytics__get_degree mcp__nuanalytics__get_stored_analysis mcp__nuanalytics__compare_degrees mcp__nuanalytics__search_institutions mcp__nuanalytics__validate_degree
---

# Stored programs

**Goal:** answer from what is stored, and say which run and variant the answer came from.

**Where files go:** exported degrees in `degrees/`, reports in `reports/`.

## Tools

- `search_institutions(name=…)` gives the `unitid`. `search_degrees(unitid=…, name=…)`
  lists that school's stored programs, each with its `program_key`.
- `get_degree(program_key=…)` returns one program and the analysis runs stored for it.
  `include_document=true` adds the whole degree.
- `convert_degree(degree=<program_key>, format="yaml", output_path="degrees/…")`
  exports the degree to a file. Run `validate_degree` on the file afterwards.
- `get_stored_analysis(degree=<program_key>)` returns the newest run per variant.
  `include_plans` and `include_course_metrics` add detail, and `latest=false` gives
  the history.
- `render_stored_report(degree=…, variant="full", output_path="reports/…")` writes the
  report of that stored run.
- `compare_degrees(sources=[{degree: …}, …], metrics="stored")` compares stored runs
  side by side.
- Any degree tool takes a `program_key` as `degree`. There is no need to export first.

## What you would otherwise get wrong

- **Runs append; they are never replaced.** "The metrics" means the newest run of a
  variant, which is what `get_stored_analysis` returns by default. Say which run
  (`created_at`) you used.
- **Variants:** `full` is the degree as written. `trimmed` collapses its alternatives
  to one path. Never compare a full run with a trimmed one.
- **Across `analyzer_version`s, a difference can come from the analyzer**, not the
  degree. Compare runs of the same version, or say that the versions differ.
- **Stored figures are reproducible; fresh ones are not.** `analyze_degree` and
  `compare_degrees(metrics="fresh")` enumerate again. Use them only when there is no
  stored run, or when the question is about the degree as it stands now.
- **A `degree_id` can span catalog years.** When it matches several programs, you get
  their `program_key`s. Pick one, and never average across the matches.
- This skill only reads. Adding a program is `import_degree`, which the server offers
  only when started with `--allow-writes`.

## Done when

- Each figure names its program, variant and run date.
- Exported files validate.
