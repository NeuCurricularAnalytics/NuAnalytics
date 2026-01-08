# Planner Command

The `planner` command analyzes computer science curricula, computing detailed metrics about course structure and dependencies. It is based on the work of Greg Heilman and [CurricularAnalytics.org](https://curricularanalytics.org/help/metrics). Current planner input matches the expected format from Curricular Analytics, though output files include additional computed metrics.

## Overview

The `planner` command:

- Loads one or more curriculum CSV files
- Builds a directed acyclic graph (DAG) of course dependencies
- Computes curriculum metrics for each course
- Exports results to CSV format with computed metrics

## Basic Usage

### Analyze a Single Curriculum

```bash
nuanalytics planner path/to/curriculum.csv
```

This will:
1. Parse the curriculum CSV
2. Compute all metrics
3. Export results to the default output directory (configured via `config set out_dir`)

### Analyze Multiple Curricula

```bash
nuanalytics planner curriculum1.csv curriculum2.csv curriculum3.csv
```

### Specify Output File(s)

```bash
# Single input, single output
nuanalytics planner input.csv -o result.csv

# Multiple inputs with corresponding outputs
nuanalytics planner input1.csv input2.csv -o output1.csv output2.csv
```

When using `-o` or `--output`:
- If you provide one input file, you can provide one output file
- If you provide N input files, you must provide exactly N output files (1:1 mapping)
- Output paths can be absolute or relative

## Input File Format

Curriculum CSV files have a specific structure with metadata and course data sections.

### Metadata Section

The file begins with curriculum metadata:

```
Curriculum,My_University_BS_CS,,,,,,,,,
Institution,"University of My State",,,,,,,,,
Degree Type,"BS",,,,,,,,,
System Type,"semester",,,,,,,,,
CIP,"11.0701",,,,,,,,,
```

**Metadata Fields:**

- `Curriculum` - Name/identifier for the curriculum
- `Institution` - University or institution name
- `Degree Type` - Degree level (BS, BA, MS, etc.)
- `Year` (optional) - Academic year of the curriculum
- `System Type` - Academic system ("semester" or "quarter")
- `CIP` - Classification of Instructional Programs code

### Courses Section

After metadata, the file contains course data:

```
Courses

Course ID,Course Name,Prefix,Number,Prerequisites,Corequisites,Strict-Corequisites,Credit Hours,Institution,Canonical Name
1,"Introduction to CS","CS","110",,,,3,
2,"Data Structures","CS","210","1",,,4,
3,"Algorithms","CS","310","2",,,4,
...
```

**Course Fields:**

- `Course ID` - Unique numeric identifier for the course
- `Course Name` - Full course name
- `Prefix` - Department prefix (CS, MATH, PHYS, etc.)
- `Number` - Course number
- `Prerequisites` - Semicolon-separated list of prerequisite Course IDs
- `Corequisites` - Semicolon-separated list of corequisite Course IDs
- `Strict-Corequisites` - Corequisites that must be taken in the same term
- `Credit Hours` - Number of credit hours
- `Institution` - Optional institution override
- `Canonical Name` - Optional standardized course name

### Example Curriculum File

```
Curriculum,State_University_CS,,,,,,,,,
Institution,"State University",,,,,,,,,
Degree Type,"BS",,,,,,,,,
System Type,"semester",,,,,,,,,
CIP,"11.0701",,,,,,,,,
Courses

Course ID,Course Name,Prefix,Number,Prerequisites,Corequisites,Strict-Corequisites,Credit Hours,Institution,Canonical Name
1,"Intro to Computer Science","CS","101",,,,3,
2,"Discrete Math","MATH","150",,,,4,
3,"Calculus I","MATH","160",,,,4,
4,"Data Structures","CS","201","1;2",,,4,
5,"Linear Algebra","MATH","250","3",,,4,
6,"Algorithms","CS","301","4;5",,,4,
7,"Database Systems","CS","350","4",,,4,
8,"Systems Programming","CS","310","4",,,4,
9,"Capstone Project","CS","490","6;7;8",,,3,
```

## Output File Format

The planner generates a CSV file with computed metrics for each course:

### Output Structure

```
Curriculum,State_University_CS,,,,,,,,,
Institution,"State University",,,,,,,,,
Degree Type,"BS",,,,,,,,,
Year,"2023",,,,,,,,,
System Type,"semester",,,,,,,,,
CIP,"11.0701",,,,,,,,,
Total Structural Complexity,47.5
Longest Delay,6
Highest Centrality Course,"Algorithms",0.45
Courses

Course ID,Course Name,Prefix,Number,Prerequisites,Corequisites,Strict-Corequisites,Credit Hours,Institution,Canonical Name,Complexity,Blocking,Delay,Centrality
1,"Intro to CS","CS","101",,,,3,,"",0.7,3,1,0
2,"Discrete Math","MATH","150",,,,4,,"",0.7,2,2,0
...
```

### Metrics Explained

Each course gets four computed metrics:

- **Complexity** - Measure of how complex the course makes the curriculum (0-20+)
  - Based on course dependencies and position in dependency graph
  - Higher values indicate courses with more downstream impact
  - For quarter-based systems, automatically scaled by 2/3 to account for shorter terms

- **Blocking** - Number of courses that have this course as a prerequisite
  - Direct blocking count (not transitive)
  - Shows how many other courses depend on this one

- **Delay** - Longest path from this course to a course with no prerequisites
  - Measured in terms (semesters or quarters)
  - Shows how far into the curriculum this course is located

- **Centrality** - Linear combination of all paths through this course
  - Indicates how central the course is in the curriculum network
  - Higher values mean the course is important to many other courses


## Command Examples

### Simple Analysis

```bash
# Analyze with default output location, creates a file named out_dir/my_curriculum_w_metrics.csv
nuanalytics planner my_curriculum.csv
```

### Custom Output Location

```bash
# Save to specific file
nuanalytics planner curriculum.csv -o analysis_results.csv

# Save to specific directory using config
nuanalytics config set out_dir /home/user/analysis
nuanalytics planner curriculum.csv
```

### Batch Processing

```bash
# Analyze three curricula at once, each with its own output file (must match count)
nuanalytics planner cs_degree.csv math_degree.csv physics_degree.csv \
  -o cs_metrics.csv math_metrics.csv physics_metrics.csv
```

more commonly

```bash
#  Creates a file for every plan in glob expansion in the out_dir location
nuanalytics planner directory_with_plans/*.csv
```

## Report Generation

In addition to CSV metrics, the planner can generate visual reports in HTML, PDF, or Markdown format.

### Default Behavior

By default, running `nuanalytics planner` generates **both**:
- CSV metrics file in the metrics directory
- HTML report in the reports directory

```bash
# Generates both CSV and HTML report
nuanalytics planner curriculum.csv
```

### Report Formats

#### HTML Report (default)

```bash
nuanalytics planner curriculum.csv --report-format html
```

HTML reports include:
- Term-by-term course schedule with credit totals
- Visual dependency graph with prerequisite/corequisite lines
- Color-coded complexity badges
- Detailed metrics table
- Summary statistics

#### PDF Report

```bash
nuanalytics planner curriculum.csv --report-format pdf
```

PDF reports are generated by converting HTML to PDF using headless Chrome/Chromium. Requires Chrome or Chromium to be installed.

To specify a custom PDF converter:

```bash
nuanalytics planner curriculum.csv --report-format pdf --pdf-converter /path/to/chrome
```

#### Markdown Report

```bash
nuanalytics planner curriculum.csv --report-format md
```

Generates a text-based report suitable for documentation systems.

### Output Control

```bash
# Generate only CSV (no report)
nuanalytics planner curriculum.csv --no-report

# Generate only report (no CSV)
nuanalytics planner curriculum.csv --no-csv

# Specify custom output directories
nuanalytics planner curriculum.csv --metrics-dir ./metrics --report-dir ./reports

# Specify exact output file (format inferred from extension)
nuanalytics planner curriculum.csv -o curriculum_report.pdf
```

### Term Scheduling

Reports include automatic term scheduling. Control credit targets per term:

```bash
# Default: 15 credits per semester
nuanalytics planner curriculum.csv --term-credits 16
```

The scheduler:
1. Groups corequisites into the same term
2. Respects prerequisite ordering
3. Balances credit hours across terms
4. Places chain-starting courses early

### With Logging

```bash
# Enable debug logging to see detailed analysis progress
nuanalytics planner curriculum.csv --debug

# Log to a file for later review
nuanalytics planner curriculum.csv --log-file analysis.log
```

## Workflow: Analyzing a New Curriculum

1. **Prepare your curriculum CSV** following the format described in "Input File Format"

2. **Run the planner**:
   ```bash
   nuanalytics planner your_curriculum.csv -o your_curriculum_metrics.csv
   ```

3. **Review the output**:
   ```bash
   # Check the metrics
   cat your_curriculum_metrics.csv
   ```

4. **Analyze the results**:
   - Look for high complexity courses (might be bottlenecks)
   - Check blocking counts (which courses gate access to others?)
   - Review delay values (how is the curriculum sequenced?)
   - Compare centrality scores (which are the key courses?)

5. **Iterate**:
   - Modify the curriculum structure in your CSV if desired
   - Re-run the planner to see how metrics change

## Troubleshooting

### File Not Found

```
Error: Failed to parse plan: No such file or directory
```

**Solution**: Verify the input file path is correct and the file exists.

### Invalid CSV Format

```
Error: Failed to parse plan: Invalid CSV format
```

**Solution**:
- Check that your CSV has the correct metadata section
- Verify all required course columns are present
- Ensure Course IDs are unique
- Check that prerequisite references use valid Course IDs

### Circular Dependencies

```
Error: Cycle detected in prerequisites
```

**Solution**: Verify your course prerequisites don't form a circular dependency. A course cannot (directly or indirectly) require itself as a prerequisite.

### Output Permission Denied

```
Error: Failed to write output file: Permission denied
```

**Solution**: Verify you have write permission to the output directory:
```bash
chmod u+w /path/to/output/directory
```
