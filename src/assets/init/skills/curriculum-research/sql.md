# Writing SQL for query_sql

Use SQL only for what the typed tools cannot express: joins across tables, rankings over
the whole corpus, or a figure the tools do not compute. Call
`get_reference(topic="database")` first. It lists every table and column, and how they
join. `get_reference(topic="database", table="analysis_runs")` shows one table in full.

## The rules the server enforces

- **One read-only statement per call:** `SELECT` or `WITH`. A write is refused by the
  client, and by the database if it gets that far.
- **Rows are capped:** 200 by default, 2,000 at most (`max_rows`). When the cap cuts
  the answer off, the response says `truncated`. Aggregate rather than page.
- **30 seconds per statement.** A timeout means: add filters, or aggregate sooner.
- **Inputs go in `params`, never in the SQL text.** `params` is a JSON object; read it
  from `$1`:

  ```sql
  SELECT name, state FROM institutions
  WHERE state = $1->>'state' AND carnegie_class = ($1->>'carnegie')::int
  ```

  ```json
  {"sql": "…", "params": {"state": "CA", "carnegie": 15}}
  ```

## What the schema does not say

- **There are no foreign keys.** Join explicitly, and use `LEFT JOIN` where a match can
  be missing.
- **`unitid` joins everything about a school:** `institutions`, `completions`,
  `programs`.
- **`program_key`** identifies a stored program. `run_key` ties `analysis_plans` and
  `analysis_course_metrics` to their run in `analysis_runs`.
- **Runs append.** A program's current figures are its newest run per variant:

  ```sql
  SELECT DISTINCT ON (program_key, variant) program_key, variant, complexity_mean, created_at
  FROM analysis_runs
  ORDER BY program_key, variant, created_at DESC
  ```

- **`completions` holds CIP `99` grand-total rows.** Filter them out of every sum:
  `cip_code <> '99'`.
- **Sum `completions` rather than trusting `institution_completion_totals`.** The
  reference says why.

## Worked queries (in `queries/`)

These are the server's own queries. The demographics tools run exactly this text, so
they show how the figures are computed.

- `completions_total.sql`: completions pooled over the matched schools, with the
  baseline.
- `completions_by_school.sql`: the same for each school, ranked.
- `completions_by_cip.sql`: one school's programs, with the nearby-CIP fallback.
- `institutions_with_programs.sql`: schools that have stored programs, with those
  programs.

Each opens with an `arg` CTE that reads every parameter from `$1`. To reuse one, pass
the same keys in `params`.

From the project's examples:

- `hardest-degrees.sql`: the most complex stored degrees, newest run per program.
- `shortest-path-by-school.sql`: the shortest plan of every degree at one school.
- `shortest-path-schedule.sql`: one degree's shortest plan, term by term.

The two shortest-path queries embed the school as a literal (`-- <<< edit this`).
Replace it with `$1->>'school'` and pass `params` rather than editing the text.
