-- NuAnalytics Lookup Tables Seed
-- Run in Supabase SQL Editor after schema.sql.
-- Verified against: HD2024.csv (HD survey), C2024_A.csv (Completions survey)
-- All code values confirmed from actual IPEDS data and official IPEDS data dictionaries.

-- =============================================================================
-- Award levels (AWLEVEL column in IPEDS C survey)
-- Confirmed from C2024_A data dictionary strings and actual C2024_A data.
-- Codes 9-16 were deprecated after Fall 2010-11; modern data uses 2-8, 17-21.
-- Codes 20-21 split the old "< 1 academic year" category into 12-week buckets.
-- =============================================================================
INSERT INTO award_levels (code, label, is_degree) VALUES
  (2,  'Postsecondary award, certificate, or diploma of at least 1 but less than 2 academic years', false),
  (3,  'Associate''s degree',                                                                        true),
  (4,  'Postsecondary award, certificate, or diploma of at least 2 but less than 4 academic years', false),
  (5,  'Bachelor''s degree',                                                                         true),
  (6,  'Post-baccalaureate certificate',                                                             false),
  (7,  'Master''s degree',                                                                           true),
  (8,  'Post-master''s certificate',                                                                 false),
  (17, 'Doctor''s degree – research/scholarship',                                                    true),
  (18, 'Doctor''s degree – professional practice',                                                   true),
  (19, 'Doctor''s degree – other',                                                                   true),
  (20, 'Postsecondary award, certificate, or diploma of less than 12 weeks',                        false),
  (21, 'Postsecondary award, certificate, or diploma of at least 12 weeks but less than 1 year',   false)
ON CONFLICT (code) DO UPDATE
  SET label = EXCLUDED.label, is_degree = EXCLUDED.is_degree;

-- =============================================================================
-- Control type (CONTROL column in IPEDS HD survey)
-- Confirmed from HD2024 data: values 1, 2, 3, -3.
-- =============================================================================
INSERT INTO institution_control (code, label) VALUES
  (1,  'Public'),
  (2,  'Private not-for-profit'),
  (3,  'Private for-profit'),
  (-3, 'Not available')
ON CONFLICT (code) DO UPDATE SET label = EXCLUDED.label;

-- =============================================================================
-- Institution level (ICLEVEL column in IPEDS HD survey)
-- Confirmed from HD2024 data: values 1, 2, 3, -3.
-- =============================================================================
INSERT INTO institution_level (code, label) VALUES
  (1,  '4-year or above'),
  (2,  '2-year'),
  (3,  'Less than 2-year'),
  (-3, 'Not available')
ON CONFLICT (code) DO UPDATE SET label = EXCLUDED.label;

-- =============================================================================
-- Sector (SECTOR column in IPEDS HD survey)
-- Cross-classification of control × level.
-- Confirmed from HD2024 data: values 0-9, 99.
-- =============================================================================
INSERT INTO institution_sector (code, label) VALUES
  (0,  'Administrative unit'),
  (1,  'Public, 4-year or above'),
  (2,  'Private not-for-profit, 4-year or above'),
  (3,  'Private for-profit, 4-year or above'),
  (4,  'Public, 2-year'),
  (5,  'Private not-for-profit, 2-year'),
  (6,  'Private for-profit, 2-year'),
  (7,  'Public, less-than 2-year'),
  (8,  'Private not-for-profit, less-than 2-year'),
  (9,  'Private for-profit, less-than 2-year'),
  (99, 'Not classified')
ON CONFLICT (code) DO UPDATE SET label = EXCLUDED.label;

