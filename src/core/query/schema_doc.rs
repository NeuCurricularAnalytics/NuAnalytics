//! A description of the database for someone writing SQL against it.
//!
//! Generated from the schema files compiled into the binary
//! ([`crate::core::database::bootstrap::SCHEMA_FILES`]), so it describes the
//! tables this build expects and cannot drift from them, plus curated notes on what the
//! DDL does not say — how tables join, which rows not to sum. It needs no database, so it
//! answers even when the backend is down.

use std::collections::BTreeMap;

use super::sql::{lex, Span};
use crate::core::database::bootstrap::SCHEMA_FILES;
use serde::Serialize;

/// What the DDL does not say: joins, pitfalls, how to pass inputs.
const NOTES: &str = include_str!("schema_notes.md");

/// One table as the schema files define it.
#[derive(Debug, Clone, Serialize)]
pub struct TableDoc {
    /// Table name.
    pub name: String,
    /// Column names, in definition order, including columns added later by `ALTER TABLE`.
    pub columns: Vec<String>,
    /// The `CREATE TABLE` statement and any `ADD COLUMN`s, with their comments.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ddl: String,
}

/// Every table the schema files create, by name.
#[must_use]
pub fn tables() -> BTreeMap<String, TableDoc> {
    let mut out = BTreeMap::new();
    for file in &SCHEMA_FILES {
        for (name, ddl) in create_statements(file.sql) {
            let columns = column_names(&ddl);
            out.insert(name.clone(), TableDoc { name, columns, ddl });
        }
    }
    for file in &SCHEMA_FILES {
        for line in file.sql.lines() {
            let Some((table, column)) = added_column(line) else {
                continue;
            };
            if let Some(doc) = out.get_mut(&table) {
                if !doc.columns.contains(&column) {
                    doc.columns.push(column);
                }
                doc.ddl.push('\n');
                doc.ddl.push_str(line.trim());
            }
        }
    }
    out
}

/// The whole database in brief, or one table in full.
///
/// With no table: the notes and every table's columns. With a table: its full definition.
/// An unknown table is an error naming the ones there are.
#[must_use]
pub fn describe(table: Option<&str>) -> serde_json::Value {
    let tables = tables();
    let overview = || {
        serde_json::json!({
            "notes": NOTES,
            "tables": tables
                .values()
                .map(|t| serde_json::json!({ "name": t.name, "columns": t.columns }))
                .collect::<Vec<_>>(),
        })
    };
    let one = |name: &str| {
        tables.get(name).map_or_else(
            || {
                serde_json::json!({
                    "error": format!("no table `{name}`"),
                    "code": "bad_arguments",
                    "tables": tables.keys().collect::<Vec<_>>(),
                })
            },
            |t| serde_json::json!({ "name": t.name, "columns": t.columns, "ddl": t.ddl }),
        )
    };
    table.map_or_else(overview, one)
}

/// `(name, statement)` for each `CREATE TABLE` statement in `sql`.
///
/// Only a line that *starts* `CREATE TABLE` opens one: the files' comments mention the
/// phrase too (`CREATE TABLE IF NOT EXISTS` is a no-op on an existing table, …).
fn create_statements(sql: &str) -> Vec<(String, String)> {
    const OPENER: &str = "CREATE TABLE ";
    let mut out = Vec::new();
    let mut offset = 0;
    for line in sql.split_inclusive('\n') {
        if line.starts_with(OPENER) {
            let from = &sql[offset..];
            let header_end = from.find('(').unwrap_or(from.len());
            let name = from[OPENER.len()..header_end]
                .trim()
                .trim_start_matches("IF NOT EXISTS ")
                .trim()
                .to_string();
            out.push((name, from[..statement_end(from)].to_string()));
        }
        offset += line.len();
    }
    out
}

/// Byte length of the parenthesised statement starting at `from`, through its `;`.
fn statement_end(from: &str) -> usize {
    let mut depth = 0usize;
    for (i, c) in from.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ';' if depth == 0 => return i + 1,
            _ => {}
        }
    }
    from.len()
}

/// Column names in a `CREATE TABLE` statement, skipping constraints and comments.
fn column_names(ddl: &str) -> Vec<String> {
    let Some(open) = ddl.find('(') else {
        return Vec::new();
    };
    let body = &ddl[open + 1..ddl.rfind(')').unwrap_or(ddl.len())];
    // Split on the lexer's spans, not the raw text: a comma in a comment
    // (`-- 'R1', 'R2', ...`) or a literal is not a column boundary, and a comment
    // dropped only after splitting both invents columns and swallows the next real one.
    let mut columns = Vec::new();
    let mut depth = 0usize;
    let mut item = String::new();
    for (span, text) in lex(body) {
        match span {
            Span::Comment => item.push(' '),
            Span::Literal => item.push_str(text),
            Span::Code => {
                for c in text.chars() {
                    match c {
                        '(' => depth += 1,
                        ')' => depth = depth.saturating_sub(1),
                        ',' if depth == 0 => {
                            columns.extend(column_of(&item));
                            item.clear();
                            continue;
                        }
                        _ => {}
                    }
                    item.push(c);
                }
            }
        }
    }
    columns.extend(column_of(&item));
    columns
}

