# Degree Command

The `degree` command validates and analyzes degree program definitions from YAML files. It checks course prerequisites, requirement structures, and provides detailed audit reports about course dependencies.

## Overview

The `degree` command:

- Loads degree program definitions from YAML files
- Validates course data and prerequisite structures
- Detects circular dependencies and invalid references
- Analyzes prerequisite chains and course complexity
- Generates comprehensive audit reports
- Visualizes course prerequisite graphs

## Basic Usage

### Validate a Degree Program

```bash
nuanalytics degree path/to/degree.yaml
```

This performs basic validation including:
- YAML syntax and structure validation
- Course reference validation (courses mentioned in prerequisites exist)
- Circular prerequisite detection
- Requirement structure validation

### Run a Full Audit

```bash
nuanalytics degree --audit path/to/degree.yaml
```

A full audit includes validation plus:
- Identification of upper-level courses without prerequisites
- Analysis of prerequisite chain depth
- Courses with complex prerequisite structures
- Subject-area filtering for degree-relevant courses

### Print Prerequisite Graph

```bash
nuanalytics degree --print-graph path/to/degree.yaml
```

Displays the prerequisite graph structure showing:
- All courses in the degree
- Prerequisite relationships (AND/OR)
- Expanded prerequisite options

### Combined Operations

```bash
# Run validation and print graph
nuanalytics degree --validate --print-graph path/to/degree.yaml

# Run audit and print graph
nuanalytics degree --audit --print-graph path/to/degree.yaml
```

## Input File Format

Degree programs are defined in YAML files with three main sections:

### Degree Metadata

```yaml
degree:
  name: "Bachelor of Science in Computer Science"
  abbreviation: "BS CS"
  institution: "University Name"
  department: "Khoury College of Computer Sciences"
  credits_required: 128
  catalog_year: "2024-2025"
```

### Course Definitions

```yaml
courses:
  CS101:
    name: "Introduction to Computer Science"
    credits: 4
    prerequisites_raw: ""

  CS220:
    name: "Data Structures"
    credits: 4
    prerequisites_raw: "CS101[C]"

  CS320:
    name: "Algorithms"
    credits: 4
    prerequisites_raw: "(CS220[C] & CS165[C]) & (MATH155[C] | MATH156[C])"
```

**Course Fields:**

- `name` - Full course name
- `credits` - Number of credit hours
- `prerequisites_raw` - Prerequisite expression (see syntax below)
- `corequisites_raw` (optional) - Corequisite expression
- `strict_corequisites_raw` (optional) - Must be taken in same term

### Prerequisite Expression Syntax

Prerequisites use a logical expression syntax:

- `&` - AND operator (all courses required)
- `|` - OR operator (choose one)
- `()` - Grouping for precedence
- `[C]` - Grade requirement suffix (e.g., "C" grade or better)

**Examples:**

```yaml
# Single prerequisite
prerequisites_raw: "CS101[C]"

# Multiple prerequisites (AND)
prerequisites_raw: "CS101[C] & MATH156[C]"

# Alternative prerequisites (OR)
prerequisites_raw: "MATH124[C] | MATH127[C]"

# Complex expression
prerequisites_raw: "(CS220[C] & CS165[C]) & (MATH155[C] | MATH156[C] | MATH160[C])"
```

The expression `(CS220 & CS165) & (MATH155 | MATH156 | MATH160)` means:
- BOTH CS220 AND CS165 are required
- AND one of MATH155, MATH156, or MATH160

### Requirements Section

```yaml
requirements:
  core_cs:
    name: "Computer Science Core"
    required_courses:
      - CS101
      - CS220
      - CS320
    min_credits: 16

  math_foundation:
    name: "Mathematics Foundation"
    required_courses:
      - MATH156
      - MATH160
    min_credits: 8

  cs_electives:
    name: "CS Electives"
    choose: 3
    from:
      - CS425
      - CS430
      - CS445
      - CS460
    min_credits: 12
```

**Requirement Fields:**

