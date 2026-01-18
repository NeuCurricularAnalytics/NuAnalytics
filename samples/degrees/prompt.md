# Degree Requirements YAML Generation Prompt v3.0

You are generating a structured YAML file representing degree requirements for academic planning software. The output must conform to the provided schema and capture ALL requirements, courses, and constraints from the source catalog.

## INPUT
- Schema: [provided separately]
- Catalog URL(s): [provided at end of prompt]

---

## PROCESS

### Step 1: Navigate and Inventory the Catalog

Before extracting any data, thoroughly explore the catalog structure:

1. **Identify all relevant pages** - Degree requirements are often split across multiple pages:
   - Main degree/program page
   - Concentration/track/specialization pages
   - Course catalog/descriptions
   - Sample 4-year plans
   - College/school-wide requirements
   - University-wide requirements

2. **Note the catalog year** - Verify you are looking at the correct academic year. Requirements change; using outdated information is a common error.

3. **Look for recent changes** - Search for language like:
   - "Effective Fall 202X..."
   - "This course replaces..."
   - "Previously numbered as..."
   - "New requirement starting..."

### Step 2: Extract Degree Metadata

From the catalog, identify:

| Field | Description | Where to Find |
|-------|-------------|---------------|
| Institution | Full university name | Page header |
| Program | Complete degree name with track | Program title |
| Catalog year | Academic year (e.g., "2024-2025") | Catalog header/footer |
| Total credits | Minimum for graduation | Usually stated explicitly |
| Upper division credits | Minimum 300+ level (null if not stated) | Degree requirements section |
| In-major credits | Credits within major subjects | Often in major requirements |
| GPA requirements | Overall and major GPA | Academic standards section |
| Minimum grades | Per-course minimums (watch for "C" vs "C-") | Prerequisites or policies |
| Major subjects | Subject codes counting toward major | Elective definitions |

**Pay special attention to grade requirements:**
- "C" typically means C (2.0), not C- (1.7)
- Some programs require "B" in foundational courses
- Prerequisites may have different grade requirements than the course itself

### Step 3: Map ALL Requirements (Exhaustive Inventory)

Create a complete inventory of every requirement category. **Do not skip any section visible in the catalog.**

#### 3a: Common Requirement Categories Checklist

Verify each exists in the catalog or explicitly note as absent:

**Major Requirements:**
- [ ] Orientation/overview/first-year seminar
- [ ] Foundational/introductory sequence
- [ ] Core/required major courses
- [ ] Upper-division major courses
- [ ] Capstone/senior project/thesis
- [ ] Concentration/specialization/track (if applicable)
- [ ] Major electives (department courses)

**Supporting Requirements:**
- [ ] Mathematics (calculus, linear algebra, discrete math, statistics)
- [ ] Natural sciences (often with lab components)
- [ ] Engineering fundamentals (circuits, digital design, etc.)
- [ ] Technical electives outside major

**General Education / University Requirements:**
- [ ] First-year writing
- [ ] Advanced/discipline-specific writing
- [ ] Communication/presentation/public speaking
- [ ] Ethics/social issues in the discipline
- [ ] Humanities distribution
- [ ] Social sciences distribution
- [ ] Diversity/global perspectives
- [ ] Other university core requirements

**Other:**
- [ ] Free/general electives
- [ ] Experiential learning (co-op, internship, research)
- [ ] Professional development courses

#### 3b: Correctly Interpret Requirement Structure

**CRITICAL**: Catalog language can be ambiguous. Carefully distinguish between these patterns:

| Catalog Language | Schema Type | Structure |
|------------------|-------------|-----------|
| "Complete all of the following:" | `type: all` | `courses: [A, B, C]` |
| "Complete 3 courses from:" | `type: select` | `from.courses`, `count: 3` |
| "Complete 12 credits from SUBJ 300+:" | `type: select` | `from.pattern`, `credits: 12` |
| "Choose one track/concentration:" | `type: one_of` | `options: [...]` |
| "Complete one course from each area:" | `type: select` | `from.groups`, `per_group: 1` |
| "Complete courses from 2 of the 3 areas:" | `type: select` | `from.groups`, `groups_required: 2` |

**Common Misinterpretation Patterns:**

1. **Parenthetical choices within a list:**
   ```
   Catalog says: "ICS 312, (ICS 313 or ICS 361), ICS 314"
   WRONG: type: all, courses: [ICS312, ICS313, ICS361, ICS314]
   RIGHT: type: all, courses: [ICS312, "{ICS313, ICS361}", ICS314]
   ```

