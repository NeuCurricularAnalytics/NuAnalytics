//! Rendering for `db query` results.
//!
//! JSON is the default because the intended caller is a script or an LLM; `--format
//! table` is for humans reading a terminal. Success payloads arrive pretty-printed from
//! the engines and error payloads arrive compact (`json!(...).to_string()`); `Json`
//! passes both through untouched rather than risk reformatting them.
//!
//! The table renderer works off `serde_json::Value` rather than typed rows on purpose:
//! the six engines return six unrelated shapes, and a per-command table spec would have
//! to be maintained in lockstep with every response struct.

use std::fmt::Write as _;

use clap::ValueEnum;

/// How `db query` prints its results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    /// Pretty-printed JSON (default) — what an LLM or `jq` wants.
    Json,
    /// Aligned columns for reading in a terminal.
    Table,
}

/// Footer naming the columns dropped for being nested. A constant so the tests cannot
/// pass against a message that changed.
const NESTED_NOTE: &str = "nested, use --format json: ";

/// Longest cell rendered before truncation, so one long `document` field cannot push
/// every other column off the screen.
const MAX_CELL: usize = 48;

/// Render an engine's JSON response in the requested format.
///
/// `Json` passes the text through untouched — the engines already pretty-print, and
/// re-parsing only risks changing it.
#[must_use]
pub fn render(json_text: &str, format: OutputFormat) -> String {
    match format {
        OutputFormat::Json => json_text.to_string(),
        // Not parseable as JSON: hand back what we got rather than swallow it. An engine
        // that failed before producing JSON still has something worth reading.
        OutputFormat::Table => serde_json::from_str::<serde_json::Value>(json_text)
            .map_or_else(|_| json_text.to_string(), |v| render_table(&v)),
    }
}

/// Render a parsed response as a table, falling back to JSON when there is no single
/// obvious row set.
fn render_table(value: &serde_json::Value) -> String {
    match find_rows(value) {
        Some(rows) if !rows.is_empty() => {
            let mut out = rows_to_table(rows);
            // Silently dropping a column would make the table look like the whole answer.
            let nested = nested_columns(rows);
            if !nested.is_empty() {
                let _ = write!(out, "\n{NESTED_NOTE}{}", nested.join(", "));
            }
            // Scalar siblings of the row array carry the context that makes a count
            // meaningful ("year": 2025, "truncated": true) and would otherwise be lost.
            if let Some(obj) = value.as_object() {
                let notes: Vec<String> = obj
                    .iter()
                    .filter(|(_, v)| !v.is_array() && !v.is_object())
                    .map(|(k, v)| format!("{k}: {}", scalar(v)))
                    .collect();
                if !notes.is_empty() {
                    let _ = write!(out, "\n{}", notes.join("  |  "));
                }
            }
            out
        }
        Some(_) => "(no rows)".to_string(),
        // An error payload, a single object, or several arrays with no obvious subject:
        // JSON is the honest rendering rather than a misleading one-row table.
        None => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    }
}

/// Tie-break for a response carrying more than one array, in priority order.
///
/// `demographics` and `rows` win over their sibling `cross_tab`, which is the race x
/// gender expansion of the same figures rather than a second subject — `cross_tab` is
/// excluded by not appearing here at all. `runs` is defensive: `metrics` currently
/// returns a single array and so takes the sole-array branch instead.
const SUBJECT_KEYS: [&str; 3] = ["demographics", "rows", "runs"];

/// The array of rows in a response.
///
/// A response that *is* an array is itself the row set. Otherwise engine responses look
/// like `{"count": 12, "institutions": [ ... ]}`. Picking the sole
/// array is what lets one renderer serve every command without a per-command table spec.
/// When there are several, only a `SUBJECT_KEYS` name breaks the tie — otherwise the
/// choice would be a guess and JSON is the honest rendering.
fn find_rows(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    if let Some(arr) = value.as_array() {
        return Some(arr);
    }
    let obj = value.as_object()?;
    let mut arrays = obj.values().filter_map(serde_json::Value::as_array);
    let first = arrays.next()?;
    if arrays.next().is_none() {
        return Some(first);
    }
    SUBJECT_KEYS
        .iter()
        .find_map(|k| obj.get(*k).and_then(serde_json::Value::as_array))
}

