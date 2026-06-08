-- NuAnalytics Programs Schema (normalized degree storage)
-- Run in Supabase SQL Editor or via: nuanalytics db exec-sql docs/database/programs-schema.sql
--
-- After running this file, also run:
--   docs/database/program-lookup-seed.sql   (degree_types lookup)
--
-- Adds the normalized, queryable storage for imported degree programs:
--   programs              one row per degree program (+ lossless `document` JSONB)
--   courses               shared per-institution course catalog
--   program_courses       M:N junction programs <-> courses (with per-program overrides)
--   program_requirements  the requirement tree, flattened by `req_path`
--   degree_types          small lookup for degree_type codes
--
-- Design notes:
--   * Rows link by NATURAL keys (program_key, (institution_ref, course_code)),
--     NOT surrogate-id FKs — matching the FK-free / LEFT JOIN convention used by
--     `degrees` and `completions`, so write order never matters.
--   * `programs.document` is the authoritative, lossless unified-JSON degree;
--     the normalized rows are a queryable projection. Reads reconstruct from
--     `document`, so a partially-written projection is never user-visible.
--   * Child rows carry a `generation` stamp; a re-import bumps the program's
--     `generation` last (the commit marker) and stale children
--     (generation < programs.generation) are filtered at read time / GC'd later.
--
-- Safe to run on a live database: every object uses IF NOT EXISTS and policies
-- are dropped before being (re)created, so the file is idempotent.

-- Trigram index support for course-name substring search ("all calculus courses").
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- =============================================================================
-- degree_types — lookup for normalized degree_type codes
-- Seeded from docs/database/program-lookup-seed.sql.
-- =============================================================================
CREATE TABLE IF NOT EXISTS degree_types (
    code      TEXT PRIMARY KEY,            -- 'BS','BA','BSE','BAS','AS','MS','MINOR','CERT','MICRO'
    label     TEXT NOT NULL,
    level     TEXT,                        -- 'undergraduate' | 'graduate'
    is_degree BOOLEAN NOT NULL DEFAULT true
);

-- =============================================================================
-- programs — one row per degree program
--
-- program_key is the deterministic idempotency key (ON CONFLICT target):
--   "id:<degree.id>"  |  "nat:<unitid>|<cip>|<catalog_year>|<degree_type>"
--   |  "fp:<sha256(institution_raw|name|degree_type|catalog_year|source_url)>"
--
-- No FK on unitid / cip_code / degree_type: LEFT JOIN institutions / cip_codes /
-- degree_types (IPEDS cross-survey coverage isn't guaranteed; degree_type may be
-- a value not yet seeded).
-- =============================================================================
CREATE TABLE IF NOT EXISTS programs (
    id                          BIGSERIAL PRIMARY KEY,
    program_key                 TEXT UNIQUE NOT NULL,

    -- identity / provenance
    degree_id                   TEXT,           -- Degree.id (may be null; non-unique here)
    unitid                      INTEGER,        -- resolved IPEDS unit id (no FK)
    institution_ref             TEXT NOT NULL,  -- unitid-as-text when resolved, else normalized slug
    institution_raw             TEXT,           -- original Degree.institution string
    cip_code                    TEXT,           -- no FK; LEFT JOIN cip_codes

    -- queryable type dimensions
    name                        TEXT NOT NULL,
    degree_type                 TEXT,           -- normalized -> degree_types.code (no FK)
    program_kind                TEXT,           -- major|minor|concentration|certificate|track|specialization|emphasis|micro
    discipline                  TEXT,           -- ai|cs|ds|cy|...
    system_type                 TEXT NOT NULL DEFAULT 'semester',
    tags                        TEXT[],         -- raw tag set (GIN-indexed)

    -- scalar projection of Degree
    catalog_year                TEXT,
    source_url                  TEXT,
    total_credits               INTEGER,
    upper_division_credits      INTEGER,
    in_major_credits            INTEGER,
    gpa_minimum                 REAL,
    gpa_major                   REAL,
    grade_minimum               TEXT,
    major_subjects              TEXT[],
    allow_double_counting       BOOLEAN,        -- GLOBAL DEFAULT ONLY; effective control is per-requirement

    -- authoritative document + control
    document                    JSONB NOT NULL, -- lossless unified-JSON degree (source of truth)
    document_hash               TEXT NOT NULL,  -- sha256 of canonical document (change detection)
    verified                    BOOLEAN NOT NULL DEFAULT false,  -- human-confirmed; gates overwrite
    institution_resolved        BOOLEAN NOT NULL DEFAULT false,
    has_impossible_requirements BOOLEAN NOT NULL DEFAULT false,
    generation                  BIGINT  NOT NULL DEFAULT 0,       -- re-import commit marker
    created_at                  TIMESTAMPTZ DEFAULT NOW(),
    updated_at                  TIMESTAMPTZ DEFAULT NOW()
);

