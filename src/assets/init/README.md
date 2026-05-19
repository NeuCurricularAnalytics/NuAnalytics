# NuAnalytics Research Project

This directory was scaffolded by `nuanalytics init`. It is set up for a research
workflow that combines authoring/reviewing degree program YAMLs with running
curriculum-plan analyses, driven either from the CLI or from Claude via MCP.

## Layout

- `degrees/` — degree program YAML files (schema v5.2). Drop new programs here.
- `plans/` — curriculum-plan CSVs in CurricularAnalytics.org format.
- `metrics/` — generated CSV metrics (created on first run).
- `reports/` — generated HTML/Markdown/PDF reports (created on first run).
- `nuanalytics.toml` — local config; overrides the user/global config.
- `.claude/` — MCP wiring and SKILL.md skills for Claude Code.

## Common commands

```sh
# Validate or analyze a single degree YAML
nuanalytics degree degrees/my-program.yaml

# Batch-analyze every degree YAML in this project
nuanalytics degree degrees/*.yaml

# Plan analysis on a curriculum CSV (both metrics + HTML report)
nuanalytics planner plans/my-plan.csv

# Inspect or change the merged config
nuanalytics config
```

## Using Claude in this directory

Running `claude` from this directory picks up `.claude/settings.json` and the
skills under `.claude/skills/`. The NuAnalytics MCP server is wired in, so
Claude can call `validate_degree`, `audit_degree`, `analyze_degree`, and
`get_degree_schema` directly.

Three skills auto-trigger based on what you ask:

- **degree-author** — generate a new degree YAML from a catalog source.
- **degree-review** — validate and critique an existing degree YAML.
- **plan-analyze** — run analyses on a degree YAML or curriculum CSV.
