-- Re-key programs for 0.7.0: take the CIP out of `program_key`
--
--   old  prog:<unitid or ref>|<cip>|<catalog_year>|<degree_type>|<degree_id or name>[|v<date>]
--   new  prog:<unitid or ref>|<catalog_year>|<degree_type>|<degree_id or name>[|v<date>]
--
-- For a database created before 0.7.0. A fresh install never needs it: it has no old
-- keys, and `programs-schema.sql` adds the constraint itself.
--
-- Run with psql, as one transaction, with every importer stopped (nightly paused, MCP
-- servers restarted onto 0.7.0 afterwards). A 0.6.x importer running after this would
-- insert duplicates; the constraints at the end refuse its writes instead.
--
--   psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f program-key-drop-cip.sql
--
-- It writes program-key-rekey.csv (old_key,new_key) to the current directory: the
-- nightly-sync queue is rewritten from it.
--
-- `program_key` links six tables by convention, with no foreign key, so all six move
-- together. A key is re-keyed only when it starts with exactly the prefix the row's own
-- columns produce under the old rule — institution, CIP, catalog year and degree type,
-- the same values `base_program_key` used (`starts_with`, not `LIKE`). That is exact: it
-- cannot touch the `prog:<degree_id>|<catalog_year>` or `fp:` forms, which never held a
-- CIP, and re-running the file changes nothing, because a new key never starts with its
-- row's old prefix.
--
-- `analysis_runs.config_fingerprint` hashed the old key and is left as it is: it is an
-- advisory duplicate check, read by nothing, and a stale one only means an identical
-- re-run is not recognised as a duplicate of a pre-migration run.

\set ON_ERROR_STOP on
BEGIN;
SET LOCAL statement_timeout = 0;   -- ~600k child rows; don't let a role default cut it off

CREATE TEMP TABLE rekey ON COMMIT DROP AS
WITH k AS (
    SELECT program_key,
           'prog:' || coalesce(unitid::text, institution_ref) || '|' || coalesce(cip_code, '')
             || '|' || coalesce(catalog_year, '') || '|' || coalesce(degree_type, '') || '|'
             AS old_prefix,
           'prog:' || coalesce(unitid::text, institution_ref)
             || '|' || coalesce(catalog_year, '') || '|' || coalesce(degree_type, '') || '|'
             AS new_prefix
      FROM programs
)
SELECT program_key AS old_key,
       new_prefix || substr(program_key, length(old_prefix) + 1) AS new_key
  FROM k
 WHERE starts_with(program_key, old_prefix);

-- Refuse to continue on anything surprising, rather than leave it to whoever reads the
-- output. On 2026-10-26 every institution-form key matched (2,012 of 2,012).
DO $$
DECLARE
    n_programs  bigint := (SELECT count(*) FROM programs);
    n_rekey     bigint := (SELECT count(*) FROM rekey);
    n_distinct  bigint := (SELECT count(DISTINCT new_key) FROM rekey);
    n_clash     bigint := (SELECT count(*) FROM rekey r JOIN programs p ON p.program_key = r.new_key);
    n_left      bigint := (SELECT count(*) FROM programs p
                            WHERE p.program_key NOT IN (SELECT old_key FROM rekey)
                              AND p.program_key ~ ('^prog:[^|]+\|\d\d\.\d{4}\|'));
BEGIN
    RAISE NOTICE 'programs: %, re-keyed: %', n_programs, n_rekey;
    IF n_distinct <> n_rekey THEN
        RAISE EXCEPTION 'two programs would share a key (% re-keyed, % distinct)', n_rekey, n_distinct;
    END IF;
    IF n_clash > 0 THEN
        RAISE EXCEPTION '% new keys already exist in programs', n_clash;
    END IF;
    IF n_left > 0 THEN
        RAISE EXCEPTION '% keys still carry a CIP but do not match their row''s columns; '
                        'inspect them before re-running', n_left;
    END IF;
END $$;

UPDATE programs                p SET program_key = r.new_key FROM rekey r WHERE p.program_key = r.old_key;
UPDATE program_courses         t SET program_key = r.new_key FROM rekey r WHERE t.program_key = r.old_key;
UPDATE program_requirements    t SET program_key = r.new_key FROM rekey r WHERE t.program_key = r.old_key;
UPDATE analysis_runs           t SET program_key = r.new_key FROM rekey r WHERE t.program_key = r.old_key;
UPDATE analysis_plans          t SET program_key = r.new_key FROM rekey r WHERE t.program_key = r.old_key;
UPDATE analysis_course_metrics t SET program_key = r.new_key FROM rekey r WHERE t.program_key = r.old_key;

-- No child row may point at a program that is not there. Rows already orphaned before
-- the migration would also fail this; on 2026-10-26 there were none in any of the five.
DO $$
DECLARE
    orphans bigint := (
        SELECT (SELECT count(*) FROM program_courses t        WHERE NOT EXISTS (SELECT 1 FROM programs p WHERE p.program_key = t.program_key))
             + (SELECT count(*) FROM program_requirements t   WHERE NOT EXISTS (SELECT 1 FROM programs p WHERE p.program_key = t.program_key))
             + (SELECT count(*) FROM analysis_runs t          WHERE NOT EXISTS (SELECT 1 FROM programs p WHERE p.program_key = t.program_key))
             + (SELECT count(*) FROM analysis_plans t         WHERE NOT EXISTS (SELECT 1 FROM programs p WHERE p.program_key = t.program_key))
             + (SELECT count(*) FROM analysis_course_metrics t WHERE NOT EXISTS (SELECT 1 FROM programs p WHERE p.program_key = t.program_key)));
BEGIN
    IF orphans > 0 THEN
        RAISE EXCEPTION '% child rows point at no program after re-keying', orphans;
    END IF;
END $$;

-- Export the mapping now: the temp table is dropped at COMMIT.
\copy rekey TO 'program-key-rekey.csv' CSV HEADER

-- Refuse pre-0.7.0 keys from here on — the same definitions as programs-schema.sql,
-- whose comment explains why the child tables need one too.
ALTER TABLE programs DROP CONSTRAINT IF EXISTS program_key_has_no_cip;
ALTER TABLE programs ADD CONSTRAINT program_key_has_no_cip CHECK (NOT starts_with(
    program_key,
    'prog:' || coalesce(unitid::text, institution_ref) || '|' || coalesce(cip_code, '')
      || '|' || coalesce(catalog_year, '') || '|' || coalesce(degree_type, '') || '|'));
DO $$
DECLARE
    t text;
BEGIN
    FOREACH t IN ARRAY ARRAY['program_courses', 'program_requirements', 'analysis_runs',
                             'analysis_plans', 'analysis_course_metrics'] LOOP
        EXECUTE format('ALTER TABLE %I DROP CONSTRAINT IF EXISTS program_key_has_no_cip', t);
        EXECUTE format('ALTER TABLE %I ADD CONSTRAINT program_key_has_no_cip CHECK (program_key !~ %L)',
                       t, '^prog:[^|]+\|\d\d\.\d{4}\|');
    END LOOP;
END $$;

COMMIT;
