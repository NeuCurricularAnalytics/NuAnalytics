-- The actual term-by-term shortest path for one degree, one row per term.
--
-- Edit the school pattern and variant below, then:
--   nuanalytics db query --sql docs/database/examples/shortest-path-schedule.sql --format table
--
-- `schedule` is JSONB shaped [{term, courses:[...], credits}, ...]; jsonb_array_elements
-- expands it so each term is a row and `--format table` can render it. Nested JSON would
-- otherwise be dropped from the table view and only shown under --format json.
WITH latest_run AS (
    SELECT DISTINCT ON (r.program_key, r.variant)
           r.run_key, r.program_key, r.variant
    FROM   analysis_runs r
    ORDER  BY r.program_key, r.variant, r.created_at DESC
),
chosen AS (
    SELECT pl.schedule, lr.variant, p.name AS degree
    FROM   latest_run lr
    JOIN   analysis_plans pl ON pl.run_key = lr.run_key
    LEFT   JOIN programs     p ON p.program_key = lr.program_key
    LEFT   JOIN institutions i ON i.unitid      = p.unitid
    WHERE  pl.category = 'Shortest Path'
      AND  i.name ILIKE '%hawaii at manoa%'     -- <<< edit this
      AND  lr.variant = 'trimmed'               -- <<< 'full' or 'trimmed'
    LIMIT  1
)
SELECT (t->>'term')::int                          AS term,
       (t->>'credits')::numeric                   AS credits,
       jsonb_array_length(t->'courses')           AS n_courses,
       array_to_string(
           ARRAY(SELECT jsonb_array_elements_text(t->'courses')), ', '
       )                                          AS courses
FROM   chosen, jsonb_array_elements(chosen.schedule) AS t
ORDER  BY 1
