---
name: degree-review
description: Check, fix or update an existing NuAnalytics degree file (YAML or unified JSON). Use when the user shares a degree file and asks for feedback, validation or an audit, wants requirements or courses changed, or wants the file brought up to a new catalog year.
allowed-tools: mcp__nuanalytics__validate_degree mcp__nuanalytics__audit_degree mcp__nuanalytics__get_course_detail mcp__nuanalytics__find_courses_matching mcp__nuanalytics__get_reference
---

# Degree review

**Goal:** a degree file that validates and says what the catalog says, with every
change explained.

## Tools

- `validate_degree(path=…)` returns `is_valid`, `errors`, `warnings` and
  `suggestions`. A failing file is a finding, not a failed call.
- `audit_degree(path=…)` finds upper-level courses with no prerequisites and deep
  prerequisite chains. `chain_threshold` sets how deep counts as deep. Findings are
  scoped by the degree's `major_subjects`.
- `get_course_detail(path=…, course_id="CS3500")` shows one course's prerequisites,
  dependents, and the requirements that name it. Use it before changing a course
  others depend on.
- `find_courses_matching(path=…, patterns=…)` shows which courses a pattern selects.
- `get_reference(topic="degree-yaml", section=…)` is the format.
- `trim_degree` derives a single-path copy of a degree. It is not a way to edit one.

## Steps

1. Validate first and record the errors that were already there, so your changes can be
   told apart from them.
2. Audit. Group the findings: prerequisites, chains, credits, structure.
3. If the user gave a catalog, check the file against it.
4. Fix or propose fixes. Show each change as the corrected YAML block.
5. Validate again. Every error you introduced must be gone.

## What you would otherwise get wrong

- **Keep the file's format.** Edit YAML as YAML and JSON as JSON. Convert only when
  asked, with `convert_degree`.
- **A trimmed file is derived.** Edit the source degree and trim again. An edit made to
  the trimmed copy is lost the next time it is regenerated.
- **Ask before deleting a requirement.** Removing one changes every plan and metric.
- **Common warnings:**
  - A hidden requirement means a course is required only through a prerequisite chain.
    Usually the prerequisite sits in just one option of a choice.
  - A pattern that matches nothing means the courses aren't listed yet, or the pool is
    external (see `allow_unmatched_patterns`).
- **Course numbers drift.** CS 100 and CS 1000 are different courses. So is a course
  "formerly numbered" something else. Check against the catalog rather than guessing.

## Done when

- `validate_degree` reports no error that your changes introduced. Report any error
  that was there before and is still there.
- Every change is listed with its reason and, where one exists, its catalog source.
- The credit totals are reconciled for every section that changed.