-- =============================================================================
-- courses — shared, per-institution course catalog
--
-- institution_ref partitions the catalog (so courses dedup within an institution
-- even when its unitid is unresolved). Canonical attributes are best-effort
-- (last-write-wins); a program's divergent credits/name live on program_courses.
-- Courses are NOT generation-swept per program (they belong to many programs).
-- =============================================================================
CREATE TABLE IF NOT EXISTS courses (
    id                 BIGSERIAL PRIMARY KEY,
    institution_ref    TEXT NOT NULL,
    unitid             INTEGER,
    course_code        TEXT NOT NULL,      -- the DegreeProgram course-map key, e.g. "CMPSC121"
    prefix             TEXT,
    number             TEXT,
    name               TEXT,
    credit_hours       REAL,
    credit_min         INTEGER,
    credit_max         INTEGER,
    prerequisites      JSONB,              -- {and|or|leaf} tree
    prerequisites_raw  TEXT,
    gen_ed_attributes  TEXT[],
    cross_listed_as    TEXT[],
    tags               TEXT[],
    generation         BIGINT NOT NULL DEFAULT 0,
    created_at         TIMESTAMPTZ DEFAULT NOW(),
    updated_at         TIMESTAMPTZ DEFAULT NOW(),
    UNIQUE (institution_ref, course_code)
);

-- =============================================================================
-- program_courses — M:N junction programs <-> courses
-- Joins to courses on (institution_ref, course_code). Carries per-program
-- overrides for the divergent-attribute case. Generation-swept per program.
-- =============================================================================
CREATE TABLE IF NOT EXISTS program_courses (
    id                    BIGSERIAL PRIMARY KEY,
    program_key           TEXT NOT NULL,
    institution_ref       TEXT NOT NULL,
    course_code           TEXT NOT NULL,
    credit_hours_override REAL,
    name_as_listed        TEXT,
    generation            BIGINT NOT NULL DEFAULT 0,
    UNIQUE (program_key, course_code)
);

