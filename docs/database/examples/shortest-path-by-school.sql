-- Shortest path to graduation for every stored degree at one school.
--
-- Edit the school pattern below (case-insensitive substring), then:
--   nuanalytics db query --sql docs/database/examples/shortest-path-by-school.sql --format table
--
-- `analysis_plans.category` is one of 'Shortest Path' | 'Longest Path' | 'Random Sample' |
-- 'Calculus-Ready Shortest'. Runs append rather than replace, so DISTINCT ON picks the
-- newest run per program — without it a program analysed twice reports both generations.
-- No foreign keys on this schema, hence LEFT JOIN throughout.
WITH latest_run AS (
    SELECT DISTINCT ON (r.program_key, r.variant)
           r.run_key, r.program_key, r.variant
    FROM   analysis_runs r
    ORDER  BY r.program_key, r.variant, r.created_at DESC
)
SELECT lr.variant,
       pl.terms_required        AS terms,
       round(pl.credits::numeric, 1)         AS credits,
       pl.course_count          AS courses,
       round(pl.total_complexity::numeric, 1) AS complexity,
       p.degree_type,
       p.name                   AS degree
FROM   latest_run lr
JOIN   analysis_plans pl ON pl.run_key = lr.run_key
LEFT   JOIN programs     p ON p.program_key = lr.program_key
LEFT   JOIN institutions i ON i.unitid      = p.unitid
WHERE  pl.category = 'Shortest Path'
  AND  i.name ILIKE '%hawaii at manoa%'        -- <<< edit this
ORDER  BY lr.variant, pl.terms_required, pl.credits
