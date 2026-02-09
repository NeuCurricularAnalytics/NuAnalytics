# Degree Analysis Implementation Plan

**Created**: 2026-02-09
**Status**: In Progress
**Goal**: Generate all valid degree plans, compute metrics across plans, and produce comprehensive HTML reports with statistical summaries.

## Overview

When a user runs `nuanalytics degree <yaml_file>`, the system will:

1. Generate all possible valid plans from the degree requirements
2. Stream metrics computation (to handle large plan counts)
3. Aggregate statistics across all plans
4. Identify special plans (shortest, longest, calc-ready)
5. Sample random plans for export
6. Generate HTML report with box plots and statistics
7. Export CSV files for special and sampled plans

## Output Specifications

### HTML Report Contents
- **Box Plots** for:
  - Degree complexity (sum of all course complexities)
  - Longest delay factor
- **Statistics** (min, Q1, median, Q3, max) for degree-level metrics
- **Per-course metrics table** with median/min/max for:
  - Complexity
  - Centrality
  - Delay
  - Blocking factor
- **Special Plans** with full metrics (same as planner report):
  - Calculus Ready (shortest path assuming calc prereqs satisfied)
  - Shortest path to completion (minimum terms)
  - Longest path (maximum terms)

### CSV Output
- Directory: `{metrics_dir}/plans/{degree-id}/`
- Files:
  - `shortest.csv`
  - `longest.csv`
  - `calc-ready.csv`
  - `sample-{n}.csv` (configurable count, default 5)

## Design Decisions

### Plan Generation Strategy
- **Streaming**: Compute metrics and discard plans to manage memory
- **Special plan tracking**: Keep best shortest/longest/calc-ready during streaming
- **Random sampling**: Reservoir sampling for random plan exports
- **Duplicate detection**: Optional `--ignore-duplicates` to skip equivalent combinations

### Requirement Handling
- `type: all` - Fixed courses, no options
- `type: select` - Generate all C(n,k) combinations (or deduplicate if equivalent)
- `type: one_of` - Generate plan for each option

### Electives
- Divide into 3-credit placeholder courses
- Remainder (1-2 credits) as small course
- No prerequisites on placeholders

### Prerequisites
- Default to same subject code when choosing between OR options
- Use shortest prerequisite path

### Statistics
- **CalculationStrategy** pattern for extensibility
- Default: Median
- Also support: Mean
- Future: Weighted strategies for research

---

## Phase 1: Core Infrastructure ✅ IN PROGRESS

**Goal**: Build the foundation for statistics and plan representation

### 1.1 CalculationStrategy Trait
```rust
pub trait CalculationStrategy {
    fn aggregate(&self, values: &[f64]) -> f64;
    fn name(&self) -> &'static str;
}

pub struct MedianStrategy;
pub struct MeanStrategy;
```

### 1.2 Statistics Struct
```rust
pub struct DescriptiveStats {
    pub min: f64,
    pub q1: f64,
    pub median: f64,
    pub q3: f64,
    pub max: f64,
    pub mean: f64,
    pub std_dev: f64,
    pub count: usize,
}
```

### 1.3 Config/CLI Integration
- Add `calc_strategy` to Config (default: "median")
- Add `sample_plan_count` to Config (default: 5)
- CLI flag: `--calc-strategy median|mean`
- CLI flag: `--sample-plans N`

### 1.4 PlanVariant Representation
```rust
pub struct PlanVariant {
    pub courses: Vec<String>,
    pub requirement_choices: HashMap<String, Vec<String>>,
}
```

### Files to Create/Modify
- [ ] `src/core/statistics/mod.rs` - Module root
- [ ] `src/core/statistics/strategy.rs` - CalculationStrategy trait + impls
- [ ] `src/core/statistics/descriptive.rs` - DescriptiveStats + computation
- [ ] `src/core/config.rs` - Add new config options
- [ ] `src/cli/args.rs` - Add CLI arguments

---

## Phase 2: Plan Generation Engine

**Goal**: Full combinatorial plan generation with optimizations

### 2.1 RequirementResolver
- Expand each requirement into concrete course choices
- Handle patterns, groups, course lists
- Track which requirements contribute to "explosion"

