//! `nuanalytics init <DIR>` — scaffold a research project directory.
//!
//! Creates a directory pre-wired for a `NuAnalytics` research workflow:
//! a `.claude/` folder with the `NuAnalytics` MCP server registered and a
//! set of `SKILL.md` skills, plus a working layout for `degrees/` and
//! `plans/` and a local `nuanalytics.toml`.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// Standard PATH directories where a bare `nuanalytics` lookup will succeed
/// across machines. Used by [`detect_mcp_command`] to decide whether the
/// generated MCP config should reference the binary by name or by absolute
/// path.
const STD_PATH_DIRS: &[&str] = &["/usr/bin", "/usr/local/bin", "/opt/homebrew/bin"];

/// Binary name used in the generated MCP config when on a standard PATH dir.
const BIN_NAME: &str = "nuanalytics";

/// Subcommand argument passed to the binary in the generated MCP config.
const MCP_SUBCOMMAND: &str = "mcp";

const SETTINGS_TEMPLATE: &str = include_str!("../../assets/init/settings.json.tmpl");
const MCP_JSON_TEMPLATE: &str = include_str!("../../assets/init/mcp.json.tmpl");
const NUANALYTICS_TOML: &str = include_str!("../../assets/init/nuanalytics.toml");
const PROJECT_README: &str = include_str!("../../assets/init/README.md");

const SKILL_DEGREE_AUTHOR: &str = include_str!("../../assets/init/skills/degree-author/SKILL.md");
const SKILL_DEGREE_REVIEW: &str = include_str!("../../assets/init/skills/degree-review/SKILL.md");
const SKILL_DEGREE_UPDATE: &str = include_str!("../../assets/init/skills/degree-update/SKILL.md");
const SKILL_DEGREE_FETCH: &str = include_str!("../../assets/init/skills/degree-fetch/SKILL.md");
const SKILL_PLAN_ANALYZE: &str = include_str!("../../assets/init/skills/plan-analyze/SKILL.md");

// Schema reference is shared with the MCP server's `get_degree_schema` tool —
// embed the canonical file rather than maintain a second copy.
const REF_SCHEMA: &str = include_str!("../../assets/Degree-schema.yaml");
const REF_GUIDE: &str = include_str!("../../assets/init/skills/degree-author/generation-guide.md");
const REF_QUICK: &str = include_str!("../../assets/init/skills/degree-author/quick-reference.md");
const REF_EXAMPLE: &str =
    include_str!("../../assets/init/skills/degree-author/example-bscs-general.yaml");

/// Files written verbatim from embedded assets, keyed by their path relative
/// to the target directory.
const STATIC_FILES: &[(&str, &str)] = &[
    ("nuanalytics.toml", NUANALYTICS_TOML),
    ("README.md", PROJECT_README),
    (".claude/skills/degree-author/SKILL.md", SKILL_DEGREE_AUTHOR),
    (".claude/skills/degree-author/schema-v5.2.yaml", REF_SCHEMA),
    (
        ".claude/skills/degree-author/generation-guide.md",
        REF_GUIDE,
    ),
    (".claude/skills/degree-author/quick-reference.md", REF_QUICK),
    (
        ".claude/skills/degree-author/example-bscs-general.yaml",
        REF_EXAMPLE,
    ),
    (".claude/skills/degree-review/SKILL.md", SKILL_DEGREE_REVIEW),
    (".claude/skills/degree-update/SKILL.md", SKILL_DEGREE_UPDATE),
    (".claude/skills/degree-fetch/SKILL.md", SKILL_DEGREE_FETCH),
    (".claude/skills/plan-analyze/SKILL.md", SKILL_PLAN_ANALYZE),
    ("degrees/.gitkeep", ""),
    ("plans/.gitkeep", ""),
];

/// Path (relative to the target directory) of the templated Claude Code MCP config.
const SETTINGS_REL: &str = ".claude/settings.json";

/// Path (relative to the target directory) of the project-root MCP config.
const MCP_JSON_REL: &str = ".mcp.json";

