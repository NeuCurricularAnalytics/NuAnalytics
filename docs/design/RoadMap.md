# NuAnalytics Roadmap

This roadmap organizes planned features by functional area. Items marked with ✅ are implemented, 🚧 are in progress, and 📋 are planned.

## Current Status (v0.2.0)

### Core Functionality ✅
- **CSV Plan Analysis**: Parse CSV curriculum files and compute metrics (delay, blocking, complexity, centrality)
- **YAML Degree Programs**: Load and validate comprehensive degree definitions with requirements
- **Prerequisite Parsing**: Parse complex AND/OR prerequisite expressions and build course graphs
- **Validation Framework**: Detect circular dependencies, missing courses, invalid patterns
- **Report Generation**: HTML (interactive), Markdown (text), PDF (via Chrome/Chromium)
- **Term Scheduling**: Automatic course scheduling respecting prerequisites and credit limits
- **Configuration System**: Persistent TOML config with CLI overrides
- **Audit Reports**: Identify missing prerequisites and deep prerequisite chains

---

## Phase 1: Plan Generation from Degrees 🚧

**Goal**: Generate valid course plans from YAML degree definitions that satisfy all requirements.

### 1.1 Plan Extraction Engine
- **Status**: 📋 Not Started
- **Description**: Core algorithm to generate valid plans from degree requirements
- **Tasks**:
  - Implement requirement satisfaction algorithm (all/select/one_of types)
  - Handle course patterns and grouping (`from.pattern`, `from.groups`)
  - Respect constraints (min/max credits, upper-division requirements, subject distribution)
  - Credit accounting system (track total, upper-division, in-major credits)
  - Double-counting rules (allow/prevent courses satisfying multiple requirements)
- **Dependencies**: Existing validation framework, prerequisite parser
- **Output**: `Plan` objects compatible with existing metrics computation

