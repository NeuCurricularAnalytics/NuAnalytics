//! `db cip` — record the CIP codes schools confirm (the `program_cips` registry).
//!
//! The logic is in [`nu_analytics::database::cips`]; this module parses, prints, and
//! sets the exit status.

use nu_analytics::config::Config;
use nu_analytics::database::cips::{
    self, parse_academic_year, parse_school, span_label, NewCip, ReapplyReport, RegisterError,
    Registration, RowSelection,
};
use nu_analytics::database::DbClient;

use super::db::{connect_or_exit, make_runtime, report_db_error};
use crate::args::CipSubcommand;

/// How many of a school's stored `degree_id`s an unbuilt registration lists.
const SHOWN_DEGREE_IDS: usize = 20;

/// Run one `db cip` subcommand. Exits 1 on any failure.
pub fn run(subcommand: CipSubcommand, config: &Config) {
    let Some(rt) = make_runtime() else {
        std::process::exit(1)
    };
    let client = connect_or_exit(&rt, config);
    let ok = match subcommand {
        CipSubcommand::Set(args) => {
            let dry_run = args.dry_run;
            rt.block_on(set_one(&client, config, args.into(), dry_run))
        }
        CipSubcommand::Import { file, apply } => {
            rt.block_on(import_file(&client, config, &file, apply))
        }
        CipSubcommand::List {
            school,
            unbuilt,
            format,
        } => {
            let json = rt.block_on(list(&client, school.as_deref(), unbuilt));
            println!("{}", crate::output::render(&json, format));
            !super::db::is_error_payload(&json)
        }
        CipSubcommand::Relink {
            school,
            old_id,
            new_id,
        } => rt.block_on(relink(&client, config, &school, &old_id, &new_id)),
        CipSubcommand::Remove {
            school,
            degree_id,
            from,
            all_years,
        } => {
            let which = RowSelection::from_flags(all_years, from);
            rt.block_on(remove(&client, config, &school, &degree_id, which))
        }
    };
    if !ok {
        std::process::exit(1);
    }
}

fn report_failure(config: &Config, e: &RegisterError) {
    eprintln!("✗ {e}");
    if let RegisterError::Database(db) = e {
        report_db_error(db, &config.database.endpoint);
    }
}

/// The school's `institution_ref`, or `None` having said why not.
fn school_ref_or_report(school: &str) -> Option<String> {
    match parse_school(school) {
        Ok((inst_ref, _)) => Some(inst_ref),
        Err(e) => {
            eprintln!("✗ {e}");
            None
        }
    }
}

fn print_reapplied(report: &ReapplyReport, dry_run: bool) {
    let verb = if dry_run { "would change" } else { "changed" };
    println!(
        "  stored programs: {} — {} {verb}",
        report.programs,
        report.changed.len()
    );
    for c in &report.changed {
        if c.from.as_deref() == Some(c.to.as_str()) {
            println!("    {}: {} — now marked confirmed", c.program_key, c.to);
        } else {
            println!(
                "    {}: {} → {}",
                c.program_key,
                c.from.as_deref().unwrap_or("(none)"),
                c.to
            );
        }
    }
    for key in &report.uncovered {
        let note = if report.unconfirmed.contains(key) {
            "; its stale confirmation note was removed"
        } else {
            ""
        };
        println!("    {key}: kept its CIP (no confirmed row covers its year){note}");
    }
    for key in &report.unreadable {
        println!("    {key}: left as is (its stored document could not be read)");
    }
}

fn print_registration(r: &Registration, dry_run: bool) {
    let mark = if dry_run { "ℹ" } else { "✓" };
    let state = if r.built { "built" } else { "not built" };
    println!(
        "{mark} {} {} at {}: {} ({}){}",
        r.row.degree_id,
        r.row.name,
        r.row.institution_ref,
        r.row.cip_code,
        span_label(r.row.from_year),
        if dry_run {
            " — dry run, nothing written"
        } else {
            ""
        }
    );
    println!(
        "  program: {state}; award level {}",
        r.row
            .award_level
            .map_or_else(|| "unknown".to_string(), |l| l.to_string())
    );
    print_reapplied(&r.reapplied, dry_run);
    for w in &r.warnings {
        println!("  ⚠ {w}");
    }
    if !r.built && !r.school_degree_ids.is_empty() {
        println!("  degree_ids stored for this school:");
        for id in r.school_degree_ids.iter().take(SHOWN_DEGREE_IDS) {
            println!("    {id}");
        }
        if r.school_degree_ids.len() > SHOWN_DEGREE_IDS {
            println!(
                "    … and {} more",
                r.school_degree_ids.len() - SHOWN_DEGREE_IDS
            );
        }
    }
}

async fn set_one(client: &DbClient, config: &Config, new: NewCip, dry_run: bool) -> bool {
    match cips::register(client, new, dry_run).await {
        Ok(r) => {
            print_registration(&r, dry_run);
            true
        }
        Err(e) => {
            report_failure(config, &e);
            false
        }
    }
}

/// One row of a `db cip import` file.
#[derive(Debug, serde::Deserialize)]
struct CsvRow {
    school: String,
    degree_id: String,
    cip_code: String,
    #[serde(default)]
    from: String,
    source: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    degree_type: String,
    #[serde(default)]
    award_level: String,
    #[serde(default)]
    note: String,
}

