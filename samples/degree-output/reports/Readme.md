# Sample Outputs

The following are sample outputs for the degree analysis. For the the MCP Generated, we used the following prompt:

```text
Using  nuanalytics - evaluate Colorado State University - Fort Collins, Computer Science concentration in Computer Science / General - degree. The evaluation should do teh following:

Build a YAML file, validate it - really pay close attention to the first two semesters and prereqs especially hidden ones with math - keep the yaml as an artifact
Use IPEDs data (via nuanalytics) to get the graduation rates from the last three years. I want to see charts broken up by gender and demographics . Also include line charts on parity scores, which is demographics of major (by gender by demographics) / demographics of all completions at CSU that year. 1 indicates a perfect match on demographics of major to university for that group.
Use the yaml to build a degree analysis - Include box plots for complexity and delay factor. Highlight bottle neck courses (high blocking factors), use the built in visualization to build graphs for shortest path, longest path, and samples, build a table of courses (sortable) listing all CS, Stats, Math, Datascience (DS), courses median complexity, delay, and blocking - use color coding to indicate high, median, low
Provide insights about the degree, both from the above information (strengths, areas for improvement, etc) and a websearch for additional information. Ideally include references to research / peer reviewed articles if any, or any other articles. Primary focus is work on education and diversity. Deliverable: html file artifact for the degree program anlaysis.
All plans should include CS150B and CS164 and MATH 156 (which means they shouldn't take MATH 160)
Be careful of the prerequistes for MATH 156 / MATH 160
You will want to be careful on the preqs with the yaml
Here is the requirements https://catalog.colostate.edu/general-catalog/colleges/natural-sciences/computer-science/computer-science-major/computer-science-concentration/#requirementstext
```

The others are base don the yaml file in the [../../degrees](../../degrees/) directory.
