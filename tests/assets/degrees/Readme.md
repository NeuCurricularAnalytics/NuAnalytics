# Test degree fixtures

Unified-degree JSON compiled into the integration tests with `include_str!`. These are
real, catalog-sourced degree builds, not synthetic data.

> [!WARNING]
> These files are `include_str!`d at compile time by `tests/rs/degree_fixtures.rs`.
> Removing or renaming one breaks the build of the `integration` test target, not just a
> test.

## Provenance

Copied verbatim from the `WebScrappedCombinedDataMetrics` corpus, `full_degree/degree/`
(AI-landscape scrape of 2026-06-01, rebuilt from live institution catalogs and validated
with `nuanalytics degree validate`; 1,088 degrees / 617 institutions). Filenames are the
upstream slugs, so each file is traceable back to that corpus and to its `degree.id`.

The tests previously read these from `/tmp/first_sem_unified/` and
`/tmp/asu_unified.json/` via absolute `include_str!` paths. Those fixtures were never
committed and `/tmp` has since been cleared, so the test target stopped compiling. The
files below were re-identified from the corpus by matching `degree.institution`,
`degree.name`, and the target course code each test probes.

| Test label | Institution | Degree | Target course(s) |
|---|---|---|---|
| Tulane | Tulane University of Louisiana | BS Computer Science | `CMPS2200`, `CMPS3340` |
| CoC | College of Charleston | BS Computer Science | `CSCI218`, `CSCI495` |
| Bowdoin | Bowdoin College | Computer Science Major (AB) | `CSCI2101`, `CSCI3465` |
| NMSU | New Mexico State University (main campus) | Computer Science - BS | `CSCI2220`, `CSCI4270` |
| Liberty | Liberty University | BS Computer Science - General (Resident) | `CSIS316`, `CSCN354` |
| RIC | Rhode Island College | BS Artificial Intelligence | `DATA245`, `CSCI446` |
| CalStateLA | California State University, Los Angeles | BS Computer Science | `CS4963` |
| Metro | Metropolitan State University of Denver | Computer Science Major, B.S. | `CS4050` |
| WKU | Western Kentucky University | BS Computer Science | `STAT402` |
| TxState | Texas State University | BS Computer Science | `CS4398` |
| ASU | Arizona State University | BS Computer Science | `MAT266`, `DAT402` |
| Syracuse | Syracuse University | BS Computer Science | `MAT397` |
| Bellevue | Bellevue College | Software Development BAS, AI Concentration | `AI240` |

Two mappings are worth flagging, because the old `/tmp` filenames disagreed with the
data they held:

- **Metro** — the old fixture was named `Metropolitan_State_University_...`, but
  `CS4050` (Algorithms and Algorithm Analysis) is an **MSU Denver** course. Metropolitan
  State University (Minnesota) numbers its computing courses `ICS*` and has no `CS4050`,
  so the Denver build is the only file the test can mean.
- **WKU** — the old fixture was named `..._certificate_webpage__...` but its degree name
  was `Bachelor of Science in Computer Science`; the BS build is used, not the
  AI-and-analytics certificate.

**University of Alaska Anchorage is absent.** The dropped `tc_uaa_matha252f__*` cases
wanted `University_of_Alaska_Anchorage_..._Bachelor_of_Science_in_Computer_Science`.
No such build exists anywhere in the corpus (`full_degree/`, `trimmed_degree/`,
`json_corrected_old/`, `validated/`) and `MATHA252F` appears in no file, so there was
nothing real to vendor.

## Refreshing a fixture

Recopy from the corpus and keep the slug name:

    cp ../WebScrappedCombinedDataMetrics/full_degree/degree/<slug>.unified.json \
       tests/assets/degrees/

If the upstream build changed, `earliest_term_matches_recorded_baseline` in
`tests/rs/target_course_population.rs` will fail and name every case that moved. Re-record
those baselines from the new run and say why in the commit -- do not assume the movement
is benign, since the baseline is the only regression signal these cases have.

## The original hand-checked expectations

The cases were first written as `println!`-only probes carrying hand-checked `expected`
term numbers, recorded in July 2026 against the pre-rebuild degree snapshots in
`/tmp/first_sem_unified/`. Those numbers are **not** the baselines the tests assert today
— the vendored builds are fully-inclusive re-authorings of the same degrees (gen-ed
modelled, prerequisites deep-traced), so term placements legitimately moved.

They are kept here because they are the only independent oracle these cases ever had.
The current values are not repeated here -- `CASES` in
`tests/rs/target_course_population.rs` is the single source of truth for those, and
duplicating them would drift. The final column records how each compared at the time the
fixtures were vendored.

`=` agreed · `~` agreed with one of two conflicting group values · `x` differed

| Institution | Course | Original expectation(s) | |
|---|---|---|---|
| Tulane | `CMPS2200` | reasonable_true=3 | = |
| Tulane | `CMPS3340` | reasonable_true=2 / reasonable_false=2 | = |
| CoC | `CSCI218` | reasonable_true=2 | x |
| CoC | `CSCI495` | reasonable_true=4 | x |
| Bowdoin | `CSCI2101` | reasonable_true=3 | x |
| Bowdoin | `CSCI3465` | reasonable_true=5 | x |
| NMSU | `CSCI2220` | reasonable_true=3 | = |
| NMSU | `CSCI4270` | reasonable_true=6 / reasonable_false=6 | = |
| Liberty | `CSIS316` | reasonable_true=2 | = |
| Liberty | `CSCN354` | reasonable_true=2 | x |
| RIC | `DATA245` | reasonable_true=2 | x |
| RIC | `CSCI446` | reasonable_true=5 / reasonable_false=4 | ~ |
| CalStateLA | `CS4963` | reasonable_true=10 / reasonable_false=8 | x |
| Metro | `CS4050` | reasonable_false=6 | x |
| WKU | `STAT402` | reasonable_false=4 | x |
| TxState | `CS4398` | reasonable_false=7 | = |
| ASU | `MAT266` | calc_ready=2 / not_calc_ready=2 | = |
| Syracuse | `MAT397` | calc_ready=3 / not_calc_ready=4 | ~ |
| Bellevue | `AI240` | not_calc_ready=4 | x |

The `reasonable_true` / `reasonable_false` / `calc_ready` / `not_calc_ready` grouping was
dropped rather than carried forward. No parameter of `analyze::execute_json` corresponds
to it, the generating commit (`f2a5f30`) recorded no definition for it, and the paired
rows above issued byte-identical calls while carrying different expectations — so at most
one row of each conflicting pair could ever have held. `ASU DAT402` (baseline 7) is a
20th case, added from `asu_target_course.rs`, which used a separate `/tmp` fixture.
