# Degree Requirements YAML Generation Guide v3.0

This guide provides detailed instructions for generating degree requirement YAML files. Read this document before generating any YAML to avoid common errors.

---

## Table of Contents
1. [Catalog Navigation](#1-catalog-navigation)
2. [Degree Metadata Extraction](#2-degree-metadata-extraction)
3. [Requirement Mapping](#3-requirement-mapping)
4. [Interpreting Requirement Structures](#4-interpreting-requirement-structures)
5. [Course Verification](#5-course-verification)
6. [Building the Course Catalog](#6-building-the-course-catalog)
7. [Lab/Lecture Bundles](#7-lablecture-bundles)
8. [Concentrations and Tracks](#8-concentrations-and-tracks)
9. [Credit Arithmetic](#9-credit-arithmetic)
10. [Validation Checklist](#10-validation-checklist)
11. [Common Errors](#11-common-errors)

---

## 1. Catalog Navigation

Before extracting any data, thoroughly explore the catalog structure.

### 1.1 Identify All Relevant Pages

Degree requirements are often split across multiple pages:

| Page Type | What It Contains |
|-----------|------------------|
| Main degree page | Overall structure, credit totals, GPA requirements |
| Concentration pages | Track-specific requirements, elective lists |
| Course catalog | Course descriptions, credits, prerequisites |
| Sample plans | 4-year schedules, course sequencing |
| College requirements | School-wide requirements beyond major |
| University requirements | Gen-ed, writing, diversity requirements |

### 1.2 Verify Catalog Year

- Confirm you're viewing the correct academic year
- Look for "Effective Fall 202X" notices
- Check for curriculum change announcements
- Cross-reference sample plans (they reflect current curriculum)

### 1.3 Note Recent Changes

Search for language indicating changes:
- "This course replaces..."
- "Previously numbered as..."
- "New requirement starting..."
- "No longer required effective..."

---

## 2. Degree Metadata Extraction

Extract these fields from the catalog:

```yaml
degree:
  id: institution-dept-degree-track    # Unique identifier
  institution: string                   # Full university name
  program: string                       # Full degree name with track
  catalog_year: "YYYY-YYYY"            # Academic year
  source_url: string                    # Primary catalog URL
  total_credits: integer                # Minimum for graduation
  upper_division_credits: integer|null  # Min 300+ credits (null if not stated)
  in_major_credits: integer|null        # Credits in major subjects
  gpa_minimum: number                   # Overall GPA required
  gpa_major: number|null                # Major GPA if different
  grade_minimum: string|null            # Default min grade for major courses
  grade_minimum_note: string|null       # Clarification (e.g., "C means C, not C-")
  major_subjects: [string]              # Subject codes for major (e.g., ["CS", "CY"])
  allow_double_counting: boolean        # Can courses satisfy multiple requirements?
```

### Grade Requirement Notes

Pay special attention to grade requirements:
- "C" typically means C (2.0), not C- (1.7) - verify in catalog
- Some programs require "B" in foundational courses
- Prerequisites may have different grade requirements than degree default
- Document ambiguities in `grade_minimum_note`

---

## 3. Requirement Mapping

### 3.1 Requirement Categories Checklist

Verify each category exists or explicitly note as absent:

**Major Requirements:**
- [ ] Orientation/first-year seminar
- [ ] Foundational/introductory sequence
- [ ] Core required courses
- [ ] Upper-division required courses
- [ ] Capstone/senior project/thesis
- [ ] Concentration/track requirements
- [ ] Major electives

**Supporting Requirements:**
- [ ] Mathematics (calculus, linear algebra, discrete math, statistics)
- [ ] Natural sciences (with labs)
- [ ] Engineering fundamentals (digital design, circuits)
- [ ] Technical electives outside major

**General Education:**
- [ ] First-year writing
- [ ] Advanced/discipline-specific writing
- [ ] Communication/presentation
- [ ] Ethics/social issues
- [ ] Humanities distribution
- [ ] Social sciences distribution
- [ ] Diversity/global perspectives

**Other:**
- [ ] Free/general electives
- [ ] Experiential (co-op, internship)
- [ ] Professional development

### 3.2 Requirement Schema Types

The schema supports three requirement types:

| Type | Use When | Key Fields |
|------|----------|------------|
| `all` | Must complete every listed course | `courses: [list]` |
| `select` | Choose N courses/credits from options | `from`, `count` or `credits` |
| `one_of` | Choose one mutually exclusive path | `options: [list]` |

---

## 4. Interpreting Requirement Structures

**This is where most errors occur.** Carefully analyze catalog language.

### 4.1 Pattern Recognition

| Catalog Language | Correct Type | Structure |
|------------------|--------------|-----------|
| "Complete all of the following:" | `all` | `courses: [A, B, C]` |
| "Complete 3 courses from:" | `select` | `count: 3` |
| "Complete 12 credits from SUBJ 300+:" | `select` | `from.pattern`, `credits: 12` |
| "Choose one track:" | `one_of` | `options: [...]` |
| "One course from each area:" | `select` | `from.groups`, `per_group: 1` |
| "Courses from 2 of these 3 areas:" | `select` | `from.groups`, `groups_required: 2` |

### 4.2 Common Misinterpretations

#### Pattern 1: Parenthetical Choices in a List

```
Catalog: "Complete: ICS 312, (ICS 313 or ICS 361), ICS 314, ICS 321"

WRONG:
  type: all
  courses: [ICS312, ICS313, ICS361, ICS314, ICS321]

CORRECT:
  type: all
  courses:
    - ICS312
    - "{ICS313, ICS361}"    # Choice syntax
    - ICS314
    - ICS321
```

#### Pattern 2: "Two of the Following Pairs"

```
Catalog: "Complete two of: (A or B), (C or D), (E or F)"

WRONG:
  type: all
  courses: [A, B, C, D, E, F]

CORRECT:
  type: select
  from:
    groups:
      - id: pair_1
        courses: [A, B]
      - id: pair_2
        courses: [C, D]
      - id: pair_3
        courses: [E, F]
    groups_required: 2
    per_group: 1
```

#### Pattern 3: Alternative Sequences

```
Catalog: "(MATH 141 and 142) or (MATH 151 and 152)"

CORRECT:
  type: all
  courses:
    - "{[MATH141, MATH142], [MATH151, MATH152]}"
```

#### Pattern 4: Electives with Exclusions

```
Catalog: "8 credits from CS 3000+, excluding CS 3999 and CS 3998"

CORRECT:
  type: select
  from:
    pattern: "CS:3000+"
    exclude: [CS3999, CS3998]
  credits: 8
```

### 4.3 Verification Questions

After mapping each requirement, ask:

1. **Does the credit total make sense?**
   - If you mapped 8 courses as "all required" at 3 credits each = 24 credits
   - But catalog says "15 credits" → you misinterpreted something

2. **Does the course count match?**
   - Catalog says "57 credits, 18 courses"
   - Your "all required" list has 25 courses → wrong interpretation

3. **Are there "or" statements you missed?**
   - Re-read for parentheses, slashes, "or" words

---

## 5. Course Verification

**Course numbers change frequently. Verify every course.**

### 5.1 For Each Course Referenced

1. Search the institution's current course catalog for exact course ID
2. Verify the course exists in the stated catalog year
3. Confirm credits match catalog
4. Extract prerequisites exactly as stated
5. Note any corequisites

### 5.2 Signs of Outdated Information

- Course not found in current catalog
- Different course title than expected
- Different credit value
- Sample plan shows different course number
- "Formerly..." or "Replaces..." notes

### 5.3 Cross-Reference Sample Plans

Sample 4-year plans show current course numbers:
- If sample shows "CS 2000" but you wrote "CS 2500" → investigate
- All courses in sample must exist in your `courses` section
- Prerequisite ordering in sample should be valid

---

## 6. Building the Course Catalog

### 6.1 Required Course Entries

Every course referenced ANYWHERE must be defined:

- Courses in `courses` arrays
- Courses in `from.courses` arrays
- Courses in `from.groups`
- Courses in concentration requirements
- All prerequisite courses (trace to roots)
- All corequisite courses
- All courses in sample plans

### 6.2 Course Entry Format

```yaml
COURSEID:
  subject: string           # "CS"
  number: string            # "314" (string for leading zeros)
  title: string             # Full title from catalog
  credits: integer          # Credit hours
  prerequisites: string     # Boolean expression or null
  corequisites: [string]    # Concurrent courses or null
  typically_offered: [string]  # ["fall", "spring", "summer"]
  grade_minimum: string     # If stricter than degree default
  repeatable: boolean       # For research/special topics
  max_repeat_credits: integer  # If repeatable
```

### 6.3 Prerequisite Expression Syntax

```
Single:        "CS165"
AND:           "CS165 & CS220"
OR:            "CS250 | CS270"
Grouped:       "(CS165 & CS220) | MATH301"
With grade:    "ICS111[B]"          # B or better required
Complex:       "(ICS211[B] & ICS241) | (MATH301 & MATH372)"
None:          null
```

---

## 7. Lab/Lecture Bundles

### 7.1 Zero-Credit Corequisite (Bundled)

When lab credits are included in lecture:

```yaml
PHYS1151:
  title: Physics for Engineering 1
  credits: 4                    # Includes lab
  corequisites: [PHYS1152]

PHYS1152:
  title: Lab for PHYS 1151
  credits: 0                    # Bundled with lecture
  corequisites: [PHYS1151]
```

### 7.2 Separate Credits

When lab has its own credits:

```yaml
CHEM161:
  title: General Chemistry I
  credits: 3
  corequisites: [CHEM161L]

CHEM161L:
  title: General Chemistry I Lab
  credits: 1
  corequisites: [CHEM161]
```

### 7.3 Rules

- Corequisites must be **bidirectional**
- In requirements, use bundle syntax: `"[CHEM161, CHEM161L]"`
- Total credits (lecture + lab) must match catalog
- Be consistent within the file

---

## 8. Concentrations and Tracks

### 8.1 Basic Structure

```yaml
concentration:
  name: Concentration Selection
  type: one_of
  category: major
  options:
    - id: track_id
      name: Track Display Name
      requirements:
        - name: Required Core
          type: all
          courses: [COURSE1, COURSE2]
        - name: Track Electives
          type: select
          from:
            courses: [OPT1, OPT2, OPT3, OPT4, OPT5]  # ALL options
          count: 2
          constraints:
            exclude_used: true
```

### 8.2 Pick-From-Groups Within Track

```yaml
requirements:
  - name: Breadth Requirement
    type: select
    from:
      groups:
        - id: systems
          name: Systems
          courses: [CS370, CS470, CS457]
        - id: theory
          name: Theory
          courses: [CS420, CS441, CS480]
        - id: applications
          name: Applications
          courses: [CS430, CS440, CS445]
      groups_required: 2
      per_group: 1
```

---

## 9. Credit Arithmetic

### 9.1 Sum All Requirements

```
Major Core:              XX credits
Major Electives:         XX credits
Concentration:           XX credits
Mathematics:             XX credits
Science:                 XX credits
Writing:                 XX credits
General Education:       XX credits
Free Electives:          XX credits
─────────────────────────────────────
CALCULATED TOTAL:        XXX credits
STATED TOTAL:            XXX credits
DIFFERENCE:              XXX ← Must be 0 or explained
```

### 9.2 Per-Block Verification

| Type | Calculation |
|------|-------------|
| `type: all` | Sum credits of all courses |
| `type: select` with `count` | count × typical credits |
| `type: select` with `credits` | Use stated credits |
| Bundles | Sum all courses in bundle |

### 9.3 Common Arithmetic Errors

- Counting lab credits twice
- Wrong multiplication (5 × 4 = 20, not 25)
- Forgetting 0-credit components
- Not handling variable-credit courses
- Free electives should = total - sum(others)

---

## 10. Validation Checklist

### Completeness
- [ ] All catalog sections have corresponding requirements
- [ ] All courses in requirements are in `courses` section
- [ ] All prerequisites reference defined courses
- [ ] Corequisites are bidirectional
- [ ] Elective lists are complete (not truncated)

### Accuracy
- [ ] Course numbers verified against current catalog
- [ ] Course credits match catalog
- [ ] Prerequisites match catalog exactly
- [ ] Requirement type matches catalog intent
- [ ] Grade requirements captured correctly

### Arithmetic
- [ ] Each requirement block credits verified
- [ ] Total credits sum correctly
- [ ] Free electives = total - other requirements

### Structure
- [ ] Concentrations use `type: one_of`
- [ ] Pick-from-groups uses `from.groups`
- [ ] Bundle syntax for lecture+lab
- [ ] Choice syntax for interchangeable courses
- [ ] `exclude_used: true` where needed

---

## 11. Common Errors

### Error 1: Misinterpreting "Pick From" as "All Required"

**Symptom:** Credit total doesn't match; too many required courses

**Fix:** Re-read for "select," "choose," "or" language; check parenthetical groupings

### Error 2: Outdated Course Numbers

**Symptom:** Courses not in current catalog; sample plans differ

**Fix:** Verify every course ID in current course catalog

### Error 3: Missing Requirement Categories

**Symptom:** Credits don't sum; catalog sections not in YAML

**Fix:** Use checklist; map every catalog header to a requirement

### Error 4: Incomplete Elective Lists

**Symptom:** Fewer options than expected; truncated lists

**Fix:** Include ALL courses; check concentration pages

### Error 5: Broken Prerequisite Chains

**Symptom:** Referenced courses don't exist

**Fix:** Trace every prerequisite to its root; define all courses

### Error 6: Credit Calculation Errors

**Symptom:** Arithmetic doesn't work

**Fix:** Show work; verify each course's credits

### Error 7: Inconsistent Lab Handling

**Symptom:** Credit totals off; missing corequisites

**Fix:** Check catalog for values; ensure bidirectional links

### Error 8: Flattening Nested Structures

**Symptom:** Too many required courses; doesn't match "2 of 3" language

**Fix:** Use `from.groups` with `groups_required`