- `name` - Descriptive name for the requirement
- `required_courses` - List of specific courses required
- `choose` (optional) - Number of courses to choose from list
- `from` (optional) - List of courses to choose from
- `min_credits` - Minimum credits for this requirement

### Example Degree File

```yaml
degree:
  name: "Bachelor of Science in Computer Science"
  abbreviation: "BS CS"
  institution: "Northeastern University"
  credits_required: 128
  catalog_year: "2024-2025"

courses:
  CS101:
    name: "Fundamentals of Computer Science"
    credits: 4
    prerequisites_raw: ""

  MATH156:
    name: "Calculus for Scientists/Engineers I"
    credits: 4
    prerequisites_raw: ""

  CS220:
    name: "Discrete Structures"
    credits: 4
    prerequisites_raw: "CS101[C]"

  CS320:
    name: "Introduction to Algorithms"
    credits: 4
    prerequisites_raw: "CS220[C] & MATH156[C]"

requirements:
  core:
    name: "CS Core"
    required_courses:
      - CS101
      - CS220
      - CS320
    min_credits: 12
```

## Validation Output

Basic validation displays any errors or warnings:

```
=== Validation Report ===

✓ No circular dependencies detected
✓ All course references are valid
✓ All requirements reference existing courses

Validation: PASSED
```

If issues are found:

```
=== Validation Report ===

✗ Error: Circular dependency detected: CS101 → CS220 → CS101
✗ Warning: Course CS999 referenced in requirements does not exist
✗ Warning: Course CS320 has prerequisite CS999 which doesn't exist

Validation: FAILED (2 errors, 1 warning)
```

## Audit Output

The audit report provides detailed analysis:

```
=== Degree Audit Report ===

Degree: Bachelor of Science in Computer Science
Institution: Northeastern University
Catalog Year: 2024-2025
Total Courses: 45
Total Credits Required: 128

=== Validation Results ===
✓ No circular dependencies detected
✓ All course references valid

=== Upper-Level Courses Without Prerequisites ===

The following courses are 300-level or above but have no prerequisites:
  • CS350 - Introduction to Databases
  • CS425 - Software Engineering
  • MATH350 - Abstract Algebra

=== Courses with Deep Prerequisite Chains ===

Courses requiring multiple prerequisite chains (threshold: 3+):

  CS425 - Software Engineering (3 chains)
    Chain 1: CS220 → CS101
    Chain 2: CS320 → CS220 → CS101 & CS320 → MATH156
    Chain 3: CS165 → CS162 → CS150B

  CS460 - Database Systems (2 chains)
    Chain 1: CS320 → CS220 → CS101
    Chain 2: CS320 → MATH156

=== Summary ===
  Total courses analyzed: 45
  Courses in requirements: 38
  Upper-level without prereqs: 3
  Courses with complex chains: 2
```

### Audit Configuration

The audit behavior can be configured:

```bash
# Set the prerequisite chain threshold (default: 3)
nuanalytics config set prerequisite_chain_threshold 4

# View current threshold
nuanalytics config get prerequisite_chain_threshold
```

## Graph Output

The `--print-graph` option displays prerequisite relationships:

```
=== Course Prerequisite Graph ===

CS101 → (no prerequisites)

CS220 → CS101

CS320 → CS220 & (MATH155 | MATH156 | MATH160)

CS425 → CS320 & CS345 & STAT302A
  Expanded paths:
    - CS320 → CS220 → CS101
    - CS320 → (MATH155 | MATH156 | MATH160)
    - CS345 → CS220 → CS101
    - STAT302A → MATH156

MATH156 → MATH127 | (MATH124 & MATH126)
  Expanded paths:
    Option 1: MATH127
    Option 2: MATH124 & MATH126
```

## Command Examples

### Basic Validation

```bash
# Validate a degree file
nuanalytics degree degrees/cs_2024.yaml

# Validate with verbose output
nuanalytics degree degrees/cs_2024.yaml --verbose
```

### Comprehensive Analysis

