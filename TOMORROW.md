# Quick Start for Tomorrow

## The Problem
**Strict-Corequisites (column 7) are NOT being parsed at all.**

Example:
```
Course 9: CSE1321 (lecture) has "10" in Strict-Corequisites field
Course 10: CSE1321L (lab) is isolated with delay=1, blocking=0
Expected: Course 10 should have delay=7, blocking=18
```

## The Fix (30 minutes)

### 1. Add Strict-Corequisite Parsing
File: `src/core/planner/csv_parser.rs` around line 127

Add after corequisite parsing:
```rust
// Parse strict-corequisites
if let Some(strict_coreq_str) = get_field(line, "Strict-Corequisites", &headers) {
    if !strict_coreq_str.trim().is_empty() {
        add_corequisites_with_mapping(course, &strict_coreq_str, &course_id_to_storage_key);
    }
}
```

### 2. Test
```bash
cargo run --release -- planner samples/plans/Kennesaw_State_University_CS.csv
python3 /tmp/compare_metrics.py
```

### 3. Expected Result
- Kennesaw course 10 should now have delay=7, blocking=18
- 24 wrong courses should drop to ~0-5
- Target: 8-9 out of 9 files perfect

## Current Status
- ✅ 5/9 files PERFECT
- ❌ 4/9 files with errors (all due to missing strict-corequisites)

## Files to Check
1. **METRICS_ERROR_REPORT.md** - Detailed analysis of each error
2. **SUMMARY.md** - Full session summary
3. This file - Quick start guide
