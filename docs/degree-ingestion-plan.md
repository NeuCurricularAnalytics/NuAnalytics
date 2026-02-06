# Degree YAML Ingestion Implementation Plan

## Overview

Add degree YAML parsing and validation without plan extraction. Load degrees via CLI and test the structure. Incremental delivery focused on reading YAML, validating structure, and ensuring proper data representation for future database storage.

## Key Design Principles

- **Degree-centric model**: A degree contains the full curriculum structure (all courses, requirements, prerequisites); plans are paths through that degree.
- **Course identity**: Institution-scoped; course keys use subject + number (e.g., `ICS111`) directly.
- **CSV plan compatibility**: Plans that violate institution-scoping rules remain one-time CSV loads; full degrees require well-formed courses sections.
- **Database readiness**: Each degree and course is a separate struct suitable for independent DB population.
- **Staged delivery**: Each stage is independent; prerequisite parsing deferred to stage 5.

## Stages

### Stage 1: Core degree structs and YAML deserialization
- Create `src/core/degree.rs` with serde-annotated structs:
  - `Degree` (metadata: id, institution, program, catalog_year, credit/GPA/grade requirements, major_subjects, allow_double_counting)
  - `Requirement` (name, type: all|select|one_of, category, courses list, from options for select, options list for one_of, constraints)
  - `RequirementOption` (id, name, nested requirements for one_of paths)
  - `Course` (subject, number, title, credits, prerequisites as raw string, corequisites, typically_offered, gen_ed_attributes, cross_listed_as)
- Add `serde_yaml` to `Cargo.toml` dependencies
- Create `src/core/degree_loader.rs` with `load_degree_from_yaml(path: &Path) -> Result<Degree>`
- Export both modules in `src/lib.rs`
- **Test**: Deserialize sample degrees; validate struct fields populate correctly

### Stage 2: Validation logic
- Add validation methods to `Degree` (e.g., `validate()` → checks course references exist, credit totals, constraints)
- Create `src/core/degree_validator.rs`: validate course existence in `courses:` map, requirement constraints (grades, credits, patterns)
- Integrate validator into `load_degree_from_yaml` error reporting
- **Test**: Validation catches missing course references, constraint violations

### Stage 3: CLI entry point
- Add CLI subcommand `degree <path>` (defaults to load, other subcommands possible) in `src/cli/`
- Wire to call `load_degree_from_yaml` + `validate()`, print degree metadata, requirement summary, course inventory, or validation errors
- Update `docs/design/CLI.md` with new command docs
- **Test**: CLI invocation produces expected output format

### Stage 4: Integration tests
- Add test in `tests/integration.rs` loading sample degrees (e.g., `samples/degrees/uhm-ics-bscs-general.yaml`, `csu-cs-bscs-general.yaml`)
- Assert metadata, requirement structures, and courses section parse and validate correctly
- Test validation catches missing course references, constraint violations
- **Coverage**: At least 2 sample degrees, both valid and error cases

### Stage 5 (future): Prerequisite expression parsing
- Parse prerequisite raw strings into structured AST (AND/OR/NOT nodes)
- Add cyclic dependency detection in DAG builder (similar to existing plan CSV flow)
- Integrate with plan extraction logic when that phase begins

## CSV Plan Compatibility

Current CSV plan loading remains unchanged. Plans are loaded via existing `Curriculum` and `Course` models in core. Degree YAML ingestion adds a parallel pathway that does not interfere with:
- Existing CSV loader in plan loading pipeline
- Metrics computation via DAG and prerequisite analysis
- CLI planner export, report generation, and other CSV-dependent features
- Test suite for CSV loading and metrics

## Files to Create/Modify

### Create
- `src/core/degree/` module directory
- `src/core/degree/mod.rs` - module definition and re-exports
- `src/core/degree/models.rs` - data structures
- `src/core/degree/yaml_parser.rs` - YAML parsing (file and string)
- `src/core/degree_validator.rs` (stage 2)

### Modify
- `Cargo.toml` (add `serde_yaml`)
- `src/core/mod.rs` (export new modules)
- `src/cli/` (add `degree` subcommand)
- `docs/design/CLI.md` (document new command)
- `tests/integration.rs` (add degree loading tests)

---

## Current Status

- [x] Stage 1: Degree structs + YAML deserialization
  - ✓ Created `src/core/degree/` module following project conventions
  - ✓ `models.rs` - Full struct hierarchy for degrees, requirements, courses
  - ✓ `yaml_parser.rs` - YAML parsing with both `parse_degree_yaml(&str)` and `load_degree_from_yaml(path)`
  - ✓ Added `serde_yaml` to `Cargo.toml`
  - ✓ Exported types in `src/core/mod.rs`
  - ✓ Unit tests passing (7 tests for parsing)
  - ✓ Integration tests passing (13 tests loading real YAML files)
  - ✓ All 3 sample YAML files load correctly (UHM, CSU, NEU)
  - ✓ All existing tests still pass (105 lib tests, 26 integration tests)

- [x] Stage 1.5: Unified model integration
  - ✓ Extended `Degree` model with optional YAML fields (id, institution, catalog_year, etc.)
  - ✓ Extended `Course` model with optional YAML fields (prerequisites_raw, typically_offered, etc.)
  - ✓ Added `CreditRange` struct to course model
  - ✓ Added `DegreeMeta.into_degree()` conversion method
  - ✓ Added `YamlCourse.into_course()` conversion method
  - ✓ Re-exported `CreditRange` from models to avoid duplication
  - ✓ Added 6 integration tests for model conversions
  - ✓ Maintained backward compatibility with CSV plan loading

