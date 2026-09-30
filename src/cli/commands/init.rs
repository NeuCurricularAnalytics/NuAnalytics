//! `nuanalytics init <DIR>` — scaffold a research project directory.
//!
//! Creates a directory pre-wired for a `NuAnalytics` research workflow: the MCP server
//! registered in `.mcp.json` and approved in `.claude/settings.json`, a set of `SKILL.md`
//! skills, a working layout for `degrees/` and `plans/`, and a local `nuanalytics.toml`.
//! The files themselves are [`nu_analytics::core::init_assets`].

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

use nu_analytics::core::init_assets::{self, MCP_JSON_PATH, STATIC_FILES};

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
    let mcp_json = init_assets::render_mcp_json(&mcp_command, &mcp_args)?;
    let mcp_json_path = dir.join(MCP_JSON_PATH);

    if !force {
        let mut conflicts: Vec<PathBuf> = STATIC_FILES
            .iter()
            .map(|(rel, _)| dir.join(rel))
            .filter(|p| p.exists())
            .collect();
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

/// Decide what to put in `.mcp.json` for `command` / `args`.
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
        ".claude/skills/degree-author/catalog-patterns.md",
        ".claude/skills/degree-author/example.yaml",
        ".claude/skills/degree-review/SKILL.md",
        ".claude/skills/degree-analyze/SKILL.md",
        ".claude/skills/stored-programs/SKILL.md",
        ".claude/skills/curriculum-research/SKILL.md",
        ".claude/skills/curriculum-research/ipeds.md",
        ".claude/skills/curriculum-research/sql.md",
        ".claude/skills/curriculum-research/queries/completions_total.sql",
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
    fn settings_json_approves_the_registered_server() {
        let tmp = TempDir::new().expect("tempdir");
        let target = tmp.path().join("proj");

        run(&target, false).expect("init succeeds");

        let body = fs::read_to_string(target.join(".claude/settings.json")).expect("read settings");
        let v: Value = serde_json::from_str(&body).expect("settings.json parses as JSON");
        assert_eq!(
            v["enabledMcpjsonServers"],
            serde_json::json!([nu_analytics::core::init_assets::MCP_SERVER_NAME])
        );
        assert!(
            v.get("mcpServers").is_none(),
            "servers belong in .mcp.json: {v}"
        );
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
        assert!(msg.contains(".mcp.json"), "should name the conflict: {msg}");

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
        assert!(
            v["enabledMcpjsonServers"].is_array(),
            "overwritten with the shipped file"
        );
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
    fn render_mcp_json_escapes_quotes_and_backslashes_in_command() {
        // A Windows-style path with backslashes plus a literal double quote
        // would corrupt the JSON if `render_mcp_json` used naive interpolation
        // instead of `serde_json::to_string`.
        let weird = r#"C:\Program Files\Nu"Analytics\nuanalytics.exe"#;
        let rendered = init_assets::render_mcp_json(weird, &[MCP_SUBCOMMAND.to_string()])
            .expect("render_mcp_json");

        let v: Value = serde_json::from_str(&rendered)
            .expect("rendered .mcp.json must be valid JSON even for odd command paths");
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
