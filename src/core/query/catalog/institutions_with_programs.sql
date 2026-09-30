-- Institutions that hold stored degree programs, each with its programs.
--
-- Behind `db query schools --with-programs`. Driven from `programs`, because only a few
-- hundred of the ~6,500 institutions have any: filtering an institution search after the
-- fact would make a limit of 25 return a handful. The institution filters mirror the
-- ones `db query schools` applies; `WithProgramsParams::from` in institutions.rs
-- destructures the request, so a new filter cannot be added to one and not the other.
--
-- Returns one row: `schools_with_programs` (matching schools, before `limit`) and
-- `schools`, the first `limit` of them by unitid, each with its programs by program_key.
-- A program whose unitid matches no institution belongs to no school and is left out.
WITH arg AS (
    SELECT $1->>'name'                    AS name,
           $1->>'state'                   AS state,
           ($1->>'carnegie_class')::int   AS carnegie_class,
           ($1->>'control')::int          AS control,
           ($1->>'hbcu')::boolean         AS hbcu,
           ($1->>'tribal')::boolean       AS tribal,
           ($1->>'inst_size_min')::int    AS inst_size_min,
           ($1->>'limit')::int            AS lim
),
matched AS (
    SELECT i.unitid, i.name, i.city, i.state, i.carnegie_class, i.control, i.iclevel,
           i.hbcu, i.tribal, i.inst_size,
           count(*) AS program_count,
           jsonb_agg(
               jsonb_build_object(
                   'program_key',  p.program_key,
                   'name',         p.name,
                   'degree_type',  p.degree_type,
                   'catalog_year', p.catalog_year,
                   'program_kind', p.program_kind
               -- Byte order, not the database's collation: cloud and self-hosted
               -- deployments need not share a locale, and must answer the same.
               ) ORDER BY p.program_key COLLATE "C"
           ) AS programs
    FROM programs p
    JOIN institutions i ON i.unitid = p.unitid
    CROSS JOIN arg
    -- `*` is PostgREST's wildcard in `db query schools --name`; accept it here too.
    WHERE (arg.name IS NULL OR i.name ILIKE '%' || replace(arg.name, '*', '%') || '%')
      AND (arg.state IS NULL OR i.state = arg.state)
      AND (arg.carnegie_class IS NULL OR i.carnegie_class = arg.carnegie_class)
      AND (arg.control IS NULL OR i.control = arg.control)
      AND (arg.hbcu IS NULL OR i.hbcu = arg.hbcu)
      AND (arg.tribal IS NULL OR i.tribal = arg.tribal)
      AND (arg.inst_size_min IS NULL OR i.inst_size >= arg.inst_size_min)
    GROUP BY i.unitid
)
SELECT (SELECT count(*) FROM matched) AS schools_with_programs,
       coalesce(
           (SELECT jsonb_agg(school.* ORDER BY school.unitid)
              FROM (SELECT * FROM matched ORDER BY unitid
                    LIMIT (SELECT least(coalesce(lim, 25), 100) FROM arg)) AS school),
           '[]'::jsonb
       ) AS schools