- [ ] Stage 2: Validation logic
- [ ] Stage 3: CLI integration
- [ ] Stage 4: Integration tests (additional coverage)
- [ ] Stage 5: Prerequisite parsing (future)

---

## Stage 1.5: Unified Model Completion Summary

Extended existing models to support both CSV and YAML sources with optional fields:

### Degree Model Extensions (`src/core/models/degree.rs`)
- Added optional fields: `id`, `institution`, `catalog_year`, `source_url`, `total_credits`,
  `upper_division_credits`, `in_major_credits`, `gpa_minimum`, `gpa_major`, `grade_minimum`,
  `grade_minimum_note`, `major_subjects`, `allow_double_counting`
- Added `with_metadata()` constructor for YAML loading
- Added `degree_id()` method (deprecated `id()`)
- Changed `PartialEq, Eq` to `PartialEq` (f32 fields)

### Course Model Extensions (`src/core/models/course.rs`)
- Added `CreditRange` struct
- Added optional fields: `prerequisites_raw`, `typically_offered`, `gen_ed_attributes`,
  `cross_listed_as`, `repeatable`, `max_repeat_credits`, `credit_range`
- Added `actual_credits()` method for variable credits

### Degree Module Updates (`src/core/degree/models.rs`)
- Added `DegreeMeta.into_degree()` → converts to unified `Degree` model
- Added `DegreeMeta.to_degree()` → borrowing version
- Added `YamlCourse.into_course()` → converts to unified `Course` model
- Added `YamlCourse.to_course()` → borrowing version
- Re-exported `CreditRange` from `crate::core::models::course::CreditRange`
- Removed duplicate `CreditRange` definition

### Key Benefits
1. **Single source of truth**: No need to maintain parallel Degree/Course structs
2. **Gradual population**: CSV loading uses basic fields; YAML loading adds optional fields
3. **Metrics compatibility**: Unified Course can flow into existing DAG and metrics pipeline
4. **Future plan extraction**: When generating plans from degrees, courses are already in correct format

---

## Stage 1: Completion Summary

Successfully implemented core degree YAML structures and deserialization:

### Files Created
1. **`src/core/degree/mod.rs`** (~30 lines)
   - Module definition with re-exports
   - Example usage in doc comments

2. **`src/core/degree/models.rs`** (~310 lines)
   - `YamlDegree`: Top-level degree container with metadata, requirements, courses
   - `DegreeMeta`: Degree metadata (id, institution, program, credits, GPA, grades, etc.)
   - `Requirement`: Flexible requirement type supporting all/select/one_of patterns
   - `RequirementType`: Enum for requirement types
   - `FromClause`: Source of courses (explicit list, pattern, groups)
   - `CourseGroup`: Grouped course selection
   - `CreditRange`: Variable credit ranges
   - `RequirementConstraints`: Per-requirement constraints
   - `RequirementOption`: Mutually exclusive option paths
   - `YamlCourse`: Full course definition (subject, number, title, credits, prerequisites, etc.)

3. **`src/core/degree/yaml_parser.rs`** (~200 lines)
   - `parse_degree_yaml(&str)`: Parse from string (supports network/database sources)
   - `load_degree_from_yaml(path)`: Convenience wrapper for file loading
   - `DegreeParseError`: Custom error type with detailed messages
   - Unit tests covering invalid YAML, valid parsing, and file loading

4. **`tests/rs/degree_yaml.rs`** (~150 lines)
   - 8 integration tests loading real sample degrees (UHM, CSU, NEU)
   - Tests verify metadata, requirement types, course parsing
   - Tests validate string parsing (`parse_degree_yaml`) and course fields
   - Tests validate optional fields and derived methods

### Files Modified
1. **`Cargo.toml`**: Added `serde_yaml = "0.9"` dependency
2. **`src/core/mod.rs`**: Exported `degree` module with re-exports
3. **`tests/rs/mod.rs`**: Exported `degree_yaml` test module
4. **`docs/degree-ingestion-plan.md`**: This file, design document

### Key Design Decisions
1. **Module structure**:
   - Follows project convention (`degree/` submodule like `planner/`)
   - Separates models from parsing logic
   - Supports both file and string parsing for network flexibility

2. **Field optionality**:
   - Requirement `name` is optional (nested requirements in `one_of` options may omit it)
   - Course `credits` is optional (courses with variable credit use `credit_range` instead)
   - Many metadata fields are optional with `skip_serializing_if`

3. **Prerequisite representation**:
   - Stored as raw strings (no parsing in stage 1)
   - Deferred to stage 5 for AST parsing
   - Enables flexible format support (both simple and complex expressions)

4. **Course identity**:
   - Simple course key: `{subject}{number}` (e.g., "ICS111")
   - Institution-scoped naturally (stored in parent Degree)
   - No compound keys needed for now

5. **Flexible requirement patterns**:
   - `RequirementType::All`: All listed courses
   - `RequirementType::Select`: N courses/credits from options
   - `RequirementType::OneOf`: Mutually exclusive paths (for alternative tracks)
   - Supports both flat lists and grouped selections

### Test Coverage
- **Unit tests**: 7 tests in yaml_parser + 2 in models
- **Integration tests**: 8 tests across 3 sample degrees
- **All sample YAML files**: UHM, CSU, NEU all load successfully
- **Backward compatibility**: All 103 existing lib tests pass

### Next Steps
1. Stage 2: Add validation logic for course references, cycles, constraints
2. Stage 3: Wire CLI subcommand `degree <path>` with summary output
3. Stage 4: Add more comprehensive integration tests
4. Stage 5: Implement prerequisite expression parsing