fn non_blank(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

impl CsvRow {
    fn into_new(self) -> Result<NewCip, String> {
        let from_year = match self.from.trim() {
            "" => None,
            y => Some(parse_academic_year(y).map_err(|e| format!("from: {e}"))?),
        };
        let award_level = match self.award_level.trim() {
            "" => None,
            n => Some(
                n.parse()
                    .map_err(|_| format!("award_level must be a number, got \"{n}\""))?,
            ),
        };
        Ok(NewCip {
            school: self.school,
            degree_id: self.degree_id.trim().to_string(),
            cip_code: self.cip_code,
            from_year,
            source: self.source,
            name: non_blank(&self.name),
            degree_type: non_blank(&self.degree_type),
            award_level,
            note: non_blank(&self.note),
        })
    }
}

/// The CSV reader for `path`, or `None` having said why not.
fn open_csv(path: &std::path::Path) -> Option<csv::Reader<std::fs::File>> {
    match csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_path(path)
    {
        Ok(r) => Some(r),
        Err(e) => {
            eprintln!("✗ cannot read {}: {e}", path.display());
            None
        }
    }
}

async fn import_file(
    client: &DbClient,
    config: &Config,
    path: &std::path::Path,
    apply: bool,
) -> bool {
    let Some(mut reader) = open_csv(path) else {
        return false;
    };
    let dry_run = !apply;
    let (mut ok, mut failed) = (0usize, 0usize);
    for (i, record) in reader.deserialize::<CsvRow>().enumerate() {
        // Line 1 is the header.
        let line = i + 2;
        let new = match record.map_err(|e| e.to_string()).and_then(CsvRow::into_new) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("✗ {}:{line}: {e}", path.display());
                failed += 1;
                continue;
            }
        };
        match cips::register(client, new, dry_run).await {
            Ok(r) => {
                print_registration(&r, dry_run);
                ok += 1;
            }
            Err(RegisterError::Invalid(m)) => {
                eprintln!("✗ {}:{line}: {m}", path.display());
                failed += 1;
            }
            Err(e) => {
                // A backend failure will fail every later row too: stop.
                report_failure(config, &e);
                return false;
            }
        }
    }
    let verb = if apply {
        "recorded"
    } else {
        "checked (dry run; --apply writes)"
    };
    println!("{ok} {verb}, {failed} refused");
    failed == 0
}

async fn list(client: &DbClient, school: Option<&str>, unbuilt_only: bool) -> String {
    let refs = match school.map(parse_school).transpose() {
        Ok(s) => s.map(|(r, _)| vec![r]).unwrap_or_default(),
        Err(e) => return nu_analytics::json::bad_arguments(e),
    };
    match cips::list(client, &refs, unbuilt_only).await {
        Ok(rows) => serde_json::json!({ "count": rows.len(), "cips": rows }).to_string(),
        Err(e) => e.to_json("reading program_cips"),
    }
}

async fn relink(client: &DbClient, config: &Config, school: &str, old: &str, new: &str) -> bool {
    let Some(inst_ref) = school_ref_or_report(school) else {
        return false;
    };
    match cips::relink(client, &inst_ref, old, new).await {
        Ok((0, _)) => {
            eprintln!("✗ no confirmed CIP is recorded for {old} at {inst_ref}");
            false
        }
        Ok((moved, report)) => {
            println!("✓ {moved} row(s) moved from {old} to {new} at {inst_ref}");
            print_reapplied(&report, false);
            true
        }
        Err(e) => {
            report_failure(config, &RegisterError::Database(e));
            false
        }
    }
}

async fn remove(
    client: &DbClient,
    config: &Config,
    school: &str,
    degree_id: &str,
    which: RowSelection,
) -> bool {
    let Some(inst_ref) = school_ref_or_report(school) else {
        return false;
    };
    match cips::remove(client, &inst_ref, degree_id, which).await {
        Ok((0, _)) => {
            eprintln!(
                "✗ nothing to remove for {degree_id} at {inst_ref} ({})",
                which.label()
            );
            false
        }
        Ok((n, report)) => {
            println!(
                "✓ {n} row(s) removed for {degree_id} at {inst_ref}. A stored program no row \
                 covers keeps the CIP it has; the guess it replaced is not restored."
            );
            print_reapplied(&report, false);
            true
        }
        Err(e) => {
            report_failure(config, &RegisterError::Database(e));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(csv_text: &str) -> Vec<Result<NewCip, String>> {
        csv::ReaderBuilder::new()
            .trim(csv::Trim::All)
            .from_reader(csv_text.as_bytes())
            .deserialize::<CsvRow>()
            .map(|r| r.map_err(|e| e.to_string()).and_then(CsvRow::into_new))
            .collect()
    }

    #[test]
    fn test_csv_rows_read_blanks_as_absent_and_refuse_bad_values() {
        let got = parse(
            "school,degree_id,cip_code,from,source,name,degree_type,award_level,note\n\
             167358, neu-cs ,11.0701,,registrar,,,,\n\
             167358,neu-ai,11.0102,2024-2025,dept,BS in AI,BS,5,new\n\
             167358,neu-x,11.0102,2024,dept,,,,\n\
             167358,neu-y,11.0102,,dept,,,five,\n",
        );
        let a = got[0].as_ref().unwrap();
        assert_eq!(
            (
                a.degree_id.as_str(),
                a.from_year,
                a.name.as_deref(),
                a.award_level
            ),
            ("neu-cs", None, None, None)
        );
        let b = got[1].as_ref().unwrap();
        assert_eq!(
            (b.from_year, b.award_level, b.note.as_deref()),
            (Some(2024), Some(5), Some("new"))
        );
        let bad_year = got[2].as_ref().unwrap_err();
        assert!(
            bad_year.starts_with("from: ") && bad_year.contains("YYYY-YYYY"),
            "{bad_year}"
        );
        assert!(got[3]
            .as_ref()
            .unwrap_err()
            .contains("award_level must be a number"));
        // The optional columns may be left out entirely.
        assert!(
            parse("school,degree_id,cip_code,source\n167358,neu-cs,11.0701,registrar\n")[0].is_ok()
        );
    }
}
