# Metrics Error Analysis Report
**Date:** January 5, 2026
**Status:** 5/9 files perfect, 4/9 files with discrepancies

## Executive Summary

After implementing the three-pass deduplication parser, **5 out of 9 curriculum files now have PERFECT metrics**:
- ✅ BSCS Hawaii Manoa (41 courses)
- ✅ California Berkeley V2 (30 courses)
- ✅ Colostate CS Degree (37 courses)
- ✅ Colostate CS Degree 2017 (37 courses)
- ✅ Colostate CS Degree 2017 w/MATH (41 courses)

**4 files still have metric discrepancies:**
- ❌ Kennesaw State University CS (43 courses) - **MAJOR ISSUE**
- ❌ Metropolitan State University CS (34 courses) - **MINOR ISSUE**
- ❌ Michigan Ann Arbor CS (38 courses) - **UNIQUE PATTERN**
- ❌ U of Colorado Boulder CS (38 courses) - **TRIVIAL ISSUE**

---

## Detailed Analysis by File

### 1. Kennesaw State University CS ❌ MAJOR ISSUE
**Error Severity:** HIGH
**Courses with differences:** 24 out of 43 (56%)
**Total complexity difference:** -50 (230 vs 280)

**Primary Issue:** Course 10 (CSE1321L - Programming and Problem Solving I Lab)
- **Delay:** 1.0 vs 7.0 (off by 6!)
- **Blocking:** 0 vs 18 (off by 18!)
- **Complexity:** 1.0 vs 25.0 (off by 24!)

**Pattern Observed:**
- Course 10 (CSE1321L) is a LAB course that should be a corequisite of Course 9 (CSE1321)
- The lab has NO prerequisites or blocking in our output, but should have high metrics
- This suggests **corequisite relationships are not being parsed or linked correctly**
- Multiple courses show systematic -1 delay offset, indicating a missing prerequisite link
- Centrality differences are very large (up to -92), suggesting graph structure is wrong

**Root Cause Hypothesis:**
- Lab courses (Course ID 10, 12) may have strict-corequisite relationships that aren't being handled
- The reference treats course 10 as if it's a prerequisite of many courses, but we treat it as isolated
- Check if course 9 (CSE1321) should have course 10 (CSE1321L) as a **strict corequisite**

**Next Steps:**
1. Check CSV for strict-corequisite column values for courses 9 and 10
2. Verify if strict corequisites should create prerequisite-like edges in the DAG
3. Examine if lab courses need special handling in prerequisite chains

---

### 2. Metropolitan State University CS ❌ MINOR ISSUE
**Error Severity:** LOW
**Courses with differences:** 2 out of 34 (6%)
**Total complexity difference:** -3 (169 vs 172)

**Issues:**
- Course 22 (XXXX - Goal 3 - Natural Science Lab): delay 1→2, blocking 0→1
- Course 21 (XXXX - Goal 3 - Natural Science): delay 1→2

**Pattern Observed:**
- Both are XXXX courses (duplicate natural key)
- Both are science lab related
- Similar pattern to Kennesaw: a lab course (22) should depend on lecture course (21)

**Root Cause Hypothesis:**
- Course 21 and 22 likely have a corequisite/strict-corequisite relationship
- The relationship isn't being added to the DAG correctly

**Next Steps:**
1. Check if course 22 has course 21 as corequisite or strict-corequisite
2. Verify XXXX courses are being deduplicated correctly (they are, but check relationships)

---

### 3. Michigan Ann Arbor CS ❌ UNIQUE PATTERN
**Error Severity:** MEDIUM (but UNUSUAL)
**Courses with differences:** 38 out of 38 (100%!)
**Total complexity difference:** +33 (149 vs 116) - **WE'RE HIGHER**

**Issues:**
- Course 4 (Chemistry126 - Lab II): delay 1→3, blocking 0→2
- Course 7 (Physics141 - Lab I): delay 1→3, blocking 0→2
- Course 3 (Chemistry125 - Lab I): delay 1→3, blocking 0→1
- **ALL courses have metric differences** (even if small)

**Unique Pattern:**
- Our total complexity is HIGHER than reference (149 vs 116)
- Multiple lab courses show similar pattern (delay off by 2, blocking off by 1-2)
- This is the ONLY file where we're consistently OVER-counting metrics

**Root Cause Hypothesis:**
- Michigan file may have a different prerequisite structure in the reference
- Lab courses should be parallel (coreqs) but we might be treating them as sequential (prereqs)
- OR the reference has a different interpretation of how lab courses contribute to metrics
- Need to check if we're adding extra edges that shouldn't exist

**Next Steps:**
1. Compare prerequisite strings in CSV vs what we're parsing
2. Check if lab courses have corequisites being treated as prerequisites
3. Examine if course name parsing is creating spurious dependencies

---

### 4. U of Colorado Boulder CS ❌ TRIVIAL ISSUE
**Error Severity:** TRIVIAL
**Courses with differences:** 1 out of 38 (2.6%)
**Total complexity difference:** -2 (191 vs 193)

**Issue:**
- Course 33 (PHYS1140 - Experimental Physics 1): delay 1→2, blocking 0→1

**Pattern Observed:**
- Single physics lab course with minor metric difference
- Same pattern as other lab courses (should have delay=2, blocking=1 but we have delay=1, blocking=0)

**Root Cause Hypothesis:**
- Physics lab course should be linked to lecture course via corequisite
- Corequisite not being processed correctly