### 1.2 Plan Options and Filtering
- **Status**: 📋 Not Started
- **Description**: Generate multiple plan variations and filter by constraints
- **Tasks**:
  
  **Include Existing Plans** (`--include-plans`)
  - Import pre-existing CSV plan files as templates
  - Merge strategy options: `shortest`, `calc-ready`, `longest`, `replace`
  - Syntax: `--include-plans plan1.csv,plan2.csv:shortest,plan3.csv:calc-ready`
  - Validate imported plans satisfy degree requirements
  
  **Require Courses** (`--require`)
  - Force specific courses into all generated plans
  - Syntax: `--require CS150B,CS410,CS314`
  - Validate required courses satisfy degree requirements
  
  **Exclude Courses** (`--exclude`)
  - Prevent specific courses from appearing in generated plans
  - Syntax: `--exclude CS356,CS430`
  - Error if excluded course is mandatory for graduation
  - Verify alternative paths exist when excluding courses
  
  **Fuzzy Matching** (`--match`)
  - Include courses matching keywords in title/description
  - Syntax: `--match "machine learning"` or `--match "AI"`
  - Include related prerequisites automatically
  - Add courses only if room exists (don't exceed credit limits)
  - Respect requirement boundaries (electives, not required courses)

### 1.3 Plan Optimization Strategies
- **Status**: 📋 Not Started
- **Description**: Generate plans optimized for different student goals
- **Strategies**:
  - **Shortest Path**: Minimum courses to graduate (maximize AP/transfer credit)
  - **Calc-Ready**: Prioritize math/science prerequisites for STEM pipelines
  - **Balanced**: Even credit distribution across all terms
  - **Frontloaded**: Complete major courses early for co-op/internships
  - **Flexible**: Maximize elective space in later terms
- **CLI Syntax**: `--strategy shortest|calc-ready|balanced|frontloaded|flexible`

### 1.4 Plan Validation and Metrics
- **Status**: 📋 Not Started
- **Description**: Validate generated plans and compute curriculum metrics
- **Features**:
  - Verify all degree requirements satisfied
  - Check prerequisite ordering (no violations)
  - Compute plan-specific metrics (complexity, delay, blocking)
  - Compare multiple plans (complexity scores, flexibility metrics)
  - Generate comparative reports showing plan differences

---

## Phase 2: Database Integration 🚧

**Goal**: Store and query curriculum data across institutions with version tracking.

### 2.1 Database Schema and Backend
- **Status**: 📋 Not Started
- **Backend Options**: SQLite (local), PostgreSQL (production), Firebase (cloud sync)
- **Schema Design**:
  - **Institutions**: Name, IPEDS ID, type, location, calendar system
  - **Degrees**: Linked to institutions, versioned by catalog year
  - **Plans**: Multiple plans per degree, generated or imported
  - **Courses**: Institution-scoped, subject codes, titles, descriptions, prerequisites
  - **Metrics**: Precomputed metrics for degrees and plans (searchable)
- **Migration System**: Version database schema, handle upgrades gracefully

### 2.2 IPEDS Integration
- **Status**: 📋 Not Started
- **Description**: Integrate U.S. Department of Education IPEDS data
- **Data Included**:
  - Institution profiles (name, location, type, enrollment)
  - Degree completion statistics by CIP code
  - Graduation rates and demographics
  - Faculty and resource data
- **Features**:
  - Annual data refresh command: `nuanalytics db ipeds-refresh <year>`
  - Link degrees to IPEDS institutions via UNITID
  - Query institutions by characteristics (size, location, type)
- **Storage**: Download and cache IPEDS CSV files locally, import into database

### 2.3 Data Management Commands
- **Status**: 📋 Not Started
- **CLI Commands**:
  
  **Add/Update/Delete**
  ```bash
  nuanalytics db add-degree path/to/degree.yaml
  nuanalytics db add-plan path/to/plan.csv --degree-id neu-cs-2025
  nuanalytics db update-course CS2500 --credits 4 --title "New Title"
  nuanalytics db delete-degree <degree-id>
  ```
  
  **Import Workflows**
  - Bulk import: `nuanalytics db import-dir ./degrees/`
  - Auto-institution matching via fuzzy name matching
  - Prompt for manual assignment when fuzzy match uncertain
  - Skip duplicates or update if newer catalog year
  
  **Access Control**
  - Read-only mode (default for unprivileged users)
  - Write access requires authentication token
  - Institution-specific write permissions (maintainers can update their institution only)
  - Admin role for cross-institution operations

### 2.4 Search and Query Interface
- **Status**: 📋 Not Started
- **Description**: Search across institutions, degrees, courses, and prerequisites
- **Query Examples**:
  ```bash
  # Find institutions
  nuanalytics db search institutions --location Massachusetts --type "4-year"
  
  # Find degrees
  nuanalytics db search degrees --institution "Northeastern" --program CS
  nuanalytics db search degrees --complexity ">50" --year 2024-2025
  
  # Find courses
  nuanalytics db search courses --subject CS --level 3000+ --title "machine learning"
  nuanalytics db search courses --prereq-includes CS2500
  
  # Find prerequisite patterns
  nuanalytics db search prereqs --course CS3200 --across all-institutions
  nuanalytics db search plans --max-complexity 45 --degree-type BS
  ```
- **Output Formats**: Table, JSON, CSV for scripting and analysis

### 2.5 Comparative Analytics
- **Status**: 📋 Not Started
- **Description**: Compare curricula across institutions
- **Features**:
  - Compare complexity metrics across similar programs
  - Identify common prerequisite patterns (what do most schools require?)
  - Track curriculum evolution over catalog years
  - Generate institutional benchmarking reports
  - Identify outlier curricula (unusually complex or simple)
- **Example**: `nuanalytics db compare --program "BS Computer Science" --institutions NEU,BU,MIT`

---

## Phase 3: MCP Server Integration 📋

**Goal**: Expose NuAnalytics capabilities via Model Context Protocol for AI agent integration.

### 3.1 MCP Server Implementation
- **Status**: 📋 Not Started
- **Architecture**: 
  - Standalone MCP server binary (`nuanalytics-mcp`)
  - JSON-RPC interface following MCP specification
  - Reuses core library (`nu_analytics` crate)
- **Deployment**: 
  - Local server for development
  - Hosted service for production (with rate limiting)
  - Authentication via API tokens

### 3.2 MCP Tools - Degree Operations
- **Status**: 📋 Not Started
- **Tools**:
  
  **Validate Degree** (`validate_degree`)
  - Input: YAML degree definition (string or file path)
  - Output: Validation report (errors, warnings, statistics)
  - Use case: AI iteratively fixes degree definition until valid
  
  **Audit Degree** (`audit_degree`)
  - Input: Degree file path or YAML string
  - Output: Full audit report (validation + missing prereqs + deep chains)
  - Use case: Comprehensive degree quality assessment
  
  **Generate Plans** (`generate_plans`)
  - Input: Degree definition + constraints (require/exclude/match)
  - Output: Multiple plan variations with metrics
  - Use case: Explore curriculum options, compare strategies
  
  **Analyze Degree** (`analyze_degree`)
  - Input: Degree file path
  - Output: Metrics summary, complexity distribution, bottleneck courses
  - Use case: Quick degree assessment for curriculum designers

### 3.3 MCP Tools - Plan Operations
- **Status**: 📋 Not Started
- **Tools**:
  
  **Analyze Plan** (`analyze_plan`)
  - Input: CSV plan file
  - Output: Metrics CSV and HTML report URL
  - Use case: Quick metrics computation from CSV
  
  **Schedule Plan** (`schedule_plan`)
  - Input: Course list + constraints (credits per term, system type)
  - Output: Term-by-term schedule
  - Use case: Generate optimal course sequencing

### 3.4 MCP Tools - Database Operations
- **Status**: 📋 Not Started (requires Phase 2)
- **Tools**:
  
  **Search Institutions** (`search_institutions`)
  - Input: Filters (name, location, type, size)
  - Output: List of matching institutions with IDs
  
  **Search Degrees** (`search_degrees`)
  - Input: Filters (institution, program, complexity, year)
  - Output: List of matching degrees with metadata
  
  **Get Degree** (`get_degree`)
  - Input: Degree ID
  - Output: Full degree YAML definition
  
  **Compare Degrees** (`compare_degrees`)
  - Input: List of degree IDs
  - Output: Comparative metrics, requirement differences, complexity analysis

### 3.5 MCP Workflows
- **Status**: 📋 Not Started
- **Iterative Degree Building**: AI agent creates degree YAML, validates, fixes errors, re-validates until valid
- **Curriculum Comparison**: AI agent searches database, retrieves multiple degrees, compares structures
- **Plan Optimization**: AI agent generates multiple plans, analyzes metrics, recommends best option
- **Institutional Analysis**: AI agent pulls all degrees from institution, generates comparative report

---

## Phase 4: Advanced Analysis Features 📋

**Goal**: Add sophisticated curriculum analysis capabilities beyond basic metrics.

### 4.1 Bottleneck Detection
- **Status**: 📋 Not Started
- **Description**: Identify courses that create student progression barriers
- **Metrics**:
  - High blocking factor (many courses depend on it)
  - Low alternative paths (limited ways to satisfy prerequisites)
  - Term availability constraints (offered only once per year)
  - Capacity limitations (historically full sections)
- **Output**: Ranked list of bottleneck courses with recommendations

### 4.2 Cohort Flow Analysis
- **Status**: 📋 Not Started
- **Description**: Simulate student cohort progression through curricula
- **Features**:
  - Model course-taking patterns and failures
  - Predict time-to-degree distributions
  - Identify courses with high drop rates
  - Optimize course offering schedules
- **Input**: Historical enrollment and success rate data
- **Output**: Flow diagrams, retention predictions, scheduling recommendations

### 4.3 Curriculum Comparison and Evolution
- **Status**: 📋 Not Started
- **Description**: Track and analyze curriculum changes over time
- **Features**:
  - Diff two degree versions (show added/removed/changed courses)
  - Track complexity evolution across catalog years
  - Identify requirement restructuring patterns
  - Generate migration guides for students (old catalog → new catalog)
- **Example**: `nuanalytics compare neu-cs-2024 neu-cs-2025 --show-migrations`

### 4.4 Transfer Credit Analysis
- **Status**: 📋 Not Started
- **Description**: Analyze transfer pathways between institutions
- **Features**:
  - Map equivalent courses across institutions
  - Identify common articulation agreements
  - Compute "transfer-ability" metrics for curricula
  - Generate transfer student plans (optimize based on already-completed courses)
- **Use Case**: Community college → 4-year university pathways

### 4.5 Prerequisite Pattern Mining
- **Status**: 📋 Not Started
- **Description**: Extract common prerequisite patterns across institutions
- **Analysis**:
  - Cluster courses by prerequisite structure similarity
  - Identify consensus prerequisites for common courses
  - Detect institutional outliers (unusual prerequisite choices)
  - Recommend prerequisite adjustments based on peer institutions
- **Example**: "90% of CS programs require discrete math before data structures, but yours doesn't"

---

## Phase 5: Visualization and Reporting Enhancements 📋

**Goal**: Improve visual communication of curriculum analysis results.

### 5.1 Interactive HTML Enhancements
- **Status**: 📋 Not Started (basic HTML exists ✅)
- **Features**:
  - Drag-and-drop course reordering with live metric updates
  - Filter courses by subject, level, or complexity
  - Click course to see details panel (prerequisites, dependents, metrics)
  - Export modified plan back to CSV
  - Side-by-side plan comparison view

### 5.2 Graph Visualization
- **Status**: 🚧 In Progress (basic Mermaid diagrams exist)
- **Enhancements**:
  - Interactive graph with zoom and pan
  - Color-code nodes by complexity level
  - Highlight critical paths (longest delay paths)
  - Show/hide optional prerequisites (OR groups)
  - Export to DOT format for Graphviz rendering
  - Force-directed layout for large graphs

### 5.3 Dashboard and Summary Views
- **Status**: 📋 Not Started
- **Features**:
  - Institutional dashboard (all degrees, aggregate metrics)
  - Degree comparison matrix (side-by-side metrics)
  - Complexity heat maps (identify high-complexity areas)
  - Prerequisite chain sankey diagrams
  - Time-to-degree projections

### 5.4 Export Formats
- **Status**: 📋 Partial (HTML, Markdown, PDF exist ✅)
- **Additional Formats**:
  - LaTeX (for academic papers)
  - JSON (for programmatic access)
  - Excel/XLSX (for institutional research offices)
  - PNG/SVG graph exports

---

## Phase 6: CLI Enhancements 📋

**Goal**: Improve user experience and add power-user features.

### 6.1 Batch Processing
- **Status**: ✅ Implemented
- **Features**:
  - ✅ Process multiple files: `nuanalytics planner file1.csv file2.csv file3.csv`
  - ✅ Support glob patterns via shell expansion
  - 📋 Parallel processing for multiple files
  - 📋 Progress bars for long-running operations
  - 📋 Summary report across all processed files

### 6.2 Watch Mode
- **Status**: 📋 Not Started
- **Features**:
  - Auto-regenerate reports when files change
  - Live preview server for HTML reports
  - Hot reload in browser when reports update
- **Example**: `nuanalytics planner --watch curriculum.csv`

### 6.3 Template Generation
- **Status**: 📋 Not Started
- **Features**:
  - Generate starter degree YAML from questionnaire
  - Create CSV plan template with common courses
  - Export degree to CSV plan format
- **Example**: `nuanalytics template degree --institution NEU --program CS`

### 6.4 Shell Completions
- **Status**: 📋 Not Started
- **Description**: Generate shell completions for bash, zsh, fish
- **Implementation**: Use `clap_complete` crate
- **Example**: `nuanalytics completions bash > ~/.bash_completions/nuanalytics`

---

## Phase 7: Web Interface 📋

**Goal**: Browser-based curriculum analysis and degree building tool.

### 7.1 Web Application
- **Status**: 📋 Not Started
- **Architecture**:
  - Backend: Rust web server (Axum or Actix)
  - Frontend: SPA (React/Vue/Svelte)
  - API: RESTful endpoints wrapping core library
- **Features**:
  - Upload CSV/YAML files for analysis
  - Visual degree builder (drag-and-drop course requirements)
  - Live validation feedback as you build
  - Generate and download reports
  - Share analysis results via links

### 7.2 Collaborative Features
- **Status**: 📋 Not Started
- **Features**:
  - Multi-user editing of degree definitions
  - Comment threads on specific courses or requirements
  - Version history and rollback
  - Change proposals and approval workflows
- **Use Case**: Curriculum committees collaborating on degree revisions

---

## Phase 8: Research and Analytics 📋

**Goal**: Support academic research on curriculum design and student success.

### 8.1 Statistical Analysis
- **Status**: 📋 Not Started
- **Features**:
  - Correlate complexity metrics with graduation rates
  - Identify prerequisite structures that improve retention
  - A/B test different curriculum designs
  - Generate publication-ready figures and tables

### 8.2 Machine Learning Integration
- **Status**: 📋 Not Started
- **Features**:
  - Predict student success based on course history
  - Recommend courses based on student profile
  - Optimize curriculum structure using ML
  - Identify at-risk students early

### 8.3 Data Export for Research
- **Status**: 📋 Not Started
- **Features**:
  - Anonymized dataset export
  - Integration with R/Python for statistical analysis
  - Generate reproducible analysis scripts
  - Export citation-ready data summaries

---

## Technical Debt and Maintenance

### Code Quality
- ✅ Linting with Clippy (configured to deny perf/correctness)
- ✅ Pre-commit hooks (formatting, linting, commit messages)
- ✅ Comprehensive test suite (77 tests passing)
- 📋 Increase test coverage to 90%+ (currently ~70%)
- 📋 Add property-based testing for complex algorithms
- 📋 Performance benchmarks for large curricula (100+ courses)

### Documentation
- ✅ Command documentation (config.md, planner.md, degree.md)
- ✅ Development guide with contribution workflow
- ✅ API documentation via rustdoc
- 📋 User guide with tutorials and examples
- 📋 Video walkthroughs for common workflows
- 📋 Architecture documentation (design decisions, data flow)

### Compatibility
- 📋 Support older CSV formats from CurricularAnalytics.org
- 📋 Export to CurricularAnalytics.org JSON format
- 📋 Import from Banner, PeopleSoft, other SIS systems
- 📋 Standardize on YAML schema version (currently v5.1, v5.2 exists)

---

## Contributing

This roadmap is a living document. If you're interested in contributing to any of these features:

1. Check the [Development Guide](../../Development.md) for setup instructions
2. Look for issues tagged with the relevant phase on GitHub
3. Propose new features by opening an issue with the `enhancement` label
4. Discuss major architectural changes before implementing

**Priority guidance**: Features in Phase 1-2 align with core project goals and will be reviewed fastest. 