-- =============================================================================
-- program_requirements — the requirement tree, flattened
--
-- req_path is a deterministic address of the node within the requirement tree;
-- parent_path gives the one_of adjacency. e.g. a one_of `concentrations`, option
-- `thesis`, its 2nd nested requirement -> req_path='concentrations#thesis#1',
-- parent_path='concentrations', option_id='thesis'. Top-level map entry `core`
-- -> req_path='core', parent_path=NULL, map_key='core'.
--
-- The irreducible/recursive bits stay as JSONB: `courses` (type=all list),
-- `selection_spec` (the whole from-clause incl. patterns/groups/include/exclude;
-- wildcards like "CS:2500+"/"*:*" aren't enumerable), and `req_constraints`.
-- Generation-swept per program.
-- =============================================================================
CREATE TABLE IF NOT EXISTS program_requirements (
    id                 BIGSERIAL PRIMARY KEY,
    program_key        TEXT NOT NULL,
    req_path           TEXT NOT NULL,
    parent_path        TEXT,               -- NULL for top-level; else parent req_path
    map_key            TEXT,               -- top-level requirements-map key (NULL for nested)
    option_id          TEXT,               -- RequirementOption.id when under a one_of option
    option_name        TEXT,
    name               TEXT,
    req_type           TEXT NOT NULL,      -- 'all' | 'select' | 'one_of'
    category           TEXT,               -- major | supporting | gen_ed | elective
    count              INTEGER,
    credits            INTEGER,
    credit_min         INTEGER,
    credit_max         INTEGER,
    tags               TEXT[],
    courses            JSONB,              -- Vec<String> for type=all
    selection_spec     JSONB,              -- the FromClause (courses/pattern/include/exclude/groups/...)
    req_constraints    JSONB,              -- RequirementConstraints
    is_impossible      BOOLEAN NOT NULL DEFAULT false,  -- count > resolvable pool (queryable, not dropped)
    allow_double_count BOOLEAN,            -- derived from constraints.exclude_used (+ program default)
    generation         BIGINT NOT NULL DEFAULT 0,
    UNIQUE (program_key, req_path)
);

-- =============================================================================
-- analysis_runs — one row per `degree analyze` run of a program
--
-- Metrics are NOT columns on programs because they vary by run parameters
-- (iterations / sampling / mean-vs-median) AND by whether the analyzed degree
-- was trimmed. A run links to its parent program (program_key) but records the
-- exact analyzed artifact: `analyzed_document_hash` (always) and
-- `analyzed_document` (only when it differs from the program's document, e.g. a
-- trimmed variant that is NOT itself a stored program). Different configurations
-- of the same program coexist as distinct rows; `run_key` makes re-imports
-- idempotent:
--   run_key = sha256(program_key | analyzed_document_hash | variant |
--                    variations_run | sample_type | calc_strategy |
--                    sampling_strategy | max_plans | full_run | sort(include))
-- =============================================================================
CREATE TABLE IF NOT EXISTS analysis_runs (
    id                     BIGSERIAL PRIMARY KEY,
    run_key                TEXT UNIQUE NOT NULL,
    program_key            TEXT NOT NULL,            -- parent program (no FK; LEFT JOIN programs)
    analyzed_document_hash TEXT NOT NULL,            -- hash of the exact degree analyzed (trimmed variant if trimmed)
    variant                TEXT NOT NULL DEFAULT 'full',  -- 'full' | 'trimmed' | other transform label
    trimmed                BOOLEAN NOT NULL DEFAULT false,
    analyzed_document      JSONB,                    -- the analyzed degree when it differs from the program's document; NULL for full runs

    -- run parameters (nullable: a report may not record all of them)
    variations_run         INTEGER,
    sample_type            TEXT,
    calc_strategy          TEXT,                     -- mean | median
    sampling_strategy      TEXT,                     -- sequential | shuffled | stratified
    max_plans              INTEGER,
    full_run               BOOLEAN,
    included_courses       TEXT[],

    -- degree-level aggregate metrics
    degree_metrics         JSONB,                    -- {complexity:{min..q3}, credits:{...}, delay:{...}}
    complexity_mean        REAL,                     -- promoted from degree_metrics for ranking/filtering
    delay_mean             REAL,
    credits_mean           REAL,

    generation             BIGINT NOT NULL DEFAULT 0,
    created_at             TIMESTAMPTZ DEFAULT NOW(),
    updated_at             TIMESTAMPTZ DEFAULT NOW()
);

-- =============================================================================
-- analysis_course_metrics — per run x course graph metrics
-- Promoted *_mean columns for cross-program ranking; full 7-stat breakdown in
-- `metrics` JSONB. Generation-swept per run.
-- =============================================================================
CREATE TABLE IF NOT EXISTS analysis_course_metrics (
    id              BIGSERIAL PRIMARY KEY,
    run_key         TEXT NOT NULL,
    program_key     TEXT NOT NULL,        -- denormalized for cross-run course queries
    course_code     TEXT NOT NULL,
    plan_count      INTEGER,              -- how many variations included this course
    complexity_mean REAL,
    centrality_mean REAL,
    delay_mean      REAL,
    blocking_mean   REAL,
    metrics         JSONB,                -- {complexity,centrality,delay,blocking} each {min..q3}
    generation      BIGINT NOT NULL DEFAULT 0,
    UNIQUE (run_key, course_code)
);

-- =============================================================================
-- analysis_plans — per run x selected exemplar plan (shortest/longest/samples)
-- Generation-swept per run.
-- =============================================================================
CREATE TABLE IF NOT EXISTS analysis_plans (
    id               BIGSERIAL PRIMARY KEY,
    run_key          TEXT NOT NULL,
    program_key      TEXT NOT NULL,
    plan_index       INTEGER NOT NULL,    -- position within the run's selected_plans
    category         TEXT,                -- 'Shortest Path' | 'Longest Path' | 'Random Sample' | ...
    terms_required   INTEGER,
    total_complexity REAL,
    longest_delay    REAL,
    credits          REAL,
    course_count     INTEGER,
    is_calc_ready    BOOLEAN,
    critical_path    JSONB,               -- [course_code, ...]
    schedule         JSONB,               -- [{term, courses:[...], credits}, ...]
    generation       BIGINT NOT NULL DEFAULT 0,
    UNIQUE (run_key, plan_index)
);

-- =============================================================================
-- Indexes
-- =============================================================================
CREATE INDEX IF NOT EXISTS idx_programs_unitid       ON programs (unitid);
CREATE INDEX IF NOT EXISTS idx_programs_cip_code     ON programs (cip_code);
CREATE INDEX IF NOT EXISTS idx_programs_degree_type  ON programs (degree_type);
CREATE INDEX IF NOT EXISTS idx_programs_program_kind ON programs (program_kind);
CREATE INDEX IF NOT EXISTS idx_programs_discipline   ON programs (discipline);
CREATE INDEX IF NOT EXISTS idx_programs_tags         ON programs USING GIN (tags);
CREATE INDEX IF NOT EXISTS idx_programs_verified     ON programs (verified) WHERE verified = true;
CREATE INDEX IF NOT EXISTS idx_programs_unresolved   ON programs (institution_resolved) WHERE institution_resolved = false;
CREATE INDEX IF NOT EXISTS idx_programs_impossible   ON programs (has_impossible_requirements) WHERE has_impossible_requirements = true;

CREATE INDEX IF NOT EXISTS idx_courses_prefix_number ON courses (prefix, number);
CREATE INDEX IF NOT EXISTS idx_courses_name_trgm     ON courses USING GIN (name gin_trgm_ops);
CREATE INDEX IF NOT EXISTS idx_courses_unitid        ON courses (unitid);
CREATE INDEX IF NOT EXISTS idx_courses_inst_ref      ON courses (institution_ref);

CREATE INDEX IF NOT EXISTS idx_program_courses_pk     ON program_courses (program_key);
CREATE INDEX IF NOT EXISTS idx_program_courses_lookup ON program_courses (institution_ref, course_code);

CREATE INDEX IF NOT EXISTS idx_program_reqs_pk       ON program_requirements (program_key);
CREATE INDEX IF NOT EXISTS idx_program_reqs_parent   ON program_requirements (program_key, parent_path);
CREATE INDEX IF NOT EXISTS idx_program_reqs_dblcount ON program_requirements (allow_double_count) WHERE allow_double_count = true;
CREATE INDEX IF NOT EXISTS idx_program_reqs_category ON program_requirements (category);

CREATE INDEX IF NOT EXISTS idx_analysis_runs_program    ON analysis_runs (program_key);
CREATE INDEX IF NOT EXISTS idx_analysis_runs_variant    ON analysis_runs (variant);
CREATE INDEX IF NOT EXISTS idx_analysis_runs_complexity ON analysis_runs (complexity_mean);

CREATE INDEX IF NOT EXISTS idx_acm_run            ON analysis_course_metrics (run_key);
CREATE INDEX IF NOT EXISTS idx_acm_program_course ON analysis_course_metrics (program_key, course_code);
CREATE INDEX IF NOT EXISTS idx_acm_complexity     ON analysis_course_metrics (complexity_mean);

CREATE INDEX IF NOT EXISTS idx_analysis_plans_run     ON analysis_plans (run_key);
CREATE INDEX IF NOT EXISTS idx_analysis_plans_program ON analysis_plans (program_key);
CREATE INDEX IF NOT EXISTS idx_analysis_plans_category ON analysis_plans (category);

-- =============================================================================
-- Row-Level Security
-- Mirrors the auth-required model: every read AND write needs
-- `auth.role() = 'authenticated'`. Idempotent: policies dropped before create.
-- degree_types is a static lookup (read-only for clients; seeded via exec-sql).
-- =============================================================================
ALTER TABLE programs                ENABLE ROW LEVEL SECURITY;
ALTER TABLE courses                 ENABLE ROW LEVEL SECURITY;
ALTER TABLE program_courses         ENABLE ROW LEVEL SECURITY;
ALTER TABLE program_requirements    ENABLE ROW LEVEL SECURITY;
ALTER TABLE degree_types            ENABLE ROW LEVEL SECURITY;
ALTER TABLE analysis_runs           ENABLE ROW LEVEL SECURITY;
ALTER TABLE analysis_course_metrics ENABLE ROW LEVEL SECURITY;
ALTER TABLE analysis_plans          ENABLE ROW LEVEL SECURITY;

-- programs
DROP POLICY IF EXISTS "auth read programs"  ON programs;
CREATE POLICY "auth read programs"  ON programs FOR SELECT USING (auth.role() = 'authenticated');
DROP POLICY IF EXISTS "auth write programs" ON programs;
CREATE POLICY "auth write programs" ON programs FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');

-- courses
DROP POLICY IF EXISTS "auth read courses"  ON courses;
CREATE POLICY "auth read courses"  ON courses FOR SELECT USING (auth.role() = 'authenticated');
DROP POLICY IF EXISTS "auth write courses" ON courses;
CREATE POLICY "auth write courses" ON courses FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');

-- program_courses
DROP POLICY IF EXISTS "auth read program_courses"  ON program_courses;
CREATE POLICY "auth read program_courses"  ON program_courses FOR SELECT USING (auth.role() = 'authenticated');
DROP POLICY IF EXISTS "auth write program_courses" ON program_courses;
CREATE POLICY "auth write program_courses" ON program_courses FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');

-- program_requirements
DROP POLICY IF EXISTS "auth read program_requirements"  ON program_requirements;
CREATE POLICY "auth read program_requirements"  ON program_requirements FOR SELECT USING (auth.role() = 'authenticated');
DROP POLICY IF EXISTS "auth write program_requirements" ON program_requirements;
CREATE POLICY "auth write program_requirements" ON program_requirements FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');

-- degree_types (read-only for clients)
DROP POLICY IF EXISTS "auth read degree_types" ON degree_types;
CREATE POLICY "auth read degree_types" ON degree_types FOR SELECT USING (auth.role() = 'authenticated');

-- analysis_runs
DROP POLICY IF EXISTS "auth read analysis_runs"  ON analysis_runs;
CREATE POLICY "auth read analysis_runs"  ON analysis_runs FOR SELECT USING (auth.role() = 'authenticated');
DROP POLICY IF EXISTS "auth write analysis_runs" ON analysis_runs;
CREATE POLICY "auth write analysis_runs" ON analysis_runs FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');

-- analysis_course_metrics
DROP POLICY IF EXISTS "auth read analysis_course_metrics"  ON analysis_course_metrics;
CREATE POLICY "auth read analysis_course_metrics"  ON analysis_course_metrics FOR SELECT USING (auth.role() = 'authenticated');
DROP POLICY IF EXISTS "auth write analysis_course_metrics" ON analysis_course_metrics;
CREATE POLICY "auth write analysis_course_metrics" ON analysis_course_metrics FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');

-- analysis_plans
DROP POLICY IF EXISTS "auth read analysis_plans"  ON analysis_plans;
CREATE POLICY "auth read analysis_plans"  ON analysis_plans FOR SELECT USING (auth.role() = 'authenticated');
DROP POLICY IF EXISTS "auth write analysis_plans" ON analysis_plans;
CREATE POLICY "auth write analysis_plans" ON analysis_plans FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
