//! Static schema documentation content for degree YAML files
//!
//! This module contains the Markdown documentation returned by the `get_degree_schema` tool.

/// Get schema content for a given section
///
/// # Arguments
/// * `section` - One of: "all", "degree", "requirements", "courses", "examples"
///
/// # Returns
/// Markdown-formatted documentation string
pub fn get_schema_content(section: &str) -> String {
    match section.to_lowercase().as_str() {
        "degree" => SCHEMA_DEGREE.to_string(),
        "requirements" => SCHEMA_REQUIREMENTS.to_string(),
        "courses" => SCHEMA_COURSES.to_string(),
        "examples" => SCHEMA_EXAMPLES.to_string(),
        _ => format!(
            "{SCHEMA_OVERVIEW}\n\n{SCHEMA_DEGREE}\n\n{SCHEMA_REQUIREMENTS}\n\n{SCHEMA_COURSES}\n\n{SCHEMA_EXAMPLES}"
        ),
    }
}

/// Overview of the degree YAML schema
pub const SCHEMA_OVERVIEW: &str = r#"# Degree Program YAML Schema

A degree program YAML file defines a complete degree including metadata, requirements, and courses.
The file has three main sections: `degree`, `requirements`, and `courses`.

## File Structure

```yaml
degree:
  # Metadata about the degree program
  
requirements:
  # Named requirements that students must satisfy
  
courses:
  # Course definitions with prerequisites
```
"#;

/// Schema documentation for the degree metadata section
pub const SCHEMA_DEGREE: &str = r#"## Degree Metadata Section

The `degree` section contains metadata about the degree program.

```yaml
degree:
  id: string                    # Unique identifier, e.g., "neu-cs-bscs-2025"
  institution: string           # Institution name
  program: string               # Full degree name
  catalog_year: string          # e.g., "2024-2025"
  source_url: string | null     # Link to official catalog
  
  # Credit requirements
  total_credits: integer        # Minimum credits for graduation (e.g., 128)
  upper_division_credits: int | null  # Minimum 300+ level credits
  in_major_credits: int | null  # Minimum credits in major subjects
  
  # GPA requirements
  gpa_minimum: number           # Overall GPA required (e.g., 2.0)
  gpa_major: number | null      # Major GPA if different
  
  # Grade requirements
  grade_minimum: string | null  # Default minimum grade (e.g., "C")
  grade_minimum_note: string | null  # Clarification
  
  # Major subjects
  major_subjects: [string] | null  # e.g., ["CS", "CY", "DS"]
  
  # Double counting
  allow_double_counting: boolean  # Can courses satisfy multiple requirements?
```

### Required Fields
- `id`, `institution`, `program`, `total_credits`, `gpa_minimum`

### Example
```yaml
degree:
  id: neu-khoury-bscs-2025
  institution: Northeastern University
  program: Bachelor of Science in Computer Science
  catalog_year: "2025-2026"
  total_credits: 134
  gpa_minimum: 2.0
  gpa_major: 2.0
  grade_minimum: "C"
  major_subjects: [CS, CY, DS, IS]
  allow_double_counting: false
```
"#;

/// Schema documentation for the requirements section
pub const SCHEMA_REQUIREMENTS: &str = r#"## Requirements Section

Requirements define what students must complete. Three types are supported:

| Type     | Use Case                          | Key Fields          |
|----------|-----------------------------------|---------------------|
| all      | Must complete ALL listed courses  | courses             |
| select   | Choose N courses/credits          | from, count/credits |
| one_of   | Choose one path (mutually exclusive) | options          |

### Type: all - Complete All Courses

```yaml
requirements:
  cs_fundamentals:
    name: Computer Science Fundamentals
    type: all
    category: major
    courses:
      - CS1000
      - CS2500
      - CS2510
```

### Type: select - Choose from Options

```yaml
requirements:
  cs_electives:
    name: CS Electives
    type: select
    category: major
    from:
      courses: [CS3500, CS3650, CS3700, CS3800]
      # OR use pattern matching:
      # pattern: "CS:3000+"
    count: 2        # Number of courses to select
    # OR use credits:
    # credits: 8    # Total credits to reach
```

### Type: one_of - Mutually Exclusive Paths

```yaml
requirements:
  math_track:
    name: Mathematics Track
    type: one_of
    category: supporting
    options:
      - id: calc_track
        name: Calculus Track
        requirements:
          - MATH1341
          - MATH1342
      - id: discrete_track
        name: Discrete Track
        requirements:
          - MATH1365
          - CS1800
```

### Category Values
- `major` - Core major courses
- `supporting` - Supporting/prerequisite courses
- `gen_ed` - General education requirements
- `elective` - Elective courses