-- =============================================================================
-- Carnegie Classification 2021 Basic (C21BASIC column in IPEDS HD survey)
-- Codes 1-33 confirmed from actual HD2024 data and IPEDS HD2024 data dictionary.
-- This is the primary Carnegie classification used in 2024 IPEDS data.
--
-- research_level: convenient filter for queries like
--   WHERE carnegie_class IN (SELECT code FROM carnegie_class WHERE research_level = 'R1')
-- =============================================================================
INSERT INTO carnegie_class (code, label, research_level) VALUES
  -- Associate's Colleges (codes 1-9, 9 sub-types)
  (1,  'Associate''s Colleges: High Transfer-High Traditional',                                         'associates'),
  (2,  'Associate''s Colleges: High Transfer-Mixed Traditional/Nontraditional',                         'associates'),
  (3,  'Associate''s Colleges: High Transfer-High Nontraditional',                                      'associates'),
  (4,  'Associate''s Colleges: Mixed Transfer/Career & Technical-High Traditional',                     'associates'),
  (5,  'Associate''s Colleges: Mixed Transfer/Career & Technical-Mixed Traditional/Nontraditional',     'associates'),
  (6,  'Associate''s Colleges: Mixed Transfer/Career & Technical-High Nontraditional',                  'associates'),
  (7,  'Associate''s Colleges: High Career & Technical-High Traditional',                               'associates'),
  (8,  'Associate''s Colleges: High Career & Technical-Mixed Traditional/Nontraditional',               'associates'),
  (9,  'Associate''s Colleges: High Career & Technical-High Nontraditional',                            'associates'),
  -- Special Focus Two-Year (codes 10-13)
  (10, 'Special Focus Two-Year: Health Professions',                                                    'special-2yr'),
  (11, 'Special Focus Two-Year: Technical Professions',                                                 'special-2yr'),
  (12, 'Special Focus Two-Year: Arts & Design',                                                         'special-2yr'),
  (13, 'Special Focus Two-Year: Other Fields',                                                          'special-2yr'),
  -- Mixed Baccalaureate/Associate's (code 14)
  (14, 'Baccalaureate/Associate''s Colleges: Mixed Baccalaureate/Associate''s',                        'baccalaureate'),
  -- Doctoral Universities (codes 15-17)
  (15, 'Doctoral Universities: Highest Research Activity',                                              'R1'),
  (16, 'Doctoral Universities: Higher Research Activity',                                               'R2'),
  (17, 'Doctoral/Professional Universities',                                                            'doctoral'),
  -- Master's Colleges & Universities (codes 18-20)
  (18, 'Master''s Colleges & Universities: Larger Programs',                                            'masters'),
  (19, 'Master''s Colleges & Universities: Medium Programs',                                            'masters'),
  (20, 'Master''s Colleges & Universities: Small Programs',                                             'masters'),
  -- Baccalaureate Colleges (codes 21-22)
  (21, 'Baccalaureate Colleges: Arts & Sciences Focus',                                                 'baccalaureate'),
  (22, 'Baccalaureate Colleges: Diverse Fields',                                                        'baccalaureate'),
  -- Baccalaureate/Associate's (code 23)
  (23, 'Baccalaureate/Associate''s Colleges: Associate''s Dominant',                                   'baccalaureate'),
  -- Special Focus Four-Year (codes 24-32, confirmed from HD2024 institutional samples)
  (24, 'Special Focus Four-Year: Faith-Related Institutions',                                           'special-4yr'),
  (25, 'Special Focus Four-Year: Medical Schools & Centers',                                            'special-4yr'),
  (26, 'Special Focus Four-Year: Other Health Professions Schools',                                     'special-4yr'),
  (27, 'Special Focus Four-Year: Research Institutions',                                                'special-4yr'),
  (28, 'Special Focus Four-Year: Engineering and Other Technology-Related Schools',                     'special-4yr'),
  (29, 'Special Focus Four-Year: Business & Management Schools',                                        'special-4yr'),
  (30, 'Special Focus Four-Year: Arts, Music & Design Schools',                                         'special-4yr'),
  (31, 'Special Focus Four-Year: Law Schools',                                                          'special-4yr'),
  (32, 'Special Focus Four-Year: Other Special Focus Institutions',                                     'special-4yr'),
  -- Tribal Colleges (code 33)
  (33, 'Tribal Colleges',                                                                               'tribal'),
  -- Not applicable (code -2)
  (-2, 'Not applicable (not in Carnegie universe)',                                                     null)
ON CONFLICT (code) DO UPDATE
  SET label = EXCLUDED.label, research_level = EXCLUDED.research_level;

-- =============================================================================
-- Locale — NCES Urban-Centric Locale codes (LOCALE column in IPEDS HD survey)
-- Confirmed correct from NCES locale documentation and HD2024 data.
-- -3 = Not available (confirmed from HD2024 data showing -3 for some records).
-- =============================================================================
INSERT INTO institution_locale (code, label, category) VALUES
  (11, 'City: Large',    'City'),
  (12, 'City: Midsize',  'City'),
  (13, 'City: Small',    'City'),
  (21, 'Suburb: Large',  'Suburb'),
  (22, 'Suburb: Midsize','Suburb'),
  (23, 'Suburb: Small',  'Suburb'),
  (31, 'Town: Fringe',   'Town'),
  (32, 'Town: Distant',  'Town'),
  (33, 'Town: Remote',   'Town'),
  (41, 'Rural: Fringe',  'Rural'),
  (42, 'Rural: Distant', 'Rural'),
  (43, 'Rural: Remote',  'Rural'),
  (-3, 'Not available',  null)
ON CONFLICT (code) DO UPDATE SET label = EXCLUDED.label, category = EXCLUDED.category;

-- =============================================================================
-- Institution size (INSTSIZE column in IPEDS HD survey)
-- Confirmed from HD2024 data: values -2, -1, 1-5.
-- =============================================================================
INSERT INTO institution_size (code, label) VALUES
  (-2, 'Not applicable'),
  (-1, 'Not reported'),
  (1,  'Under 1,000'),
  (2,  '1,000 - 4,999'),
  (3,  '5,000 - 9,999'),
  (4,  '10,000 - 19,999'),
  (5,  '20,000 and above')
ON CONFLICT (code) DO UPDATE SET label = EXCLUDED.label;
