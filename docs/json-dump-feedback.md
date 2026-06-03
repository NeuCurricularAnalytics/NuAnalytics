# AI-Landscape JSON Dump — Review & Feedback

_Review of the `degree analyze` JSON output (`.debug/metrics/json-dump/`) produced
from the ai-landscape `cluster_outputs/` scrape, cross-checked against the
human-validated set (`validation_jsons_stripped/`). Date: 2026-06-03._

## What was reviewed

| | count |
|---|---|
| Converted unified programs (from `cluster_outputs/`, 643 pipeline files) | 1,033 |
| Reports successfully produced (scraped) | 930 |
| Scraped programs that **failed** (OOM / SIGKILL) | 35 |
| Validated programs converted + analyzed (from `validation_jsons_stripped/`) | 42 |
| **Reports now in `json-dump/` (scraped + validated merged)** | **972** |

The 42 validated reports were generated and merged into the dump (see
[§5](#5-validated-set-merged-into-the-dump)). They are **additions**, not
overwrites — see [§2.5](#25-scraped-vs-validated-naming-divergence).

---

## 1. NuAnalytics engine issues (these block the dump's usefulness)

These two are **our** bugs, not ai-landscape's. They are the reason the dump is
currently low-value, and they should be fixed before another full run.

### 1.1 OOM on large elective pools — *eager combination materialization*

**All 35 failures are `signal: 9 (SIGKILL)` = out-of-memory.** The cause is **not**
catalog size and **not** plan enumeration — it is that
`RequirementResolver::resolve_select_requirement` calls `generate_combinations(pool, count)`,
which **eagerly materializes every C(N, k) combination into a `Vec<Vec<String>>`**,
for *every* `select` requirement — including `category: "elective"` ones that are
later discarded from the plan space (see §1.2).

A program with one large "choose k of N" elective explodes regardless of how few
courses it has:

| Program | Courses | Largest `choose k of N` | C(N,k) | Result |
|---|---|---|---|---|
| Iowa State CS BS | **78** | choose 15 of 42 | 9.9×10¹⁰ | **OOM (>6 GB)** |
| Missouri-Columbia | 64 | choose 24 of 41 | 1.5×10¹¹ | OOM |
| UW-La Crosse | 64 | choose 12 of 51 | 1.6×10¹¹ | OOM |
| USF AI | 16,012 | choose 6 of 16,000 | 2.3×10²² | OOM |

**Proof it's the combination Vec, not the course graph:** Iowa State (78 courses)
peaks at **>6 GB and aborts**. Shrinking just that one requirement to "choose 2 of 42"
(C=861) drops peak memory to **22 MB** and it completes. The 78-course graph is
trivial; the 9.9×10¹⁰-element `Vec` is the hog.

`--jobs 8` (the default pool) then stacks ~8 of these allocations on a 15 GiB box,
so the OS OOM-killer fires and takes out multiple concurrent workers — including
small innocent ones running at the same time.

**Recommended fix (NuAnalytics):** never materialize the full combination set.
Either (a) sample `min(C(N,k), max_plans)` combinations lazily, or (b) cap the
materialized set and warn, or (c) skip materialization entirely for
non-enumerable categories (since electives are discarded anyway — see §1.2). Option
(c) alone would have prevented all 35 failures.

### 1.2 Every report has `variations_run: 1` — *degenerate, single-plan metrics*

**All 971 reports** (min = median = max = 1) analyze exactly **one** plan, so every
degree-level distribution is collapsed: `std_dev = 0`, and
`median = mean = min = max = q1 = q3`. The whole point of the metrics output
(variations, spread) is lost.

**Root cause:** `plan_generator.rs` enumerates only requirements whose category is in
`ENUMERABLE_CATEGORIES = ["major"]`. The converter labels **every** picklist/elective
`select` requirement `category: "elective"`, so none of a converted program's choice
points ever enter the plan space → exactly 1 plan.

**Proof:** flipping American University's picklist categories `elective → major` takes
it from `Estimated total plans: 1` to **74,256 estimated / 986 analyzed, complexity
std_dev 7.25** — a real distribution.

> Note the tension with §1.1: simply enabling enumeration of electives would
> immediately trigger the OOM in §1.1 for big pools. **Both must be fixed together** —
> enumerate elective choice points *and* sample combinations instead of materializing
> them.

**What is still meaningful today:** the **per-course** metrics (complexity, centrality,
blocking, delay) are derived from the prerequisite graph, not from plan sampling, so
they are valid even at `variations_run = 1`. Only the degree-level distributions are
degenerate. (E.g. CSU Fullerton: 26/31 courses carry non-zero graph metrics.)

---

## 2. ai-landscape data-quality issues

### 2.1 Picklist `[N]` is ambiguous: course-count vs. credits → 316 impossible requirements

`[N]` in a picklist tag is interpreted inconsistently in the source data:

- `"Calculus [1]"` over 2 courses → clearly **choose 1 course**.
- `"Artificial Intelligence Concentration [12]"` over **5** courses → **impossible** as
  a course count (can't choose 12 of 5); only makes sense as **12 credits** (≈ 4 courses).

Because the converter treats `[N]` as a course count, **316 `select` requirements across
225 of 1,033 files (20%)** are emitted with `count > pool_size`, which are unsatisfiable.

**Recommendation (ai-landscape):** make `[N]` unambiguous — either always "number of
courses" or always "number of credits", or tag it explicitly
(e.g. `[3 courses]` vs `[12 credits]`). This is the single highest-value data fix.
(NuAnalytics should also defensively clamp `count` to the pool size.)

### 2.2 Unparseable picklist tags (missing `[N]`) — 481+ occurrences

The most common conversion warning is **`Unparseable picklist tag`**: a picklist label
with no `[N]` annotation at all, so no choice count can be derived. Top offenders:

```
481×  "Computer Science Electives …"
136×  "Technical Electives …"
101×  "CSC 300-400 level …"
 85×  "Artificial Intelligence …"
 77×  "Upper Division …"
```

**Recommendation (ai-landscape):** every picklist tag should carry the `[N]` count, or
be omitted if it isn't actually a choice group.

### 2.3 Catalog-scrape outliers (whole-catalog ingestion)

A handful of programs captured far more courses than a degree could contain — the
scraper appears to have swallowed an entire course catalog:

```
16,012  University of South Florida — Artificial Intelligence (B.S.A.I.)
 2,046  Kean University — Artificial Intelligence
 1,991  University of Vermont — B.A. in Computer Science
 1,926  CUNY Brooklyn College — Computer Science, B.S.
 1,031  University of North Florida — Computer Science (BS)
```
(Median program course count is **29**.) These are almost certainly scraper errors and
should be flagged/re-scraped.

### 2.4 Missing `course_hours` → defaulted to 3 credits

Of 1,033 converted files, **452 carry conversion warnings (11,162 total)**; a large
share are `missing course_hours; assuming 3 credits`. The 3-credit assumption is
reasonable but silently skews credit totals. **Recommendation:** populate
`course_hours` (we can also backfill from the IPEDS/Supabase catalog before defaulting).

### 2.5 Scraped vs. validated naming divergence

The scraped and validated records for the *same* program use different degree-name
strings (scraped: `"Computer Science, B.S."`; validated:
`"Bachelor's of Science Computer Science"`), so their report filenames never collide.
23 of the 41 validated universities also appear in the scraped set, but as
differently-named files. **Recommendation:** converge on a canonical
`university` + `degree` + `degree_type` identity so records can be matched and de-duplicated.

---

## 3. What's working well

- **Conversion is robust:** all 1,033 programs converted without crashing; the cluster
  pipeline format (`course_verifier.<program>.results`) is correctly unwrapped.
- **Prerequisites** import losslessly into the tagged `{and|or}` AST.
- **Tags** (`ai`, `ai-major`/`ai-concentration`/`ai-minor`) are carried onto the degree
  and surface correctly in reports.
- **Per-course graph metrics** are sound and useful today (see §1.2).
- The validated set is clean and bounded (max branching 4.1×10⁴ vs the failed-set
  median 3.4×10⁸) — **human validation already fixes the elective explosion.**

---

## 4. Recommendations, prioritized

**NuAnalytics (we own these):**
1. **Stop materializing combinations** in `generate_combinations` (fixes all 35 OOMs). — §1.1
2. **Enumerate elective choice points**, not just `category == "major"` (fixes
   `variations_run = 1`). Must land with #1. — §1.2
3. Defensively **clamp `select.count` to the pool size** + warn. — §2.1

**ai-landscape (data):**
1. Disambiguate picklist **`[N]` (courses vs credits)**. — §2.1
2. Ensure every **picklist tag carries `[N]`**. — §2.2
3. Fix **whole-catalog scrapes** (USF, Kean, UVM, CUNY Brooklyn, UNF). — §2.3
4. Populate **`course_hours`**. — §2.4
5. Emit a **canonical program identity** for matching/dedup. — §2.5

---

## 5. Validated set merged into the dump

The 42 `validation_jsons_stripped/` programs were converted
(`degree convert … -o .debug/converted_validated/`) and analyzed
(`degree analyze … -j 4`, 0 failures) into the dump. Notably, the **scraped**
CSU-Fullerton CS BS was one of the 35 OOM failures, while its **validated** version
analyzes cleanly — a concrete win for using validated data.

The ai-landscape directories under `ai-landscape-tools/` were **not modified**.

## 6. Sample reports

Five reports were generated with the tool into `.debug/metrics/json-reports/`
(report JSON + HTML + plan CSVs), anchored on the requested CSU CS BS:

1. California State University, Fullerton — CS, B.S. _(the CSU csbs)_
2. Northeastern University — CS concentration
3. Florida International University — CS, B.S.
4. Drexel University — CS (AI concentration)
5. Georgia Institute of Technology — CS minor

All five are valid JSON with populated per-course metrics; all show
`variations_run: 1` per §1.2.
