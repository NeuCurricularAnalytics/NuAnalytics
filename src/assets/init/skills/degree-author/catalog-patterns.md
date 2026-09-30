# Catalog language → degree requirements

How to turn what a catalog says into requirements. For the syntax itself, call
`get_reference(topic="degree-yaml")`; this file covers the decisions and the traps.
`example.yaml` in this folder shows each construct in a file that validates.

## Reading the catalog

Requirements are usually split across pages. Collect all of them before writing:
the degree page (credit total, GPA), each concentration or track page, the college
and university requirements (gen-ed, writing), the course descriptions (credits,
prerequisites), and the sample four-year plan.

- Confirm the catalog year. Look for "Effective Fall 20XX", "formerly", "replaces".
- The sample plan uses current course numbers. When it disagrees with the
  requirement list, check the course catalog. Don't guess which one is right.

## Mapping phrases to requirement types

| Catalog says | Write |
|---|---|
| "Complete all of the following" | `type: all`, `courses: [...]` |
| "A, (B or C), D" | `type: all`, `courses: [A, "{B, C}", D]`: braces make a choice |
| "Lecture with its lab" | a bundle, `"[CHEM111, CHEM111L]"`, in any course list |
| "Choose 3 of the following" | `type: select`, `count: 3`, `from.courses` |
| "12 credits from SUBJ 300 or above" | `type: select`, `credits: 12`, `from.pattern: "SUBJ:300+"` |
| "…excluding SUBJ 390" | `from.exclude: [SUBJ390]`. It removes pattern matches only |
| "These courses, or any SUBJ 400-level" | `from.courses` plus `from.pattern` in one `from` |
| "One course from each of three areas" | `from.groups`, `per_group: 1` |
| "Courses from two of these three areas" | `from.groups`, `groups_required: 2`, `per_group: 1` |
| "Choose a concentration / track" | `type: one_of`, one option per track |
| "(A and B) or C" as a requirement | `type: one_of`, one option per sequence |
| "Free electives to reach 120" | `type: select` with `fills_to_total: true` |

**Alternative sequences are `one_of`.** The schema also shows a nested form,
`"{[A, B], [C]}"`, but the tools do not parse it: validation reports `[A` as a
missing course. Write one option per sequence instead.

**`exclude` never removes an explicitly listed course.** Only pattern matches are
excluded. To drop a course, leave it out of `from.courses`.

## Checking the mapping

Run these checks after every requirement you write. Most mistakes show up here.

- **Credits.** Six courses mapped as `all` at 3 credits is 18. If the catalog says
  "9 credits", you missed a "choose".
- **Course count.** If the catalog says "18 courses" and your `all` lists hold 25,
  a choice was flattened.
- **Hidden "or".** Re-read for parentheses, slashes and "or". Each one is a `{…}`
  choice or a `select`.
- **Nested "2 of 3".** If you flattened it, you'll see too many required courses.
  Use `from.groups` with `groups_required`.

Preview a pattern before using it:
`find_courses_matching(path="degrees/x.yaml", patterns="CS:300+", exclude="CS390")`.
It reports which listed courses the pool will hold.

## Courses

Every course named anywhere needs an entry under `courses:`. That includes courses
in requirements, groups and tracks, every prerequisite and corequisite (trace
chains to their roots), and every course in the sample plan.

- Copy the course number, title and credits from the course catalog. Never
  paraphrase a number: CS 100 and CS 1000 are different courses.
- Prerequisites are boolean expressions over course keys: `&`, `|`, parentheses,
  and a grade suffix. `"(CS210 | CS211) & MATH151[C]"` means "CS 210 or CS 211,
  and MATH 151 with a C or better". Patterns such as `CS:300+` are not allowed in
  prerequisites.
- Corequisites are bidirectional: the lab names the lecture and the lecture names
  the lab. If the lab's credits are included in the lecture's, give the lab
  `credits: 0`. Don't count them twice.
- Cross-listed courses (`CS201` = `PHIL201`) use `cross_listed_as` on the course.
- A variable-credit course uses `credit_range`. A repeatable course uses
  `repeatable` and `max_repeat_credits`.

### The placeholder trap

Some course keys look like placeholder names to the analyzer: 2–4 letters followed
by a number below 100 (`CSE12`, `MATH10`), or any key starting with `ELEC`.
Placeholders stand in for unchosen electives (`FE01`, `ELEC001`). They are left out
of the stored per-course metrics, and plan checks skip them.

Some catalogs number courses below 100 (UC San Diego's `CSE 12`, for example).
Adding a separator (`CSE_12`) avoids the placeholder rule. But patterns match on
the key, so `CSE:10+` would no longer find the course. In that case, list the
course explicitly in `from.courses` instead of relying on a pattern. Say which
choice you made.

## Gen-ed and other pools the catalog does not enumerate

A university pool such as "6 credits of humanities" often has hundreds of
courses. Write it as a `select` over a pattern (`"HUM:100+"`, or `"*:100+"` for
any subject), and don't list the courses. Such a pattern matches nothing that is
listed, so validate with `allow_unmatched_patterns=true`. That turns the error into
a warning. The analyzer fills these slots with placeholder courses.

## Credit arithmetic

Show the sum. Report it as follows:

```
Major core 36 + electives 9 + track 3 + math 8 + science 4 + gen-ed 30 + free 30
= 120 calculated, 120 stated, difference 0
```

- `all`: sum the course credits. For a bundle, add up its parts. For a choice,
  use the credits of the option actually offered.
- `select`: use the stated `credits`, or `count` times the typical credits.
- Free electives are the total minus everything else. Set `fills_to_total: true`
  on that block so each plan sizes it.
- A difference that is not 0 must be explained in the file or to the user.
  Never hide it.

## Metadata

Include in `degree:`:
- `institution` and `unitid`, from `search_institutions`;
- `program`, `degree_type` and `cip_code`, from `search_cip_codes`;
- `catalog_year`, `source_url` and `total_credits`;
- the GPA and grade minimums, each exactly as stated. "C" usually means C, not
  C-. Record any doubt in `grade_minimum_note`;
- `major_subjects`, the subject prefixes of the major. `audit_degree` uses them to
  scope its findings.
