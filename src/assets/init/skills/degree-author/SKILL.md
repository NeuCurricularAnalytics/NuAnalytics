---
name: degree-author
description: Build a NuAnalytics degree file from a program's catalog (a URL, PDF or pasted text) and get it to validate. Use when asked to create, write or model a degree, major, track or program for NuAnalytics, or to turn catalog requirements into a degree YAML.
allowed-tools: mcp__nuanalytics__get_reference mcp__nuanalytics__search_institutions mcp__nuanalytics__search_cip_codes mcp__nuanalytics__find_courses_matching mcp__nuanalytics__validate_degree mcp__nuanalytics__audit_degree mcp__nuanalytics__get_course_detail mcp__nuanalytics__list_sample_degrees
---

# Degree author

**Goal:** a degree file in `degrees/` that validates with 0 errors and matches the
catalog course for course.

**Where files go:** `degrees/<institution>-<program>.yaml`, e.g.
`degrees/csu-bs-computer-science.yaml`. Write YAML unless the user asks for unified JSON.

## Tools

- `get_reference(topic="degree-yaml")` is the format. Pass `section` for one part:
  `quickstart`, `degree`, `requirements`, `courses` or `examples`. Open it whenever
  you are unsure of a key.
- `search_institutions(name=…, state=…)` gives the `unitid`, and
  `search_cip_codes(query=…)` gives the `cip_code`. Both need the database. If they say
  it is unavailable, leave those fields out and tell the user.
- `find_courses_matching(path=…, patterns="CS:300+")` previews the courses a pattern
  pool will hold, before you commit to the pattern.
- `validate_degree(path=…)` lists errors and warnings. These are findings about the
  file, not a failed call.
- `audit_degree(path=…)` looks past validity: upper-level courses without
  prerequisites, and deep prerequisite chains.
- `list_sample_degrees` lists three complete real degrees (`degree="sample:csu"`, …)
  to compare structure against.

## Steps

1. Collect every catalog page before writing: the degree page, each track, gen-ed, the
   course descriptions and the sample plan. `catalog-patterns.md` says what to look
   for.
2. Write `degree:`, then `requirements:`, then `courses:`.
3. Run `validate_degree`, fix what it reports, and repeat until there are 0 errors.
4. Run `audit_degree`. Fix transcription errors and report anything else as a finding.
5. Report the credit arithmetic as a sum, with any difference from `total_credits`.

## What you would otherwise get wrong

- **Alternative sequences** ("(MATH 151 and 152) or MATH 155") go in an `all` list as
  `"{[MATH151, MATH152], MATH155}"`, or as a `one_of` with one option per sequence. The
  braced form works only in an `all` list; in a `select` pool, write the `one_of`.
- **Key each course by its prefix and full number**, as in `CS3500`. Two traps:
  - A key of 2–4 letters followed by a number below 100 (`CSE12`), or any key
    starting with `ELEC`, is read as an elective placeholder. It is left out of
    stored per-course metrics.
  - Patterns match on the key. So if you work around the first trap with `CSE_12`,
    list those courses in `from.courses`.
- **University pools** such as "6 credits of humanities" are a `select` over a pattern
  (`"HUM:100+"`) that matches nothing listed. Validate with
  `allow_unmatched_patterns=true` so the error becomes a warning.
- **`exclude` removes pattern matches only**, never a course listed in `from.courses`.
- **"Choose N of M" is never `all`.** If the credits don't add up, a choice was flattened.
- **Free electives** are a `select` with `fills_to_total: true`, so each plan sizes
  them to reach `total_credits`.

## Done when

- `validate_degree` reports 0 errors. Use `allow_unmatched_patterns=true` only for
  pools you have said are not enumerated.
- Every course and requirement traces to a named catalog page.
- The credits reconcile to `total_credits`, or the gap is stated and explained.

## References

- `catalog-patterns.md`: open it before mapping requirements, and whenever a catalog
  phrase doesn't obviously fit a requirement type.
- `example.yaml`: a small degree that validates, with every construct above. Open it to
  see one written out.