### 2.2 PlanGenerator
- Lazy iterator using cartesian product
- Plan count estimation before generation
- Integration with streaming metrics

### 2.3 Duplicate Detection
- Fingerprint by (credits, prereq-set)
- Skip equivalent when `--ignore-duplicates`

### Files to Create
- [ ] `src/core/degree/requirement_resolver.rs`
- [ ] `src/core/degree/plan_generator.rs`

---

## Phase 3: Metrics Aggregation

**Goal**: Compute and aggregate metrics across all plans via streaming

### 3.1 StreamingAggregator
- Online algorithms for statistics (Welford's for mean/variance)
- Track min/max during streaming
- Collect values for quartile computation (or use approximation)

### 3.2 Per-Course Aggregation
- Separate collector per course
- Merge after streaming completes

### 3.3 Parallel Processing
- Use `rayon` for parallel plan processing
- Thread-safe aggregator with atomics/mutex

### Files to Create
- [ ] `src/core/statistics/aggregator.rs`
- [ ] `src/core/statistics/streaming.rs`

---

## Phase 4: Special Plan Selection

**Goal**: Identify and retain shortest/longest/calc-ready plans

### 4.1 Plan Scoring
- `shortest_terms()` - Use TermScheduler, minimize terms
- `longest_terms()` - Maximize terms
- `calc_ready_shortest()` - Shortest with calc prereqs satisfied

### 4.2 Reservoir Sampling
- Keep N random plans during streaming
- Algorithm R for uniform sampling

### 4.3 Best Plan Tracking
- Track current best for each category
- Update atomically during parallel processing

### Files to Create/Modify
- [ ] `src/core/degree/plan_selector.rs`
- [ ] `src/core/degree/plan_sampler.rs`

---

## Phase 5: Report Generation

**Goal**: HTML report with visualizations + CSV exports

### 5.1 Box Plot Generation
- Use `plotters` crate for pure Rust SVG generation
- Generate for degree complexity and max delay

### 5.2 HTML Report Template
- Extend existing HTML reporter infrastructure
- Embed SVG box plots
- Per-course statistics table
- Special plan sections

### 5.3 CSV Export
- Reuse existing metrics_export format
- Output special plans + sampled plans

### Files to Create/Modify
- [ ] `src/core/statistics/box_plot.rs`
- [ ] `src/core/report/degree_report.rs`
- [ ] Update `src/core/report/formats/html.rs`

### Dependencies to Add
- `plotters` - SVG chart generation

---

## Phase 6: CLI Integration

**Goal**: Wire everything into `nuanalytics degree` command

### 6.1 New Arguments
```
--calc-strategy <median|mean>   Calculation strategy for aggregate metrics
--ignore-duplicates             Skip equivalent plan combinations
--max-plans <N>                 Safety cap on plan generation
--sample-plans <N>              Number of random plans to export (default: 5)
--export-all-plans              Export all plans (warning: may be large)
```

### 6.2 Progress Reporting
- Plan count estimation
- Progress bar for large generations
- Final summary

### Files to Modify
- [ ] `src/cli/args.rs`
- [ ] `src/cli/commands/degree.rs`

---

## Configuration Options

```toml
[degree_analysis]
calc_strategy = "median"        # median | mean
sample_plan_count = 5           # random plans to export
max_plans = 1000000             # safety cap
ignore_duplicates = false       # skip equivalent combinations
```

---

## Future Considerations

- **Course inclusion/exclusion**: "All plans must include X" or "exclude Y"
- **Weighted strategies**: Research-focused calculation methods
- **Incremental analysis**: Cache intermediate results
- **Distributed processing**: For extremely large plan spaces

---

## Progress Tracking

| Phase | Status | Started | Completed |
|-------|--------|---------|-----------|
| 1. Core Infrastructure | 🟡 In Progress | 2026-02-09 | |
| 2. Plan Generation | ⬜ Not Started | | |
| 3. Metrics Aggregation | ⬜ Not Started | | |
| 4. Special Plan Selection | ⬜ Not Started | | |
| 5. Report Generation | ⬜ Not Started | | |
| 6. CLI Integration | ⬜ Not Started | | |