**Next Steps:**
1. Check course 33's corequisite field in CSV
2. Likely same root cause as Metropolitan and Kennesaw

---

## Common Patterns Across Errors

### Lab Course Pattern (Found in 3 files)
**Courses affected:**
- Kennesaw: CSE1321L (Course 10), CSE1322L (Course 12)
- Metropolitan: XXXX Goal 3 Lab (Course 22)
- Colorado: PHYS1140 Experimental Physics (Course 33)
- Michigan: Chemistry126, Physics141, Chemistry125 (Courses 3, 4, 7)

**Observation:**
- Lab courses consistently have lower delay and blocking than reference
- Suggests lab courses aren't being linked to their lecture courses properly
- May be a **strict-corequisite** issue (column 7 in CSV)

### XXXX Duplicate Course Pattern
**All XXXX courses now have correct metrics in files that are perfect** (California, Hawaii)
- But some XXXX lab courses in other files still have issues
- This suggests deduplication is working, but corequisite relationships aren't

### Systematic Offset Pattern
- Many courses in Kennesaw show -1 delay offset across the board
- Suggests a missing "root" course or prerequisite early in the chain

---

## Root Cause Analysis

### Confirmed Fixed Issues ✅
1. **Duplicate course deduplication** - FIXED (XXXX courses now work in California, Hawaii)
2. **Cycle detection** - FIXED (SCI2++, WRITXX no longer cause errors)
3. **Course count** - FIXED (all courses export correctly)
4. **DAG construction with deduplicated keys** - FIXED

### Remaining Issues ❓

#### Primary Hypothesis: Strict-Corequisite Handling
**Evidence:**
- Lab courses are the main problem across all 4 failing files
- Lab courses typically use the "Strict-Corequisites" column (column 7)
- Current code parses "Corequisites" (column 6) but may not handle "Strict-Corequisites" (column 7)

**Check:**
```bash
# Do we parse strict-corequisites at all?
grep -r "strict" src/core/planner/csv_parser.rs
# Answer: Likely NO or not correctly
```

**Theory:**
- Strict corequisites should create STRONGER dependencies than regular corequisites
- Regular corequisite: courses must be taken together OR one after the other
- Strict corequisite: courses MUST be taken together (same semester)
- For metrics calculation, strict corequisites might need to be treated like prerequisites

#### Secondary Hypothesis: Corequisite Edge Direction
**Evidence:**
- Michigan has HIGHER metrics than reference (opposite of other files)
- Suggests we might be creating bidirectional edges for corequisites when we should create unidirectional

**Theory:**
- When course A has B as corequisite, we might be adding:
  - A → B (correct)
  - B → A (incorrect? or correct but reference doesn't?)
- Need to check DAG.add_corequisite() implementation

---

## Recommended Actions for Tomorrow

### Priority 1: Strict-Corequisite Implementation ⭐⭐⭐
1. Check if CSV parser reads "Strict-Corequisites" column at all
2. Verify how strict-corequisites should differ from regular corequisites
3. Implement strict-corequisite parsing if missing
4. Test with Kennesaw course 10 (CSE1321L) - should have course 9 as strict-coreq

### Priority 2: Lab Course Verification ⭐⭐
1. For each failing file, identify all lab courses
2. Check their corequisite and strict-corequisite fields in CSV
3. Verify they're being parsed and added to DAG correctly
4. Compare with reference files to see expected prerequisite structure

### Priority 3: Corequisite Edge Investigation ⭐
1. Review `DAG.add_corequisite()` implementation
2. Check if corequisites create bidirectional edges
3. Verify corequisite handling in metrics calculation
4. Test Michigan file separately to understand why it's over-counting

### Priority 4: Kennesaw Deep Dive ⭐
1. Kennesaw has the most errors (24 courses)
2. Course 10 (CSE1321L) is the key - fix this and many others may resolve
3. Manual trace:
   - What are course 10's relationships in CSV?
   - What edges does course 10 have in the DAG?
   - What should course 10's metrics be and why?

---

## Success Metrics

**Current Status:**
- Files with perfect metrics: 5/9 (56%)
- Total courses with correct metrics: ~290/360 (80%)
- Critical failures: 1 (Kennesaw)

**Target Status (Tomorrow):**
- Files with perfect metrics: 8/9 or 9/9 (89-100%)
- Fix all lab course relationships
- Understand Michigan's over-counting issue

---

## Technical Notes

### Files to Examine
```
src/core/planner/csv_parser.rs - Lines ~120-135 (corequisite parsing)
src/core/models/dag.rs - add_corequisite() method
src/core/models/school.rs - build_dag() corequisite edges
samples/plans/Kennesaw_State_University_CS.csv - Course 10 strict-coreq field
```

### Test Cases to Create
1. Simple lab + lecture with strict-corequisite
2. XXXX duplicate courses with corequisites
3. Chain of courses where one has lab corequisite

### Questions to Answer
1. Should strict-corequisites be treated as prerequisites for metrics?
2. Are corequisite edges directional or bidirectional?
3. Do lab courses contribute to blocking factor of their lecture course?

---

## Conclusion

The deduplication work was successful - 5 files are now perfect. The remaining issues are focused on **lab course relationships**, specifically around strict-corequisites. This is a more nuanced problem than simple deduplication and will require careful analysis of how corequisite types should influence the prerequisite DAG for metrics calculation.

**Estimated time to fix:** 2-4 hours
- 1 hour: Investigate strict-corequisite column and current parsing
- 1 hour: Implement correct strict-corequisite handling
- 1-2 hours: Debug and test remaining edge cases
