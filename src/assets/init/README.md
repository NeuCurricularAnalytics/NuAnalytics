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

Run `claude` from this directory. It reads `.mcp.json`, which registers the NuAnalytics
MCP server, and the skills under `.claude/skills/`. `.claude/settings.json` approves the
server, but Claude Code honours project settings only in a trusted folder. Accept the
trust prompt the first time you open the project.

The server's tools validate, audit, analyze and render degrees; read the stored programs
and their analysis; and query IPEDS completion data. The database tools need a backend
configured (`database.endpoint` and `database.anon_key`) and a signed-in session
(`nuanalytics db login`); the rest work without one.

Five skills load on their own when a request matches:

- **degree-author** — build a degree file from a catalog and get it to validate.
- **degree-review** — check, fix or update an existing degree file.
- **degree-analyze** — compute and explain a degree's metrics; render its report or graphs.
- **stored-programs** — find, export, analyze and compare the programs in the database.
- **curriculum-research** — answer questions from IPEDS completions and the stored programs.
