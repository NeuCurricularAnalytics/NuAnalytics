# IPEDS Data — What to Download and How to Import

IPEDS (the Integrated Postsecondary Education Data System) is the U.S. Department of
Education's database of colleges and universities. NuAnalytics imports two of its annual
survey files, which back the institution lookups and the completion demographics.

## What is imported

| Survey | File | Tables | Content |
|---|---|---|---|
| **HD** — institutional characteristics | `HD{year}.zip` | `institutions` | Name, location, control, level, size, locale, Carnegie classification, HBCU and tribal status |
| **C_A** — completions by award level | `C{year}_A.zip` | `completions`, `institution_completion_totals` | Every completion row, and per-institution totals, in one pass |

The two tables behave differently across years:

- **`institutions` has no year.** It holds one row per institution, and the last HD file
  imported wins. Import years oldest first; importing an HD file older than what is stored
  is refused unless you pass `--force`, because it would overwrite current values with
  stale ones.
- **`completions` is keyed by year.** Each year's rows sit beside the others, so years can
  be imported in any order and re-imported without touching another year.

Every completions row is stored: all CIP codes, both major numbers, every award level —
about 300,000–314,000 rows per year. Nothing is filtered on import; choosing computing
programs (CIP family `11`, plus `30.7001` Data Science and `30.7099` Multi/Interdisciplinary
Studies, Other) is a query-time decision. That table size is also why a `PGRST_DB_MAX_ROWS`
cap matters — see `db doctor`'s row-limit check.

`institution_completion_totals` holds each institution's totals per award level and year,
summed over every CIP code except 99 (IPEDS's grand-total row, which is the sum of the
others) and over both major numbers. The built-in demographics queries sum `completions`
directly; the table is for SQL (`query_sql`, `db query --sql`) that wants whole-school
totals. For first majors only, or a subset of CIP codes, sum `completions` instead.

The fall enrollment survey (EF) is not used. Each demographic baseline is the completions
of all graduates at the same institutions, which answers the question that matters here:
is the profile of computing graduates in proportion to the profile of all graduates?

## What is stored now

The four years 2022–2025, checked on 2026-10-01 against the files from NCES: every year's
completions row count and all 21 count columns, summed over every row, equal the file.
2022 and 2023 hold the revised releases (see below). Every `institution_completion_totals`
row equals its detail rows on all 21 columns. `institutions` holds the 2025 directory.

## Where to download

From the IPEDS Data Center, <https://nces.ed.gov/ipeds/use-the-data> (every file:
<https://nces.ed.gov/ipeds/datacenter/DataFiles.aspx>). Past years download directly:

```sh
mkdir -p ~/ipeds && cd ~/ipeds
curl -L -O https://nces.ed.gov/ipeds/datacenter/data/HD2024.zip
curl -L -O https://nces.ed.gov/ipeds/datacenter/data/C2024_A.zip
```

**The newest release does not download this way.** `HD2025.zip` returned 404 to `curl`
even with browser headers and a referer, while `HD2024.zip` succeeded the same way. Save
the current year's files from the Data Center in a browser.

## Things the files do that the importer handles

- **Revised releases.** `C2022_A.zip` and `C2023_A.zip` each contain the provisional file
  and a revised one (`c2022_a_rv.csv`, `C2023_a_RV.csv`), which corrects totals and adds
  and retracts rows. The importer reads the revised file whenever an archive has one.
- **Padded headers.** The 2022 completions header ends `CNRALW` followed by two spaces.
  Headers are trimmed before they are matched; before that fix, every 2022 row was stored
  with no count of nonresident-alien women (re-imported 2026-10-01).
- **Inconsistent case.** `hd2022.csv` and `hd2025.csv` are lowercase, `HD2023.csv` and
  `HD2024.csv` uppercase. `--dir` matches either.
- **Encoding.** Some years are CP1252, not UTF-8 (`HD2022` has an `é` as byte `0xE9`). The
  importer tries UTF-8, falls back to CP1252, and says which file needed it.
- **Carnegie vintages.** An HD file can carry several Carnegie classification columns
  (`C21BASIC`, `C18BASIC`, `C15BASIC`, …). The importer takes the newest.

## Importing

Sign in first; importing writes to the database.

```sh
nuanalytics db login --email you@example.org      # or `db login` for OAuth
nuanalytics db status                             # ping: ✓ authenticated read succeeded
```

Then import a year, naming the files or a directory to search:

```sh
nuanalytics db ipeds-import --year 2024 \
  --institutions ~/ipeds/HD2024.zip \
  --completions  ~/ipeds/C2024_A.zip

nuanalytics db ipeds-import --year 2024 --dir ~/ipeds/     # finds HD2024.* and C2024_A.*
```

Either file can be imported alone. Re-importing a year upserts in place.

The output looks like this:

```
Importing institutions from /home/you/ipeds/HD2024.zip ...
  ✓ 6072 read, 6072 upserted, 0 skipped
Importing completions from /home/you/ipeds/C2024_A.zip ...
  (all CIP codes stored; query with CIP filter for CS vs all-programs)
  ✓ 307707 rows read, 307707 with a usable UNITID, 307707 upserted, 0 skipped
```

## Verifying an import

```sh
nuanalytics db validate ~/ipeds/C2024_A.zip --year 2024
nuanalytics db validate ~/ipeds/HD2025.zip  --year 2025
```

For completions it compares the row count for the year exactly and every column on a
sample of 150 institutions. For HD it compares every column of every institution and
checks which Carnegie vintage the stored values came from.

Two limits to know:

- **It parses the file with the importer's own code**, so it cannot catch a parsing
  defect — both sides share it. The padded 2022 header passed validation for exactly this
  reason. Comparing per-column sums over every row, computed from the CSV independently,
  is what found it.
- **HD only validates against the newest year.** `institutions` holds the latest
  directory, so validating an older HD file reports mismatches in names, locales and sizes
  that are expected, not faults.

## CIP code seed data

The `cip_codes` table maps the 2,173 six-digit codes of the CIP 2020 taxonomy to their
titles. It is one of the seed files `db bootstrap` applies when a backend is set up (see
[setup.md](setup.md)); the file is `docs/database/cip-seed.sql`, generated from the NCES
2010→2020 crosswalk (<https://nces.ed.gov/ipeds/cipcode/Files/Crosswalk2010to2020.csv>),
which includes the computing codes new in 2020 such as `11.0902` Cloud Computing and
`30.7001` Data Science, General. Nothing enforces it as a foreign key; completions with a
code missing from it still import.

## Data use

IPEDS data is free and public. Cite it in publications as the
[IPEDS Data Use Agreement](https://nces.ed.gov/ipeds/datacenter/InstitutionByName.aspx)
asks:

> U.S. Department of Education, National Center for Education Statistics, Integrated
> Postsecondary Education Data System (IPEDS), [Survey Component], [Year].
