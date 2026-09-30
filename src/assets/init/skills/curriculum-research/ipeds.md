# IPEDS completions: what the figures mean

IPEDS is the U.S. Department of Education's institutional survey. Its completions data
counts the awards each school confers each year, by program (CIP code), award level,
race/ethnicity and gender. The data is annual and the schools report it themselves.

## CIP codes

CIP codes are hierarchical: `11.` is a family and `11.0701` a program. Pass a family as a
prefix with its trailing dot (`cip_prefix="11."`), so that `11` does not also match
`110`.

| code | program |
|---|---|
| `11.` | Computer and Information Sciences (the whole family) |
| `11.0101` | Computer and Information Sciences, General |
| `11.0102` | Artificial Intelligence |
| `11.0103` | Information Technology |
| `11.0701` | Computer Science |
| `11.1003` | Computer and Information Systems Security / Information Assurance |
| `14.0901` | Computer Engineering, General |
| `14.0903` | Computer Software Engineering |
| `30.7001` | Data Science, General |

Schools choose their own codes. The same program can be `11.0701` at one school,
`11.0101` at another, and `14.0903` at a third. For "computing", use the `11.` family
and say so; for one discipline, check what the schools you care about actually use.

**CIP `99` is a grand total, not a program:** each school's sum over all its CIPs.
The typed tools never add it in. In SQL, leave it out of every sum
(`cip_code <> '99'`), or every graduate is counted twice.

## Award levels

Confirm with `get_lookup_codes(table="award_levels")`.

| code | award |
|---|---|
| 3 | Associate's degree |
| 5 | Bachelor's degree |
| 7 | Master's degree |
| 17, 18, 19 | Doctor's degree: research, professional practice, other |
| 2, 4, 20, 21 | Undergraduate certificates of various lengths |
| 6, 8 | Post-baccalaureate and post-master's certificates |

With no `award_level`, every level is counted together. That is rarely what a question
about "graduates" means, so set it.

## Carnegie classes (2021)

| code | class |
|---|---|
| 15 | Doctoral Universities: Highest Research Activity (R1) |
| 16 | Doctoral Universities: Higher Research Activity (R2) |
| 17 | Doctoral/Professional Universities |
| 18–20 | Master's Colleges and Universities (larger, medium, small programs) |
| 21–22 | Baccalaureate Colleges |

Control is 1 public, 2 private not-for-profit, 3 private for-profit.

## Reading the demographics

- **`representation_ratio`** is a group's share of the selected completions divided by
  its share of the baseline. 1.0 is parity; 0.57 means the group earns these degrees at
  57% of the rate its presence among the schools' graduates would predict.
- **The baseline is all completions at the same schools, in every field.** For
  `group_by="total"` it is pooled across the matched schools; otherwise it is each
  school's own. The response carries it as `baseline_completions`, `baseline_total` and
  `baseline_pct` (`baseline_pct` alone in `group_by="cip"` rows). The database holds no
  enrollment data.
- **`major_num`:** 1 is a graduate's first major and 2 the second. The default counts
  both, so a CS–math double major appears under both CIPs. Totals across CIPs then
  exceed the number of people. Use `major_num=1` for one count per graduate.
- **Race categories** are the IPEDS ones, including "Nonresident Alien" (international
  students, counted in no race group) and "Unknown Race/Ethnicity". Say which groups a
  figure leaves out.
- **Small cells.** A school with 12 CS graduates can swing from 0 to 2.0 on one student.
  Put the counts beside every ratio. When ranking schools, set `min_completions`.
- **Year.** The default is the latest year with data for the matched schools. Name the
  year in every answer, and compare years only when both are named.
- **Empty results.** `group_by="cip"` for a school that reports nothing under the CIP you
  asked for returns `nearby_cips_with_data`: the school's largest other programs. A CS
  program filed under `11.0101` is the common case.
