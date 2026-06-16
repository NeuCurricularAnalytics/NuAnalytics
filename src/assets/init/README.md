# NuAnalytics Research Project

This directory was scaffolded by `nuanalytics init`. It is set up for a research
workflow that combines authoring/reviewing degree program YAMLs with running
curriculum-plan analyses, driven either from the CLI or from Claude via MCP.

## Layout

- `degrees/` — degree program files (`*.yaml` or `*.unified.json`). Drop new programs here.
- `plans/` — curriculum-plan CSVs in CurricularAnalytics.org format.
- `metrics/` — generated CSV metrics (created on first run).
- `reports/` — generated HTML/Markdown/PDF reports (created on first run).
- `nuanalytics.toml` — local config; overrides the user/global config.
- `.mcp.json` — project-root MCP server registration (read by Claude Code and compatible clients).
- `.claude/` — Claude Code MCP wiring and SKILL.md skills.

## Common commands

```sh
# Validate a single degree file (YAML or unified JSON)
nuanalytics degree validate degrees/my-program.yaml
nuanalytics degree validate degrees/my-program.unified.json

# Full plan-enumeration analysis (writes CSV + HTML)
nuanalytics degree analyze degrees/my-program.yaml

# Batch-analyze every degree file in this project
nuanalytics degree analyze degrees/*.yaml degrees/*.unified.json

# Trim alternatives down to a single shared shortest path
nuanalytics degree trim degrees/my-program.yaml -o trimmed/

# Plan analysis on a curriculum CSV (both metrics + HTML report)
nuanalytics planner plans/my-plan.csv

# Inspect or change the merged config
nuanalytics config
```

## Using Claude in this directory

Running `claude` from this directory picks up `.mcp.json` (or `.claude/settings.json`)
and the skills under `.claude/skills/`. The NuAnalytics MCP server is wired in, so
Claude can call the degree tools — `validate_degree`, `audit_degree`,
`analyze_degree`, `trim_degree`, `get_degree_schema`, and friends —
directly.

Five skills auto-trigger based on what you ask:

- **degree-author** — generate a new degree YAML or unified JSON from a catalog source.
- **degree-review** — validate and critique an existing degree YAML or unified JSON.
- **degree-update** — revise an existing degree file (`*.yaml` or `*.unified.json`).
- **degree-fetch** — pull a degree from the database with dual-build verification.
- **plan-analyze** — run analyses on a degree file or curriculum CSV.
