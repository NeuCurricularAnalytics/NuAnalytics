//! The files `nuanalytics init` writes into a new project, compiled in.
//!
//! In the library rather than the CLI command so the MCP server's tests can read them: the
//! skills name MCP tools, and a guard test checks every name against the server's actual
//! tool list, so a renamed or removed tool cannot leave a skill pointing at nothing.

/// Name the project's `.mcp.json` registers the server under. Claude Code prefixes tool
/// names with it (`mcp__nuanalytics__validate_degree`), and `settings.json` approves it.
pub const MCP_SERVER_NAME: &str = "nuanalytics";

/// `.mcp.json`, with `{{MCP_COMMAND}}` and `{{MCP_ARGS}}` still to fill in.
///
/// Project-scoped MCP servers live in `.mcp.json`; Claude Code does not read an
/// `mcpServers` block from `.claude/settings.json`.
pub const MCP_JSON_TEMPLATE: &str = include_str!("../assets/init/mcp.json.tmpl");

/// `.mcp.json` for a server started as `command args…`.
///
/// Both are JSON-encoded, so a path with quotes or backslashes cannot corrupt the file.
///
/// # Errors
/// When a value cannot be encoded as JSON.
pub fn render_mcp_json(command: &str, args: &[String]) -> Result<String, serde_json::Error> {
    let command_json = serde_json::to_string(command)?;
    let args_json = serde_json::to_string(args)?;
    Ok(MCP_JSON_TEMPLATE
        .replace("{{MCP_COMMAND}}", &command_json)
        .replace("{{MCP_ARGS}}", &args_json))
}

/// Path of the MCP config, relative to the project directory.
pub const MCP_JSON_PATH: &str = ".mcp.json";

/// `.claude/settings.json`: approves the project's own server, so opening the project
/// does not stop at a prompt asking whether to trust it.
pub const SETTINGS_JSON: &str = include_str!("../assets/init/settings.json");

/// Directory the skills are written to, relative to the project directory.
pub const SKILLS_DIR: &str = ".claude/skills/";

/// A skill file, at `.claude/skills/<rel>`, compiled in from `assets/init/skills/<rel>`.
macro_rules! skill {
    ($rel:literal) => {
        (
            concat!(".claude/skills/", $rel),
            include_str!(concat!("../assets/init/skills/", $rel)),
        )
    };
}

/// Files written verbatim, keyed by their path relative to the project directory.
pub const STATIC_FILES: &[(&str, &str)] = &[
    (
        "nuanalytics.toml",
        include_str!("../assets/init/nuanalytics.toml"),
    ),
    ("README.md", include_str!("../assets/init/README.md")),
    (".claude/settings.json", SETTINGS_JSON),
    skill!("degree-author/SKILL.md"),
    skill!("degree-author/catalog-patterns.md"),
    skill!("degree-author/example.yaml"),
    skill!("degree-review/SKILL.md"),
    skill!("degree-analyze/SKILL.md"),
    skill!("stored-programs/SKILL.md"),
    skill!("curriculum-research/SKILL.md"),
    skill!("curriculum-research/ipeds.md"),
    skill!("curriculum-research/sql.md"),
    // The server's own queries and the project's examples, from their one copy each: the
    // demographics tools run exactly the catalog text shipped here.
    (
        ".claude/skills/curriculum-research/queries/completions_total.sql",
        include_str!("query/catalog/completions_total.sql"),
    ),
    (
        ".claude/skills/curriculum-research/queries/completions_by_school.sql",
        include_str!("query/catalog/completions_by_school.sql"),
    ),
    (
        ".claude/skills/curriculum-research/queries/completions_by_cip.sql",
        include_str!("query/catalog/completions_by_cip.sql"),
    ),
    (
        ".claude/skills/curriculum-research/queries/institutions_with_programs.sql",
        include_str!("query/catalog/institutions_with_programs.sql"),
    ),
    (
        ".claude/skills/curriculum-research/queries/hardest-degrees.sql",
        include_str!("../../docs/database/examples/hardest-degrees.sql"),
    ),
    (
        ".claude/skills/curriculum-research/queries/shortest-path-by-school.sql",
        include_str!("../../docs/database/examples/shortest-path-by-school.sql"),
    ),
    (
        ".claude/skills/curriculum-research/queries/shortest-path-schedule.sql",
        include_str!("../../docs/database/examples/shortest-path-schedule.sql"),
    ),
    ("degrees/.gitkeep", ""),
    ("plans/.gitkeep", ""),
];

