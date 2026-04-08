-- RLS Policy Patch
-- For databases set up before schema.sql included RLS policies.
-- Adds all missing read + write policies.
-- Safe to run on an existing database with data — does not modify any rows.

-- Enable RLS on all data tables (idempotent)
ALTER TABLE institutions                   ENABLE ROW LEVEL SECURITY;
ALTER TABLE cip_codes                      ENABLE ROW LEVEL SECURITY;
ALTER TABLE completions                    ENABLE ROW LEVEL SECURITY;
ALTER TABLE institution_completion_totals  ENABLE ROW LEVEL SECURITY;
ALTER TABLE degrees                        ENABLE ROW LEVEL SECURITY;

-- Drop any old INSERT-only policies that block ON CONFLICT DO UPDATE
DROP POLICY IF EXISTS "auth insert institutions"                  ON institutions;
DROP POLICY IF EXISTS "auth insert completions"                   ON completions;
DROP POLICY IF EXISTS "auth insert institution_completion_totals" ON institution_completion_totals;
DROP POLICY IF EXISTS "auth upsert degrees"                       ON degrees;

-- Public read policies (anon key, no login required)
DROP POLICY IF EXISTS "public read institutions"                  ON institutions;
DROP POLICY IF EXISTS "public read cip_codes"                     ON cip_codes;
DROP POLICY IF EXISTS "public read completions"                   ON completions;
DROP POLICY IF EXISTS "public read institution_completion_totals" ON institution_completion_totals;
DROP POLICY IF EXISTS "public read degrees"                       ON degrees;

CREATE POLICY "public read institutions"                  ON institutions                  FOR SELECT USING (true);
CREATE POLICY "public read cip_codes"                     ON cip_codes                     FOR SELECT USING (true);
CREATE POLICY "public read completions"                   ON completions                   FOR SELECT USING (true);
CREATE POLICY "public read institution_completion_totals" ON institution_completion_totals  FOR SELECT USING (true);
CREATE POLICY "public read degrees"                       ON degrees                       FOR SELECT USING (true);

-- Auth write policies — FOR ALL covers INSERT + UPDATE + DELETE so upserts work
DROP POLICY IF EXISTS "auth write institutions"                  ON institutions;
DROP POLICY IF EXISTS "auth write completions"                   ON completions;
DROP POLICY IF EXISTS "auth write institution_completion_totals" ON institution_completion_totals;
DROP POLICY IF EXISTS "auth write degrees"                       ON degrees;

CREATE POLICY "auth write institutions"                  ON institutions                  FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
CREATE POLICY "auth write completions"                   ON completions                   FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
CREATE POLICY "auth write institution_completion_totals" ON institution_completion_totals  FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
CREATE POLICY "auth write degrees"                       ON degrees                       FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
