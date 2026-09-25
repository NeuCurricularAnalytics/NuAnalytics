-- The 15 most structurally complex stored degrees, with their institution.
--
-- This is the kind of question `db query` subcommands deliberately cannot express: it
-- spans three tables and needs the newest analysis run per program. `db query metrics`
-- answers it for one degree; this answers it across the corpus.
--
--   nuanalytics db query --sql docs/database/examples/hardest-degrees.sql
--
-- DISTINCT ON takes the first row of each program_key group, so the ORDER BY inside the
-- CTE decides which run is "current" — newest first, matching what `db query metrics`
-- reports. There are no foreign keys on this schema, so every join is a LEFT JOIN.
WITH latest_run AS (
    SELECT DISTINCT ON (r.program_key)
           r.program_key,
           r.variant,
           r.complexity_mean,
           r.delay_mean,
           r.credits_mean,
           r.created_at
    FROM   analysis_runs r
    WHERE  r.variant = 'full'
    ORDER  BY r.program_key, r.created_at DESC
)
SELECT round(lr.complexity_mean::numeric, 1) AS complexity,
       round(lr.credits_mean::numeric, 1)    AS credits,
       p.degree_type,
       p.catalog_year,
       i.state,
       i.name                                AS school,
       p.name                                AS degree
FROM   latest_run lr
LEFT   JOIN programs     p ON p.program_key = lr.program_key
LEFT   JOIN institutions i ON i.unitid      = p.unitid
WHERE  lr.complexity_mean IS NOT NULL
ORDER  BY lr.complexity_mean DESC
LIMIT  15
