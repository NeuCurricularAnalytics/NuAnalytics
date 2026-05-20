---
name: plan-analyze
description: Run NuAnalytics analysis on a curriculum file. Use when the user asks for metrics, reports, audit, or analysis of a degree YAML or curriculum CSV plan in this project.
---

# Plan Analyze

Run NuAnalytics on a curriculum file and surface the results.

## Tool selection

- **Degree YAML** (`degrees/*.yaml`) → use the `analyze_degree` MCP tool. If the user wants only validation, use `validate_degree`; for structural issues only, `audit_degree`.
- **Curriculum CSV plan** (`plans/*.csv`) → run `nuanalytics planner <file>` from the shell. Use `--no-report` for CSV-only metrics or `--no-csv` for HTML/MD/PDF reports only.

## Workflow

1. Identify the file the user means. If ambiguous, list candidates from `degrees/` and `plans/`.
2. Run the right tool. Capture metrics output (path under `metrics/`) and reports (path under `reports/`).
3. Summarize: total credits, term load distribution, longest prereq chain, any audit warnings.
4. Offer a follow-up: comparison against another file in the project, or re-running with different `--term-credits`.

## Tips

- Batch mode: `nuanalytics degree analyze degrees/*.yaml` processes every YAML in one pass.
- For comparative work, keep two files side-by-side and diff their metrics CSVs.
