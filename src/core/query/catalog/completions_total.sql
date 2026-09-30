-- Completion demographics aggregated over every matched institution:
-- `db query demographics --group-by total`.
--
-- CIP 99 is IPEDS's grand-total row: the sum of every other CIP at that institution,
-- award level and year. It is never summed here — counted beside the detail rows it
-- counts every graduate twice — and every baseline is summed from the detail rows.
--
-- Returns one row: `year`, `institutions_matched`, `counts` (the 21 demographic sums
-- over the selected completions) and `baseline` (the same sums over every CIP at the
-- matched institutions — their whole graduating population — or null unless asked for).
-- The baseline keeps the award-level filter and drops the CIP and major filters.
WITH arg AS (
    SELECT ($1->>'unitid')::int            AS unitid,
           ($1->>'carnegie_class')::int    AS carnegie_class,
           ($1->>'control')::int           AS control,
           $1->>'state'                    AS state,
           ($1->>'hbcu')::boolean          AS hbcu,
           ($1->>'tribal')::boolean        AS tribal,
           ($1->>'inst_size_min')::int     AS inst_size_min,
           $1->>'cip_prefix'               AS cip_prefix,
           CASE WHEN jsonb_typeof($1->'cip_codes') = 'array'
                THEN ARRAY(SELECT jsonb_array_elements_text($1->'cip_codes'))
           END                             AS cip_codes,
           ($1->>'award_level')::int       AS award_level,
           ($1->>'major_num')::int         AS major_num,
           ($1->>'year')::int              AS year,
           ($1->>'with_baseline')::boolean AS with_baseline
),
matched AS (
    SELECT i.unitid, i.name, i.city, i.state, i.carnegie_class
    FROM institutions i
    CROSS JOIN arg
    WHERE (arg.unitid IS NULL OR i.unitid = arg.unitid)
      AND (arg.carnegie_class IS NULL OR i.carnegie_class = arg.carnegie_class)
      AND (arg.control IS NULL OR i.control = arg.control)
      AND (arg.state IS NULL OR i.state = arg.state)
      AND (arg.hbcu IS NULL OR i.hbcu = arg.hbcu)
      AND (arg.tribal IS NULL OR i.tribal = arg.tribal)
      AND (arg.inst_size_min IS NULL OR i.inst_size >= arg.inst_size_min)
),
-- Not materialised: `yr` wants the maximum over every year, `counts` one year of it.
selected AS NOT MATERIALIZED (
    SELECT c.*
    FROM completions c
    JOIN matched USING (unitid)
    CROSS JOIN arg
    WHERE TRUE
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
counts AS (
    SELECT
        coalesce(sum(s.total), 0) AS total,
        coalesce(sum(s.total_men), 0) AS total_men,
        coalesce(sum(s.total_women), 0) AS total_women,
        coalesce(sum(s.nonresident_alien_men), 0) AS nonresident_alien_men,
        coalesce(sum(s.nonresident_alien_women), 0) AS nonresident_alien_women,
        coalesce(sum(s.hispanic_men), 0) AS hispanic_men,
        coalesce(sum(s.hispanic_women), 0) AS hispanic_women,
        coalesce(sum(s.american_indian_men), 0) AS american_indian_men,
        coalesce(sum(s.american_indian_women), 0) AS american_indian_women,
        coalesce(sum(s.asian_men), 0) AS asian_men,
        coalesce(sum(s.asian_women), 0) AS asian_women,
        coalesce(sum(s.black_men), 0) AS black_men,
        coalesce(sum(s.black_women), 0) AS black_women,
        coalesce(sum(s.native_hawaiian_men), 0) AS native_hawaiian_men,
        coalesce(sum(s.native_hawaiian_women), 0) AS native_hawaiian_women,
        coalesce(sum(s.white_men), 0) AS white_men,
        coalesce(sum(s.white_women), 0) AS white_women,
        coalesce(sum(s.two_or_more_men), 0) AS two_or_more_men,
        coalesce(sum(s.two_or_more_women), 0) AS two_or_more_women,
        coalesce(sum(s.unknown_race_men), 0) AS unknown_race_men,
        coalesce(sum(s.unknown_race_women), 0) AS unknown_race_women
    FROM selected s
    JOIN yr ON s.year = yr.year
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
    JOIN matched USING (unitid)
    CROSS JOIN arg
    JOIN yr ON c.year = yr.year
    WHERE c.cip_code <> '99'
      AND (arg.award_level IS NULL OR c.award_level = arg.award_level)
)
SELECT (SELECT year FROM yr)                     AS year,
       (SELECT count(*) FROM matched)            AS institutions_matched,
       (SELECT to_jsonb(counts.*) FROM counts)   AS counts,
       CASE WHEN arg.with_baseline
            THEN (SELECT to_jsonb(baseline.*) FROM baseline)
       END                                       AS baseline
FROM arg
