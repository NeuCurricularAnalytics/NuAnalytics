# IPEDS Data — What to Download and How to Import

IPEDS (Integrated Postsecondary Education Data System) is the U.S. Department of
Education's primary database of college and university information. NuAnalytics imports
three annual survey files to support institutional and demographic analysis.

---

## What data we use

Only two IPEDS survey files are needed — the completions file (`C_A`) is used in a
single pass to populate two tables:

| Survey | Tables | Content |
|--------|--------|---------|
| **HD** — Institutional Characteristics Directory | `institutions` | Name, location, Carnegie classification, control type, HBCU status |
| **C** — Completions by Award Level | `completions` + `institution_completion_totals` | CS-filtered completions AND institution-wide totals for representation ratios |

The **Fall Enrollment (EF)** survey is not needed. Using completions-as-denominator
gives a more meaningful representation metric:

> *"Is the demographic profile of CS graduates proportional to the demographic
> profile of ALL graduates at this institution?"*

### CS filter (for `completions` table)

Computing-relevant CIP code families:
- `11.*` — Computer and Information Sciences and Support Services (CS, IS, cybersecurity, etc.)
- `30.7001` — Data Science (interdisciplinary)
- `30.7099` — Multi/Interdisciplinary Studies, Other (related computing programs)

### Institution totals (for `institution_completion_totals` table)

All CIP codes, all award levels, primary major only (MAJORNUM=1).
One aggregated row per institution per year.

---

## Current data availability

| Survey | Latest year | Notes |
|--------|------------|-------|
| **HD** (institutions) | **2024** | HD2024 available |
| **C** (completions) | **2024** | C2024_A available |

Both files are from the same year — no mixed-year import needed.

---

## Where to download

All files are available from the **IPEDS Data Center**:

**Main download page:**
> <https://nces.ed.gov/ipeds/use-the-data>

**Direct links — 2024:**

| File | Description | Direct link |
|------|-------------|------------|
| `HD2024.zip` | Institutional Characteristics | <https://nces.ed.gov/ipeds/datacenter/data/HD2024.zip> |
| `C2024_A.zip` | Completions by Award Level | <https://nces.ed.gov/ipeds/datacenter/data/C2024_A.zip> |

> **Note:** The NCES website sometimes requires accepting a data use agreement before
> downloading. If the direct links above do not work, navigate to the Data Center page
> and download from there.

**Browse all available files:**
> <https://nces.ed.gov/ipeds/datacenter/DataFiles.aspx>

---

## File contents

### HD — Institutional Characteristics

- **Rows**: ~6,500 (one per institution)
- **Key columns**: `UNITID`, `INSTNM`, `CITY`, `STABBR`, `CONTROL`, `ICLEVEL`, `C18BASIC` / `C21BASIC` (Carnegie), `HBCU`, `TRIBAL`
- **Note**: The Carnegie classification column name changes by survey cycle:
  `C15BASIC` (2015), `C18BASIC` (2018), `C21BASIC` (2021). NuAnalytics tries all
  variants automatically.

### C_A — Completions by Award Level

- **Rows**: ~200,000+ (one per institution × CIP code × award level × major number)
- **Key columns**: `UNITID`, `CIPCODE`, `AWLEVEL`, `CTOTALT`, `CTOTALM`, `CTOTALW`, plus one column per race/gender combination
- **Award levels**: 5 (bachelor's), 7 (master's), 9 (doctoral), others included
- **After filtering to CS CIP codes**: ~15,000–20,000 rows

**CIP code format in the file**: Dot notation — e.g. `11.0101`. NuAnalytics stores
them in the same format in the database.

### EF — Fall Enrollment

- **Rows**: ~50,000+ (multiple rows per institution for different student levels)
- **Key column for filtering**: `EFALEVEL = 1` selects the all-students aggregate row (avoids double-counting breakdowns)
- **Key columns**: `UNITID`, `EFTOTAT`, `EFTOTAM`, `EFTOTAW`, plus race/gender breakdowns

---

## Import workflow

### 1. Sign in (required for write access)

```sh
nuanalytics db login
```

Verify you have read-write access:
```sh
nuanalytics db status
# Auth: read-write  (signed in as you@northeastern.edu)
```

### 2. Create a directory and download the files

```sh
mkdir -p ~/ipeds
cd ~/ipeds

curl -L -O https://nces.ed.gov/ipeds/datacenter/data/HD2024.zip
curl -L -O https://nces.ed.gov/ipeds/datacenter/data/C2024_A.zip
```

Or download manually from the browser and save to `~/ipeds/`.

### 3. Import

```sh
nuanalytics db ipeds-import \
  --year 2024 \
  --institutions ~/ipeds/HD2024.zip \
  --completions  ~/ipeds/C2024_A.zip
```

Or use `--dir` for auto-detection (looks for `HD2024.*` and `C2024_A.*`):

```sh
nuanalytics db ipeds-import --year 2024 --dir ~/ipeds/
```

### 4. Expected output

```
Importing institutions from /home/.../HD2024.zip ...
  ✓ 6072 read, 6072 upserted, 0 skipped
Importing completions from /home/.../C2024_A.zip ...
  (CS completions → `completions` table; all-major totals → `institution_completion_totals`)
  ✓ 218462 rows read, 18741 matched CS CIP codes, 18741 upserted, 12 skipped
```

The completions import populates two tables in one pass — no second file needed.

---

## Re-importing (updating existing data)

All imports use upsert — re-running with newer data updates rows in place. The database
stores the year on each row so multiple years coexist cleanly.

---

## CIP code seed data

The `cip_codes` lookup table maps 6-digit CIP codes to human-readable titles. It must
be populated **before** importing completions data (the completions table has a foreign
key reference to it).

Run the SQL seed file in the Supabase **SQL Editor** (same place you ran the schema):

```
docs/database/cip-seed.sql
```

This contains all 2,173 6-digit CIP codes from the **CIP 2020 taxonomy** as a single
`INSERT ... ON CONFLICT DO UPDATE` statement. Paste it into the SQL Editor and run.

The file was generated from the NCES 2010→2020 crosswalk, which contains all current
2020 codes and titles including 12 new computing codes that didn't exist in 2010
(e.g. `11.0902` Cloud Computing, `11.0105` Human-Centered Technology Design,
`30.7001` Data Science, General):
> <https://nces.ed.gov/ipeds/cipcode/Files/Crosswalk2010to2020.csv>

---

## Data use agreement

IPEDS data is publicly available at no cost. By downloading, you agree to the
[IPEDS Data Use Agreement](https://nces.ed.gov/ipeds/datacenter/InstitutionByName.aspx),
which requires proper citation in any publications:

> U.S. Department of Education, National Center for Education Statistics, Integrated
> Postsecondary Education Data System (IPEDS), [Survey Component], [Year].