2. **"Two of the following pairs":**
   ```
   Catalog says: "Two of (A or B), (C or D), (E or F)"
   WRONG: type: all, courses: [A, B, C, D, E, F]
   RIGHT: type: select with from.groups, groups_required: 2, per_group: 1
   ```

3. **"Or" between sequences:**
   ```
   Catalog says: "(MATH 141 and 142) or (MATH 151 and 152)"
   RIGHT: type: all, courses: ["{[MATH141, MATH142], [MATH151, MATH152]}"]
   ```

4. **Electives with exclusions:**
   ```
   Catalog says: "8 credits from CS 3000+, excluding CS 3999"
   RIGHT: type: select, from.pattern: "CS:3000+", from.exclude: [CS3999], credits: 8
   ```

#### 3c: Verify Your Interpretation

After mapping each requirement, verify:

1. **Credit arithmetic**: Do the credits sum correctly?
   - Count courses × credits per course
   - Compare to stated requirement total
   - If mismatch, re-read the catalog

2. **Course count**: Does the number of courses make sense?
   - If catalog says "57 credits in major" and you have 20 courses at 3 credits = 60, something is wrong

3. **Logical consistency**:
   - Are there prerequisites for courses that aren't themselves required?
   - Does the stated sequence make sense?

### Step 4: Verify Course Information Against Current Catalog

**CRITICAL**: Course numbers, titles, credits, and prerequisites change frequently. Do not assume.

#### 4a: For Every Course Referenced

1. **Search the institution's course catalog** for the exact course ID
2. **Verify the course exists** in the current catalog year
3. **Confirm credits** match what the catalog states
4. **Extract prerequisites** exactly as stated
5. **Note corequisites** (labs, recitations, seminars)

#### 4b: Watch for Curriculum Changes

Look for indicators that courses have changed:
- Course not found in current catalog → may be discontinued or renumbered
- Different course title → verify it's the same course
- Different credit value → use current catalog value
- "Formerly known as..." or "Replaces..." notes

#### 4c: Cross-Reference with Sample Plans

If the catalog provides sample 4-year plans:
- Every course in the sample plan should exist in your courses section
- Course numbers in sample plans reflect current curriculum
- If sample plan shows "CS 2000" but you wrote "CS 2500", investigate

### Step 5: Build Complete Course Catalog

**Every course referenced anywhere in the YAML must have a full entry in the `courses` section.**

#### 5a: Courses That MUST Be Defined

1. All courses in requirement `courses` arrays
2. All courses in `from.courses` arrays
3. All courses in concentration/track requirements
4. All courses in `from.groups`
5. All prerequisite courses (trace chains to roots)
6. All corequisite courses (labs, recitations, seminars)
7. All courses that appear in sample plans

#### 5b: Course Entry Format

```yaml
COURSEID:
  subject: string           # e.g., "CS"
  number: string            # e.g., "314" (string to preserve leading zeros)
  title: string             # Full course title from catalog
  credits: integer          # Credit hours (use 0 for bundled labs)
  prerequisites: string     # Boolean expression (see syntax below)
  corequisites: [list]      # Courses that must be taken concurrently
  typically_offered: [list] # ["fall", "spring", "summer"] if known
  grade_minimum: string     # If stricter than degree default
  repeatable: boolean       # For research, special topics, etc.
  max_repeat_credits: int   # If repeatable, maximum total credits
```

#### 5c: Prerequisite Expression Syntax

Use boolean expressions for prerequisites:

```
Single course:        "CS165"
AND (both required):  "CS165 & CS220"
OR (either works):    "CS250 | CS270"
Grouping:             "(CS165 & CS220) | MATH301"
Grade requirement:    "ICS111[B]" (B or better required)
Complex:              "(ICS211[B] & ICS241) | (MATH301 & MATH372)"
None:                 null
```

### Step 6: Handle Lab/Lecture Bundles

When a course has required corequisites (labs, recitations, seminars):

**Option A - Zero-credit corequisite** (lab bundled into lecture credits):
```yaml
PHYS1151:
  credits: 4
  corequisites: [PHYS1152, PHYS1153]

PHYS1152:
  title: Lab for PHYS 1151
  credits: 0
  corequisites: [PHYS1151]
```

**Option B - Separate credits**:
```yaml
CHEM161:
  credits: 3
  corequisites: [CHEM161L]

CHEM161L:
  title: General Chemistry I Lab
  credits: 1
  corequisites: [CHEM161]
```