/// The shipped markdown — every skill and the project README — as `(path, text)`.
pub fn markdown_files() -> impl Iterator<Item = (&'static str, &'static str)> {
    STATIC_FILES.iter().copied().filter(|(path, _)| {
        std::path::Path::new(path)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_approve_the_server_the_mcp_json_registers() {
        let settings: serde_json::Value =
            serde_json::from_str(SETTINGS_JSON).expect("settings.json parses");
        assert_eq!(
            settings["enabledMcpjsonServers"],
            serde_json::json!([MCP_SERVER_NAME])
        );
        assert!(
            settings.get("mcpServers").is_none(),
            "an mcpServers block in settings.json is not read; servers belong in .mcp.json"
        );
        let rendered = render_mcp_json("nuanalytics", &["mcp".to_string()]).expect("renders");
        let mcp: serde_json::Value = serde_json::from_str(&rendered).expect(".mcp.json parses");
        let servers = mcp["mcpServers"].as_object().expect("mcpServers object");
        assert_eq!(
            servers.keys().collect::<Vec<_>>(),
            [MCP_SERVER_NAME],
            ".mcp.json must register exactly the server settings.json approves"
        );
    }

    #[test]
    fn every_skill_has_frontmatter_naming_its_own_directory() {
        for (path, text) in markdown_files().filter(|(p, _)| p.ends_with("/SKILL.md")) {
            let dir = path
                .trim_end_matches("/SKILL.md")
                .rsplit('/')
                .next()
                .expect("skill directory");
            let body = text
                .strip_prefix("---\n")
                .unwrap_or_else(|| panic!("{path}: must open with --- frontmatter"));
            let (front, _) = body
                .split_once("\n---")
                .unwrap_or_else(|| panic!("{path}: frontmatter is not closed"));
            let front: serde_yaml::Value =
                serde_yaml::from_str(front).unwrap_or_else(|e| panic!("{path}: {e}"));
            assert_eq!(
                front["name"].as_str(),
                Some(dir),
                "{path}: name must match its directory"
            );
            let description = front["description"]
                .as_str()
                .unwrap_or_else(|| panic!("{path}: no description"));
            assert!(
                !description.is_empty() && description.len() <= 1024,
                "{path}: description must be 1-1024 characters"
            );
            // The Agent Skills form, a space-separated string, so the skill works outside
            // Claude Code too; and only this project's server's tools.
            let allowed = front["allowed-tools"]
                .as_str()
                .unwrap_or_else(|| panic!("{path}: allowed-tools must be a string"));
            let prefix = format!("mcp__{MCP_SERVER_NAME}__");
            for tool in allowed.split_whitespace() {
                assert!(
                    tool.starts_with(&prefix),
                    "{path}: allowed-tools may name only {prefix}* tools, not {tool}"
                );
            }
        }
    }

    /// The skill directory a shipped path is in, e.g. `degree-author`.
    fn skill_dir(path: &str) -> Option<&str> {
        path.strip_prefix(SKILLS_DIR)?.split('/').next()
    }

    #[test]
    fn every_skill_stays_short_enough_to_load_whole() {
        for (path, text) in markdown_files().filter(|(p, _)| p.ends_with("/SKILL.md")) {
            let lines = text.lines().count();
            assert!(
                lines <= 150,
                "{path}: {lines} lines; move detail to a reference file"
            );
        }
    }

    #[test]
    fn every_file_a_skill_names_is_shipped_beside_it() {
        let shipped: Vec<&str> = STATIC_FILES.iter().map(|(p, _)| *p).collect();
        for (path, text) in markdown_files() {
            let Some(dir) = skill_dir(path) else {
                continue;
            };
            let named = text.split('`').skip(1).step_by(2).filter(|t| {
                !t.contains('/')
                    && !t.contains(' ')
                    && std::path::Path::new(t).extension().is_some_and(|e| {
                        ["md", "yaml", "sql"]
                            .iter()
                            .any(|x| e.eq_ignore_ascii_case(x))
                    })
            });
            for name in named {
                let here = format!("{SKILLS_DIR}{dir}/{name}");
                let in_queries = format!("{SKILLS_DIR}{dir}/queries/{name}");
                assert!(
                    shipped.contains(&here.as_str()) || shipped.contains(&in_queries.as_str()),
                    "{path} names `{name}`, which is not shipped in {dir}/"
                );
            }
        }
    }

    #[test]
    fn the_example_degree_validates() {
        let (_, example) = STATIC_FILES
            .iter()
            .find(|(p, _)| p.ends_with("degree-author/example.yaml"))
            .expect("example shipped");
        let (program, _) = crate::core::degree::parse_degree_auto(example).expect("parses");
        let result = crate::core::validate_degree_program_with_options(
            &program,
            crate::core::degree::ValidationOptions {
                allow_unmatched_patterns: true,
            },
        );
        assert!(result.errors.is_empty(), "{:?}", result.errors);
    }

    #[cfg(feature = "database")]
    #[test]
    fn every_shipped_query_is_read_only() {
        let is_sql = |p: &str| {
            std::path::Path::new(p)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("sql"))
        };
        for (path, sql) in STATIC_FILES.iter().filter(|(p, _)| is_sql(p)) {
            assert_eq!(
                crate::core::query::sql::reject_if_not_read_only(sql),
                None,
                "{path}"
            );
        }
    }
}