### Constraints (Optional)

```yaml
requirements:
  advanced_electives:
    name: Advanced Electives
    type: select
    category: major
    from:
      pattern: "CS:4000+"
    credits: 12
    constraints:
      min_grade: "B"
      min_upper_division: 8
      max_from_subject:
        CS: 8
```
"#;

/// Schema documentation for the courses section
pub const SCHEMA_COURSES: &str = r#"## Courses Section

Define all courses referenced in requirements.

```yaml
courses:
  CS2500:
    title: Fundamentals of Computer Science 1
    credits: 4
    prerequisites_raw: ""  # No prerequisites
    
  CS2510:
    title: Fundamentals of Computer Science 2
    credits: 4
    prerequisites_raw: "CS2500"
    
  CS3500:
    title: Object-Oriented Design
    credits: 4
    prerequisites_raw: "CS2510 & CS2810"
    
  CS4500:
    title: Software Development
    credits: 4
    prerequisites_raw: "(CS3500 | CS3800) & CS3000"
```

### Course Fields

| Field | Required | Description |
|-------|----------|-------------|
| title | Yes | Course title |
| credits | Yes | Credit hours (integer or decimal) |
| prerequisites_raw | No | Prerequisite expression |
| corequisites | No | Corequisite courses |
| typically_offered | No | When offered: "fall", "spring", "both", "varies" |
| gen_ed_attributes | No | Gen-ed categories satisfied |
| cross_listed_as | No | Equivalent courses in other departments |
| repeatable | No | Can course be repeated? |
| max_repeat_credits | No | Max credits if repeatable |

### Prerequisite Expression Syntax

- `&` = AND (all required)
- `|` = OR (any one)
- `()` = Grouping
- `[grade]` = Grade requirement (e.g., `CS2500[B]`)

Examples:
- `CS2500` - Single prerequisite
- `CS2500 & CS2510` - Both required
- `CS2500 | CS2510` - Either one
- `(CS2500 & CS2510) | CS3000` - Complex logic
- `MATH1341[C] & CS2500[B]` - With grade requirements

### Course Key Format

Course keys are typically: `{SUBJECT}{NUMBER}` (no space)
- ✓ `CS2500`, `MATH1341`, `PHYS1151`
- ✗ `CS 2500`, `cs2500`
"#;

/// Example degree YAML and common patterns
pub const SCHEMA_EXAMPLES: &str = r#"## Complete Example

```yaml
degree:
  id: example-bscs-2025
  institution: Example University
  program: Bachelor of Science in Computer Science
  catalog_year: "2025-2026"
  total_credits: 120
  gpa_minimum: 2.0
  major_subjects: [CS, MATH]
  allow_double_counting: false

requirements:
  intro_sequence:
    name: Introductory Sequence
    type: all
    category: major
    courses:
      - CS101
      - CS102
      - CS201
      
  math_requirements:
    name: Mathematics
    type: all
    category: supporting
    courses:
      - MATH151
      - MATH152
      
  cs_electives:
    name: CS Electives
    type: select
    category: major
    from:
      courses: [CS301, CS302, CS401, CS402]
    count: 2

courses:
  CS101:
    title: Introduction to Programming
    credits: 4
    prerequisites_raw: ""
    
  CS102:
    title: Data Structures
    credits: 4
    prerequisites_raw: "CS101"
    
  CS201:
    title: Algorithms
    credits: 4
    prerequisites_raw: "CS102 & MATH151"
    
  CS301:
    title: Operating Systems
    credits: 4
    prerequisites_raw: "CS201"
    
  CS302:
    title: Databases
    credits: 4
    prerequisites_raw: "CS201"
    
  CS401:
    title: Machine Learning
    credits: 4
    prerequisites_raw: "CS201 & MATH152"
    
  CS402:
    title: Computer Networks
    credits: 4
    prerequisites_raw: "CS201"
    
  MATH151:
    title: Calculus I
    credits: 4
    prerequisites_raw: ""
    
  MATH152:
    title: Calculus II
    credits: 4
    prerequisites_raw: "MATH151"
```

## Common Patterns

### Lecture + Lab Bundles
Use bundle syntax `[course1, course2]` in requirements:
```yaml
courses:
  - "[CHEM1211, CHEM1211L]"  # Both must be taken together
```

### Equivalent/Cross-listed Courses
Use equivalent syntax `{course1, course2}` in requirements:
```yaml
courses:
  - "{CS201, PHIL201}"  # Either satisfies requirement
```

Or define cross-listing in course:
```yaml
CS201:
  title: Ethics in Computing
  credits: 4
  cross_listed_as: [PHIL201]
```
"#;
