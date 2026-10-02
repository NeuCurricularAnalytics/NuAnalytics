# Assets

Files compiled into the binary with `include_str!`, so they ship with it and need no
install step. Renaming or moving one breaks the build.

| File | Used for |
|---|---|
| `DefaultCLIConfigRelease.toml`, `DefaultCLIConfigDebug.toml` | The built-in configuration defaults for release and debug builds. **Their `endpoint` and `anon_key` must stay blank** — this repository is public, and a non-blank default would silently re-point self-hosted users (see the SECURITY comment in each file). |
| `Degree-schema.yaml` | The degree format reference, served by the MCP server as `get_reference(topic="degree-yaml")`. |
| `degree.schema.json` | The unified degree JSON Schema, printed by `degree schema` and served as `get_reference(topic="degree-json-schema")`. |
| `graph_vanilla.js` | The curriculum-graph renderer embedded in HTML reports. |
| `init/` | What `nuanalytics init` writes into a new project: its README, `nuanalytics.toml`, the `.mcp.json` template, `.claude/settings.json`, and the Claude Code skills. Tests check the skills against the MCP server. |