**Rules:**
- Corequisites must be bidirectional (both courses list each other)
- In requirements, use bundle syntax: `"[CHEM161, CHEM161L]"`
- Be consistent about credit handling (check catalog for actual values)
- Verify total credits match catalog (lecture + lab should equal stated total)

### Step 7: Handle Concentrations/Tracks

For programs with concentrations, use `type: one_of` with nested requirements:

```yaml
concentration:
  name: Concentration
  type: one_of
  category: major
  options:
    - id: concentration_id
      name: Concentration Display Name
      requirements:
        - name: Required Core
          type: all
          courses: [COURSE1, COURSE2]
        - name: Electives
          type: select
          from:
            courses: [OPT1, OPT2, OPT3, OPT4, OPT5, OPT6]  # ALL options from catalog
          count: 2
          constraints:
            exclude_used: true
```

**For "pick from groups" within concentrations:**
```yaml
requirements:
  - name: Breadth Areas
    type: select
    from:
      groups:
        - id: area_a
          name: Area A - Systems
          courses: [COURSE1, COURSE2, COURSE3]
        - id: area_b
          name: Area B - Theory
          courses: [COURSE4, COURSE5, COURSE6]
        - id: area_c
          name: Area C - Applications
          courses: [COURSE7, COURSE8, COURSE9]
      groups_required: 2  # Pick from 2 of the 3 areas
      per_group: 1        # One course from each selected area
```

### Step 8: Capture COMPLETE Elective Lists

**CRITICAL**: Elective and pick lists must be EXHAUSTIVE, not representative samples.

When the catalog lists course options:

1. **Include ALL named courses** - Do not truncate or summarize
2. **Check multiple pages** - Concentration pages often have fuller lists
3. **Verify cross-listed courses** - Include all valid cross-listings
4. **Note open-ended language** - If catalog says "including but not limited to," capture all named courses AND note this

**Red flags suggesting incomplete extraction:**
- Elective list has fewer than 5-6 options (most have 8+)
- Science requirement has only 1-2 courses per category
- Ethics/social issues requirement has fewer than 5 options
- Concentration electives have fewer options than the selection count

### Step 9: Credit Arithmetic Verification

**CRITICAL**: Perform explicit credit calculation to catch errors.

#### 9a: Sum All Requirement Credits

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
DIFFERENCE:              XX  ← Must be 0 or explained
```

#### 9b: Verify Each Block

- `type: all`: Sum credits of all listed courses
- `type: select` with `count`: count × typical credits per course
- `type: select` with `credits`: Stated credits should be achievable from options
- Bundles: Sum all courses in bundle

#### 9c: Common Calculation Errors

- Counting lab credits twice (in lecture AND separately)
- Wrong course count (5 courses × 4 credits = 20, not 25)
- Forgetting 0-credit components don't add to total
- Not accounting for variable-credit courses
- General electives should equal: total_credits - sum(all other requirements)

#### 9d: If Discrepancy Exists

1. Re-read catalog for missed requirements
2. Check if some requirements overlap (double-counting allowed?)
3. Verify course credit values
4. Document discrepancy in notes if unresolvable

### Step 10: Validate Prerequisite Chains

Before finalizing:

1. **Referential integrity**: Every course in `prerequisites` exists in `courses` section
2. **Corequisite integrity**: Every course in `corequisites` exists in `courses` section
3. **Chain completeness**: If A requires B requires C, all three must be defined
4. **No circular dependencies**: A cannot require B if B requires A
5. **Bidirectional corequisites**: If A lists B as corequisite, B must list A

### Step 11: Cross-Check Against Sample Plans

If the catalog includes sample 4-year plans:

1. **Verify all courses exist** in your `courses` section
2. **Check prerequisite ordering** - no course appears before its prerequisites
3. **Confirm credit totals** match or document difference
4. **Note if sample exceeds minimum** (common: minimum 120, sample shows 124)

---

## OUTPUT FORMAT

```yaml
# =============================================================================
# [Institution] - [Degree Program]
# Schema v5.1 | Catalog [Year]
# =============================================================================

degree:
  id: institution-dept-degree-track
  institution: Full Institution Name
  program: Full Degree Name Including Track
  catalog_year: "YYYY-YYYY"
  source_url: https://catalog.example.edu/path/to/degree
  total_credits: integer
  upper_division_credits: integer | null
  in_major_credits: integer | null
  gpa_minimum: number
  gpa_major: number | null
  grade_minimum: string | null
  grade_minimum_note: string | null  # e.g., "C means C, not C-"
  major_subjects: [SUBJ1, SUBJ2]
  allow_double_counting: boolean