```bash
# Full audit report
nuanalytics degree --audit degrees/cs_2024.yaml

# Audit with custom threshold for prerequisite chains
nuanalytics config set prerequisite_chain_threshold 5
nuanalytics degree --audit degrees/cs_2024.yaml
```

### Graph Visualization

```bash
# View prerequisite structure
nuanalytics degree --print-graph degrees/cs_2024.yaml

# Combine audit and graph
nuanalytics degree --audit --print-graph degrees/cs_2024.yaml
```

### Batch Analysis

```bash
# Validate multiple degree files
nuanalytics degree degrees/*.yaml

# Audit all degree files in directory
for file in degrees/*.yaml; do
  echo "Auditing $file"
  nuanalytics degree --audit "$file"
done
```

### With Logging

```bash
# Enable debug logging
nuanalytics degree --audit degrees/cs_2024.yaml --debug

# Log to file
nuanalytics degree --audit degrees/cs_2024.yaml --log-file degree_audit.log
```

## Workflow: Creating a New Degree Program

1. **Create YAML file** with degree metadata, courses, and requirements

2. **Validate structure**:
   ```bash
   nuanalytics degree my_degree.yaml
   ```

3. **Review validation results** and fix any errors

4. **Run comprehensive audit**:
   ```bash
   nuanalytics degree --audit my_degree.yaml
   ```

5. **Analyze audit findings**:
   - Review upper-level courses without prerequisites (might need to add prereqs)
   - Check courses with deep prerequisite chains (might indicate curriculum bottlenecks)
   - Verify subject area filtering is capturing relevant courses

6. **Visualize structure**:
   ```bash
   nuanalytics degree --print-graph my_degree.yaml
   ```

7. **Iterate**: Refine the degree structure based on findings and rerun audit

## Troubleshooting

### YAML Syntax Errors

```
Error: Failed to parse YAML: invalid type at line 45
```

**Solution**: Use a YAML validator to check syntax. Common issues:
- Incorrect indentation (use spaces, not tabs)
- Missing colons after keys
- Unquoted strings with special characters

### Invalid Course References

```
Error: Course CS999 referenced in CS425 prerequisites does not exist
```

**Solution**: Ensure all courses referenced in prerequisites are defined in the `courses` section.

### Circular Dependencies

```
Error: Circular dependency detected: CS101 → CS220 → CS330 → CS101
```

**Solution**: Review and break the circular prerequisite chain. A course cannot (directly or indirectly) require itself.

### Missing Prerequisites

```
Warning: CS425 (400-level) has no prerequisites
```

**Solution**: Consider if this upper-level course should have prerequisites. If intentional, this is just a warning.

## Configuration

Audit behavior is controlled by configuration settings:

```bash
# View current settings
nuanalytics config get

# Set prerequisite chain threshold (default: 3)
nuanalytics config set prerequisite_chain_threshold 4

# Reset to defaults
nuanalytics config reset
```

Available configuration options:
- `prerequisite_chain_threshold` - Minimum chain depth to report in audit (default: 3)

## Advanced Features

### Subject Area Filtering

The audit automatically filters courses to those relevant to the degree's primary subject area:
- Determined from courses listed in requirements
- Falls back to all courses if subject cannot be determined
- Ensures audit focuses on degree-relevant courses

### Prerequisite Chain Analysis

The audit computes prerequisite chains using:
- **Shortest path** when OR alternatives exist
- **Same-subject preference** when choosing between alternatives
- **Chain merging** to eliminate redundant prerequisites
- **Proper ordering** to show dependency direction

Example: For `CS320` with prerequisites `(CS220 & CS165) & (MATH155 | MATH156)`:
- Chooses shortest path among OR alternatives
- Prefers same subject code (CS over CIS if both options exist)
- Merges overlapping chains
- Orders from foundation courses to advanced courses

## See Also

- [Config Command](config.md) - Manage configuration settings
- [Planner Command](planner.md) - Analyze curriculum metrics
- [Development Guide](../Development.md) - Contributing to NuAnalytics
