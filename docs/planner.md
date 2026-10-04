# Planner Command

`nuanalytics planner` analyzes a curriculum given as a Curricular Analytics CSV: it
builds the prerequisite graph, computes each course's metrics, and writes the curriculum
back out with the metrics added, plus a report with a term-by-term schedule. The metrics
follow Greg Heileman's work and [CurricularAnalytics.org](https://curricularanalytics.org/help/metrics).

For degree programs with choices — electives, alternatives, requirement blocks — use
[`degree analyze`](degree.md) instead; the planner analyzes one fixed list of courses.

## Usage

```bash
nuanalytics planner path/to/curriculum.csv                 # CSV metrics + HTML report
nuanalytics planner a.csv b.csv c.csv                      # several curricula
nuanalytics planner plans/*.csv                            # every file a glob matches
nuanalytics planner input.csv -o result.csv                # one named output
nuanalytics planner a.csv b.csv -o a_out.csv b_out.csv     # one output per input, in order
```

Without `-o`, each input `name.csv` writes `name_w_metrics.csv` to the metrics directory
and `name_report.html` to the reports directory (`[paths]` in the configuration, or
`--metrics-dir` and `--report-dir`). With `-o`, give one output per input; the extension
chooses the output — `.csv` for metrics only, `.html`, `.md` or `.pdf` for a report only.

## Input format

A metadata block, a line reading `Courses`, then the course table:

```
Curriculum,State_University_CS,,,,,,,,,
Institution,"State University",,,,,,,,,
Degree Type,"BS",,,,,,,,,
System Type,"semester",,,,,,,,,
CIP,"11.0701",,,,,,,,,
Courses
Course ID,Course Name,Prefix,Number,Prerequisites,Corequisites,Strict-Corequisites,Credit Hours,Institution,Canonical Name
1,"Intro to Computer Science","CS","101",,,,3,,
2,"Discrete Math","MATH","150",,,,4,,
3,"Calculus I","MATH","160",,,,4,,
4,"Data Structures","CS","201","1;2",,,4,,
5,"Linear Algebra","MATH","250","3",,,4,,
6,"Algorithms","CS","301","4;5",,,4,,
7,"Database Systems","CS","350","4",,,4,,
8,"Systems Programming","CS","310","4",,,4,,
9,"Capstone Project","CS","490","6;7;8",,,3,,
```

Blank lines between `Courses` and the header row are skipped; the header is the first
non-blank line after `Courses`. A file from which no course can be read is an error, never
an empty analysis.

Metadata rows:

| Row | Meaning |
|---|---|
| `Curriculum` | The curriculum's name. |
| `Institution` | The institution. |
| `Degree Type` | BS, BA, MS, … |
| `Year` | The catalog year (optional). |
| `System Type` | `semester` or `quarter`. Quarter-system complexity is scaled by 2/3. |
| `CIP` | The program's Classification of Instructional Programs code. |

Course columns:

| Column | Meaning |
|---|---|
| `Course ID` | A number unique within the file; the requisite columns refer to it. |
| `Course Name`, `Prefix`, `Number` | The course. |
| `Prerequisites` | Course IDs that must be passed first, separated by `;`. |
| `Corequisites` | Course IDs to take before or alongside, separated by `;`. |
| `Strict-Corequisites` | Course IDs that must be taken in the same term. |
| `Credit Hours` | Credits. |
| `Institution` | Optional; overrides the file's institution for this course. |
| `Canonical Name` | Optional standardized name. |

## Output

The metrics CSV repeats the input with summary rows and five metric columns added. For
the example above:

```
Curriculum,State_University_CS
Institution,State University
Degree Type,"BS"
System Type,semester
CIP,"11.0701"
Total Structural Complexity,58.0
Longest Delay,4,MATH150->CS201->CS310->CS490
Highest Centrality Course,"CS201",24
Courses
Course ID,Course Name,Prefix,Number,Prerequisites,Corequisites,Strict-Corequisites,Credit Hours,Institution,Canonical Name,Complexity,Blocking,Delay,Centrality,Chain Length
1,Intro to Computer Science,"CS","101","","","",3,"State University","",9.0,5,4,0,1
2,Discrete Math,"MATH","150","","","",4,"State University","",9.0,5,4,0,1
4,Data Structures,"CS","201","1;2","","",4,"State University","",8.0,4,4,24,2
9,Capstone Project,"CS","490","6;7;8","","",3,"State University","",4.0,0,4,0,4
...
```

### The metrics

- **Delay**: the number of courses on the longest prerequisite path through the course.
  Every course here lies on a four-course path, so every delay is 4.
- **Blocking**: the number of courses that cannot be taken until this one is passed —
  all of them downstream, not just the courses that list it directly. Intro to Computer
  Science blocks 5.
- **Complexity**: delay plus blocking (scaled by 2/3 for a quarter system). Total
  structural complexity is the sum over all courses.
- **Centrality**: the total length of the paths from a course with no prerequisites to a
  course nothing depends on that pass through this course. A course at either end of
  every path it is on scores 0.
- **Chain length**: the number of courses in the longest prerequisite chain ending at the
  course, itself included — how deep into the program it sits.

`Longest Delay` names the longest path; `Highest Centrality Course` names the course with
the highest centrality.

## Reports

By default the planner writes both the metrics CSV and an HTML report. The HTML report
has a term-by-term schedule with credit totals, a dependency graph with prerequisite and
corequisite lines, colour-coded complexity, and the metrics table.

```bash
nuanalytics planner curriculum.csv --report-format pdf     # PDF, via headless Chrome or Chromium
nuanalytics planner curriculum.csv --report-format md      # Markdown
nuanalytics planner curriculum.csv --report-format pdf --pdf-converter /path/to/chrome
nuanalytics planner curriculum.csv --no-report             # metrics CSV only
nuanalytics planner curriculum.csv --no-csv                # report only
```

The schedule places corequisites in the same term, keeps prerequisites in earlier terms,
starts long chains early, and balances credits against a target per term — 15 by
default, or `--term-credits`:

```bash
nuanalytics planner curriculum.csv --term-credits 16
```

## Logging

Logging flags are global, so they go before `planner`:

```bash
nuanalytics --debug planner curriculum.csv
nuanalytics --log-file analysis.log planner curriculum.csv
```

## Troubleshooting

- **`✗ Failed to load missing.csv: No such file or directory`** — the input path is
  wrong.
- **`no courses could be read after the 'Courses' row`** — the line after `Courses` (and
  any blank lines) must be the column header, and each course line must have a `Course ID`,
  `Prefix` and `Number`.
- **A metric looks wrong** — check that each prerequisite refers to the right `Course ID`.
  A prerequisite cycle (a course requiring itself, directly or through others) cannot be
  measured.