# =============================================================================
# REQUIREMENTS
# =============================================================================

requirements:

  # --- Major Requirements ---

  requirement_id:
    name: Human-Readable Name
    type: all | select | one_of
    category: major | supporting | gen_ed | elective
    # ... (type-specific fields)

  # --- Supporting Requirements ---

  # --- General Education ---

  # --- Electives ---

# =============================================================================
# COURSES
# =============================================================================

courses:
  # This section should be substantial (typically 50-150+ courses)
  # Every course referenced in requirements MUST appear here

  COURSEID:
    subject: SUBJ
    number: "XXX"
    title: Full Course Title
    credits: X
    prerequisites: "expression" | null
    corequisites: [list] | null
```

---

## COMMON ERRORS TO AVOID

### 1. Misinterpreting "Pick From" as "All Required"
- **Wrong:** Listing all courses in a pick-list as required
- **Sign:** Credit total doesn't match, or course count seems too high
- **Fix:** Re-read catalog for "select," "choose," "or" language

### 2. Using Outdated Course Numbers
- **Wrong:** Using course numbers from an old curriculum
- **Sign:** Courses not found in current catalog, or sample plans show different numbers
- **Fix:** Verify every course ID against current course catalog

### 3. Missing Requirement Categories
- **Wrong:** Omitting ethics, communication, or engineering fundamentals
- **Sign:** Credit total doesn't add up; catalog has sections not in YAML
- **Fix:** Use the checklist; verify every catalog section header is captured

### 4. Incomplete Elective Lists
- **Wrong:** Listing 3-4 courses when catalog shows 10+
- **Sign:** List seems truncated; count doesn't match catalog
- **Fix:** Include ALL courses; check concentration pages for complete lists

### 5. Incorrect Prerequisite Chains
- **Wrong:** Referencing courses not defined in `courses` section
- **Sign:** Validation fails; chain breaks at undefined course
- **Fix:** Trace every prerequisite to its root; define all courses

### 6. Credit Calculation Errors
- **Wrong:** Credits don't sum to stated total
- **Sign:** Arithmetic doesn't work out
- **Fix:** Show your work; verify each course's credits

### 7. Inconsistent Lab Handling
- **Wrong:** Some labs 0-credit, others separate, with no pattern
- **Sign:** Credit totals off; corequisites missing
- **Fix:** Check catalog for credit values; ensure bidirectional corequisites

### 8. Flattening Nested Structure
- **Wrong:** Using `type: all` when catalog shows grouped choices
- **Sign:** Too many required courses; doesn't match "select 2 of 3" language
- **Fix:** Use `from.groups` with `groups_required` for pick-from-groups

---

## QUALITY CHECKLIST

Before submitting, verify each item:

### Completeness
- [ ] All catalog sections mapped to requirements
- [ ] All requirement categories from checklist present (or noted N/A)
- [ ] All courses in requirements defined in `courses` section
- [ ] All prerequisites trace to defined courses
- [ ] All corequisites are bidirectional
- [ ] Elective lists are complete (not samples)
- [ ] Sample plan courses (if any) are all defined

### Accuracy
- [ ] Course numbers verified against current catalog
- [ ] Course credits match catalog values
- [ ] Prerequisites match catalog (including grade requirements)
- [ ] Requirement structure matches catalog language (all vs. select vs. one_of)
- [ ] Grade minimums correctly captured

### Arithmetic
- [ ] Credits sum correctly for each requirement block
- [ ] Total credits match stated degree requirement
- [ ] General electives = total - sum(other requirements)
- [ ] Lab/lecture credit splits are correct

### Structure
- [ ] Concentrations use `type: one_of` with nested requirements
- [ ] "Pick from groups" uses `from.groups` with appropriate parameters
- [ ] `exclude_used: true` applied where courses shouldn't double-count
- [ ] Bundle syntax `[A, B]` used for lecture+lab
- [ ] Choice syntax `{A, B}` used for interchangeable courses

### Documentation
- [ ] Source URL is correct and accessible
- [ ] Catalog year matches source
- [ ] Grade requirements clarified if ambiguous (e.g., "C means C, not C-")
- [ ] Any unresolvable discrepancies noted

---

## CATALOG URL(S) TO PROCESS:

[Insert URLs here]
