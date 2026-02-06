# Schema v5.2 Update Summary

## Overview

Schema v5.2 adds support for combining explicit course lists with pattern-based matching in a single `from` block. This addresses a common limitation when modeling requirements like "Technology Focus" electives that allow both specific approved courses AND courses from certain ranges.

## Key Changes

### 1. Combined `courses` + `pattern`/`include`

**Before (v5.1):**
```yaml
# Could only use ONE of these:
from:
  courses: [BZ350, MGT340]     # Option A: explicit list
  # OR
  pattern: "CS:300-479"         # Option B: pattern
  # But NOT both together
```

**After (v5.2):**
```yaml
# Can now combine them:
from:
  courses: [BZ350, MGT340]      # Explicit courses (always included)
  pattern: "CS:300-479"          # Pattern matches (added to pool)
  exclude: [CS314]               # Exclusions (only affect pattern)
```

### 2. New `include` Field for Multiple Patterns

```yaml
from:
  courses: [BZ350, ECE452]
  include:                        # NEW: array of patterns
    - "CS:300-479"
    - "MATH:300-479"
    - "STAT:300-479"
  exclude: [CS314, MATH369]
```

### 3. Clarified Exclusion Behavior

- `exclude` only removes courses from **pattern matches**
- Explicit courses in the `courses` array are **never excluded**
- This allows precise control over the course pool

## Pool Calculation Logic

```
pool = explicit_courses ∪ pattern_matches − exclusions

Where:
- explicit_courses = all courses in `courses` array
- pattern_matches = all courses matching `pattern` OR any pattern in `include`
- exclusions = courses/patterns in `exclude` (applied ONLY to pattern_matches)
```

## Files Updated

| File | Description |
|------|-------------|
| `schema-v5.2.yaml` | Full schema definition with new features documented |
| `quick-reference-v5.2.md` | Quick reference card updated for v5.2 |
| `csu-cs-bscs-general.yaml` | Example using the new feature (Technology Focus) |

## Project Instructions Updates Needed

### 1. Update Schema Reference

Change references from `schema-v5.1.yaml` to `schema-v5.2.yaml`.

### 2. Update Generation Guide

Add a new section for combined courses+patterns:

```markdown
### Pattern 5: Specific Courses + Course Range

When the catalog says "Select from [specific list] OR any [SUBJECT] 300-479":

```yaml
# Catalog: "6 credits from: BZ 350, MGT 340, or any CS 300-479"
electives:
  type: select
  from:
    courses:
      - BZ350
      - MGT340
    pattern: "CS:300-479"
    exclude: [CS314]  # If CS 314 is required elsewhere
  credits: 6
```

For multiple patterns:
```yaml
# Catalog: "From approved list OR any CS/MATH/STAT 300-479"
from:
  courses: [BZ350, ECE452, PHIL410]
  include:
    - "CS:300-479"
    - "MATH:300-479"
    - "STAT:300-479"
  exclude:
    - MATH369      # Required elsewhere
    - "CS:380-399" # Special topics excluded
```
```

### 3. Update Quick Reference Mapping Table

Add this row to the "Catalog → Schema" table:

| Catalog Says | Schema Structure |
|--------------|------------------|
| "Specific courses + range" | `courses` + `pattern`/`include` |

### 4. Update Validation Checklist

Add:
- [ ] When using `courses` + `pattern`, verify `exclude` logic is correct
- [ ] Explicit courses are never accidentally excluded

## Backward Compatibility

- v5.1 files work unchanged in v5.2
- No migration required for existing files
- New feature is opt-in only

## Real-World Example: CSU Technology Focus

The CSU BS Computer Science "Technology Focus" requirement demonstrates the new feature:

**Catalog says:**
> Select 6 credits from: BZ 350, BZ 360, CIS 320, DSCI 235, ECE 452, ENGR 422, JTC 372/472, MATH 161, MATH 256, MGT 330/340/420, PHIL 410/411/415, PSY 252/352/452/454/456/458, CS 300-479, CT 300-479, DSCI 300-479, MATH 300-479, STAT 300-479, IDEA 300-479

**v5.2 modeling:**
```yaml
from:
  courses:
    - BZ350
    - BZ360
    - CIS320
    - DSCI235
    - ECE452
    - ENGR422
    - JTC372
    - JTC472
    - MATH161
    - MATH256
    - MGT330
    - MGT340
    - MGT420
    - PHIL410
    - PHIL411
    - PHIL415
    - PSY252
    - PSY352
    - PSY452
    - PSY454
    - PSY456
    - PSY458
  include:
    - "CS:300-479"
    - "CT:300-479"
    - "DSCI:300-479"
    - "MATH:300-479"
    - "STAT:300-479"
    - "IDEA:300-479"
  exclude:
    - CT301           # Required elsewhere
    - DSCI369         # Required elsewhere
    - MATH369         # Required elsewhere
    - STAT301         # Statistics requirement
    - STAT302A
    - STAT307
    - STAT315
    - "CS:380-399"    # Special topics
    - "CS:480-499"    # Capstone range
credits: 6
constraints:
  exclude_used: true
  min_upper_division: 1
```

This accurately captures that students can pick from specific approved courses OR any course matching the patterns, while properly excluding courses required elsewhere.