/// Columns a reader looks for first, in this order, before everything else.
///
/// Without this the table is alphabetical, which buries the only two columns anyone
/// scans for: a `schools` listing led with `carnegie_class` and put `name` sixth and
/// `unitid` last. `serde_json::Map` is a `BTreeMap` (this crate does not enable
/// `preserve_order`), so struct field order is not available to fall back on — and
/// enabling it globally is not an option, because `document_hash` is computed over
/// serialized JSON and would change for every stored program.
const LEAD_COLUMNS: [&str; 10] = [
    "unitid",
    "program_key",
    "degree_id",
    "run_key",
    "variant",
    "group",
    "code",
    "cip_code",
    "name",
    "title",
];

/// Columns whose value is a nested array or object in **any** row.
///
/// A `schools` row carries its whole `demographics` breakdown; rendered as a cell that is
/// a truncated JSON blob it is unreadable and crowds out the columns that do fit. They
/// are dropped from the table and named underneath instead.
///
/// A column that is nested in one row and scalar in the rest is dropped wholesale, losing
/// those scalars — the trade for never rendering a blob inline. When *every* column is
/// nested there are no columns left, and `rows_to_table` falls through to printing each
/// row as compact JSON.
fn nested_columns(rows: &[serde_json::Value]) -> Vec<String> {
    let mut nested: Vec<String> = Vec::new();
    for row in rows {
        let Some(obj) = row.as_object() else { continue };
        for (k, v) in obj {
            if (v.is_array() || v.is_object()) && !nested.iter().any(|c| c == k) {
                nested.push(k.clone());
            }
        }
    }
    nested.sort();
    nested
}

/// Column order: the identity-ish columns above first, then everything else in first-seen
/// order — which is alphabetical in the common case where every row carries the same keys,
/// because `serde_json::Map` is a `BTreeMap`. A key only a later row has is appended after
/// those already seen. Nested columns are excluded — see `nested_columns`.
fn columns(rows: &[serde_json::Value]) -> Vec<String> {
    let nested = nested_columns(rows);
    let mut cols: Vec<String> = Vec::new();
    for row in rows {
        let Some(obj) = row.as_object() else { continue };
        for k in obj.keys() {
            if !cols.iter().any(|c| c == k) && !nested.iter().any(|c| c == k) {
                cols.push(k.clone());
            }
        }
    }
    cols.sort_by_key(|c| {
        LEAD_COLUMNS
            .iter()
            .position(|lead| lead == c)
            .unwrap_or(LEAD_COLUMNS.len())
    });
    cols
}

/// One cell's text. Nested structures collapse to compact JSON so a row stays one line.
fn scalar(v: &serde_json::Value) -> String {
    let raw = match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    if raw.chars().count() > MAX_CELL {
        let head: String = raw.chars().take(MAX_CELL - 1).collect();
        format!("{head}…")
    } else {
        raw
    }
}