/// The column one comma-separated item of a table body defines, if it defines one.
fn column_of(item: &str) -> Option<String> {
    let first = item.split_whitespace().next()?;
    let keyword = first.to_ascii_uppercase();
    if [
        "PRIMARY",
        "UNIQUE",
        "CONSTRAINT",
        "FOREIGN",
        "CHECK",
        "EXCLUDE",
    ]
    .contains(&keyword.as_str())
    {
        return None;
    }
    Some(first.trim_matches('"').to_string())
}

/// `(table, column)` for an `ALTER TABLE t ADD COLUMN IF NOT EXISTS c …` line.
fn added_column(line: &str) -> Option<(String, String)> {
    let rest = line.trim().strip_prefix("ALTER TABLE ")?;
    let mut words = rest.split_whitespace();
    let table = words.next()?.to_string();
    let words: Vec<&str> = words.collect();
    let at = words.windows(2).position(|w| w == ["ADD", "COLUMN"])?;
    let mut after = words[at + 2..].iter();
    let mut column = after.next()?;
    if *column == "IF" {
        after.next(); // NOT
        after.next(); // EXISTS
        column = after.next()?;
    }
    Some((table, (*column).to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::database::tables;
    use std::collections::BTreeSet;

    #[test]
    fn the_description_covers_exactly_the_tables_the_client_knows() {
        let described: BTreeSet<String> = tables().into_keys().collect();
        let known: BTreeSet<String> = tables::ALL.iter().map(|t| (*t).to_string()).collect();
        assert_eq!(described, known, "schema files vs tables::ALL");
    }

    #[test]
    fn columns_include_later_additions_and_skip_constraints() {
        let all = tables();
        let completions = &all["completions"];
        assert!(completions.columns.contains(&"cip_code".to_string()));
        assert!(completions
            .columns
            .contains(&"unknown_race_women".to_string()));
        assert!(
            !completions
                .columns
                .iter()
                .any(|c| c.eq_ignore_ascii_case("unique")),
            "{:?}",
            completions.columns
        );
        let requirements = &all["program_requirements"];
        assert!(
            requirements.columns.contains(&"external_note".to_string()),
            "ALTER TABLE ... ADD COLUMN is picked up: {:?}",
            requirements.columns
        );
    }

    #[test]
    fn describe_gives_the_overview_one_table_or_the_names_to_pick_from() {
        let overview = describe(None);
        assert!(overview["notes"]
            .as_str()
            .unwrap()
            .contains("cip_code <> '99'"));
        assert!(overview["tables"].as_array().unwrap().len() >= 20);
        let one = describe(Some("analysis_runs"));
        assert!(one["ddl"].as_str().unwrap().contains("CREATE TABLE"));
        let unknown = describe(Some("nope"));
        assert_eq!(unknown["code"], "bad_arguments");
        assert!(unknown["tables"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "programs"));
    }

    #[test]
    fn every_listed_column_is_defined_and_none_is_lost_to_a_comment() {
        let all = tables();
        for (name, doc) in &all {
            // Identifiers the DDL's code spans hold: a word that appears only in a comment
            // (`so`, `etc.`) or a literal (`'R2'`) is not among them.
            let code_words: BTreeSet<&str> = lex(&doc.ddl)
                .into_iter()
                .filter(|(span, _)| *span == Span::Code)
                .flat_map(|(_, text)| text.split(|c: char| !(c.is_alphanumeric() || c == '_')))
                .collect();
            for column in &doc.columns {
                assert!(
                    code_words.contains(column.as_str()),
                    "{name}: `{column}` is not a column: {:?}",
                    doc.columns
                );
            }
        }
        let runs = &all["analysis_runs"].columns;
        for column in [
            "complexity_mean",
            "random_seed",
            "config_fingerprint",
            "backfilled_metrics",
        ] {
            assert!(
                runs.iter().any(|c| c == column),
                "analysis_runs lacks `{column}`: {runs:?}"
            );
        }
    }

    #[test]
    fn column_names_ignore_commas_in_comments_and_literals() {
        let ddl = "CREATE TABLE t (\n    a INT, -- 'x', 'y', so\n    b TEXT DEFAULT 'p, q',\n    \"order\" INT,\n    CHECK (a IN (1, 2))\n);";
        assert_eq!(column_names(ddl), ["a", "b", "order"]);
    }

    #[test]
    fn added_column_reads_the_if_not_exists_form() {
        assert_eq!(
            added_column("ALTER TABLE programs ADD COLUMN IF NOT EXISTS created_by UUID;"),
            Some(("programs".to_string(), "created_by".to_string()))
        );
        assert_eq!(
            added_column("ALTER TABLE t ENABLE ROW LEVEL SECURITY;"),
            None
        );
    }
}
