# Schema v5.1 Quick Reference Card

## Requirement Types

| Type | Use Case | Required Fields |
|------|----------|-----------------|
| `all` | Complete every course | `courses: [list]` |
| `select` | Choose N from options | `from:`, `count:` or `credits:` |
| `one_of` | Mutually exclusive paths | `options: [list]` |

## Course Reference Syntax

```yaml
# Single course
courses: [CS314]

# Bundle (must take all together - lecture + lab)
courses: ["[CHEM161, CHEM161L]"]

# Choice (pick one - interchangeable)
courses: ["{CS250, CS270}"]

# Combined (pick one bundle)
courses: ["{[CHEM161, CHEM161L], [CHEM107, CHEM108]}"]
```

## Pattern Syntax

```yaml
from:
  pattern: "CS:300+"      # CS courses 300 and above
  pattern: "CS:300-399"   # CS courses 300-399 only
  pattern: "CS:*"         # Any CS course
  pattern: "*:300+"       # Any subject, 300+ level
  exclude: [CS399, CS499] # Specific exclusions
  exclude: ["CS:480-499"] # Range exclusion
```

## Select from Groups

```yaml
# "Pick one course from each of 2 areas (out of 3)"
breadth:
  type: select
  from:
    groups:
      - id: area_a
        courses: [A1, A2]
      - id: area_b
        courses: [B1, B2]
      - id: area_c
        courses: [C1, C2]
    groups_required: 2
    per_group: 1
```

## Prerequisite Expressions

```yaml
# Single
prerequisites: "CS165"

# AND
prerequisites: "CS165 & CS220"

# OR
prerequisites: "CS250 | CS270"

# Grouped
prerequisites: "(CS165 & CS220) | MATH301"

# With grade requirement
prerequisites: "CS111[B]"

# Complex
prerequisites: "(CS211[B] & CS241) | (MATH301 & MATH372)"

# None
prerequisites: null
```

## Constraints

```yaml
constraints:
  exclude_used: true        # Don't reuse courses from prior requirements
  distinct_subjects: true   # Must be from different subjects
  grade_minimum: "B"        # Override degree default
  min_upper_division: 2     # Minimum 300+ level courses
  max_from_subject: 6       # Max credits from one subject
```

## Common Catalog → Schema Mappings

| Catalog Says | Schema Structure |
|--------------|------------------|
| "Complete all:" | `type: all`, `courses: [...]` |
| "Complete 3 from:" | `type: select`, `count: 3` |
| "12 credits from X 300+:" | `type: select`, `from.pattern`, `credits: 12` |
| "Choose one track:" | `type: one_of`, `options: [...]` |
| "One from each area:" | `type: select`, `from.groups`, `per_group: 1` |
| "Two of three pairs:" | `from.groups`, `groups_required: 2`, `per_group: 1` |
| "A or B" in a list | Use choice syntax: `"{A, B}"` |
| "A and B" together | Use bundle syntax: `"[A, B]"` |

## Categories

```yaml
category: major       # Core major requirements
category: supporting  # Math, science, engineering
category: gen_ed      # University/college requirements
category: elective    # Free electives
```

## Lab/Lecture Patterns

```yaml
# Bundled (lab credits in lecture)
PHYS151:
  credits: 4
  corequisites: [PHYS151L]
PHYS151L:
  credits: 0  # Bundled
  corequisites: [PHYS151]

# Separate credits
CHEM161:
  credits: 3
  corequisites: [CHEM161L]
CHEM161L:
  credits: 1
  corequisites: [CHEM161]

# In requirements, use bundle syntax:
courses: ["[PHYS151, PHYS151L]"]
```

## Variable Credit / Repeatable

```yaml
CS499:
  credit_range:
    min: 1
    max: 3
  repeatable: true
  max_repeat_credits: 6
```

## Validation Checklist

- [ ] All courses in requirements exist in `courses` section
- [ ] All prerequisites reference defined courses
- [ ] Corequisites are bidirectional
- [ ] Credits sum to degree total
- [ ] Course numbers verified against current catalog
- [ ] "Pick from" vs "all required" correctly interpreted
- [ ] Elective lists are complete (not truncated)
