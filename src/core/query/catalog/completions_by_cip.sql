-- Completion demographics per CIP code at one institution:
-- `db query demographics --group-by cip`.
--
-- CIP 99 is IPEDS's grand-total row: the sum of every other CIP at that institution,
-- award level and year. It is never summed here — counted beside the detail rows it
-- counts every graduate twice — and every baseline is summed from the detail rows.
--
-- Returns one row: `unitid`, `name`, `year`, `rows` (one per CIP code, award level and
-- major number, by CIP code in byte order), `baseline` (the school's completions over
-- every CIP, or null unless asked for) and — only when no row matched —
-- `nearby_cips_with_data`: the school's ten largest other CIPs in `nearby_year`, the
-- requested year or the school's latest, because filing a program under an unexpected
-- CIP is the commonest reason a query comes back empty.
WITH arg AS (
    SELECT ($1->>'unitid')::int            AS unitid,
           $1->>'cip_prefix'               AS cip_prefix,
           CASE WHEN jsonb_typeof($1->'cip_codes') = 'array'
                THEN ARRAY(SELECT jsonb_array_elements_text($1->'cip_codes'))
           END                             AS cip_codes,
           ($1->>'award_level')::int       AS award_level,
           ($1->>'major_num')::int         AS major_num,
           ($1->>'year')::int              AS year,
           ($1->>'with_baseline')::boolean AS with_baseline
),
selected AS NOT MATERIALIZED (
    SELECT c.*
    FROM completions c
    CROSS JOIN arg
    WHERE c.unitid = arg.unitid
      AND c.cip_code <> '99'
      AND (arg.cip_codes IS NULL OR c.cip_code = ANY (arg.cip_codes))
      AND (arg.cip_prefix IS NULL OR starts_with(c.cip_code, arg.cip_prefix))
      AND (arg.award_level IS NULL OR c.award_level = arg.award_level)
      AND (arg.major_num IS NULL OR c.major_num = arg.major_num)
),
-- The requested year, or the latest with matching data, so every figure below comes
-- from one reporting cycle rather than several added together.
yr AS (
    SELECT coalesce(arg.year, (SELECT max(year) FROM selected)) AS year FROM arg
),
picked AS (
    SELECT s.year, s.cip_code, cc.title AS cip_title, s.award_level, s.major_num,
           coalesce(s.total, 0) AS total,
           coalesce(s.total_men, 0) AS total_men,
           coalesce(s.total_women, 0) AS total_women,
           coalesce(s.nonresident_alien_men, 0) AS nonresident_alien_men,
           coalesce(s.nonresident_alien_women, 0) AS nonresident_alien_women,
           coalesce(s.hispanic_men, 0) AS hispanic_men,
           coalesce(s.hispanic_women, 0) AS hispanic_women,
           coalesce(s.american_indian_men, 0) AS american_indian_men,
           coalesce(s.american_indian_women, 0) AS american_indian_women,
           coalesce(s.asian_men, 0) AS asian_men,
           coalesce(s.asian_women, 0) AS asian_women,
           coalesce(s.black_men, 0) AS black_men,
           coalesce(s.black_women, 0) AS black_women,
           coalesce(s.native_hawaiian_men, 0) AS native_hawaiian_men,
           coalesce(s.native_hawaiian_women, 0) AS native_hawaiian_women,
           coalesce(s.white_men, 0) AS white_men,
           coalesce(s.white_women, 0) AS white_women,
           coalesce(s.two_or_more_men, 0) AS two_or_more_men,
           coalesce(s.two_or_more_women, 0) AS two_or_more_women,
           coalesce(s.unknown_race_men, 0) AS unknown_race_men,
           coalesce(s.unknown_race_women, 0) AS unknown_race_women
    FROM selected s
    JOIN yr ON s.year = yr.year
    LEFT JOIN cip_codes cc ON cc.cip_code = s.cip_code
    ORDER BY s.cip_code COLLATE "C", s.award_level, s.major_num
    LIMIT 2000
),
baseline AS (
    SELECT
        coalesce(sum(c.total), 0) AS total,
        coalesce(sum(c.total_men), 0) AS total_men,
        coalesce(sum(c.total_women), 0) AS total_women,
        coalesce(sum(c.nonresident_alien_men), 0) AS nonresident_alien_men,
        coalesce(sum(c.nonresident_alien_women), 0) AS nonresident_alien_women,
        coalesce(sum(c.hispanic_men), 0) AS hispanic_men,
        coalesce(sum(c.hispanic_women), 0) AS hispanic_women,
        coalesce(sum(c.american_indian_men), 0) AS american_indian_men,
        coalesce(sum(c.american_indian_women), 0) AS american_indian_women,
        coalesce(sum(c.asian_men), 0) AS asian_men,
        coalesce(sum(c.asian_women), 0) AS asian_women,
        coalesce(sum(c.black_men), 0) AS black_men,
        coalesce(sum(c.black_women), 0) AS black_women,
        coalesce(sum(c.native_hawaiian_men), 0) AS native_hawaiian_men,
        coalesce(sum(c.native_hawaiian_women), 0) AS native_hawaiian_women,
        coalesce(sum(c.white_men), 0) AS white_men,
        coalesce(sum(c.white_women), 0) AS white_women,
        coalesce(sum(c.two_or_more_men), 0) AS two_or_more_men,
        coalesce(sum(c.two_or_more_women), 0) AS two_or_more_women,
        coalesce(sum(c.unknown_race_men), 0) AS unknown_race_men,
        coalesce(sum(c.unknown_race_women), 0) AS unknown_race_women
    FROM completions c
    CROSS JOIN arg
    JOIN yr ON c.year = yr.year
    WHERE c.unitid = arg.unitid
      AND c.cip_code <> '99'
      AND (arg.award_level IS NULL OR c.award_level = arg.award_level)
),
nearby_year AS (
    SELECT coalesce(arg.year, (
               SELECT max(c.year) FROM completions c
               WHERE c.unitid = arg.unitid
                 AND c.cip_code <> '99'
                 AND (arg.award_level IS NULL OR c.award_level = arg.award_level)
                 AND (arg.major_num IS NULL OR c.major_num = arg.major_num)
           )) AS year
    FROM arg
),
nearby AS (
    SELECT c.cip_code, cc.title AS cip_title, sum(c.total) AS total_completions
    FROM completions c
    CROSS JOIN arg
    JOIN nearby_year ny ON c.year = ny.year
    LEFT JOIN cip_codes cc ON cc.cip_code = c.cip_code
    WHERE c.unitid = arg.unitid
      AND c.cip_code <> '99'
      AND (arg.award_level IS NULL OR c.award_level = arg.award_level)
      AND (arg.major_num IS NULL OR c.major_num = arg.major_num)
      -- Not the filter that came back empty: echoing it would suggest nothing.
      AND NOT coalesce(starts_with(c.cip_code, arg.cip_prefix), false)
      AND NOT coalesce(c.cip_code = ANY (arg.cip_codes), false)
    GROUP BY c.cip_code, cc.title
    HAVING sum(c.total) > 0
    ORDER BY total_completions DESC, c.cip_code COLLATE "C"
    LIMIT 10
)
SELECT arg.unitid,
       (SELECT i.name FROM institutions i WHERE i.unitid = arg.unitid) AS name,
       (SELECT year FROM yr)                                            AS year,
       coalesce(
           (SELECT jsonb_agg(p.* ORDER BY p.cip_code COLLATE "C", p.award_level, p.major_num)
              FROM picked p),
           '[]'::jsonb
       ) AS rows,
       CASE WHEN arg.with_baseline
            THEN (SELECT to_jsonb(baseline.*) FROM baseline)
       END AS baseline,
       (SELECT year FROM nearby_year)                                   AS nearby_year,
       CASE WHEN NOT EXISTS (SELECT 1 FROM picked)
            THEN coalesce(
                     (SELECT jsonb_agg(n.* ORDER BY n.total_completions DESC, n.cip_code COLLATE "C")
                        FROM nearby n),
                     '[]'::jsonb)
       END AS nearby_cips_with_data
FROM arg
