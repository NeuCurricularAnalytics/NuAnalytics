Read-only SQL over the NuAnalytics database (Postgres). One SELECT or WITH per call; writes are refused by the database itself. Results are capped (200 rows by default, 2,000 at most) and a statement stops after 30 seconds — aggregate rather than page.

Inputs: pass them in `params` and read them from `$1`, a JSON object — `($1->>'unitid')::int`, `$1->>'state'` — never paste values into the SQL.

How the tables fit together — there are no foreign keys, so join explicitly (LEFT JOIN where a match may be missing):

- `institutions.unitid` is the IPEDS unit id; `completions`, `programs` and `institution_completion_totals` carry it.
- `completions` holds every IPEDS completions row: all CIP codes, both majors (`major_num` 1 and 2 — filter to 1 for first majors only), every award level (decode with `award_levels`). CIP `99` is IPEDS's grand-total row, the sum of every other CIP at that school, award level and year: leave it out of every sum (`cip_code <> '99'`) or you count each graduate twice.
- `institution_completion_totals` is each school's completions per award level and year, summed over every CIP except `99` and over both major numbers — equal to `SUM(...)` over `completions` with `cip_code <> '99'`, grouped by `unitid, award_level, year`. Use it for whole-school totals; sum `completions` yourself for anything narrower (first majors only, a CIP family).
- `programs.program_key` identifies a stored degree program; `degree_id` is a slug that can span catalog years. `programs.document` is the lossless degree (unified JSON).
- `analysis_runs` append — re-importing adds a run — so the metrics for a program are its newest run per `variant` (`full` or `trimmed`): `SELECT DISTINCT ON (program_key, variant) … ORDER BY program_key, variant, created_at DESC`. `complexity_mean`, `delay_mean`, `credits_mean` are promoted from `degree_metrics` (jsonb).
- `analysis_plans` and `analysis_course_metrics` belong to a run by `run_key`. `analysis_plans.category` is one of `Shortest Path`, `Longest Path`, `Random Sample`, `Calculus-Ready Shortest`; `schedule` is jsonb, one element per term.
- Lookup tables (`carnegie_class`, `award_levels`, `institution_control`, `institution_level`, `institution_sector`, `institution_locale`, `institution_size`) map codes to labels: `code`, `label`.
