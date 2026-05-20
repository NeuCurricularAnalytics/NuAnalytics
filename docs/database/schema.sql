-- NuAnalytics Database Schema
-- Run in Supabase SQL Editor: Dashboard → SQL Editor → New query → paste → Run
--
-- After running this file, also run IN ORDER:
--   docs/database/cip-seed.sql      (CIP 2020 taxonomy, ~2173 codes)
--   docs/database/lookup-seed.sql   (award levels, Carnegie, locale, control, etc.)
--
-- See docs/database/setup.md for the full setup guide.

-- =============================================================================
-- Lookup tables
-- Decode the numeric codes used throughout IPEDS into human-readable labels.
-- Seeded from docs/database/lookup-seed.sql.
-- =============================================================================

-- IPEDS C survey AWLEVEL codes (1=cert<1yr … 9=doctoral, etc.)
CREATE TABLE award_levels (
    code      INTEGER PRIMARY KEY,
    label     TEXT NOT NULL,
    is_degree BOOLEAN NOT NULL DEFAULT false
);

-- IPEDS HD CONTROL codes (1=public, 2=nonprofit, 3=for-profit)
CREATE TABLE institution_control (
    code  INTEGER PRIMARY KEY,
    label TEXT NOT NULL
);

-- IPEDS HD ICLEVEL codes (1=4-year, 2=2-year, 3=<2-year)
CREATE TABLE institution_level (
    code  INTEGER PRIMARY KEY,
    label TEXT NOT NULL
);

-- IPEDS HD SECTOR codes (cross of control × level, 0-9)
CREATE TABLE institution_sector (
    code  INTEGER PRIMARY KEY,
    label TEXT NOT NULL
);

-- Carnegie Classification 2021 Basic (C21BASIC in IPEDS HD)
-- Codes 1-33 plus -2 (not applicable). Codes 1-14 = two-year/associate's,
-- 15=R1, 16=R2, 17=Doctoral/Professional, 18-20=Master's, 21-23=Baccalaureate,
-- 24-32=Special Focus Four-Year, 33=Tribal.
CREATE TABLE carnegie_class (
    code           INTEGER PRIMARY KEY,
    label          TEXT    NOT NULL,
    research_level TEXT              -- 'R1', 'R2', 'doctoral', 'masters', 'baccalaureate', etc.
);

-- NCES Urban-Centric Locale codes (LOCALE in IPEDS HD)
CREATE TABLE institution_locale (
    code     INTEGER PRIMARY KEY,
    label    TEXT NOT NULL,
    category TEXT          -- 'City', 'Suburb', 'Town', 'Rural'
);

-- IPEDS HD INSTSIZE codes
CREATE TABLE institution_size (
    code  INTEGER PRIMARY KEY,
    label TEXT NOT NULL
);

-- =============================================================================
-- CIP code taxonomy
-- Seed from docs/database/cip-seed.sql.
-- CIP 2020 taxonomy; all joins to this table use LEFT JOIN.
-- =============================================================================
CREATE TABLE cip_codes (
    cip_code TEXT PRIMARY KEY,   -- e.g. "11.0101"
    title    TEXT NOT NULL
);

-- =============================================================================
-- Institution directory (IPEDS HD survey)
-- =============================================================================
CREATE TABLE institutions (
    unitid         INTEGER PRIMARY KEY,
    name           TEXT    NOT NULL,
    city           TEXT,
    state          TEXT,
    sector         INTEGER,   -- → institution_sector.code  (0-9, 99)
    control        INTEGER,   -- → institution_control.code (1=public, 2=nonprofit, 3=for-profit)
    iclevel        INTEGER,   -- → institution_level.code   (1=4yr+, 2=2yr, 3=<2yr)
    carnegie_class INTEGER,   -- → carnegie_class.code      (C21BASIC: 1-33, -2)
    hbcu           BOOLEAN,
    tribal         BOOLEAN,
    locale         INTEGER,   -- → institution_locale.code
    inst_size      INTEGER,   -- → institution_size.code
    updated_year   INTEGER
);

-- =============================================================================
-- Degree completions (IPEDS C survey — ALL CIP codes, both primary and secondary majors)
--
-- major_num: 1 = primary major, 2 = second major (double-major).
-- Both are stored so CS completions aren't missed when CS is the student's
-- second major. Filter at query time:
--   Primary only:      WHERE major_num = 1
--   All CS:            WHERE cip_code LIKE '11.%' OR cip_code IN ('30.7001','30.7099')
--   Institution total: no CIP filter, SUM demographics GROUP BY unitid, year
--
-- No FK on unitid or cip_code: IPEDS surveys don't guarantee cross-survey coverage.
-- =============================================================================
CREATE TABLE completions (
    id                       BIGSERIAL PRIMARY KEY,
    unitid                   INTEGER,
    cip_code                 TEXT,
    award_level              INTEGER,   -- → award_levels.code
    major_num                INTEGER,   -- 1=primary, 2=second major
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
    UNIQUE (unitid, cip_code, award_level, major_num, year)
);

