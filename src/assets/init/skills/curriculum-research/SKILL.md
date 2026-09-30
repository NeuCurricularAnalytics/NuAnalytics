---
name: curriculum-research
description: Answer questions about institutions, programs and who graduates from them, using IPEDS completion data and the stored degree programs in the NuAnalytics database. Use for questions such as which schools award computing degrees, how women or Black graduates are represented, how R1 programs compare, or anything that needs a query across schools.
allowed-tools: mcp__nuanalytics__search_institutions mcp__nuanalytics__search_cip_codes mcp__nuanalytics__get_lookup_codes mcp__nuanalytics__get_completion_demographics mcp__nuanalytics__search_degrees mcp__nuanalytics__get_stored_analysis mcp__nuanalytics__get_reference mcp__nuanalytics__query_sql
---

# Curriculum research

**Goal:** a figure the user can trust, with the filters, year and caveats that produced
it.

## Tools, typed first

- `search_institutions` finds schools by name, state, Carnegie class, control, HBCU or
  tribal status. It gives the `unitid`. `with_programs=true` keeps only schools that
  have stored degree programs.
- `search_cip_codes(query=…)` or `search_cip_codes(prefix="11.")` gives CIP codes.
- `get_lookup_codes(table=…)` decodes the codes: `carnegie_class`, `award_levels`,
  `institution_control` and the rest.
- `get_completion_demographics(group_by=…)` gives graduates by race and gender, with
  representation ratios:
  - `total` pools the matched schools;
  - `school` ranks them;
  - `cip` lists one school's programs, and needs `unitid`.
- `search_degrees` and `get_stored_analysis` give the stored programs and their metrics.
- `query_sql(sql=…, params={…})` runs read-only SQL for what the typed tools cannot
  express. Call `get_reference(topic="database")` first.

## What you would otherwise get wrong

Open `ipeds.md` before interpreting any demographics figure. In short:

- **A ratio of 1.0 is parity.** It compares a group's share of the selected graduates
  with its share of the baseline, which is all graduates of the same schools in every
  field, reported as `baseline_completions` and `baseline_pct`. There is no enrollment
  data.
- **Completions count awards, not people.** A double major counts under both CIPs.
  Pass `major_num=1` for one count per graduate.
- **Name the year.** The year defaults to the latest with data; report which one you
  got.
- **Small counts swing.** Report the counts beside every ratio, and use
  `min_completions` when ranking schools.
- **A program can be filed under an unexpected CIP.** When a school returns nothing, use
  the `nearby_cips_with_data` hint rather than concluding it has no such program.

## Done when

- Each figure states its filters, year, award level and `major_num`.
- The ratios come with the counts behind them.
- Any SQL used is shown, with its row cap and whether the result was truncated.

## References

- `ipeds.md`: CIP families, award levels, Carnegie classes and the data's traps. Open it
  before the first demographics answer.
- `sql.md`: open it before writing SQL. It covers row caps, parameters, join keys and
  the newest-run pattern, with worked queries in `queries/`.