/// Scaffold a `NuAnalytics` research project at `dir`.
///
/// # Errors
///
/// Returns an error if:
/// - The target directory cannot be created.
/// - Any target file already exists and `force` is false.
/// - Writing any scaffold file or rendering the MCP config fails.
pub fn run(dir: &Path, force: bool) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(dir)?;

    let (mcp_command, mcp_args) = detect_mcp_command();
    let settings_json = render_mcp_config(SETTINGS_TEMPLATE, &mcp_command, &mcp_args)?;
    let mcp_json = render_mcp_config(MCP_JSON_TEMPLATE, &mcp_command, &mcp_args)?;
    let settings_path = dir.join(SETTINGS_REL);
    let mcp_json_path = dir.join(MCP_JSON_REL);

    if !force {
        let mut conflicts: Vec<PathBuf> = STATIC_FILES
            .iter()
            .map(|(rel, _)| dir.join(rel))
            .filter(|p| p.exists())
            .collect();
        if settings_path.exists() {
            conflicts.push(settings_path.clone());
        }
        if mcp_json_path.exists() {
            conflicts.push(mcp_json_path.clone());
        }
        if !conflicts.is_empty() {
            let mut msg =
                String::from("the following files already exist (use --force to overwrite):\n");
            for c in conflicts {
                writeln!(msg, "  {}", c.display())?;
            }
            return Err(msg.into());
        }
    }

    for (rel, content) in STATIC_FILES {
        write_file(&dir.join(rel), content.as_bytes())?;
    }
    write_file(&settings_path, settings_json.as_bytes())?;
    write_file(&mcp_json_path, mcp_json.as_bytes())?;

    println!(
        "\n✓ scaffolded NuAnalytics research project at {}",
        dir.display()
    );
    println!("  next:");
    println!("    cd {} && claude", dir.display());
    Ok(())
}

/// Write `content` to `path`, creating parent directories as needed, and
/// print a `created <path>` line so the caller can see what happened.
fn write_file(path: &Path, content: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    println!("  created {}", path.display());
    Ok(())
}

/// Render an MCP config template by substituting the command and args.
/// Values are JSON-encoded so quoting and escaping are handled correctly.
fn render_mcp_config(template: &str, command: &str, args: &[String]) -> Result<String, serde_json::Error> {
    let command_json = serde_json::to_string(command)?;
    let args_json = serde_json::to_string(args)?;
    Ok(template
        .replace("{{MCP_COMMAND}}", &command_json)
        .replace("{{MCP_ARGS}}", &args_json))
}

