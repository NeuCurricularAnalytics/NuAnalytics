-- NuAnalytics Program Lookup Seed
-- Run after docs/database/programs-schema.sql, via:
--   nuanalytics db exec-sql docs/database/program-lookup-seed.sql
--
-- Seeds the degree_types lookup. Codes are the normalized degree_type values the
-- importer writes to programs.degree_type. Values observed in the corpus
-- (json_corrected + json_corrected_rebuilt): BS, BA, MS, Minor, Certificate,
-- BSE, BAS, AS, Micro-Credential. Idempotent (ON CONFLICT DO UPDATE).

INSERT INTO degree_types (code, label, level, is_degree) VALUES
  ('BS',    'Bachelor of Science',                'undergraduate', true),
  ('BA',    'Bachelor of Arts',                   'undergraduate', true),
  ('BSE',   'Bachelor of Science in Engineering', 'undergraduate', true),
  ('BAS',   'Bachelor of Applied Science',        'undergraduate', true),
  ('AS',    'Associate of Science',               'undergraduate', true),
  ('MS',    'Master of Science',                  'graduate',      true),
  ('MINOR', 'Minor',                              'undergraduate', false),
  ('CERT',  'Certificate',                        NULL,            false),
  ('MICRO', 'Micro-Credential',                   NULL,            false)
ON CONFLICT (code) DO UPDATE
  SET label = EXCLUDED.label, level = EXCLUDED.level, is_degree = EXCLUDED.is_degree;
