-- RLS Policy Patch
-- Migrate an existing database to the auth-required model:
--   every read AND every write needs `auth.role() = 'authenticated'`.
-- Safe to run on a live database — does not modify any rows.
--
-- Old policies (anon SELECT on the five data tables, no RLS on lookup
-- tables) are explicitly dropped so re-running is idempotent.

-- Enable RLS on every table (idempotent).
ALTER TABLE institutions                   ENABLE ROW LEVEL SECURITY;
ALTER TABLE cip_codes                      ENABLE ROW LEVEL SECURITY;
ALTER TABLE completions                    ENABLE ROW LEVEL SECURITY;
ALTER TABLE institution_completion_totals  ENABLE ROW LEVEL SECURITY;
ALTER TABLE degrees                        ENABLE ROW LEVEL SECURITY;
ALTER TABLE award_levels                   ENABLE ROW LEVEL SECURITY;
ALTER TABLE institution_control            ENABLE ROW LEVEL SECURITY;
ALTER TABLE institution_level              ENABLE ROW LEVEL SECURITY;
ALTER TABLE institution_sector             ENABLE ROW LEVEL SECURITY;
ALTER TABLE carnegie_class                 ENABLE ROW LEVEL SECURITY;
ALTER TABLE institution_locale             ENABLE ROW LEVEL SECURITY;
ALTER TABLE institution_size               ENABLE ROW LEVEL SECURITY;

-- Drop legacy policies from earlier schemas.
DROP POLICY IF EXISTS "auth insert institutions"                  ON institutions;
DROP POLICY IF EXISTS "auth insert completions"                   ON completions;
DROP POLICY IF EXISTS "auth insert institution_completion_totals" ON institution_completion_totals;
DROP POLICY IF EXISTS "auth upsert degrees"                       ON degrees;

DROP POLICY IF EXISTS "public read institutions"                  ON institutions;
DROP POLICY IF EXISTS "public read cip_codes"                     ON cip_codes;
DROP POLICY IF EXISTS "public read completions"                   ON completions;
DROP POLICY IF EXISTS "public read institution_completion_totals" ON institution_completion_totals;
DROP POLICY IF EXISTS "public read degrees"                       ON degrees;

DROP POLICY IF EXISTS "auth read institutions"                  ON institutions;
DROP POLICY IF EXISTS "auth read cip_codes"                     ON cip_codes;
DROP POLICY IF EXISTS "auth read completions"                   ON completions;
DROP POLICY IF EXISTS "auth read institution_completion_totals" ON institution_completion_totals;
DROP POLICY IF EXISTS "auth read degrees"                       ON degrees;
DROP POLICY IF EXISTS "auth read award_levels"                  ON award_levels;
DROP POLICY IF EXISTS "auth read institution_control"           ON institution_control;
DROP POLICY IF EXISTS "auth read institution_level"             ON institution_level;
DROP POLICY IF EXISTS "auth read institution_sector"            ON institution_sector;
DROP POLICY IF EXISTS "auth read carnegie_class"                ON carnegie_class;
DROP POLICY IF EXISTS "auth read institution_locale"            ON institution_locale;
DROP POLICY IF EXISTS "auth read institution_size"              ON institution_size;

-- Authenticated-read policies for every table.
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

-- Auth write policies — FOR ALL covers INSERT + UPDATE + DELETE so upserts work.
DROP POLICY IF EXISTS "auth write institutions"                  ON institutions;
DROP POLICY IF EXISTS "auth write completions"                   ON completions;
DROP POLICY IF EXISTS "auth write institution_completion_totals" ON institution_completion_totals;
DROP POLICY IF EXISTS "auth write degrees"                       ON degrees;

CREATE POLICY "auth write institutions"                  ON institutions                  FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
CREATE POLICY "auth write completions"                   ON completions                   FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
CREATE POLICY "auth write institution_completion_totals" ON institution_completion_totals  FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
CREATE POLICY "auth write degrees"                       ON degrees                       FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
