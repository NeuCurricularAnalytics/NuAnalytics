# Samples

| Directory | Contents |
|---|---|
| `plans/` | Curriculum CSVs in the Curricular Analytics format, hand-built for that tool — input for `nuanalytics planner`. |
| `planner-output/correct/` | The same curricula with metrics from [curricularanalytics.org](https://curricularanalytics.org/), the reference the planner's metrics are checked against. |
| `planner-output/reports/` | Example planner reports (HTML, Markdown, PDF). |
| `degrees/` | Three complete degree programs (CSU, Northeastern, University of Hawaiʻi at Mānoa), each with its analysis report JSON — input for `nuanalytics degree`, and the `sample:` degrees the MCP server serves. |
| `degree-output/` | Example `degree analyze` output: metrics, plan CSVs and HTML reports. |
| `claude-project-reference/` | Reference files for a Claude project that authors degree files. |

> [!WARNING]
> The integration tests read these files, so keep each curriculum in `plans/` and its
> reference in `planner-output/correct/`:
>
> | `plans/` | `planner-output/correct/` |
> |---|---|
> | `BSCS_Hawaii_Manoa.csv` | `BSCS_Hawaii_Manoa_w_metrics.csv` |
> | `California_Berkely_V2.csv` | `California_Berkely_V2_w_metrics.csv` |
> | `Colostate_CSDegree_2017_w_MATH.csv` | `Colostate_CSDegree_2017_w_MATH_w_metrics.csv` |
> | `Colostate_CSDegree_2017.csv` | `Colostate_CSDegree_2017_w_metrics.csv` |
> | `Colostate_CSDegree.csv` | `Colostate_CSDegree_w_metrics.csv` |
> | `Kennesaw_State_University_CS.csv` | `Kennesaw_State_University_CS_w_metrics.csv` |
> | `Metropolitan_State_University_CS.csv` | `Metropolitan_State_University_CS_w_metrics.csv` |
> | `Michigan_Ann_Arbor_CS.csv` | `Michigan_Ann_Arbor_CS_w_metrics.csv` |
> | `U_of_Colorado_Boulder_CS.csv` | `U_of_Colorado_Boulder_CS_w_metrics.csv` |