-- =============================================================================
-- Institution completion totals cache (populated automatically by ipeds-import)
--
-- Pre-aggregated total completions per (institution, award_level, year) across
-- ALL CIP codes. Written in the same single pass as the completions table.
-- Used as the denominator for demographic representation queries — avoids
-- scanning 100K+ completion rows on every MCP tool call.
--
-- award_level = NULL means the row covers all award levels combined.
-- =============================================================================
CREATE TABLE institution_completion_totals (
    id                       BIGSERIAL PRIMARY KEY,
    unitid                   INTEGER,
    award_level              INTEGER,   -- → award_levels.code; NULL = all levels
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

-- =============================================================================
-- Stored degree programs (populated via CLI or future MCP write tools)
-- =============================================================================
CREATE TABLE degrees (
    id           BIGSERIAL PRIMARY KEY,
    degree_id    TEXT UNIQUE NOT NULL,
    unitid       INTEGER,   -- no FK; use LEFT JOIN institutions
    cip_code     TEXT,      -- no FK; use LEFT JOIN cip_codes
    catalog_year TEXT,
    yaml_content TEXT NOT NULL,
    created_at   TIMESTAMPTZ DEFAULT NOW()
);

-- =============================================================================
-- Indexes
-- =============================================================================

CREATE INDEX idx_institutions_state         ON institutions (state);
CREATE INDEX idx_institutions_carnegie      ON institutions (carnegie_class);
CREATE INDEX idx_institutions_control       ON institutions (control);
CREATE INDEX idx_institutions_hbcu          ON institutions (hbcu) WHERE hbcu = true;
CREATE INDEX idx_institutions_sector        ON institutions (sector);

CREATE INDEX idx_completions_unitid         ON completions (unitid);
CREATE INDEX idx_completions_cip_code       ON completions (cip_code);
CREATE INDEX idx_completions_year           ON completions (year);
CREATE INDEX idx_completions_award_level    ON completions (award_level);
CREATE INDEX idx_completions_major_num      ON completions (major_num);

CREATE INDEX idx_inst_totals_unitid         ON institution_completion_totals (unitid);
CREATE INDEX idx_inst_totals_year           ON institution_completion_totals (year);

CREATE INDEX idx_degrees_unitid             ON degrees (unitid);
CREATE INDEX idx_degrees_cip_code          ON degrees (cip_code);

-- =============================================================================
-- Row-Level Security
-- Every database access requires a signed-in user (`nuanalytics db login`).
-- The anon key alone is treated as identification, not authorisation —
-- both reads and writes need `auth.role() = 'authenticated'`.
-- =============================================================================

-- Data tables
ALTER TABLE institutions                   ENABLE ROW LEVEL SECURITY;
ALTER TABLE cip_codes                      ENABLE ROW LEVEL SECURITY;
ALTER TABLE completions                    ENABLE ROW LEVEL SECURITY;
ALTER TABLE institution_completion_totals  ENABLE ROW LEVEL SECURITY;
ALTER TABLE degrees                        ENABLE ROW LEVEL SECURITY;

-- Lookup tables — small, static reference rows but still gated so the
-- database surface is uniformly auth-only.
ALTER TABLE award_levels         ENABLE ROW LEVEL SECURITY;
ALTER TABLE institution_control  ENABLE ROW LEVEL SECURITY;
ALTER TABLE institution_level    ENABLE ROW LEVEL SECURITY;
ALTER TABLE institution_sector   ENABLE ROW LEVEL SECURITY;
ALTER TABLE carnegie_class       ENABLE ROW LEVEL SECURITY;
ALTER TABLE institution_locale   ENABLE ROW LEVEL SECURITY;
ALTER TABLE institution_size     ENABLE ROW LEVEL SECURITY;

-- Authenticated users can read every table.
CREATE POLICY "auth read institutions"                  ON institutions                  FOR SELECT USING (auth.role() = 'authenticated');
CREATE POLICY "auth read cip_codes"                     ON cip_codes                     FOR SELECT USING (auth.role() = 'authenticated');
CREATE POLICY "auth read completions"                   ON completions                   FOR SELECT USING (auth.role() = 'authenticated');
CREATE POLICY "auth read institution_completion_totals" ON institution_completion_totals  FOR SELECT USING (auth.role() = 'authenticated');
CREATE POLICY "auth read degrees"                       ON degrees                       FOR SELECT USING (auth.role() = 'authenticated');
CREATE POLICY "auth read award_levels"                  ON award_levels                  FOR SELECT USING (auth.role() = 'authenticated');
CREATE POLICY "auth read institution_control"           ON institution_control           FOR SELECT USING (auth.role() = 'authenticated');
CREATE POLICY "auth read institution_level"             ON institution_level             FOR SELECT USING (auth.role() = 'authenticated');
CREATE POLICY "auth read institution_sector"            ON institution_sector            FOR SELECT USING (auth.role() = 'authenticated');
CREATE POLICY "auth read carnegie_class"                ON carnegie_class                FOR SELECT USING (auth.role() = 'authenticated');
CREATE POLICY "auth read institution_locale"            ON institution_locale            FOR SELECT USING (auth.role() = 'authenticated');
CREATE POLICY "auth read institution_size"              ON institution_size              FOR SELECT USING (auth.role() = 'authenticated');

-- Authenticated users can insert, update, and delete on writable tables.
-- FOR ALL is required because ipeds-import uses ON CONFLICT DO UPDATE,
-- which needs UPDATE permission in addition to INSERT.
CREATE POLICY "auth write institutions"                  ON institutions                  FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
CREATE POLICY "auth write completions"                   ON completions                   FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
CREATE POLICY "auth write institution_completion_totals" ON institution_completion_totals  FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
CREATE POLICY "auth write degrees"                       ON degrees                       FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
