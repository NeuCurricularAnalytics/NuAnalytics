-- RLS Policy Patch
-- Fixes the INSERT-only policies that block ON CONFLICT DO UPDATE (upsert).
-- Run once in the Supabase SQL Editor. Safe to run on an existing database
-- with data — does not drop or modify any tables or rows.

-- Remove old INSERT-only policies
DROP POLICY IF EXISTS "auth insert institutions"                  ON institutions;
DROP POLICY IF EXISTS "auth insert completions"                   ON completions;
DROP POLICY IF EXISTS "auth insert institution_completion_totals" ON institution_completion_totals;
DROP POLICY IF EXISTS "auth upsert degrees"                       ON degrees;

-- Ensure RLS is enabled on the cache table (may not exist in older setups)
ALTER TABLE IF EXISTS institution_completion_totals ENABLE ROW LEVEL SECURITY;

-- Add public read policy for the cache table if it doesn't exist
DROP POLICY IF EXISTS "public read institution_completion_totals" ON institution_completion_totals;
CREATE POLICY "public read institution_completion_totals"
    ON institution_completion_totals FOR SELECT USING (true);

-- Replace with ALL policies (INSERT + UPDATE + DELETE) so upserts work
-- correctly. The USING clause applies to rows already in the table (UPDATE/DELETE),
-- WITH CHECK applies to new/modified rows (INSERT/UPDATE).
CREATE POLICY "auth write institutions"                  ON institutions                  FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
CREATE POLICY "auth write completions"                   ON completions                   FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
CREATE POLICY "auth write institution_completion_totals" ON institution_completion_totals  FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
CREATE POLICY "auth write degrees"                       ON degrees                       FOR ALL USING (auth.role() = 'authenticated') WITH CHECK (auth.role() = 'authenticated');
