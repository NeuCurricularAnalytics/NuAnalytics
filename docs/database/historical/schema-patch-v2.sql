-- Schema Patch v2
-- Run against an existing database that was created before major_num was added
-- to the completions table and before institution_completion_totals existed.
-- Safe to run with data in place — uses ADD COLUMN IF NOT EXISTS and named
-- constraints so it is idempotent.
--
-- If you are setting up a fresh database, run schema.sql instead — it already
-- includes all of the below.

-- =============================================================================
-- Fix completions table: add major_num column + updated unique constraint
-- =============================================================================

ALTER TABLE completions ADD COLUMN IF NOT EXISTS major_num INTEGER;

-- Drop the old constraint (without major_num)
ALTER TABLE completions
    DROP CONSTRAINT IF EXISTS completions_unitid_cip_code_award_level_year_key;

-- Add the new constraint (with major_num)
ALTER TABLE completions
    ADD CONSTRAINT completions_unique
    UNIQUE (unitid, cip_code, award_level, major_num, year);

-- =============================================================================
-- Add institution_completion_totals cache table (if it doesn't exist)
-- =============================================================================

CREATE TABLE IF NOT EXISTS institution_completion_totals (
    id                       BIGSERIAL PRIMARY KEY,
    unitid                   INTEGER,
    award_level              INTEGER,
    year                     INTEGER,
    total                    INTEGER,
    total_men                INTEGER,
    total_women              INTEGER,
    nonresident_alien_men    INTEGER,   nonresident_alien_women    INTEGER,
    hispanic_men             INTEGER,   hispanic_women             INTEGER,
    american_indian_men      INTEGER,   american_indian_women      INTEGER,
    asian_men                INTEGER,   asian_women                INTEGER,
    black_men                INTEGER,   black_women                INTEGER,
    native_hawaiian_men      INTEGER,   native_hawaiian_women      INTEGER,
    white_men                INTEGER,   white_women                INTEGER,
    two_or_more_men          INTEGER,   two_or_more_women          INTEGER,
    unknown_race_men         INTEGER,   unknown_race_women         INTEGER,
    UNIQUE (unitid, award_level, year)
);

CREATE INDEX IF NOT EXISTS idx_inst_totals_unitid ON institution_completion_totals (unitid);
CREATE INDEX IF NOT EXISTS idx_inst_totals_year   ON institution_completion_totals (year);

-- =============================================================================
-- RLS for institution_completion_totals
-- =============================================================================

ALTER TABLE institution_completion_totals ENABLE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS "public read institution_completion_totals" ON institution_completion_totals;
DROP POLICY IF EXISTS "auth read institution_completion_totals"   ON institution_completion_totals;
CREATE POLICY "auth read institution_completion_totals"
    ON institution_completion_totals FOR SELECT USING (auth.role() = 'authenticated');

DROP POLICY IF EXISTS "auth write institution_completion_totals" ON institution_completion_totals;
CREATE POLICY "auth write institution_completion_totals"
    ON institution_completion_totals
    FOR ALL
    USING (auth.role() = 'authenticated')
    WITH CHECK (auth.role() = 'authenticated');