/// Decide what to put in `.claude/settings.json` for `command` / `args`.
///
/// Prefer the bare binary name when the running executable lives in a
/// standard PATH directory (so the generated config is portable across
/// machines); otherwise embed the absolute path so the project works
/// without any further setup.
fn detect_mcp_command() -> (String, Vec<String>) {
    let args = vec![MCP_SUBCOMMAND.to_string()];

    let Ok(exe) = std::env::current_exe().and_then(|p| p.canonicalize()) else {
        return (BIN_NAME.to_string(), args);
    };

    let cargo_bin = std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo/bin"));

    let in_std_dir = exe.parent().is_some_and(|p| {
        STD_PATH_DIRS.iter().any(|d| p == Path::new(d))
            || cargo_bin.as_deref().is_some_and(|c| p == c)
    });

    if in_std_dir {
        (BIN_NAME.to_string(), args)
    } else {
        (exe.to_string_lossy().into_owned(), args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::fs;
    use tempfile::TempDir;

    /// Files that `run` must produce inside the target directory.
    const EXPECTED_FILES: &[&str] = &[
        "nuanalytics.toml",
        "README.md",
        ".mcp.json",
        ".claude/settings.json",
        ".claude/skills/degree-author/SKILL.md",
        ".claude/skills/degree-author/schema-v5.2.yaml",
        ".claude/skills/degree-author/generation-guide.md",
        ".claude/skills/degree-author/quick-reference.md",
        ".claude/skills/degree-author/example-bscs-general.yaml",
        ".claude/skills/degree-review/SKILL.md",
        ".claude/skills/degree-update/SKILL.md",
        ".claude/skills/degree-fetch/SKILL.md",
        ".claude/skills/plan-analyze/SKILL.md",
        "degrees/.gitkeep",
        "plans/.gitkeep",
    ];

    #[test]
    fn scaffolds_full_layout() {
        let tmp = TempDir::new().expect("tempdir");
        let target = tmp.path().join("proj");

        run(&target, false).expect("init succeeds on a fresh directory");

        for rel in EXPECTED_FILES {
            let p = target.join(rel);
            assert!(p.exists(), "missing scaffolded file: {}", p.display());
        }
    }

    #[test]
    fn settings_json_is_valid_with_nuanalytics_server() {
        let tmp = TempDir::new().expect("tempdir");
        let target = tmp.path().join("proj");

        run(&target, false).expect("init succeeds");

        let body = fs::read_to_string(target.join(".claude/settings.json")).expect("read settings");
        let v: Value = serde_json::from_str(&body).expect("settings.json parses as JSON");

        let server = &v["mcpServers"]["nuanalytics"];
        assert!(
            server.get("command").and_then(Value::as_str).is_some(),
            "mcpServers.nuanalytics.command must be a string; got {server}"
        );
        let args = server
            .get("args")
            .and_then(Value::as_array)
            .expect("args array");
        assert_eq!(args.len(), 1);
        assert_eq!(args[0].as_str(), Some("mcp"));
    }

    #[test]
    fn aborts_on_existing_file_without_force() {
        let tmp = TempDir::new().expect("tempdir");
        let target = tmp.path().join("proj");
        fs::create_dir_all(target.join(".claude")).expect("mkdir");
        fs::write(target.join(".mcp.json"), "pre-existing\n").expect("seed");

        let err = run(&target, false).expect_err("must refuse to overwrite without --force");
        let msg = err.to_string();
        assert!(msg.contains("already exist"), "unexpected error: {msg}");
        assert!(
            msg.contains(".mcp.json"),
            "should name the conflict: {msg}"
        );

        let body = fs::read_to_string(target.join(".mcp.json")).expect("read");
        assert_eq!(body, "pre-existing\n");
    }

    #[test]
    fn force_overwrites_existing_files() {
        let tmp = TempDir::new().expect("tempdir");
        let target = tmp.path().join("proj");
        fs::create_dir_all(target.join(".claude")).expect("mkdir");
        fs::write(target.join(".claude/settings.json"), "pre-existing\n").expect("seed");

        run(&target, true).expect("init --force succeeds despite existing file");

        let body = fs::read_to_string(target.join(".claude/settings.json")).expect("read");
        let v: Value = serde_json::from_str(&body).expect("settings.json parses");
        assert!(v["mcpServers"]["nuanalytics"]["command"].is_string());
    }

    #[test]
    fn force_succeeds_with_no_existing_files() {
        let tmp = TempDir::new().expect("tempdir");
        let target = tmp.path().join("proj");

        run(&target, true).expect("init --force on a clean dir should still succeed");

        for rel in EXPECTED_FILES {
            let p = target.join(rel);
            assert!(p.exists(), "missing scaffolded file: {}", p.display());
        }
    }

    #[test]
    fn render_mcp_config_escapes_quotes_and_backslashes_in_command() {
        // A Windows-style path with backslashes plus a literal double quote
        // would corrupt the JSON if `render_mcp_config` used naive interpolation
        // instead of `serde_json::to_string`.
        let weird = r#"C:\Program Files\Nu"Analytics\nuanalytics.exe"#;
        let rendered =
            render_mcp_config(SETTINGS_TEMPLATE, weird, &[MCP_SUBCOMMAND.to_string()])
                .expect("render_mcp_config");

        let v: Value = serde_json::from_str(&rendered)
            .expect("rendered settings must be valid JSON even for odd command paths");
        assert_eq!(
            v["mcpServers"]["nuanalytics"]["command"].as_str(),
            Some(weird),
            "command round-trips through JSON unchanged"
        );
    }

    #[test]
    fn mcp_json_is_valid_with_stdio_type() {
        let tmp = TempDir::new().expect("tempdir");
        let target = tmp.path().join("proj");

        run(&target, false).expect("init succeeds");

        let body = fs::read_to_string(target.join(".mcp.json")).expect("read .mcp.json");
        let v: Value = serde_json::from_str(&body).expect(".mcp.json parses as JSON");

        let server = &v["mcpServers"]["nuanalytics"];
        assert_eq!(
            server.get("type").and_then(Value::as_str),
            Some("stdio"),
            "mcpServers.nuanalytics.type must be \"stdio\"; got {server}"
        );
        assert!(
            server.get("command").and_then(Value::as_str).is_some(),
            "mcpServers.nuanalytics.command must be a string; got {server}"
        );
        let args = server
            .get("args")
            .and_then(Value::as_array)
            .expect("args array");
        assert_eq!(args.len(), 1);
        assert_eq!(args[0].as_str(), Some("mcp"));
    }

    #[test]
    fn generated_nuanalytics_toml_parses_as_config() {
        use nu_analytics::config::Config;

        let tmp = TempDir::new().expect("tempdir");
        let target = tmp.path().join("proj");
        run(&target, false).expect("init succeeds");

        let body =
            fs::read_to_string(target.join("nuanalytics.toml")).expect("read nuanalytics.toml");
        let cfg: Config =
            toml::from_str(&body).expect("embedded nuanalytics.toml must deserialize into Config");

        assert_eq!(cfg.paths.metrics_dir, "./metrics");
        assert_eq!(cfg.paths.reports_dir, "./reports");
    }
}
