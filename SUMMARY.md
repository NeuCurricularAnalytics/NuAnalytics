# Metrics Fix Session Summary
**Date:** January 5, 2026

## What We Fixed Today ✅

### 1. Course Deduplication (MAJOR FIX)
**Problem:** Courses with identical PREFIX+NUMBER (e.g., "XXXX") were overwriting each other in HashMap
**Solution:** Implemented 3-pass CSV parser:
- Pass 1: Count occurrences of each natural key (PREFIX+NUMBER)
- Pass 2: Create courses with deduplicated keys (append "_ID" if duplicates exist)
- Pass 3: Add prerequisites using ID-to-key mapping

**Result:** All duplicate courses now export correctly with proper keys (XXXX_12, XXXX_13, etc.)

### 2. Cycle Detection Errors (FIXED)
**Problem:** Files failing with "Cycle detected" errors for SCI2++ and WRITXX courses
**Solution:** Deduplication fixed this - prerequisites now map to correct deduplicated keys
**Result:** All 9 files now process without cycle errors

### 3. Missing Courses in Export (FIXED)
**Problem:** California showing 15/30 courses, other files missing courses
**Solution:** DAG wasn't using deduplicated keys; fixed by ensuring build_dag() uses storage keys
**Result:** All 9 files now export complete course lists

### 4. Metrics for Duplicate Courses (FIXED)
**Problem:** XXXX courses had delay=0, blocking=0 (impossible according to formula)
**Solution:** Metrics HashMap wasn't finding deduplicated keys; fixed by ensuring DAG uses correct keys
**Result:** All duplicate courses now have proper metrics (delay ≥ 1)

---

## Current Status 📊

### Perfect Files (5/9) 🎉
1. ✅ BSCS Hawaii Manoa - 41 courses, C:236 Co:99 B:137 D:399
2. ✅ California Berkeley V2 - 30 courses, C:100 Co:30 B:70 D:127
3. ✅ Colostate CS Degree - 37 courses, C:175 Co:61 B:114 D:147
4. ✅ Colostate CS Degree 2017 - 37 courses, C:208 Co:74 B:134 D:521
5. ✅ Colostate CS Degree 2017 w/MATH - 41 courses, C:415 Co:174 B:241 D:2866

### Files with Remaining Issues (4/9)
1. ❌ Kennesaw State - 24/43 courses wrong, -50 complexity
2. ❌ Metropolitan State - 2/34 courses wrong, -3 complexity
3. ❌ Michigan Ann Arbor - 38/38 courses wrong, +33 complexity (OVER-counting)
4. ❌ Colorado Boulder - 1/38 courses wrong, -2 complexity

---

## Root Cause of Remaining Errors 🔍

### CONFIRMED: Missing Strict-Corequisite Parsing

**Evidence:**
```csv
# Kennesaw Course 9 (CSE1321):
9,Programming and Problem Solving I,CSE,1321,,,10,3,,
                                                  ^^
                                          Column 7 = "10"

# This is STRICT-COREQUISITE field!
```

**Current Code:**
- ✅ Parses Prerequisites (column 5)
- ✅ Parses Corequisites (column 6)
- ❌ Does NOT parse Strict-Corequisites (column 7)

**Impact:**
- Lab courses (course 10 in example) should be strict-coreqs of lecture courses
- These relationships aren't in the DAG at all
- Causes lab courses to have delay=1, blocking=0 (isolated) when they should inherit parent's metrics
- Kennesaw course 10 should have delay=7, blocking=18 but has delay=1, blocking=0

---

## What Needs to be Done Tomorrow 🔧

### Step 1: Implement Strict-Corequisite Parsing (1 hour)
**File:** `src/core/planner/csv_parser.rs`

Add to Pass 3 (around line 127):
```rust
// Parse strict-corequisites
if let Some(strict_coreq_str) = get_field(line, "Strict-Corequisites", &headers) {
    if !strict_coreq_str.trim().is_empty() {
        add_corequisites_with_mapping(course, &strict_coreq_str, &course_id_to_storage_key);
    }
}
```

**Question to answer:** Should strict-coreqs be added as:
- Regular corequisites (current coreq implementation)?
- Prerequisites (stronger dependency)?
- Or a third type of edge?

### Step 2: Verify Strict-Coreq Behavior (30 min)
Test with Kennesaw:
1. Add strict-corequisite parsing
2. Check if course 10 now has correct metrics
3. If not, may need to treat strict-coreqs as prerequisites for metrics

### Step 3: Debug Michigan Over-counting (1 hour)
Michigan is the ONLY file where our metrics are higher than reference
- Check if we're creating extra edges
- Verify corequisite edge direction (uni vs bidirectional)
- May need different handling for lab courses

### Step 4: Final Validation (30 min)
Run all 9 files and confirm:
- Target: 8/9 or 9/9 perfect matches
- All lab courses have correct metrics
- No cycle errors, no missing courses

---

## Key Files Modified Today 📝

1. **src/core/planner/csv_parser.rs** - Added 3-pass deduplication parser
2. **src/core/models/course.rs** - Added csv_id and id fields
3. **src/core/models/school.rs** - Added add_course_with_key() method
4. **src/core/models/dag.rs** - No changes (already handles deduplicated keys correctly)
5. **src/core/metrics_export.rs** - Uses storage_key from plan.courses

---

## Test Commands 🧪

```bash
# Run all files
cargo run --release -- planner samples/plans/*.csv

# Compare metrics
python3 /tmp/compare_metrics.py

# Check specific course in CSV
awk -F',' 'NR>=8 && $1 == "10"' samples/plans/Kennesaw_State_University_CS.csv

# Check if strict-coreqs are parsed
grep -i "strict" src/core/planner/csv_parser.rs
```

---

## Success Metrics 🎯

**Today's Achievements:**
- ✅ 5/9 files perfect (up from 1/9)
- ✅ 100% course counts correct
- ✅ 0 cycle detection errors (down from 3)
- ✅ All duplicate courses export correctly

**Tomorrow's Goals:**
- 🎯 8-9/9 files perfect
- 🎯 Fix all lab course relationships
- 🎯 Understand and resolve Michigan over-counting
- 🎯 < 1% error rate across all metrics

---

## Notes for Tomorrow 📋

1. **Start with Kennesaw course 10** - it's the key test case
2. **Strict-corequisites likely need special handling** - not just regular coreqs
3. **Michigan is unusual** - investigate separately from lab course issues
4. **Reference files may have inconsistencies** - Colorado and Metropolitan are very close already

**Estimated completion time:** 2-4 hours
**Confidence level:** HIGH (root cause identified)