/// Render rows as aligned columns: header, rule, then one line per row.
///
/// Rows with no columns at all (an array of scalars, or rows whose every key is nested)
/// print one value per line instead.
fn rows_to_table(rows: &[serde_json::Value]) -> String {
    let cols = columns(rows);
    if cols.is_empty() {
        // An array of scalars (e.g. a list of codes) has no columns to name.
        return rows.iter().map(scalar).collect::<Vec<_>>().join("\n");
    }

    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|row| {
            cols.iter()
                .map(|c| row.get(c).map_or_else(String::new, scalar))
                .collect()
        })
        .collect();

    let widths: Vec<usize> = cols
        .iter()
        .enumerate()
        .map(|(i, c)| {
            cells
                .iter()
                .map(|r| r[i].chars().count())
                .chain(std::iter::once(c.chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();

    let mut out = String::new();
    let header: Vec<String> = cols
        .iter()
        .zip(&widths)
        .map(|(c, w)| format!("{c:<w$}"))
        .collect();
    let _ = writeln!(out, "{}", header.join("  ").trim_end());
    let _ = writeln!(
        out,
        "{}",
        widths
            .iter()
            .map(|w| "-".repeat(*w))
            .collect::<Vec<_>>()
            .join("  ")
    );
    for row in &cells {
        let line: Vec<String> = row
            .iter()
            .zip(&widths)
            .map(|(c, w)| format!("{c:<w$}"))
            .collect();
        let _ = writeln!(out, "{}", line.join("  ").trim_end());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_format_passes_the_text_through_unchanged() {
        // The engines already pretty-print; re-serializing could only change their output.
        let text = "{\n  \"count\": 1\n}";
        assert_eq!(render(text, OutputFormat::Json), text);
    }

    #[test]
    fn a_response_wrapping_one_array_renders_as_a_table() {
        let text = json!({
            "count": 2,
            "institutions": [
                {"unitid": 1, "name": "A University", "state": "HI"},
                {"unitid": 2, "name": "B College", "state": "MA"},
            ]
        })
        .to_string();
        let out = render(&text, OutputFormat::Table);
        assert!(out.contains("unitid"), "header missing: {out}");
        assert!(out.contains("A University"), "row missing: {out}");
        // The scalar sibling is the context for the rows and must survive.
        assert!(out.contains("count: 2"), "count note missing: {out}");
    }

    #[test]
    fn identity_columns_lead_the_table() {
        // The map is a BTreeMap, so without reordering this renders alphabetically and
        // `name`/`unitid` end up buried behind `carnegie_class`. That defeats the point
        // of a table meant to be read.
        let text = json!([{
            "carnegie_class": 15, "state": "HI", "name": "U of H", "unitid": 141_574,
        }])
        .to_string();
        let out = render(&text, OutputFormat::Table);
        let header = out.lines().next().expect("header");
        let pos = |c: &str| header.find(c).unwrap_or(usize::MAX);
        assert!(pos("unitid") < pos("name"), "unitid should lead: {header}");
        assert!(
            pos("name") < pos("carnegie_class"),
            "name should precede the attribute columns: {header}"
        );
    }

    #[test]
    fn non_identity_columns_stay_in_alphabetical_order_behind_the_leaders() {
        // The property: columns the lead list does not name stay in the map's own
        // alphabetical order, so a table's tail is predictable.
        //
        // `sort_by_key` is stable, which guarantees this by contract. Swapping in
        // `sort_unstable_by_key` does NOT fail this test even at sixteen columns —
        // pdqsort happens not to permute all-equal keys — so treat that as an equivalent
        // mutant rather than a hole here. The contract is still the reason to keep the
        // stable sort.
        let mut row = serde_json::Map::new();
        row.insert("name".into(), json!("x"));
        for c in [
            "alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
            "juliet", "kilo", "lima", "mike", "november", "oscar",
        ] {
            row.insert(c.into(), json!(1));
        }
        let text = serde_json::Value::Array(vec![row.into()]).to_string();
        let header = render(&text, OutputFormat::Table)
            .lines()
            .next()
            .expect("header")
            .to_string();
        let cols: Vec<&str> = header.split_whitespace().collect();
        // Without this a `columns()` that dropped half its output still passes — a short
        // list is still sorted.
        assert_eq!(cols.len(), 16, "columns were dropped: {header}");
        assert_eq!(cols[0], "name", "lead column first: {header}");
        let rest = &cols[1..];
        let mut sorted = rest.to_vec();
        sorted.sort_unstable();
        assert_eq!(rest, sorted.as_slice(), "trailing columns not alphabetical");
    }

    #[test]
    fn a_row_missing_a_column_leaves_it_blank_rather_than_shifting_the_row() {
        let text = json!([
            {"a": 1, "b": 2},
            {"a": 3},
        ])
        .to_string();
        let out = render(&text, OutputFormat::Table);
        let body: Vec<&str> = out.lines().skip(2).collect();
        assert_eq!(body.len(), 2, "{out}");
        assert!(body[1].starts_with('3'), "second row shifted: {out}");
    }

    #[test]
    fn a_later_row_can_introduce_a_column() {
        let text = json!([{"a": 1}, {"a": 2, "z": 9}]).to_string();
        let out = render(&text, OutputFormat::Table);
        assert!(out.contains('z'), "new column dropped: {out}");
        assert!(out.contains('9'), "new column's value dropped: {out}");
    }

    #[test]
    fn an_error_payload_is_shown_as_json_not_a_one_row_table() {
        let text = json!({"error": "degree_id not found"}).to_string();
        let out = render(&text, OutputFormat::Table);
        // The message alone is not evidence — a one-row table would contain it too.
        // The JSON quoting is the only thing that tells the two renderings apart.
        assert!(out.starts_with('{'), "expected JSON, got a table: {out}");
        assert!(
            out.contains("\"error\": \"degree_id not found\""),
            "error payload mangled: {out}"
        );
    }

    #[test]
    fn an_empty_result_says_so() {
        let text = json!({"count": 0, "institutions": []}).to_string();
        assert_eq!(render(&text, OutputFormat::Table), "(no rows)");
    }

    #[test]
    fn a_long_cell_is_truncated_so_one_column_cannot_eat_the_row() {
        let long = "x".repeat(200);
        let text = json!([{"doc": long, "id": 1}]).to_string();
        let out = render(&text, OutputFormat::Table);
        assert!(out.contains('…'), "no truncation marker: {out}");
        assert!(
            out.lines().all(|l| l.chars().count() < 120),
            "a line ran long: {out}"
        );
        assert!(out.contains('1'), "other columns must survive: {out}");
    }

    #[test]
    fn a_nested_column_is_dropped_and_named_rather_than_stuffed_into_a_cell() {
        // A `schools` row carries its whole demographics breakdown. Rendered inline it
        // was a truncated JSON blob that crowded out every column that did fit.
        let text = json!([{"id": 1, "metrics": {"complexity": 87}}]).to_string();
        let out = render(&text, OutputFormat::Table);
        assert!(
            !out.contains("complexity"),
            "nested value rendered into a cell: {out}"
        );
        assert!(
            out.contains(&format!("{NESTED_NOTE}metrics")),
            "dropped column not named, so the table looks complete: {out}"
        );
        assert!(out.contains("id"), "scalar column lost: {out}");
    }

    #[test]
    fn every_column_nested_prints_the_row_as_json_and_still_names_the_columns() {
        let text = json!([{"a": [1], "b": {"k": 2}}]).to_string();
        let out = render(&text, OutputFormat::Table);
        // No scalar columns means no table to draw, so the row falls through to the
        // scalar path. The values must not vanish along with the columns.
        assert!(
            out.starts_with(r#"{"a":[1],"b":{"k":2}}"#),
            "row content lost: {out}"
        );
        assert!(out.contains(&format!("{NESTED_NOTE}a, b")), "{out}");
        assert!(
            !out.contains("---"),
            "no header rule without columns: {out}"
        );
    }

    #[test]
    fn a_column_nested_in_only_one_row_is_dropped_for_all_of_them() {
        // The trade recorded in `nested_columns`: scalars in the other rows go too.
        // Pinned so the loss is a decision rather than a surprise.
        let text = json!([{"id": 1, "note": {"k": 2}}, {"id": 2, "note": "plain"}]).to_string();
        let out = render(&text, OutputFormat::Table);
        assert!(
            !out.contains("plain"),
            "scalar survived a nested column: {out}"
        );
        assert!(out.contains(&format!("{NESTED_NOTE}note")), "{out}");
        assert!(out.starts_with("id"), "{out}");
    }

    #[test]
    fn scalar_truncates_only_past_the_limit_and_never_splits_a_character() {
        // Exactly MAX_CELL survives; one more is cut to MAX_CELL *including* the
        // ellipsis, so a column's width is bounded by the constant, not by the data.
        let at_limit = "x".repeat(MAX_CELL);
        assert_eq!(
            scalar(&json!(at_limit.clone())),
            at_limit,
            "boundary cell truncated"
        );

        let cut = scalar(&json!("x".repeat(MAX_CELL + 1)));
        assert_eq!(
            cut.chars().count(),
            MAX_CELL,
            "truncated cell is not MAX_CELL wide: {cut}"
        );
        assert!(cut.ends_with('…'), "no truncation marker: {cut}");

        // Byte slicing here would panic mid-character.
        assert_eq!(
            scalar(&json!("é".repeat(MAX_CELL + 10))).chars().count(),
            MAX_CELL
        );
    }

    #[test]
    fn scalar_renders_each_json_type_the_way_a_table_column_needs_it() {
        // A column of `null` is noise, and the missing-key path already renders blank.
        assert_eq!(scalar(&serde_json::Value::Null), "");
        assert_eq!(scalar(&json!(false)), "false");
        assert_eq!(scalar(&json!(1.5)), "1.5");
        assert_eq!(scalar(&json!(42)), "42");
        assert_eq!(
            scalar(&json!("plain")),
            "plain",
            "strings must not be requoted"
        );
    }

    #[test]
    fn scalar_still_collapses_a_nested_value_for_rows_that_are_not_objects() {
        // `columns` drops nested keys, so this path is only reached by array-shaped
        // rows — keep it working rather than let it rot into a panic.
        assert_eq!(scalar(&json!({"complexity": 87})), "{\"complexity\":87}");
        assert_eq!(scalar(&json!([1, 2])), "[1,2]");
    }

    #[test]
    fn demographics_picks_its_subject_array_instead_of_falling_back_to_json() {
        // Two arrays, but `cross_tab` is the race x gender expansion of `demographics`,
        // not a second subject — declining here would print raw JSON for the single
        // most common demographics query.
        let text = json!({
            "demographics": [{"group": "Women", "completions": 69}],
            "cross_tab": [{"group": "Women", "men_count": 3}],
            "total_completions": 208,
        })
        .to_string();
        let out = render(&text, OutputFormat::Table);
        assert!(out.starts_with("group"), "subject array not chosen: {out}");
        assert!(out.contains("completions"), "{out}");
        assert!(
            !out.contains("men_count"),
            "cross_tab rendered instead of demographics: {out}"
        );
    }

    #[test]
    fn a_named_subject_array_wins_over_an_earlier_unnamed_one() {
        // `context` sorts first in the BTreeMap, but the tie-break is the named list,
        // not map position — otherwise a metrics response that grew a second array
        // would start tabulating the wrong one.
        let text = json!({
            "context": [{"note": "not the subject"}],
            "runs": [{"run_key": "r1"}],
        })
        .to_string();
        let out = render(&text, OutputFormat::Table);
        assert!(
            out.starts_with("run_key"),
            "subject array not chosen: {out}"
        );
        assert!(!out.contains("not the subject"), "{out}");
    }

    #[test]
    fn two_arrays_with_no_subject_key_still_decline() {
        // Choosing between them would be a guess, and a table of the wrong array is
        // worse than JSON. The tie-break is a named list, not "pick the first".
        let text = json!({"alpha": [{"a": 1}], "beta": [{"b": 2}]}).to_string();
        let out = render(&text, OutputFormat::Table);
        assert!(out.contains("\"alpha\""), "expected JSON fallback: {out}");
    }

    #[test]
    fn unparseable_text_is_returned_rather_than_swallowed() {
        // An engine that failed before producing JSON still has something to say.
        let out = render("not json at all", OutputFormat::Table);
        assert_eq!(out, "not json at all");
    }

    #[test]
    fn an_array_of_scalars_renders_one_per_line() {
        let text = json!(["11.0701", "11.0101"]).to_string();
        let out = render(&text, OutputFormat::Table);
        assert_eq!(out, "11.0701\n11.0101");
    }
}
